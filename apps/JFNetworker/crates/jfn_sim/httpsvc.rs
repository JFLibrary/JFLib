// httpsvc -- the HTTP services: a web server with routes, a client that sends requests, and a
// load generator with open and closed loops whose latencies and errors go into windows of
// base::metrics (p50 / p95 / p99 per second and over the last five minutes)
//
//   kind          config (JSON; every member may be left out)
//   http-server   {"port": 80, "root": "/var/www", "workers": 0, "queue": 0, "time_ms": 0,
//                  "jitter_ms": 0, "error_rate": 0, "error_status": 500, "keepalive_ms": 75000,
//                  "max_requests": 1000, "header_timeout_ms": 60000, "max_body": 1048576,
//                  "backlog": 0, "server": "jfn-httpd/1.0", "routes": [{"path": "/",
//                  "method": "GET", "status": 200, "body": "Hallo von {host}\n",
//                  "type": "text/plain", "size": 0, "chunk": 0, "time_ms": 0, "jitter_ms": 0,
//                  "error_rate": 0, "error_status": 500, "headers": {"Cache-Control": "no-store"},
//                  "backend": "kv", "cpu_ms": 0, "serial_us": 0}], "kv": {"nodes": ["10.0.2.21"], "port": 7000,
//                  "rf": 2, ...}, "db": {"addr": "10.0.2.30", "port": 7100, "timeout_ms": 5000, "conns": 32}, "cpu_ms": 0.02, "serial_us": 0, "usl": {"sigma": 0, "kappa": 0}, "mem_mb": 32,
//                  "conn_kb": 64, "data_mb": 0}
//                  With "tls" (tlssim.rs: {"names": [...], "issuer": "lab" | "self", "org": "..", "days":
//                  365, "age_days": 0, "starts_in_days": 0}) it speaks TLS; its port is then 443 unless
//                  "port" says otherwise, and metrics add tls_handshakes and tls_failed
//   http-client   {"url": "http://10.0.2.10/", "method": "GET", "count": 1, "interval_ms": 0,
//                  "delay_ms": 0, "body": "", "headers": {}, "timeout_ms": 0,
//                  "keepalive": true}
//   http-load     {"targets": ["http://10.0.2.10/"], "mode": "closed", "users": 10,
//                  "think_ms": 0, "think": "fixed", "rps": 100, "start_rps": 0,
//                  "arrival": "uniform", "ramp_ms": 0, "delay_ms": 0, "duration_ms": 0,
//                  "requests": 0, "timeout_ms": 5000, "connect_timeout_ms": 0,
//                  "keepalive": true, "max_conns": 0, "balance": "round-robin",
//                  "mix": [{"method": "GET", "path": "/", "weight": 1, "body": "",
//                  "headers": {}}]}
//
// Server: a route matches its path exactly, "/api/*" by prefix, "*" every path; its method or
// any ("" or "*"; GET routes answer HEAD too).  A path without a route of the method gets 405
// with Allow, none at all 404.  Bodies may name {host}, {method}, {path}, {client} and
// {addr}; "size" makes a body of that many bytes instead; "chunk" sends it chunked.  A route
// with "backend" asks before it answers, while its worker waits: "kv" uses the key-value
// store of "kv" (kv.rs: the key is the part of the path after the route's prefix, GET reads,
// PUT and POST write, DELETE deletes; status, value and X-Version come back, with X-Kv-Node),
// a URL forwards the request to another server (the rest of the path appended).  "db" asks the
// database of "db" (db.rs): the route's "sql" is one statement or a list of them, each a round trip with
// its worker waiting, a list inside the transaction BEGIN gave; "param" says what a ? is: "rest" (the
// rest of the path, the default), "query:NAME" (a field of the URL), "draw" (a key by the distribution of
// the statement's table, drawn by the database once per request, so all statements see the same) or
// "none"; "retries": how often it starts again after an answer that says retryable (a deadlock).  Every
// request needs a worker for its time (time_ms plus up to jitter_ms, the route's or the
// server's); with "workers" > 0 at most that many work at once, the others wait in order,
// and beyond "queue" waiting ones (> 0) a request gets 503 at once.  The error rate (the
// route's, else the server's) answers a request with error_status after its time.  Without
// routes and without "root" the server answers every path with "Hallo von {host}".
//
// On a device with hardware (res.rs) a request's "cpu_ms" (the route's, else the server's: 20 µs at
// a reference core) comes first and waits for a core, in pieces of res::QUANTUM when it is long;
// time_ms stays waiting that takes no CPU, so its number keeps its meaning.  "serial_us" is a
// critical section after the CPU: one request at a time holds the server's lock, at the speed of
// the device's cores, with or without hardware.  "usl" is the factor of the Universal Scalability
// Law on its cost, "mem_mb", "conn_kb", "data_mb" what it holds in memory.  Without hardware
// cpu_ms costs nothing.
//
// Static files: "root" names a folder of the device.  A GET or HEAD that no route of its method
// takes is answered from the files below it: the path percent-decoded and without dot segments
// (it cannot climb out of the folder), a folder's index.html, a folder without its "/" redirected
// (301) to the path with it, the type from the file's extension (netpkt::http::mime_of); the
// read takes the disk's time, on top of the server's time.  Other methods on a file get 405 with
// Allow, everything else what the routes say (404).  Files are read at each request, so a saved
// page shows at once.
//
// Client: `count` requests (0: until stopped) one after the other, each `interval_ms` after
// the previous answer.
//
// Load generator: "closed": `users` virtual users, each sends a request, waits for the answer
// and then thinks (fixed or exponential with the mean think_ms); "ramp_ms" starts them one
// after the other.  "open": requests arrive at `rps` per second (decimals allowed) whatever
// the answers do, evenly or as a Poisson process ("arrival"), "ramp_ms" raises the rate from
// start_rps; the latency counts from the moment a request was due (no coordinated omission),
// "max_conns" limits the connections per target.  Targets take turns (or "balance": "random");
// the mix picks method and path by weight; paths and bodies may name {seq} (the number of
// the request, from 0), {rand:N} (0 to N-1) and {hot:N} (four of five requests among the
// first fifth of 0 to N-1: a few keys get most of the load).  It stops after duration_ms or `requests`.
// A request that cannot even start (no route, ARP gave up) fails at once; what follows comes 100 ms
// later, so that a target that reboots gets its load back.
//
// Metrics (Service::metrics_at; latencies in µs over the five-minute window):
//   http-server  requests, responses, status_2xx..5xx, errors (injected), rejected (503 when
//                full), bad, connections, active, inflight, queued, max_queued, bytes_in,
//                bytes_out, idle_closed, p50_us, p95_us, p99_us (arrival to answer), busy_ms;
//                with backends also backend_calls, backend_errors, failovers, backend_p99_us
//   http-client  sent, responses, ok, errors, last_status, last_us, p50_us, p95_us, p99_us
//   http-load    sent, responses, ok, http_errors, failed, timeouts, errors, inflight,
//                connections, status_2xx..5xx, users / rate_mrps, rps (answers per second
//                over the last 10 s), err_pm (errors per mille), p50_us, p95_us, p99_us,
//                mean_us, max_us
// Time series (series.rs): busy_ms counts up (its rate is the workers' load in ms per
// second); the latency windows give p50_us, p95_us, p99_us of each interval, a web server's
// backend also backend_p50_us .. backend_p99_us, the load generator also mean_us and max_us.
// Views for the lab: HttpServer::view, HttpLoad::view (per-second slots with p50/p95/p99);
// Engine::http_server_view / http_load_view.  HttpLoad::lat_between: the latencies of a span as one
// histogram (a sweep).  Actions HttpLoadSet and HttpServerSet change a
// running service without losing its numbers.  Service::reconfigure takes a whole new config: a web
// server all but its port, certificate, listen backlog and backends; a load generator all but its
// mode and duration.  from_def (svcact.rs) refuses a definition with
// German text, from_def_msg with a Msg for the app's catalogue; status_msg() words the status
// lines so too.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::metrics::{Hist, Window};
use base::rng::Pcg32;
use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use netpkt::http::{self, Headers, Version};
use netpkt::uri;

use crate::http::{Agent, Done, Fail, Reply, ReqIn, Req, Serve, ServeCfg, Url, Wheel};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{metric_kind_of, LatStat, LatWindow, MetricKind, LAT};
use crate::trace::Note;

/// timer keys: the service's own deadlines, the HTTP part (Serve or Agent)
const K_OWN: u32 = 0;
const K_HTTP: u32 = 1;
/// the backends of a web server's routes (kv.rs KvClient)
const K_UP: u32 = 2;
/// seconds the per-second counters keep (15 minutes: load averages)
const KEEP_SECS: usize = 900;

// ---------------------------------------------------------------- helpers

/// a decimal number of JSON (or a string with one) scaled by `scale`: ("0.5", 1000) -> 500
pub fn decimal(v: &Value, scale: u64) -> Option<u64> {
    let text = match v {
        Value::Number(n) => String::from(n.as_str()),
        Value::String(s) => String::from(s.trim()),
        _ => return None,
    };
    let (mant, exp) = match text.find(['e', 'E']) {
        Some(i) => (&text[..i], text[i + 1..].parse::<i32>().ok()?),
        None => (text.as_str(), 0),
    };
    if mant.starts_with('-') {
        return None;
    }
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) || (int.is_empty() && frac.is_empty()) {
        return None;
    }
    // digits * 10^(exp - frac.len()) * scale
    let digits = format!("{}{}", int, frac);
    let mut shift = exp - frac.len() as i32;
    let mut n: u128 = 0;
    for b in digits.bytes() {
        n = n.checked_mul(10)?.checked_add((b - b'0') as u128)?;
        if n > u64::MAX as u128 * 1000 {
            return None;
        }
    }
    n = n.checked_mul(scale as u128)?;
    while shift > 0 {
        n = n.checked_mul(10)?;
        shift -= 1;
    }
    while shift < 0 {
        n /= 10;
        shift += 1;
    }
    u64::try_from(n).ok()
}

/// a rate as parts per million: 0.01, "1%", "5000ppm"
pub fn rate_ppm(v: &Value) -> Option<u32> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            return decimal(&Value::string(p), 10_000).map(|x| x.min(1_000_000) as u32);
        }
        if let Some(p) = s.strip_suffix("ppm") {
            return decimal(&Value::string(p), 1).map(|x| x.min(1_000_000) as u32);
        }
    }
    decimal(v, 1_000_000).map(|x| x.min(1_000_000) as u32)
}

fn headers_of(v: Option<&Value>) -> Headers {
    let mut h = Headers::new();
    if let Some(o) = v.and_then(|x| x.as_object()) {
        for (k, v) in o {
            if let Some(s) = v.as_str() {
                h.add(k, s);
            } else if let Some(n) = v.as_u64() {
                h.add(k, &format!("{}", n));
            } else if let Some(l) = v.as_array() {
                // several headers of a name (Set-Cookie)
                for s in l.iter().filter_map(|x| x.as_str()) {
                    h.add(k, s);
                }
            }
        }
    }
    h
}

