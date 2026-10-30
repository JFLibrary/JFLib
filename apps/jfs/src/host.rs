// host -- the console host of jfs: the names of script::GLOBALS for a script on the command line,
// and the event loop that runs its top-level code, then its timers and disk callbacks
//
//   print(..)            stdout (the VM's output, written after every slice of fuel; a log
//                        line makes the slice end, so stdout and stderr keep their order)
//   log(x, ..), log.debug|info|warn|error(..)   a line on stderr ("warn: ..."; log and
//                        log.info without a prefix)
//   now()                milliseconds since the start (a float): the real clock, or with
//                        --virtual-time the virtual one, which stands still while code runs
//                        and jumps to the next timer
//   rand() / rand(n) / rand(a, b)   [0, 1) / 0 .. n-1 / a .. b (PCG32: --seed N, else a
//                        random seed)
//   config               a map: JFS_CONFIG (a JSON object), then every --config (K=V, V as
//                        JSON or else as text, or a JSON object), env: the --env variables,
//                        args: the arguments after the script, script: its name
//   timer.after(ms, f) / timer.every(ms, f) -> id, timer.cancel(id) -> bool   every: 10 µs at
//                        least; due timers run in the order of their time, then of their ids
//   disk.get(key, fn(err, value)), disk.put(key, value, fn(err)), disk.delete(key, fn(err,
//   had)), disk.scan(prefix, fn(err, items))   JSON values as in JFNetworker; the callbacks
//                        run after the current code, before the next timer; --disk FILE
//                        keeps them in a JSON object in FILE (written after every
//                        invocation that changed them), else they live in memory
//   net, dns, http, metrics   JFNetworker's: a catchable error
//
// The program ends when no timer and no callback is left (or the clock passes --until): exit
// code 0; an uncaught error or a limit ends it at once with the message, line and column and
// the calls on stderr: 1.  Limits::fuel holds per invocation (the top-level code, each
// callback), as in JFNetworker.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Bound;

use base::rng::Pcg32;
use script::{num, Cx, Error, Fail, Host, Pin, Program, Status, Value, Vm};

use crate::{err, excerpt, out, Opts};

/// nanoseconds
type Ns = u64;
const MS: Ns = 1_000_000;
/// the shortest period of timer.every
const EVERY_MIN: Ns = 10_000;
/// fuel of one run() between two writes of the output
const SLICE: u64 = 100_000;

/// the functions a script may also read as values (`let f = log`)
const FUNCS: &[&str] = &[
    "log", "log.debug", "log.info", "log.warn", "log.error", "now", "rand", "timer.after", "timer.every", "timer.cancel", "disk.get", "disk.put",
    "disk.delete", "disk.scan",
];

/// the names of JFNetworker's hosts that the runner does not have
const ELSEWHERE: &[&str] = &["net", "dns", "http", "metrics"];

struct Tmr {
    pin: Pin,
    period: Option<Ns>,
    at: Ns,
}

/// what a disk access answers its callback
enum DiskOut {
    Nothing,
    Value(Option<String>),
    Had(bool),
    Items(Vec<(String, String)>),
}

pub struct Console {
    virt: bool,
    /// the virtual clock
    clock: Ns,
    /// time_us() at the start (the real clock)
    start: u64,
    rng: Pcg32,
    next: u64,
    timers: BTreeMap<u64, Tmr>,
    /// (due, id) of every timer
    due: BTreeSet<(Ns, u64)>,
    /// disk callbacks waiting for the current code to end
    ready: VecDeque<(Pin, DiskOut)>,
    /// key -> JSON text
    store: BTreeMap<String, String>,
    dirty: bool,
    /// log lines not yet written
    logs: String,
}

/// milliseconds (int or float, at least 0) as nanoseconds
fn ms_of(v: Value) -> Option<Ns> {
    match v {
        Value::Int(i) if i >= 0 => Some((i as u64).saturating_mul(MS)),
        Value::Float(f) if !num::is_nan(f) && num::le(0.0, f) => {
            let ns = num::floor(num::add(num::mul(f, 1_000_000.0), 0.5));
            Some(num::to_i64(ns).map_or(u64::MAX, |n| n as u64))
        }
        _ => None,
    }
}

