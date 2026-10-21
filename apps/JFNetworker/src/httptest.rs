// httptest -- checks of HTTP in jfn_sim (part of --selftest): a request and its answer byte for
// byte and to the nanosecond (the handshake, the request with the ACK of the handshake, the
// answer with its Date from the server's clock), keep-alive (a reused connection, the idle
// timeout of the server, the request limit, HTTP/1.0), pipelined requests answered in their
// order, chunked answers, workers with their queue and 503 when it is full, the error rate,
// timeouts (of a request, of the connect, of a head that never comes; no route to a host that
// does not answer ARP), the load generator in a closed and an open loop with its latency
// window and per-second slots, changes while it runs (HttpLoadSet, HttpServerSet), the
// console programs curl and nc, ss, top, the IOS-like show commands and determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable and a switch between the hosts: a frame of n bytes
// takes (max(n, 60) + 24) * 8 + 50 ns per cable, so a frame without data 722 ns and over the
// switch 1444 ns.  ARP is set up by hand.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::http::{Agent, AgentEv, Done, Fail, Req, Url};
use jfn_sim::httpsvc::{decimal, exp_sample, rate_ppm};
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Engine, Hop, Metric, Note, OsApi, Report, Service, SvcEvent, TcpEnd, MS, SEC, US};
use netpkt::decode::{self, P_HTTP};
use netpkt::http::{self, Headers, Version};
use netpkt::ipv4::{Ipv4View, PROTO_TCP};
use netpkt::tcp::{TcpView, FIN, RST};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;
const SEED: u64 = 11;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn url(s: &str) -> Url {
    Url::parse(s).unwrap_or(Url { host: String::new(), port: 80, target: String::from("/") })
}

/// PC1 (10.0.0.1) and PC2 (10.0.0.2) on a switch, ARP entries set by hand
fn lan() -> (Engine, u32, u32, bool) {
    let mut p = Project::new("http", SEED);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        e.apply(Action::IfAddr { dev: pc1, iface: String::from("eth0"), prefix: pf("10.0.0.1/24"), add: true }),
        e.apply(Action::IfAddr { dev: pc2, iface: String::from("eth0"), prefix: pf("10.0.0.2/24"), add: true }),
        e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.2"), mac: Some(mac(pc2)) }),
        e.apply(Action::StaticArp { dev: pc2, ip: ip("10.0.0.1"), mac: Some(mac(pc1)) }),
    ];
    (e, pc1, pc2, ok.iter().all(|r| r.is_ok()))
}

// ---------------------------------------------------------------- a client for the checks

/// a service that sends requests of a plan through an Agent and keeps what comes back
#[derive(Clone, Debug)]
struct Client {
    plan: Vec<(u64, Req)>,
    agent: Agent,
    done: Vec<Done>,
    events: Vec<(u64, AgentEv)>,
}

impl Client {
    fn new(plan: Vec<(u64, Req)>) -> Client {
        let mut agent = Agent::new(1);
        agent.trace = true;
        Client { plan, agent, done: Vec::new(), events: Vec::new() }
    }
    fn arm(&mut self, os: &mut OsApi) {
        if let Some(t) = self.plan.iter().map(|p| p.0).min() {
            let at = t.max(os.now());
            os.timer(0, at);
        }
    }
}

impl Service for Client {
    fn kind(&self) -> &'static str {
        "client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Timer(0) = ev {
            let now = os.now();
            let (due, rest): (Vec<(u64, Req)>, Vec<(u64, Req)>) = self.plan.drain(..).partition(|p| p.0 <= now);
            self.plan = rest;
            for (_, r) in due {
                self.agent.submit(os, r);
            }
            self.arm(os);
        } else {
            self.agent.on(&ev, os);
        }
        let now = os.now();
        for e in self.agent.take_events() {
            self.events.push((now, e));
        }
        self.done.extend(self.agent.take_done());
    }
    fn metrics(&self) -> Vec<Metric> {
        vec![Metric::new("done", self.done.len() as i64)]
    }
    fn status(&self) -> String {
        format!("{} fertig", self.done.len())
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

fn client<'a>(e: &'a Engine, dev: u32, name: &str) -> Option<&'a Client> {
    e.service::<Client>(dev, name)
}

/// GET of a path on PC2, kept alive, the body kept
fn get(path: &str) -> Req {
    let mut h = Headers::new();
    h.add("User-Agent", "check/1");
    let mut r = Req::get(url(&format!("http://10.0.0.2{}", path)));
    r.headers = h;
    r
}

/// a raw TCP connection that sends bytes at times and keeps what it reads
#[derive(Clone, Debug, Default)]
struct Raw {
    plan: Vec<(u64, Vec<u8>)>,
    dst: (Ipv4, u16),
    sock: Option<u32>,
    got: Vec<u8>,
    eof: Option<u64>,
    closed: Option<(u64, TcpEnd)>,
}

impl Service for Raw {
    fn kind(&self) -> &'static str {
        "raw"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.sock = os.tcp_connect(self.dst.0, self.dst.1, jfn_sim::SockOpts::default()).ok();
        if let Some(t) = self.plan.iter().map(|p| p.0).min() {
            os.timer(0, t);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        let now = os.now();
        match ev {
            SvcEvent::Timer(0) => {
                let (due, rest): (Vec<(u64, Vec<u8>)>, Vec<(u64, Vec<u8>)>) = self.plan.drain(..).partition(|p| p.0 <= now);
                self.plan = rest;
                if let Some(s) = self.sock {
                    for (_, d) in due {
                        os.tcp_send(s, &d);
                    }
                }
                if let Some(t) = self.plan.iter().map(|p| p.0).min() {
                    os.timer(0, t);
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&d);
            }
            SvcEvent::TcpEof { sock, .. } => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&d);
                self.eof = Some(now);
            }
            SvcEvent::TcpClosed { sock, end } => {
                self.closed = Some((now, end));
                os.tcp_close(sock);
                self.sock = None;
            }
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

// ---------------------------------------------------------------- helpers

/// the TCP payloads a device sent, with their send times
fn payloads(e: &Engine, dev: u32) -> Vec<(u64, Vec<u8>)> {
    e.trace.hops().filter(|h| h.from.0 == dev).filter_map(|h| {
        let v = Ipv4View::parse(h.bytes.get(14..)?)?;
        if v.proto() != PROTO_TCP {
            return None;
        }
        let t = TcpView::parse(v.payload())?;
        (!t.payload().is_empty()).then(|| (h.t_tx, t.payload().to_vec()))
    }).collect()
}

/// the TCP segments with a flag a device sent: their send times
fn flagged(e: &Engine, dev: u32, flag: u8) -> Vec<u64> {
    e.trace.hops().filter(|h| h.from.0 == dev).filter_map(|h: &Hop| {
        let v = Ipv4View::parse(h.bytes.get(14..)?)?;
        let t = TcpView::parse(v.payload())?;
        (v.proto() == PROTO_TCP && t.has(flag)).then_some(h.t_tx)
    }).collect()
}

fn notes_of(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev && f(&x.note)).map(|x| (x.t, x.note.clone())).collect()
}

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

