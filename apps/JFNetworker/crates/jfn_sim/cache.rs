// cache -- a cache in front of a web server (or the store): GET and HEAD from memory while the
// answer is fresh, everything else and the misses from the backend; LRU with a time to live,
// hits and misses counted
//
//   kind   config (JSON; every member may be left out but "backend")
//   cache  {"port": 80, "backend": "http://10.0.2.11/", "entries": 1000, "max_kb": 65536,
//           "ttl_ms": 30000, "cache_control": true, "hit_us": 50, "collapse": true,
//           "timeout_ms": 5000, "connect_timeout_ms": 1000}
//
// A GET or HEAD is a hit when the cache holds a fresh answer for its target (path and query):
// it goes back after hit_us with X-Cache: HIT and Age.  A miss asks the backend (the target
// appended to the backend's path); answers 200, 203, 204, 301, 404 and 410 are kept for
// ttl_ms, unless the backend's Cache-Control says otherwise (max-age wins, no-store, no-cache
// and private keep nothing) when "cache_control" is on.  With "collapse" the misses of one
// target wait for the first one's answer instead of asking the backend again.  Other methods go
// to the backend and the target is dropped (invalidation by writing).  Over "entries" or
// "max_kb" the least recently used answers go first.
//
// Metrics: requests, hits, misses, hit_pm (hits per mille of GET and HEAD), collapsed,
// expired, evictions, invalidations, entries, bytes, backend_requests, backend_errors, p50_us,
// p95_us, p99_us (arrival to answer), hit_p50_us, miss_p50_us.  Engine::cache_view for the
// lab, cachectl on the console.  The time series (series.rs) take p50_us .. p99_us,
// hit_p50_us .. hit_p99_us and miss_p50_us .. miss_p99_us of each interval from the windows.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use base::lru::Lru;
use base::metrics::Window;
use netpkt::http::{Headers, Version};

use crate::http::{Agent, Done, Reply, Req, ReqIn, Serve, ServeCfg, Url, Wheel};
use crate::httpsvc::decimal;
use crate::kernel::{Engine, Ns, MS, SEC, US};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{LatStat, LatWindow, LAT};
use crate::trace::Note;

const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
const K_AGENT: u32 = 2;

/// the time series of the hits' and the misses' latencies (series.rs)
const HIT_LAT: &[(&str, LatStat)] = &[("hit_p50_us", LatStat::Quantile(500)), ("hit_p95_us", LatStat::Quantile(950)), ("hit_p99_us", LatStat::Quantile(990))];
const MISS_LAT: &[(&str, LatStat)] = &[("miss_p50_us", LatStat::Quantile(500)), ("miss_p95_us", LatStat::Quantile(950)), ("miss_p99_us", LatStat::Quantile(990))];

/// a kept answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub stored: Ns,
    pub expires: Ns,
}

/// a request that waits for the backend
#[derive(Clone, Debug)]
struct Waiter {
    sid: u64,
    at: Ns,
}

/// what a backend request is for
#[derive(Clone, Debug)]
enum Fetch {
    /// a miss of this target (and the requests collapsed onto it)
    Miss { key: String, first: Waiter },
    /// a request passed on (not GET or HEAD): the target to drop afterwards
    Pass { key: String, w: Waiter },
}

/// the cache
#[derive(Clone, Debug)]
pub struct Cache {
    pub port: u16,
    pub backend: Url,
    pub ttl: Ns,
    pub cache_control: bool,
    pub hit_time: Ns,
    pub collapse: bool,
    pub timeout: Ns,
    pub connect_timeout: Ns,
    store: Lru<String, Cached>,
    serve: Serve,
    agent: Agent,
    wheel: Wheel<(u64, Reply, Ns)>,
    fetch: BTreeMap<u64, Fetch>,
    /// targets being fetched -> the requests waiting for them
    waiting: BTreeMap<String, Vec<Waiter>>,
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub collapsed: u64,
    pub expired: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub backend_requests: u64,
    pub backend_errors: u64,
    lat: Window,
    hit_lat: Window,
    miss_lat: Window,
    seen: Ns,
}

fn hop_by_hop(name: &str) -> bool {
    ["connection", "keep-alive", "transfer-encoding", "content-length", "te", "trailer", "upgrade"].iter().any(|h| name.eq_ignore_ascii_case(h))
}

