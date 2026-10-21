// lb -- the load balancer of the lab: HTTP (layer 7: every request goes to a backend over
// kept-alive connections and its answer back) or TCP (layer 4: every connection is spliced to
// a backend byte for byte); round robin, least connections, weighted round robin and
// consistent hashing with virtual nodes; health checks with interval, timeout and thresholds;
// backends that drain before they go; numbers per backend
//
//   kind  config (JSON; every member but "backends" may be left out)
//   lb    {"mode": "http", "port": 80, "algorithm": "round-robin", "hash": "path", "vnodes": 64,
//          "backends": [{"name": "web1", "addr": "10.0.2.11", "port": 80, "weight": 1}],
//          "health": {"type": "http", "path": "/", "interval_ms": 2000, "timeout_ms": 1000,
//          "rise": 2, "fall": 3}, "retries": 1, "timeout_ms": 10000, "connect_timeout_ms": 1000,
//          "drain_ms": 30000, "max_conns": 0, "keepalive": true, "tag": true}
//
// Algorithms ("algorithm"): "round-robin" takes the backends in turn; "least-conn" the one
// with the fewest requests (HTTP) or connections (TCP) in flight, in turn among equals;
// "weighted" is smooth weighted round robin (a backend of weight 3 gets three of every four
// picks against one of weight 1, spread out); "hash" puts the backends on a ring with "vnodes"
// tokens each and takes the first usable one from the hash of the key: "path" (HTTP), "client"
// (the client's address, the default for TCP) or "header:NAME".  A backend is usable while its
// health checks hold it up and it is neither draining nor disabled (TCP: and below max_conns).
//
// Health checks ("health"): every interval_ms, starting at once; "tcp" opens a connection,
// "http" sends GET path (2xx and 3xx count as success), "none" switches them off; a check
// fails when it gets no success within timeout_ms.  A backend goes down after "fall" failed
// checks in a row and up again after "rise" successful ones; it starts up.
//
// HTTP: a request that cannot reach its backend (refused, unreachable, connect timeout) goes
// to another one, at most "retries" times, and so does a GET or HEAD whose backend did not
// answer in time or closed the connection on it (they change nothing); then 502, 504 for a
// request that timed out at its backend, 503 when no backend is usable.  "tag" adds X-Served-By with the backend's name.
// TCP: a connection that cannot reach its backend tries the next one the same way, then
// closes.  Removing a backend (Action::LbBackend) drains it: it gets nothing new, and once
// what it has is done (or drain_ms passed) it is gone.
//
// Metrics (Service::metrics_at): requests (HTTP) or connections (TCP), responses, active,
// errors, no_backend, retries, status_2xx..5xx, backends, backends_up, p50_us, p95_us, p99_us
// (HTTP: arrival to answer; TCP: connect time), and per backend in the order of the
// configuration b1_requests, b1_active, b1_up, b1_errors .. b8_*.  Engine::lb_view gives every
// backend with its state, checks and latencies (the lab); lbctl shows it on the console.  The
// time series (series.rs) take p50_us, p95_us, p99_us of each interval from the window.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use base::hashring::{hash64, Ring};
use base::metrics::Window;
use netpkt::http::{Headers, Version};
use netpkt::Ipv4;

use crate::http::{Agent, Done, Fail, Reply, Req, ReqIn, Serve, ServeCfg, Url, Wheel};
use crate::httpsvc::decimal;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{LatWindow, LAT};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

/// timer keys: its own deadlines, the server side, the forwarding agent, the check agent
const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
const K_AGENT: u32 = 2;
const K_CHECK: u32 = 3;
/// bytes a TCP pipe buffers per direction before it stops reading
const PIPE_BUF: usize = 64 * 1024;

/// HTTP or TCP
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LbMode {
    Http,
    Tcp,
}

/// how a backend is picked
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    RoundRobin,
    LeastConn,
    Weighted,
    Hash,
}

impl Algo {
    pub fn name(self) -> &'static str {
        match self {
            Algo::RoundRobin => "round-robin",
            Algo::LeastConn => "least-conn",
            Algo::Weighted => "weighted",
            Algo::Hash => "hash",
        }
    }
    pub fn parse(s: &str) -> Option<Algo> {
        Some(match s {
            "round-robin" | "roundrobin" | "rr" => Algo::RoundRobin,
            "least-conn" | "leastconn" | "lc" => Algo::LeastConn,
            "weighted" | "wrr" => Algo::Weighted,
            "hash" | "consistent-hash" => Algo::Hash,
            _ => return None,
        })
    }
}

/// what the hash of the "hash" algorithm is made of
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HashKey {
    Path,
    Client,
    Header(String),
}

/// the kind of health check
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    None,
    Tcp,
    Http,
}

/// the health checks of the backends
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthCfg {
    pub kind: CheckKind,
    pub path: String,
    pub interval: Ns,
    pub timeout: Ns,
    pub rise: u32,
    pub fall: u32,
}

/// what the administrator wants of a backend
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admin {
    Active,
    /// gets nothing new; gone (`remove`) or disabled when what it has is done
    Draining { since: Ns, remove: bool },
    /// gets nothing (drained)
    Disabled,
    Removed,
}

/// a backend and its numbers
#[derive(Clone, Debug)]
pub struct Backend {
    pub name: String,
    pub addr: Ipv4,
    pub port: u16,
    pub weight: u32,
    pub admin: Admin,
    /// the health checks hold it up
    pub up: bool,
    /// successes and failures in a row
    pub ok_run: u32,
    pub fail_run: u32,
    /// the last check: when it ended and whether it succeeded
    pub last_check: Option<(Ns, bool)>,
    pub checks: u64,
    pub check_fails: u64,
    /// when its health or admin state last changed
    pub since: Ns,
    /// times the checks took it down
    pub downs: u32,
    /// requests (HTTP) or connections (TCP) in flight / sent to it
    pub active: u32,
    pub total: u64,
    /// requests or connections that failed at it
    pub errors: u64,
    /// answers by class 1xx .. 5xx
    pub by_class: [u64; 5],
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// HTTP: sent to answer; TCP: connect time
    lat: Window,
    /// smooth weighted round robin
    cw: i64,
    /// a check running now
    checking: bool,
    /// the wheel entry of its next check
    check_t: Option<u64>,
}

