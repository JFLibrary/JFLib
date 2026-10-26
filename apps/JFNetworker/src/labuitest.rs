// labuitest -- the selftest's group "labui": what U6a adds for the lab in the window, checked
// without a window (tests/fixtures/lab_window.jfnet: a cache, a load balancer, two web servers,
// three KV nodes, a queue with a worker, DNS, DHCP and a script; lab_shards.jfnet)
//
//   texts    the status lines of lb, kv, cache, mq and worker as messages (their German text is
//            status(), the English comes from engine.tsv), the disk's line (Disk::msg against
//            Disk::text, also slowed), the line of every kind of fault (FaultMark::msg against
//            its text, which the runner's reports print) and the notes of start and end
//   kpis     kpi_between and shard_load_between over a span that ends now equal kpi and
//            shard_load; a span in the past stays what it was after a fault
//   panel    the Metriken panel's tables: every series of the charts has a word and every word,
//            title, kind and pill an English row; the tiles of a KPI as the charts write them;
//            the charts of the lab's services have series; the pills and sparklines of a
//            list of services
//   steps    faults as undo steps of the document (doc/faults.rs): an injection and its undo
//            and redo, a heal and its undo with the time left, heal_all, the document as clean
//            as it was throughout, an old step after a fresh engine

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::Project;
use jfn_sim::lab::SvcBrief;
use jfn_sim::{Action, Cmd, Engine, Fault, FaultMark, Kpi, Note, MS, SEC};

use crate::doc::faults::Step;
use crate::doc::Doc;
use crate::selftest::T;
use crate::workspace::canvas::lab::{pill_of, pills_from, sparks_from, PILLS};
use crate::workspace::metrics::{groups, kind_name, kpi_tiles, series_label};

const BUDGET: usize = 50_000_000;
const LAB: &str = include_str!("../tests/fixtures/lab_window.jfnet");
const SHARDS: &str = include_str!("../tests/fixtures/lab_shards.jfnet");

fn s(x: &str) -> String {
    String::from(x)
}

// the rows of both catalogues: German -> English
fn rows() -> Vec<(&'static str, &'static str)> {
    let all = |tsv: &'static str| tsv.lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t'));
    all(include_str!("locales/ui.tsv")).chain(all(include_str!("locales/engine.tsv"))).collect()
}

/// the English of the catalogues as the app's lookup gives it (the source when missing)
fn english() -> impl Fn(&'static str) -> &'static str {
    let rows = rows();
    move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1)
}

fn project(text: &str) -> Project {
    Project::from_text(text).unwrap_or_else(|_| Project::new("lab", 1))
}

pub(crate) fn run(t: &mut T) {
    texts(t);
    kpis(t);
    panel(t);
    steps(t);
}

// a fault into an engine: its mark
fn mark(e: &mut Engine, f: Fault) -> Option<FaultMark> {
    let id = e.next_fault_id();
    e.apply(Action::Fault { fault: f, dur: None }).ok()?;
    e.faults().iter().find(|m| m.id == id).cloned()
}

