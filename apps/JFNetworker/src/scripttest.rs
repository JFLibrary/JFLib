// scripttest -- checks of scripts as services (part of --selftest, group "script";
// crates/jfn_sim/scriptsvc.rs): fuel as CPU time to the nanosecond (a reply leaves when the
// script's CPU time reaches the send, a second datagram waits for the first callback), timers,
// UDP, TCP servers and clients (also a character split between segments), HTTP servers (the
// answer of a returned value, 500 after an error, 503 when the queue is full, the plan's
// example with its disk and cache) and http.request, dns.resolve, the disk with its access
// times and capacity, metrics, an endless loop as 100 % CPU while the simulation goes on, the
// limits (fuel, heap, depth), errors with line and column in status, log and trace,
// stop_on_error, a failed start and a restart, the shell's script command and top,
// definitions and their refusals, the English of the texts, determinism (twice, and a clone in
// the middle of a callback)
//
// Times: as labtest, 1 Gbit/s copper with 50 ns of cable and a switch between the hosts, every
// ARP entry set by hand: a frame of up to 60 bytes takes 722 ns per cable, 1444 ns from host to
// host.  Scripts cost 5 ns per instruction; cost() runs a callback in a VM of its own and says
// how much fuel it used up to each host call, so the expected times are exact.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Engine, Metric, Note, OsApi, Report, ScriptService, ScriptState, Service, SvcEvent, MS, SEC, US};
use netpkt::{Ipv4, Mac, Prefix4};
use script::{Cx, ErrorKind, Fail, Host, Limits, Pin, Status, Value, Vm};

use crate::selftest::T;

const BUDGET: usize = 50_000_000;
const SEED: u64 = 29;
/// a frame of up to 60 bytes from host to host over the switch
const HOP2: u64 = 1444;
/// ns per instruction
const OP: u64 = 5;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

/// the address of host i (0-based): 10.0.0.(i + 1)
fn addr(i: usize) -> Ipv4 {
    Ipv4([10, 0, 0, i as u8 + 1])
}

/// hosts SRV1..SRVn (10.0.0.1..n) on one switch, every ARP entry set by hand
fn net(n: usize) -> (Engine, Vec<u32>) {
    let mut p = Project::new("scripts", SEED);
    let sw = p.add_device("jf-s24p", 0, 200).unwrap_or(0);
    let hosts: Vec<u32> = (0..n).map(|i| p.add_device("server-1u", i as i32 * 100, 0).unwrap_or(0)).collect();
    for (i, h) in hosts.iter().enumerate() {
        let _ = p.connect(&pr(*h, "eth0"), &pr(sw, &format!("gi1/0/{}", i + 1)), None);
    }
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    for (i, h) in hosts.iter().enumerate() {
        let _ = e.apply(Action::IfAddr { dev: *h, iface: String::from("eth0"), prefix: Prefix4 { addr: addr(i), len: 24 }, add: true });
    }
    for (i, a) in hosts.iter().enumerate() {
        for (j, b) in hosts.iter().enumerate() {
            if i != j {
                let _ = e.apply(Action::StaticArp { dev: *a, ip: addr(j), mac: Some(mac(*b)) });
            }
        }
    }
    (e, hosts)
}

fn run_to(e: &mut Engine, t: u64) {
    e.run_until(t, BUDGET);
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Json::Null) }
}

fn add(e: &mut Engine, dev: u32, kind: &str, name: &str, config: &str) -> bool {
    e.apply(Action::Service { dev, def: def(kind, name, config) }).is_ok()
}

/// the definition of a script: its source and more members of the config ("\"port\": 7")
fn script_def(name: &str, src: &str, more: &str) -> ServiceDef {
    let mut c = Json::Object(Vec::new());
    c.insert("source", Json::string(src));
    if let Ok(Json::Object(m)) = agent_core::json::parse(format!("{{{}}}", more).as_bytes()) {
        for (k, v) in m {
            c.insert(&k, v);
        }
    }
    ServiceDef { kind: String::from("script"), name: String::from(name), disabled: false, config: c }
}

/// adds a script; true when the engine took it
fn script(e: &mut Engine, dev: u32, name: &str, src: &str, more: &str) -> bool {
    e.apply(Action::Service { dev, def: script_def(name, src, more) }).is_ok()
}

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

/// the lines a service wrote to the device's log
fn logs(e: &Engine, dev: u32, tag: &str) -> Vec<String> {
    e.log_lines(dev).into_iter().filter(|l| l.tag == tag).map(|l| l.text).collect()
}

/// (time, text) of the lines a service wrote to the log
fn log_times(e: &Engine, dev: u32, tag: &str) -> Vec<(u64, String)> {
    e.log_lines(dev).into_iter().filter(|l| l.tag == tag).map(|l| (l.t, l.text)).collect()
}

fn sv<'a>(e: &'a Engine, dev: u32, name: &str) -> Option<&'a ScriptService> {
    e.service::<ScriptService>(dev, name)
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut s = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            s.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    s
}

/// a command line on a device, then `run` of virtual time: what it printed
fn sh(e: &mut Engine, dev: u32, line: &str, run: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + run;
    run_to(e, t);
    out(e, dev)
}

