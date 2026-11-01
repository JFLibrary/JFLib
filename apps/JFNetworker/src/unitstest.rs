// unitstest -- checks of services as units, deployments and the lab of microservices (group units
// of --selftest: jfn_model::unit, jfn_sim::units, deploy, cli/units.rs): parsing a unit file and what
// it gets wrong, each systemctl verb with its exit code, generated units of the project's
// services, enable across a power cycle and across saving and loading the project, the restart
// policies in virtual time (on-failure, always, none, RestartSec, the start limit, a stop by
// request), journalctl (-u, -n, -f with Ctrl+C, in another session), a rolling deployment whose
// check fails on the second host and stops, the same with a rollback, one that goes through,
// refusals and an abort, determinism (two runs, a clone in mid-deployment), the scenario grammar,
// and the lab 05-microservices unsolved, solved, and spoiled by restarting all hosts at once
//
// The project is tests/fixtures/units.jfnet: DEV1 (10.0.0.10) holds the web service as files and
// its units, WEB1 to WEB3 (10.0.0.11 to 13) are the group Webserver, on WEB2 a project service
// "alt" holds port 80, PC1 (10.0.0.20) asks.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_model::scenario::{self, Expect, StepKind};
use jfn_model::unit::{self, DeploySpec, Exec, Health, OnFail, Restart};
use jfn_sim::keyframe::digest;
use jfn_sim::{Action, Engine, HostState, LabRun, Report, RunState, UnitEnabled, UnitOp, UnitState, MS, SEC};

use crate::selftest::T;

const FIX: &str = include_str!("../tests/fixtures/units.jfnet");
const LAB: &str = include_str!("../assets/labs/05-microservices.jfnet");
const SHELL: &str = include_str!("../tests/fixtures/shell_tour.jfnet");
const BUDGET: usize = 8_000_000;
const DEV1: u32 = 1;
const WEB1: u32 = 3;
const WEB2: u32 = 4;
const WEB3: u32 = 5;
const PC1: u32 = 6;

fn s(x: &str) -> String {
    String::from(x)
}

fn project() -> Project {
    Project::from_text(FIX).unwrap_or_else(|_| Project::new("units", 11))
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
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

/// a line typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    run_for(e, d);
    out(e, dev)
}

/// a line and its exit status
fn code(e: &mut Engine, dev: u32, line: &str) -> (String, String) {
    let o = cli(e, dev, line, 0);
    let c = cli(e, dev, "echo $?", 0);
    (o, s(c.trim()))
}

fn state(e: &Engine, dev: u32, name: &str) -> Option<UnitState> {
    e.unit(dev, name).map(|u| u.state)
}

pub fn run(t: &mut T) {
    parse_t(t);
    verbs_t(t);
    generated_t(t);
    enable_t(t);
    restart_t(t);
    deps_t(t);
    journal_t(t);
    deploy_t(t);
    rollback_t(t);
    refuse_t(t);
    determinism_t(t);
    scenario_t(t);
    lab_t(t);
}

// ---------------------------------------------------------------- unit files

fn parse_t(t: &mut T) {
    let u = unit::parse("# ein Kommentar\n[Unit]\nDescription=Die API\nAfter=network.target kv.service\nRequires=db\n\n[Service]\nExecStart=jfs /root/a.jfs --port 80 \"zwei Wörter\"\nEnvironment=A=1 \"B=x y\"\nEnvironment=C=3\nRestart=on-failure\nRestartSec=500ms\nSettings={\"ns_per_op\": \\\n  2}\n\n[Install]\nWantedBy=multi-user.target\n");
    t.eq("description", u.description.as_str(), "Die API");
    t.eq("after without targets, what it needs too", u.after.clone(), alloc::vec![s("kv.service"), s("db.service")]);
    t.eq("requires", u.requires.clone(), alloc::vec![s("db.service")]);
    t.eq("exec", u.exec.clone(), Exec::Jfs { file: s("/root/a.jfs"), args: alloc::vec![s("--port"), s("80"), s("zwei Wörter")] });
    t.eq("env", u.env.clone(), alloc::vec![(s("A"), s("1")), (s("B"), s("x y")), (s("C"), s("3"))]);
    t.eq("restart", (u.restart, u.restart_ns), (Restart::OnFailure, 500_000_000));
    t.eq("settings over two lines", u.settings.as_str(), "{\"ns_per_op\":  2}");
    t.eq("wanted by", u.wanted_by.clone(), alloc::vec![s("multi-user.target")]);
    t.check("nothing to say", u.issues.is_empty() && !u.bad());
    let d = u.service_def("api", None);
    match d {
        Ok(d) => {
            t.eq("the service", (d.kind.as_str(), d.name.as_str()), ("script", "api"));
            t.eq("its file", d.config.get("file").and_then(|v| v.as_str()), Some("/root/a.jfs"));
            t.eq("its env", d.config.get("env").and_then(|e| e.get("B")).and_then(|v| v.as_str()), Some("x y"));
            t.eq("its settings", d.config.get("ns_per_op").and_then(|v| v.as_u64()), Some(2));
            t.eq("its unit", d.config.get("unit").and_then(|v| v.as_str()), Some("api.service"));
        }
        Err(m) => t.check(&format!("service_def: {}", m.text()), false),
    }
    // a built-in service
    let b = unit::parse("[Service]\nExecStart=jfn-service lb /etc/jfn/lb.json\n");
    t.eq("built-in", b.exec.clone(), Exec::Builtin { kind: s("lb"), file: Some(s("/etc/jfn/lb.json")) });
    t.eq("no install", b.wanted_by.len(), 0);
    match b.service_def("x", Some("{\"port\": 8}")) {
        Ok(d) => t.eq("built-in's settings", (d.kind.as_str(), d.config.get("port").and_then(|v| v.as_u64())), ("lb", Some(8))),
        Err(m) => t.check(&format!("built-in: {}", m.text()), false),
    }
    t.check("settings that are no JSON", b.service_def("x", Some("{kaputt")).is_err());
    // what it gets wrong
    let words = |text: &str| -> Vec<(u32, bool, String)> { unit::parse(text).issues.iter().map(|i| (i.line, i.error, i.msg.text())).collect() };
    t.eq("no ExecStart", words("[Unit]\nDescription=x\n"), alloc::vec![(0, true, s("[Service] braucht ExecStart."))]);
    t.eq("a relative path", words("[Service]\nExecStart=jfs a.jfs\n"), alloc::vec![(2, true, s("Zeile 2: ExecStart braucht einen absoluten Pfad, nicht a.jfs."))]);
    t.eq("another program", words("[Service]\nExecStart=/bin/sleep 5\n"), alloc::vec![(2, true, s("Zeile 2: ExecStart startet hier nur jfs oder jfn-service, nicht /bin/sleep."))]);
    t.eq("jfs without a file", words("[Service]\nExecStart=jfs\n"), alloc::vec![(2, true, s("Zeile 2: jfs braucht eine Datei."))]);
    t.eq("warnings", words("[Service]\nExecStart=jfs /a.jfs\nRestart=manchmal\nRestartSec=bald\nQuatsch=1\nohne\n[Neu]\nX=1\n").len(), 5);
    let w = unit::parse("[Service]\nExecStart=jfs /a.jfs\nRestart=manchmal\nRestartSec=bald\n");
    t.eq("warnings leave the defaults", (w.restart, w.restart_ns, w.bad()), (Restart::No, unit::RESTART_DEFAULT, false));
    t.eq("an empty value empties", unit::parse("[Unit]\nAfter=a b\nAfter=\nAfter=c\n[Service]\nExecStart=jfs /x\n").after, alloc::vec![s("c.service")]);
    t.eq("seconds", unit::parse("[Service]\nExecStart=jfs /x\nRestartSec=5\n").restart_ns, 5 * SEC);
    t.eq("minutes", unit::parse("[Service]\nExecStart=jfs /x\nRestartSec=1min\n").restart_ns, 60 * SEC);
    // names
    t.eq("names", (unit::unit_name("web"), unit::unit_name("web.service"), s(unit::stem("web.service")), unit::path_of("web")), (s("web.service"), s("web.service"), s("web"), s("/etc/systemd/system/web.service")));
    t.eq("link", unit::link_path("multi-user.target", "web"), s("/etc/systemd/system/multi-user.target.wants/web.service"));
    t.check("name ok", unit::name_ok("web-1_a.b") && !unit::name_ok("a b") && !unit::name_ok(""));
}

// ---------------------------------------------------------------- the verbs

