// scriptmodt -- checks of lib/script's modules, debug interface and test functions (SK7), and
// the VM benchmark
//
//   run(t)    group "scriptmod": import (maps, privacy, caching, relative and absolute paths,
//             cycles, errors across files with their file, line and column, a host without
//             modules, strict compiling, shared host values, fuel and yields), the line table,
//             breakpoints (also conditional, in modules before they load) and every step kind
//             through recursion, closures, try / catch and module boundaries, a pause in the
//             middle of the fuel, paused and resumed runs equal to uninterrupted ones, the
//             stack, locals, upvalues, globals and eval, assert / assert_eq, the benchmark's
//             programs with and without a debugger (same fuel and output)
//   bench()   `coretest --bench vm`: ns per unit of fuel of a few programs without a debugger
//             (calls, loops, strings, maps), the best of five runs each; then all of them once
//             with a breakpoint that never stops (the debugger checks every instruction)

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use script::{Cx, ErrorKind, Fail, Host, Limits, Pause, PauseReason, Status, Step, Value, Vm};

use crate::T;

/// the benchmark programs: (name, source); each prints one line
const BENCH: &[(&str, &str)] = &[
    ("calls", "fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) }\nprint(fib(25))"),
    ("loop", "let s = 0\nlet i = 0\nwhile i < 1000000 {\n  s += i % 7\n  i += 1\n}\nprint(s)"),
    ("for", "let s = 0\nfor i in 0..300000 {\n  let x = i * 3\n  if x % 2 == 0 { s += x } else { s -= 1 }\n}\nprint(s)"),
    ("strings", "let n = 0\nfor i in 0..100000 {\n  let t = \"k${i % 100}\"\n  n += len(t)\n}\nprint(n)"),
    ("maps", "let m = {}\nfor i in 0..100000 {\n  m[\"k${i % 500}\"] = (m[\"k${i % 500}\"] ?? 0) + i\n}\nprint(len(m))"),
    ("closures", "fn mk(a) { return fn(b) { return a + b } }\nlet f = mk(3)\nlet s = 0\nfor i in 0..200000 { s += f(i) }\nprint(s)"),
];

/// one run of a program: (fuel, microseconds); debug: with a breakpoint that never stops
/// (exec checks every instruction)
fn bench_one(src: &str, debug: bool) -> (u64, u64) {
    let prog = script::compile(src).expect("bench program");
    let lim = Limits { fuel: u64::MAX, ..Limits::default() };
    let mut vm = Vm::new(&prog, lim);
    if debug {
        let _ = vm.set_breakpoint("", 100_000, None);
    }
    let t0 = plat::time_us();
    loop {
        match vm.run(&mut (), 1 << 20) {
            Status::Yielded => {}
            _ => break,
        }
    }
    (vm.call_fuel(), plat::time_us().saturating_sub(t0))
}

/// see the module comment
pub fn bench() {
    let mut total = (0u64, 0u64);
    for (name, src) in BENCH {
        let mut best = u64::MAX;
        let mut fuel = 0;
        for _ in 0..5 {
            let (f, us) = bench_one(src, false);
            fuel = f;
            best = best.min(us.max(1));
        }
        total = (total.0 + fuel, total.1 + best);
        let ns = best as f64 * 1000.0 / fuel.max(1) as f64;
        plat::write_stdout(format!("vm {:<10} {:>10} fuel {:>8} us {:>7.2} ns/op\n", name, fuel, best, ns).as_bytes());
    }
    let ns = total.1 as f64 * 1000.0 / total.0.max(1) as f64;
    plat::write_stdout(format!("vm {:<10} {:>10} fuel {:>8} us {:>7.2} ns/op\n", "all", total.0, total.1, ns).as_bytes());
    // the same with a debugger that checks every instruction (one run each)
    let mut dbg = (0u64, 0u64);
    for (_, src) in BENCH {
        let (f, us) = bench_one(src, true);
        dbg = (dbg.0 + f, dbg.1 + us.max(1));
    }
    let ns = dbg.1 as f64 * 1000.0 / dbg.0.max(1) as f64;
    plat::write_stdout(format!("vm {:<10} {:>10} fuel {:>8} us {:>7.2} ns/op\n", "debugger", dbg.0, dbg.1, ns).as_bytes());
}

// ---------------------------------------------------------------- a host with files

/// files in memory for import, the paths it was asked for
#[derive(Default)]
struct Files {
    files: Vec<(String, String)>,
    loads: Vec<String>,
}