/// the status, body and times of a Done
fn summary(d: &Done) -> (u16, String, u64) {
    match &d.result {
        Ok(r) => (r.head.status, String::from_utf8_lossy(&r.body).into_owned(), d.end),
        Err(_) => (0, String::new(), d.end),
    }
}

/// what every device printed since the last call
fn outs(e: &mut Engine) -> BTreeMap<u32, String> {
    let mut m: BTreeMap<u32, String> = BTreeMap::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev, bytes, .. } = r {
            m.entry(dev).or_default().push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    m
}

/// a command line, the time to run, and what the device printed
fn cmd(e: &mut Engine, dev: u32, line: &str, run: u64) -> String {
    cmd_all(e, dev, line, run).remove(&dev).unwrap_or_default()
}

/// a command line, the time to run, and what every device printed
fn cmd_all(e: &mut Engine, dev: u32, line: &str, run: u64) -> BTreeMap<u32, String> {
    let _ = outs(e);
    let _ = e.cli_line(dev, line);
    let t = e.now() + run;
    e.run_until(t, BUDGET);
    outs(e)
}

pub fn run(t: &mut T) {
    units(t);
    request(t);
    keep_alive(t);
    pipelining(t);
    chunked(t);
    workers(t);
    errors(t);
    timeouts(t);
    load_closed(t);
    load_open(t);
    load_changes(t);
    curl_t(t);
    nc_t(t);
    ss_top(t);
    ios_t(t);
    determinism(t);
}

// ---------------------------------------------------------------- units

fn units(t: &mut T) {
    let v = |s: &str| agent_core::json::parse(s.as_bytes()).unwrap_or(Value::Null);
    t.eq("decimal", [decimal(&v("5"), 1000), decimal(&v("0.5"), 1000), decimal(&v("1e-3"), 1_000_000), decimal(&v("\"2.25\""), 100), decimal(&v("-1"), 1)], [Some(5000), Some(500), Some(1000), Some(225), None]);
    t.eq("rates", [rate_ppm(&v("0.01")), rate_ppm(&v("\"5%\"")), rate_ppm(&v("\"250ppm\"")), rate_ppm(&v("2"))], [Some(10_000), Some(50_000), Some(250), Some(1_000_000)]);
    // -ln(1/2) = 0.693..., -ln(1/2^32) = 22.18 times the mean at most
    t.eq("exp half", exp_sample(1_000_000, 0x7FFF_FFFF), 693_147);
    t.eq("exp largest", exp_sample(1_000_000, 0), 22_180_709);
    t.eq("exp smallest", exp_sample(1_000_000, u32::MAX), 0);
    let mut g = base::rng::Pcg32::new(1, 2);
    let n = 20_000u64;
    let sum: u64 = (0..n).map(|_| exp_sample(1_000_000, g.next_u32())).sum();
    t.check("exp mean within 2 %", (980_000..=1_020_000).contains(&(sum / n)));
    let u = |s: &str| Url::parse(s);
    t.eq("url", u("http://10.0.0.2:8080/a?b=1#x"), Ok(Url { host: String::from("10.0.0.2"), port: 8080, target: String::from("/a?b=1") }));
    t.eq("url bare", u("web.lab"), Ok(Url { host: String::from("web.lab"), port: 80, target: String::from("/") }));
    t.eq("url query", u("h?x").map(|x| x.target), Ok(String::from("/?x")));
    t.check("url errors", u("https://x/").is_err() && u("http://x:99999/").is_err() && u("http:///a").is_err() && u("http://a b/").is_err());
    t.eq("host header", (url("http://h/").host_header(), url("http://h:81/").host_header()), (String::from("h"), String::from("h:81")));
    t.eq("join", (url("http://h/a/b").join("c").map(|x| x.text()), url("http://h/a/b").join("/d").map(|x| x.text()), url("http://h/").join("http://g:81/e").map(|x| x.text())), (Some(String::from("http://h/a/c")), Some(String::from("http://h/d")), Some(String::from("http://g:81/e"))));
}

// ---------------------------------------------------------------- one request