fn texts(t: &mut T) {
    let en = english();
    let p = project(LAB);
    let id = |n: &str| p.find(n).map(|d| d.id).unwrap_or(0);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.run_until(8 * SEC, BUDGET);
    // the status lines: German as before, English through the catalogue
    let lines = [
        ("LB1", "lb", "HTTP port 80, round-robin: 2 of 2 backends available, 249 requests, 0 errors"),
        ("KV2", "kv", "10.0.2.22: 35 keys, 3 of 3 nodes reachable, RF 2, eventual W1/R1"),
        ("CACHE1", "cache", "Port 80 in front of http://10.0.1.1/: 22 entries, 263 hits, 99 misses (72.6 % hits)"),
        ("MQ1", "mq", "Port 9000: 1 topics, 0 waiting, 1 out, 209 acknowledged"),
        ("WRK1", "worker", "http://10.0.2.30:9000/ jobs: 1 of 2 busy, 209 done, 0 failed"),
    ];
    for (dev, svc, want) in lines {
        let v = e.services(id(dev)).into_iter().find(|x| x.name == svc);
        t.check(&format!("status {}: its German text", dev), v.as_ref().is_some_and(|v| v.status == v.status_msg.text() && !v.status.is_empty()));
        t.eq(&format!("status {}: English", dev), v.map(|v| v.status_msg.text_with(&en)), Some(s(want)));
    }
    // a coordinator without a node of its own says so
    let kv = e.services(id("WEB1")).into_iter().find(|x| x.name == "web").map(|v| v.status_msg.text_with(&en));
    t.check(&format!("status of a web server translates: {:?}", kv), kv.is_some_and(|x| x.starts_with("Port 80: ")));
    // the disk's line
    let disk = e.disk_of(id("KV2")).cloned();
    t.eq("disk: its message is its text", disk.as_ref().map(|d| (d.msg().text(), d.msg().text_with(&en))), disk.as_ref().map(|d| (d.text(), s("853 B of 1 GiB, 100 \u{b5}s"))));
    let slow = mark(&mut e, Fault::SlowDisk { dev: id("KV3"), extra: 10 * MS, factor_pm: 1500 });
    let d3 = e.disk_of(id("KV3")).cloned();
    t.check("disk slowed: German", d3.as_ref().is_some_and(|d| d.msg().text() == d.text() && d.text().ends_with("10,100 ms (verlangsamt)")));
    t.check("disk slowed: English", d3.as_ref().is_some_and(|d| d.msg().text_with(&en).ends_with("10.100 ms (slowed down)")));
    // the lines of the faults: the message's German text is the mark's (the reports print it)
    let cable = p.links.iter().find(|l| l.a.dev == id("GEN")).map_or(0, |l| l.id);
    let marks = [
        (mark(&mut e, Fault::Unplug { link: cable }), "Cable GEN eth0 \u{2013} SW1 Gi1/0/1 pulled"),
        (mark(&mut e, Fault::Impair { link: cable, delay: 20 * MS, jitter: MS, loss_ppm: 5000, kbps: Some(10_000) }), "Cable GEN eth0 \u{2013} SW1 Gi1/0/1: +20.000 ms delay, +1.000 ms jitter, 0.5 % loss, at most 10 Mbit/s"),
        (mark(&mut e, Fault::Impair { link: cable, delay: 0, jitter: 0, loss_ppm: 50_000, kbps: None }), "Cable GEN eth0 \u{2013} SW1 Gi1/0/1: 5 % loss"),
        (mark(&mut e, Fault::Impair { link: cable, delay: 0, jitter: 0, loss_ppm: 250, kbps: Some(2_500_000) }), "Cable GEN eth0 \u{2013} SW1 Gi1/0/1: 250 ppm loss, at most 2.5G"),
        (mark(&mut e, Fault::Impair { link: cable, delay: 0, jitter: 0, loss_ppm: 0, kbps: None }), "Cable GEN eth0 \u{2013} SW1 Gi1/0/1: unchanged"),
        (mark(&mut e, Fault::Power { dev: id("KV2") }), "KV2 switched off"),
        (mark(&mut e, Fault::Stop { dev: id("KV1"), svc: s("kv") }), "Service kv on KV1 stopped"),
        (slow, "Disk of KV3 slow: +10.000 ms per access, transfer \u{d7}1.5"),
        (mark(&mut e, Fault::SlowDisk { dev: id("MQ1"), extra: 0, factor_pm: 3000 }), "Disk of MQ1 slow: transfer \u{d7}3"),
        (mark(&mut e, Fault::Partition { a: alloc::vec![id("KV1")], b: Vec::new() }), "Partition: KV1 | all others"),
        (mark(&mut e, Fault::Partition { a: alloc::vec![id("KV1")], b: alloc::vec![id("KV2"), id("KV3")] }), "Partition: KV1 | KV2, KV3"),
    ];
    for (k, (m, want)) in marks.iter().enumerate() {
        t.check(&format!("fault {}: its message is its text", k), m.as_ref().is_some_and(|m| m.msg.text() == m.text));
        t.eq(&format!("fault {}: English", k), m.as_ref().map(|m| m.msg.text_with(&en)), Some(s(want)));
    }
    t.eq("fault: the German of one", marks[1].0.as_ref().map(|m| m.text.clone()), Some(s("Kabel GEN eth0 \u{2013} SW1 Gi1/0/1: +20,000 ms Laufzeit, +1,000 ms Jitter, 0,5 % Verlust, höchstens 10 Mbit/s")));
    // the notes of start and end carry the message
    let first = marks[5].0.as_ref().map_or(0, |m| m.id);
    let _ = e.apply(Action::Heal { id: first });
    let ev: Vec<(bool, String, String)> = e
        .trace
        .events()
        .filter_map(|x| match &x.note {
            Note::FaultStart { id, .. } | Note::FaultEnd { id, .. } if *id == first => Some((matches!(x.note, Note::FaultStart { .. }), e.describe(x), e.message(x).text_with(&en))),
            _ => None,
        })
        .collect();
    t.eq(
        "fault: its notes",
        ev,
        alloc::vec![
            (true, format!("Fehler {} beginnt: KV2 ausgeschaltet.", first), format!("Fault {} starts: KV2 switched off.", first)),
            (false, format!("Fehler {} behoben: KV2 ausgeschaltet.", first), format!("Fault {} healed: KV2 switched off.", first)),
        ],
    );
}