/// nanoseconds as milliseconds (float)
fn ms_float(t: Ns) -> f64 {
    num::div(num::from_i64(t.min(i64::MAX as u64) as i64), 1_000_000.0)
}

fn is_fn(v: Value) -> bool {
    matches!(v, Value::Func(_) | Value::Native(_))
}

/// a JSON string
fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

impl Console {
    fn new(o: &Opts) -> Console {
        let seed = o.seed.unwrap_or_else(plat::random_u64);
        Console {
            virt: o.virtual_time,
            clock: 0,
            start: plat::time_us(),
            rng: Pcg32::new(seed, 0),
            next: 0,
            timers: BTreeMap::new(),
            due: BTreeSet::new(),
            ready: VecDeque::new(),
            store: BTreeMap::new(),
            dirty: false,
            logs: String::new(),
        }
    }

    fn now(&self) -> Ns {
        if self.virt { self.clock } else { plat::time_us().saturating_sub(self.start).saturating_mul(1000) }
    }

    /// let the clock reach `at`: the virtual one jumps, the real one is waited for
    fn wait(&mut self, at: Ns) {
        if self.virt {
            self.clock = self.clock.max(at);
            return;
        }
        loop {
            let now = self.now();
            if now >= at {
                return;
            }
            plat::sleep_ms(((at - now).div_ceil(MS)).min(1000) as u32);
        }
    }

    fn log(&mut self, cx: &mut Cx, level: &str, args: &[Value]) -> Result<Value, Fail> {
        let parts: Vec<String> = args.iter().map(|v| cx.display(*v)).collect();
        let text = parts.join(" ");
        cx.charge(text.len() as u64 / 64);
        self.logs.push_str(level);
        self.logs.push_str(&text);
        self.logs.push('\n');
        // end the slice: what print wrote before goes out first
        cx.yield_now();
        Ok(Value::Nil)
    }

    fn rand(&mut self, cx: &mut Cx, args: &[Value]) -> Result<Value, Fail> {
        let a = args.first().copied().unwrap_or(Value::Nil);
        let b = args.get(1).copied().unwrap_or(Value::Nil);
        match (a, b) {
            (Value::Nil, Value::Nil) => {
                let x = self.rng.next_u64() >> 11;
                Ok(Value::Float(num::mul(num::from_i64(x as i64), 1.0 / 9_007_199_254_740_992.0)))
            }
            (Value::Int(n), Value::Nil) if n >= 1 => Ok(Value::Int(self.rng.range(0, n - 1))),
            (Value::Int(lo), Value::Int(hi)) if lo <= hi => Ok(Value::Int(self.rng.range(lo, hi))),
            _ => Err(cx.error("rand() gibt eine Zahl in [0, 1), rand(n) eine ganze Zahl von 0 bis n - 1, rand(a, b) eine von a bis b")),
        }
    }

    fn timer(&mut self, cx: &mut Cx, name: &str, ms: Value, f: Value) -> Result<Value, Fail> {
        let Some(d) = ms_of(ms) else {
            return Err(cx.error(&format!("{}: die Zeit muss eine Zahl ab 0 sein (Millisekunden)", name)));
        };
        if !is_fn(f) {
            return Err(cx.error(&format!("{}: das zweite Argument muss eine Funktion sein", name)));
        }
        let period = if name == "timer.every" { Some(d.max(EVERY_MIN)) } else { None };
        let at = self.now().saturating_add(period.unwrap_or(d));
        self.next += 1;
        let id = self.next;
        let pin = cx.pin(f);
        self.timers.insert(id, Tmr { pin, period, at });
        self.due.insert((at, id));
        Ok(Value::Int(id as i64))
    }

