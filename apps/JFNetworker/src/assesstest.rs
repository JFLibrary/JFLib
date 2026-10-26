// assesstest -- checks of assessments (A1; part of --selftest, group "assess"): the format (items
// with what is at its default left out, "points" 1 when missing, a round trip, a lab without an
// assessment as before, what Assess::issues finds), canonical values (canon, RouteVal: masks,
// prefixes, IOS and Linux spellings), the reach expectation (parsed, judged on a copy of the
// engine: answered, not, negated, through a name, an unknown name, a device that is off, the
// engine untouched, the runner's line), each item kind with its normalisation (hostname,
// address with masks, interface spellings and IPv6, state, VLAN, mode, a static route in IOS and
// Linux spellings, a service's setting as JSON, a cable in either direction and with any port,
// configuration lines typed abbreviated, "no shutdown" and a refused line, Exact and Any), points
// and percentages, a renamed device found through its id, the feedback modes and the time limit
// (workspace/assess.rs's Run on a clock of its own), the authoring steps (capture, the tree of
// candidates, the start network: every kind it takes away fails there and holds in the target),
// the labs of assets/labs unchanged, and the samples of assets/assess: unsolved low, their
// target full, solved the way the window solves them (consoles and edits) full, the same twice

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::{Field, Value};
use jfn_model::lab::{canon, Assess, Compare, Feedback, Item, ItemKind, Lab, RouteVal, Text};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, Expect};
use jfn_sim::assess::{candidates, capture, config_pairs, item_key, item_text, normalise, start_network, would_change, Grader, Score};
use jfn_sim::goal::{judge, Found};
use jfn_sim::{Action, Cmd, Engine, Report, SEC};

use crate::doc::sim::Sim;
use crate::selftest::T;
use crate::workspace::assess::{clock_text, Run, EDIT_US, EVERY_US};

const LAN_ROUTER: &str = include_str!("../tests/fixtures/lan_router.jfnet");
const STICK: &str = include_str!("../tests/fixtures/router_on_a_stick.jfnet");
const CAMPUS: &str = include_str!("../assets/labs/01-campus-vlan.jfnet");
const LB: &str = include_str!("../assets/labs/02-lastverteilung.jfnet");
const SHARDS: &str = include_str!("../assets/labs/03-shards.jfnet");
const CACHE: &str = include_str!("../assets/labs/04-cache.jfnet");
const S1: &str = include_str!("../assets/assess/01-router-zwei-netze.jfnet");
const S2: &str = include_str!("../assets/assess/02-vlans-trunk.jfnet");
const S3: &str = include_str!("../assets/assess/03-statische-routen.jfnet");
const BUDGET: usize = 10_000_000;

fn s(x: &str) -> String {
    String::from(x)
}

// the rows of engine.tsv and ui.tsv: German template -> English
fn english(k: &str) -> Option<&'static str> {
    let rows = include_str!("locales/engine.tsv").lines().chain(include_str!("locales/ui.tsv").lines());
    rows.filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).find(|r| r.0 == k).map(|r| r.1)
}

/// an item of a kind on a device's port with a text value
fn item(what: &str, dev: &str, port: &str, value: &str) -> Item {
    Item { what: s(what), dev: s(dev), port: s(port), value: Value::string(value), points: 1, ..Item::default() }
}

/// a config item
fn config(dev: &str, lines: &[&str]) -> Item {
    Item { what: s("config"), dev: s(dev), value: Value::Array(lines.iter().map(|l| Value::string(*l)).collect()), points: 1, ..Item::default() }
}

/// a check item
fn check(checks: &[&str], points: u32) -> Item {
    Item { what: s("check"), checks: checks.iter().map(|c| s(c)).collect(), points, ..Item::default() }
}

/// a project with its engine after `secs` seconds
fn running(text: &str, secs: u64) -> Option<(Project, Engine)> {
    let p = Project::from_text(text).ok()?;
    let mut e = Engine::from_project(&p).ok()?;
    e.run_until(secs * SEC, BUDGET);
    Some((p, e))
}

/// the items graded against a project and its engine
fn grade(items: Vec<Item>, p: &Project, e: &Engine) -> Score {
    let a = Assess { items, ..Assess::default() };
    Grader::new(&a).grade(p, e, &|_| String::new())
}

/// whether each item holds
fn held(items: Vec<Item>, p: &Project, e: &Engine) -> Vec<bool> {
    grade(items, p, e).items.iter().map(|g| g.held).collect()
}

/// lines typed at a device's console now
fn cli(e: &mut Engine, dev: u32, lines: &[&str]) {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    let now = e.now();
    e.run_until(now, BUDGET);
}

pub(crate) fn run(t: &mut T) {
    format_t(t);
    canon_t(t);
    reach_t(t);
    kinds_t(t);
    config_t(t);
    points_t(t);
    run_t(t);
    author_t(t);
    old_labs_t(t);
    samples_t(t);
    solve1_t(t);
    solve2_t(t);
    solve3_t(t);
}

// ---------------------------------------------------------------- the format