/// log2(y) for y >= 1 in 32.32 fixed point (bit by bit, integers only)
fn log2_fx(y: u64) -> u64 {
    let ip = 63 - y.leading_zeros() as u64;
    let mut x: u128 = ((y as u128) << 32) >> ip;
    let mut frac: u64 = 0;
    for i in (0..32).rev() {
        x = (x * x) >> 32;
        if x >= (2u128 << 32) {
            x >>= 1;
            frac |= 1 << i;
        }
    }
    (ip << 32) | frac
}

/// an exponentially distributed time with mean `mean` from 32 random bits: -ln(U) * mean,
/// U = (u + 1) / 2^32, in integer arithmetic (the same on every target)
pub fn exp_sample(mean: Ns, u: u32) -> Ns {
    let y = u as u64 + 1;
    let nlog2 = (32u64 << 32) - log2_fx(y);
    // ln 2 in 0.32 fixed point
    let nln = (nlog2 as u128 * 2_977_044_472u128) >> 32;
    ((mean as u128 * nln) >> 32) as Ns
}

/// when the expected number of arrivals reaches `pos` (millionths of a request) for a rate
/// that goes from r0 to r1 (thousandths of a request per second) over `ramp` and then stays:
/// the time since the start, None when the rate stays 0.  Integers only: within the ramp the
/// quadratic is solved in microseconds with an integer square root.
pub fn arrival_offset(r0: u64, r1: u64, ramp: Ns, pos: u128) -> Option<Ns> {
    // t_ns = pos * 1e6 / r for a constant rate
    let flat = |r: u64, p: u128| -> Option<Ns> { if r == 0 { None } else { u64::try_from(p * 1_000_000 / r as u128).ok() } };
    let rr = ramp / 1000;
    if rr == 0 || r0 == r1 {
        return flat(if rr == 0 { r1 } else { r0 }, pos);
    }
    // arrivals over the whole ramp: (r0 + r1) * R / 2000 millionths (R in microseconds)
    let n_ramp = (r0 as u128 + r1 as u128) * rr as u128 / 2000;
    if pos > n_ramp {
        return flat(r1, pos - n_ramp).map(|x| rr * 1000 + x);
    }
    // (r1 - r0) t^2 + 2 R r0 t - 2000 R pos = 0, t = 2c / (b + sqrt(b^2 + 4ac))
    let a = r1 as i128 - r0 as i128;
    let b = 2 * rr as i128 * r0 as i128;
    let c = 2000 * rr as i128 * pos as i128;
    let d = b * b + 4 * a * c;
    if d < 0 {
        return None;
    }
    let den = b + crate::cli::fmt::isqrt(d as u128) as i128;
    let t_us = if den <= 0 { 0 } else { (2 * c + den - 1) / den };
    Some((t_us as u64).saturating_mul(1000))
}

/// a uniform extra time up to `jitter` (inclusive), as the cables draw theirs
fn jitter_of(rng: &mut Pcg32, jitter: Ns) -> Ns {
    if jitter == 0 { 0 } else { rng.range(0, jitter as i64) as Ns }
}

fn seeded(os: &mut OsApi) -> Pcg32 {
    let a = os.rand() as u64;
    let b = os.rand() as u64;
    Pcg32::new((a << 32) | b, os.svc_id() as u64)
}

/// counters per second of virtual time, the last KEEP_SECS seconds
#[derive(Clone, Debug)]
struct PerSec<const K: usize> {
    /// the second of the newest entry
    head: u64,
    vals: VecDeque<[u64; K]>,
}

impl<const K: usize> PerSec<K> {
    fn new() -> Self {
        PerSec { head: 0, vals: VecDeque::new() }
    }
    fn roll(&mut self, sec: u64) {
        if self.vals.is_empty() {
            self.head = sec;
            self.vals.push_back([0; K]);
            return;
        }
        while self.head < sec {
            self.head += 1;
            self.vals.push_back([0; K]);
            if self.vals.len() > KEEP_SECS {
                self.vals.pop_front();
            }
        }
    }
    fn add(&mut self, t: Ns, k: usize, v: u64) {
        let sec = t / SEC;
        self.roll(sec);
        if sec + self.vals.len() as u64 <= self.head {
            return;
        }
        let i = self.vals.len() - 1 - (self.head - sec) as usize;
        self.vals[i][k] += v;
    }
    /// field k summed over the `secs` seconds that end with the second of t
    fn sum(&self, t: Ns, k: usize, secs: u64) -> u64 {
        let now = t / SEC;
        let first = (now + 1).saturating_sub(secs);
        let base = self.head + 1 - self.vals.len() as u64;
        (first..=now).filter(|&s| s >= base && s <= self.head).map(|s| self.vals[(s - base) as usize][k]).sum()
    }
    /// the seconds before t (oldest first): (start, values)
    fn slots(&self, t: Ns, secs: u64) -> Vec<(Ns, [u64; K])> {
        let now = t / SEC;
        let first = (now + 1).saturating_sub(secs);
        let base = (self.head + 1).saturating_sub(self.vals.len() as u64);
        (first..=now).map(|s| (s * SEC, if !self.vals.is_empty() && s >= base && s <= self.head { self.vals[(s - base) as usize] } else { [0; K] })).collect()
    }
}

fn us(v: Option<u64>) -> i64 {
    v.map(|x| (x / 1000) as i64).unwrap_or(0)
}

/// a time window for latencies: 300 slots of one second
fn lat_window() -> Window {
    Window::new(300, SEC)
}

/// the time series of a web server's backend latency (series.rs)
const BACKEND_LAT: &[(&str, LatStat)] = &[("backend_p50_us", LatStat::Quantile(500)), ("backend_p95_us", LatStat::Quantile(950)), ("backend_p99_us", LatStat::Quantile(990))];

/// the load generator's latency series: the quantiles, the mean and the maximum of each
/// interval
const LOAD_LAT: &[(&str, LatStat)] = &[("p50_us", LatStat::Quantile(500)), ("p95_us", LatStat::Quantile(950)), ("p99_us", LatStat::Quantile(990)), ("mean_us", LatStat::Mean), ("max_us", LatStat::Max)];

/// one second of a latency window and its counters, for charts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub t: Ns,
    /// requests sent (load generator) or come (server)
    pub sent: u64,
    /// answers
    pub done: u64,
    pub errors: u64,
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
}

/// the slots of a window and counters, oldest first
fn slots_of(lat: &Window, per: &PerSec<4>, t: Ns, n: usize) -> Vec<Slot> {
    let q50 = lat.per_slot(t, 500);
    let q95 = lat.per_slot(t, 950);
    let q99 = lat.per_slot(t, 990);
    let counts = per.slots(t, n as u64);
    let skip = q50.len().saturating_sub(n);
    counts
        .iter()
        .enumerate()
        .map(|(i, (start, v))| {
            let j = skip + i;
            Slot { t: *start, sent: v[0], done: v[1], errors: v[2], p50: q50.get(j).and_then(|x| x.2), p95: q95.get(j).and_then(|x| x.2), p99: q99.get(j).and_then(|x| x.2) }
        })
        .collect()
}

// ---------------------------------------------------------------- the web server

/// one route of a web server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRoute {
    /// "/" exactly, "/api/*" by prefix, "*" every path
    pub path: String,
    /// "" or "*": every method; "GET" answers HEAD too
    pub method: String,
    pub status: u16,
    /// may name {host}, {method}, {path}, {client}, {addr}
    pub body: String,
    pub ctype: String,
    /// > 0: a body of this many bytes instead
    pub size: usize,
    /// > 0: chunked in pieces of this size
    pub chunk: usize,
    /// its time and jitter (None: the server's)
    pub time: Option<Ns>,
    pub jitter: Option<Ns>,
    /// its error rate (None: the server's) and the status of an error
    pub error_ppm: Option<u32>,
    pub error_status: Option<u16>,
    pub headers: Headers,
    /// what it asks before it answers
    pub backend: RouteBackend,
    /// its CPU work at a reference core and its critical section (None: the server's; res.rs)
    pub cpu: Option<Ns>,
    pub serial: Option<Ns>,
}

/// what a route asks before it answers
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteBackend {
    None,
    /// the key-value store of the server ("kv"): GET reads, PUT and POST write, DELETE
    /// deletes the key after the route's prefix
    Kv,
    /// another web server: the request goes on to this URL, the rest of the path appended
    Url(Url),
    /// the database of the server ("db"): these statements
    Db(alloc::sync::Arc<DbRoute>),
}

/// a route's statements (db.rs)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbRoute {
    pub sql: Vec<String>,
    /// how many ? each one has
    pub params: Vec<u16>,
    pub param: DbParam,
    pub retries: u32,
}

/// what a ? of a route's statement is
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbParam {
    None,
    Rest,
    Query(String),
    Draw,
}

/// the database of a web server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbBackend {
    pub host: String,
    pub port: u16,
}

/// how far a request is through its route's statements
#[derive(Clone, Debug)]
struct DbRun {
    route: alloc::sync::Arc<DbRoute>,
    step: usize,
    txn: Option<u64>,
    /// the value of its ? as JSON (a drawn key after the first answer)
    param: Option<String>,
    tries: u32,
}

impl HttpRoute {
    pub fn new(path: &str, body: &str) -> HttpRoute {
        HttpRoute { path: String::from(path), method: String::from("GET"), status: 200, body: String::from(body), ctype: String::from("text/plain"), size: 0, chunk: 0, time: None, jitter: None, error_ppm: None, error_status: None, headers: Headers::new(), backend: RouteBackend::None, cpu: None, serial: None }
    }
    /// the part of a path after the route's prefix ("/item/*" and "/item/42": "42"); an exact
    /// route gives the last segment
    pub fn rest<'a>(&self, path: &'a str) -> &'a str {
        match self.path.strip_suffix('*') {
            Some(pre) => path.strip_prefix(pre).unwrap_or(""),
            None => path.rsplit('/').next().unwrap_or(""),
        }
    }
    fn path_matches(&self, p: &str) -> bool {
        if self.path == "*" {
            return true;
        }
        match self.path.strip_suffix('*') {
            Some(pre) => p.starts_with(pre),
            None => self.path == p,
        }
    }
    fn method_matches(&self, m: &str) -> bool {
        self.method.is_empty() || self.method == "*" || self.method.eq_ignore_ascii_case(m) || (m == "HEAD" && self.method.eq_ignore_ascii_case("GET"))
    }
}

/// a request in work or waiting for a worker
#[derive(Clone, Debug)]
struct Work {
    req: ReqIn,
    reply: Reply,
    time: Ns,
    /// when it got its worker
    started: Ns,
    /// the backend it asks after its time, and the rest of its path
    call: Option<(RouteBackend, String)>,
    /// the CPU work it has left, its critical section still to come (res.rs)
    cpu: Ns,
    serial: Ns,
    /// its route's statements (db.rs)
    db: Option<DbRun>,
}

/// what a request asks of the CPU and the server's lock (res.rs)
#[derive(Clone, Copy, Debug, Default)]
struct Costs {
    cpu: Ns,
    serial: Ns,
}