    fn cancel(&mut self, cx: &mut Cx, v: Value) -> Result<Value, Fail> {
        let Value::Int(id) = v else { return Ok(Value::Bool(false)) };
        match self.timers.remove(&(id as u64)) {
            Some(t) => {
                self.due.remove(&(t.at, id as u64));
                cx.unpin(t.pin);
                Ok(Value::Bool(true))
            }
            None => Ok(Value::Bool(false)),
        }
    }

    fn disk(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        let Some(key) = cx.str(a(0)).map(String::from) else {
            return Err(cx.error(&format!("{}: der Schlüssel muss ein Text sein", name)));
        };
        let (cb, needs) = match name {
            "disk.put" => (a(2), false),
            "disk.delete" => (a(1), false),
            _ => (a(1), true),
        };
        if !is_fn(cb) && (needs || cb != Value::Nil) {
            return Err(cx.error(&format!("{}: der Rückruf muss eine Funktion sein: fn(err, ..)", name)));
        }
        let res = match name {
            "disk.get" => DiskOut::Value(self.store.get(&key).cloned()),
            "disk.put" => {
                let v = cx.json_encode(a(1), 0)?;
                cx.charge((key.len() + v.len()) as u64 / 64);
                self.store.insert(key, v);
                self.dirty = true;
                DiskOut::Nothing
            }
            "disk.delete" => {
                let had = self.store.remove(&key).is_some();
                self.dirty |= had;
                DiskOut::Had(had)
            }
            _ => {
                let items: Vec<(String, String)> = self.store.range::<str, _>((Bound::Included(key.as_str()), Bound::Unbounded)).take_while(|(k, _)| k.starts_with(key.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
                cx.charge(items.len() as u64 / 4);
                DiskOut::Items(items)
            }
        };
        if is_fn(cb) {
            let pin = cx.pin(cb);
            self.ready.push_back((pin, res));
        }
        Ok(Value::Nil)
    }

    /// write what print and log produced: stdout first
    fn flush(&mut self, vm: &mut Vm) {
        let o = vm.take_output();
        if !o.is_empty() {
            out(&o);
        }
        if !self.logs.is_empty() {
            err(&core::mem::take(&mut self.logs));
        }
    }

    /// the store of --disk FILE (a missing file is an empty store)
    fn load(&mut self, vm: &mut Vm, file: &str) -> Result<(), String> {
        if plat::file_exists(file) == 0 {
            return Ok(());
        }
        let Some(bytes) = plat::read_file(file) else {
            return Err(format!("--disk {} lässt sich nicht lesen", file));
        };
        let Ok(text) = core::str::from_utf8(&bytes) else {
            return Err(format!("--disk {} ist kein UTF-8-Text", file));
        };
        let text = text.trim_start_matches('\u{feff}');
        if text.trim().is_empty() {
            return Ok(());
        }
        let m = vm.json_decode(text).map_err(|e| format!("--disk {}: {}", file, e.msg))?;
        if !matches!(m, Value::Map(_)) {
            return Err(format!("--disk {}: kein JSON-Objekt", file));
        }
        for (k, v) in vm.map_entries(m) {
            let key = String::from(vm.str(k).unwrap_or(""));
            let val = vm.json_encode(v, 0).map_err(|e| format!("--disk {}: {}", file, e.msg))?;
            self.store.insert(key, val);
        }
        Ok(())
    }

    /// write the store to --disk FILE: one key per line
    fn save(&mut self, file: &str) -> Result<(), String> {
        self.dirty = false;
        let mut s = String::from("{");
        for (i, (k, v)) in self.store.iter().enumerate() {
            s.push_str(if i == 0 { "\n  " } else { ",\n  " });
            s.push_str(&quote(k));
            s.push_str(": ");
            s.push_str(v);
        }
        s.push_str(if self.store.is_empty() { "}\n" } else { "\n}\n" });
        if plat::write_file(file, s.as_bytes()) { Ok(()) } else { Err(format!("--disk {} lässt sich nicht schreiben", file)) }
    }
}

impl Host for Console {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "log" | "log.info" => self.log(cx, "", args),
            "log.debug" => self.log(cx, "debug: ", args),
            "log.warn" => self.log(cx, "warn: ", args),
            "log.error" => self.log(cx, "error: ", args),
            "now" => Ok(Value::Float(ms_float(self.now()))),
            "rand" => self.rand(cx, args),
            "timer.after" | "timer.every" => self.timer(cx, name, a(0), a(1)),
            "timer.cancel" => self.cancel(cx, a(0)),
            "disk.get" | "disk.put" | "disk.delete" | "disk.scan" => self.disk(cx, name, args),
            _ if ELSEWHERE.contains(&name.split('.').next().unwrap_or(name)) => {
                Err(cx.error(&format!("{} gibt es nur in JFNetworker, nicht im Runner jfs", name)))
            }
            _ => Err(Fail::Unknown),
        }
    }

