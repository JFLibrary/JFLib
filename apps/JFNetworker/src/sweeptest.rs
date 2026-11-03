// sweeptest -- parameter sweeps (part of --selftest, group sweep; jfn_model's experiment.rs, jfn_sim's
// experiment.rs, jfn_run's run_sweep): the file and what it asks of a project, with exact messages for
// a bad axis; what each axis does to a copy of the project; a clone against a fresh boot at each level;
// the bytes at 1 and N threads and between two runs; the knee and the figures against closed forms;
// the quiet trace; Service::reconfigure of each service under load; the steps of a scenario and their
// replay from keyframes; lib/rt's heaps under threads, as a sweep leans on them
//
// The closed forms:
//
//   knee           open load, evenly spaced, on c cores with a job of S (no longer than a piece of CPU
//                  work, res::QUANTUM, so that it runs first come first served): no wait up to c / S,
//                  beyond it the backlog grows by the difference, so the knee is the highest level below
//   achieved       X = min(level, c / S), closed loop X = min(users / S, c / S)
//   CPU            rho = X * S / c
//   lock           X = 1 / hold: the lock is held all the time, the CPU is not
//   disk           the channel's share of time = X * bytes / rate
//   cable          the share of the line = X * bytes * 8 / bit rate (frames without preamble and gap)
//   cost           euros per 1000 = euros per hour / (requests per hour) * 1000
//
// "--selftest --only=sweep --numbers" prints each comparison with its numbers.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::catalog::{DiskClass, Hw, Prices};
use jfn_model::experiment::{set_path, Experiment, Level};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, LbStep, StepKind};
use jfn_sim::experiment::{Plan, Status};
use jfn_sim::keyframe::{digest, Budget};
use jfn_sim::{Action, Cmd, Engine, Inline, MS, SEC};

use crate::doc::sim::live_steps;
use crate::selftest::T;

const BUDGET: usize = 80_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn near(got: f64, want: f64, tol: f64) -> bool {
    (got - want).abs() <= want.abs() * tol
}

fn check_near(t: &mut T, name: &str, got: f64, want: f64, tol: f64) {
    let line = format!("{}: {:.3} against {:.3} (within {:.1} %)", name, got, want, tol * 100.0);
    if plat::args().iter().any(|a| a == "--numbers") {
        plat::write_stdout(format!("{}, off by {:.2} %\n", line, (got - want).abs() / want.abs().max(1e-12) * 100.0).as_bytes());
    }
    t.check(&line, near(got, want, tol));
}

/// a load generator and a server with a config of its own on one cable
fn pair(hw: Option<Hw>, server: &str, load: &str) -> Project {
    let mut p = Project::new("sweep", 5);
    let g = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let s = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(g, "eth0"), &pr(s, "eth1"), None);
    if let Some(d) = p.device_mut(g) {
        d.name = String::from("GEN");
        d.startup_config = String::from("ip addr add 10.0.0.1/24 dev eth0\n");
        d.services.push(def("http-load", "load", load));
    }
    if let Some(d) = p.device_mut(s) {
        d.name = String::from("WEB");
        d.startup_config = String::from("ip addr add 10.0.0.2/24 dev eth1\n");
        d.services.push(def("http-server", "web", server));
        d.hw = hw;
    }
    p
}

fn cores(n: u32) -> Hw {
    Hw { cores: n, speed_pm: 1000, ram_mb: 8 << 10, swap_mb: 1 << 10, disk: DiskClass::Sata, disk_gb: 240 }
}

const LOAD: &str = r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 10, "timeout_ms": 3000}"#;
const PATIENT: &str = r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 10, "timeout_ms": 60000}"#;