/// the English of src/locales/engine.tsv (the German when a row is missing)
fn english() -> impl Fn(&'static str) -> &'static str {
    let rows: Vec<(&'static str, &'static str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1)
}

// ---------------------------------------------------------------- a UDP client for the checks

/// sends datagrams from port 5000 at given times and keeps what comes back, with the time
#[derive(Clone, Debug)]
struct UdpProbe {
    to: (Ipv4, u16),
    plan: Vec<(u64, Vec<u8>)>,
    next: usize,
    got: Vec<(u64, Vec<u8>)>,
}

impl UdpProbe {
    fn arm(&mut self, os: &mut OsApi) {
        if let Some(p) = self.plan.get(self.next) {
            let at = p.0.max(os.now());
            os.timer(0, at);
        }
    }
}

impl Service for UdpProbe {
    fn kind(&self) -> &'static str {
        "udp-probe"
    }
    fn start(&mut self, os: &mut OsApi) {
        let _ = os.bind(5000, None);
        self.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Timer(0) => {
                let now = os.now();
                while let Some(p) = self.plan.get(self.next) {
                    if p.0 > now {
                        break;
                    }
                    let d = p.1.clone();
                    os.send(5000, self.to.0, self.to.1, &d);
                    self.next += 1;
                }
                self.arm(os);
            }
            SvcEvent::Udp(d) => self.got.push((os.now(), d.data)),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::new()
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// a probe on `dev` that sends `plan` to (to, port)
fn probe(e: &mut Engine, dev: u32, to: Ipv4, port: u16, plan: &[(u64, &str)]) -> bool {
    let p = UdpProbe { to: (to, port), plan: plan.iter().map(|(t, d)| (*t, Vec::from(d.as_bytes()))).collect(), next: 0, got: Vec::new() };
    e.add_service(dev, "probe", Box::new(p), true).is_ok()
}

/// what the probe of `dev` got
fn got(e: &Engine, dev: u32) -> Vec<(u64, String)> {
    e.service::<UdpProbe>(dev, "probe").map(|p| p.got.iter().map(|(t, d)| (*t, String::from_utf8_lossy(d).into_owned())).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------- the fuel of a callback

/// a host that keeps the callback the top-level code gives `reg` and notes the fuel at every
/// other host call
struct Fake {
    reg: &'static str,
    cb: Option<Pin>,
    calls: Vec<(String, u64)>,
}

impl Host for Fake {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        if name == self.reg {
            if let Some(f) = args.iter().copied().find(|v| matches!(v, Value::Func(_))) {
                self.cb = Some(cx.pin(f));
            }
            return cx.new_map();
        }
        self.calls.push((String::from(name), cx.fuel()));
        if name == "rand" {
            return Ok(Value::Float(0.5));
        }
        Ok(Value::Nil)
    }
}

/// what a script costs: the fuel at the host calls of its top-level code and in all, those
/// of the callback it gives `reg` when that runs with `args`, and the callback's fuel in all
struct Cost {
    top: Vec<(String, u64)>,
    top_fuel: u64,
    calls: Vec<(String, u64)>,
    total: u64,
}

impl Cost {
    /// the fuel at the first call of `name` in the callback (0: none)
    fn at(&self, name: &str) -> u64 {
        self.calls.iter().find(|c| c.0 == name).map_or(0, |c| c.1)
    }
    fn top_at(&self, name: &str) -> u64 {
        self.top.iter().find(|c| c.0 == name).map_or(0, |c| c.1)
    }
}

fn cost(src: &str, reg: &'static str, args: &dyn Fn(&mut Vm) -> Vec<Value>) -> Cost {
    let mut c = Cost { top: Vec::new(), top_fuel: 0, calls: Vec::new(), total: 0 };
    let Ok(prog) = script::compile(src) else {
        return c;
    };
    let mut vm = Vm::new(&prog, Limits::default());
    let mut h = Fake { reg, cb: None, calls: Vec::new() };
    while vm.run(&mut h, u64::MAX) == Status::Yielded {}
    c.top = core::mem::take(&mut h.calls);
    c.top_fuel = vm.call_fuel();
    let Some(cb) = h.cb else {
        return c;
    };
    let f = vm.pinned(cb);
    let a = args(&mut vm);
    if vm.call_value(f, &a).is_err() {
        return c;
    }
    while vm.run(&mut h, u64::MAX) == Status::Yielded {}
    c.calls = h.calls;
    c.total = vm.call_fuel();
    c
}

/// a map with str fields and int fields
fn mk_map(vm: &mut Vm, strs: &[(&str, &str)], ints: &[(&str, i64)]) -> Value {
    let Ok(m) = vm.new_map() else {
        return Value::Nil;
    };
    for (k, v) in strs {
        if let (Ok(k), Ok(v)) = (vm.new_str(k), vm.new_str(v)) {
            let _ = vm.map_set(m, k, v);
        }
    }
    for (k, v) in ints {
        if let Ok(k) = vm.new_str(k) {
            let _ = vm.map_set(m, k, Value::Int(*v));
        }
    }
    m
}

/// the arguments of a UDP callback: the socket and {data, from, port}
fn udp_args(vm: &mut Vm) -> Vec<Value> {
    let sock = mk_map(vm, &[("kind", "udp")], &[("port", 7)]);
    let msg = mk_map(vm, &[("data", "hallo"), ("from", "10.0.0.2")], &[("port", 5000)]);
    vec![sock, msg]
}

// ---------------------------------------------------------------- the checks

pub fn run(t: &mut T) {
    udp_cpu(t);
    timers(t);
    tcp(t);
    http(t);
    example(t);
    dns(t);
    disk(t);
    metrics(t);
    endless(t);
    limits(t);
    errors(t);
    shell(t);
    defs(t);
    texts(t);
    determinism(t);
}

const ECHO: &str = "net.udp(7, fn(s, m) { s.send(m.from, m.port, m.data) })";
const BUSY_ECHO: &str = "fn work(n) { let i = 0; while i < n { i += 1 } }\nnet.udp(7, fn(s, m) { work(100); s.send(m.from, m.port, m.data) })";

// a reply leaves when the CPU time reaches the send; a busy callback delays the next datagram
fn udp_cpu(t: &mut T) {
    let (mut e, h) = net(3);
    let ok = script(&mut e, h[0], "echo", ECHO, "") && probe(&mut e, h[1], addr(0), 7, &[(MS, "hallo")]);
    t.check("udp: services", ok);
    run_to(&mut e, 2 * MS);
    let c = cost(ECHO, "net.udp", &udp_args);
    let fs = c.at("map.send");
    t.check(&format!("udp: the send after a few instructions ({}, of {})", fs, c.total), fs > 3 && fs < 40 && c.total > fs);
    t.eq("udp: the reply leaves at the send's CPU time", got(&e, h[1]), vec![(MS + 2 * HOP2 + OP * fs, String::from("hallo"))]);
    t.eq("udp: calls and fuel", (metric(&e, h[0], "echo", "calls"), metric(&e, h[0], "echo", "fuel")), (2, (c.top_fuel + c.total) as i64));
    t.eq("udp: idle again", (metric(&e, h[0], "echo", "queued"), sv(&e, h[0], "echo").is_some_and(|s| s.busy(e.now()))), (0, false));

    // two datagrams: the second comes while the first callback still computes and waits for it
    let (mut e, h) = net(3);
    let ok = script(&mut e, h[0], "echo", BUSY_ECHO, "") && probe(&mut e, h[1], addr(0), 7, &[(MS, "a")]) && probe(&mut e, h[2], addr(0), 7, &[(MS + 100, "b")]);
    t.check("cpu: services", ok);
    run_to(&mut e, 2 * MS);
    let c = cost(BUSY_ECHO, "net.udp", &udp_args);
    let (fs, ft) = (c.at("map.send"), c.total);
    t.check(&format!("cpu: the loop costs ({} up to the send, {} in all)", fs, ft), fs > 900 && ft > fs);
    let end1 = MS + HOP2 + OP * ft;
    t.eq("cpu: the first reply after the loop", got(&e, h[1]), vec![(MS + 2 * HOP2 + OP * fs, String::from("a"))]);
    // b reached SRV1 at 1 ms + 2116 ns, behind a on the switch's port, while a still computed
    t.check("cpu: b came during a's callback", MS + 2116 < end1);
    t.eq("cpu: the second waits for the CPU", got(&e, h[2]), vec![(end1 + OP * fs + HOP2, String::from("b"))]);
    t.eq("cpu: it waited in the queue", metric(&e, h[0], "echo", "max_queued"), 1);
    let busy = notes(&e, h[0], |n| matches!(n, Note::ScriptBusy { .. }));
    t.eq("cpu: the note of the wait", busy.iter().map(|x| x.0).collect::<Vec<u64>>(), vec![MS + 2116]);
    t.eq("cpu: busy time", metric(&e, h[0], "echo", "busy_us"), (OP * (c.top_fuel + 2 * ft) / 1000) as i64);
}

/// (time, note) of a device's decisions that `f` picks
fn notes(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev && f(&x.note)).map(|x| (x.t, x.note.clone())).collect()
}

const TIMERS: &str = r#"let n = 0
let id = timer.every(100, fn() {
  n += 1
  metrics.gauge("ticks", n)
  if n == 3 { timer.cancel(id) }
})
timer.after(250, fn() {
  metrics.gauge("at", int(now()))
  log("nach 250 ms: ${n} Ticks")
})
let k = 0
let fast = timer.every(0, fn() { k += 1; metrics.gauge("k", k) })
timer.after(1, fn() { timer.cancel(fast) })
"#;

// timer.after, timer.every (at least every 10 µs), cancel, now()
fn timers(t: &mut T) {
    let (mut e, h) = net(1);
    t.check("timers: service", script(&mut e, h[0], "tick", TIMERS, ""));
    run_to(&mut e, SEC);
    t.eq("timers: three ticks, then cancelled", metric(&e, h[0], "tick", "ticks"), 3);
    t.eq("timers: now() in ms", metric(&e, h[0], "tick", "at"), 250);
    let lines = log_times(&e, h[0], "tick");
    t.eq("timers: the log line", lines.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), vec!["nach 250 ms: 2 Ticks"]);
    t.check("timers: logged at 250 ms and some CPU time", lines.first().is_some_and(|l| l.0 > 250 * MS && l.0 < 250 * MS + 10 * US));
    // every(0) fires every 10 µs: 100 times before the cancel a few ns after 1 ms
    t.eq("timers: every 10 µs at most", metric(&e, h[0], "tick", "k"), 100);
    t.eq("timers: none left", metric(&e, h[0], "tick", "timers"), 0);
    let c = cost(TIMERS, "", &udp_args);
    t.check("timers: set by the top-level code", c.top_at("timer.every") > 0);
}

const TCP_ECHO: &str = r#"let open = 0
net.listen(7000, {
  binary: true,
  accept: fn(c) { open += 1; metrics.inc("accepted"); metrics.gauge("open", open) },
  data: fn(c, d) { metrics.inc("bytes", len(d)); c.send(d) },
  close: fn(c, why) { open -= 1; metrics.gauge("open", open); log("zu: ${c.peer} ${why}") },
})
"#;

const TEXT_SERVER: &str = r#"metrics.inc("broken", 0)
net.listen(7001, {
  data: fn(c, d) { metrics.inc("chars", len(d)); metrics.inc("pieces"); if d.contains("\u{fffd}") { metrics.inc("broken") } },
  close: fn(c, why) { metrics.inc("closed"); log("Server: ${why}") },
})
"#;

const TEXT_CLIENT: &str = r#"let c = net.connect("10.0.0.1", 7001, {
  connect: fn(c) { c.send("x" + "ä".repeat(3000)); c.close() },
  close: fn(c, why) { metrics.inc("closed"); log("Client: ${why} ${c.port > 0}") },
})
"#;