    fn knows(&self, name: &str) -> bool {
        FUNCS.contains(&name)
    }
}

/// the config map of the command line and the environment
fn config(vm: &mut Vm, o: &Opts, name: &str) -> Result<Value, String> {
    let e = |e: Error| e.msg;
    let m = vm.new_map().map_err(e)?;
    let merge = |vm: &mut Vm, text: &str, what: &str| -> Result<(), String> {
        let v = vm.json_decode(text).map_err(|x| format!("{}: {}", what, x.msg))?;
        if !matches!(v, Value::Map(_)) {
            return Err(format!("{}: kein JSON-Objekt", what));
        }
        for (k, x) in vm.map_entries(v) {
            vm.map_set(m, k, x).map_err(e)?;
        }
        Ok(())
    };
    if let Some(t) = plat::env("JFS_CONFIG").filter(|t| !t.trim().is_empty()) {
        merge(vm, &t, "JFS_CONFIG")?;
    }
    for c in &o.config {
        if c.trim_start().starts_with('{') {
            merge(vm, c, "--config")?;
            continue;
        }
        let Some((k, v)) = c.split_once('=').filter(|(k, _)| !k.is_empty()) else {
            return Err(format!("--config {}: KEY=WERT oder ein JSON-Objekt erwartet", c));
        };
        let val = match vm.json_decode(v) {
            Ok(x) => x,
            Err(_) => vm.new_str(v).map_err(e)?,
        };
        let key = vm.new_str(k).map_err(e)?;
        vm.map_set(m, key, val).map_err(e)?;
    }
    if !o.env.is_empty() {
        let env = vm.new_map().map_err(e)?;
        for n in &o.env {
            if let Some(v) = plat::env(n) {
                let (k, x) = (vm.new_str(n).map_err(e)?, vm.new_str(&v).map_err(e)?);
                vm.map_set(env, k, x).map_err(e)?;
            }
        }
        let k = vm.new_str("env").map_err(e)?;
        vm.map_set(m, k, env).map_err(e)?;
    }
    let mut items = Vec::with_capacity(o.args.len());
    for a in &o.args {
        items.push(vm.new_str(a).map_err(e)?);
    }
    let (list, k) = (vm.new_list(items).map_err(e)?, vm.new_str("args").map_err(e)?);
    vm.map_set(m, k, list).map_err(e)?;
    let (s, k) = (vm.new_str(name).map_err(e)?, vm.new_str("script").map_err(e)?);
    vm.map_set(m, k, s).map_err(e)?;
    Ok(m)
}

/// the arguments of a disk callback
fn disk_args(vm: &mut Vm, res: DiskOut) -> Result<Vec<Value>, Error> {
    Ok(match res {
        DiskOut::Nothing => vec![Value::Nil],
        DiskOut::Value(v) => vec![Value::Nil, v.and_then(|t| vm.json_decode(&t).ok()).unwrap_or(Value::Nil)],
        DiskOut::Had(b) => vec![Value::Nil, Value::Bool(b)],
        DiskOut::Items(items) => {
            let m = vm.new_map()?;
            for (k, t) in items {
                let k = vm.new_str(&k)?;
                let v = vm.json_decode(&t).unwrap_or(Value::Nil);
                vm.map_set(m, k, v)?;
            }
            vec![Value::Nil, m]
        }
    })
}