fn request(t: &mut T) {
    let (mut e, pc1, pc2, ok) = lan();
    t.check("request lan", ok);
    t.check("server", e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"routes": [{"path": "/", "body": "hello\n"}]}"#) }).is_ok());
    t.check("client", e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, get("/"))])), true).is_ok());
    e.run_until(10 * MS, BUDGET);
    let Some(c) = client(&e, pc1, "cli") else {
        t.check("client there", false);
        return;
    };
    t.eq("one answer", c.done.len(), 1);
    let Some(d) = c.done.first().cloned() else {
        return;
    };
    t.eq("status and body", (summary(&d).0, summary(&d).1), (200, String::from("hello\n")));
    // the handshake: SYN at 1 ms, SYN-ACK 1444 ns later, ACK and request at 2888
    t.eq("times", (d.start, d.connected, d.sent), (MS, Some(MS + 2888), Some(MS + 2888)));
    let req = b"GET / HTTP/1.1\r\nHost: 10.0.0.2\r\nUser-Agent: check/1\r\n\r\n";
    let sent = payloads(&e, pc1);
    t.eq("request bytes", sent.first().map(|s| s.1.clone()), Some(req.to_vec()));
    // the request leaves after the handshake's ACK (672 ns of serialisation); 55 bytes of
    // HTTP make a frame of 109 bytes, 1114 ns per cable
    let req_tx = MS + 2888 + 672;
    t.eq("request on the wire", sent.first().map(|s| s.0), Some(req_tx));
    let arrive = req_tx + 2 * 1114;
    let answers = payloads(&e, pc2);
    t.eq("one answer segment", answers.len(), 1);
    // the server ACKs at once (quick ACK), then answers: 154 bytes, a frame of 208 bytes,
    // 1906 ns per cable
    let ans_tx = arrive + 672;
    t.eq("answer on the wire", answers.first().map(|a| a.0), Some(ans_tx));
    t.eq("answer arrives", (d.first, d.end), (Some(ans_tx + 2 * 1906), ans_tx + 2 * 1906));
    let ans = answers.first().map(|a| a.1.clone()).unwrap_or_default();
    t.eq("answer length", ans.len(), 154);
    let mut p = http::Parser::responses();
    p.expect("GET");
    let head = match p.push(&ans) {
        Ok(http::Event::Head(h)) => Some(h),
        _ => None,
    };
    t.eq("answer head", head.as_ref().map(|h| h.headers.0.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(",")), Some(String::from("Server,Date,Content-Type,Content-Length,Connection")));
    t.eq("answer values", head.as_ref().map(|h| (h.status, h.header("server").map(String::from), h.header("content-length").map(String::from), h.header("connection").map(String::from))), Some((200, Some(String::from("jfn-httpd/1.0")), Some(String::from("6")), Some(String::from("keep-alive")))));
    // the Date comes from the server's clock at the moment it answered
    let clock = e.clock_state(pc2).map(|c| c.read(ans_tx)).unwrap_or(0);
    t.eq("date", head.as_ref().and_then(|h| h.header("date").map(String::from)), Some(http::http_date(clock.div_euclid(1_000_000_000))));
    // the server's notes and numbers
    let n = notes_of(&e, pc2, |n| n.is_http());
    t.eq("server notes", n, vec![(arrive, Note::HttpRequest { method: String::from("GET"), path: String::from("/") }), (arrive, Note::HttpAnswered { status: 200, bytes: 6 })]);
    t.eq("server numbers", (metric(&e, pc2, "web", "requests"), metric(&e, pc2, "web", "responses"), metric(&e, pc2, "web", "status_2xx"), metric(&e, pc2, "web", "connections"), metric(&e, pc2, "web", "active")), (1, 1, 1, 1, 1));
    t.eq("client note", notes_of(&e, pc1, |n| n.is_http()), vec![(ans_tx + 2 * 1906, Note::HttpReply { status: 200, ms: 0 })]);
    // the frames as HTTP in the dissector, and the filter bit
    let req_hop = e.trace.hops().find(|h| h.from.0 == pc1 && h.mask & P_HTTP != 0).map(|h| decode::info(&h.bytes).1);
    t.eq("dissected request", req_hop, Some(String::from("GET / HTTP/1.1")));
    let resp_hop = e.trace.hops().find(|h| h.from.0 == pc2 && h.mask & P_HTTP != 0).map(|h| decode::info(&h.bytes).1);
    t.eq("dissected answer", resp_hop, Some(String::from("HTTP/1.1 200 OK  (text/plain)")));
    t.check("notes in the HTTP filter", e.trace.events().filter(|x| x.note.is_http()).all(|x| x.note.mask() & P_HTTP != 0 && x.note.layer() == 7));
}

// ---------------------------------------------------------------- keep-alive

fn keep_alive(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"keepalive_ms": 5000, "max_requests": 3}"#) });
    let plan = vec![(MS, get("/a")), (100 * MS, get("/b")), (200 * MS, get("/c")), (300 * MS, get("/d")), (6 * SEC, get("/e"))];
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(plan)), true);
    e.run_until(12 * SEC, BUDGET);
    let c = client(&e, pc1, "cli").cloned();
    let Some(c) = c else {
        return;
    };
    t.eq("five answers", c.done.iter().map(|d| summary(d).0).collect::<Vec<_>>(), vec![200; 5]);
    t.eq("default body", c.done.first().map(|d| summary(d).1), Some(String::from("Hallo von SRV1\n")));
    // three on the first connection (the third says close: the request limit), the fourth on
    // a new one, the fifth after the server closed the idle second one
    t.eq("connections", c.done.iter().map(|d| (d.conn, d.reused)).collect::<Vec<_>>(), vec![(Some(0), false), (Some(0), true), (Some(0), true), (Some(1), false), (Some(2), false)]);
    t.eq("connection header", c.done.iter().map(|d| d.result.as_ref().ok().and_then(|r| r.head.header("connection").map(String::from)).unwrap_or_default()).collect::<Vec<_>>(), vec![String::from("keep-alive"), String::from("keep-alive"), String::from("close"), String::from("keep-alive"), String::from("keep-alive")]);
    // the server closes an idle connection 5 s after the answer that left it idle
    let answered: Vec<u64> = notes_of(&e, pc2, |n| matches!(n, Note::HttpAnswered { .. })).iter().map(|x| x.0).collect();
    let idle: Vec<u64> = notes_of(&e, pc2, |n| *n == Note::HttpIdleClose { header: false }).iter().map(|x| x.0).collect();
    t.eq("idle closes", idle, vec![answered[3] + 5 * SEC, answered[4] + 5 * SEC]);
    t.eq("server numbers", (metric(&e, pc2, "web", "connections"), metric(&e, pc2, "web", "idle_closed"), metric(&e, pc2, "web", "requests"), metric(&e, pc2, "web", "active")), (3, 2, 5, 0));
    // HTTP/1.0 without keep-alive: Connection: close and the server closes first
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    let mut r = get("/");
    r.version = Version::Http10;
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, r)])), true);
    e.run_until(SEC, BUDGET);
    let d = client(&e, pc1, "cli").and_then(|c| c.done.first().cloned());
    t.eq("http/1.0", d.as_ref().and_then(|d| d.result.as_ref().ok().map(|r| (r.head.status, r.head.header("connection").map(String::from)))), Some((200, Some(String::from("close")))));
    t.eq("fins", (flagged(&e, pc2, FIN).len(), flagged(&e, pc1, FIN).len()), (1, 1));
    t.check("server fin first", flagged(&e, pc2, FIN).first() < flagged(&e, pc1, FIN).first());
}

// ---------------------------------------------------------------- pipelining

fn pipelining(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"routes": [{"path": "/slow", "body": "slow\n", "time_ms": 30}, {"path": "/fast", "body": "fast\n", "time_ms": 1}]}"#) });
    let mut cl = Client::new(vec![(MS, get("/slow")), (MS, get("/fast")), (MS, get("/fast"))]);
    cl.agent.pipeline = 3;
    let _ = e.add_service(pc1, "cli", Box::new(cl), true);
    e.run_until(SEC, BUDGET);
    let c = client(&e, pc1, "cli").cloned();
    let Some(c) = c else {
        return;
    };
    t.eq("answers in order", c.done.iter().map(|d| summary(d).1).collect::<Vec<_>>(), vec![String::from("slow\n"), String::from("fast\n"), String::from("fast\n")]);
    t.eq("one connection", c.done.iter().map(|d| d.conn).collect::<Vec<_>>(), vec![Some(0); 3]);
    // the three requests go in one segment; the fast answers wait for the slow one
    let sent = payloads(&e, pc1);
    t.eq("one segment", sent.len(), 1);
    let frame = e.trace.hops().find(|h| h.from.0 == pc1 && h.mask & P_HTTP != 0).map(|h| decode::info(&h.bytes).1);
    t.eq("pipelined in the dissector", frame, Some(String::from("GET /slow HTTP/1.1 , GET /fast HTTP/1.1 , GET /fast HTTP/1.1")));
    let ends: Vec<u64> = c.done.iter().map(|d| d.end).collect();
    t.check("answers together after 30 ms", ends.iter().all(|&x| x > MS + 30 * MS && x < MS + 31 * MS));
    let answers = payloads(&e, pc2);
    t.eq("answers in one segment", answers.len(), 1);
}

