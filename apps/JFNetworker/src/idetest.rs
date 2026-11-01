// idetest -- checks of what the IDE builds on (part of --selftest, group "ide": jfn_sim's script
// services from files, `jfs --test` in cli/jfstest.rs, doc/ide.rs, the IDE's search): a service
// started from a file, a missing file, a compile error naming the file (status, `script
// errors`, the problem), a restart after the file changed (saving alone changes nothing),
// imports beside the file, the conversion of an inline script into a file and its undo, the
// steps on files (save, new file and folder, rename, delete, a service bound to a file),
// replace in files as one undo step, `jfs --test` with a passing and a failing test (the
// terminal, the record, the exit code), determinism (twice the same), and an old project
// with "source" unchanged (it loads, runs and saves as before)
//
// SRV1 is a "server-1u"; scripts cost 5 ns per instruction.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::files::File;
use jfn_model::project::{Project, ServiceDef};
use jfn_sim::{Action, Cmd, Engine, Report, ScriptService, ScriptState, Service, UnitOp, UnitState, MS, SEC};

use crate::doc::ide as dide;
use crate::doc::Doc;
use crate::selftest::T;
use crate::workspace::ide::search;

const BUDGET: usize = 20_000_000;

fn s(x: &str) -> String {
    String::from(x)
}

const LIB: &str = "/// verdoppelt\nfn double(x) {\n  return x * 2\n}\nlet name = \"lib\"\n";
const MAIN: &str = "let lib = import(\"lib.jfs\")\nprint(\"Start\", lib.double(21), lib.name)\n";
const TESTS: &str = "let lib = import(\"lib.jfs\")\nfn test_double() {\n  assert_eq(lib.double(2), 4)\n}\nfn helper() {\n  return 1\n}\nfn test_wrong() {\n  print(\"vorher\")\n  assert_eq(lib.double(2), 5, \"zwei mal zwei\")\n}\nfn test_later() {\n  let done = false\n  timer.after(50, fn() {\n    print(\"nach 50 ms\")\n  })\n}\n";

