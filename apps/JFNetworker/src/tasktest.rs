// tasktest -- checks of the labs and their tasks (part of --selftest, group "task"): the lab
// section of a project file (texts as strings, pairs and objects, a round trip, projects without
// one, members left out), what Lab::issues finds, single expectations (parse_expectation,
// Expect::dev), the judging of jfn_sim's goal.rs (the same report lines as the runner's, the
// reasons as messages and their English), LabRun (goals that stay reached, an ordered lab,
// broken and empty checks, a fresh start); and the four labs of assets/labs: listed in
// app://labs, without issues, in both languages, no goal reached while nothing is done, every
// goal once the lab is solved the way the window solves it (the project edited and the
// simulator following it as doc/sim.rs does, console lines for the campus), the quorum
// lesson of the shards (two replicas with strong do not survive a node, three do; eventual
// does), and the same moments twice

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::{Field, Value};
use jfn_model::lab::{Goal, Lab, Text};
use jfn_model::project::{PortRef, Project};
use jfn_model::scenario::{self, Cmp, Expect, Span};
use jfn_sim::goal::{check_text, judge, Found, LabRun, Verdict};
use jfn_sim::{Cmd, Engine, Report, SeriesErr, SEC};

use crate::doc::sim::Sim;
use crate::selftest::T;

const CAMPUS: &str = include_str!("../assets/labs/01-campus-vlan.jfnet");
const LB: &str = include_str!("../assets/labs/02-lastverteilung.jfnet");
const SHARDS: &str = include_str!("../assets/labs/03-shards.jfnet");
const CACHE: &str = include_str!("../assets/labs/04-cache.jfnet");
const TWO_PCS: &str = include_str!("../tests/fixtures/two_pcs.jfnet");
const BUDGET: usize = 10_000_000;

fn s(x: &str) -> String {
    String::from(x)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

// the rows of engine.tsv: German template -> English
fn english(k: &str) -> Option<&'static str> {
    include_str!("locales/engine.tsv").lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).find(|r| r.0 == k).map(|r| r.1)
}

pub(crate) fn run(t: &mut T) {
    format_t(t);
    expect_t(t);
    judge_t(t);
    labrun_t(t);
    bundled_t(t);
    campus_t(t);
    lb_t(t);
    shards_t(t);
    cache_t(t);
}

// ---------------------------------------------------------------- the file format