// ---------------------------------------------------------------- chunked

fn chunked(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"routes": [{"path": "/c", "body": "hello chunked world\n", "chunk": 6}, {"path": "/big", "size": 5000, "chunk": 1000}]}"#) });
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, get("/c")), (2 * MS, get("/big"))])), true);
    e.run_until(SEC, BUDGET);
    let c = client(&e, pc1, "cli").cloned();
    let Some(c) = c else {
        return;
    };
    t.eq("chunked body", c.done.first().map(|d| summary(d).1), Some(String::from("hello chunked world\n")));
    t.eq("chunked head", c.done.first().and_then(|d| d.result.as_ref().ok().and_then(|r| r.head.header("transfer-encoding").map(String::from))), Some(String::from("chunked")));
    let wire = payloads(&e, pc2).first().map(|p| p.1.clone()).unwrap_or_default();
    let body_at = wire.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    t.eq("chunks on the wire", String::from_utf8_lossy(&wire[body_at..]).into_owned(), String::from("6\r\nhello \r\n6\r\nchunke\r\n6\r\nd worl\r\n2\r\nd\n\r\n0\r\n\r\n"));
    t.eq("big body", c.done.get(1).map(|d| (d.body_len, summary(d).1.len())), Some((5000, 5000)));
    let fields: Vec<&'static str> = e.trace.hops().find(|h| h.from.0 == pc2 && h.mask & P_HTTP != 0).map(|h| decode::layers(&h.bytes).into_iter().filter(|l| l.proto == decode::Proto::Http).flat_map(|l| l.fields.into_iter().map(|f| f.name)).collect()).unwrap_or_default();
    t.eq("chunk fields", fields.iter().filter(|f| **f == "Data chunk").count(), 4);
    t.check("last chunk field", fields.contains(&"End of chunked encoding"));
}

// ---------------------------------------------------------------- workers, queue, 503

fn workers(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"workers": 1, "queue": 1, "time_ms": 10}"#) });
    // three requests at once on three connections
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, get("/1")), (MS, get("/2")), (MS, get("/3"))])), true);
    e.run_until(5 * MS, BUDGET);
    t.eq("in work, waiting, refused", (metric(&e, pc2, "web", "inflight"), metric(&e, pc2, "web", "queued"), metric(&e, pc2, "web", "rejected")), (1, 1, 1));
    e.run_until(SEC, BUDGET);
    let c = client(&e, pc1, "cli").cloned();
    let Some(c) = c else {
        return;
    };
    let mut st: Vec<(u16, u64)> = c.done.iter().map(|d| (summary(d).0, d.end)).collect();
    st.sort_by_key(|x| x.1);
    t.eq("statuses", st.iter().map(|x| x.0).collect::<Vec<_>>(), vec![503, 200, 200]);
    // the requests came at a and b: the first is answered at a + 10 ms, the second waits for
    // the worker and is answered at a + 20 ms
    let came: Vec<u64> = notes_of(&e, pc2, |n| matches!(n, Note::HttpRequest { .. })).iter().map(|x| x.0).collect();
    let answered: Vec<u64> = notes_of(&e, pc2, |n| matches!(n, Note::HttpAnswered { status: 200, .. })).iter().map(|x| x.0).collect();
    t.eq("worker times", answered, vec![came[0] + 10 * MS, came[0] + 20 * MS]);
    t.eq("numbers", (metric(&e, pc2, "web", "max_queued"), metric(&e, pc2, "web", "status_5xx"), metric(&e, pc2, "web", "busy_ms")), (1, 1, 20));
    t.check("notes", notes_of(&e, pc2, |n| *n == Note::HttpOverload).len() == 1 && notes_of(&e, pc2, |n| *n == Note::HttpQueued { waiting: 0 }).len() == 1);
    t.eq("retry-after", c.done.iter().find(|d| summary(d).0 == 503).and_then(|d| d.result.as_ref().ok().and_then(|r| r.head.header("retry-after").map(String::from))), Some(String::from("1")));
    // the server's window: 0 for the 503, 10 ms, and 20 ms less the head start of the first;
    // the median is the midpoint of the bucket of 10 ms ([9961472, 10223616) ns)
    let v = e.http_server_view(pc2, "web", 3);
    t.eq("server window", v.as_ref().map(|v| (v.p50, v.p99)), Some((Some(10_092_543), Some(came[0] + 20 * MS - came[1]))));
    // the slots, oldest first, up to the second of now (at 1 s: the seconds 0 and 1)
    t.eq("server slots", v.as_ref().map(|v| v.slots.iter().map(|s| (s.t, s.sent, s.done, s.errors)).collect::<Vec<_>>()), Some(vec![(0, 3, 3, 1), (SEC, 0, 0, 0)]));
}

fn errors(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"error_rate": 0.25, "error_status": 503, "routes": [{"path": "/", "body": "ok\n"}, {"path": "/always", "error_rate": 1, "body": "never\n"}]}"#) });
    let plan: Vec<(u64, Req)> = (0..40).map(|i| (MS + i * 10 * MS, get("/"))).chain([(SEC, get("/always")), (SEC + MS, get("/missing")), (SEC + 2 * MS, Req { method: String::from("POST"), ..get("/") })]).collect();
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(plan)), true);
    e.run_until(2 * SEC, BUDGET);
    let c = client(&e, pc1, "cli").cloned();
    let Some(c) = c else {
        return;
    };
    let st: Vec<u16> = c.done.iter().map(|d| summary(d).0).collect();
    t.eq("all answered", st.len(), 43);
    let errs = st[..40].iter().filter(|&&s| s == 503).count();
    t.check("about a quarter", (5..=15).contains(&errs) && st[..40].iter().all(|&s| s == 200 || s == 503));
    t.eq("route rate", st[40], 503);
    t.eq("not found", st[41], 404);
    t.eq("method", st[42], 405);
    t.eq("allow", c.done.get(42).and_then(|d| d.result.as_ref().ok().and_then(|r| r.head.header("allow").map(String::from))), Some(String::from("GET, HEAD")));
    t.eq("counted", (metric(&e, pc2, "web", "errors"), metric(&e, pc2, "web", "status_5xx"), metric(&e, pc2, "web", "status_4xx")), (errs as i64 + 1, errs as i64 + 1, 2));
    t.eq("injected notes", notes_of(&e, pc2, |n| matches!(n, Note::HttpInjected { status: 503 })).len(), errs + 1);
    // the error rate comes from the server's own random numbers: the same every run
    let again = {
        let (mut e, pc1, pc2, _) = lan();
        let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"error_rate": 0.25, "error_status": 503, "routes": [{"path": "/", "body": "ok\n"}, {"path": "/always", "error_rate": 1, "body": "never\n"}]}"#) });
        let plan: Vec<(u64, Req)> = (0..40).map(|i| (MS + i * 10 * MS, get("/"))).collect();
        let _ = e.add_service(pc1, "cli", Box::new(Client::new(plan)), true);
        e.run_until(2 * SEC, BUDGET);
        client(&e, pc1, "cli").map(|c| c.done.iter().map(|d| summary(d).0).collect::<Vec<_>>()).unwrap_or_default()
    };
    t.eq("same errors again", again, st[..40].to_vec());
}