/// run the pending invocation to its end, writing the output as it goes
fn drive(vm: &mut Vm, host: &mut Console) -> Result<(), Error> {
    loop {
        let st = vm.run(host, SLICE);
        host.flush(vm);
        match st {
            Status::Done(_) => return Ok(()),
            Status::Yielded => {}
            Status::Error(e) => return Err(e),
        }
    }
}

/// an error of the run on stderr: "FILE:LINE:COL: Fehler: ...", the line, the calls
fn report(e: &Error, name: &str, src: &str) {
    let mut s = if e.line > 0 { format!("{}:{}:{}: Fehler: {}\n", name, e.line, e.col, e.msg) } else { format!("{}: Fehler: {}\n", name, e.msg) };
    excerpt(&mut s, src, e.line, e.col, e.col + 1);
    // a recursion's frames once, with their count
    let mut i = 0;
    while i < e.trace.len() {
        let t = &e.trace[i];
        let n = e.trace[i..].iter().take_while(|x| *x == t).count();
        if t.func.is_empty() {
            s.push_str(&format!("  im Hauptprogramm ({}:{})\n", t.line, t.col));
        } else {
            s.push_str(&format!("  in {} ({}:{})\n", t.func, t.line, t.col));
        }
        if n > 1 {
            s.push_str(&format!("  … dieselbe Zeile noch {}-mal\n", n - 1));
        }
        i += n;
    }
    err(&s);
}

/// run a compiled script with its timers and callbacks -> the exit code
pub fn run(prog: &Program, o: &Opts, name: &str, src: &str) -> i32 {
    let mut vm = Vm::new(prog, o.limits);
    let mut host = Console::new(o);
    if let Some(f) = &o.disk {
        if let Err(m) = host.load(&mut vm, f) {
            err(&format!("jfs: {}\n", m));
            return 2;
        }
    }
    match config(&mut vm, o, name) {
        Ok(c) => {
            vm.set_global("config", c);
        }
        Err(m) => {
            err(&format!("jfs: {}\n", m));
            return 2;
        }
    }
    let mut r = drive(&mut vm, &mut host);
    loop {
        if let (Some(f), true) = (&o.disk, host.dirty) {
            if let Err(m) = host.save(f) {
                err(&format!("jfs: {}\n", m));
                return 1;
            }
        }
        if let Err(e) = r {
            report(&e, name, src);
            return 1;
        }
        let (f, args) = if let Some((pin, res)) = host.ready.pop_front() {
            let f = vm.pinned(pin);
            vm.unpin(pin);
            match disk_args(&mut vm, res) {
                Ok(a) => (f, a),
                Err(e) => {
                    r = Err(e);
                    continue;
                }
            }
        } else if let Some(&(at, id)) = host.due.first() {
            if let Some(u) = o.until.filter(|u| at > *u) {
                host.wait(u);
                break;
            }
            host.wait(at);
            host.due.remove(&(at, id));
            let now = host.now();
            let Some(t) = host.timers.get_mut(&id) else { continue };
            let f = vm.pinned(t.pin);
            match t.period {
                Some(p) => {
                    // a clock that fell behind skips the missed rounds
                    t.at = t.at.saturating_add(p);
                    if t.at <= now {
                        t.at = t.at.saturating_add((now - t.at) / p * p + p);
                    }
                    host.due.insert((t.at, id));
                }
                None => {
                    if let Some(t) = host.timers.remove(&id) {
                        vm.unpin(t.pin);
                    }
                }
            }
            (f, Vec::new())
        } else {
            break;
        };
        r = match vm.call_value(f, &args) {
            Ok(()) => drive(&mut vm, &mut host),
            Err(e) => Err(e),
        };
    }
    0
}
