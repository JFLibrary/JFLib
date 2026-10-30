// module -- import(path): the modules of a script, read through the host
//
//   let m = import("lib/util.jfs")   the module's top-level names (fns and lets; a name that
//                                    starts with _ stays private) as a map, with the values
//                                    they have when the module's code has run
//   Host::load(cx, path)             the host's files: the source of a normalised path, or the
//                                    German reason (vm.rs; the runner reads the file system,
//                                    JFNetworker the device's files)
//   norm_path(from, path)            `path` resolved against the file `from` and normalised
//
// A module runs once per VM.  The first import of a path compiles the file the way the
// program was compiled (compile_named, or compile_with with the same host names), links its
// code behind the VM's own -- functions, constants and globals get the places after the ones
// there, host names are shared by name (so config holds for every module) -- and runs its
// top-level code as a frame of the importing invocation: with its fuel, limits, heap and
// output, so it yields, pauses and steps like any call, and its errors name its file.  When
// that code ends (its last Return becomes Op::EndModule, so exec's Return knows nothing of
// modules), import gives the map and keeps it; a later import of the same normalised
// path gives the same map at once.  The map is a snapshot: the module's functions see its
// globals as they change, the map's entries do not (export a function to read a changing
// value).  The program itself can be imported once its top-level code has run.
//
// Errors (each a str the script can catch):
//   "Zyklischer Import: a.jfs → b.jfs → a.jfs"   a module whose top-level code still runs
//   "Modul lib/x.jfs: die Datei gibt es nicht"   the host's reason
//   "lib/x.jfs:3:5: Hier fehlt …"                it does not compile; uncaught, the invocation
//                                                ends with the module's file, line and column
// A module whose top-level code throws (or is cut off) is forgotten: the next import runs it
// again.  At most MAX_UNITS modules per VM.
//
// Paths: "/" separates the parts (a "\" counts as one), "." and ".." are resolved; a path that
// does not start with "/" or a drive ("C:/") is relative to the directory of the importing
// file, and a file without a directory ("-e", an inline script) passes it on as it is: the
// host reads it relative to its working directory.  Fuel: 1 per 64 bytes of source on top of
// the call, then the module's code as it runs.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::code::{Code, Const, HostName, Op};
use crate::heap::{self, Obj, Value};
use crate::vm::{builtin_of, msg, Cx, ErrorKind, Exc, Frame, Host, TraceLine, Vm};
use crate::{Diagnostic, Program};

/// modules per VM at most (the program included)
pub(crate) const MAX_UNITS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UnitState {
    /// linked, its top-level code runs (a frame with closure None) or ran
    Ready,
    /// its top-level code threw: an import runs a new copy
    Failed,
}

/// the program (unit 0) or a module in a VM
#[derive(Clone, Debug)]
pub(crate) struct Unit {
    /// the normalised path (Program::name for the program)
    pub path: String,
    /// its globals: gbase .. gbase + gcount of Vm::globals and code.globals
    pub gbase: u32,
    pub gcount: u32,
    /// its top-level function in code.protos
    pub pbase: u32,
    pub state: UnitState,
    /// the map import gives, once its code has run
    pub map: Option<Value>,
}

/// see the module comment
pub(crate) fn norm_path(from: &str, path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let full = if absolute(&path) {
        path
    } else {
        let from = from.replace('\\', "/");
        match from.rfind('/') {
            Some(i) => format!("{}/{}", &from[..i], path),
            None => path,
        }
    };
    normalize(&full)
}

fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
}

/// without "." and "..", "/" between the parts; the root ("/", "C:/") stays
fn normalize(p: &str) -> String {
    let b = p.as_bytes();
    let (root, rest) = if p.starts_with('/') {
        ("/", &p[1..])
    } else if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        if b.get(2) == Some(&b'/') { (&p[..3], &p[3..]) } else { (&p[..2], &p[2..]) }
    } else {
        ("", p)
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&l) if l != ".." => {
                    parts.pop();
                }
                _ if !root.is_empty() => {}
                _ => parts.push(".."),
            },
            c => parts.push(c),
        }
    }
    format!("{}{}", root, parts.join("/"))
}

/// an instruction of a module moved to its places in the linked code
fn reloc(op: Op, pb: u32, kb: u32, gb: u32, h: &[u32]) -> Op {
    match op {
        Op::Const(k) => Op::Const(k + kb),
        Op::GetField(k) => Op::GetField(k + kb),
        Op::SetField(k) => Op::SetField(k + kb),
        Op::CallMethod(k, a) => Op::CallMethod(k + kb, a),
        Op::GetGlobal(g) => Op::GetGlobal(g + gb),
        Op::SetGlobal(g) => Op::SetGlobal(g + gb),
        Op::GetHost(x) => Op::GetHost(h[x as usize]),
        Op::GetHostField(x) => Op::GetHostField(h[x as usize]),
        Op::AssignHost(x) => Op::AssignHost(h[x as usize]),
        Op::CallHost(x, a) => Op::CallHost(h[x as usize], a),
        Op::CallHostMethod(x, a) => Op::CallHostMethod(h[x as usize], a),
        Op::Closure(p) => Op::Closure(p + pb),
        o => o,
    }
}