// ---------------------------------------------------------------- timeouts

fn timeouts(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"header_timeout_ms": 2000, "routes": [{"path": "/slow", "time_ms": 100, "body": "late\n"}]}"#) });
    let mut r = get("/slow");
    r.timeout = 50 * MS;
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, r)])), true);
    e.run_until(SEC, BUDGET);
    let d = client(&e, pc1, "cli").and_then(|c| c.done.first().cloned());
    t.eq("request timeout", d.as_ref().map(|d| (d.result.clone().err(), d.end)), Some((Some(Fail::Timeout { got: 0, of: None }), 51 * MS)));
    // the client closed with FIN at the timeout; the late answer meets a closed socket: RST
    t.eq("client fin", flagged(&e, pc1, FIN).first().copied(), Some(51 * MS));
    t.check("answer reset", !flagged(&e, pc1, RST).is_empty());
    t.eq("server lost nothing it could know", metric(&e, pc2, "web", "responses"), 1);
    // a connection that sends no head is closed after the header timeout
    let raw = Raw { dst: (ip("10.0.0.2"), 80), ..Raw::default() };
    let _ = e.add_service(pc1, "raw", Box::new(raw), true);
    let t0 = e.now();
    e.run_until(t0 + 3 * SEC, BUDGET);
    let r = e.service::<Raw>(pc1, "raw").cloned().unwrap_or_default();
    let hn = notes_of(&e, pc2, |n| *n == Note::HttpIdleClose { header: true });
    t.eq("header timeout", hn.len(), 1);
    t.check("closed after 2 s", hn.first().is_some_and(|x| x.0 >= t0 + 2 * SEC && x.0 < t0 + 2 * SEC + MS) && r.eof.is_some());
    // connect timeout to a host that keeps silent (its ARP entry points into nowhere)
    let (mut e, pc1, _, _) = lan();
    let _ = e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.7"), mac: Some(Mac([2, 0, 0, 0, 0, 7])) });
    let mut r = get("/");
    r.url = url("http://10.0.0.7/");
    r.connect_timeout = 500 * MS;
    let _ = e.add_service(pc1, "cli", Box::new(Client::new(vec![(MS, r)])), true);
    e.run_until(SEC, BUDGET);
    let d = client(&e, pc1, "cli").and_then(|c| c.done.first().cloned());
    t.eq("connect timeout", d.as_ref().map(|d| (d.result.clone().err(), d.end)), Some((Some(Fail::ConnectTimeout), 501 * MS)));
    // no ARP answer at all: no route to host after the three ARP requests (Linux)
    let mut r = get("/");
    r.url = url("http://10.0.0.9/");
    let _ = e.add_service(pc1, "cli2", Box::new(Client::new(vec![(2 * SEC, r)])), true);
    e.run_until(10 * SEC, BUDGET);
    let d = client(&e, pc1, "cli2").and_then(|c| c.done.first().cloned());
    t.eq("no route to host", d.as_ref().map(|d| (d.result.clone().err(), d.end - d.start)), Some((Some(Fail::Connect(TcpEnd::Unreachable)), 3 * SEC)));
}

// ---------------------------------------------------------------- the load generator

fn load_closed(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"time_ms": 2}"#) });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "closed", "users": 2, "think_ms": 100, "requests": 10}"#) });
    e.run_until(5 * SEC, BUDGET);
    t.eq("sent and answered", (metric(&e, pc1, "load", "sent"), metric(&e, pc1, "load", "responses"), metric(&e, pc1, "load", "ok"), metric(&e, pc1, "load", "errors")), (10, 10, 10, 0));
    t.eq("two connections", metric(&e, pc1, "load", "connections"), 2);
    // a user sends, waits 2 ms and a little for the answer, thinks 100 ms: five rounds each
    let sends: Vec<u64> = notes_of(&e, pc2, |n| matches!(n, Note::HttpRequest { .. })).iter().map(|x| x.0).collect();
    t.eq("ten requests", sends.len(), 10);
    let lat: Vec<u64> = notes_of(&e, pc1, |n| matches!(n, Note::HttpReply { .. })).iter().map(|x| x.0).collect();
    t.check("rounds of about 102 ms", sends.windows(2).skip(2).all(|w| w[1] - w[0] < 103 * MS) && lat.len() == 10);
    let p50 = metric(&e, pc1, "load", "p50_us");
    t.check("latency about 2 ms", (2000..2100).contains(&p50) && metric(&e, pc1, "load", "p99_us") >= p50 && metric(&e, pc1, "load", "max_us") >= metric(&e, pc1, "load", "p99_us"));
    t.eq("server saw them", metric(&e, pc2, "web", "requests"), 10);
    let v = e.http_load_view(pc1, "load", 6);
    t.eq("view", v.as_ref().map(|v| (v.running, v.sent, v.users, v.slots.len())), Some((false, 10, 2, 6)));
    t.eq("the first second", v.as_ref().and_then(|v| v.slots.first().map(|s| (s.t, s.sent, s.done, s.errors))), Some((0, 10, 10, 0)));
    t.check("quantiles of the first second", v.as_ref().and_then(|v| v.slots.first().map(|s| s.p50.is_some() && s.p99 >= s.p50)).unwrap_or(false));
}

