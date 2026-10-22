// scriptedtest -- checks of what the script editor builds on (part of --selftest, group
// "scripted"; doc/scripts.rs, doc/sim.rs keep_steps, devwin/scripts.rs): a script's source as
// the editor shows it and back, names, the templates (they compile, German and English), the
// steps through Doc -- a new source as one undo step that keeps the running script's disk and
// numbers and the device's running configuration (undo and redo too), a source that does not
// compile, a new script from a template, a rename and a removal with their undo, a script the
// simulator could not take from the project file --, keep_steps for what still restarts a
// device, and the marks of runtime errors on an edited text (map_line, word_at)
//
// SRV1 is device 1; its script "zaehler" counts its starts on the disk.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{Project, ServiceDef};
use jfn_sim::{Action, Cmd, ScriptService, MS};

use crate::doc::scripts::{self, KIND};
use crate::doc::sim::keep_steps;
use crate::doc::Doc;
use crate::selftest::T;
use crate::workspace::devwin::scripts::{map_line, word_at};

const BUDGET: usize = 10_000_000;
const SRV: u32 = 1;
const COUNT: &str = "disk.get(\"starts\", fn(err, v) {\n  let n = (v ?? 0) + 1\n  disk.put(\"starts\", n)\n  metrics.gauge(\"starts\", n)\n})";

fn def(name: &str, source: Value) -> ServiceDef {
    ServiceDef { kind: String::from(KIND), name: String::from(name), disabled: false, config: Value::object([("source", source)]) }
}

fn lines(text: &str) -> Value {
    Value::Array(text.split('\n').map(Value::string).collect())
}