fn format_t(t: &mut T) {
    // an item writes what is not at its default; "points" defaults to 1
    let it = item("address", "R1", "Gi0/0/0", "10.0.1.1/24");
    let v = it.put();
    t.check("item: members", v.get("what").and_then(Value::as_str) == Some("address") && v.get("port").is_some() && v.get("svc").is_none() && v.get("cmp").is_none() && v.get("checks").is_none() && v.get("id").is_none() && v.get("text").is_none());
    t.eq("item round trip", Item::take(Some(&v)), it.clone());
    let min = agent_core::json::parse(b"{\"what\": \"hostname\", \"dev\": \"R1\", \"value\": \"R1\"}").unwrap_or(Value::Null);
    t.eq("item: points default to 1", Item::take(Some(&min)).points, 1);
    let zero = agent_core::json::parse(b"{\"what\": \"hostname\", \"points\": 0}").unwrap_or(Value::Null);
    t.eq("item: 0 points stay", Item::take(Some(&zero)).points, 0);
    let any = Item { cmp: Compare::Any, value: Value::Array(vec![Value::string("a"), Value::string("b")]), ..it.clone() };
    t.eq("item: any", Item::take(Some(&any.put())).values().len(), 2);
    t.eq("item: values of one", it.values().len(), 1);
    // an assessment survives the file
    let Some((p, e)) = running(LAN_ROUTER, 1) else {
        t.check("lan_router runs", false);
        return;
    };
    let mut q = p.clone();
    let asm = Assess { feedback: Feedback::End, time_limit_s: 900, items: vec![it.clone(), check(&["PC1 reaches 10.0.2.10"], 4)], target: capture(&p, &e) };
    q.lab = Some(Lab { title: Text::new("Titel", "Title"), assess: Some(asm.clone()), ..Lab::default() });
    let text = q.to_text();
    t.check("assessment written", text.contains("\"assess\": {") && text.contains("\"feedback\": \"end\"") && text.contains("\"time_limit_s\": 900") && text.contains("\"target\": {"));
    t.eq("assessment round trip", Project::from_text(&text).ok().and_then(|x| x.lab).and_then(|l| l.assess), Some(asm.clone()));
    t.eq("total", asm.total(), 5);
    // a lab without one: "assess": null, and as before
    q.lab = Some(Lab { title: Text::new("T", ""), ..Lab::default() });
    let text = q.to_text();
    t.check("no assessment: null", text.contains("\"assess\": null"));
    t.eq("no assessment read", Project::from_text(&text).ok().and_then(|x| x.lab).map(|l| l.assess.is_none()), Some(true));
    // what issues finds
    let texts = |a: &Assess| a.issues(&p).iter().map(|m| m.text()).collect::<Vec<_>>();
    t.eq("issues of an empty assessment", texts(&Assess::default()), vec![s("Die Aufgabe hat keine Kriterien."), s("Die Aufgabe hat kein Zielnetz.")]);
    let bad = Assess {
        items: vec![
            Item { what: s("frob"), ..Item::default() },
            item("address", "", "Gi0/0/0", "10.0.1.1/24"),
            item("address", "R9", "Gi0/0/0", "10.0.1.1/24"),
            item("state", "R1", "", "up"),
            item("vlan", "R1", "Gi0/0/0", "4095"),
            Item { what: s("service"), dev: s("PC1"), ..Item::default() },
            Item { what: s("cable"), dev: s("PC1"), ..Item::default() },
            Item { what: s("address"), dev: s("R1"), port: s("Gi0/0/0"), cmp: Compare::Any, value: Value::string("x"), ..Item::default() },
            check(&[], 1),
            check(&["PC1 frobnicates", "PC9 reaches 10.0.0.1"], 1),
        ],
        target: capture(&p, &e),
        ..Assess::default()
    };
    let found = texts(&bad);
    let want = [
        "Kriterium 1: unbekannte Art „frob“.",
        "Kriterium 2 nennt kein Gerät.",
        "Kriterium 3: das Gerät R9 gibt es im Projekt nicht.",
        "Kriterium 4 nennt keine Schnittstelle.",
        "Kriterium 5: „4095“ ist kein gültiger Wert.",
        "Kriterium 6 nennt keinen Dienst.",
        "Kriterium 6 erwartet keinen Wert.",
        "Kriterium 7 nennt kein zweites Gerät.",
        "Kriterium 8: „any“ braucht eine Liste von Werten.",
        "Kriterium 9 hat keine Prüfung.",
    ];
    for w in want {
        t.check(&format!("issue: {}", w), found.iter().any(|f| f == w));
    }
    t.check("issue: a check that does not parse", found.iter().any(|f| f.starts_with("Kriterium 10: „PC1 frobnicates“ ist keine Prüfung: ")));
    t.check("issue: a check's device", found.iter().any(|f| f == "Kriterium 10: das Gerät PC9 gibt es im Projekt nicht."));
    let good = Assess { items: vec![item("address", "R1", "Gi0/0/0", "10.0.1.1/24"), check(&["PC1 reaches 10.0.2.10"], 2)], target: capture(&p, &e), ..Assess::default() };
    t.eq("issues: none", texts(&good), Vec::<String>::new());
    // a lab may grade with its items alone
    let lab = Lab { title: Text::new("T", ""), assess: Some(good.clone()), ..Lab::default() };
    t.eq("lab issues with an assessment and no goals", lab.issues(&p).len(), 0);
    let none = Lab { title: Text::new("T", ""), ..Lab::default() };
    t.eq("lab issues without either", none.issues(&p).iter().map(|m| m.text()).collect::<Vec<_>>(), vec![s("Das Labor hat keine Ziele.")]);
    for w in want {
        let tpl = bad.issues(&p).into_iter().find(|m| m.text() == w).map(|m| m.tpl).unwrap_or("");
        t.check(&format!("issue in English: {}", tpl), english(tpl).is_some_and(|en| !en.is_empty()));
    }
}

// ---------------------------------------------------------------- canonical values

fn canon_t(t: &mut T) {
    let c = |k: ItemKind, x: &str| canon(k, x);
    t.eq("address /24", c(ItemKind::Address, "10.0.1.1/24"), Some(s("10.0.1.1/24")));
    t.eq("address mask", c(ItemKind::Address, "10.0.1.1 255.255.255.0"), Some(s("10.0.1.1/24")));
    t.eq("address IOS", c(ItemKind::Address, " ip address 10.0.1.1  255.255.255.0 "), Some(s("10.0.1.1/24")));
    t.eq("address Linux", c(ItemKind::Address, "ip addr add 10.0.1.1/24 dev eth0"), Some(s("10.0.1.1/24")));
    t.eq("address IPv6", c(ItemKind::Address, "2001:DB8:0:1::1/64"), Some(s("2001:db8:0:1::1/64")));
    t.eq("address without length", c(ItemKind::Address, "10.0.1.1"), None);
    t.eq("address bad mask", c(ItemKind::Address, "10.0.1.1 255.0.255.0"), None);
    t.eq("state no shutdown", c(ItemKind::State, "no shutdown"), Some(s("up")));
    t.eq("state aus", c(ItemKind::State, "Aus"), Some(s("down")));
    t.eq("state nonsense", c(ItemKind::State, "maybe"), None);
    t.eq("vlan words", c(ItemKind::Vlan, "VLAN 20"), Some(s("20")));
    t.eq("vlan IOS", c(ItemKind::Vlan, "switchport access vlan 0020"), Some(s("20")));
    t.eq("vlan too big", c(ItemKind::Vlan, "4095"), None);
    t.eq("mode IOS", c(ItemKind::Mode, "switchport mode trunk"), Some(s("trunk")));
    t.eq("mode routed", c(ItemKind::Mode, "no switchport"), Some(s("routed")));
    t.eq("mode dynamic auto", c(ItemKind::Mode, "dynamic auto"), Some(s("dynamic")));
    t.eq("hostname", c(ItemKind::Hostname, "hostname Core-1"), Some(s("Core-1")));
    t.eq("hostname bad", c(ItemKind::Hostname, "1abc"), None);
    let r = |x: &str| RouteVal::parse(x).map(|v| format!("{}", v));
    t.eq("route canonical", r("10.0.2.0/24 via 10.0.1.2"), Some(s("10.0.2.0/24 via 10.0.1.2")));
    t.eq("route IOS", r("ip route 10.0.2.0 255.255.255.0 10.0.1.2"), Some(s("10.0.2.0/24 via 10.0.1.2")));
    t.eq("route IOS with interface and distance", r("ip route 10.0.2.0 255.255.255.0 GigabitEthernet0/0/1 10.0.1.2 5"), Some(s("10.0.2.0/24 via 10.0.1.2 dev Gi0/0/1 distance 5")));
    t.eq("route Linux default", r("ip route add default via 10.0.1.1 dev eth0"), Some(s("default via 10.0.1.1 dev eth0")));
    t.eq("route IOS default", r("ip route 0.0.0.0 0.0.0.0 10.0.1.1"), Some(s("default via 10.0.1.1")));
    t.eq("route host bits dropped", r("10.0.2.7/24 via 10.0.1.2"), Some(s("10.0.2.0/24 via 10.0.1.2")));
    t.eq("route without a next hop", r("10.0.2.0/24"), None);
    t.eq("route canon", c(ItemKind::Route, "ip route 10.0.2.0 255.255.255.0 10.0.1.2"), Some(s("10.0.2.0/24 via 10.0.1.2")));
}

// ---------------------------------------------------------------- reaching an address