/// the web server
#[derive(Clone, Debug)]
pub struct HttpServer {
    pub routes: Vec<HttpRoute>,
    /// requests in work at once (0: no limit)
    pub workers: u32,
    /// requests waiting for a worker at most (0: no limit); more get 503
    pub queue_max: u32,
    pub time: Ns,
    pub jitter: Ns,
    pub error_ppm: u32,
    pub error_status: u16,
    /// the folder of the device whose files it serves ("root"; None: none)
    pub root: Option<String>,
    serve: Serve,
    wheel: Wheel<u64>,
    work: BTreeMap<u64, Work>,
    queue: VecDeque<Work>,
    rng: Pcg32,
    /// arrival to answer
    lat: Window,
    /// per second: requests, answers, errors (5xx), busy ns
    per: PerSec<4>,
    /// per second: ns of waiting for a worker
    waits: PerSec<1>,
    pub injected: u64,
    pub rejected: u64,
    pub max_queued: u32,
    pub busy_ns: u64,
    /// the virtual time of its last event (metrics() without a time)
    seen: Ns,
    /// the backends of its routes: the key-value store and other web servers
    pub upstream: Option<crate::kv::KvClient>,
    /// requests whose worker waits for a backend, by the backend request
    calls: BTreeMap<u64, Work>,
    pub backend_calls: u64,
    pub backend_errors: u64,
    /// its certificate: it speaks TLS (tlssim.rs)
    pub tls: Option<crate::tlssim::TlsSpec>,
    /// the CPU work of a request and its critical section unless its route says otherwise, the
    /// factor on its cost, what it holds (res.rs)
    pub cpu: Ns,
    pub serial: Ns,
    pub usl: crate::res::Usl,
    pub mem: crate::res::SvcMem,
    /// its lock is free then
    lock_free: Ns,
    /// the time its lock was held so far (a sweep reads it, experiment.rs)
    pub lock_busy: Ns,
    /// "kv" of its config, as Service::reconfigure compares it
    kv_cfg: Option<Value>,
    /// the database of its routes, and "db" of its config
    pub db: Option<DbBackend>,
    db_cfg: Option<Value>,
}

impl HttpServer {
    pub fn new(port: u16) -> HttpServer {
        let cfg = ServeCfg { port, ..ServeCfg::default() };
        HttpServer {
            routes: Vec::new(),
            workers: 0,
            queue_max: 0,
            time: 0,
            jitter: 0,
            error_ppm: 0,
            error_status: 500,
            root: None,
            serve: Serve::new(cfg, K_HTTP),
            wheel: Wheel::new(K_OWN),
            work: BTreeMap::new(),
            queue: VecDeque::new(),
            rng: Pcg32::new(0, 0),
            lat: lat_window(),
            per: PerSec::new(),
            waits: PerSec::new(),
            injected: 0,
            rejected: 0,
            max_queued: 0,
            busy_ns: 0,
            seen: 0,
            upstream: None,
            calls: BTreeMap::new(),
            backend_calls: 0,
            backend_errors: 0,
            tls: None,
            cpu: 20 * crate::kernel::US,
            serial: 0,
            usl: crate::res::Usl::default(),
            mem: crate::res::SvcMem::default(),
            lock_free: 0,
            lock_busy: 0,
            kv_cfg: None,
            db: None,
            db_cfg: None,
        }
    }
    /// the settings of its connections (port, keep-alive, limits, Server header)
    pub fn cfg(&self) -> &ServeCfg {
        &self.serve.cfg
    }
    pub fn cfg_mut(&mut self) -> &mut ServeCfg {
        &mut self.serve.cfg
    }
    /// the counters of its connections
    pub fn stats(&self) -> crate::http::ServeStats {
        self.serve.stats
    }
    /// requests in work now
    pub fn busy(&self) -> u32 {
        (self.work.len() + self.calls.len()) as u32
    }
    /// requests waiting for a worker now
    pub fn queued(&self) -> u32 {
        self.queue.len() as u32
    }
    /// the busy time of its workers in the `secs` seconds up to `now`, and the time requests
    /// waited for one (top: %CPU, load average)
    pub fn load(&self, now: Ns, secs: u64) -> (u64, u64) {
        (self.per.sum(now, 3, secs), self.waits.sum(now, 0, secs))
    }

    /// changes the server while it runs (Action::HttpServerSet): an error rate or a time for
    /// every route from now on, more or fewer workers (more take waiting requests at once)
    pub fn set(&mut self, os: &mut OsApi, error_ppm: Option<u32>, time: Option<Ns>, workers: Option<u32>) {
        if let Some(p) = error_ppm {
            self.error_ppm = p;
            for r in self.routes.iter_mut() {
                r.error_ppm = None;
            }
        }
        if let Some(t) = time {
            self.time = t;
            for r in self.routes.iter_mut() {
                r.time = None;
            }
        }
        if let Some(w) = workers {
            self.workers = w;
            while (self.workers == 0 || self.busy() < self.workers)
                && let Some(next) = self.queue.pop_front()
            {
                self.begin(os, next);
            }
        }
        self.wheel.arm(os);
    }

    /// the answer of the routes, the time it takes, the backend it asks, what it costs
    fn route(&mut self, os: &mut OsApi, r: &ReqIn) -> (Reply, Ns, Option<(RouteBackend, String)>, Costs) {
        let costs = Costs { cpu: self.cpu, serial: self.serial };
        let path = r.head.path();
        let m = r.head.method.as_str();
        let hit = self.routes.iter().position(|x| x.path_matches(path) && x.method_matches(m));
        let Some(i) = hit else {
            if let Some((rep, disk)) = self.file_answer(os, r) {
                return (rep, self.time + jitter_of(&mut self.rng, self.jitter) + disk, None, costs);
            }
            let allowed: Vec<String> = self.routes.iter().filter(|x| x.path_matches(path)).map(|x| if x.method.is_empty() || x.method == "*" { String::from("*") } else { x.method.to_ascii_uppercase() }).collect();
            if allowed.is_empty() {
                return (Reply::status_page(404), self.time + jitter_of(&mut self.rng, self.jitter), None, costs);
            }
            let mut list: Vec<String> = Vec::new();
            for a in allowed {
                if !list.contains(&a) {
                    list.push(a.clone());
                }
                if a == "GET" && !list.iter().any(|x| x == "HEAD") {
                    list.push(String::from("HEAD"));
                }
            }
            let mut rep = Reply::status_page(405);
            rep.headers.add("Allow", &list.join(", "));
            return (rep, self.time + jitter_of(&mut self.rng, self.jitter), None, costs);
        };
        let rt = self.routes[i].clone();
        let costs = Costs { cpu: rt.cpu.unwrap_or(costs.cpu), serial: rt.serial.unwrap_or(costs.serial) };
        let time = rt.time.unwrap_or(self.time) + jitter_of(&mut self.rng, rt.jitter.unwrap_or(self.jitter));
        let ppm = rt.error_ppm.unwrap_or(self.error_ppm);
        if ppm > 0 && self.rng.chance_ppm(ppm) {
            let st = rt.error_status.unwrap_or(self.error_status);
            self.injected += 1;
            os.note(Note::HttpInjected { status: st });
            return (Reply::status_page(st), time, None, costs);
        }
        let call = match &rt.backend {
            RouteBackend::None => None,
            b => Some((b.clone(), String::from(rt.rest(path)))),
        };
        let body: Vec<u8> = if rt.size > 0 {
            (0..rt.size).map(|k| b"0123456789abcdefghijklmnopqrstuvwxyz\n"[k % 37]).collect()
        } else {
            let host = os.hostname();
            rt.body.replace("{host}", &host).replace("{method}", m).replace("{path}", path).replace("{client}", &format!("{}", r.peer.0)).replace("{addr}", &format!("{}", r.local.0)).into_bytes()
        };
        let mut h = Headers::new();
        if !rt.ctype.is_empty() {
            h.add("Content-Type", &rt.ctype);
        }
        for (k, v) in &rt.headers.0 {
            h.add(k, v);
        }
        (Reply { status: rt.status, headers: h, body, chunk: rt.chunk, close: false }, time, call, costs)
    }

    /// the answer of the static files ("root") and how long the disk takes for it; None when
    /// there is no such file
    fn file_answer(&self, os: &mut OsApi, r: &ReqIn) -> Option<(Reply, Ns)> {
        let root = self.root.as_deref()?;
        let wire = r.head.path();
        let path = String::from_utf8(uri::percent_decode(wire)).ok()?;
        if !path.starts_with('/') || path.contains('\0') {
            return None;
        }
        let clean = uri::remove_dot_segments(&path);
        let mut file = format!("{}{}", root.trim_end_matches('/'), clean);
        if os.file_stat(&file)?.dir {
            if !clean.ends_with('/') {
                let query = r.head.target.find('?').map_or("", |i| &r.head.target[i..]);
                let mut rep = Reply::status_page(301);
                rep.headers.add("Location", &format!("{}/{}", wire, query));
                return Some((rep, 0));
            }
            file.push_str("index.html");
            if os.file_stat(&file)?.dir {
                return None;
            }
        }
        let m = r.head.method.as_str();
        if m != "GET" && m != "HEAD" {
            let mut rep = Reply::status_page(405);
            rep.headers.add("Allow", "GET, HEAD");
            return Some((rep, 0));
        }
        let body = os.file_read(&file).ok()?;
        let now = os.now();
        let disk = os.disk_read(body.len() as u64).saturating_sub(now);
        let mut h = Headers::new();
        h.add("Content-Type", http::mime_of(&file));
        Some((Reply { status: 200, headers: h, body, chunk: 0, close: false }, disk))
    }

    fn arrive(&mut self, os: &mut OsApi, r: ReqIn) {
        let now = os.now();
        if os.recording() {
            os.note(Note::HttpRequest { method: r.head.method.clone(), path: String::from(r.head.path()) });
        }
        self.per.add(now, 0, 1);
        let (reply, time, call, costs) = self.route(os, &r);
        let cpu = if os.metered() { costs.cpu } else { 0 };
        let w = Work { req: r, reply, time, started: now, call, cpu, serial: costs.serial, db: None };
        if self.workers > 0 && self.busy() >= self.workers {
            if self.queue_max > 0 && self.queue.len() as u32 >= self.queue_max {
                self.rejected += 1;
                os.note(Note::HttpOverload);
                let mut rep = Reply::status_page(503);
                rep.headers.add("Retry-After", "1");
                self.answer(os, w.req, rep);
                return;
            }
            os.note(Note::HttpQueued { waiting: self.queue.len() as u32 });
            self.queue.push_back(w);
            self.max_queued = self.max_queued.max(self.queue.len() as u32);
            return;
        }
        self.begin(os, w);
    }

    fn begin(&mut self, os: &mut OsApi, mut w: Work) {
        let now = os.now();
        let waited = now - w.req.at;
        if waited > 0 {
            self.waits.add(now, 0, waited);
        }
        w.started = now;
        let id = w.req.id;
        let at = if w.cpu > 0 || w.serial > 0 { self.step(os, &mut w) } else { now + w.time };
        self.wheel.add(at, id);
        self.work.insert(id, w);
    }