/// SRV1 with the counting script (its source a list of lines) and one that does not compile
fn project() -> Project {
    let mut p = Project::new("skripte", 3);
    let srv = p.add_device("server-1u", 0, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(srv) {
        d.startup_config = String::from("hostname SRV1\nip addr add 10.0.0.1/24 dev eth0\n");
        d.services.push(def("zaehler", lines(COUNT)));
        d.services.push(def("kaputt", Value::string("let = 1")));
    }
    p
}

/// the engine runs on by `ms` of virtual time
fn go(doc: &mut Doc, ms: u64) {
    doc.sim.with_engine(|e| {
        let t = e.now() + ms * MS;
        e.run_until(t, BUDGET);
    });
}

fn metric(doc: &Doc, name: &str, m: &str) -> i64 {
    doc.sim.engine().metric(SRV, name, m).unwrap_or(-1)
}

fn engine_has(doc: &Doc, name: &str) -> bool {
    doc.sim.engine().service::<ScriptService>(SRV, name).is_some()
}

fn engine_source(doc: &Doc, name: &str) -> String {
    doc.sim.engine().service::<ScriptService>(SRV, name).map(|s| String::from(s.source())).unwrap_or_default()
}

fn svc_id(doc: &Doc, name: &str) -> Option<u32> {
    doc.sim.engine().service_ids(SRV).into_iter().find(|s| s.1 == name).map(|s| s.0)
}

fn proj_source(doc: &Doc, name: &str) -> Option<String> {
    let d = doc.proj.device(SRV)?;
    scripts::find(d, name).map(|j| scripts::source_of(&d.services[j]))
}

pub(crate) fn run(t: &mut T) {
    texts(t);
    names(t);
    templates(t);
    steps(t);
    broken(t);
    keep(t);
    marks(t);
}

// the source as the editor shows it and back: a list of lines stays one, a text a text
fn texts(t: &mut T) {
    let l = def("a", lines("x\ny"));
    t.eq("source of lines", scripts::source_of(&l), String::from("x\ny"));
    let back = scripts::with_source(&l, "x\ny\nz");
    t.eq("lines stay lines", back.get("source").cloned(), Some(lines("x\ny\nz")));
    let s = def("b", Value::string("eins\n"));
    t.eq("source of a text", scripts::source_of(&s), String::from("eins\n"));
    t.eq("a text stays a text", scripts::with_source(&s, "zwei").get("source").cloned(), Some(Value::string("zwei")));
    let mut more = def("c", lines("1"));
    more.config.insert("port", Value::u64(7));
    let c = scripts::with_source(&more, "2");
    t.eq("the rest of the config stays", (c.get("port").cloned(), c.as_object().map(|o| o[0].0.clone())), (Some(Value::u64(7)), Some(String::from("source"))));
    t.eq("no source", scripts::source_of(&ServiceDef { kind: String::from(KIND), name: String::from("d"), disabled: false, config: Value::Null }), String::new());
    t.eq("the first error", scripts::first_error("let a = 1\nlet = 2").map(|e| (e.0, e.1)), Some((2, 5)));
    t.eq("warnings do not count", scripts::first_error("pirnt(1)"), None);
}

// names of scripts: one word, free on the device
fn names(t: &mut T) {
    let p = project();
    let Some(d) = p.device(SRV) else {
        t.check("names: device", false);
        return;
    };
    t.eq("a good name", scripts::check_name(d, " web-2.a_b ", None), Ok(String::from("web-2.a_b")));
    t.check("empty", scripts::check_name(d, "  ", None) == Err(String::from("Ein Skript braucht einen Namen.")));
    for bad in ["a b", "-x", "ä", "x/y", &"a".repeat(33)] {
        t.check(&format!("bad name {:?}", bad), scripts::check_name(d, bad, None).is_err_and(|e| e.starts_with("\u{201e}")));
    }
    t.eq("taken", scripts::check_name(d, "ZAEHLER", None), Err(String::from("Auf SRV1 gibt es schon einen Dienst \u{201e}ZAEHLER\u{201c}.")));
    t.eq("its own name in another case", scripts::check_name(d, "Zaehler", Some("zaehler")), Ok(String::from("Zaehler")));
    t.eq("free names", (scripts::free_name(d, "echo"), scripts::free_name(d, "zaehler"), scripts::free_name(d, "kaputt")), (String::from("echo"), String::from("zaehler2"), String::from("kaputt2")));
}

/// the English of src/locales/ui.tsv (the German when a row is missing)
fn english() -> impl Fn(&'static str) -> &'static str {
    let rows: Vec<(&'static str, &'static str)> = include_str!("locales/ui.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1)
}

// every template compiles in both languages, and the English ones are English
fn templates(t: &mut T) {
    let de = |s: &'static str| s;
    let en = english();
    for k in 0..scripts::TEMPLATES.len() {
        let name = scripts::TEMPLATES[k].name;
        for (lang, text) in [("de", scripts::template(k, &de).join("\n")), ("en", scripts::template(k, &en).join("\n"))] {
            let diags: Vec<String> = script::check(&text).iter().map(|d| format!("{}", d)).collect();
            t.check(&format!("template {} ({}) compiles and has no warning: {:?}", name, lang, diags), diags.is_empty());
        }
        t.check(&format!("template {} in English", name), scripts::template(k, &en) != scripts::template(k, &de));
    }
    t.eq("five templates", scripts::TEMPLATES.len(), 5);
}

// the steps through Doc: each one undo step, the running device and the script's disk stay
fn steps(t: &mut T) {
    let mut doc = Doc::new();
    doc.replace(project(), None);
    // something only the running device knows: a restart would lose it
    let _ = doc.sim.send(Cmd::Cli { dev: SRV, line: String::from("ip addr add 10.0.0.99/24 dev eth0") });
    let alive = |doc: &Doc| doc.sim.engine().running_config(SRV).contains("10.0.0.99/24");
    go(&mut doc, 10);
    t.eq("the script counts", metric(&doc, "zaehler", "starts"), 1);
    let id = svc_id(&doc, "zaehler");
    // a new source: the project has it as lines, the script runs it with its disk
    let new = format!("{}\nmetrics.gauge(\"v\", 2)", COUNT);
    t.eq("apply", scripts::apply(&mut doc, SRV, "zaehler", &new), Ok(true));
    go(&mut doc, 10);
    t.eq("the project has it", proj_source(&doc, "zaehler"), Some(new.clone()));
    t.check("as lines", doc.proj.device(SRV).is_some_and(|d| matches!(d.services[0].config.get("source"), Some(Value::Array(_)))));
    t.eq("the engine runs it", engine_source(&doc, "zaehler"), format!("{}\n", new));
    t.eq("its disk stayed", (metric(&doc, "zaehler", "starts"), metric(&doc, "zaehler", "v")), (2, 2));
    t.eq("the same service", svc_id(&doc, "zaehler"), id);
    t.check("no restart of the device", alive(&doc));
    t.check("unsaved", doc.dirty());
    t.eq("the same again", scripts::apply(&mut doc, SRV, "zaehler", &new), Ok(false));
    let bad = scripts::apply(&mut doc, SRV, "zaehler", "let = 3");
    t.check(&format!("does not compile ({:?})", bad), bad.as_ref().is_err_and(|e| e.starts_with("Das Skript übersetzt nicht: Zeile 1, Spalte 5: ")));
    t.eq("the project keeps it", proj_source(&doc, "zaehler"), Some(new.clone()));
    // undo and redo: the source before and again, the disk counts on
    t.check("undo", doc.undo());
    go(&mut doc, 10);
    t.eq("undo: the old source", (proj_source(&doc, "zaehler"), engine_source(&doc, "zaehler")), (Some(String::from(COUNT)), format!("{}\n", COUNT)));
    t.eq("undo: the disk counts on", metric(&doc, "zaehler", "starts"), 3);
    t.check("redo", doc.redo());
    go(&mut doc, 10);
    t.eq("redo: the new source", engine_source(&doc, "zaehler"), format!("{}\n", new));
    t.eq("redo: the disk counts on", metric(&doc, "zaehler", "starts"), 4);
    t.check("still no restart", alive(&doc));
    // a new script from a template, undone and redone
    t.eq("add", scripts::add(&mut doc, SRV, 1), Ok(String::from("echo")));
    go(&mut doc, 10);
    t.check("added: it runs", engine_has(&doc, "echo") && doc.sim.engine().sockets(SRV).iter().any(|s| s.port == 7));
    t.eq("added: in the project", doc.proj.device(SRV).map(|d| scripts::of(d).iter().map(|s| s.name.clone()).collect::<Vec<_>>()), Some(vec![String::from("zaehler"), String::from("kaputt"), String::from("echo")]));
    t.eq("add again: another name", scripts::add(&mut doc, SRV, 1), Ok(String::from("echo2")));
    t.check("undo the second", doc.undo() && !engine_has(&doc, "echo2"));
    t.check("undo the first", doc.undo() && !engine_has(&doc, "echo"));
    t.check("redo it", doc.redo() && engine_has(&doc, "echo"));
    // a rename goes on as it ran
    t.eq("rename", scripts::rename(&mut doc, SRV, "zaehler", "starts"), Ok(String::from("starts")));
    go(&mut doc, 10);
    t.eq("renamed: the same service, no new start", (svc_id(&doc, "starts"), engine_has(&doc, "zaehler"), metric(&doc, "starts", "starts")), (id, false, 4));
    t.eq("rename to a taken name", scripts::rename(&mut doc, SRV, "starts", "Echo"), Err(String::from("Auf SRV1 gibt es schon einen Dienst \u{201e}Echo\u{201c}.")));
    t.check("undo the rename", doc.undo() && svc_id(&doc, "zaehler") == id);
    // a removal and its undo (the disk starts empty)
    t.eq("remove", scripts::remove(&mut doc, SRV, "zaehler"), Ok(()));
    t.check("removed", !engine_has(&doc, "zaehler") && proj_source(&doc, "zaehler").is_none());
    t.check("undo the removal", doc.undo() && engine_has(&doc, "zaehler"));
    go(&mut doc, 10);
    t.eq("back with an empty disk", metric(&doc, "zaehler", "starts"), 1);
    t.check("a removed one's name", scripts::remove(&mut doc, SRV, "nix").is_err_and(|e| e == "Das Skript \u{201e}nix\u{201c} gibt es auf SRV1 nicht."));
    // stop, start and restart in the simulator only
    t.eq("stop", scripts::run(&mut doc, SRV, "echo", false), Ok(()));
    go(&mut doc, 1);
    t.check("stopped", !doc.sim.engine().services(SRV).iter().any(|s| s.name == "echo" && s.running));
    t.check("the project still starts it", doc.proj.device(SRV).is_some_and(|d| d.services.iter().any(|s| s.name == "echo" && !s.disabled)));
    t.eq("restart", scripts::restart(&mut doc, SRV, "echo"), Ok(()));
    go(&mut doc, 1);
    t.check("running again", doc.sim.engine().services(SRV).iter().any(|s| s.name == "echo" && s.running));
    t.check("never a restart of the device", alive(&doc));
}

// a script the simulator could not take from the project file: applying a source that compiles
// brings it in; undone, it goes again
fn broken(t: &mut T) {
    let mut doc = Doc::new();
    doc.replace(project(), None);
    t.check("left out", !engine_has(&doc, "kaputt"));
    t.eq("apply a fix", scripts::apply(&mut doc, SRV, "kaputt", "metrics.gauge(\"ok\", 1)"), Ok(true));
    go(&mut doc, 10);
    t.eq("it runs", metric(&doc, "kaputt", "ok"), 1);
    t.check("a text stays a text", doc.proj.device(SRV).is_some_and(|d| d.services[1].config.get("source") == Some(&Value::string("metrics.gauge(\"ok\", 1)"))));
    t.check("undo: out again", doc.undo() && !engine_has(&doc, "kaputt"));
}

// keep_steps: what keeps the running device and what restarts it
fn keep(t: &mut T) {
    let a = def("a", lines("1"));
    let mut a2 = def("a", lines("2"));
    let has = |_: &str| true;
    let none = |_: &str| false;
    let names = |v: Option<Vec<Action>>| {
        v.map(|v| {
            v.iter()
                .map(|x| {
                    let s = format!("{:?}", x);
                    String::from(s.split([' ', '{', '(']).next().unwrap_or(""))
                })
                .collect::<Vec<String>>()
        })
    };
    t.eq("a new source", names(keep_steps(1, &[a.clone()], &[a2.clone()], &has)), Some(vec![String::from("ScriptSource")]));
    t.eq("one the engine lacks", names(keep_steps(1, &[a.clone()], &[a2.clone()], &none)), Some(vec![String::from("Service")]));
    t.eq("one that does not compile", names(keep_steps(1, &[a.clone()], &[def("a", lines("let = 1"))], &has)), Some(vec![String::from("RemoveService")]));
    t.eq("added", names(keep_steps(1, &[], &[a.clone()], &has)), Some(vec![String::from("Service")]));
    t.eq("removed", names(keep_steps(1, &[a.clone()], &[], &has)), Some(vec![String::from("RemoveService")]));
    t.eq("renamed", names(keep_steps(1, &[a.clone()], &[def("b", lines("1"))], &has)), Some(vec![String::from("RenameService")]));
    t.eq("another case", names(keep_steps(1, &[a.clone()], &[def("A", lines("1"))], &has)), Some(vec![String::from("RenameService")]));
    a2.disabled = true;
    t.eq("switched and new", names(keep_steps(1, &[a.clone()], &[a2.clone()], &has)), Some(vec![String::from("ScriptSource"), String::from("ServiceRun")]));
    let echo = ServiceDef { kind: String::from("tcp-echo"), name: String::from("echo"), disabled: false, config: Value::object([("port", Value::u64(7))]) };
    let mut echo2 = echo.clone();
    echo2.config.insert("port", Value::u64(8));
    t.eq("another service changed: a restart", names(keep_steps(1, &[echo.clone()], &[echo2], &has)), None);
    t.eq("another service added: a restart", names(keep_steps(1, &[], &[echo.clone()], &has)), None);
    let mut off = echo.clone();
    off.disabled = true;
    t.eq("another service switched", names(keep_steps(1, &[echo], &[off], &has)), Some(vec![String::from("ServiceRun")]));
}

// the marks of runtime errors on an edited text: lines before the first change stay, lines
// after the last one move with it, a changed line has none; a mark covers the word
fn marks(t: &mut T) {
    let run: Vec<&str> = vec!["a", "b", "c", "d"];
    let now: Vec<String> = ["a", "x", "y", "c", "d"].iter().map(|s| String::from(*s)).collect();
    let m = |l: usize| map_line(&run, &now, l);
    t.eq("before, changed, after", (m(1), m(2), m(3), m(4), m(5), m(0)), (Some(1), None, Some(4), Some(5), None, None));
    let same: Vec<String> = run.iter().map(|s| String::from(*s)).collect();
    t.eq("unchanged", (1..=4).map(|l| map_line(&run, &same, l)).collect::<Vec<_>>(), vec![Some(1), Some(2), Some(3), Some(4)]);
    let shorter: Vec<String> = vec![String::from("a"), String::from("d")];
    t.eq("lines removed", (map_line(&run, &shorter, 1), map_line(&run, &shorter, 3), map_line(&run, &shorter, 4)), (Some(1), None, Some(2)));
    t.eq("a word", word_at("  let x = foo_1(2)", 11), (11, 16));
    t.eq("an operator", word_at("1 / 0", 3), (3, 4));
    t.eq("past the end", word_at("ab", 9), (9, 10));
}