fn reach_t(t: &mut T) {
    t.eq("reach parsed", scenario::parse_expectation("PC1 reaches 10.0.2.10"), Ok(Expect::Reach { dev: s("PC1"), host: s("10.0.2.10"), negate: false }));
    t.eq("not reach parsed", scenario::parse_expectation("expect PC1 not reaches server.lab"), Ok(Expect::Reach { dev: s("PC1"), host: s("server.lab"), negate: true }));
    t.eq("reach dev", scenario::parse_expectation("PC1 reaches 10.0.2.10").ok().and_then(|x| x.dev().map(String::from)), Some(s("PC1")));
    let Some((p, e)) = running(LAN_ROUTER, 1) else { return };
    let none = |_: u32| String::new();
    let x = |c: &str| scenario::parse_expectation(c).unwrap_or(Expect::Hops { proto: s("x"), cmp: scenario::Cmp::Eq, n: 0 });
    let (now, events, hops) = (e.now(), e.events_done(), e.trace.hops().count());
    let v = judge(&p, &e, &x("PC1 reaches 10.0.2.10"), &none);
    t.check(&format!("reached across the router ({})", v.report()), v.held && matches!(v.found, Found::Ping { received, .. } if received >= 1));
    t.eq("the engine untouched", (e.now(), e.events_done(), e.trace.hops().count()), (now, events, hops));
    t.eq("judged twice the same", judge(&p, &e, &x("PC1 reaches 10.0.2.10"), &none), v.clone());
    t.check("the report line", v.report().starts_with("expect PC1 reaches 10.0.2.10 (") && v.report().ends_with(" beantwortet)"));
    let w = judge(&p, &e, &x("PC1 reaches 10.0.9.9"), &none);
    t.check(&format!("no route on the router ({})", w.report()), !w.held && w.reason().text() == "keine Antwort: 10.0.1.1 meldet: Netz nicht erreichbar");
    t.check("the reason in English", english(w.reason().tpl).is_some());
    let n = judge(&p, &e, &x("PC1 not reaches 10.0.9.9"), &none);
    t.check("not reached holds", n.held);
    let nn = judge(&p, &e, &x("PC1 not reaches 10.0.2.10"), &none);
    t.check("negated and reached fails", !nn.held && nn.reason().text().contains("Pings beantwortet"));
    let name = judge(&p, &e, &x("PC1 reaches server.lab"), &none);
    t.check(&format!("an unknown name ({})", name.report()), !name.held && name.reason().text().contains("server.lab"));
    let no = judge(&p, &e, &x("PC9 reaches 10.0.2.10"), &none);
    t.check("no such device", !no.held && no.found == Found::NoDevice);
    // the router off: the probes time out; PC1 off: none leaves
    let mut off = p.clone();
    if let Some(d) = off.devices.iter_mut().find(|d| d.name == "R1") {
        d.off = true;
    }
    if let Ok(e2) = Engine::from_project(&off) {
        // the cable to the router is down: PC1 has no route left
        let v = judge(&off, &e2, &x("PC1 reaches 10.0.2.10"), &none);
        t.check(&format!("the router off ({})", v.report()), !v.held && v.reason().text() == "keine Antwort: keine Route zum Ziel");
    }
    if let Some(d) = off.devices.iter_mut().find(|d| d.name == "PC1") {
        d.off = true;
    }
    if let Ok(e3) = Engine::from_project(&off) {
        let v = judge(&off, &e3, &x("PC1 reaches 10.0.2.10"), &none);
        t.eq("the device off", v.reason().text(), s("keine Antwort: das Gerät ist ausgeschaltet"));
    }
    // the runner writes the same line
    if let Ok(sc) = scenario::parse("run 1s\nexpect PC1 reaches 10.0.2.10\nexpect PC1 not reaches 10.0.9.9\n") {
        let out = jfn_sim::run_scenario(&p, &sc, "reach", false);
        t.check(&format!("the runner's lines: {}", out.report), out.report.lines().any(|l| l.contains(" ok ") && l.ends_with("expect PC1 reaches 10.0.2.10 (4 von 5 beantwortet)")) && out.report.lines().any(|l| l.contains(" ok ") && l.ends_with("expect PC1 not reaches 10.0.9.9 (keine Antwort: 10.0.1.1 meldet: Netz nicht erreichbar)")));
    }
}

// ---------------------------------------------------------------- the kinds