fn kpis(t: &mut T) {
    let p = project(SHARDS);
    let id = |n: &str| p.find(n).map(|d| d.id).unwrap_or(0);
    let (lb, kv2) = (id("LB1"), id("KV2"));
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.run_until(6 * SEC, BUDGET);
    let now = e.kpi(lb, "lb", 4 * SEC);
    t.check("kpi_between up to now is kpi", now.is_some() && e.kpi_between(lb, "lb", 2 * SEC, 6 * SEC) == now);
    let shards = e.shard_load(4 * SEC);
    t.check("shard_load_between up to now is shard_load", !shards.is_empty() && e.shard_load_between(2 * SEC, 6 * SEC) == shards);
    let _ = e.apply(Action::Fault { fault: Fault::Power { dev: kv2 }, dur: Some(6 * SEC) });
    e.run_until(12 * SEC, BUDGET);
    t.eq("a span in the past stays", e.kpi_between(lb, "lb", 2 * SEC, 6 * SEC), now.clone());
    t.eq("its shards stay", e.shard_load_between(2 * SEC, 6 * SEC), shards);
    let during = e.kpi_between(lb, "lb", 6 * SEC, 12 * SEC);
    t.check(&format!("the span of the fault differs: {:?}", during.as_ref().map(|k| k.p99_us)), during.is_some_and(|k| k.p99_us > now.as_ref().and_then(|n| n.p99_us)));
    t.eq("KV2's share while off", e.shard_load_between(7 * SEC, 12 * SEC).iter().find(|x| x.dev == kv2).map(|x| x.share_pm), Some(0));
}

