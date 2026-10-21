// vm -- the stack machine that runs compiled scripts: fuel, limits, exceptions, the
// mark-and-sweep collector, the host interface
//
//   let mut vm = Vm::new(&program, Limits::default());   the top-level code is pending
//   match vm.run(&mut host, 10_000) {                     run it with up to 10 000 fuel
//       Status::Done(v) => ..                             finished with the value v
//       Status::Yielded => ..                             fuel used up, or a native asked to
//                                                         yield: run() again continues
//       Status::Error(e) => ..                            an uncaught exception or a limit; the
//                                                         invocation is gone, globals stay
//   }
//   vm.call("on_request", &[arg])?;  then run() as above   (only while idle)
//   vm.call_value(f, &args)?                               a function value, e.g. a pinned
//                                                          callback
//   vm.global("hits") / set_global("config", v)            globals of the script; a name the
//                                                          script takes from the host gets its
//                                                          value here
//   vm.output() / take_output()                            what print() wrote
//   vm.resume(host, fuel)                                  the same as run(), after Yielded
//
// Fuel is virtual CPU time: one unit per instruction, more for instructions that copy (string
// and list concatenation, literals, iterating a map), plus what natives charge.  run(host,
// fuel) stops before the next instruction once `fuel` is used, so a run may use a little more
// than asked (the extra of its last instruction); last_fuel() tells how much.  Splitting an
// invocation into runs of any size never changes its result, its output or its total fuel:
// all state lives in the VM, and the collector runs at allocations only.
//
// Values: nil and false are falsy, everything else is truthy.  int is i64 with wrapping +, -,
// * and unary minus; / and % truncate (like C, Go, Rust), x / 0 and x % 0 throw.  An int
// meeting a float becomes a float; float arithmetic is num.rs (bit-identical everywhere).  ==
// compares numbers exactly across int and float, str and bytes by content, lists, maps and
// functions by identity.  + also joins str, bytes and lists.  Map keys are str, int or bool;
// a missing key or field reads as nil; lists throw on an index outside 0..len.  for over a map
// visits the keys it had when the loop started (in insertion order, skipping removed ones);
// over a list it reads the length each round.  Calls pass missing arguments as nil and drop
// extra ones.
//
// Names the script does not declare come from the host: print, len, type, str, int and float
// are built in, the standard library (stdlib.rs: str.split, list.sort, math.sqrt, json.encode,
// math.pi ..) comes next; f(..) and obj.method(..) on any other host name call Host::call with
// the name ("log", "http.serve"); obj.m(..) on a value calls a function stored under m in a
// map, the built-in list.push / list.pop, the library's type.m, or Host::call("list.m", [obj,
// args..]).  set_global gives a host name a value (config), which then takes precedence.
// Reading a host name as a value (let f = log) needs Host::knows (library names need none);
// an unknown one throws "Unbekannter Name".  A native can call a function value back to its
// end with Cx::call (sort, map); a yield it asks for then waits until the native returns, and
// an exception that leaves the callback keeps the position it was thrown at.  cancel() drops
// a pending invocation.
//
// Runtime errors (wrong types, division by 0, a bad index) throw a str with a German message
// that try/catch can catch; an uncaught one ends the invocation with an Error (message,
// line and column, a stack trace).  The limits (fuel per invocation, heap, call depth, stack)
// end it at once and cannot be caught.
//
// Collector: mark and sweep over the heap arena, from the stack, the globals, host values,
// pinned values, the last result and objects a native made during its call.  It runs only in
// run(), when an allocation crosses the threshold (twice the live bytes after the last
// collection, at least 256 KiB, at most the heap limit).  Values the host keeps between runs
// must be pinned (pin / unpin) or be reachable from the script.  Values the host passes in
// are checked (valid()): a stale value of a freed object gives an error or None, not a crash.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::code::{Code, Const, Op, Program, Up};
use crate::heap::{self, Closure, Heap, Key, Map, Obj, Ref, StrObj, Value};
use crate::{jsonlib, num, stdlib};

/// the limits of a VM; crossing one ends the invocation (scripts cannot catch it)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// heap bytes, counted the same way on every target (default 16 MiB)
    pub heap: u64,
    /// values on the stack: parameters, locals and temporaries of all calls (100 000)
    pub stack: u32,
    /// calls inside each other (500)
    pub depth: u32,
    /// fuel of one invocation (the top-level code, call, call_value) over all its runs
    /// (10 000 000)
    pub fuel: u64,
    /// bytes of print output kept; more is dropped (1 MiB)
    pub output: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { heap: 16 << 20, stack: 100_000, depth: 500, fuel: 10_000_000, output: 1 << 20 }
    }
}

/// what an Error is about
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    /// an exception nobody caught: throw, or a runtime error such as a division by 0
    Uncaught,
    /// the invocation used more fuel than Limits::fuel
    Fuel,
    /// the heap would grow beyond Limits::heap
    Heap,
    /// more nested calls than Limits::depth
    Depth,
    /// more stack values than Limits::stack
    Stack,
    /// a native function failed for good (Fail::Fatal)
    Host,
    /// the request does not fit the VM's state: busy, nothing to run, no such function
    Usage,
    /// a bug of the VM or the compiler
    Internal,
}

/// a function and the position it was at when the error happened
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TraceLine {
    /// "" for the top-level code, "fn" for an anonymous function
    pub func: String,
    pub line: u32,
    pub col: u32,
}

/// why an invocation ended without a result
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    /// German text; for an uncaught throw the thrown value as text
    pub msg: String,
    /// where it happened (0 when not in the script)
    pub line: u32,
    pub col: u32,
    /// the calls, innermost first
    pub trace: Vec<TraceLine>,
}

impl core::fmt::Display for Error {
    /// "3:5: Fehler: Division durch 0", then a line per call: "  in teile (3:5)"
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}:{}: Fehler: {}", self.line, self.col, self.msg)?;
        for t in &self.trace {
            if t.func.is_empty() {
                write!(f, "\n  im Hauptprogramm ({}:{})", t.line, t.col)?;
            } else {
                write!(f, "\n  in {} ({}:{})", t.func, t.line, t.col)?;
            }
        }
        Ok(())
    }
}

/// the outcome of run()
#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    /// the invocation returned this value (valid until the next run)
    Done(Value),
    /// fuel used up, or a native asked to yield; run() again continues
    Yielded,
    Error(Error),
}

/// why a native function returned no value
#[derive(Clone, PartialEq, Debug)]
pub enum Fail {
    /// the host has no function of that name
    Unknown,
    /// throw this value in the script (catchable); Cx::error makes one from a message
    Throw(Value),
    /// end the invocation with this message (not catchable)
    Fatal(String),
}

/// the functions a script can call besides its own: Host::call gets the name as the script
/// wrote it ("log", "http.serve"), or "type.method" with the object as args[0] for methods of
/// the built-in types ("list.sum", "map.json"); return Err(Fail::Unknown) for names you do
/// not have.  The built-ins (print, len, type, str, int, float, list.push, list.pop) and the
/// standard library (script::library(): str.split, list.sort, json.encode ..) never get here.
pub trait Host {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail>;

    /// whether `name` ("log", "http", "http.serve") may be read as a value (a native function
    /// value), as in `let f = log` or `list.each(log)`; reading an unknown name throws
    /// "Unbekannter Name".  Calls do not ask.
    fn knows(&self, name: &str) -> bool {
        let _ = name;
        false
    }
}

/// no host functions
impl Host for () {
    fn call(&mut self, _: &mut Cx, _: &str, _: &[Value]) -> Result<Value, Fail> {
        Err(Fail::Unknown)
    }
}

/// a value the host keeps alive (see Vm::pin)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pin(u32);

#[derive(Clone, Copy, Debug)]
struct Frame {
    proto: u32,
    closure: Option<Ref>,
    /// stack index of slot 0 (the first parameter); the callee sits below it
    base: u32,
    /// the next instruction (pc - 1 is the current one, for callers the call)
    pc: u32,
}

pub(crate) enum Exc {
    /// a runtime error: thrown as a str
    Msg(String),
    Throw(Value),
    Fatal(ErrorKind, String),
}

fn msg<T>(s: String) -> Result<T, Exc> {
    Err(Exc::Msg(s))
}

fn internal<T>() -> Result<T, Exc> {
    Err(Exc::Fatal(ErrorKind::Internal, String::from("Interner Fehler der Skript-VM")))
}

/// the message of the fuel limit
pub(crate) fn fuel_msg(limit: u64) -> String {
    format!("Rechenzeit aufgebraucht: mehr als {} Schritte in einem Aufruf – eine Endlosschleife?", limit)
}

#[derive(Clone, Copy, Debug)]
enum Builtin {
    Print,
    Len,
    Type,
    Str,
    Int,
    Float,
    /// a function of the standard library (stdlib.rs)
    Lib(stdlib::LibFn),
}

fn builtin_of(name: &str) -> Option<Builtin> {
    Some(match name {
        "print" => Builtin::Print,
        "len" => Builtin::Len,
        "type" => Builtin::Type,
        "str" => Builtin::Str,
        "int" => Builtin::Int,
        "float" => Builtin::Float,
        _ => return stdlib::find(name).map(Builtin::Lib),
    })
}

const TYPES: [&str; 9] = ["nil", "bool", "int", "float", "str", "bytes", "list", "map", "fn"];

const MIN_GC: u64 = 256 << 10;

/// Cx::call nested in Cx::call at most this deep (each level also nests Rust calls)
const MAX_NEST: u32 = 20;