fn kinds_t(t: &mut T) {
    let Some((p, mut e)) = running(LAN_ROUTER, 1) else { return };
    // hostname: any case (Exact: the same text)
    t.eq("hostname", held(vec![item("hostname", "R1", "", "r1"), Item { cmp: Compare::Exact, ..item("hostname", "R1", "", "r1") }, Item { cmp: Compare::Exact, ..item("hostname", "R1", "", "R1") }], &p, &e), vec![true, false, true]);
    // address: masks, prefixes, interface spellings, Exact, Any
    let addr = |port: &str, v: &str| item("address", "R1", port, v);
    t.eq(
        "address spellings",
        held(vec![addr("Gi0/0/0", "10.0.1.1/24"), addr("g0/0/0", "10.0.1.1 255.255.255.0"), addr("GigabitEthernet0/0/0", "ip address 10.0.1.1 255.255.255.0"), addr("Gi0/0/0", "10.0.1.2/24"), addr("Gi0/0/1", "10.0.1.1/24")], &p, &e),
        vec![true, true, true, false, false],
    );
    t.eq("address exact", held(vec![Item { cmp: Compare::Exact, ..addr("Gi0/0/0", "10.0.1.1/24") }, Item { cmp: Compare::Exact, ..addr("Gi0/0/0", "10.0.1.1 255.255.255.0") }], &p, &e), vec![true, false]);
    let any = Item { cmp: Compare::Any, value: Value::Array(vec![Value::string("10.9.9.9/24"), Value::string("10.0.1.1 255.255.255.0")]), ..addr("Gi0/0/0", "") };
    t.eq("address any", held(vec![any], &p, &e), vec![true]);
    let g = grade(vec![addr("Gi0/0/0", "10.0.1.2/24"), addr("Gi0/0/9", "10.0.1.2/24"), item("address", "PC9", "eth0", "10.0.1.2/24")], &p, &e);
    let found: Vec<String> = g.items.iter().map(|x| x.found.text()).collect();
    t.eq("address found", found, vec![s("hat 10.0.1.1/24"), s("kein Interface Gi0/0/9"), s("das Gerät PC9 gibt es nicht")]);
    t.eq("Linux address", held(vec![item("address", "PC1", "eth0", "10.0.1.10/24"), item("address", "PC1", "ETH0", "10.0.1.10 255.255.255.0")], &p, &e), vec![true, true]);
    // state: "no shutdown" spelled any way
    t.eq("state", held(vec![item("state", "R1", "Gi0/0/0", "up"), item("state", "R1", "gi0/0/0", "no shutdown"), item("state", "R1", "Gi0/0/0", "down"), item("state", "R1", "Gi0/0/2", "down")], &p, &e), vec![true, true, false, true]);
    t.eq("state found", grade(vec![item("state", "R1", "Gi0/0/0", "down")], &p, &e).items[0].found.text(), s("ist eingeschaltet"));
    // static routes: Linux and IOS, every spelling
    let r1 = p.find("R1").map_or(0, |d| d.id);
    cli(&mut e, r1, &["enable", "configure terminal", "ip route 10.0.9.0 255.255.255.0 10.0.2.10", "ip route 10.0.8.0 255.255.255.0 GigabitEthernet0/0/1 10.0.2.10 7", "end"]);
    let route = |dev: &str, v: &str| item("route", dev, "", v);
    t.eq(
        "routes",
        held(
            vec![
                route("PC1", "default via 10.0.1.1"),
                route("PC1", "ip route add default via 10.0.1.1"),
                route("PC1", "ip route 0.0.0.0 0.0.0.0 10.0.1.1"),
                route("PC1", "default via 10.0.1.9"),
                route("R1", "10.0.9.0/24 via 10.0.2.10"),
                route("R1", "ip route 10.0.9.0 255.255.255.0 10.0.2.10"),
                route("R1", "10.0.8.0/24 via 10.0.2.10 dev g0/0/1 distance 7"),
                route("R1", "10.0.8.0/24 via 10.0.2.10 distance 1"),
                route("R1", "10.0.7.0/24 via 10.0.2.10"),
            ],
            &p,
            &e,
        ),
        vec![true, true, true, false, true, true, true, false, false],
    );
    let g = grade(vec![route("PC1", "default via 10.0.1.9"), route("R1", "10.0.7.0/24 via 10.0.2.10")], &p, &e);
    t.eq("route found", g.items.iter().map(|x| x.found.text()).collect::<Vec<_>>(), vec![s("statt dessen: default via 10.0.1.1"), s("keine statische Route nach 10.0.7.0/24")]);
    t.eq("route exact", held(vec![Item { cmp: Compare::Exact, ..route("R1", "10.0.9.0/24 via 10.0.2.10") }, Item { cmp: Compare::Exact, ..route("R1", "ip route 10.0.9.0 255.255.255.0 10.0.2.10") }], &p, &e), vec![true, false]);
    // cables: either direction, any port, the kind
    let cable = |a: &str, ap: &str, b: &str, bp: &str| Item { what: s("cable"), dev: s(a), port: s(ap), peer: s(b), peer_port: s(bp), points: 1, ..Item::default() };
    t.eq(
        "cables",
        held(vec![cable("PC1", "eth0", "R1", "Gi0/0/0"), cable("R1", "gi0/0/0", "PC1", "eth0"), cable("PC1", "", "R1", ""), cable("PC2", "", "R1", "Gi0/0/0"), cable("PC1", "eth0", "PC2", "eth0")], &p, &e),
        vec![true, true, true, false, false],
    );
    let kind = |k: &str| Item { value: Value::string(k), ..cable("PC1", "eth0", "R1", "Gi0/0/0") };
    t.eq("cable kinds", held(vec![kind("copper-cross"), kind("fiber-mm")], &p, &e), vec![true, false]);
    let g = grade(vec![cable("PC1", "eth0", "PC2", "eth0"), kind("fiber-mm")], &p, &e);
    t.eq("cable found", g.items.iter().map(|x| x.found.text()).collect::<Vec<_>>(), vec![s("PC1 eth0 hängt an R1 Gi0/0/0"), s("falsches Kabel: Kupfer, gekreuzt")]);
    // a service's setting: JSON in any order, more members, lists of records in any order
    let mut q = p.clone();
    let cfg = agent_core::json::parse(b"{\"port\": 80, \"backends\": [{\"name\": \"a\", \"addr\": \"10.0.2.10\", \"port\": 80}, {\"name\": \"b\", \"addr\": \"10.0.2.11\"}], \"health\": {\"type\": \"http\", \"interval_ms\": 1000}}").unwrap_or(Value::Null);
    if let Some(d) = q.devices.iter_mut().find(|d| d.name == "PC2") {
        d.services.push(ServiceDef { kind: s("lb"), name: s("lb"), disabled: false, config: cfg });
    }
    let svc = |key: &str, json: &[u8]| Item { what: s("service"), dev: s("PC2"), svc: s("lb"), key: s(key), value: agent_core::json::parse(json).unwrap_or(Value::Null), points: 1, ..Item::default() };
    t.eq(
        "service settings",
        held(
            vec![
                svc("port", b"80"),
                svc("port", b"\"80\""),
                svc("backends", b"[{\"addr\": \"10.0.2.11\", \"name\": \"b\"}, {\"name\": \"a\", \"addr\": \"10.0.2.10\"}]"),
                svc("backends", b"[{\"name\": \"a\", \"addr\": \"10.0.2.10\"}]"),
                svc("health.interval_ms", b"1000"),
                svc("backends.1.addr", b"\"10.0.2.11\""),
                svc("health", b"{\"type\": \"tcp\"}"),
                svc("nosuch", b"1"),
            ],
            &q,
            &e,
        ),
        vec![true, true, true, false, true, true, false, false],
    );
    t.eq("service exact", held(vec![Item { cmp: Compare::Exact, ..svc("port", b"80") }, Item { cmp: Compare::Exact, ..svc("health", b"{\"interval_ms\": 1000, \"type\": \"http\"}") }], &q, &e), vec![true, false]);
    let g = grade(vec![svc("health", b"{\"type\": \"tcp\"}"), svc("nosuch", b"1"), Item { svc: s("web"), ..svc("port", b"80") }], &q, &e);
    t.eq("service found", g.items.iter().map(|x| x.found.text()).collect::<Vec<_>>(), vec![s("ist {\"type\":\"http\",\"interval_ms\":1000}"), s("keine Einstellung nosuch"), s("kein Dienst web")]);
    // VLANs and modes on a switch
    let Some((sp, se)) = running(STICK, 1) else { return };
    let sw = |what: &str, port: &str, v: &str| item(what, "SW1", port, v);
    t.eq(
        "vlans and modes",
        held(
            vec![
                sw("vlan", "Gi1/0/1", "10"),
                sw("vlan", "gi1/0/2", "VLAN 20"),
                sw("vlan", "Gi1/0/2", "10"),
                sw("vlan", "Gi1/0/3", "1"),
                sw("mode", "Gi1/0/1", "access"),
                sw("mode", "Gi1/0/24", "switchport mode trunk"),
                sw("mode", "Gi1/0/3", "dynamic"),
                sw("mode", "Gi1/0/24", "access"),
                item("mode", "R1", "Gi0/0/0", "routed"),
                item("vlan", "R1", "Gi0/0/0", "1"),
            ],
            &sp,
            &se,
        ),
        vec![true, true, false, true, true, true, true, false, true, false],
    );
    let g = grade(vec![sw("vlan", "Gi1/0/2", "10"), sw("mode", "Gi1/0/24", "access"), item("vlan", "R1", "Gi0/0/0", "1")], &sp, &se);
    t.eq("vlan and mode found", g.items.iter().map(|x| x.found.text()).collect::<Vec<_>>(), vec![s("im VLAN 20"), s("im Modus trunk"), s("Gi0/0/0 ist kein Switchport")]);
    // a check of several expectations: all of them at once
    t.eq("checks", held(vec![check(&["PC1 reaches 10.0.2.10", "R1 route 10.0.2.0/24 via connected"], 2), check(&["PC1 reaches 10.0.2.10", "R1 route 10.0.3.0/24"], 2), check(&["PC1 frobnicates"], 2)], &p, &e), vec![true, false, false]);
    let g = grade(vec![check(&["PC1 reaches 10.0.2.10", "R1 route 10.0.3.0/24"], 2), check(&["PC1 frobnicates"], 2)], &p, &e);
    t.check("check found", g.items[0].found.text() == "R1 route 10.0.3.0/24: keine Route" && g.items[0].verdicts.len() == 2 && g.items[1].found.text().starts_with("Prüfung 1 ist fehlerhaft: "));
    // every reason has its English
    let mut tpls = Vec::new();
    for x in [&g] {
        tpls.extend(x.items.iter().map(|i| i.found.tpl));
    }
    for tpl in ["heißt {}", "hat {}", "hat keine Adresse", "kein Interface {}", "das Gerät {} gibt es nicht", "ist eingeschaltet", "ist ausgeschaltet (shutdown)", "{} ist kein Switchport", "im VLAN {}", "im Modus {}", "keine statische Route nach {}", "statt dessen: {}", "kein Dienst {}", "keine Einstellung {}", "ist {}", "nicht verbunden", "{} {} hängt an {} {}", "falsches Kabel: {}", "es fehlt: {}", "zu viel: {}", "Prüfung {} ist fehlerhaft: {}", "unbekannte Art {}"] {
        t.check(&format!("reason in English: {}", tpl), english(tpl).is_some_and(|x| !x.is_empty()));
    }
    // the default texts of the kinds, and their English
    let texts = [
        (item("hostname", "R1", "", "R1"), "R1 heißt R1"),
        (addr("GigabitEthernet0/0/0", "10.0.1.1 255.255.255.0"), "R1 Gi0/0/0: Adresse 10.0.1.1/24"),
        (item("state", "R1", "Gi0/0/0", "no shutdown"), "R1 Gi0/0/0 ist eingeschaltet"),
        (item("state", "R1", "Gi0/0/0", "shutdown"), "R1 Gi0/0/0 ist ausgeschaltet"),
        (sw("vlan", "GigabitEthernet1/0/1", "10"), "SW1 Gi1/0/1 im VLAN 10"),
        (sw("mode", "Gi1/0/24", "trunk"), "SW1 Gi1/0/24 im Modus trunk"),
        (route("R1", "ip route 10.0.9.0 255.255.255.0 10.0.2.10"), "R1: statische Route 10.0.9.0/24 via 10.0.2.10"),
        (svc("port", b"80"), "PC2: lb port = 80"),
        (cable("PC1", "eth0", "R1", "GigabitEthernet0/0/0"), "Kabel PC1 eth0 – R1 Gi0/0/0"),
        (cable("PC1", "", "R1", ""), "Kabel zwischen PC1 und R1"),
        (config("R1", &["interface GigabitEthernet0/0/1", " no shutdown"]), "R1: interface GigabitEthernet0/0/1 · no shutdown"),
        (check(&["PC1 reaches 10.0.2.10"], 1), "PC1 erreicht 10.0.2.10"),
        (check(&["PC1 not reaches 10.0.2.10"], 1), "PC1 erreicht 10.0.2.10 nicht"),
        (check(&["PC1 route 0.0.0.0/0", "hops icmp >= 1"], 1), "PC1 route 0.0.0.0/0; hops icmp >= 1"),
    ];
    for (it, want) in texts {
        let m = item_text(&it);
        t.eq(&format!("text of {}", it.what), m.text(), s(want));
        if m.tpl.chars().any(char::is_alphabetic) {
            t.check(&format!("text in English: {}", m.tpl), english(m.tpl).is_some_and(|x| !x.is_empty()));
        }
    }
}