impl Backend {
    fn new(name: &str, addr: Ipv4, port: u16, weight: u32, now: Ns) -> Backend {
        Backend {
            name: String::from(name),
            addr,
            port,
            weight: weight.max(1),
            admin: Admin::Active,
            up: true,
            ok_run: 0,
            fail_run: 0,
            last_check: None,
            checks: 0,
            check_fails: 0,
            since: now,
            downs: 0,
            active: 0,
            total: 0,
            errors: 0,
            by_class: [0; 5],
            bytes_in: 0,
            bytes_out: 0,
            lat: Window::new(300, SEC),
            cw: 0,
            checking: false,
            check_t: None,
        }
    }
    fn gone(&self) -> bool {
        self.admin == Admin::Removed
    }
}

/// a change of a running load balancer's backends (Action::LbBackend)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LbOp {
    /// a new backend (or a removed one back, with this address)
    Add { name: String, addr: Ipv4, port: u16, weight: u32 },
    /// drains it, then it is gone
    Remove { name: String },
    /// drains it, then it is disabled
    Drain { name: String },
    /// takes a draining or disabled backend back
    Enable { name: String },
    Weight { name: String, weight: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LTimer {
    /// the next health check of a backend
    Check(usize),
    /// a TCP check takes too long (its socket)
    CheckTimeout(u32),
    /// a TCP pipe's connect takes too long
    Connect(u32),
    /// the drain time of a backend is over
    Drain(usize),
}

/// an HTTP request on its way to a backend
#[derive(Clone, Debug)]
struct Fwd {
    req: ReqIn,
    key: u64,
    b: usize,
    tried: Vec<usize>,
    sent: Ns,
}

/// a TCP connection spliced to a backend
#[derive(Clone, Debug)]
struct Pipe {
    client: u32,
    back: Option<u32>,
    b: usize,
    key: u64,
    tried: Vec<usize>,
    connected: bool,
    /// bytes of the client for the backend, of the backend for the client
    up: Vec<u8>,
    down: Vec<u8>,
    c_eof: bool,
    b_eof: bool,
    c_shut: bool,
    b_shut: bool,
    c_closed: bool,
    b_closed: bool,
    start: Ns,
    timer: Option<u64>,
    /// counted as active at its backend
    counted: bool,
}

/// the load balancer
#[derive(Clone, Debug)]
pub struct Lb {
    pub mode: LbMode,
    pub port: u16,
    pub algo: Algo,
    pub hash_key: HashKey,
    pub vnodes: u32,
    pub health: HealthCfg,
    pub retries: u32,
    pub timeout: Ns,
    pub connect_timeout: Ns,
    pub drain: Ns,
    pub max_conns: u32,
    pub keepalive: bool,
    pub tag: bool,
    pub backends: Vec<Backend>,
    ring: Ring<u32>,
    turn: usize,
    serve: Serve,
    agent: Agent,
    checker: Agent,
    listener: Option<u32>,
    wheel: Wheel<LTimer>,
    fwd: BTreeMap<u64, Fwd>,
    by_agent: BTreeMap<u64, u64>,
    hchecks: BTreeMap<u64, usize>,
    tchecks: BTreeMap<u32, (usize, u64)>,
    pipes: BTreeMap<u32, Pipe>,
    socks: BTreeMap<u32, (u32, bool)>,
    next_pipe: u32,
    pub requests: u64,
    pub responses: u64,
    pub errors: u64,
    pub no_backend: u64,
    pub retried: u64,
    pub by_class: [u64; 5],
    lat: Window,
    seen: Ns,
}

/// the names of the per-backend metrics (by position)
const M_REQ: [&str; 8] = ["b1_requests", "b2_requests", "b3_requests", "b4_requests", "b5_requests", "b6_requests", "b7_requests", "b8_requests"];
const M_ACT: [&str; 8] = ["b1_active", "b2_active", "b3_active", "b4_active", "b5_active", "b6_active", "b7_active", "b8_active"];
const M_UP: [&str; 8] = ["b1_up", "b2_up", "b3_up", "b4_up", "b5_up", "b6_up", "b7_up", "b8_up"];
const M_ERR: [&str; 8] = ["b1_errors", "b2_errors", "b3_errors", "b4_errors", "b5_errors", "b6_errors", "b7_errors", "b8_errors"];

/// headers that belong to one connection, not to the message
fn hop_by_hop(name: &str) -> bool {
    ["connection", "keep-alive", "proxy-connection", "transfer-encoding", "te", "trailer", "upgrade", "content-length"].iter().any(|h| name.eq_ignore_ascii_case(h))
}

impl Lb {
    pub fn new(mode: LbMode, port: u16) -> Lb {
        let cfg = ServeCfg { port, server: String::new(), max_requests: 0, max_body: 64 << 20, ..ServeCfg::default() };
        Lb {
            mode,
            port,
            algo: Algo::RoundRobin,
            hash_key: if mode == LbMode::Http { HashKey::Path } else { HashKey::Client },
            vnodes: 64,
            health: HealthCfg { kind: if mode == LbMode::Http { CheckKind::Http } else { CheckKind::Tcp }, path: String::from("/"), interval: 2 * SEC, timeout: SEC, rise: 2, fall: 3 },
            retries: 1,
            timeout: 10 * SEC,
            connect_timeout: SEC,
            drain: 30 * SEC,
            max_conns: 0,
            keepalive: true,
            tag: true,
            backends: Vec::new(),
            ring: Ring::new(),
            turn: 0,
            serve: Serve::new(cfg, K_SERVE),
            agent: Agent::new(K_AGENT),
            checker: Agent::new(K_CHECK),
            listener: None,
            wheel: Wheel::new(K_OWN),
            fwd: BTreeMap::new(),
            by_agent: BTreeMap::new(),
            hchecks: BTreeMap::new(),
            tchecks: BTreeMap::new(),
            pipes: BTreeMap::new(),
            socks: BTreeMap::new(),
            next_pipe: 0,
            requests: 0,
            responses: 0,
            errors: 0,
            no_backend: 0,
            retried: 0,
            by_class: [0; 5],
            lat: Window::new(300, SEC),
            seen: 0,
        }
    }

    /// adds a backend to the configuration (before it runs)
    pub fn add_backend(&mut self, name: &str, addr: Ipv4, port: u16, weight: u32) {
        self.backends.push(Backend::new(name, addr, port, weight, 0));
        self.rering();
    }

    fn rering(&mut self) {
        let mut r = Ring::new();
        for (i, b) in self.backends.iter().enumerate() {
            if !b.gone() {
                r.add(i as u32, &b.name, self.vnodes);
            }
        }
        self.ring = r;
    }

    /// the backend takes something new
    fn usable(&self, i: usize) -> bool {
        let b = &self.backends[i];
        b.admin == Admin::Active && b.up && (self.mode == LbMode::Http || self.max_conns == 0 || b.active < self.max_conns)
    }

    /// a backend by the algorithm, none of `skip`
    fn pick(&mut self, key: u64, skip: &[usize]) -> Option<usize> {
        let n = self.backends.len();
        let ok: Vec<bool> = (0..n).map(|i| self.usable(i) && !skip.contains(&i)).collect();
        if !ok.iter().any(|&x| x) {
            return None;
        }
        match self.algo {
            Algo::RoundRobin => {
                for k in 0..n {
                    let i = (self.turn + k) % n;
                    if ok[i] {
                        self.turn = i + 1;
                        return Some(i);
                    }
                }
                None
            }
            Algo::LeastConn => {
                let mut best: Option<usize> = None;
                for k in 0..n {
                    let i = (self.turn + k) % n;
                    if ok[i] && best.is_none_or(|b| self.backends[i].active < self.backends[b].active) {
                        best = Some(i);
                    }
                }
                if let Some(b) = best {
                    self.turn = b + 1;
                }
                best
            }
            Algo::Weighted => {
                let total: i64 = (0..n).filter(|&i| ok[i]).map(|i| self.backends[i].weight as i64).sum();
                let mut best: Option<usize> = None;
                for i in 0..n {
                    if !ok[i] {
                        continue;
                    }
                    self.backends[i].cw += self.backends[i].weight as i64;
                    let cw = self.backends[i].cw;
                    if best.is_none_or(|x| cw > self.backends[x].cw) {
                        best = Some(i);
                    }
                }
                if let Some(x) = best {
                    self.backends[x].cw -= total;
                }
                best
            }
            Algo::Hash => self.ring.preference_by(key, 1, |m| ok.get(*m as usize).copied().unwrap_or(false)).first().map(|m| *m as usize),
        }
    }

    fn key_of(&self, r: &ReqIn) -> u64 {
        match &self.hash_key {
            HashKey::Path => hash64(r.head.target.as_bytes()),
            HashKey::Client => hash64(&r.peer.0.0),
            HashKey::Header(h) => hash64(r.head.header(h).unwrap_or("").as_bytes()),
        }
    }

    // ---------------------------------------------------------------- health checks

    fn start_check(&mut self, os: &mut OsApi, i: usize) {
        let now = os.now();
        let b = &self.backends[i];
        if b.gone() || b.checking || self.health.kind == CheckKind::None {
            return;
        }
        let (addr, port) = (b.addr, b.port);
        self.backends[i].checking = true;
        match self.health.kind {
            CheckKind::Http => {
                let url = Url { host: format!("{}", addr), port, target: self.health.path.clone() };
                let mut h = Headers::new();
                h.add("User-Agent", "jfn-lb-check/1.0");
                let req = Req { method: String::from("GET"), url, version: Version::Http11, headers: h, body: Vec::new(), timeout: self.health.timeout, connect_timeout: self.health.timeout, reuse: false, keep_body: false };
                let id = self.checker.submit(os, req);
                self.hchecks.insert(id, i);
            }
            CheckKind::Tcp => match os.tcp_connect(addr, port, SockOpts::default()) {
                Ok(s) => {
                    let t = self.wheel.add(now + self.health.timeout, LTimer::CheckTimeout(s));
                    self.tchecks.insert(s, (i, t));
                }
                Err(_) => self.check_done(os, i, false),
            },
            CheckKind::None => {}
        }
    }

    /// a check of backend i ended
    fn check_done(&mut self, os: &mut OsApi, i: usize, ok: bool) {
        let now = os.now();
        let (rise, fall) = (self.health.rise.max(1), self.health.fall.max(1));
        let Some(b) = self.backends.get_mut(i) else {
            return;
        };
        b.checking = false;
        b.checks += 1;
        b.last_check = Some((now, ok));
        let mut change = None;
        if ok {
            b.fail_run = 0;
            b.ok_run += 1;
            if !b.up && b.ok_run >= rise {
                b.up = true;
                b.since = now;
                change = Some(true);
            }
        } else {
            b.check_fails += 1;
            b.ok_run = 0;
            b.fail_run += 1;
            if b.up && b.fail_run >= fall {
                b.up = false;
                b.since = now;
                b.downs += 1;
                change = Some(false);
            }
        }
        if let Some(up) = change {
            let name = b.name.clone();
            let n = if up { b.ok_run } else { b.fail_run };
            os.note(Note::LbHealth { backend: name.clone(), up });
            let text = if up { format!("Backend {} ist wieder verfügbar ({} Prüfungen erfolgreich)", name, n) } else { format!("Backend {} fällt aus ({} Prüfungen ohne Erfolg)", name, n) };
            os.log(if up { 5 } else { 4 }, "lb", &text);
        }
    }

    // ---------------------------------------------------------------- HTTP

    fn arrive(&mut self, os: &mut OsApi, r: ReqIn) {
        self.requests += 1;
        let key = self.key_of(&r);
        match self.pick(key, &[]) {
            Some(b) => self.forward(os, r, key, b, Vec::new()),
            None => {
                self.no_backend += 1;
                os.note(Note::LbNoBackend);
                self.reply_error(os, &r, 503);
            }
        }
    }

    fn forward(&mut self, os: &mut OsApi, r: ReqIn, key: u64, b: usize, tried: Vec<usize>) {
        let now = os.now();
        let be = &self.backends[b];
        let mut h = Headers::new();
        for (k, v) in &r.head.headers.0 {
            if !hop_by_hop(k) && !k.eq_ignore_ascii_case("x-forwarded-for") {
                h.add(k, v);
            }
        }
        let xff = match r.head.header("x-forwarded-for") {
            Some(x) => format!("{}, {}", x, r.peer.0),
            None => format!("{}", r.peer.0),
        };
        h.add("X-Forwarded-For", &xff);
        let url = Url { host: format!("{}", be.addr), port: be.port, target: r.head.target.clone() };
        let req = Req { method: r.head.method.clone(), url, version: Version::Http11, headers: h, body: r.body.clone(), timeout: self.timeout, connect_timeout: self.connect_timeout, reuse: self.keepalive, keep_body: true };
        os.note(Note::LbPick { backend: be.name.clone(), algo: self.algo.name() });
        let aid = self.agent.submit(os, req);
        let be = &mut self.backends[b];
        be.active += 1;
        be.total += 1;
        be.bytes_out += r.body.len() as u64;
        let sid = r.id;
        self.by_agent.insert(aid, sid);
        self.fwd.insert(sid, Fwd { req: r, key, b, tried, sent: now });
    }

    fn reply_error(&mut self, os: &mut OsApi, r: &ReqIn, status: u16) {
        let now = os.now();
        self.errors += 1;
        let mut rep = Reply::status_page(status);
        if status == 503 {
            rep.headers.add("Retry-After", "1");
        }
        if self.serve.respond(os, r.id, rep) {
            self.responses += 1;
            self.by_class[4] += 1;
            self.lat.record(now, now - r.at);
        }
    }

    fn answered(&mut self, os: &mut OsApi, d: Done) {
        let now = os.now();
        let Some(sid) = self.by_agent.remove(&d.id) else {
            return;
        };
        let Some(mut f) = self.fwd.remove(&sid) else {
            return;
        };
        let b = f.b;
        if let Some(be) = self.backends.get_mut(b) {
            be.active = be.active.saturating_sub(1);
        }
        match d.result {
            Ok(resp) => {
                let st = resp.head.status;
                let class = ((st / 100).clamp(1, 5) - 1) as usize;
                if let Some(be) = self.backends.get_mut(b) {
                    be.by_class[class] += 1;
                    be.bytes_in += resp.body.len() as u64;
                    be.lat.record(now, now - f.sent);
                    if st >= 500 {
                        be.errors += 1;
                    }
                }
                let mut h = Headers::new();
                for (k, v) in &resp.head.headers.0 {
                    if !hop_by_hop(k) {
                        h.add(k, v);
                    }
                }
                if self.tag
                    && let Some(be) = self.backends.get(b)
                {
                    h.set("X-Served-By", &be.name);
                }
                let rep = Reply { status: st, headers: h, body: resp.body, chunk: 0, close: false };
                if self.serve.respond(os, sid, rep) {
                    self.responses += 1;
                    self.by_class[class] += 1;
                    self.lat.record(now, now - f.req.at);
                }
            }
            Err(fail) => {
                if let Some(be) = self.backends.get_mut(b) {
                    be.errors += 1;
                }
                let idempotent = matches!(f.req.head.method.as_str(), "GET" | "HEAD");
                let retry = matches!(fail, Fail::Connect(_) | Fail::NoRoute(_) | Fail::ConnectTimeout) || (idempotent && matches!(fail, Fail::Timeout { .. } | Fail::Reset(_) | Fail::Empty));
                f.tried.push(b);
                if retry && f.tried.len() <= self.retries as usize {
                    if let Some(nb) = self.pick(f.key, &f.tried) {
                        self.retried += 1;
                        let name = self.backends[b].name.clone();
                        os.note(Note::LbRetry { backend: name });
                        self.forward(os, f.req, f.key, nb, f.tried);
                        self.drained(os, b);
                        return;
                    }
                }
                let status = if matches!(fail, Fail::Timeout { .. }) { 504 } else { 502 };
                self.reply_error(os, &f.req, status);
            }
        }
        self.drained(os, b);
    }

    // ---------------------------------------------------------------- TCP

    fn accept(&mut self, os: &mut OsApi, listener: u32) {
        while let Some(c) = os.tcp_accept(listener) {
            let now = os.now();
            let peer = os.tcp_info(c).map(|r| r.remote.0).unwrap_or(Ipv4::UNSPECIFIED);
            self.requests += 1;
            self.next_pipe += 1;
            let id = self.next_pipe;
            let key = match &self.hash_key {
                HashKey::Client => hash64(&peer.0),
                _ => hash64(&id.to_be_bytes()),
            };
            self.socks.insert(c, (id, false));
            self.pipes.insert(id, Pipe { client: c, back: None, b: 0, key, tried: Vec::new(), connected: false, up: Vec::new(), down: Vec::new(), c_eof: false, b_eof: false, c_shut: false, b_shut: false, c_closed: false, b_closed: false, start: now, timer: None, counted: false });
            self.pipe_connect(os, id);
        }
    }

    /// connects a pipe to the next backend, or gives up
    fn pipe_connect(&mut self, os: &mut OsApi, id: u32) {
        let now = os.now();
        let Some(p) = self.pipes.get(&id) else {
            return;
        };
        let (key, tried, client) = (p.key, p.tried.clone(), p.client);
        let first = tried.is_empty();
        let pick = if first || tried.len() <= self.retries as usize { self.pick(key, &tried) } else { None };
        let Some(b) = pick else {
            if first {
                self.no_backend += 1;
                os.note(Note::LbNoBackend);
            }
            self.errors += 1;
            os.tcp_abort(client);
            self.socks.remove(&client);
            self.pipes.remove(&id);
            return;
        };
        let (addr, port, name) = (self.backends[b].addr, self.backends[b].port, self.backends[b].name.clone());
        if !first
            && let Some(prev) = tried.last()
        {
            os.note(Note::LbRetry { backend: self.backends[*prev].name.clone() });
            self.retried += 1;
        }
        os.note(Note::LbPick { backend: name, algo: self.algo.name() });
        match os.tcp_connect(addr, port, SockOpts::default()) {
            Ok(s) => {
                let t = self.wheel.add(now + self.connect_timeout, LTimer::Connect(id));
                let be = &mut self.backends[b];
                be.active += 1;
                be.total += 1;
                if let Some(p) = self.pipes.get_mut(&id) {
                    p.back = Some(s);
                    p.b = b;
                    p.timer = Some(t);
                    p.counted = true;
                    p.start = now;
                }
                self.socks.insert(s, (id, true));
            }
            Err(_) => {
                self.backends[b].errors += 1;
                if let Some(p) = self.pipes.get_mut(&id) {
                    p.tried.push(b);
                }
                self.pipe_connect(os, id);
            }
        }
    }

    /// the backend connection of a pipe failed before it was set up: the next backend
    fn pipe_retry(&mut self, os: &mut OsApi, id: u32) {
        let Some(p) = self.pipes.get_mut(&id) else {
            return;
        };
        let b = p.b;
        if let Some(t) = p.timer.take() {
            self.wheel.cancel(t);
        }
        if let Some(s) = p.back.take() {
            self.socks.remove(&s);
            os.tcp_close(s);
        }
        p.tried.push(b);
        if p.counted {
            p.counted = false;
            if let Some(be) = self.backends.get_mut(b) {
                be.active = be.active.saturating_sub(1);
                be.errors += 1;
            }
        }
        self.pipe_connect(os, id);
        self.drained(os, b);
    }

    /// moves what can go: client bytes to the backend, backend bytes to the client, FINs
    fn pump(&mut self, os: &mut OsApi, id: u32) {
        let Some(p) = self.pipes.get_mut(&id) else {
            return;
        };
        let b = p.b;
        let (mut to_back, mut to_client) = (0u64, 0u64);
        if !p.c_closed && p.up.len() < PIPE_BUF {
            let d = os.tcp_recv(p.client, PIPE_BUF - p.up.len());
            p.up.extend_from_slice(&d);
        }
        if p.connected && !p.b_closed {
            if let Some(s) = p.back
                && p.down.len() < PIPE_BUF
            {
                let d = os.tcp_recv(s, PIPE_BUF - p.down.len());
                p.down.extend_from_slice(&d);
            }
            if let Some(s) = p.back
                && !p.up.is_empty()
            {
                let n = os.tcp_send(s, &p.up);
                p.up.drain(..n);
                to_back = n as u64;
            }
            if p.up.is_empty()
                && p.c_eof
                && !p.b_shut
                && let Some(s) = p.back
            {
                p.b_shut = true;
                os.tcp_shutdown(s);
            }
        }
        if !p.c_closed && !p.down.is_empty() {
            let n = os.tcp_send(p.client, &p.down);
            p.down.drain(..n);
            to_client = n as u64;
        }
        if p.down.is_empty() && p.b_eof && !p.c_shut && !p.c_closed {
            p.c_shut = true;
            os.tcp_shutdown(p.client);
        }
        if let Some(be) = self.backends.get_mut(b) {
            be.bytes_out += to_back;
            be.bytes_in += to_client;
        }
    }

    /// a socket of a pipe closed
    fn pipe_closed(&mut self, os: &mut OsApi, sock: u32, back: bool, end: TcpEnd) {
        let Some((id, b)) = self.pipes.iter().find(|(_, p)| if back { p.back == Some(sock) } else { p.client == sock }).map(|(i, p)| (*i, p.b)) else {
            self.socks.remove(&sock);
            os.tcp_close(sock);
            return;
        };
        let connected = self.pipes.get(&id).is_some_and(|p| p.connected);
        if back && !connected {
            // refused or unreachable: the next backend (pipe_retry closes the socket)
            self.pipe_retry(os, id);
            return;
        }
        let reset = !matches!(end, TcpEnd::Closed);
        if back
            && !reset
            && let Some(p) = self.pipes.get_mut(&id)
        {
            // an orderly end: what the backend sent last still goes to the client, then its
            // FIN; the pipe ends when the client's side closes
            let rest = os.tcp_recv(sock, usize::MAX);
            p.down.extend_from_slice(&rest);
            p.b_eof = true;
            p.b_closed = true;
            self.socks.remove(&sock);
            os.tcp_close(sock);
            let done = p.c_closed;
            self.pump(os, id);
            if done {
                self.pipe_end(os, id, b);
            }
            return;
        }
        self.socks.remove(&sock);
        os.tcp_close(sock);
        let mut finished = false;
        if let Some(p) = self.pipes.get_mut(&id) {
            if back {
                // reset: the client's connection is reset too
                p.b_closed = true;
                if !p.c_closed {
                    os.tcp_abort(p.client);
                    p.c_closed = true;
                    self.socks.remove(&p.client);
                }
            } else {
                p.c_closed = true;
                if let Some(s) = p.back
                    && !p.b_closed
                {
                    if reset || !p.up.is_empty() {
                        os.tcp_abort(s);
                    } else {
                        os.tcp_close(s);
                    }
                    p.b_closed = true;
                    self.socks.remove(&s);
                }
            }
            if p.c_closed && p.b_closed {
                finished = true;
            }
        }
        if finished {
            self.pipe_end(os, id, b);
        }
    }

    fn pipe_end(&mut self, os: &mut OsApi, id: u32, b: usize) {
        if let Some(p) = self.pipes.remove(&id) {
            if let Some(t) = p.timer {
                self.wheel.cancel(t);
            }
            if p.counted
                && let Some(be) = self.backends.get_mut(b)
            {
                be.active = be.active.saturating_sub(1);
            }
            self.responses += 1;
        }
        self.drained(os, b);
    }

    /// closes the pipes of a backend whose drain time ran out
    fn cut_pipes(&mut self, os: &mut OsApi, b: usize) {
        let ids: Vec<u32> = self.pipes.iter().filter(|(_, p)| p.b == b && p.back.is_some()).map(|(i, _)| *i).collect();
        for id in ids {
            if let Some(p) = self.pipes.get(&id) {
                if !p.c_closed {
                    os.tcp_abort(p.client);
                    self.socks.remove(&p.client);
                }
                if let Some(s) = p.back
                    && !p.b_closed
                {
                    os.tcp_abort(s);
                    self.socks.remove(&s);
                }
            }
            self.pipe_end(os, id, b);
        }
    }

    // ---------------------------------------------------------------- draining

    /// a draining backend with nothing left is gone (or disabled)
    fn drained(&mut self, os: &mut OsApi, b: usize) {
        let Some(be) = self.backends.get(b) else {
            return;
        };
        if let Admin::Draining { remove, .. } = be.admin
            && be.active == 0
        {
            self.finish_drain(os, b, remove);
        }
    }

    fn finish_drain(&mut self, os: &mut OsApi, b: usize, remove: bool) {
        let now = os.now();
        let Some(be) = self.backends.get_mut(b) else {
            return;
        };
        be.admin = if remove { Admin::Removed } else { Admin::Disabled };
        be.since = now;
        let name = be.name.clone();
        os.note(Note::LbDrained { backend: name.clone() });
        os.log(5, "lb", &format!("Backend {} ist leer gelaufen und {}", name, if remove { "entfernt" } else { "abgeschaltet" }));
        if remove {
            self.rering();
        }
    }

    /// changes the backends while it runs (Action::LbBackend)
    pub fn change(&mut self, os: &mut OsApi, op: LbOp) -> Result<(), Msg> {
        let now = os.now();
        let find = |s: &Lb, n: &str| s.backends.iter().position(|b| b.name.eq_ignore_ascii_case(n));
        match op {
            LbOp::Add { name, addr, port, weight } => {
                match find(self, &name) {
                    Some(i) if !self.backends[i].gone() => return Err(msg!("Das Backend {} gibt es schon.", name)),
                    Some(i) => {
                        // its old chain of checks ends here
                        if let Some(t) = self.backends[i].check_t {
                            self.wheel.cancel(t);
                        }
                        let mut b = Backend::new(&name, addr, port, weight, now);
                        b.downs = self.backends[i].downs;
                        self.backends[i] = b;
                    }
                    None => self.backends.push(Backend::new(&name, addr, port, weight, now)),
                }
                self.rering();
                let i = find(self, &name).unwrap_or(0);
                if self.health.kind != CheckKind::None {
                    self.backends[i].check_t = Some(self.wheel.add(now, LTimer::Check(i)));
                }
                os.log(5, "lb", &format!("Backend {} ({}:{}) hinzugefügt", name, addr, port));
            }
            LbOp::Remove { name } | LbOp::Drain { name } if find(self, &name).is_none_or(|i| self.backends[i].gone()) => {
                return Err(msg!("Das Backend {} gibt es nicht.", name));
            }
            LbOp::Remove { ref name } | LbOp::Drain { ref name } => {
                let remove = matches!(op, LbOp::Remove { .. });
                let i = find(self, name).unwrap_or(0);
                let be = &mut self.backends[i];
                if be.admin == Admin::Disabled && remove {
                    be.admin = Admin::Removed;
                    be.since = now;
                    self.rering();
                } else if be.admin != Admin::Disabled {
                    be.admin = Admin::Draining { since: now, remove };
                    be.since = now;
                    os.log(5, "lb", &format!("Backend {} leert sich", be.name));
                    self.wheel.add(now + self.drain, LTimer::Drain(i));
                    self.drained(os, i);
                }
            }
            LbOp::Enable { name } => {
                let i = find(self, &name).filter(|&i| !self.backends[i].gone()).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].admin = Admin::Active;
                self.backends[i].since = now;
            }
            LbOp::Weight { name, weight } => {
                let i = find(self, &name).filter(|&i| !self.backends[i].gone()).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].weight = weight.max(1);
            }
        }
        self.wheel.arm(os);
        Ok(())
    }

    /// changes the backends of a load balancer that does not run (they count from its start)
    pub fn change_stopped(&mut self, op: LbOp) -> Result<(), Msg> {
        let find = |s: &Lb, n: &str| s.backends.iter().position(|b| b.name.eq_ignore_ascii_case(n) && !b.gone());
        match op {
            LbOp::Add { name, addr, port, weight } => {
                if find(self, &name).is_some() {
                    return Err(msg!("Das Backend {} gibt es schon.", name));
                }
                match self.backends.iter().position(|b| b.name.eq_ignore_ascii_case(&name)) {
                    Some(i) => self.backends[i] = Backend::new(&name, addr, port, weight, 0),
                    None => self.backends.push(Backend::new(&name, addr, port, weight, 0)),
                }
            }
            LbOp::Remove { name } => {
                let i = find(self, &name).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].admin = Admin::Removed;
            }
            LbOp::Drain { name } => {
                let i = find(self, &name).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].admin = Admin::Disabled;
            }
            LbOp::Enable { name } => {
                let i = find(self, &name).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].admin = Admin::Active;
            }
            LbOp::Weight { name, weight } => {
                let i = find(self, &name).ok_or_else(|| msg!("Das Backend {} gibt es nicht.", name))?;
                self.backends[i].weight = weight.max(1);
            }
        }
        self.rering();
        Ok(())
    }

    /// the algorithm while it runs (Action::LbAlgorithm)
    pub fn set_algo(&mut self, algo: Algo) {
        self.algo = algo;
        for b in self.backends.iter_mut() {
            b.cw = 0;
        }
    }

    // ---------------------------------------------------------------- timers

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                LTimer::Check(i) => {
                    if self.backends.get(i).is_some_and(|b| !b.gone()) && self.health.kind != CheckKind::None {
                        self.start_check(os, i);
                        let t = self.wheel.add(now + self.health.interval, LTimer::Check(i));
                        self.backends[i].check_t = Some(t);
                    }
                }
                LTimer::CheckTimeout(s) => {
                    if let Some((i, _)) = self.tchecks.remove(&s) {
                        os.tcp_abort(s);
                        self.check_done(os, i, false);
                    }
                }
                LTimer::Connect(id) => {
                    let late = self.pipes.get(&id).is_some_and(|p| !p.connected && p.back.is_some());
                    if late {
                        if let Some(p) = self.pipes.get(&id)
                            && let Some(s) = p.back
                        {
                            os.tcp_abort(s);
                        }
                        self.pipe_retry(os, id);
                    }
                }
                LTimer::Drain(i) => {
                    if let Some(Admin::Draining { since, remove }) = self.backends.get(i).map(|b| b.admin)
                        && since + self.drain <= now
                    {
                        if self.mode == LbMode::Tcp {
                            self.cut_pipes(os, i);
                        }
                        if matches!(self.backends.get(i).map(|b| b.admin), Some(Admin::Draining { .. })) {
                            self.finish_drain(os, i, remove);
                        }
                    }
                }
            }
        }
        self.wheel.arm(os);
    }

    /// TCP events of check sockets, pipes and the listener; true when it was one of them
    fn tcp_event(&mut self, os: &mut OsApi, ev: &SvcEvent) -> bool {
        match *ev {
            SvcEvent::TcpConnected { sock, .. } if self.tchecks.contains_key(&sock) => {
                if let Some((i, t)) = self.tchecks.remove(&sock) {
                    self.wheel.cancel(t);
                    os.tcp_abort(sock);
                    self.check_done(os, i, true);
                }
                true
            }
            SvcEvent::TcpClosed { sock, .. } if self.tchecks.contains_key(&sock) => {
                if let Some((i, t)) = self.tchecks.remove(&sock) {
                    self.wheel.cancel(t);
                    os.tcp_close(sock);
                    self.check_done(os, i, false);
                }
                true
            }
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                self.accept(os, listener);
                true
            }
            SvcEvent::TcpConnected { sock, .. } if self.socks.contains_key(&sock) => {
                let now = os.now();
                if let Some(&(id, _)) = self.socks.get(&sock)
                    && let Some(p) = self.pipes.get_mut(&id)
                {
                    p.connected = true;
                    if let Some(t) = p.timer.take() {
                        self.wheel.cancel(t);
                    }
                    let (b, start) = (p.b, p.start);
                    if let Some(be) = self.backends.get_mut(b) {
                        be.lat.record(now, now - start);
                    }
                    self.lat.record(now, now - start);
                    self.pump(os, id);
                }
                true
            }
            SvcEvent::TcpData { sock, .. } | SvcEvent::TcpWritable { sock } if self.socks.contains_key(&sock) => {
                if let Some(&(id, _)) = self.socks.get(&sock) {
                    self.pump(os, id);
                }
                true
            }
            SvcEvent::TcpEof { sock, .. } if self.socks.contains_key(&sock) => {
                if let Some(&(id, back)) = self.socks.get(&sock) {
                    if let Some(p) = self.pipes.get_mut(&id) {
                        if back {
                            p.b_eof = true;
                        } else {
                            p.c_eof = true;
                        }
                    }
                    self.pump(os, id);
                }
                true
            }
            SvcEvent::TcpClosed { sock, end } if self.socks.contains_key(&sock) => {
                if let Some(&(_, back)) = self.socks.get(&sock) {
                    self.pipe_closed(os, sock, back, end);
                }
                true
            }
            _ => false,
        }
    }

    // ---------------------------------------------------------------- views

    /// what the lab shows
    pub fn view(&self, now: Ns) -> LbView {
        let shares = self.ring.shares();
        LbView {
            mode: self.mode,
            port: self.port,
            algo: self.algo,
            listening: if self.mode == LbMode::Http { self.serve.listening() } else { self.listener.is_some() },
            requests: self.requests,
            responses: self.responses,
            active: self.active(),
            errors: self.errors,
            no_backend: self.no_backend,
            retried: self.retried,
            by_class: self.by_class,
            p50: self.lat.p50(now),
            p95: self.lat.p95(now),
            p99: self.lat.p99(now),
            backends: self
                .backends
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.gone())
                .map(|(i, b)| BackendView {
                    name: b.name.clone(),
                    addr: b.addr,
                    port: b.port,
                    weight: b.weight,
                    admin: b.admin,
                    up: b.up,
                    since: b.since,
                    last_check: b.last_check,
                    checks: b.checks,
                    check_fails: b.check_fails,
                    downs: b.downs,
                    active: b.active,
                    total: b.total,
                    errors: b.errors,
                    by_class: b.by_class,
                    bytes_in: b.bytes_in,
                    bytes_out: b.bytes_out,
                    p50: b.lat.p50(now),
                    p95: b.lat.p95(now),
                    p99: b.lat.p99(now),
                    share_ppm: shares.iter().find(|s| s.0 as usize == i).map(|s| s.1).unwrap_or(0),
                })
                .collect(),
        }
    }

    /// requests (HTTP) or connections (TCP) in flight
    pub fn active(&self) -> u64 {
        if self.mode == LbMode::Http { self.fwd.len() as u64 } else { self.pipes.len() as u64 }
    }
}