fn verbs_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    // the fresh state: nothing runs, nothing was read wrongly
    t.eq("a project's unit waits", state(&e, DEV1, "web"), Some(UnitState::Inactive));
    t.eq("is-active inactive: 3", code(&mut e, DEV1, "systemctl is-active web"), (s("inactive\n"), s("3")));
    t.eq("is-enabled disabled: 1", code(&mut e, DEV1, "systemctl is-enabled web"), (s("disabled\n"), s("1")));
    t.eq("is-failed: 1", code(&mut e, DEV1, "systemctl is-failed web"), (s("inactive\n"), s("1")));
    let (st, c) = code(&mut e, DEV1, "systemctl status web");
    t.check(&format!("status inactive: {}", st), st.contains("web.service: Der Webdienst") && st.contains("gestoppt (inactive/dead)") && st.contains("/etc/systemd/system/web.service") && st.contains("Programm:  jfs /root/app/server.jfs") && st.contains("on-failure nach 2 s"));
    t.eq("status inactive: 3", c, s("3"));
    t.eq("start: 0", code(&mut e, DEV1, "systemctl start web").1, s("0"));
    run_for(&mut e, 100 * MS);
    t.eq("it is active", state(&e, DEV1, "web"), Some(UnitState::Active));
    t.eq("is-active active: 0", code(&mut e, DEV1, "systemctl is-active web"), (s("active\n"), s("0")));
    t.eq("-q says nothing", code(&mut e, DEV1, "systemctl is-active -q web"), (s(""), s("0")));
    t.eq("a second start: 0", code(&mut e, DEV1, "systemctl start web").1, s("0"));
    let (st, c) = code(&mut e, DEV1, "systemctl status web");
    t.check(&format!("status active: {}", st), st.contains("läuft (active/running)") && st.contains("PID:") && st.contains("gestartet: jfs /root/app/server.jfs") && st.contains("bereit, Version 2"));
    t.eq("status active: 0", c, s("0"));
    // it serves, from its file, with its environment
    let web = cli(&mut e, PC1, "curl -s http://10.0.0.10/", 200 * MS);
    t.eq("it answers", web, s("Version 2 von web\n"));
    t.eq("its numbers", e.metric(DEV1, "web", "version"), Some(2));
    // list-units and list-unit-files
    let l = cli(&mut e, DEV1, "systemctl list-units", 0);
    t.check(&format!("list-units: {}", l), l.contains("web.service") && l.contains("active") && l.contains("Der Webdienst") && !l.contains("crash.service") && l.contains("1 Einheit"));
    let a = cli(&mut e, DEV1, "systemctl list-units --all", 0);
    t.check(&format!("--all: {}", a), a.contains("crash.service") && a.contains("kaputt.service") && a.contains("6 Einheiten"));
    t.check("--failed: none", cli(&mut e, DEV1, "systemctl list-units --failed", 0).contains("Keine Einheiten"));
    let f = cli(&mut e, DEV1, "systemctl list-unit-files", 0);
    t.check(&format!("list-unit-files: {}", f), f.contains("web.service") && f.contains("disabled") && f.contains("crash.service") && f.contains("static"));
    t.check("a pattern", cli(&mut e, DEV1, "systemctl is-active 'w*'", 0).contains("active"));
    // cat
    let c = cli(&mut e, DEV1, "systemctl cat web", 0);
    t.check(&format!("cat: {}", c), c.starts_with("# /etc/systemd/system/web.service\n[Unit]\n") && c.contains("Restart=on-failure"));
    // restart: a new run
    let before = e.service::<jfn_sim::ScriptService>(DEV1, "web").map(|x| x.view(0).runs);
    t.eq("restart: 0", code(&mut e, DEV1, "systemctl restart web").1, s("0"));
    run_for(&mut e, 100 * MS);
    t.eq("restart makes a new run", e.service::<jfn_sim::ScriptService>(DEV1, "web").map(|x| x.view(0).runs), before.map(|r| r + 1));
    t.eq("restart keeps it active", state(&e, DEV1, "web"), Some(UnitState::Active));
    // stop
    t.eq("stop: 0", code(&mut e, DEV1, "systemctl stop web").1, s("0"));
    t.eq("stopped", state(&e, DEV1, "web"), Some(UnitState::Inactive));
    t.check("no more answer", cli(&mut e, PC1, "curl -s -m 1 http://10.0.0.10/", 2 * SEC).is_empty());
    // no such unit
    t.eq("start of nothing: 5", code(&mut e, DEV1, "systemctl start nix"), (s("systemctl start: Die Einheit nix.service gibt es nicht.\n"), s("5")));
    t.eq("stop of nothing: 5", code(&mut e, DEV1, "systemctl stop nix").1, s("5"));
    t.eq("restart of nothing: 5", code(&mut e, DEV1, "systemctl restart nix").1, s("5"));
    t.eq("status of nothing: 4", code(&mut e, DEV1, "systemctl status nix"), (s("Die Einheit nix.service gibt es nicht.\n"), s("4")));
    t.eq("is-active of nothing: 3", code(&mut e, DEV1, "systemctl is-active nix"), (s("inactive\n"), s("3")));
    t.eq("is-enabled of nothing: 1", code(&mut e, DEV1, "systemctl is-enabled nix"), (s("not-found\n"), s("1")));
    t.eq("enable of nothing: 1", code(&mut e, DEV1, "systemctl enable nix").1, s("1"));
    t.eq("disable of nothing: 1", code(&mut e, DEV1, "systemctl disable nix").1, s("1"));
    t.eq("cat of nothing: 1", code(&mut e, DEV1, "systemctl cat nix").1, s("1"));
    t.eq("an unknown verb: 1", code(&mut e, DEV1, "systemctl frobnicate web").1, s("1"));
    t.eq("no unit: 1", code(&mut e, DEV1, "systemctl start").1, s("1"));
    // a file with errors does not start
    let (m, c) = code(&mut e, DEV1, "systemctl start kaputt");
    t.check(&format!("kaputt: {}", m), m.contains("[Service] braucht ExecStart.") && m.contains("systemctl status kaputt.service"));
    t.eq("kaputt: 1", c, s("1"));
    t.eq("kaputt failed", state(&e, DEV1, "kaputt"), Some(UnitState::Failed));
    t.eq("is-failed: 0", code(&mut e, DEV1, "systemctl is-failed kaputt"), (s("failed\n"), s("0")));
    let st = cli(&mut e, DEV1, "systemctl status kaputt", 0);
    t.check(&format!("its status says what: {}", st), st.contains("Warnung: Zeile 3: Quatsch gibt es hier nicht") && st.contains("Fehler: [Service] braucht ExecStart."));
    t.check("list-units --failed", cli(&mut e, DEV1, "systemctl list-units --failed", 0).contains("kaputt.service"));
    let _ = cli(&mut e, DEV1, "systemctl reset-failed kaputt", 0);
    t.eq("reset-failed", state(&e, DEV1, "kaputt"), Some(UnitState::Inactive));
    // a missing program file
    let (m, c) = code(&mut e, DEV1, "mv /root/app/crash.jfs /root/app/weg.jfs; systemctl start crash");
    t.check(&format!("a missing file: {}", m), m.contains("die Datei /root/app/crash.jfs gibt es nicht"));
    t.eq("a missing file: 1", c, s("1"));
    let _ = cli(&mut e, DEV1, "mv /root/app/weg.jfs /root/app/crash.jfs; systemctl reset-failed", 0);
    // a program that does not compile
    let _ = cli(&mut e, DEV1, "cp /root/app/crash.jfs /root/app/alt.jfs; echo 'let = 1' >> /root/app/crash.jfs", 0);
    let (m, c) = code(&mut e, DEV1, "systemctl start crash");
    t.check(&format!("a syntax error: {}", m), m.contains("/root/app/crash.jfs, Zeile 6"));
    t.eq("a syntax error: 1", c, s("1"));
    let _ = cli(&mut e, DEV1, "mv /root/app/alt.jfs /root/app/crash.jfs; systemctl reset-failed", 0);
    // a changed file counts after daemon-reload
    let _ = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    let _ = cli(&mut e, DEV1, "echo '[Service]' >> /etc/systemd/system/web.service; echo 'Environment=NAME=neu' >> /etc/systemd/system/web.service", 0);
    t.check("the file is newer", cli(&mut e, DEV1, "systemctl status web", 0).contains("daemon-reload"));
    let _ = cli(&mut e, DEV1, "systemctl restart web", 100 * MS);
    t.eq("still the old text", cli(&mut e, PC1, "curl -s http://10.0.0.10/", 200 * MS), s("Version 2 von web\n"));
    t.eq("daemon-reload: 0", code(&mut e, DEV1, "systemctl daemon-reload").1, s("0"));
    t.check("the note goes", !cli(&mut e, DEV1, "systemctl status web", 0).contains("daemon-reload"));
    let _ = cli(&mut e, DEV1, "systemctl restart web", 100 * MS);
    t.eq("the new text", cli(&mut e, PC1, "curl -s http://10.0.0.10/", 200 * MS), s("Version 2 von neu\n"));
    // a built-in service with its settings in a file
    let _ = cli(&mut e, DEV1, "systemctl start statisch", 100 * MS);
    t.eq("built-in runs", e.services(DEV1).iter().find(|x| x.name == "statisch").map(|x| (x.kind, x.running)), Some(("http-server", true)));
    t.eq("built-in answers", cli(&mut e, PC1, "curl -s http://10.0.0.10:8090/", 200 * MS), s("fertig\n"));
    // help in our own words; Tab
    let h = cli(&mut e, DEV1, "man systemctl", 0);
    t.check("man", h.contains("Einheit") && h.contains("/etc/systemd/system/NAME.service") && h.contains("AUFRUF"));
    t.check("--help", cli(&mut e, DEV1, "systemctl --help", 0).starts_with("Aufruf: systemctl"));
    t.check("help lists them", { let h = cli(&mut e, DEV1, "help", 0); h.contains("journalctl") && h.contains("systemctl") });
    t.eq("which", cli(&mut e, DEV1, "which systemctl", 0), s("/usr/bin/systemctl\n"));
    let (line, _) = e.cli_complete(DEV1, "systemctl dae");
    t.eq("Tab: a verb", line, Some(s("systemctl daemon-reload ")));
    let (line, _) = e.cli_complete(DEV1, "systemctl status we");
    t.eq("Tab: a unit", line, Some(s("systemctl status web.service ")));
    let (line, _) = e.cli_complete(DEV1, "systemc");
    t.eq("Tab: the command", line, Some(s("systemctl ")));
}