// ---------------------------------------------------------------- configuration lines

fn config_t(t: &mut T) {
    let Some((p, mut e)) = running(LAN_ROUTER, 1) else { return };
    let r1 = p.find("R1").map_or(0, |d| d.id);
    let model = jfn_model::catalog::model("jf-r4");
    // a configuration's lines with their blocks
    let pairs = config_pairs("!\nhostname R1\n!\ninterface Gi0/0/0\n ip address  10.0.1.1 255.255.255.0\n no shutdown\n!\nip route 0.0.0.0 0.0.0.0 10.0.1.2\nend\n");
    t.eq(
        "pairs",
        pairs,
        vec![(s(""), s("hostname R1")), (s(""), s("interface Gi0/0/0")), (s("interface Gi0/0/0"), s("ip address 10.0.1.1 255.255.255.0")), (s("interface Gi0/0/0"), s("no shutdown")), (s(""), s("ip route 0.0.0.0 0.0.0.0 10.0.1.2"))],
    );
    // normalised on a scratch device: abbreviations, interface names, what "no shutdown" takes away
    if let Some(m) = model {
        let n = normalise(m, &[s("int g0/0/1"), s(" ip add 10.0.5.1 255.255.255.0"), s(" no shut")]);
        t.eq("normalised: there", n.have, vec![(s("interface GigabitEthernet0/0/1"), s("ip address 10.0.5.1 255.255.255.0"))]);
        t.check("normalised: gone", n.lack.contains(&(s("interface GigabitEthernet0/0/1"), s("shutdown"))) && n.lack.contains(&(s("interface GigabitEthernet0/0/1"), s("no ip address"))));
        let k = normalise(m, &[s("interface GigabitEthernet0/0/2"), s(" shutdown"), s(" frobnicate")]);
        t.check(&format!("normalised: a default makes nothing, a refused line as written ({:?})", k), k.lack.is_empty() && k.have == vec![(s("interface GigabitEthernet0/0/2"), s("frobnicate"))]);
        // typed at a copy of R1: what would change
        let run = e.running_config(r1);
        let c = would_change(m, &run, &[s("int g0/0/1"), s(" shut")]);
        t.check(&format!("would change: shut down ({:?})", c), c.as_ref().is_some_and(|c| c.add == vec![(s("interface GigabitEthernet0/0/1"), s("shutdown"))] && c.gone.is_empty() && c.refused.is_empty()));
        let c = would_change(m, &run, &[s("interface GigabitEthernet0/0/0"), s(" ip address 10.0.1.1 255.255.255.0"), s(" no shutdown")]);
        t.check("would change: nothing", c.as_ref().is_some_and(|c| c.none()));
        let c = would_change(m, &run, &[s("interface Gi0/0/0"), s(" no ip address")]);
        t.check(&format!("would change: the address goes ({:?})", c), c.as_ref().is_some_and(|c| c.gone == vec![(s("interface GigabitEthernet0/0/0"), s("ip address 10.0.1.1 255.255.255.0"))]));
        let c = would_change(m, &run, &[s("interface Gi0/0/0"), s(" frobnicate")]);
        t.check("would change: a refused line", c.as_ref().is_some_and(|c| c.none() && c.refused == vec![(s("interface Gi0/0/0"), s("frobnicate"))]));
        let o = normalise(m, &[s("enable"), s("conf t"), s("router ospf 1"), s(" network 10.0.0.0 0.0.255.255 area 0"), s("end")]);
        t.check(&format!("normalised: a new block, console words dropped ({:?})", o), o.have.contains(&(s(""), s("router ospf 1"))) && o.have.iter().any(|x| x.0 == "router ospf 1" && x.1.starts_with("network 10.0.0.0 0.0.255.255 area 0")) && !o.have.iter().any(|x| x.1 == "enable"));
    }
    let lines = |v: &[&str]| config("R1", v);
    let h = held(
        vec![
            lines(&["interface GigabitEthernet0/0/0", " ip address 10.0.1.1 255.255.255.0"]),
            lines(&["int g0/0/0", " ip add 10.0.1.1 255.255.255.0", " no shut"]),
            lines(&["interface Gi0/0/1", " shutdown"]),
            lines(&["interface Gi0/0/2", " shutdown"]),
            lines(&["ip route 10.0.9.0 255.255.255.0 10.0.2.10"]),
            Item { cmp: Compare::Exact, ..lines(&["interface GigabitEthernet0/0/0", " ip address 10.0.1.1 255.255.255.0"]) },
            Item { cmp: Compare::Exact, ..lines(&["int g0/0/0", " ip add 10.0.1.1 255.255.255.0"]) },
        ],
        &p,
        &e,
    );
    t.eq("config lines", h, vec![true, true, false, true, false, true, false]);
    let g = grade(vec![lines(&["interface Gi0/0/1", " shutdown"]), lines(&["ip route 10.0.9.0 255.255.255.0 10.0.2.10"]), lines(&["interface Gi0/0/2", " no shutdown"]), lines(&["interface Gi0/0/2", " frobnicate"])], &p, &e);
    t.eq(
        "config found",
        g.items.iter().map(|x| x.found.text()).collect::<Vec<_>>(),
        vec![s("es fehlt: interface GigabitEthernet0/0/1 › shutdown"), s("es fehlt: ip route 10.0.9.0 255.255.255.0 10.0.2.10"), s("zu viel: interface GigabitEthernet0/0/2 › shutdown"), s("es fehlt: interface Gi0/0/2 › frobnicate")],
    );
    // typed by the learner abbreviated, the running configuration has it
    cli(&mut e, r1, &["en", "conf t", "ip route 10.0.9.0 255.255.255.0 10.0.2.10", "int g0/0/2", "no shut", "end"]);
    t.eq("config after the console", held(vec![lines(&["ip route 10.0.9.0 255.255.255.0 10.0.2.10"]), lines(&["interface Gi0/0/2", " no shutdown"])], &p, &e), vec![true, true]);
    // any of several blocks
    let any = Item { cmp: Compare::Any, value: Value::Array(vec![Value::Array(vec![Value::string("ip route 0.0.0.0 0.0.0.0 10.0.2.10")]), Value::Array(vec![Value::string("ip route 10.0.9.0 255.255.255.0 10.0.2.10")])]), ..lines(&[]) };
    t.eq("config any", held(vec![any], &p, &e), vec![true]);
}