/// a running script; Clone copies the whole state (heap included), Send moves it to another
/// thread
#[derive(Clone)]
pub struct Vm {
    pub(crate) code: Arc<Code>,
    pub(crate) lim: Limits,
    pub(crate) heap: Heap,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: Vec<Value>,
    /// values the host gave to host names (indexed like code.hosts)
    hostv: Vec<Option<Value>>,
    consts: Vec<Value>,
    /// the type names as permanent strings
    tnames: [Value; 9],
    /// heap objects below this index are the permanent constants
    permanent: u32,
    /// native function names (the program's host names first) and the built-in they are
    natives: Vec<(Arc<str>, Option<Builtin>)>,
    pins: Vec<Option<Value>>,
    pin_free: Vec<u32>,
    out: String,
    /// fuel of the current invocation before this run, and used in this run
    pub(crate) call_fuel: u64,
    pub(crate) used: u64,
    last: u64,
    pub(crate) running: bool,
    in_native: bool,
    yield_req: bool,
    /// objects allocated by the native running now (and values it keeps while it calls back)
    pub(crate) nroots: Vec<Ref>,
    result: Value,
    next_gc: u64,
    gcs: u32,
    scratch: Vec<Value>,
    /// exec returns Done when a Return brings the frames down to this many (0, or where a
    /// native's Cx::call started)
    floor: usize,
    /// Cx::call inside Cx::call ..
    nest: u32,
    /// the kind of a fatal error inside Cx::call, for the Fail::Fatal it becomes
    pub(crate) fatal_kind: Option<ErrorKind>,
    /// where an exception (Some) or a fatal error (None) left a callback of a native: the
    /// trace from there, for the Error when nothing catches it further out
    escaped: Option<(Option<Value>, Vec<TraceLine>)>,
}

impl Vm {
    /// a VM for the program with its top-level code pending (run() runs it)
    pub fn new(prog: &Program, limits: Limits) -> Vm {
        let code = prog.code.clone();
        let mut heap = Heap::default();
        let mut consts = Vec::with_capacity(code.consts.len());
        for c in &code.consts {
            consts.push(match c {
                Const::Int(i) => Value::Int(*i),
                Const::Float(f) => Value::Float(*f),
                Const::Str(s) => Value::Str(heap.insert(Obj::Str(StrObj::new(s)))),
            });
        }
        let tnames = TYPES.map(|t| Value::Str(heap.insert(Obj::Str(StrObj::new(t)))));
        let permanent = heap.objs.len() as u32;
        heap.bytes = 0;
        let natives = code.hosts.iter().map(|h| (Arc::from(h.name.as_str()), builtin_of(&h.name))).collect();
        Vm {
            globals: vec![Value::Nil; code.globals.len()],
            hostv: vec![None; code.hosts.len()],
            code,
            lim: limits,
            heap,
            stack: vec![Value::Nil],
            frames: vec![Frame { proto: 0, closure: None, base: 1, pc: 0 }],
            consts,
            tnames,
            permanent,
            natives,
            pins: Vec::new(),
            pin_free: Vec::new(),
            out: String::new(),
            call_fuel: 0,
            used: 0,
            last: 0,
            running: false,
            in_native: false,
            yield_req: false,
            nroots: Vec::new(),
            result: Value::Nil,
            next_gc: MIN_GC.min(limits.heap),
            gcs: 0,
            scratch: Vec::new(),
            floor: 0,
            nest: 0,
            fatal_kind: None,
            escaped: None,
        }
    }

    // ---- invocations

    /// whether an invocation is pending (started or yielded)
    pub fn busy(&self) -> bool {
        !self.frames.is_empty()
    }

    /// start a call of the script's global function `name` (run() runs it)
    pub fn call(&mut self, name: &str, args: &[Value]) -> Result<(), Error> {
        match self.global(name) {
            Some(f @ Value::Func(_)) => self.call_value(f, args),
            _ => Err(usage(format!("Unbekannte Funktion {}", name))),
        }
    }

    /// start a call of a function value (run() runs it)
    pub fn call_value(&mut self, f: Value, args: &[Value]) -> Result<(), Error> {
        if self.busy() {
            return Err(usage(String::from("Die VM ist noch beschäftigt – erst run() bis zum Ende")));
        }
        let Value::Func(r) = f else {
            return Err(usage(format!("{} ist keine Funktion", f.type_name())));
        };
        if !self.valid(f) || !args.iter().all(|a| self.valid(*a)) {
            return Err(invalid());
        }
        self.stack.clear();
        self.stack.push(f);
        self.stack.extend_from_slice(args);
        self.call_fuel = 0;
        let code = self.code.clone();
        if let Err(e) = self.push_frame(&code, r, 0, args.len()) {
            let err = match e {
                Exc::Fatal(kind, m) => Error { kind, msg: m, line: 0, col: 0, trace: Vec::new() },
                _ => usage(String::from("Aufruf nicht möglich")),
            };
            self.abort();
            return Err(err);
        }
        Ok(())
    }

    /// run the pending invocation with up to `fuel` fuel (see the module comment)
    pub fn run(&mut self, host: &mut dyn Host, fuel: u64) -> Status {
        if !self.busy() {
            return Status::Error(usage(String::from("Es läuft kein Aufruf – erst call()")));
        }
        self.used = 0;
        self.running = true;
        let code = self.code.clone();
        if self.frames.len() == 1 && self.frames[0].closure.is_none() && self.frames[0].pc == 0 {
            // the top-level code: the stack check push_frame does for calls
            if 1 + code.protos[0].max_stack as usize + 4 > self.lim.stack as usize {
                let e = self.error_here(&code, ErrorKind::Stack, format!("Der Stapel ist voll (mehr als {} Werte)", self.lim.stack));
                self.abort();
                self.running = false;
                return Status::Error(e);
            }
        }
        let st = loop {
            match self.exec(&code, host, fuel) {
                Ok(s) => break s,
                Err(e) => {
                    if let Err(err) = self.catch(&code, e) {
                        self.abort();
                        break Status::Error(err);
                    }
                }
            }
        };
        self.running = false;
        self.call_fuel += self.used;
        self.last = self.used;
        if let Status::Done(v) = st {
            self.result = v;
        }
        st
    }

    /// continue an invocation after Status::Yielded: the same as run(), named for readers of
    /// host code
    pub fn resume(&mut self, host: &mut dyn Host, fuel: u64) -> Status {
        self.run(host, fuel)
    }

    /// drop the pending invocation (a yielded callback of a device that was switched off, say);
    /// globals keep what it changed so far
    pub fn cancel(&mut self) {
        self.abort();
    }

    /// fuel used by the last run()
    pub fn last_fuel(&self) -> u64 {
        self.last
    }

    /// fuel used by the current (or last) invocation so far
    pub fn call_fuel(&self) -> u64 {
        self.call_fuel
    }

    /// the limits the VM was made with
    pub fn limits(&self) -> Limits {
        self.lim
    }

    /// what print() wrote since the last take_output()
    pub fn output(&self) -> &str {
        &self.out
    }

    /// what print() wrote, emptying the buffer
    pub fn take_output(&mut self) -> String {
        core::mem::take(&mut self.out)
    }

    /// a global of the script, or the value the host gave a host name
    pub fn global(&self, name: &str) -> Option<Value> {
        if let Some(i) = self.code.globals.iter().position(|g| g == name) {
            return Some(self.globals[i]);
        }
        let i = self.code.hosts.iter().position(|h| h.name == name)?;
        self.hostv[i]
    }

    /// set a global of the script, or give a name the script takes from the host a value (a
    /// map for config, say); false when the script uses no such name
    pub fn set_global(&mut self, name: &str, v: Value) -> bool {
        if !self.valid(v) {
            return false;
        }
        if let Some(i) = self.code.globals.iter().position(|g| g == name) {
            self.globals[i] = v;
            return true;
        }
        match self.code.hosts.iter().position(|h| h.name == name && h.dotted.is_none()) {
            Some(i) => {
                self.hostv[i] = Some(v);
                true
            }
            None => false,
        }
    }

    // ---- values for hosts

    /// whether a value belongs to this VM and is alive: a value of an object the collector
    /// freed (not pinned between runs), of another VM, or a cell is not
    pub fn valid(&self, v: Value) -> bool {
        match v {
            Value::Native(i) => (i as usize) < self.natives.len(),
            Value::Cell(_) => false,
            _ => match v.heap_ref() {
                None => true,
                Some(r) => matches!(
                    (v, self.heap.objs.get(r.0 as usize)),
                    (Value::Str(_), Some(Obj::Str(_)))
                        | (Value::Bytes(_), Some(Obj::Bytes(_)))
                        | (Value::List(_), Some(Obj::List(_)))
                        | (Value::Map(_), Some(Obj::Map(_)))
                        | (Value::Func(_), Some(Obj::Func(_)))
                ),
            },
        }
    }

    /// the text of a str
    pub fn str(&self, v: Value) -> Option<&str> {
        match v {
            Value::Str(r) if self.valid(v) => Some(heap::text(&self.heap, r)),
            _ => None,
        }
    }

    /// the content of a bytes value
    pub fn bytes(&self, v: Value) -> Option<&[u8]> {
        match v {
            Value::Bytes(r) if self.valid(v) => Some(self.heap.bytes_of(r)),
            _ => None,
        }
    }

    /// the items of a list
    pub fn list(&self, v: Value) -> Option<&[Value]> {
        match v {
            Value::List(r) if self.valid(v) => Some(self.heap.list(r)),
            _ => None,
        }
    }

    /// m[key]; None when m is no map, the key is missing or of a type maps do not have
    pub fn map_get(&self, m: Value, key: Value) -> Option<Value> {
        let Value::Map(r) = m else { return None };
        if !self.valid(m) || !self.valid(key) {
            return None;
        }
        let k = self.heap.key(key)?;
        self.heap.map_find(r, k).map(|i| self.heap.map(r).entries[i].val)
    }

    /// m.key
    pub fn map_get_str(&self, m: Value, key: &str) -> Option<Value> {
        let Value::Map(r) = m else { return None };
        if !self.valid(m) {
            return None;
        }
        let k = Key::Str(key, heap::hash_str(key));
        self.heap.map_find(r, k).map(|i| self.heap.map(r).entries[i].val)
    }

    /// the entries of a map in order
    pub fn map_entries(&self, m: Value) -> Vec<(Value, Value)> {
        match m {
            Value::Map(r) if self.valid(m) => self.heap.map(r).iter().map(|e| (e.key, e.val)).collect(),
            _ => Vec::new(),
        }
    }

    /// a new str (valid until the next run unless the script keeps it or it is pinned)
    pub fn new_str(&mut self, s: &str) -> Result<Value, Error> {
        self.alloc(Obj::Str(StrObj::new(s))).map(Value::Str).map_err(plain)
    }