fn load_open(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"time_ms": 1}"#) });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 100, "duration_ms": 1000}"#) });
    e.run_until(3 * SEC, BUDGET);
    t.eq("100 in one second", (metric(&e, pc1, "load", "sent"), metric(&e, pc1, "load", "responses")), (100, 100));
    // the requests are due every 10 ms; the first one needed the handshake
    let sends: Vec<u64> = notes_of(&e, pc2, |n| matches!(n, Note::HttpRequest { .. })).iter().map(|x| x.0).collect();
    t.check("10 ms apart", sends.len() == 100 && sends.windows(2).skip(1).all(|w| w[1] - w[0] == 10 * MS));
    t.eq("rate", (metric(&e, pc1, "load", "rps"), metric(&e, pc1, "load", "rate_mrps")), (10, 100_000));
    t.eq("one connection kept", metric(&e, pc1, "load", "connections"), 1);
    // Poisson arrivals: the same count every run, about the rate
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 50, "arrival": "poisson", "duration_ms": 10000}"#) });
    e.run_until(12 * SEC, BUDGET);
    let n = metric(&e, pc1, "load", "sent");
    t.check("poisson about 500", (440..=560).contains(&n));
    // a ramp from 0 to 100 per second over 2 s: 100 arrivals, 25 in the first second
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 100, "start_rps": 0, "ramp_ms": 2000, "duration_ms": 2000}"#) });
    e.run_until(4 * SEC, BUDGET);
    let v = e.http_load_view(pc1, "load", 5);
    t.eq("ramp", v.as_ref().map(|v| v.slots.iter().map(|s| s.sent).collect::<Vec<_>>()), Some(vec![25, 75, 0, 0, 0]));
    // the k-th arrival of the ramp at sqrt(k * 2 s / 100 per s) s: 200 ms, 283 ms, 347 ms
    t.eq("ramp times", [jfn_sim::httpsvc::arrival_offset(0, 100_000, 2 * SEC, 1_000_000), jfn_sim::httpsvc::arrival_offset(0, 100_000, 2 * SEC, 2_000_000), jfn_sim::httpsvc::arrival_offset(0, 100_000, 2 * SEC, 3_000_000)], [Some(200_000_000), Some(282_843_000), Some(346_411_000)]);
    t.eq("after the ramp", jfn_sim::httpsvc::arrival_offset(0, 100_000, 2 * SEC, 101_000_000), Some(2 * SEC + 10 * MS));
    t.eq("flat and none", (jfn_sim::httpsvc::arrival_offset(500, 500, 0, 1_000_000), jfn_sim::httpsvc::arrival_offset(0, 0, 0, 1_000_000)), (Some(2 * SEC), None));
}

fn load_changes(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 10}"#) });
    e.run_until(SEC, BUDGET);
    t.check("load set", e.apply(Action::HttpLoadSet { dev: pc1, svc: String::from("load"), rate_m: Some(50_000), users: None }).is_ok());
    e.run_until(2 * SEC, BUDGET);
    let v = e.http_load_view(pc1, "load", 3);
    let slots: Vec<u64> = v.as_ref().map(|v| v.slots.iter().map(|s| s.sent).collect()).unwrap_or_default();
    t.eq("10 then 50 per second", slots, vec![10, 50, 1]);
    t.check("server set", e.apply(Action::HttpServerSet { dev: pc2, svc: String::from("web"), error_ppm: Some(1_000_000), time_ns: None, workers: None }).is_ok());
    let before = metric(&e, pc1, "load", "http_errors");
    e.run_until(3 * SEC, BUDGET);
    t.check("every answer an error now", metric(&e, pc1, "load", "http_errors") - before >= 49);
    t.check("unknown service", e.apply(Action::HttpLoadSet { dev: pc1, svc: String::from("nope"), rate_m: None, users: None }).is_err());
    t.check("wrong kind", e.apply(Action::HttpServerSet { dev: pc1, svc: String::from("load"), error_ppm: None, time_ns: None, workers: None }).is_err());
    // more users while it runs
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"time_ms": 100}"#) });
    let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "users": 1}"#) });
    e.run_until(SEC, BUDGET);
    let a = metric(&e, pc1, "load", "sent");
    let _ = e.apply(Action::HttpLoadSet { dev: pc1, svc: String::from("load"), rate_m: None, users: Some(3) });
    e.run_until(2 * SEC, BUDGET);
    let b = metric(&e, pc1, "load", "sent");
    t.check("three times as fast", (a, b - a) == (10, 30) || (b - a) >= 28);
}

// ---------------------------------------------------------------- the console tools

fn curl_t(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"routes": [{"path": "/", "body": "hello\n"}, {"path": "/r", "status": 302, "headers": {"Location": "/"}}, {"path": "/slow", "time_ms": 100, "body": "late\n"}]}"#) });
    e.run_until(MS, BUDGET);
    t.eq("plain", cmd(&mut e, pc1, "curl http://10.0.0.2/", 100 * MS), String::from("hello\n"));
    t.eq("write-out", cmd(&mut e, pc1, "curl -s -o /dev/null -w '%{http_code} %{size_download} %{num_connects}\\n' 10.0.0.2", 100 * MS), String::from("200 6 1\n"));
    let v = cmd(&mut e, pc1, "curl -v http://10.0.0.2/ http://10.0.0.2/nope", 100 * MS);
    let lines: Vec<&str> = v.lines().collect();
    t.eq("verbose start", lines.iter().take(7).copied().collect::<Vec<_>>(), vec!["*   Trying 10.0.0.2:80...", "* Connected to 10.0.0.2 (10.0.0.2) port 80", "> GET / HTTP/1.1", "> Host: 10.0.0.2", "> User-Agent: curl/8.5.0", "> Accept: */*", "> "]);
    t.check("verbose lines end in CR LF", v.contains("> GET / HTTP/1.1\r\n") && v.contains("< HTTP/1.1 200 OK\r\n"));
    t.check("verbose answer", lines.contains(&"< HTTP/1.1 200 OK") && lines.contains(&"< Content-Length: 6") && lines.contains(&"hello"));
    t.check("connection reused", lines.contains(&"* Connection #0 to host 10.0.0.2 left intact") && lines.contains(&"* Re-using existing connection with host 10.0.0.2") && lines.contains(&"404 Not Found"));
    let i = cmd(&mut e, pc1, "curl -sI http://10.0.0.2/", 100 * MS);
    t.check("head", i.starts_with("HTTP/1.1 200 OK\r\nServer: jfn-httpd/1.0\r\nDate: ") && i.ends_with("Content-Type: text/plain\r\nContent-Length: 6\r\nConnection: keep-alive\r\n\r\n"));
    t.eq("follow", cmd(&mut e, pc1, "curl -sL http://10.0.0.2/r", 100 * MS), String::from("hello\n"));
    t.eq("fail", cmd(&mut e, pc1, "curl -f http://10.0.0.2/x", 100 * MS), String::from("curl: (22) The requested URL returned error: 404\n"));
    t.eq("refused", cmd(&mut e, pc1, "curl http://10.0.0.2:81/", 100 * MS), String::from("curl: (7) Failed to connect to 10.0.0.2 port 81 after 0 ms: Couldn't connect to server\n"));
    t.eq("silent", cmd(&mut e, pc1, "curl -s http://10.0.0.2:81/", 100 * MS), String::new());
    // -m runs out while the connection is still being set up; --connect-timeout fails the connect
    t.eq("timeout", cmd(&mut e, pc1, "curl -m 0.5 http://10.0.0.7/", SEC), String::from("curl: (28) Connection timed out after 500 milliseconds\n"));
    t.eq("connect timeout", cmd(&mut e, pc1, "curl --connect-timeout 0.5 http://10.0.0.7/", SEC), String::from("curl: (28) Failed to connect to 10.0.0.7 port 80 after 500 ms: Timeout was reached\n"));
    t.eq("slow answer", cmd(&mut e, pc1, "curl -m 0.05 http://10.0.0.2/slow", SEC), String::from("curl: (28) Operation timed out after 50 milliseconds with 0 bytes received\n"));
    t.eq("no route", cmd(&mut e, pc1, "curl http://10.0.0.9/", 4 * SEC), String::from("curl: (7) Failed to connect to 10.0.0.9 port 80 after 3000 ms: Couldn't connect to server\n"));
    t.eq("no https", cmd(&mut e, pc1, "curl https://10.0.0.2/", 100 * MS), String::from("curl: (1) Protocol \"https\" not supported or disabled in libcurl\n"));
    t.eq("bad port", cmd(&mut e, pc1, "curl http://10.0.0.2:99999/", 100 * MS), String::from("curl: (3) URL rejected: Port number was not a decimal number between 0 and 65535\n"));
    t.eq("no url", cmd(&mut e, pc1, "curl", MS), String::from("curl: try 'curl --help' for more information\n"));
    t.eq("unknown option", cmd(&mut e, pc1, "curl -Q x", MS), String::from("curl: option -Q: is unknown\ncurl: try 'curl --help' for more information\n"));
    t.check("help in our words", cmd(&mut e, pc1, "curl --help", MS).starts_with("Usage: curl [options] URL...\n -d, --data DATA           send DATA in a POST request\n"));
    let m = cmd(&mut e, pc1, "curl -o /dev/null http://10.0.0.2/", 100 * MS);
    t.check("progress meter", m.starts_with("  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current\n                                 Dload  Upload   Total   Spent    Left  Speed\n\r  0     0    0     0    0     0      0      0 --:--:-- --:--:-- --:--:--     0\r100     6  100     6    0     0 ") && m.ends_with(" --:--:-- --:--:-- --:--:--  6000\n"));
    t.check("console free again", !e.cli_busy(pc1));
}