// ---------------------------------------------------------------- points

fn points_t(t: &mut T) {
    let Some((p, e)) = running(LAN_ROUTER, 1) else { return };
    let it = |v: &str, pts: u32| Item { points: pts, ..item("address", "R1", "Gi0/0/0", v) };
    let sc = grade(vec![it("10.0.1.1/24", 2), it("10.0.1.9/24", 3)], &p, &e);
    t.eq("points", (sc.points, sc.total, sc.percent(), sc.held()), (2, 5, 40, 1));
    t.eq("points per item", sc.items.iter().map(|g| (g.points, g.max)).collect::<Vec<_>>(), vec![(2, 2), (0, 3)]);
    t.check("found only when wrong", sc.items[0].found.is_empty() && !sc.items[1].found.is_empty());
    let sc = grade(vec![it("10.0.1.1/24", 999), it("10.0.1.9/24", 1)], &p, &e);
    t.eq("rounded down", sc.percent(), 99);
    t.eq("nothing to get", grade(Vec::new(), &p, &e).percent(), 0);
    t.eq("all", grade(vec![it("10.0.1.1/24", 3)], &p, &e).percent(), 100);
    // a renamed device: found through its id; checks name it as the target does
    let r1 = p.find("R1").map_or(0, |d| d.id);
    let target = capture(&p, &e);
    let mut q = p.clone();
    let _ = q.rename(r1, "Core");
    let mut e2 = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let _ = e2.apply(Action::Hostname { dev: r1, name: s("Core") });
    e2.run_until(SEC, BUDGET);
    let a = Assess {
        items: vec![Item { id: r1, ..item("hostname", "R1", "", "R1") }, Item { id: r1, ..item("address", "R1", "Gi0/0/0", "10.0.1.1/24") }, check(&["R1 route 10.0.2.0/24 via connected"], 1)],
        target,
        ..Assess::default()
    };
    let sc = Grader::new(&a).grade(&q, &e2, &|_| String::new());
    t.eq("renamed: by id", sc.items.iter().map(|g| g.held).collect::<Vec<_>>(), vec![false, true, true]);
    t.eq("renamed: what it is called", sc.items[0].found.text(), s("heißt Core"));
}

// ---------------------------------------------------------------- the panel's run

fn run_t(t: &mut T) {
    let Some((p, e)) = running(LAN_ROUTER, 1) else { return };
    let none = |_: u32| String::new();
    let items = vec![item("address", "R1", "Gi0/0/0", "10.0.1.1/24"), item("address", "R1", "Gi0/0/0", "10.0.1.9/24")];
    let a = |feedback: Feedback, limit: u32| Assess { feedback, time_limit_s: limit, items: items.clone(), ..Assess::default() };
    // feedback: what shows before and after the end
    let r = Run::new(&a(Feedback::Items, 0), 0);
    t.eq("items: score and items", (r.shows_score(), r.shows_items()), (true, true));
    let r = Run::new(&a(Feedback::Score, 0), 0);
    t.eq("score: the score only", (r.shows_score(), r.shows_items()), (true, false));
    let mut r = Run::new(&a(Feedback::End, 0), 0);
    t.eq("end: nothing", (r.shows_score(), r.shows_items()), (false, false));
    t.check("end: graded at first only (forced)", r.due(1, 1, 0));
    r.grade(&p, &e, &none, 1, 1, 0);
    t.check("end: no grade while hidden", !r.due(2, 2, 10 * EVERY_US) && r.wake(2, 2, 0).is_none());
    r.finish(&p, &e, &none, 2, 2, 5_000_000, false);
    t.check("end: handed in, everything shows", r.shows_score() && r.shows_items() && r.end.as_ref().is_some_and(|x| !x.timeout && x.at_us == 5_000_000) && r.explain);
    t.check("ended: no more grades", !r.due(3, 3, 100 * EVERY_US) && r.wake(3, 3, 0).is_none());
    t.eq("ended: the score", r.score.as_ref().map(|s| (s.points, s.total)), Some((1, 2)));
    // when a grade is due: forced first, then after a change: of the project after EDIT_US, of
    // the simulator after EVERY_US; the timer brings the window back for it
    let mut r = Run::new(&a(Feedback::Items, 0), 0);
    t.check("due first", r.due(1, 1, 0));
    r.grade(&p, &e, &none, 1, 1, 0);
    t.check("nothing changed: never due, no timer", !r.due(1, 1, 100 * EVERY_US) && r.wake(1, 1, 5).is_none());
    t.check("an edit waits", !r.due(2, 1, EDIT_US - 1) && r.due(2, 1, EDIT_US));
    t.eq("an edit: the timer", r.wake(2, 1, 100_000), Some(400));
    t.check("the simulator waits", !r.due(1, 2, EVERY_US - 1) && r.due(1, 2, EVERY_US));
    t.eq("the simulator: the timer", (r.wake(1, 2, 0), r.wake(1, 2, 10 * EVERY_US)), (Some(2000), Some(16)));
    r.force = true;
    t.check("Prüfen: due at once", r.due(1, 1, 1));
    let r = Run::new(&a(Feedback::Score, 600), 0);
    t.eq("the countdown: a timer each second", r.wake(0, 0, 1), Some(1000));
    // the time limit: on the task clock, from the start, ends with the final score at the limit
    let mut r = Run::new(&a(Feedback::Score, 600), 1_000_000);
    t.eq("no time used", r.remaining(1_000_000), Some(600_000_000));
    t.eq("time left", r.remaining(61_000_000), Some(540_000_000));
    t.check("not up yet", !r.time_up(600_999_999));
    t.check("up", r.time_up(601_000_000));
    r.finish(&p, &e, &none, 1, 1, 605_000_000, true);
    t.check("out of time: at the limit", r.end.as_ref().is_some_and(|x| x.timeout && x.at_us == 601_000_000));
    t.eq("out of time: nothing left", r.remaining(700_000_000), Some(0));
    t.check("out of time: not up again", !r.time_up(700_000_000));
    t.check("out of time: items show", r.shows_items());
    let mut r = Run::new(&a(Feedback::Items, 300), 0);
    r.finish(&p, &e, &none, 1, 1, 100_000_000, false);
    t.eq("handed in early: the time stands", (r.remaining(200_000_000), r.time_up(400_000_000)), (Some(200_000_000), false));
    t.eq("no limit", Run::new(&a(Feedback::Items, 0), 0).remaining(5), None);
    t.eq("clock", (clock_text(0), clock_text(1), clock_text(65_000_000), clock_text(3_723_000_000)), (s("0:00"), s("0:01"), s("1:05"), s("1:02:03")));
}

// ---------------------------------------------------------------- authoring