// ---------------------------------------------------------------- the project's services

fn generated_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let l = cli(&mut e, WEB2, "systemctl list-units", 0);
    t.check(&format!("the project's service: {}", l), l.contains("alt.service") && l.contains("Projekt") && l.contains("active"));
    t.eq("generated", code(&mut e, WEB2, "systemctl is-enabled alt"), (s("generated\n"), s("0")));
    let st = cli(&mut e, WEB2, "systemctl status alt", 0);
    t.check(&format!("its status: {}", st), st.contains("aus dem Projekt") && st.contains("Art:       http-server") && st.contains("läuft"));
    let (m, c) = code(&mut e, WEB2, "systemctl enable alt");
    t.check(&format!("enable refused: {}", m), m.contains("kommt aus dem Projekt"));
    t.eq("enable refused: 1", c, s("1"));
    t.eq("disable refused: 1", code(&mut e, WEB2, "systemctl disable alt").1, s("1"));
    t.check("cat is generated", cli(&mut e, WEB2, "systemctl cat alt", 0).contains("jfn-service http-server"));
    // stop, start, restart act on the service
    t.eq("stop: 0", code(&mut e, WEB2, "systemctl stop alt").1, s("0"));
    t.eq("stopped", state(&e, WEB2, "alt"), Some(UnitState::Inactive));
    t.eq("generated still", cli(&mut e, WEB2, "systemctl is-enabled alt", 0), s("generated\n"));
    t.eq("start: 0", code(&mut e, WEB2, "systemctl start alt").1, s("0"));
    run_for(&mut e, 100 * MS);
    t.eq("runs again", e.services(WEB2).iter().find(|x| x.name == "alt").map(|x| x.running), Some(true));
    t.eq("restart: 0", code(&mut e, WEB2, "systemctl restart alt").1, s("0"));
    run_for(&mut e, 100 * MS);
    t.eq("runs after restart", state(&e, WEB2, "alt"), Some(UnitState::Active));
    // a stop is not for good: a power cycle starts it again, as an enabled unit
    let _ = cli(&mut e, WEB2, "systemctl stop alt", 0);
    let _ = e.apply(Action::Power { dev: WEB2, on: false });
    run_for(&mut e, 100 * MS);
    let _ = e.apply(Action::Power { dev: WEB2, on: true });
    run_for(&mut e, 100 * MS);
    t.eq("a power cycle starts it", state(&e, WEB2, "alt"), Some(UnitState::Active));
    // what "script stop" does stays as it was: it stays stopped through a power cycle
    let mut p = Project::from_text(SHELL).unwrap_or_else(|_| Project::new("x", 1));
    p.devices[0].services.push(jfn_model::ServiceDef { kind: s("script"), name: s("s1"), disabled: false, config: agent_core::jmap::Value::object([("source", agent_core::jmap::Value::string("timer.every(1000, fn() { })"))]) });
    let dev = p.devices[0].id;
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    t.eq("a project without unit files: nothing read", e.units(dev).iter().map(|u| u.file.is_none()).collect::<Vec<_>>(), alloc::vec![true]);
    t.eq("no unit dir", cli(&mut e, dev, "ls /etc/systemd", 0), s("ls: cannot access '/etc/systemd': No such file or directory\n"));
    let _ = cli(&mut e, dev, "script stop s1", 0);
    let _ = e.apply(Action::Power { dev, on: false });
    let _ = e.apply(Action::Power { dev, on: true });
    run_for(&mut e, 100 * MS);
    t.eq("script stop stays", e.services(dev).iter().find(|x| x.name == "s1").map(|x| x.running), Some(false));
    t.eq("a switched-off service", e.unit(dev, "s1").map(|u| u.enabled), Some(UnitEnabled::GeneratedOff));
}

// ---------------------------------------------------------------- enable

fn enable_t(t: &mut T) {
    let mut p = project();
    let mut e = engine(&p);
    run_for(&mut e, 100 * MS);
    let m = cli(&mut e, DEV1, "systemctl enable web", 0);
    t.eq("enable says what", m, s("eingeschaltet: /etc/systemd/system/multi-user.target.wants/web.service\n"));
    t.eq("is-enabled", code(&mut e, DEV1, "systemctl is-enabled web"), (s("enabled\n"), s("0")));
    t.eq("enable makes a file", cli(&mut e, DEV1, "cat /etc/systemd/system/multi-user.target.wants/web.service", 0), s("/etc/systemd/system/web.service\n"));
    t.eq("enable does not start", state(&e, DEV1, "web"), Some(UnitState::Inactive));
    t.eq("a second enable: nothing to say", code(&mut e, DEV1, "systemctl enable web"), (s(""), s("0")));
    t.check("static", cli(&mut e, DEV1, "systemctl enable crash", 0).contains("hat kein [Install]"));
    t.eq("is-enabled static: 0", code(&mut e, DEV1, "systemctl is-enabled crash"), (s("static\n"), s("0")));
    // a power cycle starts it
    let _ = e.apply(Action::Power { dev: DEV1, on: false });
    t.eq("off: nothing runs", state(&e, DEV1, "web"), Some(UnitState::Inactive));
    run_for(&mut e, 100 * MS);
    let _ = e.apply(Action::Power { dev: DEV1, on: true });
    run_for(&mut e, 100 * MS);
    t.eq("power on starts it", state(&e, DEV1, "web"), Some(UnitState::Active));
    t.eq("it answers", cli(&mut e, PC1, "curl -s http://10.0.0.10/", 200 * MS), s("Version 2 von web\n"));
    // saving the project: the files say it; a new engine starts it, without a word typed
    if let Some(d) = p.devices.iter_mut().find(|d| d.id == DEV1) {
        d.files = e.files(DEV1);
    }
    let text = p.to_text();
    let again = Project::from_text(&text).unwrap_or_else(|_| Project::new("x", 1));
    let mut f = engine(&again);
    run_for(&mut f, 100 * MS);
    t.eq("saved and loaded: it runs", state(&f, DEV1, "web"), Some(UnitState::Active));
    t.eq("saved and loaded: enabled", f.unit(DEV1, "web").map(|u| u.enabled), Some(UnitEnabled::Enabled));
    t.eq("saved and loaded: it answers", cli(&mut f, PC1, "curl -s http://10.0.0.10/", 200 * MS), s("Version 2 von web\n"));
    t.eq("a Files action keeps it", e.apply(Action::Files { dev: DEV1, files: f.files(DEV1) }).is_ok(), true);
    // a device that is off at the start waits
    let mut off = again.clone();
    if let Some(d) = off.devices.iter_mut().find(|d| d.id == DEV1) {
        d.off = true;
    }
    let mut g = engine(&off);
    run_for(&mut g, 100 * MS);
    t.eq("off in the project: not started", state(&g, DEV1, "web"), Some(UnitState::Inactive));
    let _ = g.apply(Action::Power { dev: DEV1, on: true });
    run_for(&mut g, 100 * MS);
    t.eq("then on: started", state(&g, DEV1, "web"), Some(UnitState::Active));
    // disable: the next start leaves it alone
    let mut h = engine(&again);
    run_for(&mut h, 100 * MS);
    let m = cli(&mut h, DEV1, "systemctl disable web", 0);
    t.eq("disable says what", m, s("ausgeschaltet: /etc/systemd/system/multi-user.target.wants/web.service\n"));
    t.eq("disable does not stop", state(&h, DEV1, "web"), Some(UnitState::Active));
    t.eq("disabled", cli(&mut h, DEV1, "systemctl is-enabled web", 0), s("disabled\n"));
    let _ = h.apply(Action::Power { dev: DEV1, on: false });
    let _ = h.apply(Action::Power { dev: DEV1, on: true });
    run_for(&mut h, 100 * MS);
    t.eq("disabled: not started", state(&h, DEV1, "web"), Some(UnitState::Inactive));
    // --now
    let _ = cli(&mut h, DEV1, "systemctl enable --now web", 100 * MS);
    t.eq("enable --now starts", state(&h, DEV1, "web"), Some(UnitState::Active));
    let _ = cli(&mut h, DEV1, "systemctl disable --now web", 0);
    t.eq("disable --now stops", state(&h, DEV1, "web"), Some(UnitState::Inactive));
    // the action (the app, K1's recording)
    t.check("Action::Unit", h.apply(Action::Unit { dev: DEV1, name: s("web"), op: UnitOp::Start }).is_ok());
    t.eq("Action::Unit started it", state(&h, DEV1, "web"), Some(UnitState::Active));
    t.check("Action::Unit refused", h.apply(Action::Unit { dev: DEV1, name: s("nix"), op: UnitOp::Start }).is_err());
    // a unit file takes a project service of its name over
    let mut q = project();
    if let Some(d) = q.devices.iter_mut().find(|d| d.id == DEV1) {
        d.services.push(jfn_model::ServiceDef { kind: s("http-server"), name: s("web"), disabled: false, config: agent_core::jmap::Value::object([("port", agent_core::jmap::Value::i64(81))]) });
    }
    let mut r = engine(&q);
    run_for(&mut r, 100 * MS);
    t.eq("a project service under a unit: only one row", r.units(DEV1).iter().filter(|u| u.name == "web.service").count(), 1);
    t.eq("it does not start by itself", state(&r, DEV1, "web"), Some(UnitState::Inactive));
    t.eq("the service is switched off", r.services(DEV1).iter().find(|x| x.name == "web").map(|x| x.running), Some(false));
    let _ = cli(&mut r, DEV1, "systemctl start web", 100 * MS);
    t.eq("started from the file", r.services(DEV1).iter().find(|x| x.name == "web").map(|x| x.kind), Some("script"));
}