const ECHO_CLIENT: &str = r#"net.connect("10.0.0.1", 7, {
  connect: fn(c) { c.send("hallo\n") },
  data: fn(c, d) { log("Echo: ${d.trim()}"); c.close() },
  close: fn(c, why) { log("Ende: ${why}") },
})
net.connect("10.0.0.1", 9999, { close: fn(c, why) { log("9999: ${why}") } })
"#;

// a script TCP server (binary echo) for a test client, text with a character split between
// segments, a script client against the echo service, a refused connection
fn tcp(t: &mut T) {
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "echo", TCP_ECHO, "") && add(&mut e, h[1], "tcp-client", "cli", "{\"server\": \"10.0.0.1\", \"port\": 7000, \"bytes\": 10000}");
    t.check("tcp: services", ok);
    run_to(&mut e, SEC);
    t.eq("tcp: the client got its echo", (metric(&e, h[1], "cli", "received"), metric(&e, h[1], "cli", "done")), (10_000, 1));
    t.eq("tcp: the script saw it", (metric(&e, h[0], "echo", "accepted"), metric(&e, h[0], "echo", "bytes"), metric(&e, h[0], "echo", "open")), (1, 10_000, 0));
    t.eq("tcp: the close callback", logs(&e, h[0], "echo"), vec![String::from("zu: 10.0.0.2 closed")]);
    t.eq("tcp: no connection left", metric(&e, h[0], "echo", "conns"), 0);

    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "srv", TEXT_SERVER, "") && script(&mut e, h[1], "cli", TEXT_CLIENT, "");
    t.check("text: services", ok);
    run_to(&mut e, SEC);
    t.eq("text: every character, none broken", (metric(&e, h[0], "srv", "chars"), metric(&e, h[0], "srv", "broken")), (3001, 0));
    t.check("text: in pieces", metric(&e, h[0], "srv", "pieces") > 1);
    t.eq("text: both ends closed", (logs(&e, h[0], "srv"), logs(&e, h[1], "cli")), (vec![String::from("Server: closed")], vec![String::from("Client: closed true")]));

    let (mut e, h) = net(2);
    let ok = add(&mut e, h[0], "tcp-echo", "echo", "{}") && script(&mut e, h[1], "cli", ECHO_CLIENT, "");
    t.check("client: services", ok);
    run_to(&mut e, SEC);
    t.eq("client: echo, close, refused", logs(&e, h[1], "cli"), vec![String::from("9999: refused"), String::from("Echo: hallo"), String::from("Ende: closed")]);
}