    /// the next part of a request's work (res.rs), when it is done: a piece of its CPU work, then
    /// its critical section; the last part ends with the request's own time on top
    fn step(&mut self, os: &mut OsApi, w: &mut Work) -> Ns {
        let now = os.now();
        if w.cpu > 0 {
            let at = os.cpu_slice_usl(&mut w.cpu, self.usl);
            return if w.cpu > 0 || w.serial > 0 { at } else { at + w.time };
        }
        let hold = os.cpu_ps(core::mem::take(&mut w.serial));
        self.lock_free = self.lock_free.max(now) + hold;
        self.lock_busy += hold;
        self.lock_free + w.time
    }

    fn answer(&mut self, os: &mut OsApi, r: ReqIn, rep: Reply) {
        let now = os.now();
        let status = rep.status;
        // an answer whose connection is gone counts nowhere (Serve counts it as lost)
        if !self.serve.respond(os, r.id, rep) {
            return;
        }
        self.lat.record(now, now - r.at);
        self.per.add(now, 1, 1);
        if status >= 500 {
            self.per.add(now, 2, 1);
        }
    }

    fn done(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, id) in self.wheel.due(now) {
            let Some(mut w) = self.work.remove(&id) else {
                continue;
            };
            // more of its work to do on the CPU or under the lock
            if w.cpu > 0 || w.serial > 0 {
                let at = self.step(os, &mut w);
                self.wheel.add(at, id);
                self.work.insert(id, w);
                continue;
            }
            let busy = now - w.started;
            self.busy_ns += busy;
            self.per.add(now, 3, busy);
            if w.call.is_some() && self.upstream.is_some() {
                // the worker waits for the backend
                self.call(os, w);
                continue;
            }
            self.answer(os, w.req, w.reply);
            while (self.workers == 0 || self.busy() < self.workers)
                && let Some(next) = self.queue.pop_front()
            {
                self.begin(os, next);
            }
        }
        self.wheel.arm(os);
    }

    /// a request asks its backend
    fn call(&mut self, os: &mut OsApi, w: Work) {
        let Some((b, rest)) = w.call.clone() else {
            return;
        };
        if let RouteBackend::Db(route) = &b {
            self.db_start(os, w, route.clone(), &rest);
            return;
        }
        let Some(up) = self.upstream.as_mut() else {
            return;
        };
        let m = w.req.head.method.as_str();
        let id = match b {
            RouteBackend::Kv => {
                if rest.is_empty() {
                    self.answer(os, w.req, Reply::text(400, "Es fehlt der Schlüssel.\n"));
                    return;
                }
                let km = match m {
                    "PUT" | "POST" => "PUT",
                    "DELETE" => "DELETE",
                    _ => "GET",
                };
                up.kv(os, km, &rest, w.req.body.clone())
            }
            RouteBackend::Url(base) => {
                let query = w.req.head.target.find('?').map(|i| &w.req.head.target[i..]).unwrap_or("");
                let target = if rest.is_empty() { format!("{}{}", base.target, query) } else { format!("{}/{}{}", base.target.trim_end_matches('/'), rest, query) };
                let mut h = Headers::new();
                for (k, v) in &w.req.head.headers.0 {
                    if !["host", "connection", "keep-alive", "content-length", "transfer-encoding"].iter().any(|x| k.eq_ignore_ascii_case(x)) {
                        h.add(k, v);
                    }
                }
                up.http(os, m, Url { host: base.host.clone(), port: base.port, target }, h, w.req.body.clone())
            }
            RouteBackend::Db(_) | RouteBackend::None => return,
        };
        self.backend_calls += 1;
        self.calls.insert(id, w);
    }

    /// a request starts through its route's statements
    fn db_start(&mut self, os: &mut OsApi, mut w: Work, route: alloc::sync::Arc<DbRoute>, rest: &str) {
        let as_json = |t: &str| -> String {
            if let Ok(n) = t.parse::<i64>() {
                return format!("{}", n);
            }
            let mut s = String::new();
            crate::dbstore::Val::Text(String::from(t)).json(&mut s);
            s
        };
        let param = match &route.param {
            DbParam::None => None,
            DbParam::Rest => Some(as_json(rest)),
            DbParam::Query(n) => crate::kv::query_value(&w.req.head.target, n).map(as_json),
            DbParam::Draw => Some(String::from("{\"draw\":true}")),
        };
        w.db = Some(DbRun { route, step: 0, txn: None, param, tries: 0 });
        self.db_send(os, w);
    }

    /// the request's next statement
    fn db_send(&mut self, os: &mut OsApi, w: Work) {
        let (Some(run), Some(db), Some(up)) = (w.db.as_ref(), self.db.as_ref(), self.upstream.as_mut()) else {
            return;
        };
        let mut body = String::from("{\"sql\":");
        crate::dbstore::Val::Text(run.route.sql[run.step].clone()).json(&mut body);
        let n = run.route.params[run.step];
        if n > 0 {
            let p = run.param.as_deref().unwrap_or("null");
            body.push_str(",\"params\":[");
            for i in 0..n {
                if i > 0 {
                    body.push(',');
                }
                body.push_str(p);
            }
            body.push(']');
        }
        if let Some(t) = run.txn {
            body.push_str(&format!(",\"txn\":{}", t));
        }
        body.push('}');
        let mut h = Headers::new();
        h.add("Content-Type", "application/json");
        let url = Url { host: db.host.clone(), port: db.port, target: String::from("/query") };
        let id = up.http(os, "POST", url, h, body.into_bytes());
        self.backend_calls += 1;
        self.calls.insert(id, w);
    }

    /// the database answered a statement of a route
    fn db_called(&mut self, os: &mut OsApi, mut w: Work, d: crate::kv::KvDone) {
        let number = |b: &[u8], name: &str| -> Option<u64> {
            let pat = format!("\"{}\":", name);
            let s = core::str::from_utf8(b).ok()?;
            let i = s.find(&pat)? + pat.len();
            let t = &s[i..];
            let e = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
            t[..e].parse().ok()
        };
        if d.status >= 500 {
            self.backend_errors += 1;
        }
        let Some(run) = w.db.as_mut() else {
            return;
        };
        let ok = d.status == 200;
        let last = run.step + 1 >= run.route.sql.len();
        if ok && !last {
            if run.txn.is_none() {
                run.txn = number(&d.body, "txn");
            }
            if let Some(k) = number(&d.body, "drawn") {
                run.param = Some(format!("{}", k));
            }
            run.step += 1;
            self.db_send(os, w);
            return;
        }
        // an answer that says it can be tried again: from the start, the same value
        let retry = d.status == 409 && d.body.windows(16).any(|x| x == b"\"retryable\":true") && run.tries < run.route.retries;
        if retry {
            run.tries += 1;
            run.step = 0;
            run.txn = None;
            self.db_send(os, w);
            return;
        }
        // a statement failed and its transaction still stands (a conflict has ended it)
        if !ok
            && d.status != 409
            && let (Some(t), Some(db), Some(up)) = (run.txn, self.db.as_ref(), self.upstream.as_mut())
        {
            let mut h = Headers::new();
            h.add("Content-Type", "application/json");
            let url = Url { host: db.host.clone(), port: db.port, target: String::from("/query") };
            up.http(os, "POST", url, h, format!("{{\"sql\":\"ROLLBACK\",\"txn\":{}}}", t).into_bytes());
        }
        let mut h = Headers::new();
        h.add("Content-Type", if d.fail.is_some() { "text/plain" } else { "application/json" });
        let body = if d.fail.is_some() { format!("Backend nicht erreichbar: {}\n", d.fail.as_ref().map(|f| f.text()).unwrap_or_default()).into_bytes() } else { d.body };
        self.answer(os, w.req, Reply { status: d.status, headers: h, body, chunk: 0, close: false });
        while (self.workers == 0 || self.busy() < self.workers)
            && let Some(next) = self.queue.pop_front()
        {
            self.begin(os, next);
        }
        self.wheel.arm(os);
    }

    /// a backend answered: the request is answered with what it said, its worker is free
    fn called(&mut self, os: &mut OsApi, d: crate::kv::KvDone) {
        let Some(w) = self.calls.remove(&d.id) else {
            return;
        };
        if w.db.is_some() {
            self.db_called(os, w, d);
            return;
        }
        if d.status >= 500 {
            self.backend_errors += 1;
        }
        let mut h = Headers::new();
        for k in ["content-type", "x-version", "x-acks", "cache-control", "x-cache"] {
            if let Some(v) = d.headers.get(k) {
                h.add(k, v);
            }
        }
        if h.get("content-type").is_none() {
            h.add("Content-Type", "text/plain");
        }
        if let Some(n) = d.node {
            h.add("X-Kv-Node", &format!("{}", n));
        }
        let body = if d.fail.is_some() { format!("Backend nicht erreichbar: {}\n", d.fail.as_ref().map(|f| f.text()).unwrap_or_default()).into_bytes() } else { d.body };
        self.answer(os, w.req, Reply { status: d.status, headers: h, body, chunk: 0, close: false });
        while (self.workers == 0 || self.busy() < self.workers)
            && let Some(next) = self.queue.pop_front()
        {
            self.begin(os, next);
        }
        self.wheel.arm(os);
    }

    fn take_requests(&mut self, os: &mut OsApi) {
        for r in self.serve.take() {
            self.arrive(os, r);
        }
        self.wheel.arm(os);
    }

    /// what the lab shows: counters and the last `n` seconds
    pub fn view(&self, now: Ns, n: usize) -> ServerView {
        let s = self.serve.stats;
        ServerView {
            port: self.serve.cfg.port,
            listening: self.serve.listening(),
            requests: s.requests,
            responses: s.responses,
            by_class: s.by_class,
            injected: self.injected,
            rejected: self.rejected,
            bad: s.bad,
            connections: s.connections,
            active: self.serve.conns() as u64,
            busy: self.busy(),
            queued: self.queued(),
            workers: self.workers,
            p50: self.lat.p50(now),
            p95: self.lat.p95(now),
            p99: self.lat.p99(now),
            slots: slots_of(&self.lat, &self.per, now, n),
        }
    }
}

/// a web server as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerView {
    pub port: u16,
    pub listening: bool,
    pub requests: u64,
    pub responses: u64,
    /// answers by class 1xx .. 5xx
    pub by_class: [u64; 5],
    pub injected: u64,
    pub rejected: u64,
    pub bad: u64,
    pub connections: u64,
    pub active: u64,
    pub busy: u32,
    pub queued: u32,
    pub workers: u32,
    /// arrival to answer over the last five minutes
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    /// per second: sent = requests come, done = answers, errors = 5xx
    pub slots: Vec<Slot>,
}

impl HttpServer {
    /// its settings in the units of its config (svcform.rs)
    pub(crate) fn settings(&self) -> Value {
        use crate::svcform::{mem_into, ms_of, rate_of, us_of, word};
        let c = &self.serve.cfg;
        let mut o = agent_core::jmap::empty_object();
        o.insert("port", Value::u64(c.port as u64));
        o.insert("workers", Value::u64(self.workers as u64));
        o.insert("queue", Value::u64(self.queue_max as u64));
        o.insert("time_ms", ms_of(self.time));
        o.insert("jitter_ms", ms_of(self.jitter));
        o.insert("cpu_ms", ms_of(self.cpu));
        o.insert("serial_us", us_of(self.serial));
        o.insert("error_rate", rate_of(self.error_ppm));
        o.insert("error_status", Value::u64(self.error_status as u64));
        o.insert("keepalive_ms", ms_of(c.keepalive));
        o.insert("max_requests", Value::u64(c.max_requests as u64));
        o.insert("header_timeout_ms", ms_of(c.header_timeout));
        o.insert("max_body", Value::u64(c.max_body));
        o.insert("backlog", Value::u64(c.backlog as u64));
        o.insert("server", word(&c.server));
        mem_into(&mut o, &self.mem);
        o
    }
}