fn panel(t: &mut T) {
    let rows = rows();
    let has = |k: &str| rows.iter().any(|r| r.0 == k);
    let kinds = ["lb", "kv", "cache", "mq", "worker", "http-server", "http-load", "http-client", "dns-server", "dhcp-server", "script", "ntp"];
    for k in kinds {
        for g in groups(k) {
            t.check(&format!("chart title {} has a row", g.title), has(g.title));
            for name in g.names {
                let l = series_label(name);
                t.check(&format!("series {} has a word", name), l != *name);
                t.check(&format!("the word of {} has a row", name), has(&l));
            }
        }
        t.check(&format!("kind {} has a name with a row", k), has(kind_name(k)));
    }
    t.eq("an unknown series keeps its name", series_label("b1_requests"), s("b1_requests"));
    // the pills
    t.eq("pills of the lab's kinds", ["lb", "kv", "cache", "mq", "worker", "http-server", "dns-server", "dhcp-server", "script"].iter().map(|k| pill_of(k)).collect::<Vec<_>>(), (0..9).map(Some).collect::<Vec<_>>());
    t.eq("no pill for clients", (pill_of("http-load"), pill_of("dhcp-client"), pill_of("tcp-echo")), (None, None, None));
    t.check("every pill has a row", PILLS.iter().all(|p| has(p.text)));
    let b = |dev: u32, name: &str, kind: &'static str, running: bool| SvcBrief { dev, name: s(name), kind, enabled: true, running };
    let list = alloc::vec![b(1, "web", "http-server", true), b(1, "lb", "lb", true), b(2, "a", "kv", false), b(2, "b", "kv", true), b(3, "load", "http-load", true), b(4, "named", "dns-server", false)];
    t.eq("pills: one per kind in order, running when one runs", pills_from(&list).into_iter().collect::<Vec<_>>(), alloc::vec![(1, alloc::vec![(0, true), (5, true)]), (2, alloc::vec![(1, true)]), (4, alloc::vec![(6, false)])]);
    t.eq("sparklines: the first service of their kinds", sparks_from(&list).into_iter().map(|(d, (n, x))| (d, n, x)).collect::<Vec<_>>(), alloc::vec![(1, s("lb"), "requests"), (2, s("a"), "requests")]);
    // the tiles of a KPI as the charts write their values
    let k = Kpi { dev: 1, svc: s("lb"), kind: "lb", rps_milli: Some(101_750), p50_us: Some(1133), p95_us: Some(1133), p99_us: Some(249_561), err_pm: Some(5), queue: None, inflight: Some(0), hit_pm: None, load_milli: None, keys: None, bytes: None };
    let want = [("Anfragen/s", "102"), ("p50", "1,13 ms"), ("p95", "1,13 ms"), ("p99", "250 ms"), ("Fehlerquote", "0,5 %"), ("In Arbeit", "0")];
    t.eq("tiles of a balancer", kpi_tiles(&k, None, ',' as u32), want.iter().map(|(a, b)| (s(a), s(b))).collect::<Vec<_>>());
    let k = Kpi { kind: "kv", rps_milli: Some(44_500), load_milli: Some(72_750), keys: Some(138), bytes: Some(3_400), p50_us: None, p95_us: None, p99_us: None, err_pm: None, inflight: None, ..k };
    let got = kpi_tiles(&k, Some(357), '.' as u32);
    t.eq("tiles of a KV node", got, [("Anfragen/s", "44.5"), ("Last/s", "72.8"), ("Lastanteil", "35.7 %"), ("Schlüssel", "138"), ("Daten", "3.4 kB")].iter().map(|(a, b)| (s(a), s(b))).collect::<Vec<_>>());
    // the charts of the lab's services have series
    let p = project(LAB);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.run_until(6 * SEC, BUDGET);
    for x in e.series_services() {
        let have: Vec<&str> = e.series(x.dev, &x.svc).iter().map(|v| v.name).collect();
        let charts = groups(x.kind).iter().filter(|g| g.names.iter().any(|n| have.contains(n))).count();
        let want = match x.kind {
            "dns-server" | "dhcp-server" => 1,
            "script" => 2,
            _ => 3,
        };
        t.check(&format!("charts of {} {} ({}): {}", x.dev, x.svc, x.kind, charts), charts >= want);
    }
}

// the active faults of the document's engine: (number, kind, planned end)
fn active(d: &Doc) -> Vec<(u32, &'static str, Option<u64>)> {
    d.sim.engine().active_faults().map(|m| (m.id, m.fault.kind(), m.until)).collect()
}