/// the files of SRV1 (their folders are made)
fn files(list: &[(&str, &str)]) -> Vec<File> {
    let mut all: Vec<File> = Vec::new();
    for (path, text) in list {
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
    all
}

fn script(name: &str, config: Value) -> ServiceDef {
    ServiceDef { kind: s("script"), name: s(name), disabled: false, config }
}

/// a project with SRV1, its files and services
fn project(list: &[(&str, &str)], svcs: Vec<ServiceDef>) -> (Project, u32) {
    let mut p = Project::new("ide", 41);
    let h = p.add_device("server-1u", 0, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(h) {
        d.startup_config = s("hostname SRV1\nip addr add 10.0.0.1/24 dev eth0\n");
        d.files = files(list);
        d.services = svcs;
    }
    (p, h)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

fn sv<'a>(e: &'a Engine, dev: u32, name: &str) -> Option<&'a ScriptService> {
    e.service::<ScriptService>(dev, name)
}

fn output(e: &Engine, dev: u32, name: &str) -> String {
    sv(e, dev, name).map(|x| s(x.output())).unwrap_or_default()
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// a line at the device's shell, then `d` of virtual time: what it printed
fn sh(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    run_for(e, d);
    out(e, dev)
}

fn file_cfg(path: &str) -> Value {
    Value::object([("file", Value::string(path))])
}

pub(crate) fn run(t: &mut T) {
    from_file(t);
    missing(t);
    broken(t);
    restart(t);
    convert(t);
    steps(t);
    unit_file(t);
    replace(t);
    tests(t);
    determinism(t);
    old_project(t);
    find(t);
}

// a service from a file: it reads the file, its imports start beside it, config.file names it
fn from_file(t: &mut T) {
    let (p, h) = project(&[("/root/app/main.jfs", MAIN), ("/root/app/lib.jfs", LIB)], alloc::vec![script("main", file_cfg("/root/app/main.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 200 * MS);
    t.eq("from a file: it ran", output(&e, h, "main"), s("Start 42 lib\n"));
    t.eq("from a file: its file", sv(&e, h, "main").and_then(|x| x.file().map(String::from)), Some(s("/root/app/main.jfs")));
    t.eq("from a file: the text it read", sv(&e, h, "main").map(|x| s(x.source())), Some(s(MAIN)));
    t.check("from a file: no problem", !e.problems().iter().any(|x| x.kind == jfn_sim::ProblemKind::ScriptFile));
    // config.file, and a path relative to /root
    let (p, h) = project(&[("/root/app/c.jfs", "print(config.file, config.port)\n")], alloc::vec![script("c", Value::object([("file", Value::string("app/c.jfs")), ("port", Value::u64(8080))]))]);
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    t.eq("relative to /root, config.file", output(&e, h, "c"), s("app/c.jfs 8080\n"));
    // "source" and "file" together are refused
    let both = Value::object([("file", Value::string("/root/x.jfs")), ("source", Value::string("print(1)"))]);
    let r = jfn_sim::scriptsvc::from_def(&both).err();
    t.eq("source and file: refused", r, Some(s("Ein Skript hat \u{201e}source\u{201c} oder \u{201e}file\u{201c}, nicht beides.")));
}

// a missing file: the service's error names it (status, errors, script errors, the problem)
fn missing(t: &mut T) {
    let (p, h) = project(&[], alloc::vec![script("weg", file_cfg("/root/app/weg.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    let x = sv(&e, h, "weg");
    t.eq("missing: failed", x.map(|x| x.state()), Some(ScriptState::Failed));
    let err = x.and_then(|x| x.errors().last().cloned());
    t.eq("missing: the error", err.map(|e| (e.file, e.line, e.msg, e.during)), Some((s("/root/app/weg.jfs"), 0, s("/root/app/weg.jfs: die Datei gibt es nicht"), "start")));
    t.eq("missing: the status", x.map(|x| x.status_msg().text()), Some(s("beendet nach einem Fehler: /root/app/weg.jfs: die Datei gibt es nicht")));
    let o = sh(&mut e, h, "script errors weg", 0);
    t.check(&format!("missing: script errors ({:?})", o), o.contains("(start): /root/app/weg.jfs: die Datei gibt es nicht"));
    let pr: Vec<String> = e.problems().into_iter().filter(|x| x.kind == jfn_sim::ProblemKind::ScriptFile).map(|x| x.text).collect();
    t.eq("missing: the problem", pr, alloc::vec![s("Skript weg auf SRV1: die Datei /root/app/weg.jfs gibt es nicht.")]);
    let log = e.log_lines(h).into_iter().any(|l| l.tag == "weg" && l.text.contains("Fehler (start): /root/app/weg.jfs: die Datei gibt es nicht"));
    t.check("missing: the log", log);
    // the file comes: a start reads it
    let _ = e.apply(Action::Files { dev: h, files: files(&[("/root/app/weg.jfs", "print(\"da\")\n")]) });
    t.check("missing: no problem once there", !e.problems().iter().any(|x| x.kind == jfn_sim::ProblemKind::ScriptFile));
    let _ = e.apply(Action::ServiceRun { dev: h, name: s("weg"), on: true });
    run_for(&mut e, 100 * MS);
    t.eq("missing: started once there", (sv(&e, h, "weg").map(|x| x.state()), output(&e, h, "weg")), (Some(ScriptState::Running), s("da\n")));
}

// a file that does not compile: file, line and column in the error, the status, the problem
fn broken(t: &mut T) {
    let (p, h) = project(&[("/root/app/bad.jfs", "print(1)\nlet = 2\n")], alloc::vec![script("bad", file_cfg("/root/app/bad.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    let x = sv(&e, h, "bad");
    let err = x.and_then(|x| x.errors().last().cloned());
    t.eq("broken: the error", err.map(|e| (e.file, e.line, e.col)), Some((s("/root/app/bad.jfs"), 2, 5)));
    let st = x.map(|x| x.status_msg().text()).unwrap_or_default();
    t.check(&format!("broken: the status ({})", st), st.starts_with("beendet nach einem Fehler in /root/app/bad.jfs, Zeile 2, Spalte 5: "));
    let o = sh(&mut e, h, "script errors bad", 0);
    t.check(&format!("broken: script errors ({:?})", o), o.contains("/root/app/bad.jfs, Zeile 2, Spalte 5 (start): "));
    let pr = e.problems().into_iter().find(|x| x.kind == jfn_sim::ProblemKind::ScriptFile).map(|x| x.text).unwrap_or_default();
    t.check(&format!("broken: the problem ({})", pr), pr.starts_with("Skript bad auf SRV1: /root/app/bad.jfs, Zeile 2, Spalte 5: "));
    // a runtime error in the file names it too
    let (p, h) = project(&[("/root/app/div.jfs", "let a = 1\nlet b = a / 0\n")], alloc::vec![script("div", file_cfg("/root/app/div.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    let err = sv(&e, h, "div").and_then(|x| x.errors().last().cloned());
    t.eq("runtime error: its file", err.map(|e| (e.file, e.line, e.col, e.during)), Some((s("/root/app/div.jfs"), 2, 11, "start")));
}

// saving the file changes nothing in the running program; a restart reads it
fn restart(t: &mut T) {
    let (p, h) = project(&[("/root/app/v.jfs", "print(\"eins\")\ntimer.every(1000, fn() {})\n")], alloc::vec![script("v", file_cfg("/root/app/v.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    t.eq("restart: the first text", output(&e, h, "v"), s("eins\n"));
    let _ = e.apply(Action::Files { dev: h, files: files(&[("/root/app/v.jfs", "print(\"zwei\")\ntimer.every(1000, fn() {})\n")]) });
    run_for(&mut e, 100 * MS);
    t.eq("restart: saving alone changes nothing", (output(&e, h, "v"), sv(&e, h, "v").map(|x| x.state())), (s("eins\n"), Some(ScriptState::Running)));
    let _ = e.apply(Action::ServiceRun { dev: h, name: s("v"), on: false });
    let _ = e.apply(Action::ServiceRun { dev: h, name: s("v"), on: true });
    run_for(&mut e, 100 * MS);
    let run = sv(&e, h, "v").map(|x| String::from(x.run_output(0).0)).unwrap_or_default();
    t.eq("restart: the saved text", run, s("zwei\n"));
    t.eq("restart: two runs", sv(&e, h, "v").map(|x| x.view(e.now()).runs), Some(2));
}

/// the engine runs on by `ms` of virtual time
fn go(doc: &mut Doc, ms: u64) {
    doc.sim.with_engine("test: run", move |e| {
        let t = e.now() + ms * MS;
        e.run_until(t, BUDGET);
    });
}

fn doc_of(p: Project) -> Doc {
    let mut doc = Doc::new();
    doc.replace(p, None);
    doc
}

fn proj_text(doc: &Doc, dev: u32, path: &str) -> Option<String> {
    dide::store_of(&doc.proj, dev).and_then(|st| dide::text_of(&st, path))
}

// In Datei umwandeln: one undo step; the file has the source, the service runs from it and
// keeps its disk; undo brings the inline source back
fn convert(t: &mut T) {
    let count = ["disk.get(\"starts\", fn(err, v) {", "  let n = (v ?? 0) + 1", "  disk.put(\"starts\", n)", "  metrics.gauge(\"starts\", n)", "})"];
    let src = Value::Array(count.iter().map(|l| Value::string(*l)).collect());
    let (p, h) = project(&[], alloc::vec![script("zaehler", Value::object([("source", src), ("port", Value::u64(9))]))]);
    let mut doc = doc_of(p);
    go(&mut doc, 50);
    let starts = |doc: &Doc| doc.sim.engine().metric(h, "zaehler", "starts").unwrap_or(-1);
    t.eq("convert: it counts", starts(&doc), 1);
    let steps = doc.hist.len();
    let r = dide::convert(&mut doc, h, "zaehler");
    t.eq("convert: the path", r.clone(), Ok(s("/root/scripts/zaehler.jfs")));
    t.eq("convert: one step", doc.hist.len(), steps + 1);
    go(&mut doc, 50);
    t.eq("convert: the file has the source", proj_text(&doc, h, "/root/scripts/zaehler.jfs"), Some(format!("{}\n", count.join("\n"))));
    let cfg = doc.proj.device(h).map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    t.eq("convert: the definition names the file", (cfg.get("file").cloned(), cfg.get("source").is_some(), cfg.get("port").cloned()), (Some(Value::string("/root/scripts/zaehler.jfs")), false, Some(Value::u64(9))));
    t.eq("convert: it runs from the file, its disk stayed", (doc.sim.engine().service::<ScriptService>(h, "zaehler").and_then(|x| x.file().map(String::from)), starts(&doc)), (Some(s("/root/scripts/zaehler.jfs")), 2));
    t.check("convert: a second time refused", dide::convert(&mut doc, h, "zaehler").is_err());
    // the default path when it is taken
    let st = dide::store_of(&doc.proj, h).unwrap_or_else(|| jfn_model::files::Store::new(false, 0, None));
    t.eq("convert: a free path", dide::default_path(&st, "zaehler"), s("/root/scripts/zaehler2.jfs"));
    t.check("convert: undo", doc.undo());
    go(&mut doc, 50);
    let cfg = doc.proj.device(h).map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    t.eq("undo: inline again, the file gone", (cfg.get("source").is_some(), cfg.get("file").is_some(), proj_text(&doc, h, "/root/scripts/zaehler.jfs")), (true, false, None));
    t.eq("undo: it runs inline, the disk counts on", (doc.sim.engine().service::<ScriptService>(h, "zaehler").map(|x| x.file().is_none()), starts(&doc)), (Some(true), 3));
    t.check("convert: redo", doc.redo());
    go(&mut doc, 50);
    t.eq("redo: from the file again", (proj_text(&doc, h, "/root/scripts/zaehler.jfs").is_some(), starts(&doc)), (true, 4));
}

// the steps on files: each one undo step; a service bound to a file
fn steps(t: &mut T) {
    let (p, h) = project(&[("/root/app/a.jfs", "print(\"a\")\n")], Vec::new());
    let mut doc = doc_of(p);
    let n0 = doc.hist.len();
    t.eq("save", dide::save(&mut doc, h, "/root/app/a.jfs", "print(\"A\")\n"), Ok(true));
    t.eq("save: the same again", dide::save(&mut doc, h, "/root/app/a.jfs", "print(\"A\")\n"), Ok(false));
    t.eq("save: the engine has it", doc.sim.engine().file_store(h).and_then(|st| dide::text_of(st, "/root/app/a.jfs")), Some(s("print(\"A\")\n")));
    t.eq("new file", dide::new_file(&mut doc, h, "/root/app/b.jfs", ""), Ok(s("/root/app/b.jfs")));
    t.check("new file: taken", dide::new_file(&mut doc, h, "/root/app/b.jfs", "").is_err());
    t.check("new file: no folder", dide::new_file(&mut doc, h, "/root/nix/b.jfs", "").is_err());
    t.eq("new folder", dide::new_dir(&mut doc, h, "/root/app/lib"), Ok(s("/root/app/lib")));
    t.eq("rename", dide::rename(&mut doc, h, "/root/app/b.jfs", "/root/app/lib/b.jfs"), Ok(s("/root/app/lib/b.jfs")));
    t.eq("rename a folder", dide::rename(&mut doc, h, "/root/app/lib", "/root/app/mod"), Ok(s("/root/app/mod")));
    t.check("renamed with its file", proj_text(&doc, h, "/root/app/mod/b.jfs").is_some());
    t.eq("delete", dide::remove(&mut doc, h, "/root/app/mod"), Ok(()));
    t.check("deleted with its file", proj_text(&doc, h, "/root/app/mod/b.jfs").is_none());
    t.eq("six steps", doc.hist.len(), n0 + 6);
    t.check("undo the delete", doc.undo() && proj_text(&doc, h, "/root/app/mod/b.jfs").is_some());
    // a service bound to a file: made, it runs from the file
    t.eq("bind", dide::bind(&mut doc, h, "/root/app/a.jfs"), Ok(s("a")));
    go(&mut doc, 50);
    t.eq("bind: it runs", doc.sim.engine().service::<ScriptService>(h, "a").map(|x| s(x.output())), Some(s("A\n")));
    let d = doc.proj.device(h).cloned().unwrap_or_default();
    let st = dide::store_of(&doc.proj, h).unwrap_or_else(|| jfn_model::files::Store::new(false, 0, None));
    t.eq("bound", dide::bound(&d, &st, "/root/app/a.jfs"), alloc::vec![s("a")]);
    t.eq("bind again: another name", dide::bind(&mut doc, h, "/root/app/a.jfs"), Ok(s("a2")));
}

const SERVER: &str = "http.serve(80, fn(req, res) {\n  res.send(200, \"eins\\n\")\n})\nprint(\"bereit\")\n";
const UNIT: &str = "[Service]\nExecStart=jfs /root/app/server.jfs\nRestart=on-failure\n";

// a file that runs as a unit: the unit holds it (F5 starts the unit anew, debug.rs), a second
// service of the file would fail at the port; which unit holds it in which state
fn unit_file(t: &mut T) {
    let path = "/root/app/server.jfs";
    let bad = "/root/app/bad.jfs";
    let (p, h) = project(&[(path, SERVER), (bad, "fehlt_hier()\n"), ("/etc/systemd/system/web.service", UNIT), ("/etc/systemd/system/bad.service", "[Service]\nExecStart=jfs /root/app/bad.jfs\n")], Vec::new());
    let mut doc = doc_of(p);
    go(&mut doc, 50);
    let holder = |doc: &Doc, path: &str, last: Option<&str>| {
        let d = doc.proj.device(h).cloned().unwrap_or_default();
        let st = dide::store_of(&doc.proj, h).unwrap_or_else(|| jfn_model::files::Store::new(false, 0, None));
        dide::unit_of(doc.sim.engine(), &d, &st, path, last)
    };
    let op = |doc: &mut Doc, name: &str, op: UnitOp| doc.sim.send(Cmd::Apply(Action::Unit { dev: h, name: s(name), op })).is_ok();
    let state = |doc: &Doc, name: &str| doc.sim.engine().unit(h, name).map(|u| u.state);
    t.eq("unit: it never ran: nothing holds the file", holder(&doc, path, None), None);
    t.check("unit: start", op(&mut doc, "web", UnitOp::Start));
    go(&mut doc, 50);
    t.eq("unit: it runs", state(&doc, "web"), Some(UnitState::Active));
    t.eq("unit: it holds the file", holder(&doc, path, None), Some(s("web")));
    t.eq("unit: not another one", holder(&doc, "/root/app/andere.jfs", None), None);
    t.eq("unit: the list", doc.sim.engine().units_of_file(h, path), alloc::vec![(s("web"), UnitState::Active)]);
    // F5: the file saved, the unit starts anew, with the new text, and the project stays as it was
    t.eq("unit: saved", dide::save(&mut doc, h, path, &SERVER.replace("eins", "zwei")), Ok(true));
    let steps = doc.hist.len();
    t.check("unit: restart", op(&mut doc, "web", UnitOp::Restart));
    go(&mut doc, 50);
    t.eq("unit: a second run", sv(doc.sim.engine(), h, "web").map(|x| x.view(0).runs), Some(2));
    t.check("unit: the saved text runs", sv(doc.sim.engine(), h, "web").is_some_and(|x| x.state() == ScriptState::Running && x.source().contains("zwei")));
    t.eq("unit: no new service, no step", (doc.proj.device(h).map(|d| d.services.len()), doc.hist.len()), (Some(0), steps));
    // stopped: the IDE's own target only
    t.check("unit: stop", op(&mut doc, "web", UnitOp::Stop));
    go(&mut doc, 50);
    t.eq("unit: stopped: not for a stranger", holder(&doc, path, None), None);
    t.eq("unit: stopped: the IDE's target", holder(&doc, path, Some("web")), Some(s("web")));
    t.check("unit: a restart starts a stopped one", op(&mut doc, "web", UnitOp::Restart));
    go(&mut doc, 50);
    t.eq("unit: it runs again", state(&doc, "web"), Some(UnitState::Active));
    // failed: it holds the file, so the fix goes on with F5
    t.check("unit: the other one", op(&mut doc, "bad", UnitOp::Start));
    go(&mut doc, 50);
    t.eq("unit: it failed", state(&doc, "bad"), Some(UnitState::Failed));
    t.eq("unit: a failed one holds the file", holder(&doc, bad, None), Some(s("bad")));
    t.eq("unit: fixed", dide::save(&mut doc, h, bad, "timer.after(100000, fn() { })\n"), Ok(true));
    t.check("unit: a restart starts a failed one", op(&mut doc, "bad", UnitOp::Restart));
    go(&mut doc, 50);
    t.eq("unit: it runs now", state(&doc, "bad"), Some(UnitState::Active));
    // why: a service of the file next to the unit fails at the port; the running unit still comes first
    t.eq("unit: a second service", dide::bind(&mut doc, h, path), Ok(s("server")));
    go(&mut doc, 50);
    let second = sv(doc.sim.engine(), h, "server").map(|x| (x.state(), x.errors().first().map(|e| e.msg.clone())));
    t.eq("unit: a second service fails at the port", second, Some((ScriptState::Failed, Some(s("http.serve: Port 80 ist schon belegt.")))));
    t.eq("unit: the running one comes first", holder(&doc, path, None), Some(s("web")));
    // a bound service comes before a unit that does not run
    t.check("unit: stop again", op(&mut doc, "web", UnitOp::Stop));
    go(&mut doc, 50);
    t.eq("unit: stopped, a service bound", holder(&doc, path, Some("web")), None);
}

// replace in files: one undo step for every file
fn replace(t: &mut T) {
    let (p, h) = project(&[("/root/app/a.jfs", "let wert = 1\nprint(wert)\n"), ("/root/app/b.jfs", "let wert2 = wert\n"), ("/root/x.txt", "wert\n")], Vec::new());
    let mut doc = doc_of(p);
    let n0 = doc.hist.len();
    let mut out = Vec::new();
    for path in ["/root/app/a.jfs", "/root/app/b.jfs"] {
        let text = proj_text(&doc, h, path).unwrap_or_default();
        let (new, c) = search::replace(&text, "wert", "zahl", false, true);
        if c > 0 {
            out.push((s(path), new));
        }
    }
    t.eq("replace: two files", dide::save_many(&mut doc, h, &out, "„wert“ ersetzen"), Ok(2));
    t.eq("replace: one step", doc.hist.len(), n0 + 1);
    t.eq("replace: the texts", (proj_text(&doc, h, "/root/app/a.jfs"), proj_text(&doc, h, "/root/app/b.jfs")), (Some(s("let zahl = 1\nprint(zahl)\n")), Some(s("let wert2 = zahl\n"))));
    t.check("replace: undo", doc.undo());
    t.eq("undo: both back", (proj_text(&doc, h, "/root/app/a.jfs"), proj_text(&doc, h, "/root/app/b.jfs")), (Some(s("let wert = 1\nprint(wert)\n")), Some(s("let wert2 = wert\n"))));
}

// jfs --test: the terminal, the record, the exit code
fn tests(t: &mut T) {
    let (p, h) = project(&[("/root/app/lib.jfs", LIB), ("/root/app/test_lib.jfs", TESTS), ("/root/app/bad.jfs", "let = 1\n")], Vec::new());
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    let o = sh(&mut e, h, "cd app; jfs --test test_lib.jfs; echo $?", SEC);
    let want = "ok      test_double (test_lib.jfs)\nFEHLER  test_wrong (test_lib.jfs:10:3): zwei mal zwei: erwartet 5, ist 4\n        | vorher\nok      test_later (test_lib.jfs)\n3 Tests: 2 bestanden, 1 fehlgeschlagen\n1\n";
    t.eq("jfs --test: the report", o.clone(), s(want));
    let r = e.jfs_tests(h).cloned().unwrap_or_default();
    t.eq("jfs --test: the tests found", r.tests.iter().map(|x| (x.1.clone(), x.2)).collect::<Vec<_>>(), alloc::vec![(s("test_double"), 2), (s("test_wrong"), 8), (s("test_later"), 12)]);
    t.eq("jfs --test: done, counts", (r.done, r.counts()), (true, (2, 1)));
    let bad = r.results.iter().find(|x| !x.ok).cloned();
    t.eq("jfs --test: the failure", bad.map(|x| (x.name, x.at, x.output)), Some((s("test_wrong"), Some((s("/root/app/test_lib.jfs"), 10, 3)), s("vorher\n"))));
    let later = r.results.iter().find(|x| x.name == "test_later").map(|x| x.output.clone());
    t.eq("jfs --test: a test's timers run", later, Some(s("nach 50 ms\n")));
    // a file that does not compile, no tests at all
    let o = sh(&mut e, h, "jfs --test bad.jfs lib.jfs; echo $?", SEC);
    t.check(&format!("jfs --test: does not compile ({:?})", o), o.starts_with("FEHLER  bad.jfs lässt sich nicht übersetzen\nbad.jfs:1:5: Fehler: ") && o.contains("lib.jfs: keine Tests (Funktionen test_…)\n1 Test: 0 bestanden, 1 fehlgeschlagen\n1\n"));
    let r = e.jfs_tests(h).cloned().unwrap_or_default();
    t.eq("jfs --test: the record of it", (r.done, r.results.len(), r.results.first().map(|x| x.at.clone())), (true, 1, Some(Some((s("/root/app/bad.jfs"), 1, 5)))));
    let o = sh(&mut e, h, "jfs --test; echo $?", 0);
    t.eq("jfs --test: no file", o, s("jfs --test: welche Dateien? Aufruf: jfs --test DATEI.jfs ...\n2\n"));
    // a test that serves forever ends at its limit and passes; the next test gets the port
    let srv = "fn test_a() {\n  net.udp(5000, fn(s, m) {})\n}\nfn test_b() {\n  net.udp(5000, fn(s, m) {})\n  print(\"frei\")\n}\n";
    let _ = e.apply(Action::Files { dev: h, files: files(&[("/root/app/srv.jfs", srv)]) });
    let o = sh(&mut e, h, "jfs --test srv.jfs; echo $?", 25 * SEC);
    t.eq("jfs --test: the limit, the port again", o, s("ok      test_a (srv.jfs)\nok      test_b (srv.jfs)\n2 Tests: 2 bestanden, 0 fehlgeschlagen\n0\n"));
}

/// a scenario with a service from a file, an import, a restart and a test run: what it did
fn scenario() -> String {
    let (p, h) = project(&[("/root/app/main.jfs", "let lib = import(\"lib.jfs\")\ntimer.every(300, fn() {\n  print(lib.double(rand(100)))\n})\n"), ("/root/app/lib.jfs", LIB), ("/root/app/test_lib.jfs", TESTS)], alloc::vec![script("main", file_cfg("/root/app/main.jfs"))]);
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let _ = e.apply(Action::ServiceRun { dev: h, name: s("main"), on: false });
    let _ = e.apply(Action::ServiceRun { dev: h, name: s("main"), on: true });
    let o = sh(&mut e, h, "jfs --test /root/app/test_lib.jfs", SEC);
    run_for(&mut e, SEC);
    format!("{}|{}|{:?}|{:x}", output(&e, h, "main"), o, e.jfs_tests(h), jfn_sim::keyframe::digest(&e))
}

// the same twice
fn determinism(t: &mut T) {
    let a = scenario();
    t.check("determinism: something ran", a.contains("ok      test_double"));
    t.eq("determinism: twice the same", scenario(), a);
}

// an old project with "source": it loads, runs and saves as before
fn old_project(t: &mut T) {
    let text = include_str!("../tests/fixtures/scripts.jfnet");
    let p = Project::from_text(text).unwrap_or_else(|_| Project::new("x", 1));
    let mut e = engine(&p);
    run_for(&mut e, 1500 * MS);
    let z = e.service::<ScriptService>(3, "zaehler");
    t.eq("old project: inline", z.map(|x| x.file().is_none()), Some(true));
    t.check("old project: it runs", output(&e, 3, "zaehler").starts_with("Start 1\nTakt 1\n"));
    t.eq("old project: saved as it was", Project::from_text(&p.to_text()).ok().map(|q| q.to_text()), Some(p.to_text()));
    t.check("old project: no file problem", !e.problems().iter().any(|x| x.kind == jfn_sim::ProblemKind::ScriptFile));
}

// the search of the IDE: matches with case and whole words, replace
fn find(t: &mut T) {
    let text = "let Wert = wert\nprint(wert2, wert)";
    t.eq("find: any case", search::find(text, "wert", false, false).len(), 4);
    t.eq("find: the case", search::find(text, "Wert", true, false), alloc::vec![(1, 5, 9)]);
    t.eq("find: whole words", search::find(text, "wert", false, true), alloc::vec![(1, 5, 9), (1, 12, 16), (2, 14, 18)]);
    t.eq("replace", search::replace(text, "wert", "x", true, true), (s("let Wert = x\nprint(wert2, x)"), 2));
    t.eq("replace: umlauts", search::replace("größe Größe", "GRÖSSE", "x", false, false).1, 0);
    t.eq("replace: wide", search::replace("äö äö", "äö", "ü", false, false), (s("ü ü"), 2));
}