const WEB: &str = r#"let hits = 0
http.serve(80, fn(req, res) {
  hits += 1
  if req.path == "/hallo" { return res.send(200, "Hallo ${req.query.name ?? "Welt"}\n", {"X-Hits": hits}) }
  if req.path == "/json" { return res.json(201, {hits: hits, method: req.method}) }
  if req.path == "/text" { return "nur Text\n" }
  if req.path == "/verboten" { return 403 }
  if req.path == "/kaputt" { return 1 / 0 }
  res.send(404, "nicht da\n")
})
"#;

const SLOW_WEB: &str = r#"http.serve(80, fn(req, res) {
  let i = 0
  while i < 20000 { i += 1 }
  res.send(200, "fertig\n")
})
"#;

const WEB_CLIENT: &str = r#"http.request("http://10.0.0.1/hallo?name=Client", fn(err, resp) {
  if err { return log("Fehler: ${err}") }
  log("Antwort ${resp.status}: ${resp.body.trim()} (${resp.headers["x-hits"]})")
})
http.request({url: "http://10.0.0.1/json", method: "post", body: "x"}, fn(err, resp) {
  log("JSON ${resp.status}: ${json.decode(resp.body).method} ${resp.headers["content-type"]}")
})
http.request({url: "http://10.0.0.9/", timeout_ms: 500}, fn(err, resp) {
  log("weg: ${err != nil} ${resp}")
})
"#;

// http.serve: routes by path, the answer of a returned value, 500 after an error, the queue's
// 503; http.request
fn http(t: &mut T) {
    let (mut e, h) = net(3);
    t.check("http: service", script(&mut e, h[0], "web", WEB, ""));
    run_to(&mut e, MS);
    let curl = |e: &mut Engine, path: &str| sh(e, h[1], &format!("curl -s -w ' %{{http_code}}' http://10.0.0.1{}", path), 100 * MS);
    t.eq("http: query", curl(&mut e, "/hallo?name=Skript"), String::from("Hallo Skript\n 200"));
    t.eq("http: json", curl(&mut e, "/json"), String::from("{\"hits\":2,\"method\":\"GET\"} 201"));
    t.eq("http: a returned text", curl(&mut e, "/text"), String::from("nur Text\n 200"));
    t.eq("http: a returned status", curl(&mut e, "/verboten"), String::from("403 Forbidden\n 403"));
    t.eq("http: an error answers 500", curl(&mut e, "/kaputt"), String::from("500 Internal Server Error\n 500"));
    t.eq("http: not found", curl(&mut e, "/nix"), String::from("nicht da\n 404"));
    let head = sh(&mut e, h[1], "curl -si http://10.0.0.1/hallo", 100 * MS);
    t.check(&format!("http: headers ({})", head.replace('\r', "")), head.contains("X-Hits: 7\r\n") && head.contains("Content-Type: text/plain\r\n") && head.contains("Server: jfn-script/1.0\r\n"));
    let errs = sv(&e, h[0], "web").map(|s| s.errors()).unwrap_or_default();
    t.eq("http: the error", errs.iter().map(|x| (x.line, x.msg.as_str(), x.during)).collect::<Vec<_>>(), vec![(8, "Division durch 0", "http")]);
    t.eq("http: still running", sv(&e, h[0], "web").map(|s| s.state()), Some(ScriptState::Running));

    // http.request against the script server; a host that is not there times out
    t.check("http: client", script(&mut e, h[2], "cli", WEB_CLIENT, ""));
    let now = e.now();
    run_to(&mut e, now + SEC);
    t.eq("http: requests", logs(&e, h[2], "cli"), vec![String::from("Antwort 200: Hallo Client (8)"), String::from("JSON 201: POST application/json"), String::from("weg: true nil")]);

    // one request computes, one waits, the others get 503
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "slow", SLOW_WEB, "\"queue\": 1") && add(&mut e, h[1], "http-load", "load", "{\"targets\": [\"http://10.0.0.1/\"], \"mode\": \"closed\", \"users\": 4, \"requests\": 12, \"timeout_ms\": 2000}");
    t.check("queue: services", ok);
    run_to(&mut e, 2 * SEC);
    let rejected = metric(&e, h[0], "slow", "rejected");
    t.check(&format!("queue: rejected ({})", rejected), rejected >= 2 && metric(&e, h[1], "load", "status_5xx") == rejected);
    t.eq("queue: the rest answered", metric(&e, h[1], "load", "status_2xx") + rejected, 12);
    t.eq("queue: at most one waited", metric(&e, h[0], "slow", "max_queued"), 1);
}

/// the plan's example (lib/script's scriptt::EXAMPLE) with a record on the disk first
const EXAMPLE: &str = r#"let hits = 0
let cache = {}
fn key(id) { return "user/" + id }
disk.put(key("7"), {name: "Ada", id: 7})
http.serve(config.port ?? 80, fn(req, res) {
  hits += 1
  if req.path == "/user" {
    let id = req.query.id
    if cache[id] != nil { return res.json(200, cache[id]) }
    disk.get(key(id), fn(err, v) {
      if err { return res.send(500, "${err}") }
      cache[id] = v
      res.json(200, v)
    })
    return
  }
  res.send(200, "hello #${hits}")
})
"#;

// the example of the plan: the first read from the disk, the second from the cache
fn example(t: &mut T) {
    let (mut e, h) = net(2);
    t.check("example: service", script(&mut e, h[0], "users", EXAMPLE, "\"port\": 8080"));
    run_to(&mut e, MS);
    let curl = |e: &mut Engine, path: &str| sh(e, h[1], &format!("curl -s -w ' %{{time_starttransfer}}' http://10.0.0.1:8080{}", path), 100 * MS);
    let first = curl(&mut e, "/user?id=7");
    let second = curl(&mut e, "/user?id=7");
    let body = |s: &str| String::from(s.split(' ').next().unwrap_or(""));
    let secs = |s: &str| s.split(' ').nth(1).and_then(|x| x.replace('.', "").trim_start_matches('0').parse::<u64>().ok()).unwrap_or(0);
    t.eq("example: from the disk, then from the cache", (body(&first), body(&second)), (String::from("{\"name\":\"Ada\",\"id\":7}"), String::from("{\"name\":\"Ada\",\"id\":7}")));
    // the disk read takes 120 µs (its iops slot and its latency); curl shows whole microseconds
    t.check(&format!("example: the cache is faster by the disk ({} / {})", first, second), (119..=121).contains(&secs(&first).saturating_sub(secs(&second))));
    t.eq("example: a miss", body(&curl(&mut e, "/user?id=8")), String::from("null"));
    t.eq("example: the other path", body(&curl(&mut e, "/")), String::from("hello"));
}