fn author_t(t: &mut T) {
    let Some((p, e)) = running(LAN_ROUTER, 1) else { return };
    // the network as it runs
    let target = capture(&p, &e);
    t.check("capture: running configurations", target.devices.iter().all(|d| d.startup_config == e.running_config(d.id)) && target.links == p.links);
    let tp = target.project(&p);
    t.check("target project", tp.devices.len() == 3 && tp.links.len() == 2 && tp.seed == p.seed && tp.lab.is_none());
    // the tree: groups and candidate items, keys unique
    let cands = candidates(&tp, &e);
    let mut keys: Vec<u64> = cands.iter().map(|c| c.key).collect();
    keys.sort();
    keys.dedup();
    t.eq("candidates: unique keys", keys.len(), cands.len());
    let label = |c: &jfn_sim::assess::Cand| c.label.text();
    t.check("candidates: a device", cands.iter().any(|c| c.depth == 0 && c.item.is_none() && label(c) == "R1"));
    t.check("candidates: an address", cands.iter().any(|c| c.depth == 2 && label(c) == "Adresse 10.0.1.1/24" && c.item.as_ref().is_some_and(|i| i.what == "address" && i.port == "GigabitEthernet0/0/0" && i.id == 2)));
    t.check("candidates: a route", cands.iter().any(|c| label(c) == "default via 10.0.1.1" && c.item.as_ref().is_some_and(|i| i.what == "route" && i.dev == "PC1")));
    t.check("candidates: a block", cands.iter().any(|c| label(c) == "interface GigabitEthernet0/0/1" && c.item.as_ref().is_some_and(|i| i.what == "config")));
    t.check("candidates: cables", cands.iter().any(|c| c.depth == 0 && label(c) == "Kabel") && cands.iter().any(|c| label(c) == "PC1 eth0 – R1 Gi0/0/0"));
    t.check("candidates: their keys", cands.iter().filter_map(|c| c.item.as_ref().map(|i| (c.key, i))).all(|(k, i)| item_key(i) == k));
    for c in cands.iter().filter(|c| c.item.is_none()) {
        if c.label.tpl != "{}" {
            t.check(&format!("group in English: {}", c.label.tpl), english(c.label.tpl).is_some());
        }
    }
    // every candidate holds in the target
    let all: Vec<Item> = cands.iter().filter_map(|c| c.item.clone()).collect();
    let sc = grade(all.clone(), &tp, &e);
    t.check(&format!("candidates hold in the target ({:?})", sc.items.iter().zip(&all).filter(|(g, _)| !g.held).map(|(g, i)| format!("{} {}: {}", i.what, i.port, g.found.text())).collect::<Vec<_>>()), sc.held() == all.len());
    // the start network: what an item checks is gone there (one item at a time), and it holds
    // in the target
    let pick = |what: &str| all.iter().filter(|i| i.what == what).cloned().collect::<Vec<_>>();
    let mut tested = 0;
    for it in pick("address").into_iter().chain(pick("route")).chain(pick("cable")).chain(pick("hostname")).chain(pick("config").into_iter().filter(|i| jfn_model::lab::config_lines(&i.value).len() > 1)).chain(pick("state").into_iter().filter(|i| i.value.as_str() == Some("up") && i.dev == "R1")) {
        let start = start_network(&tp, &e, core::slice::from_ref(&it));
        let Ok(mut se) = Engine::from_project(&start) else {
            t.check(&format!("start of {} runs", it.what), false);
            continue;
        };
        se.run_until(SEC, BUDGET);
        let a = Assess { items: vec![it.clone()], target: target.clone(), ..Assess::default() };
        let gone = Grader::new(&a).grade(&start, &se, &|_| String::new());
        t.check(&format!("start without {} {} {} ({})", it.what, it.dev, it.port, item_text(&it).text()), !gone.items[0].held);
        tested += 1;
    }
    t.check(&format!("start network: kinds tested ({})", tested), tested >= 10);
    // the start network of a few items together: the target's devices with their ids, names
    // neutral where an item wants a name, the rest unchanged
    let few: Vec<Item> = vec![pick("hostname")[0].clone(), Item { id: 2, ..item("address", "R1", "GigabitEthernet0/0/1", "10.0.2.1/24") }, Item { id: 2, ..item("state", "R1", "GigabitEthernet0/0/1", "up") }];
    let start = start_network(&tp, &e, &few);
    t.check("start: same ids", start.devices.iter().map(|d| d.id).collect::<Vec<_>>() == tp.devices.iter().map(|d| d.id).collect::<Vec<_>>());
    t.check(&format!("start: a neutral name ({:?})", start.devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>()), start.devices[0].name != "PC1" && start.devices[0].startup_config.starts_with(&format!("hostname {}\n", start.devices[0].name)));
    t.check("start: the address gone, shut down", start.devices[1].startup_config.contains("interface GigabitEthernet0/0/1\n shutdown\n") && !start.devices[1].startup_config.contains("10.0.2.1"));
    t.check("start: the rest as it was", start.devices[1].startup_config.contains("interface GigabitEthernet0/0/0\n ip address 10.0.1.1 255.255.255.0\n") && start.devices[2] == tp.devices[2]);
    // a service's setting goes, the rest of the service stays
    let mut sp = tp.clone();
    let cfg = agent_core::json::parse(b"{\"port\": 80, \"backends\": [1, 2]}").unwrap_or(Value::Null);
    sp.devices[2].services.push(ServiceDef { kind: s("lb"), name: s("lb"), disabled: false, config: cfg });
    let svc = Item { what: s("service"), dev: s("PC2"), id: 3, svc: s("lb"), key: s("backends"), value: Value::Null, points: 1, ..Item::default() };
    let start = start_network(&sp, &e, &[svc]);
    t.eq("start: a setting gone", start.devices[2].services[0].config.get("backends").is_none() && start.devices[2].services[0].config.get("port").is_some(), true);
}

// ---------------------------------------------------------------- the labs as they were

fn old_labs_t(t: &mut T) {
    for (name, text) in [("campus", CAMPUS), ("lb", LB), ("shards", SHARDS), ("cache", CACHE)] {
        let Ok(p) = Project::from_text(text) else {
            t.check(&format!("{} loads", name), false);
            continue;
        };
        let lab = p.lab.clone().unwrap_or_default();
        t.check(&format!("{}: no assessment", name), lab.assess.is_none() && !lab.goals.is_empty());
        t.eq(&format!("{}: no issues", name), lab.issues(&p).len(), 0);
        // read again after writing: the same project, "assess": null
        let again = Project::from_text(&p.to_text());
        t.check(&format!("{}: round trip", name), again.as_ref().is_ok_and(|q| *q == p) && p.to_text().contains("\"assess\": null"));
    }
}

// ---------------------------------------------------------------- the samples of assets/assess

/// an assessment worked on as the window does it: the project, the simulator following it, a
/// console's "hostname" renaming the device in the project, the grade after each step
struct Solve {
    p: Project,
    sim: Sim,
    grader: Grader,
}

impl Solve {
    fn new(text: &str) -> Option<Solve> {
        let p = Project::from_text(text).ok()?;
        let grader = Grader::new(p.lab.as_ref()?.assess.as_ref()?);
        let sim = Sim::new(&p);
        Some(Solve { p, sim, grader })
    }
    fn id(&self, name: &str) -> u32 {
        self.p.find(name).map_or(0, |d| d.id)
    }
    // what the consoles report: a new host name renames the device (doc/console.rs)
    fn reports(&mut self) {
        for r in self.sim.take_reports() {
            if let Report::Hostname { dev, name } = r {
                if self.p.rename(dev, &name).is_ok() {
                    self.sim.sync(&self.p);
                }
            }
        }
    }
    fn secs(&mut self, n: u64) {
        for _ in 0..n {
            let _ = self.sim.send(Cmd::RunFor(SEC));
            self.reports();
        }
    }
    /// lines typed at a device's console (by id: names may change)
    fn cli(&mut self, dev: u32, lines: &[&str]) {
        for l in lines {
            let _ = self.sim.send(Cmd::Cli { dev, line: s(l) });
            self.reports();
        }
    }
    fn edit(&mut self, f: impl FnOnce(&mut Project)) {
        f(&mut self.p);
        self.sim.sync(&self.p);
    }
    fn score(&self) -> Score {
        self.grader.grade(&self.p, self.sim.engine(), &|_| String::new())
    }
    /// what the items that do not hold found
    fn why(&self) -> String {
        let sc = self.score();
        let items = self.grader.items();
        sc.items.iter().zip(items).filter(|(g, _)| !g.held).map(|(g, i)| format!("{}: {}", i.text.de, g.found.text())).collect::<Vec<_>>().join("; ")
    }
}