/// how long the backend lets an answer be kept: Some(0) for not at all, None: no word on it
fn max_age(h: &Headers) -> Option<Ns> {
    let cc = h.get("cache-control")?.to_ascii_lowercase();
    if ["no-store", "no-cache", "private"].iter().any(|t| cc.split(',').any(|p| p.trim() == *t)) {
        return Some(0);
    }
    cc.split(',').find_map(|p| p.trim().strip_prefix("max-age=")?.trim().parse::<u64>().ok()).map(|s| s * SEC)
}

impl Cache {
    pub fn new(backend: Url) -> Cache {
        let cfg = ServeCfg { port: 80, server: String::new(), max_requests: 0, ..ServeCfg::default() };
        Cache {
            port: 80,
            backend,
            ttl: 30 * SEC,
            cache_control: true,
            hit_time: 50 * US,
            collapse: true,
            timeout: 5 * SEC,
            connect_timeout: SEC,
            store: Lru::new(1000, 64 << 20),
            serve: Serve::new(cfg, K_SERVE),
            agent: Agent::new(K_AGENT),
            wheel: Wheel::new(K_OWN),
            fetch: BTreeMap::new(),
            waiting: BTreeMap::new(),
            requests: 0,
            hits: 0,
            misses: 0,
            collapsed: 0,
            expired: 0,
            evictions: 0,
            invalidations: 0,
            backend_requests: 0,
            backend_errors: 0,
            lat: Window::new(300, SEC),
            hit_lat: Window::new(300, SEC),
            miss_lat: Window::new(300, SEC),
            seen: 0,
        }
    }
    /// entries kept now
    pub fn entries(&self) -> usize {
        self.store.len()
    }
    /// the bytes of the kept bodies
    pub fn bytes(&self) -> u64 {
        self.store.weight()
    }
    /// the kept targets, the least recently used first
    pub fn keys(&self) -> Vec<String> {
        self.store.keys().cloned().collect()
    }
    /// a kept answer without using it
    pub fn peek(&self, key: &str) -> Option<&Cached> {
        self.store.peek(&String::from(key))
    }
    /// forgets every answer (Action::CacheClear)
    pub fn clear(&mut self) {
        self.invalidations += self.store.len() as u64;
        self.store.clear();
    }
    /// hits per mille of the GET and HEAD requests
    pub fn hit_pm(&self) -> u64 {
        let all = self.hits + self.misses;
        if all == 0 { 0 } else { self.hits * 1000 / all }
    }

    fn reply_of(c: &Cached, hit: bool, now: Ns) -> Reply {
        let mut h = Headers::new();
        for (k, v) in &c.headers.0 {
            if !hop_by_hop(k) && !k.eq_ignore_ascii_case("x-cache") && !k.eq_ignore_ascii_case("age") {
                h.add(k, v);
            }
        }
        h.add("X-Cache", if hit { "HIT" } else { "MISS" });
        if hit {
            h.add("Age", &format!("{}", now.saturating_sub(c.stored) / SEC));
        }
        Reply { status: c.status, headers: h, body: c.body.clone(), chunk: 0, close: false }
    }

    fn send(&mut self, os: &mut OsApi, r: &ReqIn, get: bool) -> u64 {
        let base = self.backend.target.trim_end_matches('/');
        let target = format!("{}{}", base, r.head.target);
        let mut h = Headers::new();
        for (k, v) in &r.head.headers.0 {
            if !hop_by_hop(k) && !k.eq_ignore_ascii_case("host") {
                h.add(k, v);
            }
        }
        let method = if get { String::from("GET") } else { r.head.method.clone() };
        let req = Req { method, url: Url { host: self.backend.host.clone(), port: self.backend.port, target }, version: Version::Http11, headers: h, body: if get { Vec::new() } else { r.body.clone() }, timeout: self.timeout, connect_timeout: self.connect_timeout, reuse: true, keep_body: true };
        self.backend_requests += 1;
        self.agent.submit(os, req)
    }