impl Vm {
    /// import(path) for the native at `to` (the arguments in args): Ok(false) with the map at
    /// `to`, Ok(true) when the module's top-level code is pushed as a frame (its return lands
    /// at `to`)
    #[cold]
    #[inline(never)]
    pub(crate) fn import(&mut self, host: &mut dyn Host, args: &[Value], to: usize) -> Result<bool, Exc> {
        let a = args.first().copied().unwrap_or(Value::Nil);
        let Value::Str(r) = a else {
            return msg(format!("import braucht den Pfad einer Datei (str), nicht {}", a.type_name()));
        };
        let want = String::from(heap::text(&self.heap, r));
        let here = self.frames.last().map_or(0, |f| self.code.protos[f.proto as usize].unit) as usize;
        let path = norm_path(&self.units[here].path, &want);
        if path.is_empty() || path.ends_with('/') {
            return msg(format!("import: \u{201e}{}\u{201c} ist kein Pfad einer Datei", want));
        }
        if let Some(u) = self.units.iter().rposition(|x| x.path == path && x.state != UnitState::Failed) {
            if let Some(chain) = self.cycle(u) {
                return msg(format!("Zyklischer Import: {}", chain));
            }
            let m = match self.units[u].map {
                Some(m) => m,
                None => {
                    let m = self.unit_map(u)?;
                    self.units[u].map = Some(m);
                    m
                }
            };
            self.stack.truncate(to);
            self.stack.push(m);
            return Ok(false);
        }
        if self.units.len() >= MAX_UNITS {
            return msg(format!("Zu viele Module (mehr als {})", MAX_UNITS - 1));
        }
        let (mark, was) = (self.nroots.len(), self.in_native);
        self.in_native = true;
        let src = host.load(&mut Cx { vm: self }, &path);
        self.in_native = was;
        self.nroots.truncate(mark);
        let src = match src {
            Ok(s) => s,
            Err(why) => return msg(format!("Modul {}: {}", path, why)),
        };
        self.used += src.len() as u64 / 64;
        let prog = match &self.code.strict {
            Some(g) => {
                let g: Vec<&str> = g.iter().map(|s| s.as_str()).collect();
                crate::compile_with(&path, &src, &g)
            }
            None => crate::compile_named(&path, &src),
        };
        let prog = match prog {
            Ok(p) => p,
            Err(d) => return self.module_error(&path, &d),
        };
        let u = self.link(&prog, path)?;
        let pbase = self.units[u].pbase;
        if self.frames.len() as u32 >= self.lim.depth {
            let m = format!("Zu viele Aufrufe ineinander (mehr als {}) – endet die Rekursion?", self.lim.depth);
            return Err(Exc::Fatal(ErrorKind::Depth, m));
        }
        let need = self.code.protos[pbase as usize].max_stack as usize;
        if to + 1 + need + 4 > self.lim.stack as usize {
            return Err(Exc::Fatal(ErrorKind::Stack, format!("Der Stapel ist voll (mehr als {} Werte)", self.lim.stack)));
        }
        self.stack.truncate(to + 1);
        self.frames.push(Frame { proto: pbase, closure: None, base: to as u32 + 1, pc: 0 });
        Ok(true)
    }

    /// the chain of imports when unit u's top-level code runs (outermost first, u again at the
    /// end)
    fn cycle(&self, u: usize) -> Option<String> {
        let tops: Vec<usize> = self.frames.iter().filter(|f| f.closure.is_none()).map(|f| self.code.protos[f.proto as usize].unit as usize).collect();
        let i = tops.iter().position(|&x| x == u)?;
        let name = |x: usize| if self.units[x].path.is_empty() { "(Hauptprogramm)" } else { self.units[x].path.as_str() };
        let mut parts: Vec<&str> = tops[i..].iter().map(|&x| name(x)).collect();
        parts.push(name(u));
        Some(parts.join(" \u{2192} "))
    }

    /// a module that does not compile: a str with its position (catchable); uncaught, the
    /// Error names the module's file, line and column
    fn module_error(&mut self, path: &str, d: &[Diagnostic]) -> Result<bool, Exc> {
        let Some(first) = d.first() else {
            return msg(format!("Modul {}: lässt sich nicht übersetzen", path));
        };
        let more = if d.len() > 1 { format!(" (und {} weitere Fehler)", d.len() - 1) } else { String::new() };
        let v = self.alloc_str(format!("{}:{}:{}: {}{}", path, first.line, first.col, first.msg, more))?;
        let mut trace = vec![TraceLine { func: String::new(), file: String::from(path), line: first.line, col: first.col }];
        trace.extend(self.trace());
        self.escaped = Some((Some(v), trace, Some(format!("{}{}", first.msg, more))));
        Err(Exc::Throw(v))
    }