impl Service for HttpServer {
    fn kind(&self) -> &'static str {
        "http-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        self.rng = seeded(os);
        self.work.clear();
        self.queue.clear();
        self.wheel.clear();
        self.calls.clear();
        self.lock_free = 0;
        if let Some(up) = self.upstream.as_mut() {
            up.reset();
        }
        if let Some(spec) = self.tls.clone() {
            let id = spec.identity(os, &format!("http-server:{}", self.serve.cfg.port));
            if let Some(c) = id.chain.first().and_then(|d| netpkt::x509::Cert::parse(d)) {
                os.note(Note::Tls(crate::tlssim::TlsNote::CertMade { subject: c.subject_cn, issuer: c.issuer_cn }));
            }
            self.serve.cfg.tls = Some(alloc::sync::Arc::new(id));
        }
        self.serve.start(os, "http-server");
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.done(os);
            return;
        }
        if self.serve.on(&ev, os) {
            self.take_requests(os);
            return;
        }
        let mut done = Vec::new();
        if let Some(up) = self.upstream.as_mut() {
            if !up.on(&ev, os) {
                return;
            }
            done = up.take_done();
        }
        for d in done {
            self.called(os, d);
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.serve.stop();
        self.work.clear();
        self.queue.clear();
        self.wheel.clear();
        self.calls.clear();
        if let Some(up) = self.upstream.as_mut() {
            up.reset();
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let s = self.serve.stats;
        let v = |x: u64| x as i64;
        let mut m = alloc::vec![
            Metric::new("requests", v(s.requests)),
            Metric::new("responses", v(s.responses)),
            Metric::new("status_2xx", v(s.by_class[1])),
            Metric::new("status_3xx", v(s.by_class[2])),
            Metric::new("status_4xx", v(s.by_class[3])),
            Metric::new("status_5xx", v(s.by_class[4])),
            Metric::new("errors", v(self.injected)),
            Metric::new("rejected", v(self.rejected)),
            Metric::new("bad", v(s.bad)),
            Metric::new("connections", v(s.connections)),
            Metric::new("active", self.serve.conns() as i64),
            Metric::new("inflight", self.busy() as i64),
            Metric::new("queued", self.queued() as i64),
            Metric::new("max_queued", self.max_queued as i64),
            Metric::new("bytes_in", v(s.bytes_in)),
            Metric::new("bytes_out", v(s.bytes_out)),
            Metric::new("idle_closed", v(s.idle_closed)),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
            Metric::new("busy_ms", (self.busy_ns / MS) as i64),
        ];
        if self.tls.is_some() {
            m.push(Metric::new("tls_handshakes", v(s.handshakes)));
            m.push(Metric::new("tls_failed", v(s.tls_failed)));
        }
        if let Some(up) = &self.upstream {
            m.push(Metric::new("backend_calls", v(self.backend_calls)));
            m.push(Metric::new("backend_errors", v(self.backend_errors)));
            m.push(Metric::new("failovers", v(up.stats.failovers)));
            m.push(Metric::new("backend_p99_us", us(up.latency(now).2)));
        }
        m
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        // the busy time only grows: its rate is the workers' load (1000 ms/s: one worker)
        if name == "busy_ms" { MetricKind::Counter } else { metric_kind_of(name) }
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        let mut w = alloc::vec![LatWindow::new(&self.lat, LAT)];
        if let Some(up) = &self.upstream {
            w.push(LatWindow::new(up.window(), BACKEND_LAT));
        }
        w
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        let s = self.serve.stats;
        if !self.serve.listening() {
            return msg!("Port {} ist nicht offen", self.serve.cfg.port);
        }
        if self.tls.is_some() {
            return msg!("Port {} (TLS): {} Anfragen, {} Antworten, {} in Arbeit, {} wartend", self.serve.cfg.port, s.requests, s.responses, self.busy(), self.queued());
        }
        msg!("Port {}: {} Anfragen, {} Antworten, {} in Arbeit, {} wartend", self.serve.cfg.port, s.requests, s.responses, self.busy(), self.queued())
    }
    fn footprint(&self) -> usize {
        1024 + self.serve.tls_bytes()
    }
    fn mem(&self) -> u64 {
        self.mem.bytes(self.serve.conns() as u64, 0)
    }
    fn lock_busy(&self) -> Ns {
        self.lock_busy
    }
    fn reconfigure(&mut self, c: &Value, os: Option<&mut OsApi>) -> Result<(), Msg> {
        let fresh = from_def_msg("http-server", c)?;
        let Some(f) = fresh.as_any().downcast_ref::<HttpServer>() else {
            return Err(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", "http-server"));
        };
        let (a, b) = (&self.serve.cfg, &f.serve.cfg);
        if a.port != b.port || a.backlog != b.backlog {
            return Err(msg!("Port und Warteschlange der Verbindungen ändern sich nur mit einem Neustart."));
        }
        if self.tls != f.tls {
            return Err(msg!("Das Zertifikat ändert sich nur mit einem Neustart."));
        }
        if self.kv_cfg != f.kv_cfg || self.db_cfg != f.db_cfg || self.upstream.is_some() != f.upstream.is_some() {
            return Err(msg!("Die Backends der Routen ändern sich nur mit einem Neustart."));
        }
        self.routes = f.routes.clone();
        self.queue_max = f.queue_max;
        self.time = f.time;
        self.jitter = f.jitter;
        self.error_ppm = f.error_ppm;
        self.error_status = f.error_status;
        self.root = f.root.clone();
        self.cpu = f.cpu;
        self.serial = f.serial;
        self.usl = f.usl;
        self.mem = f.mem;
        let (keepalive, max_requests, header_timeout, max_body, server) = (b.keepalive, b.max_requests, b.header_timeout, b.max_body, b.server.clone());
        let cfg = self.cfg_mut();
        cfg.keepalive = keepalive;
        cfg.max_requests = max_requests;
        cfg.header_timeout = header_timeout;
        cfg.max_body = max_body;
        cfg.server = server;
        match os {
            // more workers take waiting requests at once
            Some(os) => self.set(os, None, None, Some(f.workers)),
            None => self.workers = f.workers,
        }
        Ok(())
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

// ---------------------------------------------------------------- the client

/// a client that sends requests one after the other
#[derive(Clone, Debug)]
pub struct HttpClient {
    pub url: Url,
    pub method: String,
    /// requests to send (0: until stopped)
    pub count: u32,
    pub interval: Ns,
    pub delay: Ns,
    pub body: Vec<u8>,
    pub headers: Headers,
    pub timeout: Ns,
    pub keepalive: bool,
    agent: Agent,
    pub sent: u32,
    pub responses: u32,
    pub ok: u32,
    pub errors: u32,
    pub last_status: u16,
    pub last: Option<Ns>,
    pub last_error: Option<Fail>,
    lat: Hist,
    busy: bool,
}

impl HttpClient {
    pub fn new(url: Url) -> HttpClient {
        HttpClient {
            url,
            method: String::from("GET"),
            count: 1,
            interval: 0,
            delay: 0,
            body: Vec::new(),
            headers: Headers::new(),
            timeout: 0,
            keepalive: true,
            agent: Agent::new(K_HTTP),
            sent: 0,
            responses: 0,
            ok: 0,
            errors: 0,
            last_status: 0,
            last: None,
            last_error: None,
            lat: Hist::new(),
            busy: false,
        }
    }
    fn send(&mut self, os: &mut OsApi) {
        let mut h = self.headers.clone();
        if h.get("user-agent").is_none() {
            h.add("User-Agent", "jfn-http/1.0");
        }
        if h.get("accept").is_none() {
            h.add("Accept", "*/*");
        }
        let req = Req { method: self.method.clone(), url: self.url.clone(), version: Version::Http11, headers: h, body: self.body.clone(), timeout: self.timeout, connect_timeout: 0, reuse: self.keepalive, keep_body: false };
        self.agent.submit(os, req);
        self.sent += 1;
        self.busy = true;
    }
    fn finished(&mut self, os: &mut OsApi, d: Done) {
        self.busy = false;
        match d.result {
            Ok(r) => {
                self.responses += 1;
                self.last_status = r.head.status;
                self.last_error = None;
                if r.head.status < 400 {
                    self.ok += 1;
                } else {
                    self.errors += 1;
                }
                let l = d.end - d.start;
                self.last = Some(l);
                self.lat.record(l);
            }
            Err(f) => {
                self.errors += 1;
                self.last_status = 0;
                self.last_error = Some(f);
            }
        }
        if self.count == 0 || self.sent < self.count {
            if self.interval == 0 {
                self.send(os);
            } else {
                let at = os.now() + self.interval;
                os.timer(K_OWN, at);
            }
        } else if !self.keepalive || self.agent.active() == 0 {
            self.agent.close_idle(os);
        }
    }
}

impl Service for HttpClient {
    fn kind(&self) -> &'static str {
        "http-client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.agent.reset();
        self.sent = 0;
        self.busy = false;
        if self.delay == 0 {
            self.send(os);
        } else {
            let at = os.now() + self.delay;
            os.timer(K_OWN, at);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Timer(K_OWN) = ev {
            if !self.busy {
                self.send(os);
            }
            return;
        }
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.finished(os, d);
            }
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.agent.reset();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sent", self.sent as i64),
            Metric::new("responses", self.responses as i64),
            Metric::new("ok", self.ok as i64),
            Metric::new("errors", self.errors as i64),
            Metric::new("last_status", self.last_status as i64),
            Metric::new("last_us", us(self.last)),
            Metric::new("p50_us", us(self.lat.p50())),
            Metric::new("p95_us", us(self.lat.p95())),
            Metric::new("p99_us", us(self.lat.p99())),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        match (&self.last_error, self.last_status) {
            (Some(f), _) => msg!("{}: {}", self.url.text(), f.msg()),
            (None, 0) => msg!("{}: {} von {} gesendet", self.url.text(), self.sent, self.count),
            (None, s) => msg!("{}: {} Antworten, zuletzt {}", self.url.text(), self.responses, s),
        }
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

// ---------------------------------------------------------------- the load generator

/// how the load generator sends
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadMode {
    /// virtual users: request, answer, think, request ...
    Closed,
    /// requests arrive at a rate, whatever the answers do
    Open,
}

/// one kind of request of the mix
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixItem {
    pub method: String,
    /// path and query, or a whole URL (then it goes there instead of to the targets)
    pub path: String,
    pub weight: u32,
    pub body: Vec<u8>,
    pub headers: Headers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LTimer {
    Start,
    User(u32),
    Arrival,
    Stop,
    /// what the agent ended while it was asked comes out (come_back)
    Done,
}

/// what a virtual user does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UState {
    /// nothing (not started, or beyond the number of users)
    Idle,
    /// thinks until its timer
    Timer,
    /// waits for its answer
    Busy,
}

/// the load generator
#[derive(Clone, Debug)]
pub struct HttpLoad {
    pub targets: Vec<Url>,
    pub random_target: bool,
    pub mode: LoadMode,
    pub users: u32,
    pub think: Ns,
    pub think_exp: bool,
    /// requests per second in thousandths (open loop), the start of the ramp
    pub rate_m: u64,
    pub start_rate_m: u64,
    pub poisson: bool,
    pub ramp: Ns,
    pub delay: Ns,
    /// 0: no end
    pub duration: Ns,
    /// requests to send at most (0: no limit)
    pub max_requests: u64,
    pub timeout: Ns,
    pub connect_timeout: Ns,
    pub keepalive: bool,
    pub max_conns: u32,
    pub mix: Vec<MixItem>,
    agent: Agent,
    wheel: Wheel<LTimer>,
    rng: Pcg32,
    t0: Option<Ns>,
    stopped: bool,
    turn: usize,
    /// the user of each request in flight, and what each user does
    busy_users: BTreeMap<u64, u32>,
    /// LTimer::Done is on its way
    redo: bool,
    ustate: Vec<UState>,
    /// the open loop's schedule: a segment from seg_t whose rate goes from seg_r0 to seg_r1
    /// over seg_ramp (then stays), how far its arrivals got (expected arrivals since seg_t, in
    /// millionths) and the wheel entry of the next arrival
    seg_t: Ns,
    seg_r0: u64,
    seg_r1: u64,
    seg_ramp: Ns,
    seg_pos: u128,
    next_arrival: Option<u64>,
    pub sent: u64,
    pub responses: u64,
    pub ok: u64,
    pub http_errors: u64,
    pub failed: u64,
    pub timeouts: u64,
    pub by_class: [u64; 5],
    lat: Window,
    /// per second: sent, answers, errors (HTTP errors and failures)
    per: PerSec<4>,
    seen: Ns,
}

impl HttpLoad {
    pub fn new(targets: Vec<Url>) -> HttpLoad {
        HttpLoad {
            targets,
            random_target: false,
            mode: LoadMode::Closed,
            users: 1,
            think: 0,
            think_exp: false,
            rate_m: 10_000,
            start_rate_m: 0,
            poisson: false,
            ramp: 0,
            delay: 0,
            duration: 0,
            max_requests: 0,
            timeout: 5 * SEC,
            connect_timeout: 0,
            keepalive: true,
            max_conns: 0,
            mix: alloc::vec![MixItem { method: String::from("GET"), path: String::from("/"), weight: 1, body: Vec::new(), headers: Headers::new() }],
            agent: Agent::new(K_HTTP),
            wheel: Wheel::new(K_OWN),
            rng: Pcg32::new(0, 0),
            t0: None,
            stopped: false,
            turn: 0,
            busy_users: BTreeMap::new(),
            redo: false,
            ustate: Vec::new(),
            seg_t: 0,
            seg_r0: 0,
            seg_r1: 0,
            seg_ramp: 0,
            seg_pos: 0,
            next_arrival: None,
            sent: 0,
            responses: 0,
            ok: 0,
            http_errors: 0,
            failed: 0,
            timeouts: 0,
            by_class: [0; 5],
            lat: lat_window(),
            per: PerSec::new(),
            seen: 0,
        }
    }
    /// requests in flight
    pub fn inflight(&self) -> u64 {
        self.agent.active() as u64
    }
    /// the latencies of the answers in [from, to) as one histogram (whole seconds: its slots)
    pub fn lat_between(&self, from: Ns, to: Ns) -> Hist {
        self.lat.hist_between(from, to)
    }
    /// still sending new requests
    pub fn running(&self) -> bool {
        self.t0.is_some() && !self.stopped
    }
    /// the rate the open loop aims at now (thousandths of a request per second)
    pub fn rate_now(&self, now: Ns) -> u64 {
        if self.t0.is_none() {
            return 0;
        }
        let k = now.saturating_sub(self.seg_t);
        if self.seg_ramp == 0 || k >= self.seg_ramp {
            return self.seg_r1;
        }
        let (a, b) = (self.seg_r0 as i128, self.seg_r1 as i128);
        (a + (b - a) * k as i128 / self.seg_ramp as i128).max(0) as u64
    }
    /// changes the load while it runs (Action::HttpLoadSet): a new rate holds from now on (no
    /// ramp); more users start now, fewer stop after their answer
    pub fn set_load(&mut self, os: &mut OsApi, rate_m: Option<u64>, users: Option<u32>) {
        let now = os.now();
        if let Some(r) = rate_m {
            self.rate_m = r;
            self.start_rate_m = r;
            self.ramp = 0;
            // the schedule starts again from now at the new rate
            if self.mode == LoadMode::Open && self.running() {
                self.segment(now, r, r, 0);
                self.schedule_next(false);
            }
        }
        if let Some(u) = users {
            self.users = u;
            if self.mode == LoadMode::Closed && self.running() {
                if self.ustate.len() < u as usize {
                    self.ustate.resize(u as usize, UState::Idle);
                }
                for i in 0..u {
                    if self.ustate[i as usize] == UState::Idle {
                        self.ustate[i as usize] = UState::Timer;
                        self.wheel.add(now, LTimer::User(i));
                    }
                }
            }
        }
        self.wheel.arm(os);
    }

    fn pick(&mut self) -> (String, Url, Vec<u8>, Headers) {
        let total: u64 = self.mix.iter().map(|m| m.weight.max(1) as u64).sum();
        let mut r = if self.mix.len() > 1 { self.rng.below(total.min(u32::MAX as u64) as u32) as u64 } else { 0 };
        let mut item = self.mix[0].clone();
        for m in &self.mix {
            let w = m.weight.max(1) as u64;
            if r < w {
                item = m.clone();
                break;
            }
            r -= w;
        }
        let item = MixItem { path: self.fill(&item.path), body: if item.body.contains(&b'{') { self.fill(&String::from_utf8_lossy(&item.body)).into_bytes() } else { item.body }, ..item };
        let url = if item.path.contains("://") {
            Url::parse(&item.path).unwrap_or_else(|_| self.targets[0].clone())
        } else {
            let i = if self.random_target && self.targets.len() > 1 {
                self.rng.below(self.targets.len() as u32) as usize
            } else {
                let i = self.turn % self.targets.len();
                self.turn += 1;
                i
            };
            let t = &self.targets[i];
            Url { host: t.host.clone(), port: t.port, target: if item.path.is_empty() { t.target.clone() } else { item.path.clone() } }
        };
        (item.method, url, item.body, item.headers)
    }

    /// a path or body with {seq}, {rand:N} and {hot:N} filled in (random numbers only when
    /// it names them)
    fn fill(&mut self, s: &str) -> String {
        if !s.contains('{') {
            return String::from(s);
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let Some(j) = rest[i..].find('}') else {
                out.push_str(&rest[i..]);
                return out;
            };
            let tag = &rest[i + 1..i + j];
            let n = |t: &str| t.parse::<u32>().ok().filter(|&n| n > 0);
            if tag == "seq" {
                out.push_str(&format!("{}", self.sent));
            } else if let Some(k) = tag.strip_prefix("rand:").and_then(n) {
                out.push_str(&format!("{}", self.rng.below(k)));
            } else if let Some(k) = tag.strip_prefix("hot:").and_then(n) {
                let hot = (k / 5).max(1);
                let v = if hot >= k || self.rng.chance(4, 5) { self.rng.below(hot) } else { hot + self.rng.below(k - hot) };
                out.push_str(&format!("{}", v));
            } else {
                out.push_str(&rest[i..i + j + 1]);
            }
            rest = &rest[i + j + 1..];
        }
        out.push_str(rest);
        out
    }

    /// one request, for a user (closed loop) or an arrival
    fn fire(&mut self, os: &mut OsApi, user: Option<u32>) -> bool {
        if self.stopped || self.targets.is_empty() || (self.max_requests > 0 && self.sent >= self.max_requests) {
            return false;
        }
        let now = os.now();
        let (method, url, body, extra) = self.pick();
        let mut h = Headers::new();
        h.add("User-Agent", "jfn-load/1.0");
        h.add("Accept", "*/*");
        for (k, v) in &extra.0 {
            h.set(k, v);
        }
        let req = Req { method, url, version: Version::Http11, headers: h, body, timeout: self.timeout, connect_timeout: self.connect_timeout, reuse: self.keepalive, keep_body: false };
        let id = self.agent.submit(os, req);
        self.sent += 1;
        self.per.add(now, 0, 1);
        if let Some(u) = user {
            self.busy_users.insert(id, u);
            if let Some(st) = self.ustate.get_mut(u as usize) {
                *st = UState::Busy;
            }
        }
        true
    }

    fn begin(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.t0 = Some(now);
        self.agent.max_per_host = self.max_conns as usize;
        if self.duration > 0 {
            self.wheel.add(now + self.duration, LTimer::Stop);
        }
        match self.mode {
            LoadMode::Closed => {
                let n = self.users;
                self.ustate = alloc::vec![UState::Timer; n as usize];
                for i in 0..n {
                    let at = now + self.ramp * i as u64 / n.max(1) as u64;
                    self.wheel.add(at, LTimer::User(i));
                }
            }
            LoadMode::Open => {
                let r0 = if self.ramp > 0 { self.start_rate_m } else { self.rate_m };
                self.segment(now, r0, self.rate_m, self.ramp);
                self.schedule_next(true);
            }
        }
    }

    /// a new piece of the open loop's schedule
    fn segment(&mut self, t: Ns, r0: u64, r1: u64, ramp: Ns) {
        self.seg_t = t;
        self.seg_r0 = r0;
        self.seg_r1 = r1;
        self.seg_ramp = ramp;
        self.seg_pos = 0;
    }

    /// the next arrival of the open loop: the moment the expected number of arrivals (the
    /// integral of the rate) reaches the next whole one, or, for Poisson arrivals, grows by an
    /// exponential amount (the time change of an inhomogeneous Poisson process); `first`: the
    /// first one of a run comes at once (uniform)
    fn schedule_next(&mut self, first: bool) {
        if let Some(id) = self.next_arrival.take() {
            self.wheel.cancel(id);
        }
        let step: u128 = if self.poisson { exp_sample(1_000_000, self.rng.next_u32()) as u128 } else if first { 0 } else { 1_000_000 };
        self.seg_pos += step;
        if let Some(off) = arrival_offset(self.seg_r0, self.seg_r1, self.seg_ramp, self.seg_pos) {
            self.next_arrival = Some(self.wheel.add(self.seg_t + off, LTimer::Arrival));
        }
    }

    /// a request that cannot even start (no route, ARP gave up) fails in the call that makes it:
    /// what the agent ended so comes out 100 ms later, not in that call and not never (a closed
    /// loop's users would wait for an event that does not come, a target that reboots would stay
    /// without load)
    fn come_back(&mut self, os: &mut OsApi) {
        if self.agent.has_done() && !self.redo {
            self.redo = true;
            let at = os.now() + 100 * MS;
            self.wheel.add(at, LTimer::Done);
        }
    }

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                LTimer::Done => {
                    self.redo = false;
                    for d in self.agent.take_done() {
                        self.finished(os, d);
                    }
                }
                LTimer::Start => self.begin(os),
                LTimer::User(u) => {
                    if let Some(st) = self.ustate.get_mut(u as usize) {
                        *st = UState::Idle;
                    }
                    if u < self.users && !self.fire(os, Some(u)) {
                        self.check_end(os);
                    }
                }
                LTimer::Arrival => {
                    self.next_arrival = None;
                    if self.fire(os, None) {
                        self.schedule_next(false);
                    } else {
                        self.check_end(os);
                    }
                }
                LTimer::Stop => {
                    self.stopped = true;
                    self.check_end(os);
                }
            }
        }
        self.come_back(os);
        self.wheel.arm(os);
    }

    /// after the end: idle connections close once nothing is in flight
    fn check_end(&mut self, os: &mut OsApi) {
        if (self.stopped || (self.max_requests > 0 && self.sent >= self.max_requests)) && self.agent.active() == 0 {
            self.stopped = true;
            self.agent.close_idle(os);
        }
    }

    fn finished(&mut self, os: &mut OsApi, d: Done) {
        let now = os.now();
        let user = self.busy_users.remove(&d.id);
        let lat = d.end - d.start;
        match &d.result {
            Ok(r) => {
                self.responses += 1;
                let st = r.head.status;
                self.by_class[((st / 100).clamp(1, 5) - 1) as usize] += 1;
                self.lat.record(now, lat);
                self.per.add(now, 1, 1);
                if st >= 400 {
                    self.http_errors += 1;
                    self.per.add(now, 2, 1);
                } else {
                    self.ok += 1;
                }
            }
            Err(f) => {
                self.failed += 1;
                if f.is_timeout() {
                    self.timeouts += 1;
                }
                self.per.add(now, 2, 1);
            }
        }
        if let Some(u) = user {
            if let Some(st) = self.ustate.get_mut(u as usize) {
                *st = UState::Idle;
            }
            if u >= self.users || self.stopped {
                self.check_end(os);
                return;
            }
            let think = if self.think == 0 {
                0
            } else if self.think_exp {
                exp_sample(self.think, self.rng.next_u32())
            } else {
                self.think
            };
            if think == 0 {
                if !self.fire(os, Some(u)) {
                    self.check_end(os);
                }
            } else {
                if let Some(st) = self.ustate.get_mut(u as usize) {
                    *st = UState::Timer;
                }
                self.wheel.add(now + think, LTimer::User(u));
            }
        } else {
            self.check_end(os);
        }
    }

    /// what the lab shows: counters, quantiles and the last `n` seconds
    pub fn view(&self, now: Ns, n: usize) -> LoadView {
        LoadView {
            running: self.running(),
            mode: self.mode,
            sent: self.sent,
            responses: self.responses,
            ok: self.ok,
            http_errors: self.http_errors,
            failed: self.failed,
            timeouts: self.timeouts,
            inflight: self.inflight(),
            connections: self.agent.stats.connects,
            by_class: self.by_class,
            users: if self.mode == LoadMode::Closed { self.users } else { 0 },
            rate_m: if self.mode == LoadMode::Open { self.rate_now(now) } else { 0 },
            rps_m: self.per.sum(now, 1, 10) * 100,
            p50: self.lat.p50(now),
            p95: self.lat.p95(now),
            p99: self.lat.p99(now),
            mean: self.lat.mean(now),
            max: self.lat.max(now),
            slots: slots_of(&self.lat, &self.per, now, n),
        }
    }
}

/// a load generator as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadView {
    pub running: bool,
    pub mode: LoadMode,
    pub sent: u64,
    pub responses: u64,
    pub ok: u64,
    pub http_errors: u64,
    pub failed: u64,
    pub timeouts: u64,
    pub inflight: u64,
    pub connections: u64,
    pub by_class: [u64; 5],
    /// closed loop: virtual users; open loop: the rate now (thousandths per second)
    pub users: u32,
    pub rate_m: u64,
    /// answers per second over the last 10 s, in thousandths
    pub rps_m: u64,
    /// latencies over the last five minutes
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    pub mean: Option<Ns>,
    pub max: Option<Ns>,
    /// per second: sent, done (answers), errors (HTTP errors and failures)
    pub slots: Vec<Slot>,
}

