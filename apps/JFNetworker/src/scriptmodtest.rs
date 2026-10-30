// scriptmodtest -- checks of scripts that import the device's files and of the debugger's
// points in a script service (group "scriptmod" of --selftest: jfn_sim::scriptsvc with
// script's modules and debug interface): a module by its absolute path, by a path relative to
// /root and relative to the importing module, the disk's time of the read, a missing file
// (caught and uncaught), a cycle, a module's error with its file in the error, the log and the
// status; a script paused in virtual time (Action::ScriptDebug): its callback waits, its timer
// fires late, a step and an expression in the stop, a conditional breakpoint, the refusals;
// determinism (twice, and a clone of the engine in the stop)
//
// One server ("server-1u") with the files of the checks; scripts cost 5 ns per instruction.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value as Json;
use jfn_model::files::File;
use jfn_model::project::{Project, ServiceDef};
use jfn_sim::{Action, DebugOp, Engine, ScriptService, ScriptState, Service, MS};
use script::{PauseReason, Step};

use crate::selftest::T;

const BUDGET: usize = 20_000_000;
const SEED: u64 = 31;

fn s(x: &str) -> String {
    String::from(x)
}

/// an engine with one server and these files (their folders are made)
fn eng(files: &[(&str, &str)]) -> (Engine, u32) {
    let mut p = Project::new("modules", SEED);
    let h = p.add_device("server-1u", 0, 0).unwrap_or(0);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let mut all = e.files(h);
    for (path, text) in files {
        let mut dir = String::new();
        for part in path.split('/').filter(|x| !x.is_empty()).collect::<Vec<_>>().split_last().map_or(&[][..], |x| x.1) {
            dir.push('/');
            dir.push_str(part);
            if !all.iter().any(|f| f.path == dir) {
                all.push(File { path: dir.clone(), data: Vec::new(), mtime: 0, dir: true });
            }
        }
        all.push(File { path: s(path), data: Vec::from(text.as_bytes()), mtime: 0, dir: false });
    }
    let _ = e.apply(Action::Files { dev: h, files: all });
    (e, h)
}

/// adds a script (more: further members of its config, "\"disk\": {..}"); true when taken
fn script(e: &mut Engine, dev: u32, name: &str, src: &str, more: &str) -> bool {
    let mut c = Json::Object(Vec::new());
    c.insert("source", Json::string(src));
    if let Ok(Json::Object(m)) = agent_core::json::parse(format!("{{{}}}", more).as_bytes()) {
        for (k, v) in m {
            c.insert(&k, v);
        }
    }
    e.apply(Action::Service { dev, def: ServiceDef { kind: s("script"), name: s(name), disabled: false, config: c } }).is_ok()
}

fn sv<'a>(e: &'a Engine, dev: u32, name: &str) -> Option<&'a ScriptService> {
    e.service::<ScriptService>(dev, name)
}

fn output(e: &Engine, dev: u32, name: &str) -> String {
    sv(e, dev, name).map(|x| s(x.output())).unwrap_or_default()
}

/// (time, text) of the lines a service wrote to the log
fn logs(e: &Engine, dev: u32, tag: &str) -> Vec<(u64, String)> {
    e.log_lines(dev).into_iter().filter(|l| l.tag == tag).map(|l| (l.t, l.text)).collect()
}

fn debug(e: &mut Engine, dev: u32, name: &str, op: DebugOp) -> bool {
    e.apply(Action::ScriptDebug { dev, svc: s(name), op }).is_ok()
}

const UTIL: &str = "let greeting = \"hallo\"\nlet helper = import(\"helper.jfs\")\nfn twice(x) {\n  return helper.k * x\n}\nfn ratio(a, b) {\n  return a / b\n}";