// ---------------------------------------------------------------- restart policies

fn restart_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let t0 = e.now();
    // crash: fails 500 ms after a start, Restart=on-failure, RestartSec=1
    let _ = cli(&mut e, DEV1, "systemctl start crash", 0);
    run_for(&mut e, 400 * MS);
    t.eq("runs", state(&e, DEV1, "crash"), Some(UnitState::Active));
    run_for(&mut e, 200 * MS);
    t.eq("ended: it waits", state(&e, DEV1, "crash"), Some(UnitState::AutoRestart));
    let v = e.unit(DEV1, "crash");
    t.eq("its status", v.as_ref().map(|u| (u.exit, u.restarts)), Some((Some(1), 0)));
    t.eq("is-active says activating", cli(&mut e, DEV1, "systemctl is-active crash", 0), s("activating\n"));
    run_for(&mut e, 950 * MS);
    t.eq("after RestartSec it runs", (state(&e, DEV1, "crash"), e.unit(DEV1, "crash").map(|u| u.restarts)), (Some(UnitState::Active), Some(1)));
    t.check("to the nanosecond", e.unit(DEV1, "crash").is_some_and(|u| u.since > t0 + 1500 * MS && u.since < t0 + 1520 * MS));
    // the start limit: five starts within 10 s
    run_for(&mut e, 7 * SEC);
    t.eq("the start limit stops it", state(&e, DEV1, "crash"), Some(UnitState::Failed));
    let v = e.unit(DEV1, "crash");
    t.eq("restarts counted", v.as_ref().map(|u| u.restarts), Some(4));
    t.check("the reason", v.as_ref().and_then(|u| u.why.clone()).is_some_and(|w| w.contains("zu viele Starts")));
    t.eq("it stays down", { run_for(&mut e, 10 * SEC); state(&e, DEV1, "crash") }, Some(UnitState::Failed));
    let st = cli(&mut e, DEV1, "systemctl status crash", 0);
    t.check(&format!("its status tells: {}", st), st.contains("zu viele Starts") && st.contains("Status:    1"));
    // a manual start after the limit is allowed again
    let _ = cli(&mut e, DEV1, "systemctl stop crash; systemctl start crash", 100 * MS);
    t.eq("a start by hand goes", state(&e, DEV1, "crash"), Some(UnitState::Active));
    // a stop by request never restarts
    let _ = cli(&mut e, DEV1, "systemctl stop crash", 0);
    run_for(&mut e, 5 * SEC);
    t.eq("stopped stays stopped", state(&e, DEV1, "crash"), Some(UnitState::Inactive));
    // stopped while it waits for its restart
    let _ = cli(&mut e, DEV1, "systemctl start crash", 600 * MS);
    t.eq("waits", state(&e, DEV1, "crash"), Some(UnitState::AutoRestart));
    let _ = cli(&mut e, DEV1, "systemctl stop crash", 0);
    run_for(&mut e, 2 * SEC);
    t.eq("a stop ends the wait", state(&e, DEV1, "crash"), Some(UnitState::Inactive));
    // started by hand while it waits: the restart is off
    let _ = cli(&mut e, DEV1, "systemctl start crash", 600 * MS);
    let r0 = e.unit(DEV1, "crash").map(|u| u.restarts);
    let _ = cli(&mut e, DEV1, "systemctl start crash", 450 * MS);
    t.eq("no second start from the timer", (state(&e, DEV1, "crash"), e.unit(DEV1, "crash").map(|u| u.restarts)), (Some(UnitState::Active), r0));
    let _ = cli(&mut e, DEV1, "systemctl stop crash", 0);
    // once: ends by itself with status 0; Restart=always starts it again after 2 s
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "systemctl start once", 10 * MS);
    t.eq("ended with 0: waits, always", (state(&e, DEV1, "once"), e.unit(DEV1, "once").and_then(|u| u.exit)), (Some(UnitState::AutoRestart), Some(0)));
    run_for(&mut e, 2100 * MS);
    t.eq("always: one restart", e.unit(DEV1, "once").map(|u| u.restarts), Some(1));
    // the same without Restart ends for good, inactive; on-failure leaves status 0 alone
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "echo '[Service]' > /etc/systemd/system/a.service; echo 'ExecStart=jfs /root/app/once.jfs' >> /etc/systemd/system/a.service", 0);
    let _ = cli(&mut e, DEV1, "systemctl start a", 100 * MS);
    t.eq("Restart=no, status 0: inactive", (state(&e, DEV1, "a"), e.unit(DEV1, "a").map(|u| u.restarts)), (Some(UnitState::Inactive), Some(0)));
    let _ = cli(&mut e, DEV1, "echo '[Service]' > /etc/systemd/system/b.service; echo 'ExecStart=jfs /root/app/once.jfs' >> /etc/systemd/system/b.service; echo 'Restart=on-failure' >> /etc/systemd/system/b.service", 0);
    let _ = cli(&mut e, DEV1, "systemctl start b", SEC);
    t.eq("on-failure, status 0: no restart", (state(&e, DEV1, "b"), e.unit(DEV1, "b").map(|u| u.restarts)), (Some(UnitState::Inactive), Some(0)));
    // no restart policy, a failure: failed
    let _ = cli(&mut e, DEV1, "echo '[Service]' > /etc/systemd/system/c.service; echo 'ExecStart=jfs /root/app/crash.jfs' >> /etc/systemd/system/c.service; echo 'Settings={\"stop_on_error\": true}' >> /etc/systemd/system/c.service", 0);
    let _ = cli(&mut e, DEV1, "systemctl start c", SEC);
    t.eq("Restart=no, a failure: failed", state(&e, DEV1, "c"), Some(UnitState::Failed));
    t.eq("status 3", code(&mut e, DEV1, "systemctl status c").1, s("3"));
    // a power cycle ends the waiting
    let _ = cli(&mut e, DEV1, "systemctl start crash", 600 * MS);
    t.eq("waits before the power cycle", state(&e, DEV1, "crash"), Some(UnitState::AutoRestart));
    let _ = e.apply(Action::Power { dev: DEV1, on: false });
    run_for(&mut e, 3 * SEC);
    t.eq("off: no restart", state(&e, DEV1, "crash"), Some(UnitState::Inactive));
    // what the script shell command does is the unit's too
    let _ = e.apply(Action::Power { dev: DEV1, on: true });
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    let _ = cli(&mut e, DEV1, "script stop web", 0);
    t.eq("script stop: the unit is inactive", state(&e, DEV1, "web"), Some(UnitState::Inactive));
    let _ = cli(&mut e, DEV1, "script start web", 100 * MS);
    t.eq("script start: the unit is active", state(&e, DEV1, "web"), Some(UnitState::Active));
    // a script keeps its numbers and its service id through restarts
    let before = e.services(DEV1).iter().position(|x| x.name == "web");
    let calls = e.metric(DEV1, "web", "calls");
    let _ = cli(&mut e, DEV1, "systemctl restart web", 100 * MS);
    t.check("numbers stay", e.metric(DEV1, "web", "calls") >= calls);
    t.eq("the same service", e.services(DEV1).iter().position(|x| x.name == "web"), before);
}

