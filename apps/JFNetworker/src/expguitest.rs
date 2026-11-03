// expguitest -- the lab in the window (part of --selftest, group expgui): every form against the JSON it
// writes, unknown members kept, a refusal with its exact words, the defaults as the services say them;
// live or restart as predicted, for each kind, and what then happens; presets through placing and undo,
// a loaded file that gets nothing; runs with progress and cancel, their results the same bytes as
// jfn_run::run_sweep at 1 and N threads; kept runs through saving and loading, old files byte for byte; the
// bounds of what a project keeps; the warning for overload with a short timeout; the CSV; the bars of the
// Metriken panel
//
// What takes real threads waits for them with a wall clock of its own: a job has to end by itself.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::{empty_object, Value};
use agent_core::json;
use jfn_model::experiment::{Experiment, KEEP_POINTS, KEEP_RUNS};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_run::{run_sweep, run_sweep_ctl, SweepCtl, SweepJob, SweepOut};
use jfn_sim::experiment::{Plan, Sweep};
use jfn_sim::svcform::{defaults, effective, missing, port_of, KINDS};
use jfn_sim::{Engine, SEC};

use crate::doc::{edits, Doc};
use crate::selftest::T;
use crate::workspace::devwin::svcedit::trial;
use crate::workspace::devwin::svcform::{default_text, fields, fill, form_of, free_name, merged, takes, Fk, Row, Takes};
use crate::workspace::experiments::{export_text, keep_run, parse_numbers, parse_values};
use crate::workspace::metrics::rebin;

fn cfg(text: &str) -> Value {
    json::parse(text.as_bytes()).unwrap_or(Value::Null)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: cfg(config) }
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

/// a config after these fields changed in its form, as compact JSON
fn after(kind: &str, base: &str, sets: &[(&str, &str)]) -> Result<String, String> {
    let b = cfg(base);
    let mut v = fill(kind, &b);
    for (k, x) in sets {
        assert_key(&mut v, kind, k, x);
    }
    merged(kind, &b, &v).map(|c| json::stringify(&c).unwrap_or_default())
}

fn assert_key(v: &mut crate::workspace::devwin::svcform::Vals, kind: &str, k: &str, x: &str) {
    if !v.set(kind, k, x) {
        panic_free_note(kind, k);
    }
}

// a key the form does not have shows as a wrong result in the comparison that follows
fn panic_free_note(kind: &str, k: &str) {
    plat::write_stdout(format!("expgui: {} has no field {}\n", kind, k).as_bytes());
}

const WEB: &str = r#"{"port": 80, "cpu_ms": 1, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#;