    fn arrive(&mut self, os: &mut OsApi, r: ReqIn) {
        let now = os.now();
        self.requests += 1;
        let key = r.head.target.clone();
        let m = r.head.method.as_str();
        if m != "GET" && m != "HEAD" {
            let id = self.send(os, &r, false);
            self.fetch.insert(id, Fetch::Pass { key, w: Waiter { sid: r.id, at: r.at } });
            return;
        }
        let fresh = match self.store.get(&key) {
            Some(c) if c.expires > now => Some(c.clone()),
            Some(_) => {
                self.store.remove(&key);
                self.expired += 1;
                None
            }
            None => None,
        };
        if let Some(c) = fresh {
            self.hits += 1;
            os.note(Note::CacheHit { key });
            let rep = Cache::reply_of(&c, true, now);
            self.wheel.add(now + self.hit_time, (r.id, rep, r.at));
            return;
        }
        self.misses += 1;
        os.note(Note::CacheMiss { key: key.clone() });
        let w = Waiter { sid: r.id, at: r.at };
        if self.collapse
            && let Some(list) = self.waiting.get_mut(&key)
        {
            self.collapsed += 1;
            list.push(w);
            return;
        }
        let id = self.send(os, &r, true);
        if self.collapse {
            self.waiting.insert(key.clone(), Vec::new());
        }
        self.fetch.insert(id, Fetch::Miss { key, first: w });
    }

    fn respond(&mut self, os: &mut OsApi, w: &Waiter, rep: Reply, hit: Option<bool>) {
        let now = os.now();
        if self.serve.respond(os, w.sid, rep) {
            let l = now - w.at;
            self.lat.record(now, l);
            match hit {
                Some(true) => {
                    self.hit_lat.record(now, l);
                }
                Some(false) => {
                    self.miss_lat.record(now, l);
                }
                None => {}
            }
        }
    }

    fn fetched(&mut self, os: &mut OsApi, d: Done) {
        let now = os.now();
        let Some(f) = self.fetch.remove(&d.id) else {
            return;
        };
        match f {
            Fetch::Pass { key, w } => {
                let rep = match d.result {
                    Ok(resp) => {
                        if resp.head.status < 500 && self.store.remove(&key).is_some() {
                            self.invalidations += 1;
                        }
                        let c = Cached { status: resp.head.status, headers: resp.head.headers, body: resp.body, stored: now, expires: now };
                        let mut rep = Cache::reply_of(&c, false, now);
                        rep.headers.remove("X-Cache");
                        rep
                    }
                    Err(f) => {
                        self.backend_errors += 1;
                        Reply::text(if f.is_timeout() { 504 } else { 502 }, &format!("Backend nicht erreichbar: {}\n", f.text()))
                    }
                };
                self.respond(os, &w, rep, None);
            }
            Fetch::Miss { key, first } => {
                let rest = self.waiting.remove(&key).unwrap_or_default();
                match d.result {
                    Ok(resp) => {
                        let st = resp.head.status;
                        let age = if self.cache_control { max_age(&resp.head.headers) } else { None };
                        let keep = matches!(st, 200 | 203 | 204 | 301 | 404 | 410) && age != Some(0);
                        let c = Cached { status: st, headers: resp.head.headers, body: resp.body, stored: now, expires: now + age.unwrap_or(self.ttl) };
                        if keep && self.ttl > 0 {
                            let w = c.body.len() as u64 + 64;
                            self.evictions += self.store.insert(key, c.clone(), w).len() as u64;
                        }
                        for w in core::iter::once(&first).chain(rest.iter()) {
                            let rep = Cache::reply_of(&c, false, now);
                            self.respond(os, w, rep, Some(false));
                        }
                    }
                    Err(f) => {
                        self.backend_errors += 1;
                        for w in core::iter::once(&first).chain(rest.iter()) {
                            let rep = Reply::text(if f.is_timeout() { 504 } else { 502 }, &format!("Backend nicht erreichbar: {}\n", f.text()));
                            self.respond(os, w, rep, Some(false));
                        }
                    }
                }
            }
        }
    }

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, (sid, rep, at)) in self.wheel.due(now) {
            self.respond(os, &Waiter { sid, at }, rep, Some(true));
        }
        self.wheel.arm(os);
    }

    /// what the lab shows
    pub fn view(&self, now: Ns) -> CacheView {
        CacheView {
            port: self.port,
            backend: self.backend.text(),
            entries: self.store.len() as u64,
            max_entries: self.store.max_len as u64,
            bytes: self.store.weight(),
            max_bytes: self.store.max_weight,
            ttl: self.ttl,
            requests: self.requests,
            hits: self.hits,
            misses: self.misses,
            hit_pm: self.hit_pm(),
            collapsed: self.collapsed,
            expired: self.expired,
            evictions: self.evictions,
            invalidations: self.invalidations,
            backend_requests: self.backend_requests,
            backend_errors: self.backend_errors,
            p50: self.lat.p50(now),
            p99: self.lat.p99(now),
            hit_p50: self.hit_lat.p50(now),
            miss_p50: self.miss_lat.p50(now),
        }
    }
}