fn format_t(t: &mut T) {
    // a text: a string (German only), a pair, an object
    t.eq("text string", Text::take(Some(&Value::string("Ziel"))), Text::new("Ziel", ""));
    t.eq("text pair", Text::take(Some(&Value::Array(vec![Value::string("Ziel"), Value::string("Goal")]))), Text::new("Ziel", "Goal"));
    t.eq("text object", Text::take(Some(&Value::object([("de", Value::string("Ziel")), ("en", Value::string("Goal"))]))), Text::new("Ziel", "Goal"));
    t.eq("text written as a string", Text::new("Ziel", "").put(), Value::string("Ziel"));
    t.eq("text written as a pair", Text::new("Ziel", "Goal").put(), Value::Array(vec![Value::string("Ziel"), Value::string("Goal")]));
    t.eq("text in English", Text::new("Ziel", "Goal").get(true), "Goal");
    t.eq("text without English", Text::new("Ziel", "").get(true), "Ziel");
    t.eq("text missing", Text::take(None), Text::default());
    // a lab survives the file
    let mut p = Project::new("Lab", 3);
    let pc = p.add_device("pc", 0, 0).unwrap_or(0);
    let lab = Lab {
        title: Text::new("Titel", "Title"),
        summary: Text::new("Kurz", ""),
        topic: s("net"),
        intro: vec![Text::new("Absatz", "Paragraph")],
        ordered: true,
        goals: vec![Goal { text: Text::new("Ziel", "Goal"), checks: vec![s("PC1 route 0.0.0.0/0")], hints: vec![Text::new("Hinweis", "Hint")] }],
        hints: vec![Text::new("Allgemein", "General")],
    };
    p.lab = Some(lab.clone());
    let text = p.to_text();
    t.check("lab written", text.contains("\"lab\": {") && text.contains("\"title\": [\n      \"Titel\",\n      \"Title\"\n    ]") && text.contains("\"summary\": \"Kurz\""));
    t.eq("lab round trip", Project::from_text(&text).ok().and_then(|q| q.lab), Some(lab));
    // a free project stays as it was
    p.lab = None;
    let text = p.to_text();
    t.check("no lab: null", text.contains("\"lab\": null"));
    t.eq("no lab read", Project::from_text(&text).map(|q| q.lab.is_none()), Ok(true));
    // members left out take their defaults
    let min = Project::from_text("{\"format\": \"jfn\", \"version\": 1, \"devices\": [{\"id\": 1, \"name\": \"PC1\", \"model\": \"pc\"}], \"lab\": {\"title\": \"T\", \"goals\": [{\"text\": \"x\", \"checks\": [\"expect PC1 route 0.0.0.0/0\"]}]}}");
    let ml = min.ok().and_then(|q| q.lab);
    t.check("lab defaults", ml.as_ref().is_some_and(|l| l.title == Text::new("T", "") && !l.ordered && l.intro.is_empty() && l.goals.len() == 1 && l.goals[0].hints.is_empty() && l.topic.is_empty()));
    // what issues finds
    let mut q = Project::new("Q", 1);
    let _ = q.add_device("pc", 0, 0);
    let texts = |l: &Lab, q: &Project| l.issues(q).iter().map(|m| m.text()).collect::<Vec<_>>();
    t.eq("issues of an empty lab", texts(&Lab::default(), &q), vec![s("Das Labor hat keinen Titel."), s("Das Labor hat keine Ziele.")]);
    let bad = Lab {
        title: Text::new("T", ""),
        goals: vec![
            Goal { text: Text::new("a", ""), checks: Vec::new(), hints: Vec::new() },
            Goal { text: Text::default(), checks: vec![s("PC1 frobnicates"), s("SRV9 route 10.0.0.0/8"), s("hops icmp >= 1")], hints: Vec::new() },
        ],
        ..Lab::default()
    };
    let found = texts(&bad, &q);
    t.eq("issues: a goal without checks", found.first().cloned(), Some(s("Ziel 1 hat keine Prüfung.")));
    t.check("issues: no text, a bad check, a missing device", found.len() == 4 && found[1] == "Ziel 2 hat keinen Text." && found[2].starts_with("Ziel 2: \u{201e}PC1 frobnicates\u{201c} ist keine Prüfung: ") && found[3] == "Ziel 2: das Gerät SRV9 gibt es im Projekt nicht.");
    let good = Lab { title: Text::new("T", ""), goals: vec![Goal { text: Text::new("g", ""), checks: vec![s("PC1 route 0.0.0.0/0"), s("hops arp >= 1")], hints: Vec::new() }], ..Lab::default() };
    t.check("issues: none", p.device(pc).is_some() && texts(&good, &p).is_empty());
}

// ---------------------------------------------------------------- single expectations

fn expect_t(t: &mut T) {
    let a = scenario::parse_expectation("expect PC1 route 10.0.0.0/8");
    t.eq("with expect", a.clone(), scenario::parse_expectation("PC1 route 10.0.0.0/8"));
    t.check("a route", matches!(a, Ok(Expect::Route { .. })));
    t.check("empty", scenario::parse_expectation("  ").is_err() && scenario::parse_expectation("expect").is_err());
    t.check("unknown", scenario::parse_expectation("PC1 frobnicates").is_err());
    t.eq(
        "quoted",
        scenario::parse_expectation("PC3 output contains \", 0% packet loss\""),
        Ok(Expect::Output { dev: s("PC3"), needle: s(", 0% packet loss"), negate: false }),
    );
    match scenario::parse_expectation("GEN svc load metric max p99_us < 100000 over 10s") {
        Ok(Expect::Series { dev, svc, name, cmp, milli, span, .. }) => {
            t.eq("series", (dev.as_str(), svc.as_str(), name.as_str(), cmp, milli, span), ("GEN", "load", "p99_us", Cmp::Lt, 100_000_000, Span::Last(10 * SEC)));
        }
        other => t.check(&format!("series: {:?}", other), false),
    }
    let dev = |x: &str| scenario::parse_expectation(x).ok().and_then(|e| e.dev().map(String::from));
    t.eq("dev of a route", dev("R1 no route 10.0.0.0/8"), Some(s("R1")));
    t.eq("dev of hops", dev("hops icmp >= 1"), None);
    // the check as a lab writes it
    let x = scenario::parse_expectation("KV1 svc kv metric max members_up <= 2 over 10s").unwrap_or(Expect::Hops { proto: s("x"), cmp: Cmp::Eq, n: 0 });
    t.eq("check text", check_text(&x), s("KV1 svc kv metric max members_up <= 2 over 10s"));
}