impl Files {
    fn new(files: &[(&str, &str)]) -> Files {
        Files { files: files.iter().map(|(p, s)| (String::from(*p), String::from(*s))).collect(), loads: Vec::new() }
    }
}

impl Host for Files {
    fn call(&mut self, _: &mut Cx, _: &str, _: &[Value]) -> Result<Value, Fail> {
        Err(Fail::Unknown)
    }
    fn load(&mut self, cx: &mut Cx, path: &str) -> Result<String, String> {
        self.loads.push(String::from(path));
        cx.charge(10);
        self.files.iter().find(|f| f.0 == path).map(|f| f.1.clone()).ok_or_else(|| String::from("die Datei gibt es nicht"))
    }
}

fn vm_of(name: &str, src: &str, lim: Limits) -> Vm {
    match script::compile_named(name, src) {
        Ok(p) => Vm::new(&p, lim),
        Err(d) => {
            plat::write_stdout(format!("FAIL scriptmod: {} does not compile: {:?}\n", name, d).as_bytes());
            Vm::new(&script::compile("").expect("empty"), lim)
        }
    }
}

/// run to the end (pauses go on), in slices of `slice`: (output, the result or the error)
fn finish(vm: &mut Vm, host: &mut dyn Host, slice: u64) -> (String, Result<String, script::Error>) {
    let mut o = String::new();
    loop {
        let st = vm.run(host, slice);
        o.push_str(&vm.take_output());
        match st {
            Status::Done(v) => return (o, Ok(vm.display(v))),
            Status::Yielded | Status::Paused(_) => {}
            Status::Error(e) => return (o, Err(e)),
        }
    }
}

/// the output of a program "app/main.jfs" with these files, an error appended as
/// "ERR file:line:col message"
fn out_with(files: &[(&str, &str)], src: &str) -> (String, Vec<String>) {
    let mut h = Files::new(files);
    let mut vm = vm_of("app/main.jfs", src, Limits::default());
    let (o, r) = finish(&mut vm, &mut h, u64::MAX);
    let o = match r {
        Ok(_) => o,
        Err(e) => format!("{}ERR {}:{}:{} {}", o, e.file, e.line, e.col, e.msg),
    };
    (o, h.loads)
}