const RESOLVE: &str = r#"dns.resolve("web", fn(err, a) { log("web: ${a}") })
dns.resolve("nix.lab", fn(err, a) { log("nix: ${err}") })
"#;

// dns.resolve through the device's resolver (search domain, CNAME), an unknown name
fn dns(t: &mut T) {
    let (mut e, h) = net(2);
    let ok = add(&mut e, h[0], "dns-server", "dns", "{\"zones\": [{\"origin\": \"lab\", \"records\": [\"www A 10.0.0.1\", \"web CNAME www\"]}]}")
        && e.apply(Action::Resolver { dev: h[1], servers: Some(vec![addr(0)]), search: Some(vec![String::from("lab")]) }).is_ok();
    t.check("dns: setup", ok);
    run_to(&mut e, MS);
    t.check("dns: script", script(&mut e, h[1], "res", RESOLVE, ""));
    run_to(&mut e, SEC);
    t.eq("dns: answers", logs(&e, h[1], "res"), vec![String::from("web: [\"10.0.0.1\"]"), String::from("nix: unbekannter Name")]);
}

const DISK: &str = r#"disk.put("a", {n: 1})
disk.put("b", [1, 2, 3])
disk.put("c", "drei", fn(err) { metrics.inc("puts") })
net.udp(9, fn(s, m) {
  disk.get(m.data, fn(err, v) { s.send(m.from, m.port, json.encode(v)) })
})
timer.after(10, fn() {
  disk.delete("b", fn(err, had) { log("b weg: ${had}") })
  disk.delete("x", fn(err, had) { log("x weg: ${had}") })
  disk.scan("", fn(err, items) { log("alles: ${json.encode(items)}") })
})
"#;

// the disk: an access takes its time (iops slot and latency), what it stores outlives a
// restart, a full disk refuses
fn disk(t: &mut T) {
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "store", DISK, "") && probe(&mut e, h[1], addr(0), 9, &[(MS, "a"), (2 * MS, "b"), (20 * MS, "b")]);
    t.check("disk: services", ok);
    run_to(&mut e, 100 * MS);
    let g = got(&e, h[1]);
    t.eq("disk: values", g.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), vec!["{\"n\":1}", "[1,2,3]", "null"]);
    // 1 ms + 1444 ns there, a few ns of CPU, 20 µs + 100 µs of disk, a few ns, 1444 ns back
    let wait = g.first().map_or(0, |x| x.0 - MS - 2 * HOP2);
    t.check(&format!("disk: an access takes 120 µs ({} ns)", wait), (120 * US..121 * US).contains(&wait));
    t.eq("disk: delete and scan", logs(&e, h[0], "store"), vec![String::from("b weg: true"), String::from("x weg: false"), String::from("alles: {\"a\":{\"n\":1},\"c\":\"drei\"}")]);
    t.eq("disk: a callback for the put", metric(&e, h[0], "store", "puts"), 1);
    // what is stored outlives a restart and a power cycle
    let runs = "disk.get(\"runs\", fn(err, v) { let n = (v ?? 0) + 1; disk.put(\"runs\", n); metrics.gauge(\"runs\", n) })";
    t.check("disk: counting starts", script(&mut e, h[0], "runs", runs, ""));
    run_to(&mut e, 110 * MS);
    let _ = e.apply(Action::ServiceRun { dev: h[0], name: String::from("runs"), on: false });
    let _ = e.apply(Action::ServiceRun { dev: h[0], name: String::from("runs"), on: true });
    run_to(&mut e, 120 * MS);
    let _ = e.apply(Action::Power { dev: h[0], on: false });
    run_to(&mut e, 130 * MS);
    let _ = e.apply(Action::Power { dev: h[0], on: true });
    run_to(&mut e, 200 * MS);
    t.eq("disk: kept over a restart and the power", metric(&e, h[0], "runs", "runs"), 3);

    let (mut e, h) = net(1);
    let src = "disk.put(\"k\", \"0123456789abcdefghij\", fn(err) { log(\"put: ${err}\") })";
    t.check("full: service", script(&mut e, h[0], "full", src, "\"disk\": {\"capacity_mb\": 0.00001}"));
    run_to(&mut e, SEC);
    t.eq("full: refused", logs(&e, h[0], "full"), vec![String::from("put: Die Disk ist voll.")]);
}

const METRICS: &str = r#"metrics.inc("a")
metrics.inc("a", 4)
metrics.gauge("g", -3)
metrics.observe("lat", 100)
metrics.observe("lat", 300)
try { metrics.inc("x y") } catch e { log(e) }
try { metrics.observe("a", 1) } catch e { log(e) }
"#;

// inc, gauge, observe (windows of five minutes), what does not fit
fn metrics(t: &mut T) {
    let (mut e, h) = net(1);
    t.check("metrics: service", script(&mut e, h[0], "m", METRICS, ""));
    run_to(&mut e, SEC);
    let m = |n: &str| metric(&e, h[0], "m", n);
    t.eq("metrics: counter, gauge", (m("a"), m("g")), (5, -3));
    t.eq("metrics: histogram", (m("lat_count"), m("lat_p50"), m("lat_p95"), m("lat_p99"), m("lat_max")), (2, 100, 300, 300, 300));
    t.eq(
        "metrics: errors",
        logs(&e, h[0], "m"),
        vec![String::from("metrics.inc: \u{201e}x y\u{201c} taugt nicht als Name (Buchstaben, Ziffern, _ . -)"), String::from("metrics.observe: a ist eine Metrik anderer Art")],
    );
    let names: Vec<String> = e.services(h[0]).first().map(|s| s.metrics.iter().map(|x| String::from(&*x.name)).collect()).unwrap_or_default();
    t.eq("metrics: the script's first", names.iter().take(7).cloned().collect::<Vec<_>>(), ["a", "g", "lat_count", "lat_p50", "lat_p95", "lat_p99", "lat_max"].map(String::from).to_vec());
    // after five minutes the window is empty
    run_to(&mut e, 302 * SEC);
    t.eq("metrics: the window moves on", metric(&e, h[0], "m", "lat_count"), 0);
}