fn nc_t(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-echo", "echo", r#"{"port": 7}"#) });
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    e.run_until(MS, BUDGET);
    t.eq("connect", cmd(&mut e, pc1, "nc -v 10.0.0.2 7", 10 * MS), String::from("Connection to 10.0.0.2 7 port [tcp/echo] succeeded!\n"));
    t.check("nc busy", e.cli_busy(pc1));
    t.eq("echo", cmd(&mut e, pc1, "hello", 10 * MS), String::from("hello\n"));
    let _ = e.cli_interrupt(pc1);
    let t1 = e.now() + 10 * MS;
    e.run_until(t1, BUDGET);
    t.check("nc ended", !e.cli_busy(pc1));
    let h = cmd(&mut e, pc1, "printf 'GET / HTTP/1.0\\r\\n\\r\\n' | nc 10.0.0.2 80", 10 * MS);
    t.check("http by hand", h.starts_with("HTTP/1.1 200 OK\r\n") && h.contains("Connection: close\r\n") && h.ends_with("\r\n\r\nHallo von SRV1\n"));
    t.check("pipe done", !e.cli_busy(pc1));
    t.eq("scan", cmd(&mut e, pc1, "nc -zv 10.0.0.2 6-7", 10 * MS), String::from("nc: connect to 10.0.0.2 port 6 (tcp) failed: Connection refused\nConnection to 10.0.0.2 7 port [tcp/echo] succeeded!\n"));
    t.eq("no name", cmd(&mut e, pc1, "nc nowhere 80", 10 * MS), String::from("nc: getaddrinfo for host \"nowhere\" port 80: Name or service not known\n"));
    t.check("usage in our words", cmd(&mut e, pc1, "nc -h", MS).starts_with("nc: a network Swiss army knife of the simulator\n"));
    // a listener on SRV1 and a client on PC1: lines go both ways, the client's end ends both
    t.eq("listen", cmd(&mut e, pc2, "nc -lv 5000", 10 * MS), String::from("Listening on 0.0.0.0 5000\n"));
    let o = cmd_all(&mut e, pc1, "nc 10.0.0.2 5000", 10 * MS);
    let port = e.tcp_conns(pc1).iter().find(|r| r.remote.1 == 5000).map(|r| r.local.1).unwrap_or(0);
    t.eq("connection received", o.get(&pc2).cloned(), Some(format!("Connection received on 10.0.0.1 {}\n", port)));
    let o = cmd_all(&mut e, pc1, "ping over nc", 10 * MS);
    t.eq("to the listener", o.get(&pc2).cloned(), Some(String::from("ping over nc\n")));
    let o = cmd_all(&mut e, pc2, "pong", 10 * MS);
    t.eq("back to the client", o.get(&pc1).cloned(), Some(String::from("pong\n")));
    let _ = e.cli_interrupt(pc1);
    let t1 = e.now() + 10 * MS;
    e.run_until(t1, BUDGET);
    t.check("both ended", !e.cli_busy(pc1) && !e.cli_busy(pc2));
}