// ---------------------------------------------------------------- judging

fn judge_t(t: &mut T) {
    let Ok(p) = Project::from_text(TWO_PCS) else {
        t.check("two_pcs loads", false);
        return;
    };
    // the runner's report and judge give the same lines at the same moment
    let checks = ["PC1 route 10.0.0.0/24 via connected", "PC1 route 10.9.0.0/16", "PC1 no route 10.0.0.0/24", "hops icmp >= 1", "hops frobnet >= 1", "PC1 svc nosuch metric x == 1", "NOPE route 1.2.3.0/24", "PC1 output contains \"0% packet loss\"", "PC1 svc nosuch metric x > 1 over 1s", "PC1 svc nosuch metric x > 1 between 1s and 99s"];
    let mut scn = String::from("PC1: ping -c 1 10.0.0.2\nrun 2s\n");
    for c in checks {
        scn.push_str(&format!("expect {}\n", c));
    }
    let Ok(sc) = scenario::parse(&scn) else {
        t.check("judge scenario parses", false);
        return;
    };
    let out = jfn_sim::run_scenario(&p, &sc, "judge", false);
    let pc1 = p.find("PC1").map_or(0, |d| d.id);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let _ = e.cli_line(pc1, "ping -c 1 10.0.0.2");
    e.run_until(2 * SEC, BUDGET);
    let mut said = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev, bytes, .. } = r
            && dev == pc1
        {
            said.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    for c in checks {
        let x = scenario::parse_expectation(c).unwrap_or(Expect::Hops { proto: s("x"), cmp: Cmp::Eq, n: 0 });
        let v = judge(&p, &e, &x, &|d| if d == pc1 { said.clone() } else { String::new() });
        let (mark, rep) = (if v.held { "ok" } else { "FEHLER" }, v.report());
        t.check(&format!("runner's line for {}: {} {}", c, mark, rep), out.report.lines().any(|l| l.ends_with(&rep) && l.split_whitespace().nth(1) == Some(mark)));
    }
    // what the reasons say, and their English
    let x = Expect::Hops { proto: s("icmp"), cmp: Cmp::Ge, n: 1 };
    let v = |held: bool, found: Found| Verdict { held, text: s("expect hops icmp >= 1"), found, x: x.clone() };
    let cases: Vec<(Verdict, &str)> = vec![
        (v(false, Found::NoDevice), "das Gerät gibt es nicht"),
        (v(false, Found::Value { text: s("12"), unit: "" }), "gemessen: 12"),
        (v(false, Found::Value { text: s("245300.5"), unit: "\u{b5}s" }), "gemessen: 245300.5 \u{b5}s"),
        (v(true, Found::Value { text: s("120"), unit: "/s" }), "gemessen: 120/s"),
        (v(false, Found::Count(3)), "gezählt: 3"),
        (v(false, Found::Route(None)), "keine Route"),
        (v(false, Found::Route(Some(s("S via 10.0.1.2 dev Gi0/1")))), "gefunden: S via 10.0.1.2 dev Gi0/1"),
        (v(false, Found::Routes(2)), "es gibt 2 Routen dorthin"),
        (v(false, Found::UnknownProto), "unbekanntes Protokoll"),
        (v(false, Found::NoMetric), "keinen solchen Dienst oder Wert"),
        (v(false, Found::Series(SeriesErr::NoService)), "keinen solchen Dienst"),
        (v(false, Found::Series(SeriesErr::NoSeries)), "keine solche Reihe"),
        (v(false, Found::Series(SeriesErr::NoValues)), "keine Werte im Zeitraum"),
        (v(false, Found::Series(SeriesErr::Gone)), "das Fenster der Latenzen hält den Zeitraum nicht mehr"),
        (v(false, Found::Later), "der Zeitraum endet erst nach jetzt"),
    ];
    for (verdict, want) in &cases {
        let m = verdict.reason();
        t.eq(&format!("reason {:?}", verdict.found), m.text(), s(want));
        t.check(&format!("reason in English: {}", m.tpl), english(m.tpl).is_some_and(|en| !en.is_empty()));
    }
    t.check("held without a value: no reason", v(true, Found::Nothing).reason().is_empty());
    let out_x = Expect::Output { dev: s("PC1"), needle: s("x"), negate: false };
    let missing = Verdict { held: false, text: s(""), found: Found::Nothing, x: out_x.clone() };
    t.eq("reason of an output", missing.reason().text(), s("nicht in der Ausgabe"));
    let there = Verdict { held: false, text: s(""), found: Found::Nothing, x: Expect::Output { dev: s("PC1"), needle: s("x"), negate: true } };
    t.eq("reason of a negated output", there.reason().text(), s("steht in der Ausgabe"));
    // judging reads only: the same engine judges the same
    let x = scenario::parse_expectation("hops icmp >= 2").unwrap_or(out_x);
    let a = judge(&p, &e, &x, &|_| String::new());
    let b = judge(&p, &e, &x, &|_| String::new());
    t.check("judging twice", a == b && a.found == Found::Count(4) && a.held);
}