impl HttpLoad {
    /// its settings in the units of its config (svcform.rs)
    pub(crate) fn settings(&self) -> Value {
        use crate::svcform::{ms_of, word};
        let mut o = agent_core::jmap::empty_object();
        o.insert("mode", word(if self.mode == LoadMode::Open { "open" } else { "closed" }));
        o.insert("users", Value::u64(self.users as u64));
        o.insert("think_ms", ms_of(self.think));
        o.insert("think", word(if self.think_exp { "exp" } else { "fixed" }));
        o.insert("rps", crate::svcform::thousandths(self.rate_m));
        o.insert("start_rps", crate::svcform::thousandths(self.start_rate_m));
        o.insert("arrival", word(if self.poisson { "poisson" } else { "uniform" }));
        o.insert("ramp_ms", ms_of(self.ramp));
        o.insert("delay_ms", ms_of(self.delay));
        o.insert("duration_ms", ms_of(self.duration));
        o.insert("requests", Value::u64(self.max_requests));
        o.insert("timeout_ms", ms_of(self.timeout));
        o.insert("connect_timeout_ms", ms_of(self.connect_timeout));
        o.insert("keepalive", Value::Bool(self.keepalive));
        o.insert("max_conns", Value::u64(self.max_conns as u64));
        o.insert("balance", word(if self.random_target { "random" } else { "round-robin" }));
        o
    }
}