/// a generator, a switch, a balancer, two web servers, a queue, a worker and a cache on one network
fn lab() -> Project {
    let mut p = Project::new("expgui", 3);
    let sw = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let host = |p: &mut Project, model: &str, name: &str, ip: &str, port: &str, k: u32, svc: Vec<ServiceDef>| -> u32 {
        let id = p.add_device(model, 100 * k as i32, 200).unwrap_or(0);
        let swp = format!("Gi1/0/{}", k);
        let _ = p.connect(&pr(id, port), &pr(sw, &swp), None);
        if let Some(d) = p.device_mut(id) {
            d.name = String::from(name);
            d.startup_config = format!("ip addr add {}/24 dev {}\n", ip, port);
            d.services = svc;
        }
        id
    };
    let port = "eth1";
    host(&mut p, "traffic-gen", "GEN", "10.0.0.1", "eth0", 1, vec![def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 20, "timeout_ms": 30000, "mix": [{"path": "/", "weight": 1}]}"#)]);
    host(&mut p, "load-balancer", "LB", "10.0.0.2", "eth1", 2, vec![def("lb", "lb", r#"{"backends": [{"name": "WEB1", "addr": "10.0.0.3", "port": 80, "note": "bleibt"}, {"name": "WEB2", "addr": "10.0.0.4"}], "health": {"type": "none", "extra": 1}, "custom": "bleibt"}"#)]);
    host(&mut p, "server-1u", "WEB1", "10.0.0.3", port, 3, vec![def("http-server", "web", WEB)]);
    host(&mut p, "server-1u", "WEB2", "10.0.0.4", port, 4, vec![def("http-server", "web", WEB)]);
    host(&mut p, "database", "KV1", "10.0.0.5", "eth1", 5, vec![def("kv", "kv", r#"{"nodes": ["10.0.0.5"], "rf": 1}"#)]);
    host(&mut p, "queue", "MQ", "10.0.0.6", "eth1", 6, vec![def("mq", "mq", r#"{"topics": ["jobs"]}"#)]);
    host(&mut p, "server-1u", "WRK", "10.0.0.7", port, 7, vec![def("worker", "worker", r#"{"queue": "http://10.0.0.6:9000", "time_ms": 5}"#)]);
    host(&mut p, "cache", "CA", "10.0.0.8", "eth1", 8, vec![def("cache", "cache", r#"{"backend": "http://10.0.0.3/"}"#)]);
    p
}

pub(crate) fn run(t: &mut T) {
    forms(t);
    defaults_asked(t);
    refusals(t);
    live_or_restart(t);
    presets(t);
    problems_named(t);
    runs(t);
    kept(t);
    overload(t);
    fields_of_lists(t);
    bars(t);
}

// ---------------------------------------------------------------- the forms and their JSON

fn forms(t: &mut T) {
    t.check("every kind of the dialog has a form", KINDS.iter().all(|k| form_of(k).is_some()));
    t.check("the others keep the JSON", form_of("script").is_none() && form_of("dns-server").is_none());
    for k in KINDS {
        // a form changes nothing by itself, whatever stands there
        let base = cfg(r#"{"zzz": [1, 2], "port": 8080}"#);
        t.eq(&format!("{}: nothing typed, nothing changes", k), merged(k, &base, &fill(k, &base)).ok(), Some(base.clone()));
        // every field of the form has help and a label
        t.check(&format!("{}: every field has its label and help", k), fields(k).all(|f| !f.label.is_empty() && !f.help.is_empty()));
        // every member it shows is one the services know, so that the experiments' check agrees
        let known = jfn_model::experiment::known_keys(k).unwrap_or(&[]);
        for f in fields(k) {
            let first = f.key.split('.').next().unwrap_or("");
            t.check(&format!("{}: {} is known", k, f.key), known.contains(&first));
        }
    }
    // what each writes, the other members staying as and where they are
    t.eq("http-server: numbers", after("http-server", r#"{"zzz": 1, "port": 80, "routes": []}"#, &[("port", "8080"), ("workers", "4"), ("cpu_ms", "2,5")]), Ok(String::from(r#"{"zzz":1,"port":8080,"routes":[],"cpu_ms":2.5,"workers":4}"#)));
    t.eq("http-server: an empty field takes the member out", after("http-server", r#"{"port": 80, "workers": 4}"#, &[("workers", "")]), Ok(String::from(r#"{"port":80}"#)));
    t.eq("http-server: a share as percent", after("http-server", "{}", &[("error_rate", "2,5")]), Ok(String::from(r#"{"error_rate":"2.5%"}"#)));
    t.eq("http-server: a share reads as percent", fill("http-server", &cfg(r#"{"error_rate": 0.05}"#)).texts[6].as_str(), "5");
    t.eq("http-server: what is not typed keeps its spelling", after("http-server", r#"{"time_ms": 5.0, "workers": 1}"#, &[("workers", "2")]), Ok(String::from(r#"{"time_ms":5.0,"workers":2}"#)));
    t.eq("http-load: its choices", after("http-load", r#"{"targets": ["http://1.2.3.4/"], "mix": [{"path": "/"}]}"#, &[("mode", "open"), ("rps", "250"), ("keepalive", "false")]), Ok(String::from(r#"{"targets":["http://1.2.3.4/"],"mix":[{"path":"/"}],"mode":"open","rps":250,"keepalive":false}"#)));
    t.eq("http-load: back to the default", after("http-load", r#"{"mode": "open"}"#, &[("mode", "")]), Ok(String::from("{}")));
    t.eq("lb: inside health", after("lb", r#"{"health": {"type": "http", "extra": 1}}"#, &[("health.path", "/ok"), ("health.type", "")]), Ok(String::from(r#"{"health":{"extra":1,"path":"/ok"}}"#)));
    t.eq("lb: an empty health goes", after("lb", r#"{"port": 80, "health": {"type": "http"}}"#, &[("health.type", "")]), Ok(String::from(r#"{"port":80}"#)));
    t.eq("kv: its settings", after("kv", r#"{"nodes": ["10.0.0.1"]}"#, &[("rf", "3"), ("consistency", "eventual")]), Ok(String::from(r#"{"nodes":["10.0.0.1"],"rf":3,"consistency":"eventual"}"#)));
    t.eq("cache: its backend", after("cache", r#"{"ttl_ms": 100}"#, &[("backend", "http://10.0.0.3/")]), Ok(String::from(r#"{"ttl_ms":100,"backend":"http://10.0.0.3/"}"#)));
    t.eq("mq: a list of words", after("mq", "{}", &[("topics", "a, b ,c")]), Ok(String::from(r#"{"topics":["a","b","c"]}"#)));
    t.eq("mq: reads as words", fill("mq", &cfg(r#"{"topics": ["a", "b"]}"#)).texts[1].as_str(), "a, b");
    t.eq("worker: its queue and topic", after("worker", r#"{"cpu_ms": 1}"#, &[("queue", "http://10.0.0.6:9000"), ("topic", "x")]), Ok(String::from(r#"{"cpu_ms":1,"queue":"http://10.0.0.6:9000","topic":"x"}"#)));
    t.eq("no number", after("http-server", "{}", &[("port", "acht")]).err().as_deref(), Some("Port: das ist keine ganze Zahl."));
    t.eq("no decimal", after("http-server", "{}", &[("cpu_ms", "-1")]).err().as_deref(), Some("CPU je Anfrage: das ist keine Zahl."));
    // the free names of the dialog
    let mut p = Project::new("n", 1);
    let id = p.add_device("server-1u", 0, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(id) {
        t.eq("a new web server", free_name(d, "http-server"), String::from("web"));
        d.services.push(def("http-server", "WEB", "{}"));
        t.eq("the next one", free_name(d, "http-server"), String::from("web2"));
    }
}

fn fields_of_lists(t: &mut T) {
    // the rows of a list, unknown members of a backend kept
    let base = cfg(r#"{"backends": [{"name": "a", "addr": "10.0.0.1", "weight": 2, "x": 1}, {"addr": "10.0.0.2"}], "y": 2}"#);
    let mut v = fill("lb", &base);
    t.eq("two rows", v.rows.len(), 2);
    t.eq("its fields", (v.rows[0].name.as_str(), v.rows[0].weight.as_str(), v.rows[1].addr.as_str()), ("a", "2", "10.0.0.2"));
    v.rows[0].weight = String::from("3");
    v.rows.push(Row { name: String::from("c"), addr: String::from("10.0.0.3"), port: String::from("8080"), ..Row::default() });
    v.rows.remove(1);
    t.eq("rows into the JSON", merged("lb", &base, &v).map(|c| json::stringify(&c).unwrap_or_default()), Ok(String::from(r#"{"backends":[{"name":"a","addr":"10.0.0.1","weight":3,"x":1},{"name":"c","addr":"10.0.0.3","port":8080}],"y":2}"#)));
    let mut v = fill("lb", &base);
    v.rows.clear();
    t.eq("no rows, no backends", merged("lb", &base, &v).map(|c| json::stringify(&c).unwrap_or_default()), Ok(String::from(r#"{"y":2}"#)));
    let mut v = fill("lb", &base);
    v.rows[1].port = String::from("achtzig");
    t.eq("a port of a row", merged("lb", &base, &v).err().as_deref(), Some("achtzig: das ist keine ganze Zahl."));
    // a target as "target" becomes the list only if the rows change
    let one = cfg(r#"{"target": "http://1.1.1.1/", "users": 2}"#);
    let mut v = fill("http-load", &one);
    t.eq("target reads as a row", v.rows[0].addr.as_str(), "http://1.1.1.1/");
    t.eq("it stays while the rows do", merged("http-load", &one, &v).ok(), Some(one.clone()));
    v.rows.push(Row { addr: String::from("http://2.2.2.2/"), ..Row::default() });
    t.eq("targets when they change", merged("http-load", &one, &v).map(|c| json::stringify(&c).unwrap_or_default()), Ok(String::from(r#"{"users":2,"targets":["http://1.1.1.1/","http://2.2.2.2/"]}"#)));
    let two = cfg(r#"{"nodes": ["10.0.0.1"]}"#);
    let mut v = fill("kv", &two);
    v.rows.push(Row { addr: String::from("10.0.0.2"), ..Row::default() });
    v.rows.push(Row::default());
    t.eq("nodes, without an empty row", merged("kv", &two, &v).map(|c| json::stringify(&c).unwrap_or_default()), Ok(String::from(r#"{"nodes":["10.0.0.1","10.0.0.2"]}"#)));
    // the field kinds are what the tables say
    t.check("a form has one list at most", KINDS.iter().all(|k| fields(k).filter(|f| f.kind == Fk::List).count() <= 1));
}

// ---------------------------------------------------------------- one truth for defaults

fn defaults_asked(t: &mut T) {
    let http = defaults("http-server").unwrap_or_else(empty_object);
    t.eq("a web server's port", http.get("port").and_then(|v| v.as_u64()), Some(80));
    t.eq("its keep-alive", http.get("keepalive_ms").and_then(|v| v.as_u64()), Some(75_000));
    t.eq("its CPU", http.get("cpu_ms").map(|v| json::stringify(v).unwrap_or_default()), Some(String::from("0.02")));
    let load = defaults("http-load").unwrap_or_else(empty_object);
    t.eq("a generator's mode", load.get("mode").and_then(|v| v.as_str()), Some("closed"));
    t.eq("its time limit", load.get("timeout_ms").and_then(|v| v.as_u64()), Some(5000));
    let lb = defaults("lb").unwrap_or_else(empty_object);
    t.eq("a balancer's algorithm", lb.get("algorithm").and_then(|v| v.as_str()), Some("round-robin"));
    t.eq("its health checks", lb.get("health").and_then(|h| h.get("type")).and_then(|v| v.as_str()), Some("http"));
    t.eq("a KV store's copies", defaults("kv").and_then(|v| v.get("rf").and_then(|x| x.as_u64())), Some(1));
    t.eq("a queue's port", defaults("mq").and_then(|v| v.get("port").and_then(|x| x.as_u64())), Some(9000));
    t.eq("a cache's entries", defaults("cache").and_then(|v| v.get("entries").and_then(|x| x.as_u64())), Some(1000));
    t.eq("a worker's slots", defaults("worker").and_then(|v| v.get("concurrency").and_then(|x| x.as_u64())), Some(1));
    // what a config says wins
    t.eq("a config's own", effective("mq", &cfg(r#"{"port": 1234}"#)).ok().and_then(|v| v.get("port").and_then(|x| x.as_u64())), Some(1234));
    t.eq("port of a service", port_of(&def("mq", "q", "{}")), Some(9000));
    t.eq("port of a balancer that lacks backends", port_of(&def("lb", "b", r#"{"port": 81}"#)), Some(81));
    // what a form shows as its default is that
    for k in KINDS {
        let d = defaults(k).unwrap_or_else(empty_object);
        for (i, f) in fields(k).enumerate() {
            if f.kind == Fk::Int && f.key.find('.').is_none() {
                if let Some(v) = d.get(f.key).and_then(|x| x.as_u64()) {
                    t.eq(&format!("{}: the default of {}", k, f.key), default_text(k, i, &d), format!("{}", v));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- one truth for checking

fn refusals(t: &mut T) {
    let p = lab();
    let d = p.find("WEB1").cloned().unwrap_or_default();
    let verdict = |d: &jfn_model::project::Device, kind: &str, text: &str| trial(d, &def(kind, "x", text)).err().map(|m| m.text());
    t.eq("a port that is none", verdict(&d, "http-server", r#"{"port": 70000}"#).as_deref(), Some("Dienst http-server: Port 70000 gibt es nicht."));
    t.eq("another mode", verdict(&d, "http-load", r#"{"targets": ["http://1.1.1.1/"], "mode": "x"}"#).as_deref(), Some("Dienst http-load: „mode“ kennt x nicht (closed, open)."));
    t.eq("an address that is none", verdict(&d, "lb", r#"{"backends": [{"addr": "x"}]}"#).as_deref(), Some("Dienst lb: Backend 1 braucht „addr“ mit einer IP-Adresse."));
    t.eq("a node that is none", verdict(&d, "kv", r#"{"nodes": ["y"]}"#).as_deref(), Some("Dienst kv: „y“ ist keine IPv4-Adresse."));
    t.eq("a backend that is no URL", verdict(&d, "cache", r#"{"backend": "ftp://z"}"#).as_deref(), Some("Dienst cache: „ftp://z“ ist keine http-URL."));
    t.eq("a queue that is no URL", verdict(&d, "worker", r#"{"queue": "ftp://z"}"#).as_deref(), Some("Dienst worker: „ftp://z“ ist keine http-URL."));
    t.eq("all right", verdict(&d, "mq", r#"{"port": 9001}"#), None);
    // what a service lacks
    t.eq("a balancer without backends", missing("lb", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch keine Backends: der Load Balancer hat niemanden, an den er verteilen kann."));
    t.eq("a generator without a target", missing("http-load", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch kein Ziel: der Lastgenerator sendet nichts."));
    t.eq("a store without nodes", missing("kv", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch keine Knoten: der KV-Speicher weiß nicht, wer dazugehört."));
    t.eq("a cache without a backend", missing("cache", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch kein Backend: der Cache kann nichts nachladen."));
    t.eq("a worker without a queue", missing("worker", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch keine Queue: der Worker holt nichts ab."));
    t.eq("a queue lacks nothing", missing("mq", &empty_object()), None);
    t.eq("a web server lacks nothing", missing("http-server", &empty_object()), None);
    t.eq("with a target it lacks nothing", missing("http-load", &cfg(r#"{"target": "http://1.1.1.1/"}"#)), None);
}

// ---------------------------------------------------------------- live or restart

// did the engine keep this service's numbers through a change: (the prediction, its counter before and after)
fn change(t: &mut T, doc: &mut Doc, dev: u32, svc: &str, j: usize, config: &str, want: Takes, what: &str, metric: &str) {
    let c = cfg(config);
    let said = takes(doc, dev, j, &c);
    t.eq(&format!("{}: as predicted", what), said, want);
    let before = doc.sim.engine().metric(dev, svc, metric).unwrap_or(0);
    let label = format!("{} einstellen", svc);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| x.services[j].config = c) {
        doc.apply(e);
    }
    let now = doc.sim.engine().metric(dev, svc, metric).unwrap_or(0);
    match want {
        Takes::Live => t.eq(&format!("{}: its numbers stay", what), now, before),
        Takes::Restart => t.eq(&format!("{}: it started again", what), now, 0),
        Takes::Same => {}
    }
    t.check(&format!("{}: it had something to count", what), before > 0 || metric == "keys");
}

fn live_or_restart(t: &mut T) {
    let p = lab();
    let mut doc = Doc::new();
    doc.replace(p, None);
    let id = |doc: &Doc, n: &str| doc.proj.find(n).map_or(0, |d| d.id);
    let _ = doc.sim.send(jfn_sim::Cmd::RunFor(3 * SEC));
    // the generator takes another rate, not another mode
    let g = id(&doc, "GEN");
    change(t, &mut doc, g, "load", 0, r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 40, "timeout_ms": 30000, "mix": [{"path": "/", "weight": 1}]}"#, Takes::Live, "http-load, a rate", "sent");
    // a web server: its workers live, its port with a restart
    let w = id(&doc, "WEB1");
    change(t, &mut doc, w, "web", 0, r#"{"port": 80, "cpu_ms": 1, "workers": 8, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, Takes::Live, "http-server, workers", "requests");
    let _ = doc.sim.send(jfn_sim::Cmd::RunFor(SEC));
    change(t, &mut doc, w, "web", 0, r#"{"port": 8080, "cpu_ms": 1, "workers": 8, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#, Takes::Restart, "http-server, a port", "requests");
    // the balancer: its algorithm live, its port with a restart
    let l = id(&doc, "LB");
    let _ = doc.sim.send(jfn_sim::Cmd::RunFor(SEC));
    change(t, &mut doc, l, "lb", 0, r#"{"backends": [{"name": "WEB1", "addr": "10.0.0.3", "port": 80, "note": "bleibt"}, {"name": "WEB2", "addr": "10.0.0.4"}], "algorithm": "least-conn", "health": {"type": "none", "extra": 1}, "custom": "bleibt"}"#, Takes::Live, "lb, an algorithm", "requests");
    let _ = doc.sim.send(jfn_sim::Cmd::RunFor(SEC));
    change(t, &mut doc, l, "lb", 0, r#"{"backends": [{"name": "WEB1", "addr": "10.0.0.3", "port": 80}], "port": 81, "health": {"type": "none"}}"#, Takes::Restart, "lb, a port", "requests");
    // a cache: its time to live live, its port with a restart (its counter is 0 until something asks)
    let c = id(&doc, "CA");
    t.eq("cache, a time to live", takes(&doc, c, 0, &cfg(r#"{"backend": "http://10.0.0.3/", "ttl_ms": 5}"#)), Takes::Live);
    t.eq("cache, a port", takes(&doc, c, 0, &cfg(r#"{"backend": "http://10.0.0.3/", "port": 81}"#)), Takes::Restart);
    // a worker: its slots live, its queue with a restart
    let r = id(&doc, "WRK");
    t.eq("worker, its slots", takes(&doc, r, 0, &cfg(r#"{"queue": "http://10.0.0.6:9000", "time_ms": 5, "concurrency": 4}"#)), Takes::Live);
    t.eq("worker, another queue", takes(&doc, r, 0, &cfg(r#"{"queue": "http://10.0.0.7:9000", "time_ms": 5}"#)), Takes::Restart);
    // a queue and a store have no way but a restart
    let (m, k) = (id(&doc, "MQ"), id(&doc, "KV1"));
    t.eq("mq", takes(&doc, m, 0, &cfg(r#"{"topics": ["x"]}"#)), Takes::Restart);
    t.eq("kv", takes(&doc, k, 0, &cfg(r#"{"nodes": ["10.0.0.5"], "rf": 2}"#)), Takes::Restart);
    t.eq("what is the same", takes(&doc, m, 0, &doc.proj.find("MQ").map_or(Value::Null, |d| d.services[0].config.clone())), Takes::Same);
    // one undo step takes it back
    let before = doc.proj.find("WRK").map(|d| d.services[0].config.clone());
    let e = edits::device_edit(&doc.proj, r, "x", |x| x.services[0].config = cfg(r#"{"queue": "http://10.0.0.6:9000", "time_ms": 7}"#));
    if let Some(e) = e {
        doc.apply(e);
    }
    t.check("a step", doc.undo());
    t.eq("undone", doc.proj.find("WRK").map(|d| d.services[0].config.clone()), before);
}

// ---------------------------------------------------------------- presets

fn presets(t: &mut T) {
    let mut doc = Doc::new();
    doc.replace(Project::new("p", 1), None);
    let cases = [("load-balancer", "lb", "lb"), ("database", "db", "db"), ("cache", "cache", "cache"), ("queue", "mq", "mq"), ("traffic-gen", "http-load", "load")];
    for (model, kind, name) in cases {
        let made = edits::add_placed(&mut doc.proj, model, 0, 0);
        let Ok((id, e)) = made else {
            t.check(model, false);
            continue;
        };
        doc.record(e);
        let d = doc.proj.device(id).cloned().unwrap_or_default();
        t.eq(&format!("{}: its service", model), d.services.iter().map(|s| (s.kind.as_str(), s.name.as_str())).collect::<Vec<_>>(), vec![(kind, name)]);
        t.check(&format!("{}: in one step", model), doc.undo());
        t.check(&format!("{}: undo takes it all", model), doc.proj.device(id).is_none());
        t.check(&format!("{}: redo brings it", model), doc.redo());
        t.eq(&format!("{}: with its service", model), doc.proj.device(id).map(|d| d.services.len()), Some(1));
    }
    // other tiles bring nothing
    if let Ok((id, e)) = edits::add_placed(&mut doc.proj, "server-1u", 0, 0) {
        doc.record(e);
        t.eq("a server brings nothing", doc.proj.device(id).map(|d| d.services.len()), Some(0));
    }
    // a loaded file gets nothing: what stands there is what runs, byte for byte
    let mut old = Project::new("alt", 1);
    let lb = old.add_device("load-balancer", 0, 0).unwrap_or(0);
    let text = old.to_text();
    let again = Project::from_text(&text).unwrap_or_default();
    t.eq("a file's balancer has no service", again.device(lb).map(|d| d.services.len()), Some(0));
    t.eq("and stays the same file", again.to_text(), text);
    // a preset sends nothing
    let mut q = Project::new("q", 1);
    let g = q.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let s = q.add_device("server-1u", 200, 0).unwrap_or(0);
    let _ = q.connect(&pr(g, "eth0"), &pr(s, "eth1"), None);
    if let Some(d) = q.device_mut(g) {
        d.services.push(ServiceDef { kind: String::from("http-load"), name: String::from("load"), disabled: false, config: empty_object() });
    }
    let mut e = Engine::from_project(&q).unwrap_or_else(|_| Engine::new(1));
    e.run_until(5 * SEC, usize::MAX);
    t.eq("no frame on the cable", q.links.first().map(|l| e.cable_series(l.id).iter().map(|x| x.points.iter().map(|p| p.1).sum::<i64>()).sum::<i64>()), Some(0));
    t.check("it does not run", e.service_list().iter().all(|x| x.kind != "http-load"));
    t.eq("the engine keeps why", e.refused(g).len(), 1);
}

fn problems_named(t: &mut T) {
    let mut p = Project::new("pp", 1);
    let lb = p.add_device("load-balancer", 0, 0).unwrap_or(0);
    let gen_dev = p.add_device("traffic-gen", 200, 0).unwrap_or(0);
    let off = p.add_device("cache", 400, 0).unwrap_or(0);
    let bad = p.add_device("server-1u", 600, 0).unwrap_or(0);
    let add = |p: &mut Project, id: u32, s: ServiceDef| {
        if let Some(d) = p.device_mut(id) {
            d.services.push(s);
        }
    };
    add(&mut p, lb, def("lb", "lb", "{}"));
    add(&mut p, gen_dev, def("http-load", "load", "{}"));
    let mut sw = def("cache", "cache", "{}");
    sw.disabled = true;
    add(&mut p, off, sw);
    add(&mut p, bad, def("http-server", "web", r#"{"port": 70000}"#));
    let e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let mine: Vec<(u32, String)> = e.problems().iter().filter(|x| x.kind == jfn_sim::problems::ProblemKind::ServiceNotStarted).map(|x| (x.dev, x.text.clone())).collect();
    t.eq(
        "problems name what is missing",
        mine,
        vec![
            (lb, String::from("Der Dienst lb läuft nicht. Noch keine Backends: der Load Balancer hat niemanden, an den er verteilen kann.")),
            (gen_dev, String::from("Der Dienst load läuft nicht. Noch kein Ziel: der Lastgenerator sendet nichts.")),
            (bad, String::from("Der Dienst web läuft nicht. Dienst http-server: Port 70000 gibt es nicht.")),
        ],
    );
}

// ---------------------------------------------------------------- runs

const SMALL: &str = r#"{"format": "jfnex", "version": 1, "name": "klein", "title": "Klein", "load": {"dev": "GEN", "svc": "load", "rps": [20, 40]}, "timing": {"warmup_s": 1, "settle_s": 0, "measure_s": 1, "seeds": [1, 2]}, "slo": {"p99_ms": 200, "errors_pct": 1}, "axes": [{"kind": "config", "dev": "WEB1", "path": "web.cpu_ms", "values": [1, 2]}]}"#;

fn experiment(text: &str) -> Experiment {
    Experiment::from_text(text).unwrap_or_default()
}

// a job to its end, or to `secs` of real time
fn wait_job(job: &mut SweepJob, secs: u64) -> Option<SweepOut> {
    let until = plat::time_us() + secs * 1_000_000;
    loop {
        if let Some(o) = job.take() {
            return Some(o);
        }
        if plat::time_us() > until {
            return None;
        }
        plat::sleep_ms(2);
    }
}

fn runs(t: &mut T) {
    let p = lab();
    let x = experiment(SMALL);
    t.eq("it fits", x.problems(&p).iter().map(|m| m.text()).collect::<Vec<_>>(), Vec::<String>::new());
    let Ok(plan) = Plan::new(&x, &p, "klein.jfnex") else {
        t.check("a plan", false);
        return;
    };
    let total = plan.runs();
    t.eq("runs of the plan", total, 8);
    let alone = run_sweep(&plan, 1);
    // the job's results are those of run_sweep, at 1 and N threads
    for threads in [1, 3] {
        let mut job = SweepJob::start(x.clone(), p.clone(), String::from("klein.jfnex"), threads);
        t.eq("the job knows how many", job.as_ref().map(|j| j.total()), Some(total));
        let out = job.as_mut().and_then(|j| wait_job(j, 300));
        t.eq("it counted them all", job.as_ref().map(|j| j.done()), Some(total));
        match out {
            Some(SweepOut::Done(s)) => {
                t.eq(&format!("{} threads: the same CSV", threads), s.csv(), alone.csv());
                t.eq(&format!("{} threads: the same table", threads), s.table(), alone.table());
                t.eq(&format!("{} threads: the same details", threads), s.detail_csv(), alone.detail_csv());
            }
            o => t.check(&format!("{} threads: it ended well: {:?}", threads, o.is_some()), false),
        }
    }
    t.check("no job is left", !jfn_run::sweeps_pending());
    // cancel before it starts: nothing comes back
    let ctl = alloc::sync::Arc::new(SweepCtl::new());
    ctl.cancel();
    t.check("cancelled: no result", run_sweep_ctl(&plan, 2, &ctl).is_none());
    t.eq("cancelled: nothing ran", ctl.done(), 0);
    // cancel while it runs: a long one, ending soon after
    let mut long = x.clone();
    long.timing.warmup_s = 60;
    long.timing.measure_s = 200;
    let mut job = SweepJob::start(long, p.clone(), String::from("x.jfnex"), 2);
    t.check("it runs", job.is_some() && jfn_run::sweeps_pending());
    if let Some(j) = job.as_ref() {
        j.cancel();
    }
    let out = job.as_mut().and_then(|j| wait_job(j, 60));
    t.check("cancelled while running: nothing comes back", matches!(out, Some(SweepOut::Cancelled)));
    t.check("no job is left after it", !jfn_run::sweeps_pending());
    // what does not fit is refused, with its findings
    let mut wrong = x.clone();
    wrong.load.dev = String::from("NEIN");
    let mut job = SweepJob::start(wrong, p.clone(), String::from("x.jfnex"), 1);
    match job.as_mut().and_then(|j| wait_job(j, 60)) {
        Some(SweepOut::Refused(found)) => t.eq("refused", found.iter().map(|m| m.text()).collect::<Vec<_>>(), vec![String::from("Last: das Gerät „NEIN“ gibt es nicht.")]),
        _ => t.check("refused", false),
    }
    // edits during a run do not touch it: the job holds copies
    let mut job = SweepJob::start(x.clone(), p.clone(), String::from("klein.jfnex"), 2);
    let mut q = p.clone();
    q.devices.clear();
    drop(q);
    match job.as_mut().and_then(|j| wait_job(j, 300)) {
        Some(SweepOut::Done(s)) => t.eq("the copy ran", s.csv(), alone.csv()),
        _ => t.check("the copy ran", false),
    }
    // dropping a job cancels it and waits
    let mut long = x.clone();
    long.timing.warmup_s = 60;
    let job = SweepJob::start(long, p.clone(), String::from("x.jfnex"), 2);
    drop(job);
    t.check("a dropped job is gone", !jfn_run::sweeps_pending());
}

// ---------------------------------------------------------------- kept runs

fn kept(t: &mut T) {
    let p = lab();
    let x = experiment(SMALL);
    let Ok(plan) = Plan::new(&x, &p, "klein.jfnex") else { return };
    let mut sweep = run_sweep(&plan, 2);
    // what a kept run stores comes back: the same bytes
    let copy = Sweep::from_value(&sweep.to_value());
    t.eq("the CSV after to_value", copy.as_ref().map(|s| s.csv()), Some(sweep.csv()));
    t.eq("the table after to_value", copy.as_ref().map(|s| s.table()), Some(sweep.table()));
    t.eq("the points", copy.as_ref().map(|s| s.points.clone()), Some(sweep.points.clone()));
    t.eq("no details", copy.as_ref().map(|s| s.details.len()), Some(0));
    t.check("users come back", {
        let mut u = sweep.clone();
        u.levels = vec![jfn_model::experiment::Level::Users(4), jfn_model::experiment::Level::Users(8)];
        Sweep::from_value(&u.to_value()).is_some_and(|c| c.levels == u.levels)
    });
    t.check("something else is no sweep", Sweep::from_value(&cfg("{}")).is_none());
    // old files stay byte-identical while there are no runs
    let old = p.to_text();
    t.check("no member without runs", !old.contains("\"runs\""));
    t.eq("the same bytes", Project::from_text(&old).map(|q| q.to_text()).unwrap_or_default(), old.clone());
    // keeping it: the file has changed, there is no undo step
    let mut doc = Doc::new();
    doc.replace(p.clone(), None);
    t.check("clean", !doc.dirty());
    let (undo_before, n_exp) = (doc.hist.can_undo(), doc.proj.experiments.len());
    t.eq("keep a run", keep_run(&mut doc, &x, &sweep, " Erster Lauf ", "2026-11-03 14:05"), Ok(()));
    t.check("dirty", doc.dirty());
    t.eq("no undo step", doc.hist.can_undo(), undo_before);
    t.eq("the experiments stay", doc.proj.experiments.len(), n_exp);
    t.eq("its name", doc.proj.runs[0].name.as_str(), "Erster Lauf");
    t.eq("the experiment as it ran", &doc.proj.runs[0].experiment, &x);
    doc.saved("x.jfnet");
    t.check("saved: clean", !doc.dirty());
    // through a file
    let text = doc.proj.to_text();
    t.check("a member now", text.contains("\"runs\""));
    let back = Project::from_text(&text).unwrap_or_default();
    t.eq("a run comes back", back.runs.len(), 1);
    t.eq("its date", back.runs[0].date.as_str(), "2026-11-03 14:05");
    t.eq("its experiment", &back.runs[0].experiment, &x);
    let s = Sweep::from_value(&back.runs[0].result);
    t.eq("its CSV", s.as_ref().map(|c| c.csv()), Some(sweep.csv()));
    t.eq("its table", s.as_ref().map(|c| c.table()), Some(sweep.table()));
    t.eq("the file again", back.to_text(), text.clone());
    // the CSV of the window is jfn-sim's: Sweep::csv on the same experiment; the export names the project
    let exported = export_text(&x, "lab.jfnet");
    let again = Experiment::from_text(&exported).unwrap_or_default();
    t.eq("an exported file names its project", again.project.as_str(), "lab.jfnet");
    let mut plain = again.clone();
    plain.project = x.project.clone();
    t.eq("and is the experiment", &plain, &x);
    // the bounds
    t.eq("a name it has", keep_run(&mut doc, &x, &sweep, "erster lauf", "d").err().as_deref(), Some("Einen Lauf „erster lauf“ gibt es schon."));
    t.eq("no name", keep_run(&mut doc, &x, &sweep, "  ", "d").err().as_deref(), Some("Ein Lauf braucht einen Namen."));
    for k in doc.proj.runs.len()..KEEP_RUNS {
        let _ = keep_run(&mut doc, &x, &sweep, &format!("Lauf {}", k), "d");
    }
    t.eq("as many as it may", doc.proj.runs.len(), KEEP_RUNS);
    t.eq("one too many", keep_run(&mut doc, &x, &sweep, "noch einer", "d").err(), Some(format!("Das Projekt hält schon {} Läufe; entfernen Sie erst einen.", KEEP_RUNS)));
    doc.proj.runs.clear();
    let one = sweep.points.first().cloned().unwrap_or_default();
    sweep.points = vec![one; KEEP_POINTS + 1];
    t.eq("too many points", keep_run(&mut doc, &x, &sweep, "gross", "d").err(), Some(format!("Dieser Lauf hat {} Punkte; das Projekt hält höchstens {} je Lauf. Die CSV-Datei hält alles.", KEEP_POINTS + 1, KEEP_POINTS)));
    t.eq("nothing kept then", doc.proj.runs.len(), 0);
    // a kept run's size in the file: about 300 bytes a point, as jfn_model::experiment says, and here few
    // points with all that comes with any run (the experiment, the configurations)
    let per_point = (text.len() - old.len()) / copy.as_ref().map_or(1, |s| s.points.len());
    t.check(&format!("a point takes {} bytes of the file with all that comes with a run", per_point), per_point < 1200);
}

// ---------------------------------------------------------------- the warning for overload

fn overload(t: &mut T) {
    // a short time limit under overload: what SA2 found, without a long run
    let mut p = Project::new("o", 4);
    let g = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let s = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(g, "eth0"), &pr(s, "eth1"), None);
    if let Some(d) = p.device_mut(g) {
        d.name = String::from("GEN");
        d.startup_config = String::from("ip addr add 10.0.0.1/24 dev eth0\n");
        d.services.push(def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 10, "timeout_ms": 50, "keepalive": false}"#));
    }
    if let Some(d) = p.device_mut(s) {
        d.name = String::from("WEB");
        d.startup_config = String::from("ip addr add 10.0.0.2/24 dev eth1\n");
        d.hw = Some(jfn_model::catalog::Hw { cores: 1, speed_pm: 1000, ram_mb: 8 << 10, swap_mb: 1 << 10, disk: jfn_model::catalog::DiskClass::Sata, disk_gb: 240 });
        d.services.push(def("http-server", "web", r#"{"cpu_ms": 20, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#));
    }
    let x = experiment(r#"{"format": "jfnex", "version": 1, "load": {"dev": "GEN", "svc": "load", "rps": [20, 400]}, "timing": {"warmup_s": 1, "settle_s": 2, "measure_s": 3, "seeds": [1]}, "slo": {"p99_ms": 200, "errors_pct": 1}}"#);
    let Ok(plan) = Plan::new(&x, &p, "o.jfnex") else {
        t.check("a plan for overload", false);
        return;
    };
    let sweep = run_sweep(&plan, 1);
    let points: Vec<_> = sweep.points_of(0).cloned().collect();
    t.check("the low level answers", points[0].achieved_m > 10_000);
    t.check("the high one breaks down", points[1].achieved_m * 2 < points[0].achieved_m);
    let found: Vec<String> = sweep.overloads().iter().map(|m| m.text()).collect();
    t.eq("the warning says so", found.len(), 1);
    t.check("with the level and the numbers", found.first().is_some_and(|m| m.contains("bei 400/s sinkt der Durchsatz auf") && m.contains("in die Zeitgrenze")));
    // a run that only flattens is no breakdown: a long limit
    let mut calm = Sweep::default();
    calm.levels = vec![jfn_model::experiment::Level::Rps(1), jfn_model::experiment::Level::Rps(2)];
    calm.configs = vec![jfn_sim::experiment::ConfigOut::default()];
    calm.points = vec![jfn_sim::experiment::Point { achieved_m: 100, ..Default::default() }, jfn_sim::experiment::Point { level: 1, achieved_m: 99, ..Default::default() }];
    t.eq("flat: no warning", calm.overloads().len(), 0);
    // the fields of the page
    t.eq("numbers", parse_numbers("100, 400 1600"), Ok(vec![100, 400, 1600]));
    t.eq("not a number", parse_numbers("100, x"), Err(String::from("„x“ ist keine ganze Zahl.")));
    t.eq("values of an axis", parse_values("1, 4, 16"), vec![Value::u64(1), Value::u64(4), Value::u64(16)]);
    t.eq("words and truth", parse_values("strong; true; 2,5").iter().map(|v| json::stringify(v).unwrap_or_default()).collect::<Vec<_>>(), vec![String::from("\"strong\""), String::from("true"), String::from("2.5")]);
}

// ---------------------------------------------------------------- the bars of the Metriken panel

fn bars(t: &mut T) {
    t.eq("nothing", rebin(&[]), Vec::new());
    t.eq("one value", rebin(&[(5, 1, 3)]), vec![(10, 3)]);
    // from the first bucket's end to the last one's, limits of 1, 2, 5
    t.eq("a spread", rebin(&[(1500, 100, 2), (1900, 100, 1), (4000, 200, 4), (90_000, 2000, 1)]), vec![(2000, 3), (5000, 4), (10_000, 0), (20_000, 0), (50_000, 0), (100_000, 1)]);
    let mut h = base::metrics::Hist::new();
    for v in [1_000_000u64, 1_000_000, 3_000_000, 900_000_000] {
        h.record(v);
    }
    let got = rebin(&h.buckets());
    t.eq("nothing is lost", got.iter().map(|b| b.1).sum::<u64>(), 4);
    t.check("the limits rise", got.windows(2).all(|w| w[0].0 < w[1].0));
    // p99.9 from the window of a service
    let p = lab();
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    e.run_until(4 * SEC, usize::MAX);
    let gen_dev = p.find("GEN").map_or(0, |d| d.id);
    let k = e.kpi_between(gen_dev, "load", SEC, 4 * SEC);
    t.check("a tile for p99.9", k.as_ref().is_some_and(|k| k.p999_us.is_some()));
    t.check("it is no less than p99", k.as_ref().is_some_and(|k| k.p999_us.unwrap_or(0) >= k.p99_us.unwrap_or(0)));
    t.check("bars of its window", e.lat_hist_between(gen_dev, "load", SEC, 4 * SEC).is_some_and(|h| h.count() > 0));
    t.check("none of a service without one", e.lat_hist_between(gen_dev, "nein", SEC, 4 * SEC).is_none());
    // a cable's use stays within its bit rate, and the disk's series stay out of the CSV
    let link = p.links.first().map_or(0, |l| l.id);
    let util = e.cable_util(link);
    t.check("a cable's use", !util.is_empty() && util.iter().all(|u| (0..=100_000).contains(&u.1) && (0..=100_000).contains(&u.2)));
    let mut q = lab();
    q.metered = true;
    let mut m = Engine::from_project(&q).unwrap_or_else(|_| Engine::new(1));
    m.run_until(3 * SEC, usize::MAX);
    let web = q.find("WEB1").map_or(0, |d| d.id);
    let names: Vec<&str> = m.series(web, jfn_sim::res::HOST).iter().map(|s| s.name).collect();
    t.check("the disk's series are there", names.contains(&"disk_pm") && names.contains(&"disk_wait_us") && names.contains(&"disk_backlog_us"));
    t.check("and stay out of the CSV", !m.series_csv().contains("disk_pm") && !m.series_csv().contains("disk_backlog"));
}