fn modules(t: &mut T) {
    let files = [
        ("/home/user/lib/util.jfs", UTIL),
        ("/home/user/lib/helper.jfs", "let k = 2"),
        ("/root/lib/rel.jfs", "let where = \"root\""),
        ("/home/user/a.jfs", "let b = import(\"b.jfs\")"),
        ("/home/user/b.jfs", "let a = import(\"a.jfs\")"),
    ];
    let (mut e, h) = eng(&files);
    // absolute, relative to /root, relative to the importing module; the disk's time
    let src = "let u = import(\"/home/user/lib/util.jfs\")\nlet r = import(\"lib/rel.jfs\")\nprint(u.greeting, u.twice(21), r.where)";
    t.check("import: taken", script(&mut e, h, "imp", src, "\"disk\": {\"latency_us\": 2000}"));
    e.run_until(100 * MS, BUDGET);
    t.eq("import: the device's files", output(&e, h, "imp").as_str(), "hallo 42 root\n");
    let view = sv(&e, h, "imp").map(|x| x.view(e.now()));
    t.check(&format!("import: the reads take the disk's time ({:?})", view.as_ref().map(|v| v.busy_ns)), view.is_some_and(|v| v.busy_ns >= 6_000_000 && v.state == ScriptState::Running));
    // a missing file: caught, and uncaught at the start (the service ends)
    t.check("missing: caught", script(&mut e, h, "miss1", "try {\n  import(\"/home/user/nope.jfs\")\n} catch e {\n  print(e)\n}", ""));
    t.check("missing: uncaught", script(&mut e, h, "miss2", "let m = import(\"/home/user/nope.jfs\")", ""));
    e.run_until(200 * MS, BUDGET);
    t.eq("missing: the message", output(&e, h, "miss1").as_str(), "Modul /home/user/nope.jfs: die Datei gibt es nicht\n");
    let err = sv(&e, h, "miss2").and_then(|x| x.errors().first().cloned());
    t.eq("missing: the service's error", err.map(|x| (x.file, x.line, x.col, x.msg)), Some((s(""), 1, 9, s("Modul /home/user/nope.jfs: die Datei gibt es nicht"))));
    t.eq("missing: failed", sv(&e, h, "miss2").map(|x| x.state()), Some(ScriptState::Failed));
    // a cycle: the error is in the file whose import closes it
    t.check("cycle: taken", script(&mut e, h, "cyc", "import(\"/home/user/a.jfs\")", ""));
    e.run_until(300 * MS, BUDGET);
    let err = sv(&e, h, "cyc").and_then(|x| x.errors().first().cloned());
    let want = "Zyklischer Import: /home/user/a.jfs \u{2192} /home/user/b.jfs \u{2192} /home/user/a.jfs";
    t.eq("cycle: the error", err.as_ref().map(|x| (x.file.as_str(), x.msg.as_str())), Some(("/home/user/b.jfs", want)));
    let status = sv(&e, h, "cyc").map(|x| x.status_msg().text()).unwrap_or_default();
    t.check(&format!("cycle: the status names the file: {}", status), status.contains("/home/user/b.jfs, Zeile 1, Spalte 9"));
    // an error in a module's function, in a timer: file, line, column in the error and the log
    let src = "let u = import(\"/home/user/lib/util.jfs\")\ntimer.after(10, fn() {\n  u.ratio(1, 0)\n})";
    t.check("module error: taken", script(&mut e, h, "div", src, ""));
    e.run_until(400 * MS, BUDGET);
    let err = sv(&e, h, "div").and_then(|x| x.errors().first().cloned());
    t.eq("module error: where", err.as_ref().map(|x| (x.file.as_str(), x.line, x.col, x.during)), Some(("/home/user/lib/util.jfs", 7, 12, "timer")));
    let trace: Vec<String> = err.map(|x| x.trace.iter().map(|l| format!("{}@{}:{}", l.func, l.file, l.line)).collect()).unwrap_or_default();
    t.eq("module error: the calls", trace.join(" "), s("ratio@/home/user/lib/util.jfs:7 fn@:3"));
    let log = logs(&e, h, "div");
    t.check(&format!("module error: the log line ({:?})", log), log.iter().any(|l| l.1.contains("Fehler in /home/user/lib/util.jfs, Zeile 7, Spalte 12 (timer): Division durch 0")));
    t.eq("module error: still running", sv(&e, h, "div").map(|x| x.state()), Some(ScriptState::Running));
}

const TICK: &str = "let n = 0\ntimer.every(100, fn() {\n  n += 1\n  log(\"tick\", n)\n})";

/// the ticks so far: (time in ms, n)
fn ticks(e: &Engine, dev: u32) -> Vec<(u64, String)> {
    logs(e, dev, "tick").into_iter().map(|(t, x)| (t / MS, x)).collect()
}