const LOOP: &str = "timer.after(1, fn() { while true { } })\nnet.udp(7, fn(s, m) { s.send(m.from, m.port, m.data) })";

// an endless loop takes the CPU (100 % in top) while the simulation goes on; the fuel limit
// of a callback ends it at its exact moment
fn endless(t: &mut T) {
    let (mut e, h) = net(4);
    let ok = script(&mut e, h[0], "loop", LOOP, "\"limits\": {\"fuel\": 1000000}")
        && script(&mut e, h[2], "echo", ECHO, "")
        && probe(&mut e, h[1], addr(0), 7, &[(2 * MS, "a")])
        && probe(&mut e, h[3], addr(2), 7, &[(3 * MS, "b")]);
    t.check("loop: services", ok);
    run_to(&mut e, 20 * MS);
    let c = cost(LOOP, "net.udp", &udp_args);
    let start = MS + OP * c.top_at("timer.after");
    let end = start + OP * 1_000_000;
    let errs = sv(&e, h[0], "loop").map(|s| s.errors()).unwrap_or_default();
    t.eq("loop: ended by the fuel limit", errs.iter().map(|x| (x.at, x.kind, x.line, x.during)).collect::<Vec<_>>(), vec![(end, ErrorKind::Fuel, 1, "timer")]);
    t.eq("loop: the other host answers on time", got(&e, h[3]), vec![(3 * MS + 2 * HOP2 + OP * c.at("map.send"), String::from("b"))]);
    t.eq("loop: this one after the loop", got(&e, h[1]), vec![(end + OP * c.at("map.send") + HOP2, String::from("a"))]);
    t.eq("loop: still running", sv(&e, h[0], "loop").map(|s| s.state()), Some(ScriptState::Running));
    t.eq("loop: the error in the log", logs(&e, h[0], "loop"), vec![String::from("Fehler in Zeile 1, Spalte 29 (timer): Rechenzeit aufgebraucht: mehr als 1000000 Schritte in einem Aufruf \u{2013} eine Endlosschleife?")]);

    // no limit: the loop keeps the CPU; 1 µs per instruction, so a second is a million
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "loop", LOOP, "\"limits\": {\"fuel\": 0}, \"ns_per_op\": 1000, \"slice\": 1000") && probe(&mut e, h[1], addr(0), 7, &[(2 * MS, "a")]);
    t.check("busy: services", ok);
    run_to(&mut e, 2 * SEC + 999 * MS);
    t.check("busy: the simulation went on", e.now() == 2 * SEC + 999 * MS);
    let busy = metric(&e, h[0], "loop", "busy_us");
    t.check(&format!("busy: CPU time ({} µs)", busy), busy >= 2_998_000 && busy <= 3_000_000);
    t.eq("busy: the datagram waits", (got(&e, h[1]).len(), metric(&e, h[0], "loop", "queued")), (0, 1));
    let top = sh(&mut e, h[0], "top -b -n 1 -d 1", MS);
    let row = top.lines().find(|l| l.ends_with(" loop")).unwrap_or("");
    t.check(&format!("busy: top ({})", row), row.contains(" R  99.9 "));
    t.check(&format!("busy: the load ({})", top.lines().next().unwrap_or("")), top.contains("load average: 0.05, 0.01, 0.00"));
}

// limits of a callback: heap, depth; errors do not end the script unless it says so
fn limits(t: &mut T) {
    let (mut e, h) = net(1);
    let heap = "timer.after(1, fn() { let s = \"x\"; while true { s = s + s } })";
    let deep = "fn r(n) { return r(n + 1) }\ntimer.after(1, fn() { r(0) })";
    let ok = script(&mut e, h[0], "heap", heap, "\"limits\": {\"heap_kb\": 64}") && script(&mut e, h[0], "deep", deep, "\"limits\": {\"depth\": 50}");
    t.check("limits: services", ok);
    run_to(&mut e, SEC);
    let kinds = |e: &Engine, n: &str| sv(e, h[0], n).map(|s| s.errors().iter().map(|x| (x.kind, x.line)).collect::<Vec<_>>()).unwrap_or_default();
    t.eq("limits: heap", kinds(&e, "heap"), vec![(ErrorKind::Heap, 1)]);
    t.eq("limits: depth", kinds(&e, "deep"), vec![(ErrorKind::Depth, 1)]);
    let trace = sv(&e, h[0], "deep").and_then(|s| s.errors().first().map(|x| x.trace.len())).unwrap_or(0);
    t.eq("limits: the calls of the trace", trace, 50);
}

const DIV: &str = "net.udp(7, fn(s, m) {\n  let x = 1 / 0\n})\nprint(\"bereit\")";