// ---------------------------------------------------------------- LabRun

fn labrun_t(t: &mut T) {
    let Ok(p) = Project::from_text(TWO_PCS) else { return };
    let goal = |checks: &[&str]| Goal { text: Text::new("g", ""), checks: checks.iter().map(|c| s(c)).collect(), hints: Vec::new() };
    let lab = Lab {
        title: Text::new("T", ""),
        goals: vec![goal(&["hops icmp >= 2"]), goal(&["hops icmp >= 2", "PC1 route 10.9.0.0/16"]), goal(&[]), goal(&["PC1 frobnicates"])],
        ..Lab::default()
    };
    let mut run = LabRun::new(&lab);
    t.check("broken check", matches!(&run.goals[3].checks, Err((0, _))));
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let none = |_: u32| String::new();
    t.eq("nothing yet", run.eval(&p, &e, &none), Vec::<usize>::new());
    t.eq("judged at", run.at, Some(0));
    let _ = e.cli_line(p.find("PC1").map_or(0, |d| d.id), "ping -c 1 10.0.0.2");
    e.run_until(2 * SEC, BUDGET);
    t.eq("first goal reached", run.eval(&p, &e, &none), vec![0]);
    t.eq("reached at", run.goals[0].reached, Some(2 * SEC));
    t.check("second open: one check fails", run.goals[1].reached.is_none() && run.goals[1].last.len() == 2 && run.goals[1].last[0].held && !run.goals[1].last[1].held);
    // a reached goal stays, whatever happens later; no checks and broken ones never count
    let mut later = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    later.run_until(3 * SEC, BUDGET);
    t.eq("stays reached", run.eval(&p, &later, &none), Vec::<usize>::new());
    t.check("stays reached: the moment", run.goals[0].reached == Some(2 * SEC) && run.reached() == 1 && !run.done());
    t.check("empty and broken goals", run.goals[2].reached.is_none() && run.goals[3].reached.is_none() && run.goals[3].last.is_empty());
    // an ordered lab looks at a goal once the ones before it are reached
    let ordered = Lab { title: Text::new("T", ""), ordered: true, goals: vec![goal(&["PC1 route 10.9.0.0/16"]), goal(&["hops icmp >= 2"])], ..Lab::default() };
    let mut run = LabRun::new(&ordered);
    t.eq("ordered: the second waits", run.eval(&p, &e, &none), Vec::<usize>::new());
    t.check("ordered: open", run.open(0) && !run.open(1) && run.goals[1].last.is_empty());
    let free = Lab { ordered: false, ..ordered.clone() };
    let mut run = LabRun::new(&free);
    t.eq("unordered: the second counts", run.eval(&p, &e, &none), vec![1]);
    // both at one moment: an ordered lab reaches them together
    let both = Lab { title: Text::new("T", ""), ordered: true, goals: vec![goal(&["hops icmp >= 2"]), goal(&["hops arp >= 1"])], ..Lab::default() };
    let mut run = LabRun::new(&both);
    t.eq("ordered: one moment", run.eval(&p, &e, &none), vec![0, 1]);
    t.check("done", run.done() && run.reached() == 2);
}