    /// a new bytes value
    pub fn new_bytes(&mut self, b: &[u8]) -> Result<Value, Error> {
        self.alloc(Obj::Bytes(b.into())).map(Value::Bytes).map_err(plain)
    }

    /// a new list of these items
    pub fn new_list(&mut self, items: Vec<Value>) -> Result<Value, Error> {
        if !items.iter().all(|v| self.valid(*v)) {
            return Err(invalid());
        }
        self.alloc(Obj::List(items)).map(Value::List).map_err(plain)
    }

    /// a new empty map
    pub fn new_map(&mut self) -> Result<Value, Error> {
        self.alloc(Obj::Map(Default::default())).map(Value::Map).map_err(plain)
    }

    /// m[k] = v (k: str, int or bool)
    pub fn map_set(&mut self, m: Value, k: Value, v: Value) -> Result<(), Error> {
        if !self.valid(m) || !self.valid(k) || !self.valid(v) {
            return Err(invalid());
        }
        self.set_key(m, k, v).map_err(plain)
    }

    /// remove m[k]; the old value
    pub fn map_remove(&mut self, m: Value, k: Value) -> Option<Value> {
        let Value::Map(r) = m else { return None };
        if !self.valid(m) || !self.valid(k) {
            return None;
        }
        let i = self.heap.key(k).and_then(|key| self.heap.map_find(r, key))?;
        self.heap.update(r, |o| match o {
            Obj::Map(map) => {
                let old = map.entries[i].val;
                map.remove_at(i);
                Some(old)
            }
            _ => None,
        })
    }

    /// append v to the list l
    pub fn list_push(&mut self, l: Value, v: Value) -> Result<(), Error> {
        let Value::List(r) = l else { return Err(usage(String::from("list_push braucht eine Liste"))) };
        if !self.valid(l) || !self.valid(v) {
            return Err(invalid());
        }
        self.reserve(16).map_err(plain)?;
        self.heap.update(r, |o| {
            if let Obj::List(items) = o {
                items.push(v);
            }
        });
        Ok(())
    }

    /// l[i] = v (false when l is no list or i is out of range)
    pub fn list_set(&mut self, l: Value, i: usize, v: Value) -> bool {
        let Value::List(r) = l else { return false };
        if !self.valid(l) || !self.valid(v) {
            return false;
        }
        match self.heap.get_mut(r) {
            Obj::List(items) if i < items.len() => {
                items[i] = v;
                true
            }
            _ => false,
        }
    }

    /// replace the items of a list (sorting, filtering in place)
    pub fn list_replace(&mut self, l: Value, items: Vec<Value>) -> Result<(), Error> {
        let Value::List(r) = l else { return Err(usage(String::from("list_replace braucht eine Liste"))) };
        if !self.valid(l) || !items.iter().all(|v| self.valid(*v)) {
            return Err(invalid());
        }
        let grow = 16 * items.len().saturating_sub(self.heap.list(r).len()) as u64;
        self.reserve(grow).map_err(plain)?;
        self.heap.update(r, |o| {
            if let Obj::List(v) = o {
                *v = items;
            }
        });
        Ok(())
    }

    /// a native function value for a host name ("timer.cancel")
    pub fn native(&mut self, name: &str) -> Value {
        if let Some(i) = self.natives.iter().position(|n| &*n.0 == name) {
            return Value::Native(i as u32);
        }
        self.natives.push((Arc::from(name), builtin_of(name)));
        Value::Native(self.natives.len() as u32 - 1)
    }

    /// keep a value alive while the host holds it (a callback, say); an invalid value pins nil
    pub fn pin(&mut self, v: Value) -> Pin {
        let v = if self.valid(v) { v } else { Value::Nil };
        if let Some(i) = self.pin_free.pop() {
            self.pins[i as usize] = Some(v);
            return Pin(i);
        }
        self.pins.push(Some(v));
        Pin(self.pins.len() as u32 - 1)
    }

    /// the pinned value (nil after unpin)
    pub fn pinned(&self, p: Pin) -> Value {
        self.pins.get(p.0 as usize).copied().flatten().unwrap_or(Value::Nil)
    }

    /// let the collector have the value again
    pub fn unpin(&mut self, p: Pin) {
        if let Some(x) = self.pins.get_mut(p.0 as usize) {
            if x.take().is_some() {
                self.pin_free.push(p.0);
            }
        }
    }

    /// the value as print() shows it (strings as they are, inside lists and maps quoted)
    pub fn display(&self, v: Value) -> String {
        if !self.valid(v) {
            return String::from("<ungültig>");
        }
        let mut s = String::new();
        self.write_value(&mut s, v, false, 0, &mut Vec::new());
        s
    }

    /// == of the script
    pub fn equal(&self, a: Value, b: Value) -> bool {
        if !self.valid(a) || !self.valid(b) {
            return false;
        }
        self.same(a, b)
    }

    /// v as JSON text like json.encode (indent 0: compact, else spaces per level); an Error
    /// (kind Uncaught, the German message) for data JSON cannot hold
    pub fn json_encode(&mut self, v: Value, indent: u32) -> Result<String, Error> {
        if !self.valid(v) {
            return Err(invalid());
        }
        let saved = (self.used, self.call_fuel);
        (self.used, self.call_fuel) = (0, 0);
        let r = jsonlib::encode(self, v, indent);
        (self.used, self.call_fuel) = saved;
        r.map_err(plain)
    }

    /// the value of a JSON text like json.decode (valid until the next run unless the script
    /// keeps it or it is pinned)
    pub fn json_decode(&mut self, text: &str) -> Result<Value, Error> {
        let saved = (self.used, self.call_fuel);
        (self.used, self.call_fuel) = (0, 0);
        let r = jsonlib::decode(self, text.as_bytes());
        (self.used, self.call_fuel) = saved;
        r.map_err(plain)
    }