fn ss_top(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", "{}") });
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-echo", "echo", r#"{"port": 7}"#) });
    e.run_until(MS, BUDGET);
    // iproute2's layout: every column as wide as its widest entry, the very last one aside
    t.eq(
        "ss -tlnp",
        cmd(&mut e, pc2, "ss -tlnp", 0),
        String::from("State  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess                      \nLISTEN 0      128          0.0.0.0:80        0.0.0.0:*    users:((\"web\",pid=101,fd=3))\nLISTEN 0      128          0.0.0.0:7         0.0.0.0:*    users:((\"echo\",pid=102,fd=3))\n"),
    );
    t.eq("ss -tl names", cmd(&mut e, pc2, "ss -tl", 0), String::from("State  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      128          0.0.0.0:http      0.0.0.0:*          \nLISTEN 0      128          0.0.0.0:echo      0.0.0.0:*          \n"));
    let _ = cmd(&mut e, pc1, "nc 10.0.0.2 7", 10 * MS);
    let port = e.tcp_conns(pc1).iter().find(|r| r.remote.1 == 7).map(|r| r.local.1).unwrap_or(0);
    // Recv-Q and Send-Q print as "%-6d"; the address columns are right-aligned with their
    // delimiter, the ports left-aligned; the empty Process column still pads
    t.eq("ss -tn", cmd(&mut e, pc2, "ss -tn", 0), format!("State Recv-Q Send-Q Local Address:Port Peer Address:Port Process\nESTAB 0      0           10.0.0.2:7        10.0.0.1:{}       \n", port));
    t.eq("one state: no state column", cmd(&mut e, pc2, "ss -tn state established", 0), format!("Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n0      0           10.0.0.2:7        10.0.0.1:{}       \n", port));
    t.eq("filter", cmd(&mut e, pc2, "ss -tna sport = :80", 0).lines().count(), 2);
    t.eq("filter or", cmd(&mut e, pc2, "ss -tna ( sport = :80 or dport = :http )", 0).lines().count(), 2);
    t.eq("filter dst", cmd(&mut e, pc2, "ss -tn dst 10.0.0.1", 0).lines().count(), 2);
    let i = cmd(&mut e, pc2, "ss -tin", 0);
    let info = i.lines().nth(2).unwrap_or("");
    t.check("tcp internals", info.starts_with("\t reno rto:200 rtt:0.002/0.001 mss:1460 pmtu:1500 advmss:1460 cwnd:10 segs_out:") && info.ends_with(" rcv_space:14600 snd_wnd:64240"));
    t.eq("ss -s", cmd(&mut e, pc2, "ss -s", 0), String::from("Total: 3\nTCP:   3 (estab 1, closed 0, orphaned 0, timewait 0)\n\nTransport Total     IP        IPv6\nRAW\t  0         0         0        \nUDP\t  0         0         0        \nTCP\t  3         3         0        \nINET\t  3         3         0        \nFRAG\t  0         0         0        \n\n"));
    t.check("bad option", cmd(&mut e, pc2, "ss -Q", 0).starts_with("ss: invalid option -- 'Q'\n"));
    let top = cmd(&mut e, pc2, "top -b -n 1", 0);
    let lines: Vec<&str> = top.lines().collect();
    t.check("top head", lines.first().is_some_and(|l| l.starts_with("top - ") && l.ends_with(" up 0 min,  1 user,  load average: 0.00, 0.00, 0.00")));
    t.eq("top tasks", lines.get(1).copied(), Some("Tasks:   4 total,   1 running,   3 sleeping,   0 stopped,   0 zombie"));
    t.eq("top table", lines.iter().skip(4).copied().collect::<Vec<_>>(), vec!["    PID USER      PR  NI S  %CPU     TIME+ COMMAND", "      1 root      20   0 S   0.0   0:00.00 init", "    101 root      20   0 S   0.0   0:00.00 web", "    102 root      20   0 S   0.0   0:00.00 echo", "    103 root      20   0 R   0.0   0:00.00 top"]);
    // top until Ctrl+C: a screen every second
    let _ = cmd(&mut e, pc2, "top -d 1", 0);
    t.check("top runs", e.cli_busy(pc2));
    let two = cmd(&mut e, pc2, "", 2 * SEC);
    t.eq("two more screens", two.matches("\x1b[H\x1b[2Jtop - ").count(), 2);
    let _ = e.cli_interrupt(pc2);
    t.check("top ends", !e.cli_busy(pc2));
}

fn ios_t(t: &mut T) {
    let mut p = Project::new("ios", SEED);
    let pc = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc, "eth0"), &pr(r1, "gi0/0/0"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let _ = e.apply(Action::IfAddr { dev: pc, iface: String::from("eth0"), prefix: pf("10.0.0.1/24"), add: true });
    let _ = e.apply(Action::IfAddr { dev: r1, iface: String::from("Gi0/0/0"), prefix: pf("10.0.0.254/24"), add: true });
    let _ = e.apply(Action::IfUp { dev: r1, iface: String::from("Gi0/0/0"), up: true });
    let _ = e.apply(Action::Service { dev: r1, def: def("http-server", "web", "{}") });
    e.run_until(MS, BUDGET);
    let _ = cmd(&mut e, r1, "enable", 0);
    t.eq("no connections", cmd(&mut e, r1, "show tcp brief", 0), String::from("TCB       Local Address               Foreign Address             (state)\n"));
    let b = cmd(&mut e, r1, "show tcp brief all", 0);
    t.eq("listener", b.lines().nth(1).map(|l| String::from(&l[8..])), Some(String::from("  *.80                        *.*                         LISTEN")));
    // IOS drops the SYN-ACK that needs ARP: the client's SYN goes again after 1 s
    let _ = cmd(&mut e, pc, "nc 10.0.0.254 80", 2 * SEC);
    let port = e.tcp_conns(pc).first().map(|r| r.local.1).unwrap_or(0);
    let b = cmd(&mut e, r1, "show tcp brief", 0);
    t.eq("the connection", b.lines().nth(1).map(|l| String::from(&l[8..])), Some(format!("  {:<27} {:<27} ESTAB", "10.0.0.254.80", format!("10.0.0.1.{}", port))));
    let o = cmd(&mut e, r1, "show control-plane host open-ports", 0);
    t.eq("open ports", o, format!("Active internet connections (servers and established)\nProt               Local Address             Foreign Address                  Service    State\n tcp                        *:80                         *:0                      web   LISTEN\n tcp               10.0.0.254:80{:>28}                      web ESTABLIS\n", format!("10.0.0.1:{}", port)));
    let s = cmd(&mut e, r1, "show tcp statistics", 0);
    t.check("statistics", s.starts_with("Rcvd: ") && s.contains("0 Connections initiated, 1 connections accepted, 1 connections established"));
    let h = e.cli_help(r1, "show tcp ");
    t.check("help in our words", h.contains("One line per connection") && h.contains("What the device's TCP counted"));
}

fn determinism(t: &mut T) {
    let go = || {
        let (mut e, pc1, pc2, _) = lan();
        let _ = e.apply(Action::Service { dev: pc2, def: def("http-server", "web", r#"{"workers": 2, "time_ms": 5, "jitter_ms": 5, "error_rate": 0.1}"#) });
        let _ = e.apply(Action::Service { dev: pc1, def: def("http-load", "load", r#"{"targets": ["http://10.0.0.2/"], "mode": "open", "rps": 300, "arrival": "poisson", "duration_ms": 3000, "mix": [{"path": "/", "weight": 3}, {"path": "/x", "weight": 1}]}"#) });
        e.run_until(5 * SEC, BUDGET);
        let m: Vec<i64> = ["sent", "responses", "http_errors", "p50_us", "p95_us", "p99_us"].iter().map(|n| metric(&e, pc1, "load", n)).collect();
        (m, e.trace.hops().count(), e.trace.events().count())
    };
    let a = go();
    let b = go();
    t.eq("same run twice", &a, &b);
    t.check("did something", a.0[0] > 800);
    let _ = US;
}