/// a server of `cost_ms` per request (no limit on connections' requests)
fn web(cost_ms: &str) -> String {
    format!(r#"{{"port": 80, "cpu_ms": {}, "max_requests": 0, "routes": [{{"path": "*", "method": "", "body": "ok"}}]}}"#, cost_ms)
}

fn experiment(text: &str) -> Experiment {
    Experiment::from_text(text).unwrap_or_default()
}

/// the texts of an experiment's findings on a project
fn found(p: &Project, text: &str) -> Vec<String> {
    match Experiment::from_text(text) {
        Ok(x) => x.problems(p).iter().map(|m| m.text()).collect(),
        Err(e) => vec![e.text()],
    }
}

fn exp_rps(levels: &str, measure: u64, more: &str) -> String {
    format!(r#"{{"format": "jfnex", "version": 1, "load": {{"dev": "GEN", "svc": "load", "rps": [{}]}}, "timing": {{"warmup_s": 2, "settle_s": 1, "measure_s": {}, "seeds": [5]}}, "slo": {{"p99_ms": 200, "errors_pct": 1}}{}}}"#, levels, measure, more)
}

pub(crate) fn run(t: &mut T) {
    file_and_checks(t);
    axes(t);
    clone_and_threads(t);
    knee(t);
    figures(t);
    quiet(t);
    reconfigure(t);
    window(t);
    steps(t);
    replay(t);
    allocator(t);
}

// ---------------------------------------------------------------- the file and what it asks

fn file_and_checks(t: &mut T) {
    let p = pair(None, &web("1"), LOAD);
    let good = exp_rps("100, 200", 2, "");
    t.eq("a good file has no findings", found(&p, &good), Vec::<String>::new());
    let x = experiment(&good);
    t.eq("levels", x.levels(), vec![Level::Rps(100), Level::Rps(200)]);
    t.eq("level text", (Level::Rps(400).text(), Level::Users(8).text()), (String::from("400/s"), String::from("8 Nutzer")));
    t.eq("the same after writing it", Experiment::from_text(&x.to_text()).ok(), Some(x.clone()));
    t.eq("no seeds: the project's", experiment(r#"{"format": "jfnex", "version": 1}"#).seeds_or(7), vec![7]);
    // the file itself
    t.eq("another format", Experiment::from_text(r#"{"format": "jfn", "version": 1}"#).map_err(|e| e.text()), Err(String::from("Das ist keine Experiment-Datei.")));
    t.eq("a newer version", Experiment::from_text(r#"{"format": "jfnex", "version": 9}"#).map_err(|e| e.text()), Err(String::from("Die Datei hat die Version 9; dieses Programm liest Version 1 bis 1.")));
    t.check("no JSON", Experiment::from_text("{").is_err_and(|e| e.text().contains("kein gültiges JSON")));
    // the load
    let one = |t: &mut T, name: &str, text: String, want: &str| {
        let got = found(&p, &text);
        t.eq(name, got.first().cloned(), Some(String::from(want)));
    };
    one(t, "no levels", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load"}, "timing": {"measure_s": 1}}"#), "Last: es fehlt \u{201e}rps\u{201c} oder \u{201e}users\u{201c}.");
    one(t, "both kinds", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load", "rps": [1], "users": [1]}, "timing": {"measure_s": 1}}"#), "Last: \u{201e}rps\u{201c} und \u{201e}users\u{201c} schließen sich aus.");
    one(t, "levels must rise", exp_rps("200, 100", 2, ""), "Last: die Stufen müssen streng steigen, das Knie gilt sonst nichts.");
    one(t, "a level of 0", exp_rps("0, 100", 2, ""), "Last: eine Stufe ist 0.");
    one(t, "no generator's device", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "NIX", "svc": "load", "rps": [1]}, "timing": {"measure_s": 1}}"#), "Last: das Gerät \u{201e}NIX\u{201c} gibt es nicht.");
    one(t, "no such service", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "x", "rps": [1]}, "timing": {"measure_s": 1}}"#), "Last: GEN hat keinen Dienst \u{201e}x\u{201c}.");
    one(t, "no generator", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "WEB", "svc": "web", "rps": [1]}, "timing": {"measure_s": 1}}"#), "Last: der Dienst \u{201e}web\u{201c} ist kein Lastgenerator (http-load), sondern http-server.");
    // the time, the limits, the prices
    one(t, "no measuring", exp_rps("1", 0, ""), "Zeit: \u{201e}measure_s\u{201c} fehlt.");
    one(t, "longer than the windows", exp_rps("1", 300, ""), "Zeit: Übergang und Messung zusammen höchstens 280 s; so lange halten die Dienste ihre Latenzen.");
    one(t, "a seed twice", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load", "rps": [1]}, "timing": {"measure_s": 1, "seeds": [3, 3]}}"#), "Zeit: ein Seed steht zweimal da; er ergäbe denselben Lauf.");
    one(t, "a limit that is no number", String::from(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load", "rps": [1]}, "timing": {"measure_s": 1}, "slo": {"p99_ms": "bald"}}"#), "SLO: \u{201e}p99_ms\u{201c} ist keine Zahl.");
    one(t, "a price without a name", exp_rps("1", 1, r#", "prices": {"gold": 1}"#), "Preise: \u{201e}gold\u{201c} gibt es nicht (es gibt: core, big, huge, ram_gb, hdd_gb, sata_gb, nvme_gb, nvme_fast_gb, host, switch, router).");
    one(t, "an excluded device", exp_rps("1", 1, r#", "cost_exclude": ["NIX"]"#), "Kosten: das Gerät \u{201e}NIX\u{201c} gibt es nicht.");
    // the axes, each with what it found
    let axis = |t: &mut T, name: &str, json: &str, want: &str| {
        let text = exp_rps("1", 1, &format!(r#", "axes": [{}]"#, json));
        let got = found(&p, &text);
        t.eq(name, got.first().cloned(), Some(format!("Achse 1: {}", want)));
    };
    axis(t, "an unknown kind", r#"{"kind": "gpu", "dev": "WEB", "values": [1]}"#, "Die Art \u{201e}gpu\u{201c} gibt es nicht (hw, config, backends).");
    axis(t, "no device", r#"{"kind": "hw", "values": [{"cores": 2}]}"#, "Es fehlt \u{201e}dev\u{201c} oder \u{201e}group\u{201c}.");
    axis(t, "another device", r#"{"kind": "hw", "dev": "NIX", "values": [{"cores": 2}]}"#, "Das Gerät \u{201e}NIX\u{201c} gibt es nicht.");
    axis(t, "another group", r#"{"kind": "hw", "group": "NIX", "values": [{"cores": 2}]}"#, "Die Gruppe \u{201e}NIX\u{201c} gibt es nicht.");
    axis(t, "no values", r#"{"kind": "hw", "dev": "WEB"}"#, "\u{201e}values\u{201c} ist leer.");
    axis(t, "a word for hardware", r#"{"kind": "hw", "dev": "WEB", "values": [{"kerne": 2}]}"#, "\u{201e}kerne\u{201c} gibt es bei Hardware nicht (preset, cores, speed, ram_gb, swap_gb, disk, disk_gb).");
    axis(t, "a preset without figures", r#"{"kind": "hw", "dev": "WEB", "values": [{"preset": "jf-r2"}]}"#, "Das Modell \u{201e}jf-r2\u{201c} hat keine Hardware-Angaben.");
    axis(t, "no cores", r#"{"kind": "hw", "dev": "WEB", "values": [{"cores": 0}]}"#, "\u{201e}cores\u{201c} ist eine Zahl von 1 bis 1024.");
    axis(t, "a disk that is none", r#"{"kind": "hw", "dev": "WEB", "values": [{"disk": "band"}]}"#, "Die Plattenart \u{201e}band\u{201c} gibt es nicht (sata, nvme, nvme-fast, hdd).");
    axis(t, "a path without a member", r#"{"kind": "config", "dev": "WEB", "path": "workers", "values": [1]}"#, "Der Pfad \u{201e}workers\u{201c} nennt einen Dienst und seine Einstellung, etwa http-server.workers.");
    axis(t, "a service it does not have", r#"{"kind": "config", "dev": "WEB", "path": "lb.algorithm", "values": ["hash"]}"#, "WEB hat keinen Dienst \u{201e}lb\u{201c} (es gibt: web).");
    let keys = "port, root, workers, queue, time_ms, jitter_ms, error_rate, error_status, keepalive_ms, max_requests, header_timeout_ms, max_body, backlog, server, routes, kv, db, cpu_ms, serial_us, usl, mem_mb, conn_kb, data_mb, tls";
    axis(t, "a setting with a typo", r#"{"kind": "config", "dev": "WEB", "path": "http-server.worker", "values": [1]}"#, &format!("Ein Dienst der Art http-server kennt \u{201e}worker\u{201c} nicht (es gibt: {}).", keys));
    axis(t, "an element that is not there", r#"{"kind": "config", "dev": "WEB", "path": "web.routes.3.cpu_ms", "values": [1]}"#, "Im Pfad gibt es kein Element 3.");
    axis(t, "backends without a service", r#"{"kind": "backends", "dev": "WEB", "svc": "lb", "pool": ["WEB"], "counts": [1]}"#, "WEB hat keinen Dienst \u{201e}lb\u{201c}.");
    axis(t, "backends of something else", r#"{"kind": "backends", "dev": "WEB", "svc": "web", "pool": ["WEB"], "counts": [1]}"#, "Der Dienst \u{201e}web\u{201c} ist kein Load Balancer, sondern http-server.");
    let lb_net = lb_net();
    let lb_axis = |t: &mut T, name: &str, json: &str, want: &str| {
        let got = found(&lb_net, &exp_rps("1", 1, &format!(r#", "axes": [{}]"#, json)));
        t.eq(name, got.first().cloned(), Some(format!("Achse 1: {}", want)));
    };
    lb_axis(t, "no counts", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1"]}"#, "\u{201e}counts\u{201c} ist leer.");
    lb_axis(t, "no backend", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1"], "counts": [0]}"#, "Ein Load Balancer braucht mindestens ein Backend; 0 geht nicht.");
    lb_axis(t, "no pool", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "counts": [1]}"#, "Es fehlt \u{201e}pool\u{201c} oder \u{201e}template\u{201c}.");
    lb_axis(t, "both ways", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1"], "template": "W1", "counts": [1]}"#, "\u{201e}pool\u{201c} und \u{201e}template\u{201c} schließen sich aus.");
    lb_axis(t, "too few in the pool", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1"], "counts": [2]}"#, "\u{201e}counts\u{201c} verlangt 2 Backends, der Pool hat 1.");
    lb_axis(t, "a host that is not there", r#"{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W9"], "counts": [1]}"#, "Das Gerät \u{201e}W9\u{201c} gibt es nicht.");
    // an axis in a series names it
    let got = found(&p, &exp_rps("1", 1, r#", "series": [{"name": "a", "axes": [{"kind": "gpu"}]}]"#));
    t.eq("an axis in a series", got.first().cloned(), Some(String::from("Reihe a, Achse 1: Die Art \u{201e}gpu\u{201c} gibt es nicht (hw, config, backends).")));
    let got = found(&p, &exp_rps("1", 1, r#", "series": [{"name": "a"}, {"name": "a"}]"#));
    t.eq("a series twice", got.first().cloned(), Some(String::from("Die Reihe \u{201e}a\u{201c} steht zweimal da.")));
    // a plan says the same; a value only the service itself can judge is found when it is built
    let bad = experiment(&exp_rps("1", 1, r#", "axes": [{"kind": "config", "dev": "WEB", "path": "http-server.routes", "values": [[{"path": "kein"}]]}]"#));
    let plan = Plan::new(&bad, &p, "x");
    t.check("a value the service refuses", plan.is_err_and(|e| e.first().is_some_and(|m| m.text().contains("Der Pfad \u{201e}kein\u{201c} beginnt nicht mit /."))));
    // a project's section: not in the file while it is empty
    let mut q = p.clone();
    t.check("no section while there is none", !q.to_text().contains("experiments"));
    q.experiments.push(x.clone());
    t.check("a project keeps its experiments", Project::from_text(&q.to_text()).is_ok_and(|r| r.experiments == vec![x.clone()]));
    // a path in a config
    let mut c = agent_core::json::parse(br#"{"a": {"b": 1}, "l": [{"x": 1}]}"#).unwrap_or(Value::Null);
    t.check("a member on the way", set_path(&mut c, "a.c.d", Value::u64(2)).is_ok() && c.get("a").and_then(|a| a.get("c")).and_then(|x| x.get("d")).and_then(|x| x.as_u64()) == Some(2));
    t.check("an element", set_path(&mut c, "l.0.x", Value::u64(3)).is_ok() && c.get("l").and_then(|l| l.as_array()).and_then(|l| l.first()).and_then(|x| x.get("x")).and_then(|x| x.as_u64()) == Some(3));
    t.check("through a number", set_path(&mut c, "a.b.c", Value::u64(1)).is_err());
    // the prices are round assumptions: the table's sums
    let pr = Prices::ASSUMED;
    let s = Hw { cores: 4, speed_pm: 1000, ram_mb: 16 << 10, swap_mb: 0, disk: DiskClass::Sata, disk_gb: 100 };
    t.eq("a small server", pr.of_hw(&s), 4 * 30_000 + 16 * 4_000 + 100 * 100 + 20_000);
    t.eq("twice the speed: twice the cores' price", pr.of_hw(&Hw { speed_pm: 2000, ..s.clone() }), 8 * 30_000 + 16 * 4_000 + 100 * 100 + 20_000);
    let big = Hw { cores: 40, ram_mb: 0, disk_gb: 0, ..s.clone() };
    t.eq("beyond 16 and 32 a core costs more", pr.of_hw(&big), 30_000 * 16 + 45_000 * 16 + 60_000 * 8 + 20_000);
    t.eq("a switch and a router", (pr.of_device("jf-s24", None), pr.of_device("jf-r4", None)), (pr.switch, pr.router));
    t.eq("a load generator costs nothing", pr.of_device("traffic-gen", None), 0);
    t.eq("a model's figures, or its own", (pr.of_device("server-m", None), pr.of_device("server-m", Some(&s))), (pr.of_hw(&jfn_model::catalog::hw_of("server-m").unwrap_or_default()), pr.of_hw(&s)));
}

/// a load generator, a switch, a load balancer and two web servers
fn lb_net() -> Project {
    let mut p = Project::new("sweep lb", 5);
    let g = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24", 100, 0).unwrap_or(0);
    let lb = p.add_device("load-balancer", 200, 0).unwrap_or(0);
    let w1 = p.add_device("server-s", 300, 0).unwrap_or(0);
    let w2 = p.add_device("server-s", 300, 60).unwrap_or(0);
    let _ = p.connect(&pr(g, "eth0"), &pr(sw, "GigabitEthernet1/0/1"), None);
    let _ = p.connect(&pr(lb, "eth0"), &pr(sw, "GigabitEthernet1/0/2"), None);
    let _ = p.connect(&pr(w1, "eth0"), &pr(sw, "GigabitEthernet1/0/3"), None);
    let _ = p.connect(&pr(w2, "eth0"), &pr(sw, "GigabitEthernet1/0/4"), None);
    let name = |p: &mut Project, id: u32, n: &str, ip: &str, s: Vec<ServiceDef>| {
        if let Some(d) = p.device_mut(id) {
            d.name = String::from(n);
            d.startup_config = format!("hostname {}\nip addr add {}/24 dev eth0\n", n, ip);
            d.services = s;
        }
    };
    name(&mut p, g, "GEN", "10.0.0.1", vec![def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 10, "timeout_ms": 3000}"#)]);
    name(&mut p, lb, "LB", "10.0.0.2", vec![def("lb", "lb", r#"{"health": {"type": "none"}, "backends": [{"name": "W1", "addr": "10.0.0.11"}]}"#)]);
    name(&mut p, w1, "W1", "10.0.0.11", vec![def("http-server", "web", r#"{"max_requests": 0, "routes": [{"path": "*", "method": "", "body": "{host}"}]}"#)]);
    name(&mut p, w2, "W2", "10.0.0.12", vec![def("http-server", "web", r#"{"max_requests": 0, "routes": [{"path": "*", "method": "", "body": "{host}"}]}"#)]);
    if let Some(d) = p.device_mut(sw) {
        d.name = String::from("SW");
    }
    p
}

// ---------------------------------------------------------------- what an axis does

fn axes(t: &mut T) {
    let p = lb_net();
    // the cross product, the last axis fastest; a series with the axes of the experiment
    let x = experiment(&exp_rps("10", 1, r#", "axes": [{"kind": "hw", "dev": "W1", "name": "Kerne", "values": [{"cores": 1}, {"cores": 2}]}], "series": [{"name": "a", "axes": [{"kind": "config", "dev": "W1", "path": "web.workers", "values": [1, 2, 3]}]}, {"name": "b"}]"#));
    let labels: Vec<String> = x.configs().iter().map(|c| c.label()).collect();
    t.eq("the configurations in their order", labels, vec!["a 1 Kerne W1 web.workers 1", "a 1 Kerne W1 web.workers 2", "a 1 Kerne W1 web.workers 3", "a 2 Kerne W1 web.workers 1", "a 2 Kerne W1 web.workers 2", "a 2 Kerne W1 web.workers 3", "b 1 Kerne", "b 2 Kerne"].iter().map(|s| String::from(*s)).collect::<Vec<String>>());
    let plan = Plan::new(&x, &p, "x");
    t.check("a plan of them", plan.as_ref().is_ok_and(|pl| pl.cfgs.len() == 8 && pl.runs() == 8));
    // hardware: figures on what it has, a preset
    let x = experiment(&exp_rps("10", 1, r#", "axes": [{"kind": "hw", "dev": "W1", "values": [{"cores": 2, "speed": 1.5}, {"preset": "server-m"}]}]"#));
    let Ok(plan) = Plan::new(&x, &p, "x") else {
        t.check("hardware: a plan", false);
        return;
    };
    let hw = |pl: &Plan, i: usize| pl.cfgs[i].project.find("W1").and_then(|d| d.hw.clone());
    let s = jfn_model::catalog::hw_of("server-s").unwrap_or_default();
    t.eq("figures on what it has", hw(&plan, 0), Some(Hw { cores: 2, speed_pm: 1500, ..s }));
    t.eq("a preset", hw(&plan, 1), jfn_model::catalog::hw_of("server-m"));
    t.eq("the base stays", p.find("W1").and_then(|d| d.hw.clone()), None);
    // a config path, in place
    let x = experiment(&exp_rps("10", 1, r#", "axes": [{"kind": "config", "dev": "W1", "path": "http-server.workers", "values": [4]}, {"kind": "config", "dev": "LB", "path": "lb.algorithm", "values": ["least-conn"]}]"#));
    let Ok(plan) = Plan::new(&x, &p, "x") else {
        t.check("config: a plan", false);
        return;
    };
    let cfg = &plan.cfgs[0].project;
    t.eq("a server's setting", cfg.find("W1").and_then(|d| d.services[0].config.get("workers")).and_then(|v| v.as_u64()), Some(4));
    t.eq("a balancer's setting", cfg.find("LB").and_then(|d| d.services[0].config.get("algorithm")).and_then(|v| v.as_str()).map(String::from), Some(String::from("least-conn")));
    // the generator: the warm level, nothing that would end its run
    let mut with_end = p.clone();
    if let Some(d) = with_end.find("GEN").map(|d| d.id).and_then(|id| with_end.device_mut(id)) {
        d.services[0].config.insert("duration_ms", Value::u64(1));
        d.services[0].config.insert("requests", Value::u64(1));
        d.services[0].config.insert("mode", Value::string("closed"));
    }
    let x = experiment(&exp_rps("10, 20", 1, ""));
    let Ok(plan) = Plan::new(&x, &with_end, "x") else {
        t.check("load: a plan", false);
        return;
    };
    let g = plan.cfgs[0].project.find("GEN").map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    t.check("no end of its run", g.get("duration_ms").is_none() && g.get("requests").is_none());
    t.eq("open load at the first level", (g.get("mode").and_then(|m| m.as_str()).map(String::from), g.get("rps").and_then(|v| v.as_u64())), (Some(String::from("open")), Some(10)));
    // a pool: the first N behind it, the others off
    let x = experiment(&exp_rps("10", 1, r#", "axes": [{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1", "W2"], "counts": [1, 2]}]"#));
    let Ok(plan) = Plan::new(&x, &p, "x") else {
        t.check("pool: a plan", false);
        return;
    };
    let bk = |q: &Project| q.find("LB").and_then(|d| d.services[0].config.get("backends").cloned()).and_then(|b| b.as_array().map(|a| a.iter().filter_map(|x| x.get("addr").and_then(|v| v.as_str()).map(String::from)).collect::<Vec<String>>()));
    t.eq("the first of the pool", bk(&plan.cfgs[0].project), Some(vec![String::from("10.0.0.11")]));
    t.check("the other is off", plan.cfgs[0].project.find("W2").is_some_and(|d| d.off) && plan.cfgs[1].project.find("W2").is_some_and(|d| !d.off));
    t.eq("both", bk(&plan.cfgs[1].project), Some(vec![String::from("10.0.0.11"), String::from("10.0.0.12")]));
    t.check("what is off costs nothing", plan.cfgs[0].cost_uh < plan.cfgs[1].cost_uh);
    // and it works: the balancer spreads over the hosts it has
    let Ok(sw) = Plan::new(&experiment(&exp_rps("100", 2, r#", "axes": [{"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["W1", "W2"], "counts": [2]}]"#)), &p, "x") else {
        t.check("pool: a run", false);
        return;
    };
    let out = sw.run_all();
    let served: Vec<i64> = out.details[0].svcs.iter().filter(|s| s.kind == "http-server").map(|s| s.rps_milli).collect();
    t.eq("both hosts answer", served, vec![50_000, 50_000]);
    // a template: copies with the next addresses on the next free ports
    let x = experiment(&exp_rps("100", 2, r#", "axes": [{"kind": "backends", "dev": "LB", "svc": "lb", "template": "W2", "counts": [1, 3]}]"#));
    let plan = match Plan::new(&x, &p, "x") {
        Ok(pl) => pl,
        Err(e) => {
            t.eq("template: a plan", e.iter().map(|m| m.text()).collect::<Vec<String>>(), Vec::new());
            return;
        }
    };
    let three = &plan.cfgs[1].project;
    t.eq("as many devices as it takes", three.devices.len(), p.devices.len() + 2);
    t.eq("the addresses", bk(three), Some(vec![String::from("10.0.0.12"), String::from("10.0.0.13"), String::from("10.0.0.14")]));
    t.check("a copy's name and address", three.find("W2-3").is_some_and(|d| d.startup_config.contains("hostname W2-3") && d.startup_config.contains("10.0.0.14/24")));
    t.check("a copy's cable on a free port", three.find("W2-3").is_some_and(|d| three.link_at(d.id, "eth0").is_some()));
    t.check("the project is right", three.check().iter().all(|i| !i.error));
    let out = plan.run_all();
    let served: Vec<i64> = out.details[1].svcs.iter().filter(|s| s.kind == "http-server").map(|s| s.rps_milli).collect();
    t.check("the copies answer", served.iter().filter(|r| **r >= 25_000 && **r <= 45_000).count() == 3);
    // out of ports, out of addresses
    let many = exp_rps("100", 2, r#", "axes": [{"kind": "backends", "dev": "LB", "svc": "lb", "template": "W2", "counts": [30]}]"#);
    let r = Plan::new(&experiment(&many), &p, "x");
    t.check("no port left", r.is_err_and(|e| e.first().is_some_and(|m| m.text().contains("ist für die Kopie") && m.text().contains("kein Port mehr frei"))));
    let taken = exp_rps("100", 2, r#", "axes": [{"kind": "backends", "dev": "LB", "svc": "lb", "template": "W1", "counts": [2]}]"#);
    let r = Plan::new(&experiment(&taken), &p, "x");
    t.check("an address that is taken", r.is_err_and(|e| e.first().is_some_and(|m| m.text() == "Konfiguration -: Die Adresse 10.0.0.12 der Kopie 2 hat schon ein anderes Gerät.")));
}

// ---------------------------------------------------------------- clones and threads

fn clone_and_threads(t: &mut T) {
    let mut p = pair(Some(cores(2)), &web("2"), LOAD);
    if let Some(d) = p.find("GEN").map(|d| d.id).and_then(|id| p.device_mut(id)) {
        d.services[0].config.insert("arrival", Value::string("poisson"));
    }
    let x = experiment(r#"{"format": "jfnex", "version": 1, "title": "Klone", "load": {"dev": "GEN", "svc": "load", "rps": [100, 400, 900, 1500]}, "timing": {"warmup_s": 2, "settle_s": 1, "measure_s": 2, "seeds": [5, 6]}, "slo": {"p99_ms": 50, "errors_pct": 1}, "axes": [{"kind": "hw", "dev": "WEB", "values": [{"cores": 1}, {"cores": 2}, {"cores": 4}]}]}"#);
    let Ok(plan) = Plan::new(&x, &p, "klone.jfnex") else {
        t.check("clones: a plan", false);
        return;
    };
    t.eq("runs", plan.runs(), 3 * 2 * 4);
    // a clone of the warm engine against a fresh boot, at each level
    let Ok(warm) = plan.warm(1, 0) else {
        t.check("clones: warm", false);
        return;
    };
    let before = digest(&warm);
    for l in 0..plan.levels.len() {
        let a = plan.measure(&warm, l);
        let Ok(mut fresh) = plan.warm(1, 0) else { continue };
        let b = plan.run_level(&mut fresh, l);
        t.check(&format!("level {}: a clone equals a fresh boot", l), a == b && a.responses > 0);
    }
    t.eq("levels leave the warm state as it was", digest(&warm), before);
    // the order of the levels leaves no trace
    let up: Vec<_> = (0..4).map(|l| plan.measure(&warm, l)).collect();
    let down: Vec<_> = (0..4).rev().map(|l| plan.measure(&warm, l)).collect();
    t.check("in the other order", up.iter().eq(down.iter().rev()));
    // the same bytes on 1 and N threads, and between two runs
    let one = jfn_run::run_sweep(&plan, 1);
    let again = jfn_run::run_sweep(&plan, 1);
    let three = jfn_run::run_sweep(&plan, 3);
    let eight = jfn_run::run_sweep(&plan, 8);
    let seq = plan.run_all();
    t.check("the CSV has a row per point", one.csv().lines().count() == 1 + 3 * 4);
    t.eq("two runs: the CSV", one.csv(), again.csv());
    t.eq("1 and 3 threads: the CSV", one.csv(), three.csv());
    t.eq("1 and 8 threads: the CSV", one.csv(), eight.csv());
    t.eq("1 and 3 threads: the table", one.table(), three.table());
    t.eq("1 and 8 threads: the details", one.detail_csv(), eight.detail_csv());
    t.eq("without workers: the same", one.csv(), seq.csv());
    t.check("the table says what it is", one.table().contains("Klone") && one.table().contains("angenommene Preise"));
    t.check("a CSV is regular", {
        let csv = one.csv();
        let n = csv.lines().next().map_or(0, |h| h.split(',').count());
        csv.lines().all(|l| l.split(',').count() == n)
    });
    t.check("a detail CSV is regular", {
        let csv = one.detail_csv();
        let n = csv.lines().next().map_or(0, |h| h.split(',').count());
        csv.lines().count() > 1 && csv.lines().all(|l| l.split(',').count() == n)
    });
    t.eq("seeds are pooled", one.points[0].sent, one.details[0].sent + one.details[plan.index(0, 1, 0)].sent);
    // a level that holds too much ends where it is
    let mut q = pair(Some(cores(2)), &web("2"), LOAD);
    q.seed = 5;
    let x = experiment(&exp_rps("100", 8, ""));
    let Ok(mut lim) = Plan::new(&x, &q, "x") else {
        t.check("a limit: a plan", false);
        return;
    };
    lim.mem_limit = 1;
    let out = lim.run_all();
    t.eq("over the limit", out.points[0].status, Status::Memory);
    t.check("it counted what it had", out.points[0].seconds == 4 && out.points[0].achieved_m == 100_000);
    t.check("that level does not hold the SLO", !out.points[0].slo_ok && out.configs[0].knee.is_none());
}

// ---------------------------------------------------------------- the knee against a closed form

fn knee(t: &mut T) {
    // four cores, 4 ms: 1000 requests per second; evenly spaced arrivals never wait below that; nobody
    // gives up in time, as a request that does has cost its server for nothing
    let p = pair(Some(cores(4)), &web("4"), PATIENT);
    let x = experiment(&exp_rps("250, 500, 950, 1050, 1250", 10, ""));
    let Ok(plan) = Plan::new(&x, &p, "x") else {
        t.check("knee: a plan", false);
        return;
    };
    let out = jfn_run::run_sweep(&plan, 3);
    t.eq("the knee is the highest level below 1000/s", out.configs[0].knee, Some(2));
    t.check("only that row says so", out.points.iter().enumerate().all(|(i, p)| p.knee == (i == 2)));
    t.check("it holds up to there", out.points[..3].iter().all(|p| p.slo_ok) && !out.points[3].slo_ok && !out.points[4].slo_ok);
    for (i, want) in [250.0, 500.0, 950.0, 1000.0, 1000.0].iter().enumerate() {
        check_near(t, &format!("achieved at level {}", i), out.points[i].achieved_m as f64 / 1000.0, *want, 0.01);
    }
    for (i, want) in [250.0, 500.0, 950.0, 1000.0, 1000.0].iter().enumerate() {
        let got = out.points[i].cpu_max_pm as f64;
        check_near(t, &format!("CPU at level {}", i), got, *want, 0.015);
    }
    // below saturation a request takes its 4 ms and the way
    for i in 0..3 {
        let q = out.points[i].p50.unwrap_or(0) as f64 / MS as f64;
        t.check(&format!("level {}: its own time ({:.2} ms)", i, q), (4.0..=4.6).contains(&q));
    }
    t.check("beyond it the backlog shows", out.points[3].p99.unwrap_or(0) > 200 * MS);
    t.check("the bottleneck is named", out.configs[0].limit == "CPU WEB" && out.points[3].bottleneck == "CPU WEB");
    t.eq("below half a resource nothing limits", out.points[0].bottleneck.clone(), String::from("-"));
    t.check("the quantiles rise", out.points.iter().all(|p| p.p50 <= p.p95 && p.p95 <= p.p99 && p.p99 <= p.p999 && p.p999 <= p.max));
    // the cost: euros per hour over requests per hour
    let pr = Prices::ASSUMED;
    let hour = pr.of_hw(&cores(4));
    t.eq("the cost per hour", out.configs[0].cost_uh, hour);
    let want = hour as f64 / (out.points[2].achieved_m as f64 / 1000.0 * 3600.0) * 1000.0;
    check_near(t, "the cost per 1000 requests", out.points[2].cost_per_1000_u as f64, want, 0.01);
    // jobs in pieces take turns under overload: what is half done piles up, and fewer are done than the
    // cores could do; the CPU is as full as ever
    let s = pair(Some(cores(4)), &web("10"), PATIENT);
    let Ok(plan) = Plan::new(&experiment(&exp_rps("420", 10, "")), &s, "x") else {
        t.check("pieces: a plan", false);
        return;
    };
    let out = plan.run_all();
    t.check("pieces under overload: fewer are done", out.points[0].achieved_m < 400_000 && out.points[0].cpu_max_pm >= 990);
    // a closed loop: users / S up to the cores, then a plateau
    let mut q = pair(Some(cores(4)), &web("10"), r#"{"targets": ["http://10.0.0.2/"], "mode": "closed", "users": 1, "timeout_ms": 3000}"#);
    q.seed = 5;
    let x = experiment(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load", "users": [1, 2, 4, 8, 16]}, "timing": {"warmup_s": 1, "settle_s": 2, "measure_s": 5}, "slo": {"p99_ms": 30}}"#);
    let Ok(plan) = Plan::new(&x, &q, "x") else {
        t.check("closed: a plan", false);
        return;
    };
    let out = plan.run_all();
    // a user adds a little way to each round, so that its rate is a little below 1 / S
    for (i, want) in [99.0, 198.0, 396.0, 400.0, 400.0].iter().enumerate() {
        check_near(t, &format!("closed loop {}", i), out.points[i].achieved_m as f64 / 1000.0, *want, 0.02);
    }
    t.eq("the knee of a closed loop: 16 users wait 40 ms", out.configs[0].knee, Some(3));
    t.check("offered is what the users sent", out.points[1].offered_m > 190_000 && out.points[1].offered_m < 205_000);
    // no SLO: no knee, what limits it shows at the last level
    let x = experiment(&exp_rps("100, 500", 4, "").replace(r#""slo": {"p99_ms": 200, "errors_pct": 1}"#, r#""slo": {}"#));
    let Ok(plan) = Plan::new(&x, &p, "x") else {
        t.check("no SLO: a plan", false);
        return;
    };
    let out = plan.run_all();
    t.check("no SLO, no knee", out.configs[0].knee.is_none() && out.points.iter().all(|p| p.slo_ok) && out.configs[0].limit == "CPU WEB");
    // errors break it too: every second one fails
    let e = pair(Some(cores(4)), r#"{"cpu_ms": 1, "max_requests": 0, "error_rate": 0.5, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, LOAD);
    let Ok(plan) = Plan::new(&experiment(&exp_rps("100", 5, "")), &e, "x") else {
        t.check("errors: a plan", false);
        return;
    };
    let out = plan.run_all();
    t.check("half of them fail", !out.points[0].slo_ok && out.points[0].err_ppm > 400_000 && out.points[0].err_ppm < 600_000);
    check_near(t, "achieved counts the good ones", out.points[0].achieved_m as f64 / 1000.0, 50.0, 0.2);
}

// ---------------------------------------------------------------- the figures

fn figures(t: &mut T) {
    // a lock: held all the time at 1 / hold, the CPU far from full
    let p = pair(Some(cores(8)), r#"{"cpu_ms": 0.1, "serial_us": 2000, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, PATIENT);
    let Ok(plan) = Plan::new(&experiment(&exp_rps("250, 600", 6, "")), &p, "x") else {
        t.check("lock: a plan", false);
        return;
    };
    let out = plan.run_all();
    let hi = &out.points[1];
    check_near(t, "a lock's share at half its rate", out.points[0].lock_max_pm as f64, 500.0, 0.02);
    check_near(t, "the throughput of a lock", hi.achieved_m as f64 / 1000.0, 500.0, 0.03);
    check_near(t, "a lock's share at its limit", hi.lock_max_pm as f64, 1000.0, 0.01);
    t.eq("it is what limits", hi.bottleneck.clone(), String::from("Sperre WEB web"));
    t.check("the CPU is not", hi.cpu_max_pm < 100);
    // a disk: the channel's share of time goes with the bytes, its numbers are the ones of the disk
    let mut d = pair(Some(cores(8)), r#"{"cpu_ms": 0.01, "root": "/srv", "max_requests": 0}"#, r#"{"targets": ["http://10.0.0.2/big"], "mode": "open", "rps": 10, "timeout_ms": 3000}"#);
    if let Some(dv) = d.find("WEB").map(|x| x.id).and_then(|id| d.device_mut(id)) {
        let mut f = jfn_model::files::File::default();
        f.path = String::from("/srv/big");
        f.data = vec![b'x'; 1 << 20];
        dv.files.push(f);
    }
    let Ok(plan) = Plan::new(&experiment(&exp_rps("100, 450, 520", 5, "")), &d, "x") else {
        t.check("disk: a plan", false);
        return;
    };
    let out = plan.run_all();
    // 1 MiB at 500 MB/s
    let each = (1u64 << 20) as f64 / 500_000_000.0;
    check_near(t, "a disk's share of time", out.points[0].disk_max_pm as f64, 100.0 * each * 1000.0, 0.02);
    check_near(t, "a disk near its limit", out.points[1].disk_max_pm as f64, 450.0 * each * 1000.0, 0.02);
    t.eq("which disk", out.points[0].disk_dev.clone(), String::from("WEB"));
    t.check("a free channel: no wait", out.details[0].devs.iter().find(|x| x.name == "WEB").is_some_and(|x| x.disk_bytes > 400 << 20));
    t.check("evenly spaced below its limit: no wait", out.points[1].disk_wait_max == 0);
    t.check("beyond it the channel has a backlog", out.points[2].disk_wait_max > MS && out.points[2].disk_backlog_max > MS);
    // a cable at 10 Mbit/s: the share of the line
    let mut c = pair(Some(cores(8)), r#"{"cpu_ms": 0.01, "max_requests": 0, "routes": [{"path": "*", "method": "", "size": 1000}]}"#, LOAD);
    if let Some(l) = c.links.first_mut() {
        l.kbps = 10_000;
    }
    let Ok(plan) = Plan::new(&experiment(&exp_rps("200", 5, "")), &c, "x") else {
        t.check("cable: a plan", false);
        return;
    };
    let out = plan.run_all();
    let fig = &out.details[0];
    // the answers' bytes on the line, as the counters have them
    let bytes = fig.links.first().map_or(0, |l| l.bytes);
    t.check("the frames carry the answers", bytes > 200 * 5 * 1000);
    check_near(t, "a cable's share of the line", fig.links.first().map_or(0, |l| l.util_pm) as f64, bytes as f64 * 8.0 / (5.0 * 10_000_000.0) * 1000.0, 0.01);
    t.check("the cable is named", out.points[0].link_name.contains("GEN") && out.points[0].link_name.contains("WEB"));
    // a run queue and the waits for a core at saturation
    let q = pair(Some(cores(1)), &web("4"), LOAD);
    let Ok(plan) = Plan::new(&experiment(&exp_rps("100, 280", 5, "")), &q, "x") else {
        t.check("queue: a plan", false);
        return;
    };
    let out = plan.run_all();
    t.check("no queue below its limit", out.points[0].runq_milli == 0 && out.points[0].cpu_wait == 0);
    t.check("a queue beyond it", out.points[1].runq_milli > 1000 && out.points[1].cpu_wait > MS);
    t.eq("memory: what the server holds", out.points[0].mem_max >> 20, 32);
    t.eq("no swap", out.points[0].swap_max, 0);
    // the services' own figures come along
    let web = out.details[0].svcs.iter().find(|s| s.kind == "http-server");
    t.check("a service's rate", web.is_some_and(|s| s.rps_milli >= 95_000 && s.rps_milli <= 105_000));
    t.check("the generator's", out.details[0].svcs.iter().any(|s| s.kind == "http-load"));
    t.check("no host among the services", out.details[0].svcs.iter().all(|s| s.svc != "_host"));
    // an unmetered project: no figures of hardware, the rest as ever
    let u = pair(None, r#"{"time_ms": 2, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, LOAD);
    let Ok(plan) = Plan::new(&experiment(&exp_rps("100", 4, "")), &u, "x") else {
        t.check("unmetered: a plan", false);
        return;
    };
    let out = plan.run_all();
    t.check("without hardware", out.details[0].devs.is_empty() && out.points[0].cpu_max_pm == 0);
    check_near(t, "without hardware it takes its time", out.points[0].p50.unwrap_or(0) as f64 / MS as f64, 2.2, 0.15);
    // a project's own experiment runs the same as one from a file
    let mut own = u.clone();
    own.experiments.push(experiment(&exp_rps("100", 4, "")));
    let Ok(plan2) = Plan::new(&own.experiments[0], &own, "x") else {
        t.check("a section: a plan", false);
        return;
    };
    t.eq("a project's section", plan2.run_all().csv(), plan.run_all().csv());
}

// ---------------------------------------------------------------- the quiet trace

fn quiet(t: &mut T) {
    let mut p = pair(Some(cores(2)), &web("2"), LOAD);
    if let Some(d) = p.find("GEN").map(|d| d.id).and_then(|id| p.device_mut(id)) {
        d.services[0].config.insert("arrival", Value::string("poisson"));
    }
    let x = experiment(&exp_rps("200, 900", 3, ""));
    let Ok(mut plan) = Plan::new(&x, &p, "x") else {
        t.check("quiet: a plan", false);
        return;
    };
    let Ok(warm) = plan.warm(0, 0) else {
        t.check("quiet: warm", false);
        return;
    };
    t.eq("a quiet warm engine keeps no frames", (warm.trace.hops().len(), warm.trace.events().len()), (0, 0));
    t.check("but counts them", warm.trace.next_hop_id() > 1);
    let a = plan.run_all();
    plan.quiet = false;
    let Ok(warm) = plan.warm(0, 0) else {
        t.check("loud: warm", false);
        return;
    };
    t.check("with the recording it has them", warm.trace.hops().len() > 100 && warm.trace.events().len() > 100);
    let b = plan.run_all();
    t.eq("a quiet trace changes no figure: the CSV", a.csv(), b.csv());
    t.eq("nor the details", a.detail_csv(), b.detail_csv());
    t.check("it runs the same events", a.details == b.details);
    // a quiet trace keeps its numbers across a clone
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.trace.set_quiet();
    e.run_until(SEC, BUDGET);
    let mut c = e.clone();
    c.run_until(2 * SEC, BUDGET);
    e.run_until(2 * SEC, BUDGET);
    t.eq("a clone of a quiet engine", digest(&c), digest(&e));
}

// ---------------------------------------------------------------- reconfigure

/// the numbers of a service that must go on through a change
fn since_of(e: &Engine, dev: u32, name: &str) -> Option<(jfn_sim::Ns, bool)> {
    e.services(dev).iter().find(|s| s.name == name).map(|s| (s.since, s.running))
}

fn set(c: &Value, key: &str, v: &str) -> Value {
    let mut n = c.clone();
    let _ = set_path(&mut n, key, agent_core::json::parse(v.as_bytes()).unwrap_or(Value::Null));
    n
}

fn reconfigure(t: &mut T) {
    // a web server: more workers at once, and its numbers go on
    let srv = r#"{"workers": 1, "time_ms": 10, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#;
    let p = pair(None, srv, r#"{"targets": ["http://10.0.0.2/"], "mode": "closed", "users": 16, "timeout_ms": 60000}"#);
    let s = p.find("WEB").map_or(0, |d| d.id);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let conf = p.find("WEB").map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    e.run_until(3 * SEC, BUDGET);
    let before = since_of(&e, s, "web");
    let done = e.metric(s, "web", "responses").unwrap_or(0);
    t.check("one worker: 100 per second", (280..=300).contains(&done));
    let new = set(&conf, "workers", "4");
    t.check("it can", e.can_reconfigure(s, "web", &new));
    t.check("asking changes nothing", e.service::<jfn_sim::httpsvc::HttpServer>(s, "web").is_some_and(|h| h.workers == 1));
    t.check("the server takes it", e.apply(Action::ServiceConfig { dev: s, svc: String::from("web"), config: new.clone() }).is_ok());
    t.eq("at once", e.service::<jfn_sim::httpsvc::HttpServer>(s, "web").map(|h| (h.workers, h.queued() + h.busy())), Some((4, 16)));
    t.eq("no restart", since_of(&e, s, "web"), before);
    let at = e.metric(s, "web", "responses").unwrap_or(0);
    t.check("its numbers stay", at >= done);
    e.run_until(4 * SEC, BUDGET);
    let rate = e.metric(s, "web", "responses").unwrap_or(0) - at;
    check_near(t, "four workers: 400 per second", rate as f64, 400.0, 0.03);
    // the time of a request, for the next ones
    let new2 = set(&new, "time_ms", "20");
    t.check("a new time", e.apply(Action::ServiceConfig { dev: s, svc: String::from("web"), config: new2.clone() }).is_ok());
    e.run_until(4020 * MS, BUDGET);
    let at = e.metric(s, "web", "responses").unwrap_or(0);
    e.run_until(5020 * MS, BUDGET);
    check_near(t, "20 ms: 200 per second", (e.metric(s, "web", "responses").unwrap_or(0) - at) as f64, 200.0, 0.03);
    // what it cannot take: nothing changes, it says why
    let refused = |t: &mut T, e: &mut Engine, name: &str, dev: u32, svc: &str, c: Value, why: &str| {
        t.check(&format!("{}: it cannot", name), !e.can_reconfigure(dev, svc, &c));
        let r = e.apply(Action::ServiceConfig { dev, svc: String::from(svc), config: c });
        t.eq(name, r.map_err(|x| x.msg().text()), Err(String::from(why)));
    };
    refused(t, &mut e, "another port", s, "web", set(&new2, "port", "8080"), "Port und Warteschlange der Verbindungen ändern sich nur mit einem Neustart.");
    refused(t, &mut e, "a config it refuses", s, "web", set(&new2, "routes", r#"[{"path": "kein"}]"#), "Der Pfad \u{201e}kein\u{201c} beginnt nicht mit /.");
    t.check("still running as it was", e.service::<jfn_sim::httpsvc::HttpServer>(s, "web").is_some_and(|h| h.workers == 4 && h.cfg().port == 80 && h.routes[0].path == "*"));
    // a stopped server changes too, and starts with it
    t.check("stop", e.apply(Action::ServiceRun { dev: s, name: String::from("web"), on: false }).is_ok());
    t.check("a stopped one takes it", e.apply(Action::ServiceConfig { dev: s, svc: String::from("web"), config: set(&new2, "workers", "2") }).is_ok());
    t.eq("and has it", e.service::<jfn_sim::httpsvc::HttpServer>(s, "web").map(|h| h.workers), Some(2));
    // the load generator: another rate from now on, without a restart
    let q = pair(None, &web("0"), LOAD);
    let (g2, _) = (q.find("GEN").map_or(0, |d| d.id), q.find("WEB").map_or(0, |d| d.id));
    let lconf = q.find("GEN").map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    let mut e2 = Engine::from_project(&q).unwrap_or_else(|_| Engine::new(0));
    e2.run_until(2 * SEC, BUDGET);
    let before = since_of(&e2, g2, "load");
    let sent = e2.metric(g2, "load", "sent").unwrap_or(0);
    t.check("a higher rate", e2.apply(Action::ServiceConfig { dev: g2, svc: String::from("load"), config: set(&lconf, "rps", "500") }).is_ok());
    t.eq("no restart of the generator", since_of(&e2, g2, "load"), before);
    e2.run_until(3 * SEC, BUDGET);
    check_near(t, "500 per second", (e2.metric(g2, "load", "sent").unwrap_or(0) - sent) as f64, 500.0, 0.01);
    refused(t, &mut e2, "another kind of load", g2, "load", set(&lconf, "mode", r#""closed""#), "Offene und geschlossene Last wechseln nur mit einem Neustart.");
    refused(t, &mut e2, "another duration", g2, "load", set(&lconf, "duration_ms", "1000"), "Die Dauer eines Laufs ändert sich nur mit einem Neustart.");
    // a load balancer: a backend more, its algorithm, one that drains; the same connection numbers
    let n = lb_net();
    let (lb, w2) = (n.find("LB").map_or(0, |d| d.id), n.find("W2").map_or(0, |d| d.id));
    let bconf = n.find("LB").map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    let mut e3 = Engine::from_project(&n).unwrap_or_else(|_| Engine::new(0));
    e3.run_until(2 * SEC, BUDGET);
    let before = since_of(&e3, lb, "lb");
    let two = set(&set(&bconf, "backends", r#"[{"name": "W1", "addr": "10.0.0.11"}, {"name": "W2", "addr": "10.0.0.12"}]"#), "algorithm", r#""least-conn""#);
    t.check("a backend more", e3.apply(Action::ServiceConfig { dev: lb, svc: String::from("lb"), config: two.clone() }).is_ok());
    t.eq("no restart of the balancer", since_of(&e3, lb, "lb"), before);
    t.check("its algorithm", e3.lb_view(lb, "lb").is_some_and(|v| v.algo == jfn_sim::Algo::LeastConn && v.backends.len() == 2));
    e3.run_until(4 * SEC, BUDGET);
    t.eq("the new one takes load at once", e3.metric(w2, "web", "requests").unwrap_or(0), 10);
    t.check("one gone: it drains", e3.apply(Action::ServiceConfig { dev: lb, svc: String::from("lb"), config: set(&bconf, "backends", r#"[{"name": "W1", "addr": "10.0.0.11"}]"#) }).is_ok());
    let at = e3.metric(w2, "web", "requests").unwrap_or(0);
    e3.run_until(6 * SEC, BUDGET);
    t.check("it gets nothing new", e3.metric(w2, "web", "requests").unwrap_or(0) <= at + 1);
    refused(t, &mut e3, "another port of a balancer", lb, "lb", set(&bconf, "port", "81"), "Modus und Port eines Load Balancers ändern sich nur mit einem Neustart.");
    refused(t, &mut e3, "another address of a backend", lb, "lb", set(&bconf, "backends", r#"[{"name": "W1", "addr": "10.0.0.12"}]"#), "Das Backend W1 hat eine andere Adresse: es braucht einen anderen Namen.");
    // a cache: its time to live for what comes next, what it holds stays
    let mut cn = pair(None, r#"{"max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, r#"{"targets": ["http://10.0.0.1:8080/"], "mode": "open", "rps": 20, "timeout_ms": 3000}"#);
    if let Some(d) = cn.find("GEN").map(|d| d.id).and_then(|id| cn.device_mut(id)) {
        d.services.push(def("cache", "cache", r#"{"port": 8080, "backend": "http://10.0.0.2/", "ttl_ms": 100000}"#));
    }
    let gc = cn.find("GEN").map_or(0, |d| d.id);
    let cconf = cn.find("GEN").map(|d| d.services[1].config.clone()).unwrap_or(Value::Null);
    let mut e4 = Engine::from_project(&cn).unwrap_or_else(|_| Engine::new(0));
    e4.run_until(2 * SEC, BUDGET);
    let before = since_of(&e4, gc, "cache");
    let entries = e4.metric(gc, "cache", "entries").unwrap_or(0);
    t.check("a cache holds an answer", entries == 1);
    t.check("a cache takes it", e4.apply(Action::ServiceConfig { dev: gc, svc: String::from("cache"), config: set(&cconf, "ttl_ms", "50") }).is_ok());
    t.eq("no restart of the cache", since_of(&e4, gc, "cache"), before);
    t.eq("what it holds stays", e4.metric(gc, "cache", "entries").unwrap_or(0), entries);
    t.check("the new time counts", e4.cache_view(gc, "cache").is_some_and(|v| v.ttl == 50 * MS));
    refused(t, &mut e4, "another backend of a cache", gc, "cache", set(&cconf, "backend", r#""http://10.0.0.9/""#), "Port und Backend eines Caches ändern sich nur mit einem Neustart.");
    // a worker: more slots at once
    let mut wn = pair(None, r#"{"max_requests": 0}"#, LOAD);
    if let Some(d) = wn.find("WEB").map(|d| d.id).and_then(|id| wn.device_mut(id)) {
        d.services = vec![def("mq", "mq", r#"{"port": 9000}"#), def("worker", "worker", r#"{"queue": "http://10.0.0.2:9000", "topic": "jobs", "concurrency": 1, "time_ms": 50}"#)];
    }
    if let Some(d) = wn.find("GEN").map(|d| d.id).and_then(|id| wn.device_mut(id)) {
        d.services = vec![def("http-load", "load", r#"{"targets": ["http://10.0.0.2:9000/topics/jobs"], "mode": "open", "rps": 100, "timeout_ms": 3000, "mix": [{"method": "POST", "path": "/topics/jobs", "body": "x"}]}"#)];
    }
    let wd = wn.find("WEB").map_or(0, |d| d.id);
    let wconf = wn.find("WEB").map(|d| d.services[1].config.clone()).unwrap_or(Value::Null);
    let mut e5 = Engine::from_project(&wn).unwrap_or_else(|_| Engine::new(0));
    e5.run_until(3 * SEC, BUDGET);
    let before = since_of(&e5, wd, "worker");
    let done = e5.metric(wd, "worker", "done").unwrap_or(0);
    t.check("one slot: 20 per second", (40..=60).contains(&done));
    t.check("a worker takes it", e5.apply(Action::ServiceConfig { dev: wd, svc: String::from("worker"), config: set(&wconf, "concurrency", "4") }).is_ok());
    t.eq("no restart of the worker", since_of(&e5, wd, "worker"), before);
    t.check("more slots at once", e5.worker_view(wd, "worker").is_some_and(|v| v.concurrency == 4 && v.slots.len() == 4));
    let at = e5.metric(wd, "worker", "done").unwrap_or(0);
    e5.run_until(4 * SEC, BUDGET);
    check_near(t, "four slots: 80 per second", (e5.metric(wd, "worker", "done").unwrap_or(0) - at) as f64, 80.0, 0.1);
    refused(t, &mut e5, "another queue of a worker", wd, "worker", set(&wconf, "topic", r#""x""#), "Warteschlange und Thema eines Workers ändern sich nur mit einem Neustart.");
    // any other service cannot, by default
    let mut en = pair(None, &web("0"), LOAD);
    if let Some(d) = en.find("WEB").map(|d| d.id).and_then(|id| en.device_mut(id)) {
        d.services.push(def("tcp-echo", "echo", r#"{"port": 7}"#));
    }
    let wd = en.find("WEB").map_or(0, |d| d.id);
    let mut e6 = Engine::from_project(&en).unwrap_or_else(|_| Engine::new(0));
    e6.run_until(SEC, BUDGET);
    refused(t, &mut e6, "a service without it", wd, "echo", Value::object([("port", Value::u64(7))]), "Dieser Dienst ändert seine Einstellungen nur mit einem Neustart.");
    t.check("no such service", e6.apply(Action::ServiceConfig { dev: wd, svc: String::from("nix"), config: Value::Null }).is_err_and(|x| x.msg().text() == "Den Dienst \u{201e}nix\u{201c} gibt es nicht."));
}

// ---------------------------------------------------------------- what the window does

fn window(t: &mut T) {
    let a = def("http-server", "web", r#"{"port": 80, "workers": 1}"#);
    let b = def("http-server", "web", r#"{"port": 80, "workers": 2}"#);
    let c = def("tcp-echo", "echo", r#"{"port": 7}"#);
    let yes = |_: &ServiceDef| true;
    let no = |_: &ServiceDef| false;
    let names = |v: Option<Vec<Action>>| v.map(|l| l.iter().map(|x| String::from(format!("{:?}", x).split([' ', '{', '(']).next().unwrap_or(""))).collect::<Vec<String>>());
    t.eq("a new config: no restart", names(live_steps(1, &[a.clone(), c.clone()], &[b.clone(), c.clone()], &yes)), Some(vec![String::from("ServiceConfig")]));
    t.eq("one it cannot take: a restart", names(live_steps(1, &[a.clone()], &[b.clone()], &no)), None);
    t.eq("a service more: a restart", names(live_steps(1, &[a.clone()], &[a.clone(), c.clone()], &yes)), None);
    t.eq("another name: a restart", names(live_steps(1, &[a.clone()], &[def("http-server", "www", r#"{"port": 80, "workers": 1}"#)], &yes)), None);
    let mut off = a.clone();
    off.disabled = true;
    t.eq("switched", names(live_steps(1, &[a.clone()], &[off], &yes)), Some(vec![String::from("ServiceRun")]));
    t.eq("nothing changed", names(live_steps(1, &[a.clone()], &[a.clone()], &yes)), Some(Vec::new()));
    // the window: a new config of a web server keeps its numbers, one with another port restarts
    let p = pair(None, r#"{"workers": 1, "time_ms": 1, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, LOAD);
    let s = p.find("WEB").map_or(0, |d| d.id);
    let mut sim = crate::doc::sim::Sim::new(&p);
    let _ = sim.send(Cmd::RunFor(2 * SEC));
    let num = |sim: &crate::doc::sim::Sim| sim.engine().metric(s, "web", "requests").unwrap_or(0);
    let before = num(&sim);
    t.check("it has served some", before > 10);
    let mut q = p.clone();
    if let Some(d) = q.device_mut(s) {
        d.services[0].config = set(&d.services[0].config, "workers", "3");
    }
    sim.sync(&q);
    t.eq("the window: its numbers stay", num(&sim), before);
    t.eq("the window: it has the new setting", sim.engine().service::<jfn_sim::httpsvc::HttpServer>(s, "web").map(|h| h.workers), Some(3));
    sim.sync(&p);
    t.eq("undo: back again, without a restart", (num(&sim), sim.engine().service::<jfn_sim::httpsvc::HttpServer>(s, "web").map(|h| h.workers)), (before, Some(1)));
    let mut r = p.clone();
    if let Some(d) = r.device_mut(s) {
        d.services[0].config = set(&d.services[0].config, "port", "8080");
    }
    sim.sync(&r);
    t.eq("another port: the device starts again", (num(&sim), sim.engine().service::<jfn_sim::httpsvc::HttpServer>(s, "web").map(|h| h.cfg().port)), (0, Some(8080)));
}

// ---------------------------------------------------------------- the steps of a scenario

fn steps(t: &mut T) {
    let kind = |line: &str| scenario::parse(line).map(|s| s.steps.first().map(|x| x.kind.clone())).map_err(|e| e.first().map(|x| x.text.clone()));
    let s = String::from;
    t.eq("load set rps", kind("load set GEN load rps 200.5"), Ok(Some(StepKind::LoadSet { dev: s("GEN"), svc: s("load"), rate_m: Some(200_500), users: None })));
    t.eq("load set users", kind("load set GEN load users 40"), Ok(Some(StepKind::LoadSet { dev: s("GEN"), svc: s("load"), rate_m: None, users: Some(40) })));
    t.eq("lb add", kind("lb add LB lb web3 10.0.2.13 port 8080 weight 2"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Add { name: s("web3"), addr: s("10.0.2.13"), port: 8080, weight: 2 } })));
    t.eq("lb add by default", kind("lb add LB lb web3 10.0.2.13"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Add { name: s("web3"), addr: s("10.0.2.13"), port: 80, weight: 1 } })));
    t.eq("lb drain", kind("lb drain LB lb web3"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Drain(s("web3")) })));
    t.eq("lb remove", kind("lb remove LB lb web3"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Remove(s("web3")) })));
    t.eq("lb enable", kind("lb enable LB lb web3"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Enable(s("web3")) })));
    t.eq("lb weight", kind("lb weight LB lb web3 3"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Weight(s("web3"), 3) })));
    t.eq("lb algo", kind("lb algo LB lb hash"), Ok(Some(StepKind::Lb { dev: s("LB"), svc: s("lb"), op: LbStep::Algo(s("hash")) })));
    t.eq("kv set", kind("kv set KV1 kv consistency eventual w 1 r 2"), Ok(Some(StepKind::KvSet { dev: s("KV1"), svc: s("kv"), consistency: Some(s("eventual")), w: Some(1), r: Some(2) })));
    t.eq("svc set", kind("svc set WEB web workers 8 time_ms 2.5"), Ok(Some(StepKind::SvcSet { dev: s("WEB"), svc: s("web"), sets: vec![(s("workers"), s("8")), (s("time_ms"), s("2.5"))] })));
    t.eq("cache clear", kind("cache clear C cache"), Ok(Some(StepKind::CacheClear { dev: s("C"), svc: s("cache") })));
    t.eq("at a time", scenario::parse("at 5s load set GEN load users 4").map(|x| x.steps[0].at).map_err(|_| ()), Ok(Some(5 * SEC)));
    t.eq("no such word", kind("load set GEN load gold 3"), Err(Some(s("\u{201e}gold\u{201c} versteht load set nicht (rps, users)."))));
    t.eq("no rate", kind("load set GEN load rps viel"), Err(Some(s("\u{201e}viel\u{201c} ist keine Rate."))));
    t.eq("lb add with a wrong word", kind("lb add LB lb w 1.2.3.4 prio 1"), Err(Some(s("\u{201e}prio\u{201c} versteht lb add nicht (port, weight)."))));
    t.eq("svc set without pairs", kind("svc set WEB web workers"), Err(Some(s("svc set braucht Paare wie \u{201e}workers 8\u{201c}."))));
    t.eq("kv set without pairs", kind("kv set KV kv"), Err(Some(s("kv set braucht Paare wie \u{201e}consistency eventual\u{201c}, \u{201e}w 2\u{201c}, \u{201e}r 1\u{201c}."))));
    // a device called so stays a device: its command lines work as ever
    t.eq("a device named lb", kind("lb: ip addr"), Ok(Some(StepKind::Cli { dev: s("lb"), cmd: s("ip addr") })));
    t.eq("a device named load", kind("load: ip addr"), Ok(Some(StepKind::Cli { dev: s("load"), cmd: s("ip addr") })));
    // they run: what each refuses comes as a line of the report
    let p = pair(None, r#"{"workers": 1, "time_ms": 1, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, LOAD);
    let sc = |text: &str| scenario::parse(text).unwrap_or_default();
    let out = jfn_sim::run_scenario(&p, &sc("run 1s\nload set GEN load rps 100\nrun 1s\nsvc set WEB web workers 2\nsvc set WEB web time_ms 3\nexpect WEB svc web metric requests >= 50\n"), "x", false);
    t.check("the steps run", out.ok && out.report.contains("load set GEN load rps 100.000") && out.report.contains("svc set WEB web workers 2") && out.report.contains("svc set WEB web time_ms 3"));
    let bad = |t: &mut T, step: &str, want: &str| {
        let out = jfn_sim::run_scenario(&p, &sc(&format!("run 100ms\n{}\n", step)), "x", false);
        t.check(&format!("{}: {}", step, want), !out.ok && out.report.contains(want));
    };
    bad(t, "load set NIX load rps 1", "das Gerät NIX gibt es nicht");
    bad(t, "load set GEN nix rps 1", "Den Dienst \u{201e}nix\u{201c} gibt es nicht.");
    bad(t, "lb algo GEN load hash", "ist von anderer Art");
    bad(t, "svc set WEB web port 81", "Port und Warteschlange der Verbindungen ändern sich nur mit einem Neustart.");
    bad(t, "svc set WEB nix a 1", "WEB hat keinen Dienst nix");
    bad(t, "kv set WEB web consistency fast", "\u{201e}fast\u{201c} kennt kv set nicht");
    bad(t, "cache clear WEB web", "ist von anderer Art");
    let lb = lb_net();
    let out = jfn_sim::run_scenario(&lb, &sc("run 100ms\nlb algo LB lb magic\n"), "x", false);
    t.check("an algorithm that is none", !out.ok && out.report.contains("den Algorithmus magic gibt es nicht"));
    let out = jfn_sim::run_scenario(&lb, &sc("run 100ms\nlb add LB lb W2 nirgends\n"), "x", false);
    t.check("an address that is none", !out.ok && out.report.contains("nirgends ist keine IP-Adresse"));
}

// ---------------------------------------------------------------- rewinding replays them

fn replay(t: &mut T) {
    let lb = lb_net();
    let (ld, wd, gd) = (lb.find("LB").map_or(0, |d| d.id), lb.find("W1").map_or(0, |d| d.id), lb.find("GEN").map_or(0, |d| d.id));
    let conf = lb.find("W1").map(|d| d.services[0].config.clone()).unwrap_or(Value::Null);
    let mut drv = Inline::new(Engine::from_project(&lb).unwrap_or_else(|_| Engine::new(0)));
    drv.rewind_on(Budget { gap_events: 2000, ..Budget::default() });
    let _ = drv.send(Cmd::RunFor(2 * SEC), 0);
    let actions = vec![
        Action::HttpLoadSet { dev: gd, svc: String::from("load"), rate_m: Some(80_000), users: None },
        Action::LbBackend { dev: ld, svc: String::from("lb"), op: jfn_sim::LbOp::Add { name: String::from("W2"), addr: netpkt::IpAddr::parse("10.0.0.12").unwrap_or(netpkt::IpAddr::V4(netpkt::Ipv4::UNSPECIFIED)), port: 80, weight: 1 } },
        Action::LbAlgorithm { dev: ld, svc: String::from("lb"), algo: jfn_sim::Algo::LeastConn },
        Action::ServiceConfig { dev: wd, svc: String::from("web"), config: set(&conf, "time_ms", "2") },
        Action::LbBackend { dev: ld, svc: String::from("lb"), op: jfn_sim::LbOp::Drain { name: String::from("W2") } },
    ];
    for a in actions {
        t.check(&format!("{:?}", a).chars().take(40).collect::<String>(), drv.send(Cmd::Apply(a), 0).is_ok());
        let _ = drv.send(Cmd::RunFor(SEC), 0);
    }
    let pos = drv.pos();
    let Some(rw) = drv.rewind() else {
        t.check("a recording", false);
        return;
    };
    let got = drv.restore_pos(pos, None).map(|mut r| {
        let _ = r.run(rw, usize::MAX);
        r.finish()
    });
    t.eq("a replay from keyframes: the same engine", got.map(|p| (p.pos, digest(&p.eng))), Some((pos, digest(&drv.eng))));
    // and from a moment between the changes
    let mid = drv.restore(3500 * MS, None).map(|mut r| {
        let _ = r.run(rw, usize::MAX);
        r.finish()
    });
    t.check("a moment between them", mid.is_some_and(|p| p.pos.t <= 3500 * MS && p.eng.service::<jfn_sim::httpsvc::HttpServer>(wd, "web").is_some_and(|h| h.time == 0)));
}

// ---------------------------------------------------------------- the heap under the threads

/// lib/rt's heaps by thread: what one thread frees comes back to another that allocates, or a thread
/// that only frees would let the memory grow without limit (blocks of a size that fit 64 KB chunks)
fn allocator(t: &mut T) {
    const N: usize = 200_000;
    let mut seen: alloc::collections::BTreeSet<usize> = alloc::collections::BTreeSet::new();
    let (mut again, mut rounds) = (0, 0);
    for _ in 0..4 {
        let blocks: Vec<alloc::boxed::Box<[u8; 64]>> = (0..N).map(|i| alloc::boxed::Box::new([(i % 251) as u8; 64])).collect();
        again = blocks.iter().filter(|b| seen.contains(&(b.as_ptr() as usize))).count();
        seen.extend(blocks.iter().map(|b| b.as_ptr() as usize));
        t.check("the blocks hold what was put there", blocks.iter().enumerate().all(|(i, b)| b.iter().all(|x| *x == (i % 251) as u8)));
        // another thread frees them all
        match plat::thread_spawn(1 << 20, alloc::boxed::Box::new(move || drop(blocks))) {
            Some(h) => h.join(),
            None => return,
        }
        rounds += 1;
    }
    t.eq("four rounds", rounds, 4);
    // after the first round nearly all come back; new chunks only for what a heap holds at the moment
    check_near(t, "what another thread freed comes back", again as f64, N as f64, 0.05);
    t.check(&format!("the memory stays what one round needs ({} blocks seen)", seen.len()), seen.len() < N + N / 4);
    // many at once, each its own sizes
    let mut hs = Vec::new();
    for k in 1..=6usize {
        hs.push(plat::thread_spawn(1 << 20, alloc::boxed::Box::new(move || {
            for r in 0..2000usize {
                let v: Vec<Vec<u8>> = (0..50).map(|i| vec![(k + i) as u8; 8 + (i * k * 37 + r) % 3000]).collect();
                for (i, x) in v.iter().enumerate() {
                    assert!(x.iter().all(|b| *b == (k + i) as u8));
                }
            }
        })));
    }
    let n = hs.len();
    let mut joined = 0;
    for h in hs.into_iter().flatten() {
        h.join();
        joined += 1;
    }
    t.eq("threads with sizes of their own: all ended", joined, n);
}