/// a backend as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendView {
    pub name: String,
    pub addr: Ipv4,
    pub port: u16,
    pub weight: u32,
    pub admin: Admin,
    pub up: bool,
    /// when its state last changed
    pub since: Ns,
    pub last_check: Option<(Ns, bool)>,
    pub checks: u64,
    pub check_fails: u64,
    pub downs: u32,
    pub active: u32,
    pub total: u64,
    pub errors: u64,
    pub by_class: [u64; 5],
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    /// its part of the hash ring (parts per million)
    pub share_ppm: u32,
}

/// a load balancer as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LbView {
    pub mode: LbMode,
    pub port: u16,
    pub algo: Algo,
    pub listening: bool,
    pub requests: u64,
    pub responses: u64,
    pub active: u64,
    pub errors: u64,
    pub no_backend: u64,
    pub retried: u64,
    pub by_class: [u64; 5],
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    pub backends: Vec<BackendView>,
}

fn us(v: Option<u64>) -> i64 {
    v.map(|x| (x / 1000) as i64).unwrap_or(0)
}

impl Service for Lb {
    fn kind(&self) -> &'static str {
        "lb"
    }
    fn start(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.seen = now;
        self.serve.stop();
        self.agent.reset();
        self.checker.reset();
        self.wheel.clear();
        self.fwd.clear();
        self.by_agent.clear();
        self.hchecks.clear();
        self.tchecks.clear();
        self.pipes.clear();
        self.socks.clear();
        self.listener = None;
        self.agent.max_per_host = self.max_conns as usize;
        self.agent.idle_timeout = 60 * SEC;
        for b in self.backends.iter_mut() {
            b.active = 0;
            b.checking = false;
        }
        match self.mode {
            LbMode::Http => {
                self.serve.start(os, "lb");
            }
            LbMode::Tcp => match os.tcp_listen(self.port, 0, SockOpts::default()) {
                Ok(l) => self.listener = Some(l),
                Err(_) => os.note(Note::PortInUse { port: self.port, svc: String::from("lb") }),
            },
        }
        if self.health.kind != CheckKind::None {
            for i in 0..self.backends.len() {
                if !self.backends[i].gone() {
                    let t = self.wheel.add(now, LTimer::Check(i));
                    self.backends[i].check_t = Some(t);
                }
            }
        }
        self.wheel.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.timers(os);
            return;
        }
        if self.mode == LbMode::Http && self.serve.on(&ev, os) {
            for r in self.serve.take() {
                self.arrive(os, r);
            }
        } else if self.mode == LbMode::Http && self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.answered(os, d);
            }
        } else if self.checker.on(&ev, os) {
            for d in self.checker.take_done() {
                if let Some(i) = self.hchecks.remove(&d.id) {
                    let ok = d.result.as_ref().is_ok_and(|r| (200..400).contains(&r.head.status));
                    self.check_done(os, i, ok);
                }
            }
        } else {
            self.tcp_event(os, &ev);
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.serve.stop();
        self.agent.reset();
        self.checker.reset();
        self.wheel.clear();
        self.fwd.clear();
        self.by_agent.clear();
        self.hchecks.clear();
        self.tchecks.clear();
        self.pipes.clear();
        self.socks.clear();
        self.listener = None;
        for b in self.backends.iter_mut() {
            b.active = 0;
            b.checking = false;
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let v = |x: u64| x as i64;
        let live: Vec<&Backend> = self.backends.iter().filter(|b| !b.gone()).collect();
        let mut m = alloc::vec![
            Metric::new(if self.mode == LbMode::Http { "requests" } else { "connections" }, v(self.requests)),
            Metric::new("responses", v(self.responses)),
            Metric::new("active", v(self.active())),
            Metric::new("errors", v(self.errors)),
            Metric::new("no_backend", v(self.no_backend)),
            Metric::new("retries", v(self.retried)),
            Metric::new("status_2xx", v(self.by_class[1])),
            Metric::new("status_3xx", v(self.by_class[2])),
            Metric::new("status_4xx", v(self.by_class[3])),
            Metric::new("status_5xx", v(self.by_class[4])),
            Metric::new("backends", live.len() as i64),
            Metric::new("backends_up", live.iter().filter(|b| b.up && b.admin == Admin::Active).count() as i64),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
        ];
        for (k, b) in self.backends.iter().take(8).enumerate() {
            m.push(Metric::new(M_REQ[k], v(b.total)));
            m.push(Metric::new(M_ACT[k], b.active as i64));
            m.push(Metric::new(M_UP[k], (b.up && !b.gone()) as i64));
            m.push(Metric::new(M_ERR[k], v(b.errors)));
        }
        m
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LAT)]
    }
    fn status(&self) -> String {
        let live: Vec<&Backend> = self.backends.iter().filter(|b| !b.gone()).collect();
        let up = live.iter().filter(|b| b.up && b.admin == Admin::Active).count();
        let what = if self.mode == LbMode::Http { "Anfragen" } else { "Verbindungen" };
        format!("{} Port {}, {}: {} von {} Backends verfügbar, {} {}, {} Fehler", if self.mode == LbMode::Http { "HTTP" } else { "TCP" }, self.port, self.algo.name(), up, live.len(), self.requests, what, self.errors)
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

// ---------------------------------------------------------------- project definitions

/// a load balancer from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(String::from);
    let mode = match text("mode").as_deref().unwrap_or("http") {
        "http" => LbMode::Http,
        "tcp" => LbMode::Tcp,
        m => return Err(msg!("\u{201e}mode\u{201c} kennt {} nicht (http, tcp).", m)),
    };
    let port = match num("port", 80) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    let mut lb = Lb::new(mode, port);
    if let Some(a) = text("algorithm") {
        lb.algo = Algo::parse(&a).ok_or_else(|| msg!("\u{201e}algorithm\u{201c} kennt {} nicht (round-robin, least-conn, weighted, hash).", a))?;
    }
    if let Some(h) = text("hash") {
        lb.hash_key = match h.as_str() {
            "path" => HashKey::Path,
            "client" => HashKey::Client,
            x if x.starts_with("header:") => HashKey::Header(String::from(x[7..].trim())),
            x => return Err(msg!("\u{201e}hash\u{201c} kennt {} nicht (path, client, header:NAME).", x)),
        };
    }
    lb.vnodes = num("vnodes", 64).clamp(1, 1000) as u32;
    lb.retries = num("retries", 1).min(16) as u32;
    lb.timeout = ms("timeout_ms", 10_000);
    lb.connect_timeout = ms("connect_timeout_ms", 1000).max(MS);
    lb.drain = ms("drain_ms", 30_000);
    lb.max_conns = num("max_conns", 0).min(1_000_000) as u32;
    lb.keepalive = c.get("keepalive").and_then(|v| v.as_bool()).unwrap_or(true);
    lb.tag = c.get("tag").and_then(|v| v.as_bool()).unwrap_or(true);
    if let Some(h) = c.get("health") {
        let hs = |k: &str| h.get(k).and_then(|v| v.as_str()).map(String::from);
        let hm = |k: &str, d: Ns| h.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d);
        if let Some(k) = hs("type") {
            lb.health.kind = match k.as_str() {
                "none" | "off" => CheckKind::None,
                "tcp" => CheckKind::Tcp,
                "http" => CheckKind::Http,
                x => return Err(msg!("\u{201e}health.type\u{201c} kennt {} nicht (tcp, http, none).", x)),
            };
        }
        if let Some(p) = hs("path") {
            lb.health.path = p;
        }
        lb.health.interval = hm("interval_ms", lb.health.interval).max(10 * MS);
        lb.health.timeout = hm("timeout_ms", lb.health.timeout).clamp(MS, lb.health.interval);
        lb.health.rise = h.get("rise").and_then(|v| v.as_u64()).unwrap_or(2).clamp(1, 100) as u32;
        lb.health.fall = h.get("fall").and_then(|v| v.as_u64()).unwrap_or(3).clamp(1, 100) as u32;
    }
    for (k, b) in c.get("backends").and_then(|b| b.as_array()).unwrap_or(&[]).iter().enumerate() {
        let addr = b.get("addr").and_then(|v| v.as_str()).and_then(|s| Ipv4::parse(s.trim())).ok_or_else(|| msg!("Backend {} braucht \u{201e}addr\u{201c} mit einer IPv4-Adresse.", k + 1))?;
        let bport = b.get("port").and_then(|v| v.as_u64()).unwrap_or(if mode == LbMode::Http { 80 } else { port as u64 });
        if !(1..=65535).contains(&bport) {
            return Err(msg!("Port {} gibt es nicht.", bport));
        }
        let name = b.get("name").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| format!("{}:{}", addr, bport));
        if lb.backends.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
            return Err(msg!("Das Backend {} steht zweimal da.", name));
        }
        let weight = b.get("weight").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 1000) as u32;
        lb.add_backend(&name, addr, bport as u16, weight);
    }
    if lb.backends.is_empty() {
        return Err(msg!("\u{201e}backends\u{201c} fehlt."));
    }
    Ok(Box::new(lb))
}

impl Engine {
    /// a load balancer's view for the lab
    pub fn lb_view(&self, dev: u32, svc: &str) -> Option<LbView> {
        Some(self.service::<Lb>(dev, svc)?.view(self.now))
    }
}