fn samples_t(t: &mut T) {
    let files = [("01-router-zwei-netze.jfnet", S1, Feedback::Items, 0), ("02-vlans-trunk.jfnet", S2, Feedback::Score, 1200), ("03-statische-routen.jfnet", S3, Feedback::End, 900)];
    let listed = crate::workspace::task::assessments();
    t.eq("samples listed", listed.iter().map(|l| l.path.clone()).collect::<Vec<_>>(), files.iter().map(|f| format!("app://assess/{}", f.0)).collect::<Vec<_>>());
    for (name, text, feedback, limit) in files {
        let Ok(p) = Project::from_text(text) else {
            t.check(&format!("{} loads", name), false);
            continue;
        };
        let Some(lab) = p.lab.clone() else { continue };
        let Some(a) = lab.assess.clone() else {
            t.check(&format!("{} has an assessment", name), false);
            continue;
        };
        t.eq(&format!("{}: no issues", name), lab.issues(&p).iter().map(|m| m.text()).collect::<Vec<_>>(), Vec::<String>::new());
        t.eq(&format!("{}: no problems of the project", name), p.check().len(), 0);
        t.eq(&format!("{}: no problems of the target", name), a.target.project(&p).check().len(), 0);
        t.eq(&format!("{}: feedback and time", name), (a.feedback, a.time_limit_s), (feedback, limit));
        t.eq(&format!("{}: 20 points", name), a.total(), 20);
        t.check(&format!("{}: listed with its points", name), listed.iter().any(|l| l.path.ends_with(name) && l.title == lab.title && l.points == 20 && l.goals == 0));
        let mut texts: Vec<&Text> = vec![&lab.title, &lab.summary];
        texts.extend(lab.intro.iter());
        texts.extend(lab.hints.iter());
        texts.extend(a.items.iter().map(|i| &i.text));
        t.check(&format!("{}: both languages", name), texts.iter().all(|x| !x.de.is_empty() && !x.en.is_empty() && x.de != x.en));
        t.check(&format!("{}: runs whole", name), Sim::new(&p).errors.is_empty() && Sim::new(&a.target.project(&p)).errors.is_empty());
        // unsolved: nothing; the target: everything
        let Some(mut start) = Solve::new(text) else { continue };
        start.secs(3);
        t.eq(&format!("{}: unsolved ({})", name, start.why()), start.score().points, 0);
        let mut goal = p.clone();
        goal.devices = a.target.devices.clone();
        goal.links = a.target.links.clone();
        if let Some(mut g) = Solve::new(&goal.to_text()) {
            g.secs(3);
            let sc = g.score();
            t.eq(&format!("{}: the target ({})", name, g.why()), (sc.points, sc.percent()), (20, 100));
        }
        // the window's run of it
        let r = Run::new(&a, 0);
        t.eq(&format!("{}: what shows", name), (r.shows_score(), r.shows_items(), r.remaining(0)), (feedback != Feedback::End, feedback == Feedback::Items, if limit == 0 { None } else { Some(limit as u64 * 1_000_000) }));
    }
}

// the router for two networks: R1 named and set up in its console, one interface at a time
fn solve1_t(t: &mut T) {
    let run = || -> Option<Vec<(u32, Vec<bool>)>> {
        let mut x = Solve::new(S1)?;
        let mut steps = Vec::new();
        x.secs(2);
        steps.push((x.score().points, x.score().items.iter().map(|g| g.held).collect()));
        let r = x.id("Router1");
        x.cli(r, &["enable", "configure terminal", "hostname R1", "int g0/0/0", "ip add 10.0.1.1 255.255.255.0", "no shut", "end"]);
        x.secs(2);
        steps.push((x.score().points, x.score().items.iter().map(|g| g.held).collect()));
        x.cli(r, &["conf t", "interface GigabitEthernet0/0/1", "ip address 10.0.2.1 255.255.255.0", "no shutdown", "end"]);
        x.secs(2);
        steps.push((x.score().points, x.score().items.iter().map(|g| g.held).collect()));
        Some(steps)
    };
    let Some(steps) = run() else {
        t.check("sample 1 runs", false);
        return;
    };
    t.eq("sample 1: unsolved", steps[0].0, 0);
    t.eq("sample 1: half way", steps[1].clone(), (7, vec![true, true, true, false, false, false, false]));
    t.eq("sample 1: solved", steps[2].0, 20);
    t.eq("sample 1: the same twice", run(), Some(steps));
    // the renamed router is the project's R1 now
    if let Some(mut x) = Solve::new(S1) {
        let r = x.id("Router1");
        x.cli(r, &["enable", "configure terminal", "hostname R1", "end"]);
        t.check("sample 1: renamed in the project", x.p.device(r).is_some_and(|d| d.name == "R1"));
    }
}

// VLANs and a trunk: SW1 first, then R1
fn solve2_t(t: &mut T) {
    let Some(mut x) = Solve::new(S2) else { return };
    x.secs(2);
    let sw = x.id("SW1");
    x.cli(sw, &["enable", "conf t", "interface Gi1/0/1", "switchport mode access", "switchport access vlan 10", "interface Gi1/0/2", "switchport mode access", "switchport access vlan 20", "interface Gi1/0/24", "switchport mode trunk", "end"]);
    x.secs(2);
    t.eq(&format!("sample 2: the switch ({})", x.why()), x.score().points, 8);
    let r1 = x.id("R1");
    x.cli(r1, &["enable", "conf t", "interface g0/0/0", "no shutdown", "interface g0/0/0.10", "encapsulation dot1q 10", "ip address 10.0.10.1 255.255.255.0", "interface g0/0/0.20", "encapsulation dot1Q 20", "ip add 10.0.20.1 255.255.255.0", "end"]);
    x.secs(3);
    t.eq(&format!("sample 2: solved ({})", x.why()), x.score().points, 20);
    // a subinterface on the wrong VLAN: its item says what is missing
    x.cli(r1, &["conf t", "interface g0/0/0.20", "encapsulation dot1Q 30", "end"]);
    x.secs(1);
    let sc = x.score();
    t.check(&format!("sample 2: the wrong VLAN ({})", x.why()), !sc.items[7].held && sc.items[7].found.text() == "es fehlt: interface GigabitEthernet0/0/0.20 › encapsulation dot1Q 20" && sc.points < 20);
}

// three routers: the cable, then the routes (R1 with a default route: one of the values)
fn solve3_t(t: &mut T) {
    let Some(mut x) = Solve::new(S3) else { return };
    x.secs(2);
    let (r1, r2, r3) = (x.id("R1"), x.id("R2"), x.id("R3"));
    x.edit(|p| {
        let _ = p.connect(&PortRef { dev: r2, port: s("Gi0/0/1") }, &PortRef { dev: r3, port: s("Gi0/0/0") }, None);
    });
    x.secs(2);
    t.eq(&format!("sample 3: the cable ({})", x.why()), x.score().points, 2);
    x.cli(r1, &["enable", "conf t", "ip route 0.0.0.0 0.0.0.0 10.0.12.2", "end"]);
    x.cli(r2, &["enable", "conf t", "ip route 10.0.1.0 255.255.255.0 10.0.12.1", "ip route 10.0.3.0 255.255.255.0 10.0.23.2", "end"]);
    x.secs(1);
    t.eq(&format!("sample 3: R1 and R2 ({})", x.why()), x.score().points, 11);
    x.cli(r3, &["enable", "conf t", "ip route 10.0.1.0 255.255.255.0 10.0.23.1", "end"]);
    x.secs(2);
    t.eq(&format!("sample 3: solved ({})", x.why()), x.score().points, 20);
}