/// the last part of a path
fn base(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// "file:line" of a pause
fn at(p: &Pause) -> String {
    format!("{}:{}", base(&p.file), p.line)
}

/// run with the debugger: every stop is recorded ("main.jfs:12"); `next` says how to go on
/// (None: continue); at most 300 stops
fn stops(vm: &mut Vm, host: &mut dyn Host, slice: u64, mut next: impl FnMut(&mut Vm, &Pause) -> Option<Step>) -> (Vec<String>, String) {
    let mut v = Vec::new();
    let mut o = String::new();
    loop {
        let st = vm.run(host, slice);
        o.push_str(&vm.take_output());
        match st {
            Status::Done(_) => break,
            Status::Error(e) => {
                o.push_str(&format!("ERR {}", e));
                break;
            }
            Status::Yielded => {}
            Status::Paused(p) => {
                v.push(at(&p));
                if v.len() > 300 {
                    break;
                }
                if let Some(s) = next(vm, &p) {
                    vm.step(s);
                }
            }
        }
    }
    (v, o)
}

// ---------------------------------------------------------------- modules

fn modules(t: &mut T) {
    // a map of the public names, run once, cached by the normalised path
    let a = "print(\"lade a\")\nlet name = \"a\"\nlet _p = 1\nlet n = 0\nfn inc() {\n  n += 1\n  return n\n}";
    let (o, loads) = out_with(&[("app/lib/a.jfs", a)], "let a = import(\"lib/a.jfs\")\nlet b = import(\"./lib/../lib/a.jfs\")\nprint(a == b, a.name, a._p, a.inc(), a.inc(), b.inc(), type(a), a.n)");
    t.eq("import: map, private, once", o.as_str(), "lade a\ntrue a nil 1 2 3 map 0\n");
    t.eq("import: one load", loads, vec![String::from("app/lib/a.jfs")]);
    // imported in a native's callback (list.map), then called from the program's code
    let (o, _) = out_with(&[("app/lib/a.jfs", a)], "let ms = [\"lib/a.jfs\"].map(import)\nprint(ms[0].inc(), ms[0].inc(), import(\"lib/a.jfs\").inc())");
    t.eq("import: through a callback", o.as_str(), "lade a\n1 2 3\n");
    // relative to the importing file, absolute ones as they are
    let files = [
        ("app/lib/a.jfs", "let b = import(\"../util/b.jfs\")\nlet v = \"a\" + b.v"),
        ("app/util/b.jfs", "let c = import(\"/abs/c.jfs\")\nlet v = \"b\" + c.v"),
        ("/abs/c.jfs", "let v = \"c\""),
    ];
    let (o, loads) = out_with(&files, "print(import(\"lib/a.jfs\").v)");
    t.eq("import: relative chain", o.as_str(), "abc\n");
    t.eq("import: the paths", loads, vec![String::from("app/lib/a.jfs"), String::from("app/util/b.jfs"), String::from("/abs/c.jfs")]);
    // cycles: an error that names the chain, catchable; the program itself too
    let files = [("app/lib/a.jfs", "let b = import(\"b.jfs\")"), ("app/lib/b.jfs", "let a = import(\"a.jfs\")")];
    let (o, _) = out_with(&files, "try {\n  import(\"lib/a.jfs\")\n} catch e {\n  print(e)\n}");
    t.eq("import: cycle", o.as_str(), "Zyklischer Import: app/lib/a.jfs \u{2192} app/lib/b.jfs \u{2192} app/lib/a.jfs\n");
    let (o, _) = out_with(&[("app/lib/x.jfs", "import(\"../main.jfs\")")], "import(\"lib/x.jfs\")");
    t.eq("import: the program in a cycle", o.as_str(), "ERR app/lib/x.jfs:1:1 Zyklischer Import: app/main.jfs \u{2192} app/lib/x.jfs \u{2192} app/main.jfs");
    let (o, _) = out_with(&[], "let self = import(\"main.jfs\")");
    t.eq("import: itself", o.as_str(), "ERR app/main.jfs:1:12 Zyklischer Import: app/main.jfs \u{2192} app/main.jfs");
    // errors of other files: missing, does not compile (caught, uncaught), at run time
    let (o, _) = out_with(&[], "try {\n  import(\"nope.jfs\")\n} catch e {\n  print(e)\n}\nimport(42)");
    t.eq("import: missing, no path", o.as_str(), "Modul app/nope.jfs: die Datei gibt es nicht\nERR app/main.jfs:6:1 import braucht den Pfad einer Datei (str), nicht int");
    let bad = [("app/lib/bad.jfs", "let x = 1\nlet y = (2 +\nprint(y)")];
    let (o, _) = out_with(&bad, "try {\n  import(\"lib/bad.jfs\")\n} catch e {\n  print(e)\n}");
    t.check(&format!("import: compile error caught: {}", o), o.starts_with("app/lib/bad.jfs:3:9: Hier fehlt eine schließende Klammer )"));
    let mut h = Files::new(&bad);
    let mut vm = vm_of("app/main.jfs", "print(1)\nlet m = import(\"lib/bad.jfs\")", Limits::default());
    match finish(&mut vm, &mut h, u64::MAX) {
        (_, Err(e)) => {
            t.eq("import: compile error's place", (e.file.as_str(), e.line, e.col), ("app/lib/bad.jfs", 3, 9));
            t.check(&format!("import: compile error's message: {}", e.msg), !e.msg.contains("bad.jfs") && !e.msg.is_empty());
            let files: Vec<&str> = e.trace.iter().map(|x| x.file.as_str()).collect();
            t.eq("import: compile error's trace", files, vec!["app/lib/bad.jfs", "app/main.jfs"]);
        }
        r => t.check(&format!("import: compile error: {:?}", r), false),
    }
    let m = "fn div(a, b) {\n  let q = a / b\n  return q\n}\nlet half = div(1, 2)";
    let mut h = Files::new(&[("app/lib/m.jfs", m)]);
    let mut vm = vm_of("app/main.jfs", "let m = import(\"lib/m.jfs\")\nprint(m.half)\nm.div(1, 0)", Limits::default());
    match finish(&mut vm, &mut h, u64::MAX) {
        (o, Err(e)) => {
            t.eq("import: run error in a module function", (o.as_str(), e.file.as_str(), e.line, e.col), ("0\n", "app/lib/m.jfs", 2, 13));
            t.eq("import: its text", e.to_string().as_str(), "2:13: Fehler: Division durch 0\n  in div (2:13)\n  im Hauptprogramm (app/main.jfs:3:3)");
        }
        r => t.check(&format!("import: run error: {:?}", r), false),
    }
    // an error in a module's top-level code: "im Modul"; the module is forgotten, the next
    // import runs it again
    let top = [("app/lib/t.jfs", "print(\"t\")\nlet z = 1 / 0")];
    let mut h = Files::new(&top);
    let mut vm = vm_of("app/main.jfs", "fn f() { return import(\"lib/t.jfs\") }\ntry {\n  f()\n} catch e {\n  print(e)\n}\nf()", Limits::default());
    match finish(&mut vm, &mut h, u64::MAX) {
        (o, Err(e)) => {
            t.eq("import: error in the top-level code", o.as_str(), "t\nDivision durch 0\nt\n");
            t.eq("import: its trace", e.to_string().as_str(), "2:11: Fehler: Division durch 0\n  im Modul (2:11)\n  in f (app/main.jfs:1:17)\n  im Hauptprogramm (app/main.jfs:7:1)");
            t.eq("import: loaded again", h.loads.len(), 2);
        }
        r => t.check(&format!("import: top-level error: {:?}", r), false),
    }
    // a host without modules
    let mut vm = vm_of("x.jfs", "import(\"y.jfs\")", Limits::default());
    let (_, r) = finish(&mut vm, &mut (), u64::MAX);
    t.eq("import: no loader", r.map_err(|e| e.msg), Err(String::from("Modul y.jfs: hier lassen sich keine Module laden")));
    // compile_with: the module compiles with the same names (a typing error stops it)
    let prog = script::compile_with("app/main.jfs", "let m = import(\"m.jfs\")", &["log"]).expect("strict main");
    let mut vm = Vm::new(&prog, Limits::default());
    let mut h = Files::new(&[("app/m.jfs", "log(1)\nprnt(2)")]);
    let (_, r) = finish(&mut vm, &mut h, u64::MAX);
    t.eq("import: strict like the program", r.map_err(|e| (e.file, e.line, e.msg)), Err((String::from("app/m.jfs"), 2, String::from("Unbekannter Name prnt – meinst du print?"))));
    // a host value the program does not use reaches the module
    let mut vm = vm_of("app/main.jfs", "print(import(\"c.jfs\").port)", Limits::default());
    let conf = vm.json_decode("{\"port\": 8080}").unwrap_or(Value::Nil);
    t.check("import: config not used by the program", !vm.set_global("config", conf));
    let mut h = Files::new(&[("app/c.jfs", "let port = config.port")]);
    t.eq("import: config in the module", finish(&mut vm, &mut h, u64::MAX).0.as_str(), "8080\n");
    // fuel: the module's code counts like the importer's; limits too; slices change nothing
    let lp = "let s = 0\nfor i in 0..1000 { s += i }";
    let mut inline = vm_of("app/main.jfs", &format!("{}\nprint(s)", lp), Limits::default());
    let _ = finish(&mut inline, &mut (), u64::MAX);
    let mut h = Files::new(&[("app/l.jfs", lp)]);
    let mut vm = vm_of("app/main.jfs", "print(import(\"l.jfs\").s)", Limits::default());
    let (o, _) = finish(&mut vm, &mut h, u64::MAX);
    t.check(&format!("import: fuel of the module ({} / {})", vm.call_fuel(), inline.call_fuel()), o == "499500\n" && vm.call_fuel() > inline.call_fuel() && vm.call_fuel() < inline.call_fuel() + 40);
    let mut sliced = vm_of("app/main.jfs", "print(import(\"l.jfs\").s)", Limits::default());
    let (o2, _) = finish(&mut sliced, &mut Files::new(&[("app/l.jfs", lp)]), 3);
    t.eq("import: in slices of 3", (o2.as_str(), sliced.call_fuel()), (o.as_str(), vm.call_fuel()));
    let mut vm = vm_of("app/main.jfs", "import(\"e.jfs\")", Limits { fuel: 5000, ..Limits::default() });
    let (_, r) = finish(&mut vm, &mut Files::new(&[("app/e.jfs", "while true {}")]), 100);
    t.eq("import: fuel limit in a module", r.map_err(|e| (e.kind, e.file)), Err((ErrorKind::Fuel, String::from("app/e.jfs"))));
    t.eq("modules()", vm.modules(), vec![String::from("app/main.jfs"), String::from("app/e.jfs")]);
}

// ---------------------------------------------------------------- the line table

fn lines(t: &mut T) {
    let prog = script::compile_named("lt.jfs", "let a = 1\nfn f(x) {\n  let y = x +\n    2\n  return y\n}\nprint(f(a))\nfor i in 0..2 { a += i }").expect("compiles");
    t.eq("break lines", prog.break_lines(), vec![1, 3, 5, 7, 8]);
    t.eq("functions", prog.functions(), vec!["", "f"]);
    let tab = prog.line_table();
    let stmts: Vec<(u32, u32)> = tab.iter().filter(|e| e.stmt).map(|e| (e.func, e.line)).collect();
    t.eq("statement starts", stmts, vec![(0, 1), (0, 7), (0, 8), (1, 3), (1, 5)]);
    t.check("every instruction", tab.len() == prog.instructions() && tab.iter().any(|e| e.line == 4 && !e.stmt));
    let d = script::compile_named("lib/a.jfs", "let = 1").err().unwrap_or_default();
    t.eq("diagnostic's file", d.first().map(|x| x.file.as_str()), Some("lib/a.jfs"));
    t.eq("check has no file", script::check("let = 1").first().map(|x| x.file.clone()), Some(String::new()));
}

// ---------------------------------------------------------------- the debugger

const MAIN: &str = concat!(
    "let util = import(\"lib/util.jfs\")\n",
    "fn fact(n) {\n",
    "  if n <= 1 {\n",
    "    return 1\n",
    "  }\n",
    "  return n * fact(n - 1)\n",
    "}\n",
    "fn twice(f, x) {\n",
    "  return f(f(x))\n",
    "}\n",
    "fn adder(k) {\n",
    "  return fn(x) { return x + k }\n",
    "}\n",
    "let r = twice(adder(10), 1)\n",
    "try {\n",
    "  util.fail(\"x\")\n",
    "} catch e {\n",
    "  print(\"gefangen\", e)\n",
    "}\n",
    "let total = 0\n",
    "for i in 0..5 {\n",
    "  total += i\n",
    "}\n",
    "print(fact(4), r, util.sq(3), total)\n",
);

const UTIL: &str = concat!(
    "let calls = 0\n",
    "fn sq(x) {\n",
    "  calls += 1\n",
    "  return x * x\n",
    "}\n",
    "fn fail(m) {\n",
    "  throw \"kaputt \" + m\n",
    "}\n",
    "let _secret = 1\n",
);

const OUT: &str = "gefangen kaputt x\n24 21 9 10\n";

fn files() -> Files {
    Files::new(&[("app/lib/util.jfs", UTIL)])
}

fn main_vm() -> Vm {
    vm_of("app/main.jfs", MAIN, Limits::default())
}

fn debugger(t: &mut T) {
    // the reference: no debugger
    let mut plain = main_vm();
    let (o, r) = finish(&mut plain, &mut files(), u64::MAX);
    t.eq("debug: the program", (o.as_str(), r.is_ok()), (OUT, true));
    let fuel = plain.call_fuel();
    // step over from the start: the top-level statements, loop heads each round
    let mut vm = main_vm();
    vm.step(Step::Over);
    let (v, o) = stops(&mut vm, &mut files(), u64::MAX, |_, _| Some(Step::Over));
    t.eq("step over", v.join(" "), String::from("main.jfs:1 main.jfs:14 main.jfs:16 main.jfs:18 main.jfs:20 main.jfs:21 main.jfs:22 main.jfs:21 main.jfs:22 main.jfs:21 main.jfs:22 main.jfs:21 main.jfs:22 main.jfs:21 main.jfs:22 main.jfs:21 main.jfs:24"));
    t.eq("step over: output, fuel", (o.as_str(), vm.call_fuel()), (OUT, fuel));
    // step into: into the module's code and back, calls, closures, an exception to the catch
    let mut vm = main_vm();
    vm.step(Step::Into);
    let (v, o) = stops(&mut vm, &mut files(), u64::MAX, |_, _| Some(Step::Into));
    let first: Vec<String> = v.iter().take(18).cloned().collect();
    t.eq("step into", first.join(" "), String::from("main.jfs:1 util.jfs:1 util.jfs:9 main.jfs:1 main.jfs:14 main.jfs:12 main.jfs:14 main.jfs:9 main.jfs:12 main.jfs:9 main.jfs:12 main.jfs:9 main.jfs:14 main.jfs:16 util.jfs:7 main.jfs:18 main.jfs:20 main.jfs:21"));
    t.eq("step into: output, fuel", (o.as_str(), vm.call_fuel()), (OUT, fuel));
    // a breakpoint in a recursion, the stack, locals, step out
    let mut vm = main_vm();
    t.check("breakpoint set", vm.set_breakpoint("app/main.jfs", 4, None) == Ok(1));
    let mut seen = Vec::new();
    let (v, o) = stops(&mut vm, &mut files(), u64::MAX, |vm, p| {
        let f = vm.frames();
        let names: Vec<&str> = f.iter().map(|x| x.func.as_str()).collect();
        let n = vm.locals(0).first().map(|x| vm.display(x.1)).unwrap_or_default();
        seen.push(format!("{:?} {} {} n={}", p.reason, names.join(","), f.first().map_or(0, |x| x.line), n));
        if seen.len() < 3 { Some(Step::Out) } else { None }
    });
    t.eq("recursion: the stops", v.join(" "), String::from("main.jfs:4 main.jfs:6 main.jfs:6"));
    t.eq("recursion: stack and locals", seen.join(" | "), String::from("Breakpoint fact,fact,fact,fact, 4 n=1 | Step fact,fact,fact, 6 n=2 | Step fact,fact, 6 n=3"));
    t.eq("recursion: output", o.as_str(), OUT);
    // closures: the captured variable as an upvalue
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/main.jfs", 12, None);
    let mut seen = Vec::new();
    let (v, _) = stops(&mut vm, &mut files(), u64::MAX, |vm, _| {
        let show = |l: Vec<(String, Value)>, vm: &Vm| l.iter().map(|(n, v)| format!("{}={}", n, vm.display(*v))).collect::<Vec<_>>().join(",");
        seen.push(format!("{} [{}] [{}]", vm.frames()[0].func, show(vm.locals(0), vm), show(vm.upvalues(0), vm)));
        None
    });
    t.eq("closure: stops", v.len(), 3);
    t.eq("closure: locals and upvalues", seen.join(" | "), String::from("adder [k=10] [] | fn [x=1] [k=10] | fn [x=11] [k=10]"));
    // try / catch: over a call that throws lands in the catch, with its variable
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/main.jfs", 16, None);
    let mut caught = String::new();
    let (v, _) = stops(&mut vm, &mut files(), u64::MAX, |vm, p| {
        if p.reason == PauseReason::Breakpoint {
            return Some(Step::Over);
        }
        caught = vm.locals(0).iter().map(|(n, v)| format!("{}={}", n, vm.display(*v))).collect::<Vec<_>>().join(",");
        None
    });
    t.eq("try: over into the catch", (v.join(" "), caught.as_str()), (String::from("main.jfs:16 main.jfs:18"), "e=kaputt x"));
    // a breakpoint in a module before it is loaded; frames across files; globals
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/lib/util.jfs", 4, None);
    t.eq("module breakpoint: not bound yet", vm.breakpoints().first().map(|b| b.at), Some(None));
    let mut seen = String::new();
    let (v, _) = stops(&mut vm, &mut files(), u64::MAX, |vm, _| {
        let f = vm.frames();
        let g: Vec<String> = vm.frame_globals(0).iter().map(|(n, v)| format!("{}={}", n, vm.display(*v))).collect();
        seen = format!("{}@{}:{} {}@{}:{} [{}] at={:?} total={:?}", f[0].func, base(&f[0].file), f[0].line, f[1].func, base(&f[1].file), f[1].line, g.join(","), vm.breakpoints()[0].at, vm.globals_of("app/main.jfs").iter().find(|x| x.0 == "total").map(|x| vm.display(x.1)));
        None
    });
    t.eq("module breakpoint: stop", v.join(" "), String::from("util.jfs:4"));
    // out of the module's function back into the program, then over the rest
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/lib/util.jfs", 3, None);
    let mut n = 0;
    let (v, o) = stops(&mut vm, &mut files(), u64::MAX, |_, _| {
        n += 1;
        Some(if n == 1 { Step::Out } else { Step::Over })
    });
    t.eq("module: out and over", (v.join(" "), o.as_str()), (String::from("util.jfs:3 main.jfs:24"), OUT));
    t.eq("module breakpoint: frames and globals", seen.as_str(), "sq@util.jfs:4 @main.jfs:24 [calls=1,sq=<fn sq>,fail=<fn fail>,_secret=1] at=Some(4) total=Some(\"10\")");
    // conditions; a condition that fails stops with the reason; a breakpoint on a line
    // without a statement stops at the next one
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/main.jfs", 22, Some("i == 3"));
    let mut seen = String::new();
    let (v, _) = stops(&mut vm, &mut files(), u64::MAX, |vm, _| {
        seen = format!("i={} total={}", vm.eval(0, "i").map(|x| vm.display(x)).unwrap_or_default(), vm.eval(0, "total").map(|x| vm.display(x)).unwrap_or_default());
        None
    });
    t.eq("condition", (v.join(" "), seen.as_str()), (String::from("main.jfs:22"), "i=3 total=3"));
    let mut vm = main_vm();
    t.check("condition: syntax error", vm.set_breakpoint("app/main.jfs", 22, Some("i ==")).is_err());
    let _ = vm.set_breakpoint("app/main.jfs", 23, Some("nope + 1"));
    let mut why = None;
    let (v, _) = stops(&mut vm, &mut files(), u64::MAX, |_, p| {
        why = p.error.clone();
        None
    });
    t.eq("condition error, next line", (v.first().cloned(), why), (Some(String::from("main.jfs:24")), Some(String::from("Unbekannter Name nope"))));
    // (the refused one got no id)
    t.check("remove", vm.remove_breakpoint(1) && !vm.remove_breakpoint(1) && !vm.debugging());
    // eval: read-only, no calls, fuel unchanged
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/main.jfs", 4, None);
    let mut seen = Vec::new();
    let (_, o) = stops(&mut vm, &mut files(), u64::MAX, |vm, _| {
        let f0 = vm.call_fuel();
        for e in ["n * 10 + len([1, 2])", "\"n=${n}\"", "[{a: n}][0].a","util.calls ?? -1", "math.pi > 3", "fact(2)", "n = 2", "nope"] {
            seen.push(match vm.eval(0, e) {
                Ok(v) => vm.repr(v),
                Err(m) => format!("Err({})", m),
            });
        }
        seen.push(format!("fuel {}", vm.call_fuel() == f0));
        None
    });
    t.eq("eval", seen.join(" | "), String::from("12 | \"n=1\" | 1 | 0 | true | Err(Im Ausdruck gehen keine Aufrufe außer len, str, type, int und float) | Err(Erwartet wird genau ein Ausdruck) | Err(Unbekannter Name nope) | fuel true"));
    t.eq("eval: output, fuel", (o.as_str(), vm.call_fuel()), (OUT, fuel));
    // values for a variables view
    let mut vm = vm_of("v.jfs", "let m = {a: [1, \"zwei\"], 3: true}", Limits::default());
    let _ = finish(&mut vm, &mut (), u64::MAX);
    let m = vm.global("m").unwrap_or(Value::Nil);
    let kids: Vec<String> = vm.children(m).iter().map(|(k, v)| format!("{}={}", k, vm.repr(*v))).collect();
    t.eq("children", kids.join(" "), String::from("a=[1, \"zwei\"] 3=true"));
    t.eq("preview", vm.preview(m, 10), String::from("{a: [1, \"…"));
    // a pause request stops in the middle of the fuel; going on gives the same result
    let src = "let s = 0\nfor i in 0..2000 {\n  s += i\n}\nprint(s)";
    let mut whole = vm_of("p.jfs", src, Limits::default());
    let (wo, _) = finish(&mut whole, &mut (), u64::MAX);
    let mut vm = vm_of("p.jfs", src, Limits::default());
    t.eq("pause: first slice", vm.run(&mut (), 1000), Status::Yielded);
    vm.request_pause();
    let st = vm.run(&mut (), 1_000_000);
    t.check(&format!("pause: mid-fuel ({:?}, {})", st, vm.last_fuel()), matches!(&st, Status::Paused(p) if p.reason == PauseReason::Pause && p.line == 2) && vm.last_fuel() < 10);
    t.check("pause: paused()", vm.paused().is_some_and(|p| p.reason == PauseReason::Pause));
    let (o, _) = finish(&mut vm, &mut (), 777);
    t.eq("pause: resumed equals uninterrupted", (o, vm.call_fuel()), (wo, whole.call_fuel()));
    t.check("pause: over", vm.paused().is_none() && !vm.debugging());
    // a pause requested while idle stops the next invocation at its first statement
    let mut vm = vm_of("c.jfs", "fn cb(x) {\n  return x + 1\n}", Limits::default());
    let _ = finish(&mut vm, &mut (), u64::MAX);
    vm.request_pause();
    let _ = vm.call("cb", &[Value::Int(1)]);
    t.check("pause: next invocation", matches!(vm.run(&mut (), u64::MAX), Status::Paused(p) if p.line == 2));
    t.eq("pause: then on", vm.run(&mut (), u64::MAX), Status::Done(Value::Int(2)));
    // every stop, every step kind, in slices of 13: the same output and fuel as without
    let mut vm = main_vm();
    for l in 1..=24 {
        let _ = vm.set_breakpoint("app/main.jfs", l, None);
    }
    for l in 1..=9 {
        let _ = vm.set_breakpoint("app/lib/util.jfs", l, None);
    }
    let kinds = [Some(Step::Into), Some(Step::Over), None, Some(Step::Out), Some(Step::Into)];
    let mut k = 0;
    let (v, o) = stops(&mut vm, &mut files(), 13, |_, _| {
        k += 1;
        kinds[k % kinds.len()]
    });
    t.check(&format!("mixed: {} stops", v.len()), v.len() > 30 && v.len() < 300);
    t.eq("mixed: output and fuel", (o.as_str(), vm.call_fuel()), (OUT, fuel));
    // cancel while paused: the pause is gone, the VM is idle and runs again
    let mut vm = main_vm();
    let _ = vm.set_breakpoint("app/main.jfs", 6, None);
    t.check("cancel: paused", matches!(vm.run(&mut files(), u64::MAX), Status::Paused(_)));
    vm.cancel();
    t.check("cancel: idle", !vm.busy() && vm.paused().is_none());
}

// ---------------------------------------------------------------- assert, the benchmark

fn asserts(t: &mut T) {
    let run = |src: &str| {
        let mut vm = vm_of("a.jfs", src, Limits::default());
        match finish(&mut vm, &mut (), u64::MAX) {
            (o, Ok(_)) => o,
            (o, Err(e)) => format!("{}ERR {}:{} {}", o, e.line, e.col, e.msg),
        }
    };
    t.eq("assert", run("assert(1 < 2)\nassert(nil, \"eins\")").as_str(), "ERR 2:1 Annahme verletzt: eins");
    t.eq("assert without message", run("assert(false)").as_str(), "ERR 1:1 Annahme verletzt");
    t.eq("assert_eq: values", run("assert_eq(\"1\", 1)").as_str(), "ERR 1:1 assert_eq: erwartet 1, ist \"1\"");
    t.eq("assert_eq: message", run("assert_eq([1, 2], [1, 2, 3], \"Liste\")").as_str(), "ERR 1:1 Liste: erwartet [1, 2, 3], ist [1, 2]");
    t.eq("assert_eq: by content", run("assert_eq({a: [1, {b: 2.0}], c: nil}, {c: nil, a: [1, {b: 2}]})\nassert_eq(3, 3.0)\nprint(\"ok\")").as_str(), "ok\n");
    t.eq("assert_eq: maps differ", run("assert_eq({a: 1}, {a: 1, b: 2})").as_str(), "ERR 1:1 assert_eq: erwartet {a: 1, b: 2}, ist {a: 1}");
    t.eq("assert: catchable", run("try {\n  assert_eq(1, 2)\n} catch e {\n  print(e)\n}").as_str(), "assert_eq: erwartet 2, ist 1\n");
}

fn bench_same(t: &mut T) {
    for (name, src) in BENCH {
        let prog = script::compile(src).expect("bench program");
        let lim = Limits { fuel: u64::MAX, ..Limits::default() };
        let mut a = Vm::new(&prog, lim);
        let (oa, _) = finish(&mut a, &mut (), 1 << 20);
        // a debugger that has nothing to do
        let mut b = Vm::new(&prog, lim);
        let _ = b.set_breakpoint("", 1, None);
        b.clear_breakpoints();
        let (ob, _) = finish(&mut b, &mut (), 1 << 20);
        // one with a breakpoint that never stops: every instruction is checked
        let mut c = Vm::new(&prog, lim);
        let _ = c.set_breakpoint("", 1, Some("false"));
        let (oc, _) = finish(&mut c, &mut (), 1 << 20);
        t.eq(&format!("bench {}: the same without and with a debugger", name), (ob.as_str(), b.call_fuel(), oc.as_str(), c.call_fuel()), (oa.as_str(), a.call_fuel(), oa.as_str(), a.call_fuel()));
        t.check(&format!("bench {}: debugger off", name), !b.debugging() && c.debugging());
    }
}

/// see the module comment
pub fn run(t: &mut T) {
    modules(t);
    lines(t);
    debugger(t);
    asserts(t);
    bench_same(t);
}