fn deps_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "systemctl start braucht", 100 * MS);
    t.eq("it runs", state(&e, DEV1, "braucht"), Some(UnitState::Active));
    t.eq("what it needs runs", state(&e, DEV1, "web"), Some(UnitState::Active));
    let st = cli(&mut e, DEV1, "systemctl status braucht", 0);
    t.check(&format!("status names it: {}", st), st.contains("Braucht:   web.service"));
    let _ = cli(&mut e, DEV1, "systemctl stop web", 0);
    t.eq("it stops with it", state(&e, DEV1, "braucht"), Some(UnitState::Inactive));
    let _ = cli(&mut e, DEV1, "systemctl start braucht", 100 * MS);
    let _ = cli(&mut e, DEV1, "systemctl restart web", 100 * MS);
    t.eq("a restart leaves it", state(&e, DEV1, "braucht"), Some(UnitState::Active));
    // a dependency that is missing
    let _ = cli(&mut e, DEV1, "echo '[Unit]' > /etc/systemd/system/d.service; echo 'Requires=fehlt.service' >> /etc/systemd/system/d.service; echo '[Service]' >> /etc/systemd/system/d.service; echo 'ExecStart=jfs /root/app/needs.jfs' >> /etc/systemd/system/d.service", 0);
    let (m, c) = code(&mut e, DEV1, "systemctl start d");
    t.check(&format!("a missing one: {}", m), m.contains("fehlt.service"));
    t.eq("a missing one: 1", c, s("1"));
    // a boot in the order of After=
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "echo '[Install]' >> /etc/systemd/system/braucht.service; echo 'WantedBy=multi-user.target' >> /etc/systemd/system/braucht.service; systemctl enable braucht; systemctl enable web", 0);
    let _ = e.apply(Action::Power { dev: DEV1, on: false });
    let _ = e.apply(Action::Power { dev: DEV1, on: true });
    run_for(&mut e, 100 * MS);
    let order: Vec<String> = e.log_lines(DEV1).iter().filter(|l| l.tag == "systemd" && l.text.contains("gestartet")).map(|l| String::from(l.text.split(':').next().unwrap_or(""))).collect();
    t.eq("web before braucht", order, alloc::vec![s("web.service"), s("braucht.service")]);
}

// ---------------------------------------------------------------- journalctl

fn journal_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    let j = cli(&mut e, DEV1, "journalctl -u web", 0);
    t.check(&format!("-u: {}", j), j.contains("DEV1 systemd: web.service: gestartet") && j.contains("DEV1 web: bereit, Version 2"));
    t.eq("-u web.service is the same", cli(&mut e, DEV1, "journalctl -u web.service", 0), j);
    t.eq("-n 1", cli(&mut e, DEV1, "journalctl -u web -n 1", 0).lines().count(), 1);
    t.check("-n 1 is the last", cli(&mut e, DEV1, "journalctl -u web -n 1", 0).contains("bereit"));
    t.check("-n1", cli(&mut e, DEV1, "journalctl -u web -n1", 0).contains("bereit"));
    t.check("-r: the newest first", cli(&mut e, DEV1, "journalctl -u web -r", 0).lines().next().is_some_and(|l| l.contains("bereit")));
    t.check("another unit: none", cli(&mut e, DEV1, "journalctl -u crash", 0).contains("Keine Zeilen"));
    let all = cli(&mut e, DEV1, "journalctl", 0);
    t.check("all lines", all.contains("web: bereit") && all.contains("systemd: web.service"));
    t.eq("-u without a name: 1", code(&mut e, DEV1, "journalctl -u").1, s("1"));
    t.eq("an unknown option: 1", code(&mut e, DEV1, "journalctl --frob").1, s("1"));
    // a pipe takes it like any command
    t.eq("through grep", cli(&mut e, DEV1, "journalctl -u web | grep -c bereit", 0), s("1\n"));
    // what a project script logs counts for its generated unit, under its name
    let mut p = project();
    if let Some(d) = p.devices.iter_mut().find(|d| d.id == WEB1) {
        d.services.push(jfn_model::ServiceDef { kind: s("script"), name: s("p1"), disabled: false, config: agent_core::jmap::Value::object([("source", agent_core::jmap::Value::string("log(\"aus dem Projekt\")"))]) });
    }
    let mut g = engine(&p);
    run_for(&mut g, 100 * MS);
    t.check("a generated unit", cli(&mut g, WEB1, "journalctl -u p1", 0).contains("WEB1 p1: aus dem Projekt"));
    // -f: the last ten lines, new ones as they come, until Ctrl+C
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let f0 = cli(&mut e, DEV1, "journalctl -u crash -f", 0);
    t.eq("-f starts empty", f0, s(""));
    t.check("the console waits", e.cli_busy(DEV1));
    let s1 = e.cli_open(DEV1).unwrap_or(0);
    let _ = e.cli_ses_line(DEV1, s1, "systemctl start crash");
    run_for(&mut e, 100 * MS);
    let f1 = out(&mut e, DEV1);
    t.check(&format!("-f shows a line: {}", f1), f1.contains("crash.service: gestartet") && f1.contains("crash: gestartet"));
    t.check("-f keeps out what is not its own", !f1.contains("bereit"));
    run_for(&mut e, SEC);
    let f2 = out(&mut e, DEV1);
    t.check(&format!("-f goes on: {}", f2), f2.contains("beendet mit Status 1") && f2.contains("Neustart in"));
    let _ = e.take_reports();
    let _ = e.cli_interrupt(DEV1);
    t.check("Ctrl+C ends it", !e.cli_busy(DEV1));
    t.eq("$? after Ctrl+C", cli(&mut e, DEV1, "echo $?", 0), s("130\n"));
    run_for(&mut e, SEC);
    t.eq("nothing more comes", out(&mut e, DEV1), s(""));
    // -f in another session, with the console writing the log
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let s2 = e.cli_open(DEV1).unwrap_or(0);
    let _ = e.cli_ses_line(DEV1, s2, "journalctl -f");
    let _ = e.take_reports();
    let _ = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    let _ = e.take_reports();
    let _ = e.cli_line(DEV1, "systemctl stop web");
    let shown: String = e.take_reports().into_iter().filter_map(|r| match r {
        Report::SesOut { ses, bytes, .. } if ses == s2 => Some(String::from_utf8_lossy(&bytes).into_owned()),
        _ => None,
    }).collect();
    t.check(&format!("the other session follows: {}", shown), shown.contains("web.service: gestoppt"));
    t.check("it waits", e.cli_ses_busy(DEV1, s2));
    let _ = e.cli_ses_interrupt(DEV1, s2);
    t.eq("Ctrl+C in that session", e.cli_ses_busy(DEV1, s2), false);
    // in the background: its lines reach the terminal as they come
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let b = cli(&mut e, DEV1, "journalctl -u web -f &", 0);
    t.check(&format!("[1] PID: {}", b), b.starts_with("[1] "));
    t.check("the console takes lines", !e.cli_busy(DEV1));
    let c = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    t.check(&format!("the job's lines come: {}", c), c.contains("web.service: gestartet"));
    let k = cli(&mut e, DEV1, "kill %1", 0);
    t.check(&format!("kill ends it: {}", k), k.contains("Terminated"));
    t.check("no job left", cli(&mut e, DEV1, "jobs", 0).is_empty());
}