    /// link a compiled module behind the VM's code -> its unit
    fn link(&mut self, prog: &Program, path: String) -> Result<usize, Exc> {
        let m = &*prog.code;
        let unit = self.units.len() as u32;
        let mut code: Code = (*self.code).clone();
        let (pbase, kbase, gbase) = (code.protos.len() as u32, code.consts.len() as u32, code.globals.len() as u32);
        // host names: one entry per name for the program and every module, after the natives
        // the host made (Vm::native)
        while code.hosts.len() < self.natives.len() {
            let n = String::from(&*self.natives[code.hosts.len()].0);
            code.hosts.push(HostName { name: n, dotted: None });
        }
        let mut hmap = Vec::with_capacity(m.hosts.len());
        for h in &m.hosts {
            let i = match code.hosts.iter().position(|x| x.name == h.name) {
                Some(i) => i,
                None => {
                    code.hosts.push(HostName { name: h.name.clone(), dotted: None });
                    self.natives.push((Arc::from(h.name.as_str()), builtin_of(&h.name)));
                    code.hosts.len() - 1
                }
            };
            hmap.push(i as u32);
        }
        self.hostv.resize(self.natives.len(), None);
        for (j, h) in m.hosts.iter().enumerate() {
            if let Some((b, k)) = h.dotted {
                let i = hmap[j] as usize;
                if code.hosts[i].dotted.is_none() {
                    code.hosts[i].dotted = Some((hmap[b as usize], kbase + k));
                }
            }
        }
        // the constants: its strings live on the heap, rooted from Vm::consts
        for c in &m.consts {
            let v = match c {
                Const::Int(i) => Value::Int(*i),
                Const::Float(f) => Value::Float(*f),
                Const::Str(s) => match self.alloc_str(s.clone()) {
                    Ok(v) => v,
                    Err(e) => {
                        self.consts.truncate(kbase as usize);
                        return Err(e);
                    }
                },
            };
            self.consts.push(v);
        }
        code.consts.extend(m.consts.iter().cloned());
        for (i, p) in m.protos.iter().enumerate() {
            let mut p = p.clone();
            p.unit = unit;
            for op in p.code.iter_mut() {
                *op = reloc(*op, pbase, kbase, gbase, &hmap);
            }
            // the top-level code ends with its map (it has no return statements of its own)
            if let (0, Some(last @ Op::Return)) = (i, p.code.last_mut()) {
                *last = Op::EndModule;
            }
            code.protos.push(p);
        }
        code.globals.extend(m.globals.iter().cloned());
        self.globals.resize(code.globals.len(), Value::Nil);
        self.code = Arc::new(code);
        self.units.push(Unit { path, gbase, gcount: m.globals.len() as u32, pbase, state: UnitState::Ready, map: None });
        self.dbg_rebind();
        Ok(unit as usize)
    }

    /// a module's top-level code returned (the frame of function `proto`): its map
    #[cold]
    #[inline(never)]
    pub(crate) fn module_done(&mut self, proto: u32) -> Result<Value, Exc> {
        let u = self.code.protos[proto as usize].unit as usize;
        match self.unit_map(u) {
            Ok(m) => {
                self.units[u].map = Some(m);
                Ok(m)
            }
            Err(e) => {
                self.units[u].state = UnitState::Failed;
                Err(e)
            }
        }
    }

    /// the map of unit u's public globals as they are now
    fn unit_map(&mut self, u: usize) -> Result<Value, Exc> {
        let (gbase, n) = (self.units[u].gbase as usize, self.units[u].gcount as usize);
        let height = self.stack.len();
        let r = (|| {
            let m = Value::Map(self.alloc(Obj::Map(Default::default()))?);
            self.stack.push(m);
            for i in 0..n {
                let name = self.code.globals[gbase + i].clone();
                if name.starts_with('_') {
                    continue;
                }
                let k = self.alloc_str(name)?;
                self.stack.push(k);
                let v = self.globals[gbase + i];
                self.set_key(m, k, v)?;
                self.stack.pop();
            }
            self.used += n as u64 / 4;
            Ok(m)
        })();
        self.stack.truncate(height);
        r
    }

    /// the files of the VM: the program's name, then the modules' paths in the order they
    /// were imported (one that failed and was imported again once per try)
    pub fn modules(&self) -> Vec<String> {
        self.units.iter().map(|u| u.path.clone()).collect()
    }
}