impl Service for HttpLoad {
    fn kind(&self) -> &'static str {
        "http-load"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        self.rng = seeded(os);
        self.agent.reset();
        self.wheel.clear();
        self.busy_users.clear();
        self.ustate.clear();
        self.next_arrival = None;
        self.t0 = None;
        self.stopped = false;
        self.turn = 0;
        self.agent.max_per_host = self.max_conns as usize;
        let at = os.now() + self.delay;
        self.wheel.add(at, LTimer::Start);
        self.wheel.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.timers(os);
            return;
        }
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.finished(os, d);
            }
            self.come_back(os);
            self.wheel.arm(os);
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.agent.reset();
        self.wheel.clear();
        self.busy_users.clear();
        self.stopped = true;
        self.redo = false;
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let v = |x: u64| x as i64;
        let done = self.responses + self.failed;
        let err = self.http_errors + self.failed;
        alloc::vec![
            Metric::new("sent", v(self.sent)),
            Metric::new("responses", v(self.responses)),
            Metric::new("ok", v(self.ok)),
            Metric::new("http_errors", v(self.http_errors)),
            Metric::new("failed", v(self.failed)),
            Metric::new("timeouts", v(self.timeouts)),
            Metric::new("errors", v(err)),
            Metric::new("inflight", v(self.inflight())),
            Metric::new("connections", v(self.agent.stats.connects)),
            Metric::new("status_2xx", v(self.by_class[1])),
            Metric::new("status_3xx", v(self.by_class[2])),
            Metric::new("status_4xx", v(self.by_class[3])),
            Metric::new("status_5xx", v(self.by_class[4])),
            Metric::new("users", if self.mode == LoadMode::Closed { self.users as i64 } else { 0 }),
            Metric::new("rate_mrps", v(if self.mode == LoadMode::Open { self.rate_now(now) } else { 0 })),
            Metric::new("rps", v(self.per.sum(now, 1, 10) / 10)),
            Metric::new("err_pm", if done == 0 { 0 } else { (err * 1000 / done) as i64 }),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
            Metric::new("mean_us", us(self.lat.mean(now))),
            Metric::new("max_us", us(self.lat.max(now))),
        ]
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LOAD_LAT)]
    }
    fn reconfigure(&mut self, c: &Value, os: Option<&mut OsApi>) -> Result<(), Msg> {
        let fresh = from_def_msg("http-load", c)?;
        let Some(f) = fresh.as_any().downcast_ref::<HttpLoad>() else {
            return Err(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", "http-load"));
        };
        if self.mode != f.mode {
            return Err(msg!("Offene und geschlossene Last wechseln nur mit einem Neustart."));
        }
        if self.duration != f.duration {
            return Err(msg!("Die Dauer eines Laufs ändert sich nur mit einem Neustart."));
        }
        self.targets = f.targets.clone();
        self.random_target = f.random_target;
        self.think = f.think;
        self.think_exp = f.think_exp;
        self.poisson = f.poisson;
        self.delay = f.delay;
        self.max_requests = f.max_requests;
        self.timeout = f.timeout;
        self.connect_timeout = f.connect_timeout;
        self.keepalive = f.keepalive;
        self.max_conns = f.max_conns;
        self.agent.max_per_host = f.max_conns as usize;
        self.mix = f.mix.clone();
        let rate = if self.rate_m != f.rate_m { Some(f.rate_m) } else { None };
        let users = if self.users != f.users { Some(f.users) } else { None };
        match os {
            // the schedule goes on from now at the new rate, more users start now
            Some(os) => self.set_load(os, rate, users),
            None => {
                self.rate_m = f.rate_m;
                self.users = f.users;
            }
        }
        self.ramp = f.ramp;
        self.start_rate_m = f.start_rate_m;
        Ok(())
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        let what = match self.mode {
            LoadMode::Closed => msg!("{} Nutzer", self.users),
            LoadMode::Open => msg!("{}.{} Anfragen/s", self.rate_m / 1000, format!("{:03}", self.rate_m % 1000)),
        };
        let state = if self.running() { phrase("läuft") } else if self.t0.is_none() { phrase("wartet") } else { phrase("beendet") };
        msg!("{} ({}): {} gesendet, {} Antworten, {} Fehler", what, state, self.sent, self.responses, self.http_errors + self.failed)
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

fn url_of(s: &str) -> Result<Url, Msg> {
    Url::parse(s).map_err(|e| match e {
        crate::http::UrlErr::Scheme(s) => msg!("\u{201e}{}\u{201c}: nur http geht.", s),
        crate::http::UrlErr::NoHost => msg!("Die URL nennt keinen Host."),
        crate::http::UrlErr::Port => msg!("Der Port der URL ist keine Zahl bis 65535."),
        crate::http::UrlErr::BadHost => msg!("Der Host der URL hat ungültige Zeichen."),
    })
}

/// an HTTP service from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(kind: &str, c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(kind, c).map_err(|m| m.text())
}

/// a route's statements from its config: "sql" (one or a list), "param", "retries"
fn db_route_of(r: &Value) -> Result<DbRoute, Msg> {
    let list: Vec<String> = match r.get("sql") {
        Some(Value::String(s)) => alloc::vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(String::from).collect(),
        _ => Vec::new(),
    };
    if list.is_empty() {
        return Err(msg!("Eine Route mit \u{201e}backend\u{201c} db braucht \u{201e}sql\u{201c}."));
    }
    let mut params = Vec::with_capacity(list.len());
    for s in &list {
        match crate::dbsql::parse(s) {
            Ok(st) => params.push(crate::dbsql::params_of(&st)),
            Err(e) => return Err(msg!("Die Anweisung \u{201e}{}\u{201c}: {}", s.clone(), e.line())),
        }
    }
    let param = match r.get("param").and_then(|v| v.as_str()).unwrap_or("rest") {
        "rest" => DbParam::Rest,
        "none" => DbParam::None,
        "draw" => DbParam::Draw,
        p => match p.strip_prefix("query:") {
            Some(n) => DbParam::Query(String::from(n)),
            None => return Err(msg!("\u{201e}param\u{201c} kennt {} nicht (rest, draw, query:NAME, none).", p)),
        },
    };
    Ok(DbRoute { sql: list, params, param, retries: r.get("retries").and_then(|v| v.as_u64()).unwrap_or(0).min(1000) as u32 })
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(kind: &str, c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let flag = |k: &str, d: bool| c.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(String::from);
    let port = |d: u16| -> Result<u16, Msg> {
        match num("port", d as u64) {
            p @ 1..=65535 => Ok(p as u16),
            p => Err(msg!("Port {} gibt es nicht.", p)),
        }
    };
    Ok(match kind {
        "http-server" => {
            let mut s = HttpServer::new(port(if c.get("tls").is_some() { netpkt::tls::PORT } else { 80 })?);
            if let Some(t) = c.get("tls") {
                s.tls = Some(crate::tlssim::TlsSpec::of(t)?);
            }
            s.workers = num("workers", 0).min(1_000_000) as u32;
            s.queue_max = num("queue", 0).min(10_000_000) as u32;
            s.time = ms("time_ms", 0);
            s.jitter = ms("jitter_ms", 0);
            s.error_ppm = c.get("error_rate").and_then(rate_ppm).unwrap_or(0);
            s.error_status = num("error_status", 500).clamp(100, 599) as u16;
            s.cpu = c.get("cpu_ms").and_then(|v| decimal(v, MS)).unwrap_or(s.cpu);
            s.serial = c.get("serial_us").and_then(|v| decimal(v, crate::kernel::US)).unwrap_or(0);
            s.usl = crate::res::Usl::of(c);
            s.mem = crate::res::SvcMem::of(c, 32, 64);
            {
                let cfg = s.cfg_mut();
                cfg.keepalive = ms("keepalive_ms", 75_000);
                cfg.max_requests = num("max_requests", 1000).min(u32::MAX as u64) as u32;
                cfg.header_timeout = ms("header_timeout_ms", 60_000);
                cfg.max_body = num("max_body", 1 << 20);
                cfg.backlog = num("backlog", 0).min(65535) as u32;
                if let Some(sv) = text("server") {
                    cfg.server = sv;
                }
            }
            for r in c.get("routes").and_then(|r| r.as_array()).unwrap_or(&[]) {
                let rs = |k: &str| r.get(k).and_then(|v| v.as_str()).map(String::from);
                let rn = |k: &str| r.get(k).and_then(|v| v.as_u64());
                let mut rt = HttpRoute::new(&rs("path").unwrap_or_else(|| String::from("*")), &rs("body").unwrap_or_default());
                rt.method = rs("method").unwrap_or_else(|| String::from("GET")).to_ascii_uppercase();
                rt.status = rn("status").unwrap_or(200).clamp(100, 599) as u16;
                if let Some(t) = rs("type") {
                    rt.ctype = t;
                }
                rt.size = rn("size").unwrap_or(0).min(64 << 20) as usize;
                rt.chunk = rn("chunk").unwrap_or(0).min(1 << 20) as usize;
                rt.time = r.get("time_ms").and_then(|v| decimal(v, MS));
                rt.jitter = r.get("jitter_ms").and_then(|v| decimal(v, MS));
                rt.error_ppm = r.get("error_rate").and_then(rate_ppm);
                rt.error_status = rn("error_status").map(|x| x.clamp(100, 599) as u16);
                rt.cpu = r.get("cpu_ms").and_then(|v| decimal(v, MS));
                rt.serial = r.get("serial_us").and_then(|v| decimal(v, crate::kernel::US));
                rt.headers = headers_of(r.get("headers"));
                rt.backend = match rs("backend").as_deref() {
                    None | Some("") | Some("none") => RouteBackend::None,
                    Some("kv") => RouteBackend::Kv,
                    Some("db") => RouteBackend::Db(alloc::sync::Arc::new(db_route_of(r)?)),
                    Some(u) => RouteBackend::Url(url_of(u)?),
                };
                if !rt.path.starts_with('/') && rt.path != "*" {
                    return Err(msg!("Der Pfad \u{201e}{}\u{201c} beginnt nicht mit /.", rt.path));
                }
                s.routes.push(rt);
            }
            if let Some(root) = text("root") {
                if !root.starts_with('/') {
                    return Err(msg!("\u{201e}root\u{201c} muss ein absoluter Pfad sein: {}", root));
                }
                s.root = Some(root);
            }
            if s.routes.is_empty() && s.root.is_none() {
                let mut r = HttpRoute::new("*", "Hallo von {host}\n");
                r.method = String::new();
                s.routes.push(r);
            }
            if let Some(k) = c.get("kv") {
                s.upstream = Some(crate::kv::client_of(k, K_UP)?);
                s.kv_cfg = Some(k.clone());
            } else if s.routes.iter().any(|r| r.backend == RouteBackend::Kv) {
                return Err(msg!("Eine Route fragt den KV-Speicher, aber \u{201e}kv\u{201c} fehlt."));
            }
            if s.upstream.is_none() && s.routes.iter().any(|r| matches!(r.backend, RouteBackend::Url(_))) {
                s.upstream = Some(crate::kv::KvClient::new(K_UP));
            }
            if let Some(d) = c.get("db") {
                let addr = d.get("addr").and_then(|v| v.as_str()).unwrap_or("");
                if netpkt::Ipv4::parse(addr.trim()).is_none() {
                    return Err(msg!("\u{201e}db.addr\u{201c}: \u{201e}{}\u{201c} ist keine IPv4-Adresse.", addr));
                }
                let port = match d.get("port").and_then(|v| v.as_u64()).unwrap_or(crate::db::DB_PORT as u64) {
                    p @ 1..=65535 => p as u16,
                    p => return Err(msg!("Port {} gibt es nicht.", p)),
                };
                s.db = Some(DbBackend { host: String::from(addr.trim()), port });
                s.db_cfg = Some(d.clone());
                let up = s.upstream.get_or_insert_with(|| crate::kv::KvClient::new(K_UP));
                up.timeout = up.timeout.max(d.get("timeout_ms").and_then(|v| decimal(v, MS)).unwrap_or(5000 * MS).max(MS));
                up.set_conns(d.get("conns").and_then(|v| v.as_u64()).unwrap_or(32).clamp(1, 100_000) as usize);
            } else if s.routes.iter().any(|r| matches!(r.backend, RouteBackend::Db(_))) {
                return Err(msg!("Eine Route fragt die Datenbank, aber \u{201e}db\u{201c} fehlt."));
            }
            Box::new(s)
        }
        "http-client" => {
            let url = url_of(&text("url").ok_or_else(|| msg!("\u{201e}url\u{201c} fehlt."))?)?;
            let mut s = HttpClient::new(url);
            s.method = text("method").unwrap_or_else(|| String::from("GET")).to_ascii_uppercase();
            s.count = num("count", 1).min(u32::MAX as u64) as u32;
            s.interval = ms("interval_ms", 0);
            s.delay = ms("delay_ms", 0);
            s.body = text("body").unwrap_or_default().into_bytes();
            s.headers = headers_of(c.get("headers"));
            s.timeout = ms("timeout_ms", 0);
            s.keepalive = flag("keepalive", true);
            Box::new(s)
        }
        "http-load" => {
            let mut targets = Vec::new();
            if let Some(t) = text("target") {
                targets.push(url_of(&t)?);
            }
            for t in c.get("targets").and_then(|t| t.as_array()).unwrap_or(&[]) {
                if let Some(s) = t.as_str() {
                    targets.push(url_of(s)?);
                }
            }
            if targets.is_empty() {
                return Err(msg!("\u{201e}targets\u{201c} fehlt."));
            }
            let mut s = HttpLoad::new(targets);
            s.mode = match text("mode").as_deref().unwrap_or("closed") {
                "closed" => LoadMode::Closed,
                "open" => LoadMode::Open,
                m => return Err(msg!("\u{201e}mode\u{201c} kennt {} nicht (closed, open).", m)),
            };
            s.random_target = text("balance").as_deref() == Some("random");
            s.users = num("users", 1).clamp(1, 100_000) as u32;
            s.think = ms("think_ms", 0);
            s.think_exp = text("think").as_deref() == Some("exp");
            s.rate_m = c.get("rps").and_then(|v| decimal(v, 1000)).unwrap_or(10_000);
            s.start_rate_m = c.get("start_rps").and_then(|v| decimal(v, 1000)).unwrap_or(0);
            s.poisson = text("arrival").as_deref() == Some("poisson");
            s.ramp = ms("ramp_ms", 0);
            s.delay = ms("delay_ms", 0);
            s.duration = ms("duration_ms", 0);
            s.max_requests = num("requests", 0);
            s.timeout = ms("timeout_ms", 5000);
            s.connect_timeout = ms("connect_timeout_ms", 0);
            s.keepalive = flag("keepalive", true);
            s.max_conns = num("max_conns", 0).min(1_000_000) as u32;
            if s.ramp == 0 {
                s.start_rate_m = s.rate_m;
            }
            let mut mix = Vec::new();
            for m in c.get("mix").and_then(|m| m.as_array()).unwrap_or(&[]) {
                let ms_ = |k: &str| m.get(k).and_then(|v| v.as_str()).map(String::from);
                mix.push(MixItem {
                    method: ms_("method").unwrap_or_else(|| String::from("GET")).to_ascii_uppercase(),
                    path: ms_("path").unwrap_or_default(),
                    weight: m.get("weight").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 1_000_000) as u32,
                    body: ms_("body").unwrap_or_default().into_bytes(),
                    headers: headers_of(m.get("headers")),
                });
            }
            if !mix.is_empty() {
                s.mix = mix;
            } else if let Some(t) = s.targets.first() {
                s.mix[0].path = t.target.clone();
            }
            Box::new(s)
        }
        k => return Err(msg!("Unbekannte Dienstart \u{201e}{}\u{201c}.", k)),
    })
}

// ---------------------------------------------------------------- the engine side

impl Engine {
    /// a web server's view for the lab (the last `slots` seconds)
    pub fn http_server_view(&self, dev: u32, svc: &str, slots: usize) -> Option<ServerView> {
        Some(self.service::<HttpServer>(dev, svc)?.view(self.now, slots))
    }
    /// a load generator's view for the lab (the last `slots` seconds)
    pub fn http_load_view(&self, dev: u32, svc: &str, slots: usize) -> Option<LoadView> {
        Some(self.service::<HttpLoad>(dev, svc)?.view(self.now, slots))
    }
}