// ---------------------------------------------------------------- deployments

fn spec(health: Health, on_fail: OnFail) -> DeploySpec {
    DeploySpec { src: DEV1, hosts: alloc::vec![WEB1, WEB2, WEB3], files: alloc::vec![s("/root/app")], unit: s("web"), health, wait: 500 * MS, timeout: 2 * SEC, on_fail }
}

fn http() -> Health {
    Health::Http { port: 80, path: s("/health"), status: 200 }
}

fn deploy_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    // WEB2's project service holds port 80: the check fails there and the run stops
    t.check("it starts", e.apply(Action::Deploy { spec: spec(http(), OnFail::Stop) }).is_ok());
    t.eq("a run", e.deployments(DEV1).len(), 1);
    t.eq("first: the first host copies", e.deployments(DEV1)[0].state, RunState::Running);
    run_for(&mut e, 10 * MS);
    t.eq("WEB1 copies", e.deployments(DEV1)[0].hosts[0].state, HostState::Copying);
    t.eq("the others wait", (e.deployments(DEV1)[0].hosts[1].state, e.deployments(DEV1)[0].hosts[2].state), (HostState::Pending, HostState::Pending));
    t.eq("a second one is refused", e.apply(Action::Deploy { spec: spec(http(), OnFail::Stop) }).is_err(), true);
    run_for(&mut e, 100 * MS);
    t.eq("WEB1 checks", e.deployments(DEV1)[0].hosts[0].state, HostState::Checking);
    t.eq("WEB2 not touched yet", e.file_store(WEB2).is_some_and(|f| f.exists("/root/app/server.jfs")), false);
    t.eq("WEB1 has the files", e.file_store(WEB1).is_some_and(|f| f.exists("/root/app/server.jfs") && f.exists("/etc/systemd/system/web.service")), true);
    t.eq("WEB1 runs it", state(&e, WEB1, "web"), Some(UnitState::Active));
    t.eq("WEB1 enabled", e.unit(WEB1, "web").map(|u| u.enabled), Some(UnitEnabled::Enabled));
    run_for(&mut e, 300 * MS);
    t.eq("WEB1 waits after its check", e.deployments(DEV1)[0].hosts[0].state, HostState::Waiting);
    run_for(&mut e, 500 * MS);
    t.eq("WEB1 done, WEB2 next", (e.deployments(DEV1)[0].hosts[0].state, e.deployments(DEV1)[0].hosts[1].state), (HostState::Done, HostState::Checking));
    run_for(&mut e, 3 * SEC);
    let r = &e.deployments(DEV1)[0];
    t.eq("the run failed", r.state, RunState::Failed);
    t.eq("WEB2 failed, WEB3 untouched", (r.hosts[1].state, r.hosts[2].state), (HostState::Failed, HostState::Skipped));
    t.check(&format!("WEB2's reason: {}", r.hosts[1].note.text()), r.hosts[1].note.text().contains("Prüfung hielt nicht"));
    t.eq("WEB3 has no files", e.file_store(WEB3).is_some_and(|f| f.exists("/root/app/server.jfs")), false);
    t.eq("WEB1 keeps the new version", state(&e, WEB1, "web"), Some(UnitState::Active));
    t.check("the log of the run", r.log.iter().any(|l| l.msg.text().contains("WEB2: gescheitert")) && r.log.iter().any(|l| l.msg.text().contains("gescheitert")));
    // the files arrive in the project through Report::Files
    let reports = e.take_reports();
    t.check("Report::Files per host", reports.iter().any(|x| matches!(x, Report::Files { dev, .. } if *dev == WEB1)));
    // the way cleared: all three, one after the other
    let _ = cli(&mut e, WEB2, "systemctl stop alt", 0);
    t.check("again", e.apply(Action::Deploy { spec: spec(http(), OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    let r = &e.deployments(DEV1)[1];
    t.eq("done", (r.state, r.done()), (RunState::Done, 3));
    t.check("rolling: the hosts one after the other", r.hosts[0].since < r.hosts[1].since && r.hosts[1].since < r.hosts[2].since);
    t.check("it has an end", r.end.is_some());
    for (dev, name) in [(WEB1, "WEB1"), (WEB2, "WEB2"), (WEB3, "WEB3")] {
        t.eq(&format!("{} runs", name), state(&e, dev, "web"), Some(UnitState::Active));
        t.eq(&format!("{} answers", name), e.metric(dev, "web", "version"), Some(2));
    }
    t.eq("through the network", cli(&mut e, PC1, "curl -s http://10.0.0.13/", 200 * MS), s("Version 2 von web\n"));
    // it survives a power cycle as an enabled unit
    let _ = e.apply(Action::Power { dev: WEB3, on: false });
    let _ = e.apply(Action::Power { dev: WEB3, on: true });
    run_for(&mut e, 100 * MS);
    t.eq("after a power cycle", state(&e, WEB3, "web"), Some(UnitState::Active));
    // the other checks: active, a TCP port
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, WEB2, "systemctl stop alt", 0);
    t.check("active", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    t.eq("active: done", e.deployments(DEV1)[0].state, RunState::Done);
    t.check("tcp", e.apply(Action::Deploy { spec: spec(Health::Tcp { port: 80 }, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    t.eq("tcp: done", e.deployments(DEV1)[1].state, RunState::Done);
    // a TCP port nobody listens on: its check runs out of time
    t.check("tcp 81", e.apply(Action::Deploy { spec: spec(Health::Tcp { port: 81 }, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    let r = &e.deployments(DEV1)[2];
    t.eq("tcp 81: failed on the first", (r.state, r.hosts[0].state), (RunState::Failed, HostState::Failed));
    t.check(&format!("the reason: {}", r.hosts[0].note.text()), r.hosts[0].note.text().contains("niemand lauscht"));
    // a wrong status
    t.check("status 418", e.apply(Action::Deploy { spec: spec(Health::Http { port: 80, path: s("/"), status: 418 }, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    let r = &e.deployments(DEV1)[3];
    t.check(&format!("the status: {}", r.hosts[0].note.text()), r.hosts[0].note.text().contains("Status 200 statt 418"));
    // a program that does not start: an active check sees it
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "echo 'throw \"kaputt\"' > /root/app/server.jfs", 0);
    t.check("broken", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    t.eq("broken: failed on the first", e.deployments(DEV1)[0].hosts[0].state, HostState::Failed);
    // a unit file with an error
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let mut k = spec(Health::Active, OnFail::Stop);
    k.unit = s("kaputt");
    t.check("a bad unit", e.apply(Action::Deploy { spec: k }).is_ok());
    run_for(&mut e, 5 * SEC);
    t.check(&format!("a bad unit: {}", e.deployments(DEV1)[0].hosts[0].note.text()), e.deployments(DEV1)[0].hosts[0].note.text().contains("Die Einheit ist falsch") || e.deployments(DEV1)[0].hosts[0].note.text().contains("die Einheit ist falsch"));
    // a target that is off
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, WEB2, "systemctl stop alt", 0);
    let _ = e.apply(Action::Power { dev: WEB3, on: false });
    t.check("WEB3 off", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    run_for(&mut e, 5 * SEC);
    let r = &e.deployments(DEV1)[0];
    t.eq("WEB3 off: fails there", (r.state, r.hosts[2].state), (RunState::Failed, HostState::Failed));
    t.check(&format!("why: {}", r.hosts[2].note.text()), r.hosts[2].note.text().contains("ausgeschaltet"));
    // a source that goes off mid-way
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    t.check("start", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    run_for(&mut e, 100 * MS);
    let _ = e.apply(Action::Power { dev: DEV1, on: false });
    t.eq("the source off: aborted", e.deployments(DEV1)[0].state, RunState::Aborted);
    t.check("nobody stays under way", e.deployments(DEV1)[0].hosts.iter().all(|h| matches!(h.state, HostState::Done | HostState::Skipped)));
    run_for(&mut e, 5 * SEC);
    t.eq("WEB2 stays untouched", e.file_store(WEB2).is_some_and(|f| f.exists("/root/app/server.jfs")), false);
    // abort
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    t.check("start", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    run_for(&mut e, 100 * MS);
    t.check("abort", e.apply(Action::DeployAbort { dev: DEV1, id: 1 }).is_ok());
    let r = &e.deployments(DEV1)[0];
    t.eq("aborted", (r.state, r.hosts[0].state, r.hosts[1].state), (RunState::Aborted, HostState::Failed, HostState::Skipped));
    t.check("abort again: refused", e.apply(Action::DeployAbort { dev: DEV1, id: 1 }).is_err());
    t.check("a new one goes", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Stop) }).is_ok());
    // the run shows in the event list and in ps
    let notes: Vec<String> = e.trace.events().filter(|x| matches!(x.note, jfn_sim::trace::Note::Deploy { .. })).map(|x| format!("{:?}", x.t)).collect();
    t.check("notes", !notes.is_empty());
    run_for(&mut e, 10 * MS);
    let ps = cli(&mut e, DEV1, "ps", 0);
    t.check(&format!("ps: {}", ps), ps.contains("deploy-"));
    t.eq("no service of the project", e.services(DEV1).iter().any(|x| x.name.starts_with("deploy-")), false);
}

fn rollback_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, WEB2, "systemctl stop alt", 0);
    // version 2 everywhere
    let _ = e.apply(Action::Deploy { spec: spec(http(), OnFail::Stop) });
    run_for(&mut e, 5 * SEC);
    t.eq("version 2 is out", e.deployments(DEV1)[0].state, RunState::Done);
    // version 3, WEB3 off: its copy fails, and the two before go back
    let _ = cli(&mut e, DEV1, "echo 'print(\"Version 3\")' >> /root/app/server.jfs; echo 'neu' > /root/app/neu.txt", 0);
    let _ = e.apply(Action::Power { dev: WEB3, on: false });
    run_for(&mut e, 100 * MS);
    t.check("version 3", e.apply(Action::Deploy { spec: spec(Health::Active, OnFail::Rollback) }).is_ok());
    run_for(&mut e, 400 * MS);
    t.eq("WEB1 has it for now", e.file_store(WEB1).is_some_and(|f| f.exists("/root/app/neu.txt")), true);
    run_for(&mut e, 10 * SEC);
    let r = &e.deployments(DEV1)[1];
    t.eq("rolled back", r.state, RunState::RolledBack);
    t.eq("the hosts", (r.hosts[0].state, r.hosts[1].state, r.hosts[2].state), (HostState::RolledBack, HostState::RolledBack, HostState::Failed));
    let last: Vec<String> = r.log.iter().rev().take(3).map(|l| l.msg.text()).collect();
    t.check(&format!("the last one first: {:?}", last), last.len() == 3 && last[1].contains("WEB1") && last[2].contains("WEB2"));
    t.eq("the new file went", e.file_store(WEB1).is_some_and(|f| f.exists("/root/app/neu.txt")), false);
    let text = |e: &Engine, dev: u32| e.file_store(dev).and_then(|f| f.read("/root/app/server.jfs").ok().map(|b| String::from_utf8_lossy(b).into_owned()));
    t.check("the old program is back", text(&e, WEB1).is_some_and(|x| !x.contains("Version 3")) && text(&e, WEB2).is_some_and(|x| !x.contains("Version 3")));
    t.eq("it runs again", (state(&e, WEB1, "web"), state(&e, WEB2, "web")), (Some(UnitState::Active), Some(UnitState::Active)));
    t.check("still enabled", e.unit(WEB1, "web").is_some_and(|u| u.enabled == UnitEnabled::Enabled));
    // a fresh host goes back to nothing: the unit file goes, it is no more enabled
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    // WEB2 fails: its port is held, WEB1 goes back
    t.check("fresh", e.apply(Action::Deploy { spec: spec(http(), OnFail::Rollback) }).is_ok());
    run_for(&mut e, 10 * SEC);
    let r = &e.deployments(DEV1)[0];
    t.eq("fresh: rolled back", r.state, RunState::RolledBack);
    t.eq("fresh: WEB1 has nothing", e.file_store(WEB1).is_some_and(|f| f.exists("/root/app/server.jfs") || f.exists("/etc/systemd/system/web.service")), false);
    t.eq("fresh: no unit left", state(&e, WEB1, "web"), None);
    t.eq("fresh: not running", e.services(WEB1).iter().any(|x| x.running && x.name == "web"), false);
    t.eq("fresh: the first host's link went", e.file_store(WEB1).is_some_and(|f| f.exists("/etc/systemd/system/multi-user.target.wants/web.service")), false);
}

fn refuse_t(t: &mut T) {
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let why = |e: &mut Engine, x: DeploySpec| match e.apply(Action::Deploy { spec: x }) {
        Err(m) => m.msg().text(),
        Ok(()) => s("ok"),
    };
    let mut x = spec(Health::Active, OnFail::Stop);
    x.hosts.clear();
    t.eq("no hosts", why(&mut e, x), s("Eine Bereitstellung braucht mindestens einen Host."));
    let mut x = spec(Health::Active, OnFail::Stop);
    x.hosts = alloc::vec![2];
    t.eq("a switch", why(&mut e, x), s("SW1 hat keine Linux-Shell und nimmt keine Einheit."));
    let mut x = spec(Health::Active, OnFail::Stop);
    x.unit = s("nix");
    t.eq("no unit file", why(&mut e, x), s("Die Einheit nix.service gibt es auf DEV1 nicht: weder in /etc/systemd/system noch unter den Dateien."));
    let mut x = spec(Health::Active, OnFail::Stop);
    x.unit = s("a b");
    t.eq("a bad name", why(&mut e, x), s("„a b“ taugt nicht als Name einer Einheit."));
    let mut x = spec(Health::Active, OnFail::Stop);
    x.files = alloc::vec![s("/root/gibtsnicht")];
    t.eq("a missing path", why(&mut e, x), s("Den Pfad /root/gibtsnicht gibt es auf DEV1 nicht."));
    let mut x = spec(Health::Active, OnFail::Stop);
    x.src = 2;
    t.eq("a switch as the source", why(&mut e, x), s("SW1 hat keine Linux-Shell: von dort lässt sich nichts bereitstellen."));
    // the same host twice counts once
    let mut x = spec(Health::Active, OnFail::Stop);
    x.hosts = alloc::vec![WEB1, WEB1];
    t.eq("a host twice", why(&mut e, x), s("ok"));
    t.eq("once", e.deployments(DEV1)[0].hosts.len(), 1);
    // a unit file among the copied files is installed
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let _ = cli(&mut e, DEV1, "cp /etc/systemd/system/web.service /root/app/web2.service", 0);
    let mut x = spec(Health::Active, OnFail::Stop);
    x.unit = s("web2");
    x.hosts = alloc::vec![WEB1];
    t.eq("a unit among the files", why(&mut e, x), s("ok"));
    run_for(&mut e, 2 * SEC);
    t.eq("it is installed", (e.file_store(WEB1).is_some_and(|f| f.exists("/etc/systemd/system/web2.service")), state(&e, WEB1, "web2")), (true, Some(UnitState::Active)));
}

// ---------------------------------------------------------------- determinism

fn determinism_t(t: &mut T) {
    let go = || {
        let mut e = engine(&project());
        run_for(&mut e, 100 * MS);
        let _ = cli(&mut e, DEV1, "systemctl start crash", 0);
        let _ = e.apply(Action::Deploy { spec: spec(http(), OnFail::Rollback) });
        run_for(&mut e, 700 * MS);
        // a clone in mid-deployment goes on the same
        let mut c = e.clone();
        run_for(&mut e, 10 * SEC);
        run_for(&mut c, 10 * SEC);
        (digest(&e), digest(&c), e.deployments(DEV1)[0].clone())
    };
    let (a, b, r1) = go();
    let (a2, _, r2) = go();
    t.eq("a clone goes on the same", a, b);
    t.eq("two runs", a, a2);
    t.eq("the same run", r1, r2);
    // a scenario twice, the report byte for byte
    let sc = scenario::parse("run 100ms\nDEV1: systemctl start web\ndeploy DEV1 Webserver unit web files /root/app health http 80 /health wait 500ms timeout 2s\nrun 10s\nDEV1: systemctl status web\n").unwrap_or_default();
    let p = project();
    let x = jfn_sim::run_scenario(&p, &sc, "x", false);
    let y = jfn_sim::run_scenario(&p, &sc, "x", false);
    t.eq("the report twice", x.report.clone(), y.report);
    t.check(&format!("it deployed: {}", x.report), x.report.contains("deploy 1: WEB1: fertig") && x.ok);
    // K1's recording replays a deployment from its Action
    let mut d = jfn_sim::Inline::new(engine(&project()));
    d.rewind_on(jfn_sim::Budget::default());
    let _ = d.send(jfn_sim::Cmd::RunFor(100 * MS), 0);
    let _ = d.send(jfn_sim::Cmd::Cli { dev: DEV1, line: s("systemctl start crash") }, 0);
    let _ = d.send(jfn_sim::Cmd::Apply(Action::Deploy { spec: spec(http(), OnFail::Rollback) }), 0);
    let _ = d.send(jfn_sim::Cmd::RunFor(3 * SEC), 0);
    let _ = d.reports();
    let pos = d.pos();
    let replayed = d.restore_pos(pos, None).and_then(|mut r| d.rewind().map(|rw| (r.run(rw, usize::MAX), r))).map(|(_, r)| r.finish());
    t.eq("a replay equals the run", replayed.as_ref().map(|p| digest(&p.eng)), Some(digest(&d.eng)));
    t.eq("a replay has the run", replayed.as_ref().map(|p| p.eng.deployments(DEV1)[0].clone()), Some(d.eng.deployments(DEV1)[0].clone()));
    // the keyframes count units and deployments
    let mut e = engine(&project());
    run_for(&mut e, 100 * MS);
    let a = jfn_sim::keyframe::footprint(&e);
    let _ = cli(&mut e, DEV1, "systemctl start web", 100 * MS);
    let _ = e.apply(Action::Deploy { spec: spec(http(), OnFail::Stop) });
    run_for(&mut e, 100 * MS);
    t.check("the footprint grows", jfn_sim::keyframe::footprint(&e) > a);
}

fn scenario_t(t: &mut T) {
    let sc = scenario::parse("deploy DEV1 Webserver unit web files /root/app,/etc/x health http 8080 /health 204 wait 2s timeout 5s rollback\ndeploy DEV1 WEB1,WEB2 unit api\ndeploy DEV1 G unit a health tcp 80\nexpect WEB1 unit web active\nexpect WEB1 unit web enabled\nexpect DEV1 deploy done\n");
    match sc {
        Ok(sc) => {
            let k: Vec<&StepKind> = sc.steps.iter().map(|x| &x.kind).collect();
            match k[0] {
                StepKind::Deploy(d) => {
                    t.eq("names", (d.src.as_str(), d.hosts.as_str(), d.unit.as_str()), ("DEV1", "Webserver", "web"));
                    t.eq("files", d.files.clone(), alloc::vec![s("/root/app"), s("/etc/x")]);
                    t.eq("health", d.health.clone(), Health::Http { port: 8080, path: s("/health"), status: 204 });
                    t.eq("times", (d.wait, d.timeout, d.on_fail), (2 * SEC, 5 * SEC, OnFail::Rollback));
                }
                _ => t.check("a deploy step", false),
            }
            match k[1] {
                StepKind::Deploy(d) => t.eq("defaults", (d.health.clone(), d.wait, d.timeout, d.on_fail), (Health::Active, SEC, 10 * SEC, OnFail::Stop)),
                _ => t.check("a deploy step", false),
            }
            match k[2] {
                StepKind::Deploy(d) => t.eq("tcp", d.health.clone(), Health::Tcp { port: 80 }),
                _ => t.check("a deploy step", false),
            }
            t.eq("expect unit", k[3], &StepKind::Expect(Expect::Unit { dev: s("WEB1"), unit: s("web"), state: s("active") }));
            t.eq("expect deploy", k[5], &StepKind::Expect(Expect::Deploy { dev: s("DEV1"), state: s("done") }));
        }
        Err(e) => t.check(&format!("it parses: {:?}", e), false),
    }
    let bad = "deploy DEV1 G\ndeploy DEV1 G unit a health tcp\ndeploy DEV1 G unit a wait bald\nexpect WEB1 unit web lustig\nexpect DEV1 deploy lustig\n";
    match scenario::parse(bad) {
        Ok(_) => t.check("errors found", false),
        Err(e) => t.eq("error lines", e.iter().map(|x| x.line).collect::<Vec<_>>(), alloc::vec![1, 2, 3, 4, 5]),
    }
    t.eq("as a check", jfn_sim::goal::check_text(&Expect::Unit { dev: s("A"), unit: s("b"), state: s("failed") }), s("A unit b failed"));
    // the runner's unknowns
    let p = project();
    let sc = scenario::parse("deploy DEV1 Keine unit web\nexpect WEB1 unit nix active\n").unwrap_or_default();
    let o = jfn_sim::run_scenario(&p, &sc, "x", false);
    t.check(&format!("unknown group: {}", o.report), o.report.contains("das Gerät oder die Gruppe Keine gibt es nicht") && !o.ok);
    t.check("unknown unit", o.report.contains("(das gibt es nicht)"));
}

// ---------------------------------------------------------------- the lab

fn lab_t(t: &mut T) {
    let p = Project::from_text(LAB).unwrap_or_else(|_| Project::new("lab", 1));
    t.check("the lab loads", p.lab.is_some());
    let Some(lab) = p.lab.clone() else { return };
    t.eq("the lab's issues", lab.issues(&p).iter().map(|m| m.text()).collect::<Vec<_>>(), Vec::<String>::new());
    t.eq("its goals", lab.goals.len(), 3);
    let none = |_: u32| String::new();
    let dev = |n: &str| p.find(n).map_or(0, |d| d.id);
    // version 1 runs as units; the load goes through; nothing is reached
    let mut e = engine(&p);
    run_for(&mut e, 10 * SEC);
    let mut run = LabRun::new(&lab);
    t.eq("unsolved: no goal", run.eval(&p, &e, &none), Vec::<usize>::new());
    for h in ["API1", "API2", "API3"] {
        t.eq(&format!("{} runs its unit", h), state(&e, dev(h), "katalog"), Some(UnitState::Active));
        t.eq(&format!("{}: version 1", h), e.metric(dev(h), "katalog", "version"), Some(1));
    }
    t.eq("ORD2 runs its unit", state(&e, dev("ORD2"), "bestell"), Some(UnitState::Active));
    t.check("the load is answered", e.metric(dev("GEN"), "last", "ok").is_some_and(|x| x > 100));
    t.eq("no error so far", (e.metric(dev("GEN"), "last", "failed"), e.metric(dev("GEN"), "last", "http_errors")), (Some(0), Some(0)));
    t.eq("all backends up", e.metric(dev("LB1"), "katalog", "backends_up"), Some(3));
    // solved as the IDE does it: first the catalogue, then the orders
    let spec_of = |group: &str, unit: &str, folder: &str, port: u16| {
        let g = p.groups.iter().find(|g| g.name == group).map_or(0, |g| g.id);
        DeploySpec {
            src: dev("DEV1"),
            hosts: jfn_model::group::Groups::of(&p).devices(&p, g),
            files: alloc::vec![String::from(folder)],
            unit: String::from(unit),
            health: Health::Http { port, path: s("/health"), status: 200 },
            wait: 2 * SEC,
            timeout: 10 * SEC,
            on_fail: OnFail::Stop,
        }
    };
    let mut good = e.clone();
    t.check("the catalogue goes", good.apply(Action::Deploy { spec: spec_of("Katalog", "katalog", "/root/katalog", 8080) }).is_ok());
    let mut reached = Vec::new();
    for _ in 0..40 {
        run_for(&mut good, 500 * MS);
        reached.extend(run.eval(&p, &good, &none));
    }
    t.eq("goal 1 reached", reached.clone(), alloc::vec![0]);
    t.eq("deployed one after the other", good.deployments(dev("DEV1"))[0].state, RunState::Done);
    t.check("the orders go", good.apply(Action::Deploy { spec: spec_of("Bestellung", "bestell", "/root/bestell", 8081) }).is_ok());
    for _ in 0..40 {
        run_for(&mut good, 500 * MS);
        reached.extend(run.eval(&p, &good, &none));
    }
    let g = dev("GEN");
    t.eq("no request failed", (good.metric(g, "last", "failed"), good.metric(g, "last", "http_errors"), good.metric(g, "last", "timeouts")), (Some(0), Some(0), Some(0)));
    t.eq("all goals reached", reached, alloc::vec![0, 1, 2]);
    t.check("done", run.done());
    t.eq("version 2 on API2", good.metric(dev("API2"), "katalog", "version"), Some(2));
    t.eq("the LB never lost them all", good.series_value(dev("LB1"), "katalog", "backends_up", scenario::Agg::Min, 0, good.now()).map(|v| v.milli >= 2000), Ok(true));
    // all hosts at once: requests fail, so the last goal stays open
    let mut bad = e.clone();
    for h in ["API1", "API2", "API3"] {
        let _ = bad.cli_line(dev(h), "systemctl restart katalog");
    }
    run_for(&mut bad, 5 * SEC);
    t.check(&format!("all at once: errors {:?}", bad.metric(g, "last", "http_errors")), bad.metric(g, "last", "http_errors").is_some_and(|x| x > 0));
    // the check over the device's goals sees a broken deployment too
    let mut run = LabRun::new(&lab);
    t.eq("all at once: no goal yet", run.eval(&p, &bad, &none), Vec::<usize>::new());
}