/// the pause scenario: everything it logged and printed, for the determinism check
fn pause_run(t: &mut T, check: bool) -> String {
    let (mut e, h) = eng(&[]);
    let _ = script(&mut e, h, "tick", TICK, "");
    e.run_until(1_050 * MS, BUDGET);
    let n0 = ticks(&e, h).len();
    // stop: the next round's callback stops at its first statement and waits
    let paused = debug(&mut e, h, "tick", DebugOp::Pause);
    e.run_until(2_000 * MS, BUDGET);
    let stop = sv(&e, h, "tick").and_then(|x| x.paused());
    let (frames, eval) = match sv(&e, h, "tick") {
        Some(x) => (x.debug_vm().map(|v| v.frames().len()).unwrap_or(0), x.debug_eval(0, "n * 10")),
        None => (0, Err(s("-"))),
    };
    let view = sv(&e, h, "tick").map(|x| x.view(e.now()));
    let status = sv(&e, h, "tick").map(|x| x.status_msg().text()).unwrap_or_default();
    let n1 = ticks(&e, h).len();
    // a step over the first statement, then on
    let stepped = debug(&mut e, h, "tick", DebugOp::Step(Step::Over));
    let stop2 = sv(&e, h, "tick").and_then(|x| x.paused());
    let eval2 = sv(&e, h, "tick").map(|x| x.debug_eval(0, "n"));
    let on = debug(&mut e, h, "tick", DebugOp::Continue);
    e.run_until(2_550 * MS, BUDGET);
    let after = ticks(&e, h);
    // a breakpoint with a condition; a clone of the engine in the stop goes on the same way
    let br = debug(&mut e, h, "tick", DebugOp::Break { file: s(""), line: 4, cond: Some(s("n % 5 == 0")) });
    e.run_until(4_000 * MS, BUDGET);
    let stop3 = sv(&e, h, "tick").and_then(|x| x.paused());
    let at3 = sv(&e, h, "tick").and_then(|x| x.debug_eval(0, "n").ok());
    let mut c = e.clone();
    let _ = debug(&mut e, h, "tick", DebugOp::ClearBreaks);
    let _ = debug(&mut e, h, "tick", DebugOp::Continue);
    let _ = debug(&mut c, h, "tick", DebugOp::ClearBreaks);
    let _ = debug(&mut c, h, "tick", DebugOp::Continue);
    e.run_until(5_000 * MS, BUDGET);
    c.run_until(5_000 * MS, BUDGET);
    if check {
        t.eq("pause: before", n0, 10);
        t.check("pause: taken", paused);
        t.eq("pause: the stop", stop.as_ref().map(|p| (p.reason, p.file.as_str(), p.line, p.col)), Some((PauseReason::Pause, "", 3, 3)));
        t.eq("pause: no rounds while stopped", n1, 10);
        // (the timer's callback is the only call: it is the invocation)
        t.eq("pause: the stack and an expression", (frames, eval), (1, Ok(s("100"))));
        t.check("pause: the view", view.is_some_and(|v| v.paused && v.busy));
        t.eq("pause: the status", status.as_str(), "angehalten in Zeile 3, Spalte 3");
        t.check("step: taken", stepped);
        t.eq("step: the next statement", stop2.map(|p| (p.reason, p.line)), Some((PauseReason::Step, 4)));
        t.eq("step: the variable", eval2, Some(Ok(s("11"))));
        t.check("continue: taken", on);
        // the stopped round ends at 2 s, the round that fell due meanwhile comes late, then the
        // timer's own rhythm
        let late: Vec<u64> = after.iter().skip(10).map(|x| x.0).collect();
        t.eq("continue: late rounds, then on time", late, Vec::from([2000, 2000, 2100, 2200, 2300, 2400, 2500]));
        t.eq("continue: counted on", after.last().map(|x| x.1.clone()), Some(s("tick 17")));
        t.check("breakpoint: taken", br);
        t.eq("breakpoint: condition", stop3.map(|p| (p.reason, p.line)), Some((PauseReason::Breakpoint, 4)));
        t.eq("breakpoint: at n = 20", at3, Some(s("20")));
        t.eq("clone: the same", ticks(&c, h), ticks(&e, h));
        t.check("after: running", sv(&e, h, "tick").is_some_and(|x| x.paused().is_none() && x.state() == ScriptState::Running));
        // refusals
        t.check("refused: no such service", !debug(&mut e, h, "nope", DebugOp::Pause));
        t.check("refused: a bad condition", !debug(&mut e, h, "tick", DebugOp::Break { file: s(""), line: 3, cond: Some(s("n ==")) }));
        let _ = e.apply(Action::ServiceRun { dev: h, name: s("tick"), on: false });
        e.run_until(5_100 * MS, BUDGET);
        t.check("refused: stopped", !debug(&mut e, h, "tick", DebugOp::Pause) && debug(&mut e, h, "tick", DebugOp::Break { file: s(""), line: 3, cond: None }));
        t.eq("breakpoints stay with the service", sv(&e, h, "tick").map(|x| x.breaks().len()), Some(1));
    }
    let mut all = String::new();
    for (tm, x) in ticks(&e, h) {
        all.push_str(&format!("{} {}\n", tm, x));
    }
    all
}

fn pause(t: &mut T) {
    let a = pause_run(t, true);
    let b = pause_run(t, false);
    t.check("determinism: twice the same", !a.is_empty() && a == b);
}

pub fn run(t: &mut T) {
    modules(t);
    pause(t);
}