// ---------------------------------------------------------------- the labs of the app

/// a lab running as the window runs it: the project, the simulator following it, its goals
/// judged each second, what each device printed since its last console line
struct Run {
    p: Project,
    sim: Sim,
    run: LabRun,
    out: BTreeMap<u32, String>,
}

impl Run {
    fn new(text: &str) -> Option<Run> {
        let p = Project::from_text(text).ok()?;
        let run = LabRun::new(p.lab.as_ref()?);
        let sim = Sim::new(&p);
        Some(Run { p, sim, run, out: BTreeMap::new() })
    }
    fn id(&self, name: &str) -> u32 {
        self.p.find(name).map_or(0, |d| d.id)
    }
    fn reports(&mut self) {
        for r in self.sim.take_reports() {
            if let Report::CliOut { dev, bytes, .. } = r {
                self.out.entry(dev).or_default().push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    /// `secs` seconds of virtual time, the goals judged after each
    fn secs(&mut self, secs: u64) {
        for _ in 0..secs {
            let _ = self.sim.send(Cmd::RunFor(SEC));
            self.reports();
            let out = &self.out;
            self.run.eval(&self.p, self.sim.engine(), &|d| out.get(&d).cloned().unwrap_or_default());
        }
    }
    /// a line typed on a device's console
    fn cli(&mut self, dev: &str, line: &str) {
        let id = self.id(dev);
        self.reports();
        self.out.insert(id, String::new());
        let _ = self.sim.send(Cmd::Cli { dev: id, line: s(line) });
    }
    /// an edit of the project (the window's): the simulator follows
    fn edit(&mut self, f: impl FnOnce(&mut Project)) {
        f(&mut self.p);
        self.sim.sync(&self.p);
    }
    /// a setting of the first service of a device
    fn set(&mut self, dev: &str, key: &str, v: Value) {
        let id = self.id(dev);
        self.edit(|p| {
            if let Some(c) = p.device_mut(id).and_then(|d| d.services.first_mut()) {
                c.config.insert(key, v);
            }
        });
    }
    fn power(&mut self, dev: &str, on: bool) {
        let id = self.id(dev);
        self.edit(|p| {
            if let Some(d) = p.device_mut(id) {
                d.off = !on;
            }
        });
    }
    /// the moments the goals were reached
    fn reached(&self) -> Vec<Option<u64>> {
        self.run.goals.iter().map(|g| g.reached).collect()
    }
    /// what the open goals' failing checks found (for a failing check's name)
    fn why(&self) -> String {
        let mut v = Vec::new();
        for g in self.run.goals.iter().filter(|g| g.reached.is_none()) {
            for x in g.last.iter().filter(|x| !x.held) {
                v.push(x.report());
            }
        }
        v.join("; ")
    }
}

fn bundled_t(t: &mut T) {
    let files = [("01-campus-vlan.jfnet", CAMPUS), ("02-lastverteilung.jfnet", LB), ("03-shards.jfnet", SHARDS), ("04-cache.jfnet", CACHE)];
    // the app's list: every lab of app://labs, by file name
    let listed = crate::workspace::task::labs();
    t.eq("labs listed", listed.iter().map(|l| l.path.clone()).collect::<Vec<_>>(), files.iter().map(|f| format!("app://labs/{}", f.0)).collect::<Vec<_>>());
    for (name, text) in files {
        let Ok(p) = Project::from_text(text) else {
            t.check(&format!("{} loads", name), false);
            continue;
        };
        let Some(lab) = p.lab.clone() else {
            t.check(&format!("{} has a lab", name), false);
            continue;
        };
        t.eq(&format!("{}: no issues", name), lab.issues(&p).iter().map(|m| m.text()).collect::<Vec<_>>(), Vec::<String>::new());
        t.eq(&format!("{}: no problems of the project", name), p.check().len(), 0);
        t.check(&format!("{}: listed with its title", name), listed.iter().any(|l| l.path.ends_with(name) && l.title == lab.title && l.goals == lab.goals.len()));
        // German and English everywhere
        let mut texts: Vec<&Text> = vec![&lab.title, &lab.summary];
        texts.extend(lab.intro.iter());
        texts.extend(lab.hints.iter());
        for g in &lab.goals {
            texts.push(&g.text);
            texts.extend(g.hints.iter());
        }
        t.check(&format!("{}: both languages", name), texts.iter().all(|x| !x.de.is_empty() && !x.en.is_empty() && x.de != x.en));
        t.check(&format!("{}: a topic, goals with hints", name), (lab.topic == "net" || lab.topic == "ds") && lab.goals.len() >= 3 && lab.goals.iter().all(|g| !g.hints.is_empty()));
        // the window's simulator takes the whole project
        t.check(&format!("{}: runs whole", name), Sim::new(&p).errors.is_empty());
    }
}

// the campus: VLAN 20 on MLS1, the route back on R1, the PCs' leases, a ping by name
fn campus_t(t: &mut T) {
    let Some(mut r) = Run::new(CAMPUS) else {
        t.check("campus runs", false);
        return;
    };
    r.secs(30);
    t.eq("campus: nothing reached while nothing is done", r.run.reached(), 0);
    for line in ["enable", "configure terminal", "vlan 20", "name Students", "exit", "interface GigabitEthernet1/0/3", "switchport access vlan 20", "interface GigabitEthernet1/0/4", "switchport access vlan 20", "interface vlan 20", "ip address 10.10.20.1 255.255.255.0", "ip helper-address 10.10.0.1", "end"] {
        r.cli("MLS1", line);
    }
    r.secs(2);
    t.check(&format!("campus: the VLAN interface ({})", r.why()), r.run.goals[0].reached.is_some() && r.run.goals[1].reached.is_none());
    for line in ["enable", "configure terminal", "ip route 10.10.20.0 255.255.255.0 10.10.0.2", "end"] {
        r.cli("R1", line);
    }
    r.cli("PC3", "dhclient eth0");
    r.cli("PC4", "dhclient eth0");
    r.secs(10);
    t.check(&format!("campus: leases and the route ({})", r.why()), r.run.goals[1].reached.is_some() && r.run.goals[2].reached.is_some() && r.run.goals[3].reached.is_none());
    r.cli("PC3", "ping -c 3 server.lab");
    r.secs(4);
    t.check(&format!("campus: solved ({})", r.why()), r.run.done());
}

// the load balancer: 25 s as it is, WEB3 cabled, then in LB1's backends
fn lb_t(t: &mut T) {
    let Some(mut r) = Run::new(LB) else {
        t.check("lb runs", false);
        return;
    };
    r.secs(25);
    t.eq("lb: nothing reached while nothing is done", r.run.reached(), 0);
    let (web3, sw2) = (r.id("WEB3"), r.id("SW2"));
    r.edit(|p| {
        let _ = p.connect(&pr(web3, "eth0"), &pr(sw2, "gi1/0/4"), None);
    });
    r.secs(3);
    t.check(&format!("lb: the cable ({})", r.why()), r.run.goals[0].reached.is_some() && r.run.reached() == 1);
    let lb = r.id("LB1");
    r.edit(|p| {
        if let Some(c) = p.device_mut(lb).and_then(|d| d.services.first_mut()) {
            let mut backs: Vec<Value> = c.config.get("backends").and_then(Value::as_array).map(|a| a.to_vec()).unwrap_or_default();
            backs.push(Value::object([("name", Value::string("web3")), ("addr", Value::string("10.0.2.13"))]));
            c.config.insert("backends", Value::Array(backs));
        }
    });
    r.secs(25);
    t.check(&format!("lb: solved ({})", r.why()), r.run.done());
    let at = r.reached();
    t.check("lb: in order", at.windows(2).all(|w| w[0].is_some() && w[0] <= w[1]));
}

// the shards: replicas, a node off, its return
fn shards_t(t: &mut T) {
    let Some(mut r) = Run::new(SHARDS) else {
        t.check("shards run", false);
        return;
    };
    r.secs(25);
    r.power("KV2", false);
    r.secs(15);
    t.eq("shards: nothing reached with one replica", r.run.reached(), 0);
    r.power("KV2", true);
    r.secs(5);
    // two replicas with strong: the first goal, but the failure is not survived
    for kv in ["KV1", "KV2", "KV3"] {
        r.set(kv, "rf", Value::u64(2));
    }
    r.secs(15);
    t.check(&format!("shards: two replicas ({})", r.why()), r.run.goals[0].reached.is_some());
    r.power("KV2", false);
    r.secs(20);
    t.check("shards: strong with two replicas does not survive", r.run.goals[1].reached.is_none() && r.run.goals[1].last.iter().any(|v| !v.held && v.text.contains("status_5xx")));
    r.power("KV2", true);
    r.secs(5);
    t.check("shards: in order, the return waits", r.run.goals[2].reached.is_none() && !r.run.open(2));
    // three replicas: a majority of two survives
    for kv in ["KV1", "KV2", "KV3"] {
        r.set(kv, "rf", Value::u64(3));
    }
    r.secs(15);
    r.power("KV2", false);
    r.secs(15);
    t.check(&format!("shards: three replicas survive ({})", r.why()), r.run.goals[1].reached.is_some());
    r.power("KV2", true);
    r.secs(15);
    t.check(&format!("shards: solved ({})", r.why()), r.run.done());
    // eventual with two replicas survives as well
    let Some(mut ev) = Run::new(SHARDS) else { return };
    ev.secs(10);
    for kv in ["KV1", "KV2", "KV3"] {
        ev.set(kv, "rf", Value::u64(2));
        ev.set(kv, "consistency", Value::string("eventual"));
    }
    ev.secs(15);
    ev.power("KV2", false);
    ev.secs(15);
    ev.power("KV2", true);
    ev.secs(15);
    t.check(&format!("shards: eventual with two replicas ({})", ev.why()), ev.run.done());
}

// the cache: 20 s as it is, then more entries for longer -> the goals reached before the change
fn cache_solved(r: &mut Run) -> usize {
    r.secs(20);
    let before = r.run.reached();
    r.set("CACHE1", "entries", Value::u64(1000));
    r.set("CACHE1", "ttl_ms", Value::u64(30000));
    r.secs(30);
    before
}

fn cache_t(t: &mut T) {
    let Some(mut r) = Run::new(CACHE) else {
        t.check("cache runs", false);
        return;
    };
    t.eq("cache: nothing reached while nothing is done", cache_solved(&mut r), 0);
    t.check(&format!("cache: solved ({})", r.why()), r.run.done());
    let at = r.reached();
    t.check("cache: in order", at.windows(2).all(|w| w[0].is_some() && w[0] <= w[1]));
    // the same moments again
    if let Some(mut again) = Run::new(CACHE) {
        cache_solved(&mut again);
        t.eq("cache: the same moments twice", again.reached(), at);
    }
    // a hundred entries are too few
    let Some(mut few) = Run::new(CACHE) else { return };
    few.secs(5);
    few.set("CACHE1", "entries", Value::u64(100));
    few.set("CACHE1", "ttl_ms", Value::u64(30000));
    few.secs(25);
    t.check("cache: a hundred entries are too few", few.run.goals[0].reached.is_none());
}