/// a cache as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheView {
    pub port: u16,
    pub backend: String,
    pub entries: u64,
    pub max_entries: u64,
    pub bytes: u64,
    pub max_bytes: u64,
    pub ttl: Ns,
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub hit_pm: u64,
    pub collapsed: u64,
    pub expired: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub backend_requests: u64,
    pub backend_errors: u64,
    pub p50: Option<Ns>,
    pub p99: Option<Ns>,
    pub hit_p50: Option<Ns>,
    pub miss_p50: Option<Ns>,
}

fn us(v: Option<u64>) -> i64 {
    v.map(|x| (x / 1000) as i64).unwrap_or(0)
}

impl Service for Cache {
    fn kind(&self) -> &'static str {
        "cache"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        self.serve.cfg.port = self.port;
        self.serve.stop();
        self.agent.reset();
        self.agent.idle_timeout = 30 * SEC;
        self.wheel.clear();
        self.fetch.clear();
        self.waiting.clear();
        // what the cache held is gone with its memory
        self.store.clear();
        self.serve.start(os, "cache");
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.timers(os);
            return;
        }
        if self.serve.on(&ev, os) {
            for r in self.serve.take() {
                self.arrive(os, r);
            }
        } else if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.fetched(os, d);
            }
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.serve.stop();
        self.agent.reset();
        self.wheel.clear();
        self.fetch.clear();
        self.waiting.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let v = |x: u64| x as i64;
        alloc::vec![
            Metric::new("requests", v(self.requests)),
            Metric::new("hits", v(self.hits)),
            Metric::new("misses", v(self.misses)),
            Metric::new("hit_pm", v(self.hit_pm())),
            Metric::new("collapsed", v(self.collapsed)),
            Metric::new("expired", v(self.expired)),
            Metric::new("evictions", v(self.evictions)),
            Metric::new("invalidations", v(self.invalidations)),
            Metric::new("entries", self.store.len() as i64),
            Metric::new("bytes", v(self.store.weight())),
            Metric::new("backend_requests", v(self.backend_requests)),
            Metric::new("backend_errors", v(self.backend_errors)),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
            Metric::new("hit_p50_us", us(self.hit_lat.p50(now))),
            Metric::new("miss_p50_us", us(self.miss_lat.p50(now))),
        ]
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LAT), LatWindow::new(&self.hit_lat, HIT_LAT), LatWindow::new(&self.miss_lat, MISS_LAT)]
    }
    fn status(&self) -> String {
        format!("Port {} vor {}: {} Einträge, {} Treffer, {} Fehlschläge ({},{} % Treffer)", self.port, self.backend.text(), self.store.len(), self.hits, self.misses, self.hit_pm() / 10, self.hit_pm() % 10)
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

/// a cache from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let b = c.get("backend").and_then(|v| v.as_str()).ok_or_else(|| msg!("\u{201e}backend\u{201c} fehlt."))?;
    let url = Url::parse(b).map_err(|_| msg!("\u{201e}{}\u{201c} ist keine http-URL.", b))?;
    let mut k = Cache::new(url);
    k.port = match num("port", 80) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    k.store.max_len = num("entries", 1000).min(10_000_000) as usize;
    k.store.max_weight = num("max_kb", 65_536).min(1 << 30) * 1024;
    k.ttl = ms("ttl_ms", 30_000);
    k.cache_control = c.get("cache_control").and_then(|v| v.as_bool()).unwrap_or(true);
    k.hit_time = c.get("hit_us").and_then(|v| decimal(v, US)).unwrap_or(50 * US);
    k.collapse = c.get("collapse").and_then(|v| v.as_bool()).unwrap_or(true);
    k.timeout = ms("timeout_ms", 5000);
    k.connect_timeout = ms("connect_timeout_ms", 1000).max(MS);
    Ok(Box::new(k))
}

impl Engine {
    /// a cache's view for the lab
    pub fn cache_view(&self, dev: u32, svc: &str) -> Option<CacheView> {
        Some(self.service::<Cache>(dev, svc)?.view(self.now))
    }
}