// errors: line and column in errors(), the status, the log and the trace; stop_on_error; a
// failed start; a restart
fn errors(t: &mut T) {
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "div", DIV, "") && probe(&mut e, h[1], addr(0), 7, &[(MS, "a"), (2 * MS, "b")]);
    t.check("errors: services", ok);
    run_to(&mut e, 10 * MS);
    let s = sv(&e, h[0], "div");
    let errs = s.map(|s| s.errors()).unwrap_or_default();
    t.eq("errors: two, in line 2", errs.iter().map(|x| (x.line, x.col, x.msg.as_str())).collect::<Vec<_>>(), vec![(2, 13, "Division durch 0"), (2, 13, "Division durch 0")]);
    t.eq("errors: still running", (s.map(|s| s.state()), metric(&e, h[0], "div", "errors")), (Some(ScriptState::Running), 2));
    let status = e.services(h[0]).first().map(|x| x.status.clone()).unwrap_or_default();
    t.check(&format!("errors: the status ({})", status), status.starts_with("läuft: Rückrufe 3, CPU-Zeit ") && status.ends_with(", Fehler 2, zuletzt in Zeile 2, Spalte 13: Division durch 0"));
    t.eq("errors: the log", e.log_lines(h[0]).iter().filter(|l| l.tag == "div").map(|l| (l.sev, l.text.clone())).collect::<Vec<_>>(), vec![(3, String::from("Fehler in Zeile 2, Spalte 13 (udp): Division durch 0")); 2]);
    let n = notes(&e, h[0], |n| matches!(n, Note::ScriptError { .. }));
    t.eq("errors: the trace", n.first().map(|x| x.1.clone()), Some(Note::ScriptError { svc: String::from("div"), line: 2, col: 13, msg: String::from("Division durch 0") }));
    t.check("errors: noted with the datagram", e.trace.events().any(|x| x.dev == h[0] && matches!(x.note, Note::ScriptError { .. }) && x.hop.is_some()));
    t.eq("errors: the output", s.map(|s| String::from(s.output())), Some(String::from("bereit\n")));

    // stop_on_error: the first error ends the service
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "div", DIV, "\"stop_on_error\": true") && probe(&mut e, h[1], addr(0), 7, &[(MS, "a")]);
    t.check("stop: services", ok);
    run_to(&mut e, 10 * MS);
    let v = e.services(h[0]);
    t.eq("stop: ended", (sv(&e, h[0], "div").map(|s| s.state()), v.first().map(|x| (x.enabled, x.running))), (Some(ScriptState::Failed), Some((true, false))));
    t.eq("stop: the status", v.first().map(|x| x.status.clone()), Some(String::from("beendet nach einem Fehler in Zeile 2, Spalte 13: Division durch 0")));
    t.eq("stop: the port is closed", e.sockets(h[0]).iter().filter(|s| s.port == 7).count(), 0);
    // started again it runs (and fails again with the next datagram)
    t.check("stop: restart", e.apply(Action::ServiceRun { dev: h[0], name: String::from("div"), on: true }).is_ok());
    run_to(&mut e, 11 * MS);
    t.eq("stop: running again", (sv(&e, h[0], "div").map(|s| s.state()), e.services(h[0]).first().map(|x| x.running)), (Some(ScriptState::Running), Some(true)));

    // an error in the top-level code ends the start
    let (mut e, h) = net(1);
    t.check("start: service", script(&mut e, h[0], "bad", "let x = nil\nnet.udp(7, fn(s, m) { })\nx.y()", ""));
    run_to(&mut e, MS);
    let errs = sv(&e, h[0], "bad").map(|s| s.errors()).unwrap_or_default();
    t.eq("start: the error", errs.iter().map(|x| (x.line, x.during)).collect::<Vec<_>>(), vec![(3, "start")]);
    t.eq("start: failed", (sv(&e, h[0], "bad").map(|s| s.state()), e.services(h[0]).first().map(|x| x.running)), (Some(ScriptState::Failed), Some(false)));
    t.eq("start: its socket went", e.sockets(h[0]).len(), 0);
}

// the shell's script command
fn shell(t: &mut T) {
    let (mut e, h) = net(2);
    let echo8 = "net.udp(8, fn(s, m) { s.send(m.from, m.port, m.data) })";
    let ok = script(&mut e, h[0], "echo", echo8, "") && script(&mut e, h[0], "div", DIV, "") && probe(&mut e, h[1], addr(0), 7, &[(MS, "a")]);
    t.check("shell: services", ok);
    run_to(&mut e, 10 * MS);
    let list = sh(&mut e, h[0], "script", MS);
    let lines: Vec<&str> = list.lines().collect();
    t.eq("shell: list", lines.first().copied(), Some("Skript  Zustand  Rückrufe  Fehler  CPU-Zeit  offen"));
    let cols = |i: usize| lines.get(i).map(|l| l.split_whitespace().take(4).collect::<Vec<_>>()).unwrap_or_default();
    t.eq("shell: rows", (cols(1), cols(2)), (vec!["echo", "läuft", "1", "0"], vec!["div", "läuft", "2", "1"]));
    t.check(&format!("shell: ports ({:?})", lines), lines.len() == 3 && lines[1].ends_with(" ns     udp 8") && lines[2].ends_with(" ns     udp 7"));
    let st = sh(&mut e, h[0], "script status div", MS);
    t.check(&format!("shell: status ({})", st), st.starts_with("Skript div: läuft seit 0 ns, 4 Zeilen\n") && st.contains("(5 ns je Anweisung)") && st.contains("Grenzen je Rückruf: 10000000 Anweisungen, Tiefe 500, Stapel 100000; Speicher ") && st.contains("Letzter Fehler: 1,"));
    let er = sh(&mut e, h[0], "script errors div", MS);
    t.check(&format!("shell: errors ({})", er), er.contains("Zeile 2, Spalte 13 (udp): Division durch 0\n           in einer Funktion ohne Namen (2:13)\n"));
    t.eq("shell: no errors", sh(&mut e, h[0], "script errors echo", MS), String::from("echo: keine Fehler\n"));
    t.eq("shell: output", sh(&mut e, h[0], "script output div", MS), String::from("bereit\n"));
    t.eq("shell: no output", sh(&mut e, h[0], "script output", MS), String::from("echo: noch keine Ausgabe\n"));
    t.eq("shell: stop", (sh(&mut e, h[0], "script stop echo", MS), e.services(h[0]).iter().find(|s| s.name == "echo").map(|s| s.running)), (String::new(), Some(false)));
    t.check("shell: stopped in the list", sh(&mut e, h[0], "script list", MS).contains("echo    gestoppt"));
    t.eq("shell: start", (sh(&mut e, h[0], "script start echo", MS), e.services(h[0]).iter().find(|s| s.name == "echo").map(|s| s.running)), (String::new(), Some(true)));
    t.check("shell: restart", sh(&mut e, h[0], "script restart echo", MS).is_empty() && e.services(h[0]).iter().find(|s| s.name == "echo").is_some_and(|s| s.running));
    t.check("shell: unknown", sh(&mut e, h[0], "script frobnicate", MS).starts_with("script: frobnicate kenne ich nicht\nscript [list]"));
    t.eq("shell: not a script", sh(&mut e, h[0], "script status probe", MS), String::from("script: probe ist kein Skript dieses Geräts\n"));
    t.eq("shell: which one", sh(&mut e, h[0], "script stop", MS), String::from("script stop: welches Skript? (script stop NAME)\n"));
    t.eq("shell: none here", sh(&mut e, h[1], "script", MS), String::from("script: auf diesem Gerät läuft kein Skript\n"));
    t.check("shell: help", sh(&mut e, h[1], "script --help", MS).starts_with("script [list]  "));
    let (_, done) = e.cli_complete(h[0], "scr");
    t.eq("shell: completion", done, vec![String::from("script")]);
}