fn steps(t: &mut T) {
    let p = project(SHARDS);
    let id = |n: &str| p.find(n).map(|d| d.id).unwrap_or(0);
    let (kv1, kv2, kv3) = (id("KV1"), id("KV2"), id("KV3"));
    let mut d = Doc::new();
    d.replace(p.clone(), None);
    d.sim.with_engine("test: run", |e| {
        e.run_until(2 * SEC, BUDGET);
    });
    // an injection: one step, the document stays clean
    t.eq("inject", d.inject(Fault::Power { dev: kv2 }, None), Ok(1));
    t.check("inject: a step, clean", d.hist.can_undo() && !d.dirty());
    t.eq("inject: active", active(&d), alloc::vec![(1, "power", None)]);
    let item = Step::of(d.hist.undo_value()).and_then(|s| s.items.first().cloned());
    t.eq("inject: the step knows its run", item.as_ref().and_then(|i| d.fault_run(i)), Some(1));
    t.check("undo heals", d.undo() && active(&d).is_empty() && !d.dirty());
    t.check("redo injects anew", d.redo() && active(&d) == alloc::vec![(2, "power", None)] && !d.dirty());
    t.eq("redo: the step's run", item.as_ref().and_then(|i| d.fault_run(i)), Some(2));
    // a fault with a duration healed early: its undo brings it back with the time it had left
    d.sim.with_engine("test: run", |e| {
        e.run_until(3 * SEC, BUDGET);
    });
    t.eq("inject for 10 s", d.inject(Fault::Stop { dev: kv1, svc: s("kv") }, Some(10 * SEC)), Ok(3));
    d.sim.with_engine("test: run", |e| {
        e.run_until(5 * SEC, BUDGET);
    });
    t.eq("heal", d.heal(&[3]), Ok(1));
    t.eq("heal: one left", active(&d), alloc::vec![(2, "power", None)]);
    t.check("undo the heal", d.undo() && active(&d) == alloc::vec![(2, "power", None), (4, "stop", Some(13 * SEC))]);
    t.check("redo the heal", d.redo() && active(&d) == alloc::vec![(2, "power", None)]);
    t.check("undo it once more", d.undo() && active(&d).len() == 2);
    // every fault at once
    t.eq("heal_all", d.heal_all(), Ok(2));
    t.check("heal_all: none", active(&d).is_empty());
    t.check("undo heal_all", d.undo() && active(&d) == alloc::vec![(6, "power", None), (7, "stop", Some(13 * SEC))]);
    t.check("redo heal_all", d.redo() && active(&d).is_empty() && !d.dirty());
    t.eq("heal: nothing active", d.heal(&[99]), Ok(0));
    // the document's state: a change of the project between fault steps
    t.eq("inject KV3", d.inject(Fault::Power { dev: kv3 }, None), Ok(8));
    if let Some(e) = crate::doc::edits::title(&d.proj, "Neu") {
        d.apply(e);
    }
    t.check("a real change: dirty", d.dirty());
    t.eq("a fault after it", d.inject(Fault::Power { dev: kv1 }, None), Ok(9));
    t.check("still dirty", d.dirty());
    t.check("undo the fault: dirty", d.undo() && d.dirty() && active(&d) == alloc::vec![(8, "power", None)]);
    t.check("undo the change: clean", d.undo() && !d.dirty());
    t.check("undo the first: clean", d.undo() && !d.dirty() && active(&d).is_empty());
    t.check("redo it: clean", d.redo() && !d.dirty() && active(&d) == alloc::vec![(10, "power", None)]);
    // a fresh engine numbers anew: an old step leaves the new faults alone, also the one with
    // its run's number (another start)
    d.sim = crate::doc::sim::Sim::new(&d.proj);
    for _ in 0..10 {
        let _ = d.sim.send(Cmd::Apply(Action::Fault { fault: Fault::Power { dev: kv3 }, dur: None }));
    }
    t.eq("fresh engine: ten faults", active(&d).len(), 10);
    t.check("an old undo heals nothing new", d.undo() && active(&d).len() == 10);
}