    pub(crate) fn same(&self, a: Value, b: Value) -> bool {
        match (a, b) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => num::cmp_int(x, y) == Some(Ordering::Equal),
            (Value::Float(x), Value::Float(y)) => num::eq(x, y),
            (Value::Str(x), Value::Str(y)) => x == y || heap::text(&self.heap, x) == heap::text(&self.heap, y),
            (Value::Bytes(x), Value::Bytes(y)) => x == y || self.heap.bytes_of(x) == self.heap.bytes_of(y),
            (Value::List(x), Value::List(y)) | (Value::Map(x), Value::Map(y)) | (Value::Func(x), Value::Func(y)) => x == y,
            (Value::Native(x), Value::Native(y)) => x == y,
            _ => false,
        }
    }

    // ---- the collector

    /// collect garbage now: everything the script cannot reach, except pinned values and the
    /// last result, is freed
    pub fn collect(&mut self) {
        let mut gray: Vec<u32> = Vec::new();
        let roots = self.stack.iter().chain(self.globals.iter()).chain(self.hostv.iter().flatten()).chain(self.pins.iter().flatten());
        for v in roots.chain(core::iter::once(&self.result)) {
            self.heap.mark(*v, &mut gray);
        }
        for r in &self.nroots {
            self.heap.mark_ref(*r, &mut gray);
        }
        for f in &self.frames {
            if let Some(c) = f.closure {
                self.heap.mark_ref(c, &mut gray);
            }
        }
        self.heap.trace(&mut gray);
        self.heap.sweep(self.permanent);
        self.next_gc = (self.heap.bytes * 2).max(MIN_GC).min(self.lim.heap);
        self.gcs += 1;
    }

    /// accounted bytes of the collectable objects
    pub fn heap_bytes(&self) -> u64 {
        self.heap.bytes
    }

    /// objects on the heap (without the program's constants)
    pub fn heap_objects(&self) -> u32 {
        self.heap.count - self.permanent
    }

    /// collections so far
    pub fn collections(&self) -> u32 {
        self.gcs
    }

    /// make room for n more bytes: collect when the threshold is crossed (only in run(), where
    /// every live value is rooted), fail beyond the limit
    pub(crate) fn reserve(&mut self, n: u64) -> Result<(), Exc> {
        if self.running && self.heap.bytes + n > self.next_gc {
            self.collect();
        }
        if self.heap.bytes + n > self.lim.heap {
            return Err(Exc::Fatal(ErrorKind::Heap, format!("Speichergrenze erreicht ({} Bytes)", self.lim.heap)));
        }
        Ok(())
    }

    /// a new object; the values it holds must be rooted elsewhere until it is (on the stack)
    pub(crate) fn alloc(&mut self, o: Obj) -> Result<Ref, Exc> {
        self.reserve(heap::size_of(&o))?;
        let r = self.heap.insert(o);
        if self.in_native {
            self.nroots.push(r);
        }
        Ok(r)
    }

    pub(crate) fn alloc_str(&mut self, s: String) -> Result<Value, Exc> {
        let o = StrObj { hash: heap::hash_str(&s), chars: s.chars().count() as u32, s: s.into_boxed_str() };
        self.alloc(Obj::Str(o)).map(Value::Str)
    }

    // ---- errors

    fn abort(&mut self) {
        self.frames.clear();
        self.stack.clear();
        self.nroots.clear();
        self.in_native = false;
        self.yield_req = false;
        self.floor = 0;
        self.nest = 0;
        self.fatal_kind = None;
        self.escaped = None;
    }

    /// the position in every frame, innermost first
    fn trace(&self, code: &Code) -> Vec<TraceLine> {
        let mut trace = Vec::new();
        for f in self.frames.iter().rev() {
            let p = &code.protos[f.proto as usize];
            let at = (f.pc as usize).saturating_sub(1).min(p.pos.len().saturating_sub(1));
            let (line, col) = p.pos.get(at).copied().unwrap_or((0, 0));
            trace.push(TraceLine { func: p.name.clone(), line, col });
        }
        trace
    }

    /// an Error at the current instruction, with the trace of all frames
    fn error_here(&self, code: &Code, kind: ErrorKind, msg: String) -> Error {
        let trace = self.trace(code);
        let (line, col) = trace.first().map_or((0, 0), |t| (t.line, t.col));
        Error { kind, msg, line, col, trace }
    }

    /// the Error of an exception (Some) or a fatal error (None) nothing caught: where it left a
    /// callback when it came from one, else at the current instruction
    fn located(&mut self, code: &Code, v: Option<Value>, kind: ErrorKind, msg: String) -> Error {
        match self.escaped.take() {
            Some((ev, trace)) if ev == v && !trace.is_empty() => {
                let (line, col) = (trace[0].line, trace[0].col);
                Error { kind, msg, line, col, trace }
            }
            _ => self.error_here(code, kind, msg),
        }
    }

    /// note where an exception or a fatal error leaves a callback (the innermost one wins)
    fn escape(&mut self, code: &Code, v: Option<Value>) {
        if !self.escaped.as_ref().is_some_and(|(ev, _)| *ev == v) {
            self.escaped = Some((v, self.trace(code)));
        }
    }

    /// the value a runtime error throws (its message as a str); Err when even that does not fit
    fn thrown(&mut self, e: Exc) -> Result<Value, (ErrorKind, String)> {
        match e {
            Exc::Fatal(kind, m) => Err((kind, m)),
            Exc::Throw(v) => Ok(v),
            Exc::Msg(m) => self.alloc_str(m.clone()).map_err(|_| (ErrorKind::Uncaught, m)),
        }
    }

    /// throw v to the innermost handler around the current instruction of a frame above the
    /// floor; false when there is none (the frames stay)
    fn unwind(&mut self, code: &Code, v: Value) -> bool {
        for fi in (self.floor..self.frames.len()).rev() {
            let f = self.frames[fi];
            let at = f.pc.saturating_sub(1);
            let p = &code.protos[f.proto as usize];
            if let Some(h) = p.handlers.iter().find(|h| h.start <= at && at < h.end) {
                self.frames.truncate(fi + 1);
                self.frames[fi].pc = h.target;
                self.stack.truncate((f.base + h.height) as usize);
                self.stack.push(v);
                return true;
            }
        }
        false
    }

    /// an exception of the invocation: caught by a handler, or the Error that ends it
    fn catch(&mut self, code: &Code, e: Exc) -> Result<(), Error> {
        let v = match self.thrown(e) {
            Ok(v) => v,
            Err((kind, m)) => return Err(self.located(code, None, kind, m)),
        };
        if self.unwind(code, v) {
            self.escaped = None;
            return Ok(());
        }
        let m = match v {
            Value::Str(r) => String::from(heap::text(&self.heap, r)),
            _ => self.display(v),
        };
        Err(self.located(code, Some(v), ErrorKind::Uncaught, m))
    }

    /// Cx::call: run f with args to its end inside a native; its result, or the exception it
    /// did not catch
    pub(crate) fn nested(&mut self, host: &mut dyn Host, f: Value, args: &[Value]) -> Result<Value, Fail> {
        if self.nest >= MAX_NEST {
            self.fatal_kind = Some(ErrorKind::Depth);
            return Err(Fail::Fatal(format!("Zu tief verschachtelte Rückrufe (mehr als {})", MAX_NEST)));
        }
        if !self.valid(f) || !args.iter().all(|a| self.valid(*a)) {
            return Err(Fail::Fatal(String::from("Ungültiger Wert für Cx::call")));
        }
        let code = self.code.clone();
        let (fpos, entry) = (self.stack.len(), self.frames.len());
        self.stack.push(f);
        self.stack.extend_from_slice(args);
        let saved = (self.floor, self.in_native);
        self.floor = entry;
        self.in_native = false;
        self.nest += 1;
        let r = self.nested_run(&code, host, f, fpos, args.len());
        self.nest -= 1;
        (self.floor, self.in_native) = saved;
        self.frames.truncate(entry);
        self.stack.truncate(fpos);
        // the native may allocate before it returns the value
        if let Ok(v) | Err(Fail::Throw(v)) = &r {
            if let Some(rf) = v.heap_ref() {
                self.nroots.push(rf);
            }
        }
        r
    }

    fn nested_run(&mut self, code: &Code, host: &mut dyn Host, f: Value, fpos: usize, argc: usize) -> Result<Value, Fail> {
        let mut res: Result<Status, Exc> = match f {
            Value::Func(r) => match self.push_frame(code, r, fpos, argc) {
                Ok(()) => self.exec(code, host, u64::MAX),
                Err(e) => Err(e),
            },
            Value::Native(i) => self.call_native(host, i, fpos + 1, fpos).map(|_| Status::Done(self.stack[fpos])),
            v => msg(format!("{} ist keine Funktion", v.type_name())),
        };
        let mut yielded = false;
        loop {
            let e = match res {
                Ok(Status::Done(v)) => {
                    self.yield_req |= yielded;
                    return Ok(v);
                }
                Ok(Status::Yielded) => {
                    // a native asked to yield: that happens after the outer call
                    yielded = true;
                    res = self.exec(code, host, u64::MAX);
                    continue;
                }
                Ok(Status::Error(e)) => return Err(Fail::Fatal(e.msg)),
                Err(e) => e,
            };
            match self.thrown(e) {
                Err((kind, m)) => {
                    self.fatal_kind = Some(kind);
                    self.escape(code, None);
                    return Err(Fail::Fatal(m));
                }
                Ok(v) => {
                    if !self.unwind(code, v) {
                        self.escape(code, Some(v));
                        return Err(Fail::Throw(v));
                    }
                    self.escaped = None;
                }
            }
            res = self.exec(code, host, u64::MAX);
        }
    }

    // ---- calls

    fn push_frame(&mut self, code: &Code, r: Ref, fpos: usize, argc: usize) -> Result<(), Exc> {
        let proto = self.heap.closure(r).proto;
        let p = &code.protos[proto as usize];
        if self.frames.len() as u32 >= self.lim.depth {
            let m = format!("Zu viele Aufrufe ineinander (mehr als {}) – endet die Rekursion?", self.lim.depth);
            return Err(Exc::Fatal(ErrorKind::Depth, m));
        }
        let base = fpos + 1;
        let want = p.params as usize;
        if base + p.max_stack as usize + 4 > self.lim.stack as usize {
            return Err(Exc::Fatal(ErrorKind::Stack, format!("Der Stapel ist voll (mehr als {} Werte)", self.lim.stack)));
        }
        if argc < want {
            self.stack.resize(base + want, Value::Nil);
        } else {
            self.stack.truncate(base + want);
        }
        self.frames.push(Frame { proto, closure: Some(r), base: base as u32, pc: 0 });
        Ok(())
    }

    /// call the value at stack[fpos] with the argc values above it: true when a frame was
    /// pushed, false when a native ran (its result replaced callee and arguments)
    fn call_at(&mut self, code: &Code, host: &mut dyn Host, fpos: usize, argc: usize) -> Result<bool, Exc> {
        match self.stack[fpos] {
            Value::Func(r) => {
                self.push_frame(code, r, fpos, argc)?;
                Ok(true)
            }
            Value::Native(i) => {
                self.call_native(host, i, fpos + 1, fpos)?;
                Ok(false)
            }
            v => msg(format!("{} ist keine Funktion", v.type_name())),
        }
    }

    /// run native i with the stack values from `from` on; its result replaces the stack from
    /// `to` on
    fn call_native(&mut self, host: &mut dyn Host, i: u32, from: usize, to: usize) -> Result<(), Exc> {
        let (name, b) = self.natives[i as usize].clone();
        let mut args = core::mem::take(&mut self.scratch);
        args.clear();
        args.extend_from_slice(&self.stack[from..]);
        let r = match b {
            Some(Builtin::Lib(f)) => self.lib_call(host, f, &args),
            Some(b) => self.builtin(b, &args),
            None => self.host_call(host, &name, &args, None),
        };
        args.clear();
        self.scratch = args;
        let v = r?;
        self.stack.truncate(to);
        self.stack.push(v);
        Ok(())
    }

    fn host_call(&mut self, host: &mut dyn Host, name: &str, args: &[Value], method: Option<(&str, &str)>) -> Result<Value, Exc> {
        let (mark, was) = (self.nroots.len(), self.in_native);
        self.in_native = true;
        self.fatal_kind = None;
        let r = host.call(&mut Cx { vm: self }, name, args);
        self.in_native = was;
        self.nroots.truncate(mark);
        if !matches!(r, Err(Fail::Throw(_) | Fail::Fatal(_))) {
            self.escaped = None;
        }
        match r {
            Ok(v) | Err(Fail::Throw(v)) if !self.valid(v) => {
                Err(Exc::Fatal(ErrorKind::Host, format!("{} gab einen ungültigen Wert zurück", name)))
            }
            Ok(v) => Ok(v),
            Err(Fail::Unknown) => msg(match method {
                Some(("map", m)) => format!("Die Map hat keine Funktion {}", m),
                Some((t, m)) => format!("{} hat keine Methode {}", t, m),
                None if stdlib::constant(name).is_some() => format!("{} ist keine Funktion", name),
                None => format!("Unbekannte Funktion {}", name),
            }),
            Err(Fail::Throw(v)) => Err(Exc::Throw(v)),
            Err(Fail::Fatal(m)) => Err(Exc::Fatal(self.fatal_kind.take().unwrap_or(ErrorKind::Host), m)),
        }
    }

    /// a function of the standard library: like a host function, with the VM itself
    fn lib_call(&mut self, host: &mut dyn Host, f: stdlib::LibFn, args: &[Value]) -> Result<Value, Exc> {
        let (mark, was) = (self.nroots.len(), self.in_native);
        self.in_native = true;
        let r = f(self, host, args);
        self.in_native = was;
        self.nroots.truncate(mark);
        if r.is_ok() {
            self.escaped = None;
        }
        r
    }

    /// obj.name(args) with obj and the argc arguments on the stack (name: constant k)
    fn method(&mut self, code: &Code, host: &mut dyn Host, k: u32, argc: usize) -> Result<bool, Exc> {
        let opos = self.stack.len() - argc - 1;
        let obj = self.stack[opos];
        let Const::Str(name) = &code.consts[k as usize] else { return internal() };
        if let Some(f) = self.map_get_str(obj, name) {
            if !matches!(f, Value::Func(_) | Value::Native(_)) {
                return msg(format!("Das Feld {} ist keine Funktion, sondern {}", name, f.type_name()));
            }
            self.stack[opos] = f;
            return self.call_at(code, host, opos, argc);
        }
        let v = match (obj, name.as_str()) {
            (Value::List(l), "push") => {
                self.reserve(16 * argc as u64)?;
                let items: Vec<Value> = self.stack[opos + 1..].to_vec();
                self.heap.update(l, |o| {
                    if let Obj::List(v) = o {
                        v.extend_from_slice(&items);
                    }
                });
                Value::Nil
            }
            (Value::List(l), "pop") => self.heap.update(l, |o| match o {
                Obj::List(v) => v.pop(),
                _ => None,
            }).unwrap_or(Value::Nil),
            _ => {
                let t = obj.type_name();
                let mut args = core::mem::take(&mut self.scratch);
                args.clear();
                args.extend_from_slice(&self.stack[opos..]);
                let r = match stdlib::lookup(t, name) {
                    Some(f) => self.lib_call(host, f, &args),
                    None => self.host_call(host, &format!("{}.{}", t, name), &args, Some((t, name))),
                };
                args.clear();
                self.scratch = args;
                r?
            }
        };
        self.stack.truncate(opos);
        self.stack.push(v);
        Ok(false)
    }

    // ---- built-in functions

    fn builtin(&mut self, b: Builtin, args: &[Value]) -> Result<Value, Exc> {
        let a = args.first().copied().unwrap_or(Value::Nil);
        match b {
            Builtin::Print => {
                let mut s = String::new();
                for (i, v) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    self.write_value(&mut s, *v, false, 0, &mut Vec::new());
                }
                s.push('\n');
                self.used += s.len() as u64 / 64;
                let room = (self.lim.output as usize).saturating_sub(self.out.len());
                if s.len() <= room {
                    self.out.push_str(&s);
                } else {
                    let mut cut = room;
                    while !s.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    self.out.push_str(&s[..cut]);
                }
                Ok(Value::Nil)
            }
            Builtin::Len => Ok(Value::Int(match a {
                Value::Str(r) => self.heap.str(r).chars as i64,
                Value::Bytes(r) => self.heap.bytes_of(r).len() as i64,
                Value::List(r) => self.heap.list(r).len() as i64,
                Value::Map(r) => self.heap.map(r).live as i64,
                v => return msg(format!("len braucht str, bytes, list oder map, nicht {}", v.type_name())),
            })),
            Builtin::Type => {
                let t = a.type_name();
                Ok(TYPES.iter().position(|x| *x == t).map_or(Value::Nil, |i| self.tnames[i]))
            }
            Builtin::Str => match a {
                Value::Str(_) => Ok(a),
                _ => {
                    let s = self.display(a);
                    self.used += s.len() as u64 / 64;
                    self.alloc_str(s)
                }
            },
            Builtin::Int => match a {
                Value::Int(_) => Ok(a),
                Value::Bool(b) => Ok(Value::Int(b as i64)),
                Value::Float(f) => match num::to_i64(f) {
                    Some(i) => Ok(Value::Int(i)),
                    None => msg(format!("int(…): {} passt nicht in int", self.display(a))),
                },
                Value::Str(r) => {
                    let s = heap::text(&self.heap, r);
                    match stdlib::parse_int(s, None) {
                        Some(i) => Ok(Value::Int(i)),
                        None => msg(format!("int(…): \"{}\" ist keine ganze Zahl", s)),
                    }
                }
                v => msg(format!("int(…) braucht eine Zahl oder einen Text, nicht {}", v.type_name())),
            },
            Builtin::Float => match a {
                Value::Float(_) => Ok(a),
                Value::Int(i) => Ok(Value::Float(num::from_i64(i))),
                Value::Str(r) => {
                    let s = heap::text(&self.heap, r);
                    match s.trim().parse::<f64>() {
                        Ok(f) => Ok(Value::Float(if num::is_nan(f) { num::NAN } else { f })),
                        Err(_) => msg(format!("float(…): \"{}\" ist keine Zahl", s)),
                    }
                }
                v => msg(format!("float(…) braucht eine Zahl oder einen Text, nicht {}", v.type_name())),
            },
            // call_native runs these with the host
            Builtin::Lib(_) => internal(),
        }
    }

    // ---- the instruction loop

    fn exec(&mut self, code: &Code, host: &mut dyn Host, fuel: u64) -> Result<Status, Exc> {
        'frame: loop {
            let fi = self.frames.len() - 1;
            let fr = self.frames[fi];
            let p = &code.protos[fr.proto as usize];
            let ops = &p.code[..];
            let base = fr.base as usize;
            let mut pc = fr.pc as usize;
            loop {
                if self.used >= fuel {
                    self.frames[fi].pc = pc as u32;
                    return Ok(Status::Yielded);
                }
                if self.call_fuel + self.used >= self.lim.fuel {
                    self.frames[fi].pc = pc as u32 + 1;
                    return Err(Exc::Fatal(ErrorKind::Fuel, fuel_msg(self.lim.fuel)));
                }
                let op = ops[pc];
                pc += 1;
                self.used += 1;
                let r: Result<(), Exc> = match op {
                    Op::Nil => {
                        self.stack.push(Value::Nil);
                        Ok(())
                    }
                    Op::True => {
                        self.stack.push(Value::Bool(true));
                        Ok(())
                    }
                    Op::False => {
                        self.stack.push(Value::Bool(false));
                        Ok(())
                    }
                    Op::Int(i) => {
                        self.stack.push(Value::Int(i as i64));
                        Ok(())
                    }
                    Op::Const(k) => {
                        self.stack.push(self.consts[k as usize]);
                        Ok(())
                    }
                    Op::Reserve(n) => {
                        self.stack.resize(self.stack.len() + n as usize, Value::Nil);
                        Ok(())
                    }
                    Op::Pop => {
                        self.stack.pop();
                        Ok(())
                    }
                    Op::PopN(n) => {
                        self.stack.truncate(self.stack.len() - n as usize);
                        Ok(())
                    }
                    Op::Dup => {
                        self.stack.push(self.stack[self.stack.len() - 1]);
                        Ok(())
                    }
                    Op::Dup2 => {
                        let n = self.stack.len();
                        self.stack.extend_from_within(n - 2..n);
                        Ok(())
                    }
                    Op::GetLocal(s) => {
                        self.stack.push(self.stack[base + s as usize]);
                        Ok(())
                    }
                    Op::SetLocal(s) => {
                        let v = self.stack.pop().unwrap_or(Value::Nil);
                        self.stack[base + s as usize] = v;
                        Ok(())
                    }
                    Op::GetCell(s) => match self.stack[base + s as usize] {
                        Value::Cell(c) => {
                            self.stack.push(self.heap.cell(c));
                            Ok(())
                        }
                        _ => internal(),
                    },
                    Op::SetCell(s) => {
                        let v = self.stack.pop().unwrap_or(Value::Nil);
                        match self.stack[base + s as usize] {
                            Value::Cell(c) => {
                                self.heap.set_cell(c, v);
                                Ok(())
                            }
                            _ => internal(),
                        }
                    }
                    Op::NewCell(s) => self.alloc(Obj::Cell(Value::Nil)).map(|c| self.stack[base + s as usize] = Value::Cell(c)),
                    Op::BoxLocal(s) => {
                        let v = self.stack[base + s as usize];
                        self.alloc(Obj::Cell(v)).map(|c| self.stack[base + s as usize] = Value::Cell(c))
                    }
                    Op::GetUp(i) => match fr.closure {
                        Some(f) => {
                            let c = self.heap.closure(f).ups[i as usize];
                            self.stack.push(self.heap.cell(c));
                            Ok(())
                        }
                        None => internal(),
                    },
                    Op::SetUp(i) => match fr.closure {
                        Some(f) => {
                            let v = self.stack.pop().unwrap_or(Value::Nil);
                            let c = self.heap.closure(f).ups[i as usize];
                            self.heap.set_cell(c, v);
                            Ok(())
                        }
                        None => internal(),
                    },
                    Op::GetGlobal(g) => {
                        self.stack.push(self.globals[g as usize]);
                        Ok(())
                    }
                    Op::SetGlobal(g) => {
                        self.globals[g as usize] = self.stack.pop().unwrap_or(Value::Nil);
                        Ok(())
                    }
                    Op::GetHost(h) => match self.hostv[h as usize] {
                        Some(v) => {
                            self.stack.push(v);
                            Ok(())
                        }
                        None => self.host_value(code, host, h),
                    },
                    Op::GetHostField(h) => match code.hosts[h as usize].dotted {
                        Some((b, k)) => match self.hostv[b as usize] {
                            Some(v) => {
                                self.stack.push(v);
                                self.get_field(k)
                            }
                            None => self.host_value(code, host, h),
                        },
                        None => internal(),
                    },
                    Op::AssignHost(h) => {
                        let n = &code.hosts[h as usize].name;
                        msg(format!("{} ist nicht deklariert – lege die Variable mit let {} = … an", n, n))
                    }
                    Op::Closure(i) => self.closure(code, i, base, fr.closure),
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => self.arith(op),
                    Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => self.compare(op),
                    Op::Neg => {
                        let n = self.stack.len() - 1;
                        match self.stack[n] {
                            Value::Int(i) => {
                                self.stack[n] = Value::Int(i.wrapping_neg());
                                Ok(())
                            }
                            Value::Float(f) => {
                                self.stack[n] = Value::Float(num::neg(f));
                                Ok(())
                            }
                            v => msg(format!("Das Minus passt nicht zu {}", v.type_name())),
                        }
                    }
                    Op::Not => {
                        let n = self.stack.len() - 1;
                        self.stack[n] = Value::Bool(!self.stack[n].truthy());
                        Ok(())
                    }
                    Op::And(t) | Op::Or(t) | Op::Coalesce(t) => {
                        let v = self.stack[self.stack.len() - 1];
                        let keep = match op {
                            Op::And(_) => !v.truthy(),
                            Op::Or(_) => v.truthy(),
                            _ => v != Value::Nil,
                        };
                        if keep {
                            pc = t as usize;
                        } else {
                            self.stack.pop();
                        }
                        Ok(())
                    }
                    Op::Jump(t) => {
                        pc = t as usize;
                        Ok(())
                    }
                    Op::JumpIfFalse(t) => {
                        if !self.stack.pop().unwrap_or(Value::Nil).truthy() {
                            pc = t as usize;
                        }
                        Ok(())
                    }
                    Op::Concat(n) => self.concat(n as usize),
                    Op::MakeList(n) => {
                        let start = self.stack.len() - n as usize;
                        let items = self.stack[start..].to_vec();
                        self.used += n as u64 / 8;
                        self.alloc(Obj::List(items)).map(|r| {
                            self.stack.truncate(start);
                            self.stack.push(Value::List(r));
                        })
                    }
                    Op::MakeMap(n) => self.make_map(n as usize),
                    Op::GetField(k) => self.get_field(k),
                    Op::SetField(k) => self.set_field(k),
                    Op::GetIndex => self.get_index(),
                    Op::SetIndex => self.set_index(),
                    Op::Call(argc) => {
                        self.frames[fi].pc = pc as u32;
                        let fpos = self.stack.len() - argc as usize - 1;
                        match self.call_at(code, host, fpos, argc as usize) {
                            Ok(true) => continue 'frame,
                            Ok(false) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    Op::CallHost(h, argc) => {
                        self.frames[fi].pc = pc as u32;
                        let at = self.stack.len() - argc as usize;
                        let r = match self.hostv[h as usize] {
                            Some(v) => {
                                self.stack.insert(at, v);
                                self.call_at(code, host, at, argc as usize)
                            }
                            None => self.call_native(host, h, at, at).map(|_| false),
                        };
                        match r {
                            Ok(true) => continue 'frame,
                            Ok(false) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    Op::CallMethod(k, argc) => {
                        self.frames[fi].pc = pc as u32;
                        match self.method(code, host, k, argc as usize) {
                            Ok(true) => continue 'frame,
                            Ok(false) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    Op::CallHostMethod(h, argc) => {
                        self.frames[fi].pc = pc as u32;
                        let at = self.stack.len() - argc as usize;
                        let r = match code.hosts[h as usize].dotted {
                            Some((b, k)) => match self.hostv[b as usize] {
                                Some(v) => {
                                    self.stack.insert(at, v);
                                    self.method(code, host, k, argc as usize)
                                }
                                None => self.call_native(host, h, at, at).map(|_| false),
                            },
                            None => internal(),
                        };
                        match r {
                            Ok(true) => continue 'frame,
                            Ok(false) => Ok(()),
                            Err(e) => Err(e),
                        }
                    }
                    Op::Return => {
                        let v = self.stack.pop().unwrap_or(Value::Nil);
                        let f = self.frames.pop().expect("frame");
                        self.stack.truncate(f.base as usize - 1);
                        if self.frames.len() <= self.floor {
                            return Ok(Status::Done(v));
                        }
                        self.stack.push(v);
                        continue 'frame;
                    }
                    Op::Throw => Err(Exc::Throw(self.stack.pop().unwrap_or(Value::Nil))),
                    Op::IterPrep(b) => self.iter_prep(base + b as usize),
                    Op::IterNext(b, t) | Op::IterNext2(b, t) => match self.iter_next(base + b as usize, matches!(op, Op::IterNext2(..))) {
                        Ok(true) => Ok(()),
                        Ok(false) => {
                            pc = t as usize;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    },
                    Op::RangePrep(b) => {
                        let at = base + b as usize;
                        match (self.stack[at], self.stack[at + 1]) {
                            (Value::Int(_), Value::Int(_)) => Ok(()),
                            (Value::Int(_), v) | (v, _) => msg(format!("Ein Bereich a..b braucht ganze Zahlen, nicht {}", v.type_name())),
                        }
                    }
                    Op::RangeNext(b, t) => {
                        let at = base + b as usize;
                        match (self.stack[at], self.stack[at + 1]) {
                            (Value::Int(i), Value::Int(hi)) if i < hi => {
                                self.stack[at] = Value::Int(i + 1);
                                self.stack.push(Value::Int(i));
                            }
                            _ => pc = t as usize,
                        }
                        Ok(())
                    }
                };
                if let Err(e) = r {
                    self.frames[fi].pc = pc as u32;
                    return Err(e);
                }
                if self.yield_req {
                    self.yield_req = false;
                    self.frames[fi].pc = pc as u32;
                    return Ok(Status::Yielded);
                }
            }
        }
    }

    // ---- instructions with more work

    /// a host name read as a value: a native function value when it is built in or the host
    /// knows it
    fn host_value(&mut self, code: &Code, host: &mut dyn Host, h: u32) -> Result<(), Exc> {
        let name = &code.hosts[h as usize].name;
        if let Some(v) = stdlib::constant(name) {
            self.stack.push(v);
            return Ok(());
        }
        if self.natives[h as usize].1.is_none() && !host.knows(name) {
            return msg(format!("Unbekannter Name {}", name));
        }
        self.stack.push(Value::Native(h));
        Ok(())
    }

    fn closure(&mut self, code: &Code, i: u32, base: usize, cur: Option<Ref>) -> Result<(), Exc> {
        let p = &code.protos[i as usize];
        let mut ups = Vec::with_capacity(p.ups.len());
        for u in &p.ups {
            let c = match *u {
                Up::Local(s) => match self.stack[base + s as usize] {
                    Value::Cell(c) => c,
                    _ => return internal(),
                },
                Up::Up(n) => match cur {
                    Some(f) => self.heap.closure(f).ups[n as usize],
                    None => return internal(),
                },
            };
            ups.push(c);
        }
        let r = self.alloc(Obj::Func(Closure { proto: i, ups: ups.into_boxed_slice() }))?;
        self.stack.push(Value::Func(r));
        Ok(())
    }

    fn arith(&mut self, op: Op) -> Result<(), Exc> {
        let n = self.stack.len();
        let (a, b) = (self.stack[n - 2], self.stack[n - 1]);
        let v = match (a, b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(match op {
                Op::Add => x.wrapping_add(y),
                Op::Sub => x.wrapping_sub(y),
                Op::Mul => x.wrapping_mul(y),
                Op::Div | Op::Rem if y == 0 => return msg(String::from("Division durch 0")),
                Op::Div => x.wrapping_div(y),
                _ => x.wrapping_rem(y),
            }),
            (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
                let (x, y) = (to_float(a), to_float(b));
                Value::Float(match op {
                    Op::Add => num::add(x, y),
                    Op::Sub => num::sub(x, y),
                    Op::Mul => num::mul(x, y),
                    Op::Div => num::div(x, y),
                    _ => num::rem(x, y),
                })
            }
            (Value::Str(x), Value::Str(y)) if op == Op::Add => {
                let (p, q) = (heap::text(&self.heap, x), heap::text(&self.heap, y));
                let mut s = String::with_capacity(p.len() + q.len());
                s.push_str(p);
                s.push_str(q);
                self.used += s.len() as u64 / 64;
                self.alloc_str(s)?
            }
            (Value::List(x), Value::List(y)) if op == Op::Add => {
                let mut v = self.heap.list(x).clone();
                v.extend_from_slice(self.heap.list(y));
                self.used += v.len() as u64 / 8;
                Value::List(self.alloc(Obj::List(v))?)
            }
            (Value::Bytes(x), Value::Bytes(y)) if op == Op::Add => {
                let mut v = self.heap.bytes_of(x).to_vec();
                v.extend_from_slice(self.heap.bytes_of(y));
                self.used += v.len() as u64 / 64;
                Value::Bytes(self.alloc(Obj::Bytes(v.into_boxed_slice()))?)
            }
            _ => {
                let t = op_text(op);
                return msg(format!("Der Operator {} passt nicht zu {} und {}", t, a.type_name(), b.type_name()));
            }
        };
        self.stack.truncate(n - 2);
        self.stack.push(v);
        Ok(())
    }

    fn compare(&mut self, op: Op) -> Result<(), Exc> {
        let n = self.stack.len();
        let (a, b) = (self.stack[n - 2], self.stack[n - 1]);
        let r = match op {
            Op::Eq => self.same(a, b),
            Op::Ne => !self.same(a, b),
            _ => {
                let ord = match (a, b) {
                    (Value::Int(x), Value::Int(y)) => Some(x.cmp(&y)),
                    (Value::Int(x), Value::Float(y)) => num::cmp_int(x, y),
                    (Value::Float(x), Value::Int(y)) => num::cmp_int(y, x).map(Ordering::reverse),
                    (Value::Float(x), Value::Float(y)) => num::cmp(x, y),
                    (Value::Str(x), Value::Str(y)) => Some(heap::text(&self.heap, x).cmp(heap::text(&self.heap, y))),
                    _ => {
                        let t = op_text(op);
                        return msg(format!("Der Vergleich {} passt nicht zu {} und {}", t, a.type_name(), b.type_name()));
                    }
                };
                match (op, ord) {
                    (_, None) => false,
                    (Op::Lt, Some(o)) => o == Ordering::Less,
                    (Op::Le, Some(o)) => o != Ordering::Greater,
                    (Op::Gt, Some(o)) => o == Ordering::Greater,
                    (_, Some(o)) => o != Ordering::Less,
                }
            }
        };
        self.stack.truncate(n - 2);
        self.stack.push(Value::Bool(r));
        Ok(())
    }

    fn concat(&mut self, n: usize) -> Result<(), Exc> {
        let start = self.stack.len() - n;
        let mut s = String::new();
        for i in start..self.stack.len() {
            match self.stack[i] {
                Value::Str(r) => s.push_str(heap::text(&self.heap, r)),
                v => self.write_value(&mut s, v, false, 0, &mut Vec::new()),
            }
        }
        self.used += s.len() as u64 / 64;
        let v = self.alloc_str(s)?;
        self.stack.truncate(start);
        self.stack.push(v);
        Ok(())
    }

    fn make_map(&mut self, n: usize) -> Result<(), Exc> {
        let start = self.stack.len() - 2 * n;
        let mut m = Map::default();
        for i in 0..n {
            let (k, v) = (self.stack[start + 2 * i], self.stack[start + 2 * i + 1]);
            let Some(key) = self.heap.key(k) else { return key_error(k) };
            match m.find(&self.heap.objs, key) {
                Some(p) => m.entries[p].val = v,
                None => m.push(k, key.hash(), v),
            }
        }
        self.used += n as u64 / 4;
        let r = self.alloc(Obj::Map(alloc::boxed::Box::new(m)))?;
        self.stack.truncate(start);
        self.stack.push(Value::Map(r));
        Ok(())
    }

    /// the top of the stack replaced by its field (constant k)
    fn get_field(&mut self, k: u32) -> Result<(), Exc> {
        let n = self.stack.len() - 1;
        let Value::Str(kr) = self.consts[k as usize] else { return internal() };
        match self.stack[n] {
            Value::Map(m) => {
                let s = self.heap.str(kr);
                let v = self.heap.map_find(m, Key::Str(&s.s, s.hash)).map_or(Value::Nil, |i| self.heap.map(m).entries[i].val);
                self.stack[n] = v;
                Ok(())
            }
            v => msg(format!("{} hat kein Feld {}", v.type_name(), heap::text(&self.heap, kr))),
        }
    }

    fn set_field(&mut self, k: u32) -> Result<(), Exc> {
        let n = self.stack.len();
        let (obj, v) = (self.stack[n - 2], self.stack[n - 1]);
        let key = self.consts[k as usize];
        match obj {
            Value::Map(_) => self.set_key(obj, key, v)?,
            _ => {
                let Value::Str(kr) = key else { return internal() };
                return msg(format!("Nur Maps haben Felder, nicht {} (Feld {})", obj.type_name(), heap::text(&self.heap, kr)));
            }
        }
        self.stack.truncate(n - 2);
        Ok(())
    }

    /// m[k] = v for a map m
    fn set_key(&mut self, m: Value, k: Value, v: Value) -> Result<(), Exc> {
        let Value::Map(r) = m else { return msg(format!("{} ist keine Map", m.type_name())) };
        if self.heap.key(k).is_none() {
            return key_error(k);
        }
        self.reserve(48)?;
        self.heap.map_set(r, k, v);
        if self.heap.bytes > self.lim.heap {
            return Err(Exc::Fatal(ErrorKind::Heap, format!("Speichergrenze erreicht ({} Bytes)", self.lim.heap)));
        }
        Ok(())
    }

    fn get_index(&mut self) -> Result<(), Exc> {
        let n = self.stack.len();
        let (obj, idx) = (self.stack[n - 2], self.stack[n - 1]);
        let v = match (obj, idx) {
            (Value::List(l), Value::Int(i)) => {
                let items = self.heap.list(l);
                match usize::try_from(i).ok().and_then(|i| items.get(i)) {
                    Some(v) => *v,
                    None => return msg(format!("Index {} liegt außerhalb der Liste (Länge {})", i, items.len())),
                }
            }
            (Value::Map(m), k) => {
                let Some(key) = self.heap.key(k) else { return key_error(k) };
                self.heap.map_find(m, key).map_or(Value::Nil, |i| self.heap.map(m).entries[i].val)
            }
            (Value::Str(s), Value::Int(i)) => {
                let so = self.heap.str(s);
                let (c, chars, ascii) = (heap::char_at(so, i), so.chars, so.chars as usize == so.s.len());
                if !ascii {
                    self.used += i.max(0) as u64 / 64;
                }
                match c {
                    Some(c) => {
                        let mut t = String::new();
                        t.push(c);
                        self.alloc_str(t)?
                    }
                    None => return msg(format!("Index {} liegt außerhalb des Textes (Länge {})", i, chars)),
                }
            }
            (Value::Bytes(b), Value::Int(i)) => {
                let bs = self.heap.bytes_of(b);
                match usize::try_from(i).ok().and_then(|i| bs.get(i)) {
                    Some(x) => Value::Int(*x as i64),
                    None => return msg(format!("Index {} liegt außerhalb der Bytes (Länge {})", i, bs.len())),
                }
            }
            (Value::List(_) | Value::Str(_) | Value::Bytes(_), i) => {
                return msg(format!("Ein Index von {} muss int sein, nicht {}", obj.type_name(), i.type_name()));
            }
            _ => return msg(format!("{} lässt sich nicht indizieren", obj.type_name())),
        };
        self.stack.truncate(n - 2);
        self.stack.push(v);
        Ok(())
    }

    fn set_index(&mut self) -> Result<(), Exc> {
        let n = self.stack.len();
        let (obj, idx, v) = (self.stack[n - 3], self.stack[n - 2], self.stack[n - 1]);
        match (obj, idx) {
            (Value::List(l), Value::Int(i)) => {
                let len = self.heap.list(l).len();
                match usize::try_from(i) {
                    Ok(i) if i < len => {
                        if let Obj::List(items) = self.heap.get_mut(l) {
                            items[i] = v;
                        }
                    }
                    _ => return msg(format!("Index {} liegt außerhalb der Liste (Länge {})", i, len)),
                }
            }
            (Value::List(_), i) => return msg(format!("Ein Index von list muss int sein, nicht {}", i.type_name())),
            (Value::Map(_), k) => self.set_key(obj, k, v)?,
            (Value::Str(_) | Value::Bytes(_), _) => return msg(format!("{} lässt sich nicht ändern", obj.type_name())),
            _ => return msg(format!("{} lässt sich nicht indizieren", obj.type_name())),
        }
        self.stack.truncate(n - 3);
        Ok(())
    }

    /// slots at, at + 1, at + 2: the value, its key snapshot (maps) or char index (str), the
    /// position
    fn iter_prep(&mut self, at: usize) -> Result<(), Exc> {
        match self.stack[at] {
            Value::List(_) | Value::Str(_) | Value::Bytes(_) => {}
            Value::Map(m) => {
                let keys: Vec<Value> = self.heap.map(m).iter().map(|e| e.key).collect();
                self.used += keys.len() as u64 / 8;
                let l = self.alloc(Obj::List(keys))?;
                self.stack[at + 1] = Value::List(l);
            }
            v => return msg(format!("for braucht list, map, str oder bytes, nicht {}", v.type_name())),
        }
        if !matches!(self.stack[at], Value::Map(_)) {
            self.stack[at + 1] = Value::Int(0);
        }
        self.stack[at + 2] = Value::Int(0);
        Ok(())
    }

    /// push the next item (two: index or key first); false at the end
    fn iter_next(&mut self, at: usize, two: bool) -> Result<bool, Exc> {
        let Value::Int(pos) = self.stack[at + 2] else { return internal() };
        let pos = pos as usize;
        let (first, item) = match self.stack[at] {
            Value::List(l) => match self.heap.list(l).get(pos) {
                Some(v) => (Value::Int(pos as i64), *v),
                None => return Ok(false),
            },
            Value::Bytes(b) => match self.heap.bytes_of(b).get(pos) {
                Some(x) => (Value::Int(pos as i64), Value::Int(*x as i64)),
                None => return Ok(false),
            },
            Value::Map(m) => {
                let Value::List(keys) = self.stack[at + 1] else { return internal() };
                let mut p = pos;
                let found = loop {
                    let Some(&k) = self.heap.list(keys).get(p) else { break None };
                    p += 1;
                    let i = self.heap.key(k).and_then(|key| self.heap.map_find(m, key));
                    if let Some(i) = i {
                        break Some((k, self.heap.map(m).entries[i].val));
                    }
                };
                self.stack[at + 2] = Value::Int(p as i64);
                return match found {
                    Some((k, v)) => {
                        self.stack.push(k);
                        if two {
                            self.stack.push(v);
                        }
                        Ok(true)
                    }
                    None => Ok(false),
                };
            }
            Value::Str(s) => {
                let Value::Int(ci) = self.stack[at + 1] else { return internal() };
                let Some(c) = heap::text(&self.heap, s)[pos..].chars().next() else { return Ok(false) };
                let mut t = String::new();
                t.push(c);
                let v = self.alloc_str(t)?;
                self.stack[at + 1] = Value::Int(ci + 1);
                self.stack[at + 2] = Value::Int((pos + c.len_utf8()) as i64);
                if two {
                    self.stack.push(Value::Int(ci));
                }
                self.stack.push(v);
                return Ok(true);
            }
            _ => return internal(),
        };
        self.stack[at + 2] = Value::Int(pos as i64 + 1);
        if two {
            self.stack.push(first);
        }
        self.stack.push(item);
        Ok(true)
    }

    // ---- text of values

    pub(crate) fn write_value(&self, o: &mut String, v: Value, quoted: bool, depth: u32, seen: &mut Vec<Ref>) {
        match v {
            Value::Nil => o.push_str("nil"),
            Value::Bool(b) => o.push_str(if b { "true" } else { "false" }),
            Value::Int(i) => o.push_str(&format!("{}", i)),
            Value::Float(f) => num::fmt(f, o),
            Value::Str(r) => {
                let s = heap::text(&self.heap, r);
                if quoted { quote(o, s) } else { o.push_str(s) }
            }
            Value::Bytes(r) => {
                let b = self.heap.bytes_of(r);
                o.push_str("bytes(");
                for (i, x) in b.iter().take(64).enumerate() {
                    if i > 0 {
                        o.push(' ');
                    }
                    o.push_str(&format!("{:02x}", x));
                }
                if b.len() > 64 {
                    o.push_str(" …");
                }
                o.push(')');
            }
            Value::List(r) => {
                if depth >= 32 || seen.contains(&r) {
                    o.push_str("[…]");
                    return;
                }
                seen.push(r);
                o.push('[');
                for (i, x) in self.heap.list(r).iter().enumerate() {
                    if i > 0 {
                        o.push_str(", ");
                    }
                    self.write_value(o, *x, true, depth + 1, seen);
                }
                o.push(']');
                seen.pop();
            }
            Value::Map(r) => {
                if depth >= 32 || seen.contains(&r) {
                    o.push_str("{…}");
                    return;
                }
                seen.push(r);
                o.push('{');
                for (i, e) in self.heap.map(r).iter().enumerate() {
                    if i > 0 {
                        o.push_str(", ");
                    }
                    match e.key {
                        Value::Str(k) if is_name(heap::text(&self.heap, k)) => o.push_str(heap::text(&self.heap, k)),
                        k => self.write_value(o, k, true, depth + 1, seen),
                    }
                    o.push_str(": ");
                    self.write_value(o, e.val, true, depth + 1, seen);
                }
                o.push('}');
                seen.pop();
            }
            Value::Func(r) => {
                let p = &self.code.protos[self.heap.closure(r).proto as usize];
                if p.name == "fn" {
                    o.push_str("<fn>");
                } else {
                    o.push_str("<fn ");
                    o.push_str(&p.name);
                    o.push('>');
                }
            }
            Value::Native(i) => {
                o.push_str("<fn ");
                o.push_str(self.natives.get(i as usize).map_or("?", |n| &n.0));
                o.push('>');
            }
            Value::Cell(_) => o.push_str("<cell>"),
        }
    }
}

/// the context of a native function: values, allocation, fuel, yielding
pub struct Cx<'a> {
    vm: &'a mut Vm,
}

impl Cx<'_> {
    /// the text of a str (Vm::str)
    pub fn str(&self, v: Value) -> Option<&str> {
        self.vm.str(v)
    }

    /// the content of a bytes value
    pub fn bytes(&self, v: Value) -> Option<&[u8]> {
        self.vm.bytes(v)
    }

    /// the items of a list
    pub fn list(&self, v: Value) -> Option<&[Value]> {
        self.vm.list(v)
    }

    /// m[key]
    pub fn map_get(&self, m: Value, key: Value) -> Option<Value> {
        self.vm.map_get(m, key)
    }

    /// m.key
    pub fn map_get_str(&self, m: Value, key: &str) -> Option<Value> {
        self.vm.map_get_str(m, key)
    }

    /// the entries of a map in order
    pub fn map_entries(&self, m: Value) -> Vec<(Value, Value)> {
        self.vm.map_entries(m)
    }

    /// the value as print() shows it
    pub fn display(&self, v: Value) -> String {
        self.vm.display(v)
    }

    /// == of the script
    pub fn equal(&self, a: Value, b: Value) -> bool {
        self.vm.equal(a, b)
    }

    /// a new str (alive until the native returns; return it or store it in the script's data)
    pub fn new_str(&mut self, s: &str) -> Result<Value, Fail> {
        let r = self.vm.new_str(s);
        self.lift(r)
    }

    /// a new bytes value
    pub fn new_bytes(&mut self, b: &[u8]) -> Result<Value, Fail> {
        let r = self.vm.new_bytes(b);
        self.lift(r)
    }

    /// a new list of these items
    pub fn new_list(&mut self, items: Vec<Value>) -> Result<Value, Fail> {
        let r = self.vm.new_list(items);
        self.lift(r)
    }

    /// a new empty map
    pub fn new_map(&mut self) -> Result<Value, Fail> {
        let r = self.vm.new_map();
        self.lift(r)
    }

    /// m[k] = v (k: str, int or bool)
    pub fn map_set(&mut self, m: Value, k: Value, v: Value) -> Result<(), Fail> {
        let r = self.vm.map_set(m, k, v);
        self.lift(r)
    }

    /// remove m[k]; the old value
    pub fn map_remove(&mut self, m: Value, k: Value) -> Option<Value> {
        self.vm.map_remove(m, k)
    }

    /// append v to the list l
    pub fn list_push(&mut self, l: Value, v: Value) -> Result<(), Fail> {
        let r = self.vm.list_push(l, v);
        self.lift(r)
    }

    /// l[i] = v (false when l is no list or i is out of range)
    pub fn list_set(&mut self, l: Value, i: usize, v: Value) -> bool {
        self.vm.list_set(l, i, v)
    }

    /// replace the items of a list
    pub fn list_replace(&mut self, l: Value, items: Vec<Value>) -> Result<(), Fail> {
        let r = self.vm.list_replace(l, items);
        self.lift(r)
    }

    /// a native function value (for maps of functions the host returns)
    pub fn native(&mut self, name: &str) -> Value {
        self.vm.native(name)
    }

    /// call a function value (a callback the script passed) to its end and return its result;
    /// Err(Fail::Throw(v)) when it throws (return it to pass the exception on).  Pass the host
    /// itself (`cx.call(self, f, &args)`): the callback may call natives too.  It cannot yield
    /// in between (a yield request waits until the native returns); fuel and the limits count
    /// as usual, and calls nest at most 20 deep.
    pub fn call(&mut self, host: &mut dyn Host, f: Value, args: &[Value]) -> Result<Value, Fail> {
        self.vm.nested(host, f, args)
    }

    /// an exception with a message, for `return Err(cx.error("…"))`
    pub fn error(&mut self, msg: &str) -> Fail {
        match self.vm.new_str(msg) {
            Ok(v) => Fail::Throw(v),
            Err(e) => self.lift::<()>(Err(e)).unwrap_err(),
        }
    }

    /// add fuel for the work of this call (the call itself costs 1); spend() also stops at the
    /// fuel limit
    pub fn charge(&mut self, fuel: u64) {
        self.vm.used += fuel;
    }

    /// the fuel of the current invocation so far, over all its runs and this call included:
    /// the moment of its CPU time a host gives what the native does (JFNetworker's scripts)
    pub fn fuel(&self) -> u64 {
        self.vm.call_fuel + self.vm.used
    }

    /// charge fuel for work and end the invocation with ErrorKind::Fuel once Limits::fuel is
    /// used up: a long operation calls this as it goes, so a huge job stops in the middle
    pub fn spend(&mut self, fuel: u64) -> Result<(), Fail> {
        let r = stdlib::spend(self.vm, fuel);
        r.map_err(|e| self.fail(e))
    }

    /// keep v alive until the native returns, for values it holds while it calls back (the
    /// callback may change the list they came from, and the collector may run)
    pub fn keep(&mut self, v: Value) {
        if self.vm.valid(v) {
            stdlib::keep(self.vm, v);
        }
    }

    /// v as JSON text like json.encode (indent 0: compact, else spaces per level); data that
    /// JSON cannot hold (nan, bytes, functions, cycles) throws
    pub fn json_encode(&mut self, v: Value, indent: u32) -> Result<String, Fail> {
        if !self.vm.valid(v) {
            return Err(Fail::Fatal(invalid().msg));
        }
        let r = jsonlib::encode(self.vm, v, indent);
        r.map_err(|e| self.fail(e))
    }

    /// the value of a JSON text like json.decode (maps in the order of the members); a syntax
    /// error throws "json.decode: … (Zeile 1, Spalte 5)"
    pub fn json_decode(&mut self, text: &str) -> Result<Value, Fail> {
        let r = jsonlib::decode(self.vm, text.as_bytes());
        r.map_err(|e| self.fail(e))
    }

    /// a fatal Error as Fail::Fatal; the kind (heap, fuel) goes along for the Error it becomes
    fn lift<T>(&mut self, r: Result<T, Error>) -> Result<T, Fail> {
        r.map_err(|e| {
            if e.kind != ErrorKind::Uncaught {
                self.vm.fatal_kind = Some(e.kind);
            }
            Fail::Fatal(e.msg)
        })
    }

    /// an exception of the VM as Fail: a runtime error throws its message
    fn fail(&mut self, e: Exc) -> Fail {
        match e {
            Exc::Msg(m) => self.error(&m),
            Exc::Throw(v) => Fail::Throw(v),
            Exc::Fatal(kind, m) => {
                self.vm.fatal_kind = Some(kind);
                Fail::Fatal(m)
            }
        }
    }

    /// end the run after this call returns (Status::Yielded; run() continues after the call)
    pub fn yield_now(&mut self) {
        self.vm.yield_req = true;
    }

    /// add text to the print output
    pub fn print(&mut self, s: &str) {
        let room = (self.vm.lim.output as usize).saturating_sub(self.vm.out.len());
        let mut cut = s.len().min(room);
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.vm.out.push_str(&s[..cut]);
    }

    /// keep a value alive beyond this call (a callback, say); see Vm::pin
    pub fn pin(&mut self, v: Value) -> Pin {
        self.vm.pin(v)
    }

    /// the pinned value
    pub fn pinned(&self, p: Pin) -> Value {
        self.vm.pinned(p)
    }

    /// let the collector have the value again
    pub fn unpin(&mut self, p: Pin) {
        self.vm.unpin(p)
    }
}

fn usage(msg: String) -> Error {
    Error { kind: ErrorKind::Usage, msg, line: 0, col: 0, trace: Vec::new() }
}

fn invalid() -> Error {
    usage(String::from("Ungültiger Wert: von einer anderen VM, oder freigegeben, weil er nicht gepinnt war"))
}

fn plain(e: Exc) -> Error {
    match e {
        Exc::Msg(m) => Error { kind: ErrorKind::Uncaught, msg: m, line: 0, col: 0, trace: Vec::new() },
        Exc::Throw(_) => usage(String::from("Ausnahme")),
        Exc::Fatal(kind, m) => Error { kind, msg: m, line: 0, col: 0, trace: Vec::new() },
    }
}

fn key_error<T>(k: Value) -> Result<T, Exc> {
    msg(format!("Ein Map-Schlüssel muss str, int oder bool sein, nicht {}", k.type_name()))
}

fn to_float(v: Value) -> f64 {
    match v {
        Value::Int(i) => num::from_i64(i),
        Value::Float(f) => f,
        _ => num::NAN,
    }
}

fn op_text(op: Op) -> &'static str {
    match op {
        Op::Add => "+",
        Op::Sub => "-",
        Op::Mul => "*",
        Op::Div => "/",
        Op::Rem => "%",
        Op::Lt => "<",
        Op::Le => "<=",
        Op::Gt => ">",
        Op::Ge => ">=",
        Op::Eq => "==",
        _ => "!=",
    }
}

/// a map key that prints without quotes
fn is_name(s: &str) -> bool {
    let mut it = s.chars();
    it.next().is_some_and(|c| c.is_alphabetic() || c == '_') && it.all(|c| c.is_alphanumeric() || c == '_')
}

fn quote(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if c.is_control() => o.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}