// definitions: the refusals, the source as lines, the config the script sees
fn defs(t: &mut T) {
    let refuse = |c: &str| jfn_sim::scriptsvc::from_def(&agent_core::json::parse(c.as_bytes()).unwrap_or(Json::Null)).err().unwrap_or_default();
    t.eq("defs: no source", refuse("{}"), String::from("\u{201e}source\u{201c} fehlt: der Text des Skripts."));
    t.eq("defs: source of another type", refuse("{\"source\": 7}"), String::from("\u{201e}source\u{201c} ist ein Text oder eine Liste von Zeilen."));
    t.check(&format!("defs: a syntax error ({})", refuse("{\"source\": \"let x = (1 +\"}")), refuse("{\"source\": \"let x = (1 +\"}").starts_with("Zeile 1, Spalte "));
    t.check("defs: more errors", refuse("{\"source\": \"let = 1\\nlet = 2\"}").ends_with("(und 1 weitere Fehler)"));
    t.eq("defs: CPU time", refuse("{\"source\": \"1\", \"ns_per_op\": 0}"), String::from("\u{201e}ns_per_op\u{201c} muss eine Zahl größer als 0 sein."));
    let (mut e, h) = net(1);
    let r = e.apply(Action::Service { dev: h[0], def: def("script", "x", "{\"source\": \"let = 1\"}") });
    t.check("defs: the action refuses", matches!(&r, Err(jfn_sim::ActErr::Refused(m)) if m.text().starts_with("Dienst script: Zeile 1, Spalte ")));
    let lines = def("script", "lines", "{\"source\": [\"let a = 1\", \"metrics.gauge(\\\"a\\\", a + config.extra)\"], \"extra\": 41, \"ns_per_op\": 2.5}");
    t.check("defs: lines", e.apply(Action::Service { dev: h[0], def: lines }).is_ok());
    run_to(&mut e, MS);
    t.eq("defs: config", metric(&e, h[0], "lines", "a"), 42);
    t.eq("defs: decimals", sv(&e, h[0], "lines").map(|s| (s.cfg().ps_per_op, s.source().lines().count())), Some((2500, 2)));
    t.check("defs: the kind", e.services(h[0]).first().is_some_and(|s| s.kind == "script"));
}

// the texts in English through engine.tsv
fn texts(t: &mut T) {
    let en = english();
    let (mut e, h) = net(2);
    let ok = script(&mut e, h[0], "div", DIV, "") && probe(&mut e, h[1], addr(0), 7, &[(MS, "a")]);
    t.check("texts: services", ok);
    run_to(&mut e, 10 * MS);
    let st = e.services(h[0]).first().map(|s| s.status_msg.text_with(&en)).unwrap_or_default();
    t.check(&format!("texts: status ({})", st), st.starts_with("running: callbacks 2, CPU time ") && st.ends_with(", errors 1, the last in line 2, column 13: Division durch 0"));
    let notes: Vec<Note> = vec![
        Note::ScriptBusy { svc: String::from("echo"), waiting: 2 },
        Note::ScriptDropped { svc: String::from("echo") },
        Note::ScriptError { svc: String::from("echo"), line: 3, col: 7, msg: String::from("Division durch 0") },
    ];
    let p = |_: u16| String::new();
    let got: Vec<String> = notes.iter().map(|n| n.message(&p, &p).text_with(&en)).collect();
    t.eq("texts: notes", got, ["Script echo is still computing: the event waits (2 before it).", "Script echo overloaded: datagram dropped.", "Script echo: error in line 3, column 7: Division durch 0"].map(String::from).to_vec());
    t.eq("texts: layer and drop", (notes[2].layer(), notes[1].is_drop(), notes[0].is_drop()), (7, true, false));
    let r = jfn_sim::scriptsvc::from_def_msg(&Json::Object(Vec::new())).err().map(|m| m.text_with(&en)).unwrap_or_default();
    t.eq("texts: refusal", r, String::from("\u{201c}source\u{201d} is missing: the text of the script."));
    let _ = ScriptState::Stopped;
}

const RANDOM: &str = r#"let total = 0
http.serve(80, fn(req, res) {
  let n = rand(1000)
  let i = 0
  while i < n { i += 1 }
  total += n
  metrics.observe("work", n)
  disk.put("last", {n: n, at: now()}, fn(err) { res.send(200, "${n} ${rand()}\n") })
})
timer.every(250, fn() { metrics.gauge("total", total) })
"#;

/// the project of the determinism checks: a web server whose work is random, a load generator
fn random_net() -> (Engine, Vec<u32>) {
    let (mut e, h) = net(2);
    let _ = script(&mut e, h[0], "web", RANDOM, "\"ns_per_op\": 50, \"slice\": 1000");
    let _ = add(&mut e, h[1], "http-load", "load", "{\"targets\": [\"http://10.0.0.1/\"], \"mode\": \"open\", \"rps\": 300, \"arrival\": \"poisson\", \"duration_ms\": 1500}");
    (e, h)
}

// the same project gives the same run; a clone in the middle of a callback goes on alike
fn determinism(t: &mut T) {
    let run = |cut: Option<u64>| -> (Vec<(String, i64)>, usize, Vec<String>, Vec<(String, i64)>) {
        let (mut e, h) = random_net();
        if let Some(c) = cut {
            run_to(&mut e, c);
            // the copy goes on, the original is dropped
            e = e.clone();
        }
        run_to(&mut e, 2 * SEC);
        let m = |d: u32, s: &str| e.services(d).into_iter().find(|x| x.name == s).map(|x| x.metrics.into_iter().map(|m| (String::from(&*m.name), m.value)).collect()).unwrap_or_default();
        let outs: Vec<String> = e.log_lines(h[0]).into_iter().map(|l| l.text).collect();
        (m(h[0], "web"), e.trace.hops().count(), outs, m(h[1], "load"))
    };
    let a = run(None);
    let b = run(None);
    t.check("determinism: the script worked", a.0.iter().any(|m| m.0 == "work_count" && m.1 > 300) && a.3.iter().any(|m| m.0 == "ok" && m.1 > 400));
    t.check("determinism: twice the same", a == b);
    // cut while a callback computes (slices of 1000 instructions, 50 µs)
    let mut cut = 700 * MS;
    let (mut e, h) = random_net();
    while cut < SEC {
        run_to(&mut e, cut);
        if sv(&e, h[0], "web").is_some_and(|s| s.busy(e.now())) {
            break;
        }
        cut += 37 * US;
    }
    t.check(&format!("determinism: a cut inside a callback ({} ns)", cut), cut < SEC);
    t.check("determinism: a clone in the middle goes on alike", run(Some(cut)) == a);
}
