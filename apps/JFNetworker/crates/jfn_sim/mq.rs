// mq -- a message queue and the workers that take from it: topics with at-least-once
// delivery (a message stays until it is acknowledged; one not acknowledged within its
// visibility timeout comes again), acknowledgements, redelivery, dead letters, depth
//
//   kind    config (JSON; every member may be left out but the worker's "queue")
//   mq      {"port": 9000, "visibility_ms": 5000, "max_deliveries": 5, "max_depth": 0,
//            "topics": ["jobs"], "durable": true}
//   worker  {"queue": "http://10.0.3.10:9000", "topic": "jobs", "concurrency": 4,
//            "time_ms": 50, "jitter_ms": 0, "error_rate": 0, "wait_ms": 1000, "idle_ms": 200,
//            "timeout_ms": 5000}
//
// HTTP of the queue (topics come into being with their first message):
//   POST /topics/T             the body is a message: 201 with X-Message-Id (after the disk
//                              when "durable"); 429 when max_depth messages wait
//   GET  /topics/T/next        the oldest waiting message: 200 with X-Message-Id and
//                              X-Delivery (its how-manieth delivery); 204 when there is none;
//                              ?wait_ms=N holds the request up to N ms until one comes
//   POST /topics/T/ack/ID      done with it: 204; 404 when it is not out (acknowledged, or
//                              back in the queue after its visibility timeout)
//   POST /topics/T/nack/ID     back into the queue at once: 204
//   GET  /topics/T             the numbers of the topic as text
//
// A message handed out is invisible for visibility_ms; then it goes back (redelivered) unless
// acknowledged, and after max_deliveries deliveries it is put aside as a dead letter.  The
// messages of a durable queue survive power off (as on a disk); what was out comes back.
//
// A worker runs `concurrency` slots: each asks for the next message (long poll), works on it
// time_ms (+ up to jitter_ms), then acknowledges it, or with the error rate gives it back
// (nack); a worker that stops or loses power leaves its messages to the visibility timeout.
//
// Metrics of the queue: published, delivered, redelivered, acked, nacked, dead, depth
// (waiting), inflight (out), topics, oldest_ms, max_depth, rejected, p50_us, p95_us, p99_us
// (publish to acknowledgement).  Of a worker: taken, done, failed, busy, polls, empty, errors,
// p50_us, p95_us, p99_us (taken to acknowledged).  Engine::mq_view / worker_view; mqctl and
// workerctl on the console.  The time series (series.rs) of both take p50_us, p95_us, p99_us
// of each interval from the windows.  The status lines of both are messages for the app's
// catalogue (status_msg; status() is their German text).

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use base::metrics::Window;
use base::rng::Pcg32;
use netpkt::http::{Headers, Version};

use crate::http::{Agent, Done, Reply, Req, ReqIn, Serve, ServeCfg, Url, Wheel};
use crate::httpsvc::{decimal, rate_ppm};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{LatWindow, LAT};
use crate::trace::Note;

const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
/// the worker's agent
const K_AGENT: u32 = 1;

/// a message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    pub id: u64,
    pub body: Vec<u8>,
    pub published: Ns,
    pub deliveries: u32,
}

/// a topic: what waits, what is out, what it counted
#[derive(Clone, Debug, Default)]
pub struct Topic {
    pub ready: BTreeMap<u64, Msg>,
    /// handed out: the message and its wheel entry
    pub out: BTreeMap<u64, (Msg, u64)>,
    pub published: u64,
    pub delivered: u64,
    pub redelivered: u64,
    pub acked: u64,
    pub nacked: u64,
    pub dead: u64,
    pub rejected: u64,
    pub max_depth: u64,
    /// the last dead letters
    pub dead_letters: VecDeque<Msg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum QTimer {
    /// a message's visibility timeout
    Visible(String, u64),
    /// a long poll ends empty
    Poll(u64),
    /// a durable message is on the disk: (topic, message, request)
    Stored(String, u64, u64),
}

/// the queue
#[derive(Clone, Debug)]
pub struct Queue {
    pub port: u16,
    pub visibility: Ns,
    pub max_deliveries: u32,
    /// messages waiting in a topic at most (0: no limit)
    pub max_depth: u64,
    pub durable: bool,
    pub topics: BTreeMap<String, Topic>,
    serve: Serve,
    wheel: Wheel<QTimer>,
    /// long polls: topic -> (request, deadline entry)
    polls: BTreeMap<String, VecDeque<(u64, u64)>>,
    /// durable messages on their way to the disk
    storing: BTreeMap<u64, Msg>,
    next: u64,
    lat: Window,
    seen: Ns,
}

impl Queue {
    pub fn new() -> Queue {
        let cfg = ServeCfg { port: 9000, server: String::from("jfn-mq/1.0"), max_requests: 0, ..ServeCfg::default() };
        Queue {
            port: 9000,
            visibility: 5 * SEC,
            max_deliveries: 5,
            max_depth: 0,
            durable: true,
            topics: BTreeMap::new(),
            serve: Serve::new(cfg, K_SERVE),
            wheel: Wheel::new(K_OWN),
            polls: BTreeMap::new(),
            storing: BTreeMap::new(),
            next: 0,
            lat: Window::new(300, SEC),
            seen: 0,
        }
    }
    /// messages waiting in every topic
    pub fn depth(&self) -> u64 {
        self.topics.values().map(|t| t.ready.len() as u64).sum()
    }
    /// messages out in every topic
    pub fn inflight(&self) -> u64 {
        self.topics.values().map(|t| t.out.len() as u64).sum()
    }
    fn sum(&self, f: impl Fn(&Topic) -> u64) -> u64 {
        self.topics.values().map(f).sum()
    }

    fn text(&mut self, os: &mut OsApi, sid: u64, status: u16, text: &str) {
        let _ = self.serve.respond(os, sid, Reply::text(status, text));
    }

    fn request(&mut self, os: &mut OsApi, r: ReqIn) {
        let path = String::from(r.head.path());
        let Some(rest) = path.strip_prefix("/topics/") else {
            self.text(os, r.id, 404, "404 Not Found\n");
            return;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        let m = r.head.method.as_str();
        match (m, parts.as_slice()) {
            ("POST", [t]) | ("PUT", [t]) if !t.is_empty() => self.publish(os, r.id, t, r.body.clone()),
            ("GET", [t, "next"]) => {
                let wait = crate::kv::query_value(&r.head.target, "wait_ms").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                self.next_msg(os, r.id, t, wait * MS);
            }
            ("POST", [t, "ack", id]) => {
                let id = id.parse::<u64>().unwrap_or(0);
                self.ack(os, r.id, t, id);
            }
            ("POST", [t, "nack", id]) => {
                let id = id.parse::<u64>().unwrap_or(0);
                self.nack(os, r.id, t, id);
            }
            ("GET", [t]) => {
                let text = match self.topics.get(*t) {
                    Some(tp) => format!("Topic {}: {} wartend, {} unterwegs, {} angenommen, {} ausgeliefert, {} bestätigt, {} erneut, {} aussortiert\n", t, tp.ready.len(), tp.out.len(), tp.published, tp.delivered, tp.acked, tp.redelivered, tp.dead),
                    None => format!("Topic {}: noch keine Nachricht\n", t),
                };
                self.text(os, r.id, 200, &text);
            }
            _ => self.text(os, r.id, 404, "404 Not Found\n"),
        }
    }

    fn publish(&mut self, os: &mut OsApi, sid: u64, topic: &str, body: Vec<u8>) {
        let now = os.now();
        let tp = self.topics.entry(String::from(topic)).or_default();
        if self.max_depth > 0 && tp.ready.len() as u64 >= self.max_depth {
            tp.rejected += 1;
            let mut rep = Reply::text(429, "Die Queue ist voll.\n");
            rep.headers.add("Retry-After", "1");
            let _ = self.serve.respond(os, sid, rep);
            return;
        }
        self.next += 1;
        let msg = Msg { id: self.next, body, published: now, deliveries: 0 };
        if self.durable {
            let len = msg.body.len() as u64 + 32;
            match os.disk_write(len, len as i64) {
                Some(at) => {
                    self.wheel.add(at, QTimer::Stored(String::from(topic), msg.id, sid));
                    self.storing.insert(msg.id, msg);
                }
                None => {
                    os.note(Note::DiskFull);
                    self.text(os, sid, 507, "Disk voll\n");
                }
            }
            return;
        }
        self.accept(os, topic, msg, Some(sid));
    }

    /// a message is in its topic: the answer to its publisher, a long poll gets it
    fn accept(&mut self, os: &mut OsApi, topic: &str, msg: Msg, sid: Option<u64>) {
        let id = msg.id;
        let tp = self.topics.entry(String::from(topic)).or_default();
        tp.published += 1;
        tp.ready.insert(id, msg);
        tp.max_depth = tp.max_depth.max(tp.ready.len() as u64);
        os.note(Note::MqPublish { topic: String::from(topic), id });
        if let Some(sid) = sid {
            let mut rep = Reply::text(201, &format!("{}\n", id));
            rep.headers.add("X-Message-Id", &format!("{}", id));
            let _ = self.serve.respond(os, sid, rep);
        }
        self.feed(os, topic);
    }

    /// waiting long polls of a topic get its waiting messages
    fn feed(&mut self, os: &mut OsApi, topic: &str) {
        loop {
            let ready = self.topics.get(topic).is_some_and(|t| !t.ready.is_empty());
            if !ready {
                return;
            }
            let Some((sid, w)) = self.polls.get_mut(topic).and_then(|q| q.pop_front()) else {
                return;
            };
            self.wheel.cancel(w);
            self.deliver(os, sid, topic);
        }
    }

    /// hands out the oldest waiting message of a topic
    fn deliver(&mut self, os: &mut OsApi, sid: u64, topic: &str) {
        let now = os.now();
        let vis = self.visibility;
        let Some(tp) = self.topics.get_mut(topic) else {
            return;
        };
        let Some((&id, _)) = tp.ready.iter().next() else {
            return;
        };
        let Some(mut msg) = tp.ready.remove(&id) else {
            return;
        };
        msg.deliveries += 1;
        tp.delivered += 1;
        if msg.deliveries > 1 {
            tp.redelivered += 1;
        }
        let n = msg.deliveries;
        let w = self.wheel.add(now + vis, QTimer::Visible(String::from(topic), id));
        let body = msg.body.clone();
        tp.out.insert(id, (msg, w));
        os.note(Note::MqDeliver { topic: String::from(topic), id, n });
        let mut h = Headers::new();
        h.add("Content-Type", "application/octet-stream");
        h.add("X-Message-Id", &format!("{}", id));
        h.add("X-Delivery", &format!("{}", n));
        let _ = self.serve.respond(os, sid, Reply { status: 200, headers: h, body, chunk: 0, close: false });
    }

    fn next_msg(&mut self, os: &mut OsApi, sid: u64, topic: &str, wait: Ns) {
        let now = os.now();
        if self.topics.get(topic).is_some_and(|t| !t.ready.is_empty()) {
            self.deliver(os, sid, topic);
            return;
        }
        if wait == 0 {
            let _ = self.serve.respond(os, sid, Reply { status: 204, ..Reply::default() });
            return;
        }
        let w = self.wheel.add(now + wait, QTimer::Poll(sid));
        self.polls.entry(String::from(topic)).or_default().push_back((sid, w));
    }

    fn ack(&mut self, os: &mut OsApi, sid: u64, topic: &str, id: u64) {
        let now = os.now();
        let Some((msg, w)) = self.topics.get_mut(topic).and_then(|t| t.out.remove(&id)) else {
            self.text(os, sid, 404, "Diese Nachricht ist nicht unterwegs.\n");
            return;
        };
        self.wheel.cancel(w);
        if let Some(t) = self.topics.get_mut(topic) {
            t.acked += 1;
        }
        if self.durable {
            os.disk_free(msg.body.len() as u64 + 32);
        }
        self.lat.record(now, now - msg.published);
        os.note(Note::MqAck { topic: String::from(topic), id });
        let _ = self.serve.respond(os, sid, Reply { status: 204, ..Reply::default() });
    }

    fn nack(&mut self, os: &mut OsApi, sid: u64, topic: &str, id: u64) {
        let Some((msg, w)) = self.topics.get_mut(topic).and_then(|t| t.out.remove(&id)) else {
            self.text(os, sid, 404, "Diese Nachricht ist nicht unterwegs.\n");
            return;
        };
        self.wheel.cancel(w);
        if let Some(t) = self.topics.get_mut(topic) {
            t.nacked += 1;
        }
        self.back(os, topic, msg);
        let _ = self.serve.respond(os, sid, Reply { status: 204, ..Reply::default() });
    }

    /// a message that was out comes back (or is put aside after too many deliveries)
    fn back(&mut self, os: &mut OsApi, topic: &str, msg: Msg) {
        let id = msg.id;
        let durable = self.durable;
        let max = self.max_deliveries;
        let Some(t) = self.topics.get_mut(topic) else {
            return;
        };
        if max > 0 && msg.deliveries >= max {
            t.dead += 1;
            if durable {
                os.disk_free(msg.body.len() as u64 + 32);
            }
            t.dead_letters.push_back(msg);
            if t.dead_letters.len() > 100 {
                t.dead_letters.pop_front();
            }
            os.note(Note::MqDead { topic: String::from(topic), id });
            os.log(4, "mq", &format!("Nachricht {} in {} nach {} Zustellungen aussortiert", id, topic, max));
            return;
        }
        t.ready.insert(id, msg);
        t.max_depth = t.max_depth.max(t.ready.len() as u64);
        os.note(Note::MqRedeliver { topic: String::from(topic), id });
        self.feed(os, topic);
    }

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                QTimer::Visible(topic, id) => {
                    if let Some((msg, _)) = self.topics.get_mut(&topic).and_then(|t| t.out.remove(&id)) {
                        self.back(os, &topic, msg);
                    }
                }
                QTimer::Poll(sid) => {
                    for q in self.polls.values_mut() {
                        q.retain(|p| p.0 != sid);
                    }
                    let _ = self.serve.respond(os, sid, Reply { status: 204, ..Reply::default() });
                }
                QTimer::Stored(topic, id, sid) => {
                    if let Some(msg) = self.storing.remove(&id) {
                        self.accept(os, &topic, msg, Some(sid));
                    }
                }
            }
        }
        self.wheel.arm(os);
    }

    /// what the lab shows
    pub fn view(&self, now: Ns) -> QueueView {
        QueueView {
            port: self.port,
            visibility: self.visibility,
            max_deliveries: self.max_deliveries,
            durable: self.durable,
            topics: self
                .topics
                .iter()
                .map(|(n, t)| TopicView {
                    name: n.clone(),
                    ready: t.ready.len() as u64,
                    inflight: t.out.len() as u64,
                    published: t.published,
                    delivered: t.delivered,
                    redelivered: t.redelivered,
                    acked: t.acked,
                    nacked: t.nacked,
                    dead: t.dead,
                    rejected: t.rejected,
                    max_depth: t.max_depth,
                    oldest: t.ready.values().map(|m| now.saturating_sub(m.published)).max(),
                })
                .collect(),
            p50: self.lat.p50(now),
            p99: self.lat.p99(now),
        }
    }
}

impl Default for Queue {
    fn default() -> Queue {
        Queue::new()
    }
}

/// a topic as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicView {
    pub name: String,
    pub ready: u64,
    pub inflight: u64,
    pub published: u64,
    pub delivered: u64,
    pub redelivered: u64,
    pub acked: u64,
    pub nacked: u64,
    pub dead: u64,
    pub rejected: u64,
    pub max_depth: u64,
    /// the age of the oldest waiting message
    pub oldest: Option<Ns>,
}

/// a queue as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueView {
    pub port: u16,
    pub visibility: Ns,
    pub max_deliveries: u32,
    pub durable: bool,
    pub topics: Vec<TopicView>,
    /// publish to acknowledgement
    pub p50: Option<Ns>,
    pub p99: Option<Ns>,
}

fn us(v: Option<u64>) -> i64 {
    v.map(|x| (x / 1000) as i64).unwrap_or(0)
}

impl Service for Queue {
    fn kind(&self) -> &'static str {
        "mq"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        self.serve.cfg.port = self.port;
        self.serve.stop();
        self.wheel.clear();
        self.polls.clear();
        self.storing.clear();
        // what was out when it stopped comes again (at least once)
        for t in self.topics.values_mut() {
            let out = core::mem::take(&mut t.out);
            for (id, (m, _)) in out {
                t.ready.insert(id, m);
            }
        }
        if !self.durable {
            self.topics.clear();
        }
        self.serve.start(os, "mq");
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.timers(os);
            return;
        }
        if self.serve.on(&ev, os) {
            for r in self.serve.take() {
                self.request(os, r);
            }
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        // messages that had not reached the disk are lost (their publishers got no answer)
        for (_, m) in core::mem::take(&mut self.storing) {
            os.disk_free(m.body.len() as u64 + 32);
        }
        self.serve.stop();
        self.wheel.clear();
        self.polls.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let v = |x: u64| x as i64;
        let oldest = self.topics.values().flat_map(|t| t.ready.values()).map(|m| now.saturating_sub(m.published)).max().unwrap_or(0);
        alloc::vec![
            Metric::new("published", v(self.sum(|t| t.published))),
            Metric::new("delivered", v(self.sum(|t| t.delivered))),
            Metric::new("redelivered", v(self.sum(|t| t.redelivered))),
            Metric::new("acked", v(self.sum(|t| t.acked))),
            Metric::new("nacked", v(self.sum(|t| t.nacked))),
            Metric::new("dead", v(self.sum(|t| t.dead))),
            Metric::new("depth", v(self.depth())),
            Metric::new("inflight", v(self.inflight())),
            Metric::new("topics", self.topics.len() as i64),
            Metric::new("oldest_ms", v(oldest / MS)),
            Metric::new("max_depth", v(self.topics.values().map(|t| t.max_depth).max().unwrap_or(0))),
            Metric::new("rejected", v(self.sum(|t| t.rejected))),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
        ]
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LAT)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> jfn_model::msg::Msg {
        msg!("Port {}: {} Topics, {} wartend, {} unterwegs, {} bestätigt", self.port, self.topics.len(), self.depth(), self.inflight(), self.sum(|t| t.acked))
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

// ---------------------------------------------------------------- the worker

/// what a worker slot does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// pauses before it asks again
    Idle,
    /// asks for a message (long poll)
    Asking,
    /// works on a message until then
    Working { id: u64, until: Ns },
    /// acknowledges (or gives back) a message
    Acking { id: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WTimer {
    /// a slot asks again
    Ask(usize),
    /// a slot's work is done
    Done(usize),
}

/// a worker that takes messages from a queue
#[derive(Clone, Debug)]
pub struct Worker {
    pub queue: Url,
    pub topic: String,
    pub concurrency: u32,
    pub time: Ns,
    pub jitter: Ns,
    pub error_ppm: u32,
    pub wait: Ns,
    pub idle: Ns,
    pub timeout: Ns,
    pub slots: Vec<SlotState>,
    /// when each slot took its message
    taken_at: Vec<Ns>,
    agent: Agent,
    wheel: Wheel<WTimer>,
    /// agent request -> slot
    asks: BTreeMap<u64, usize>,
    rng: Pcg32,
    pub taken: u64,
    pub done: u64,
    pub failed: u64,
    pub polls: u64,
    pub empty: u64,
    pub errors: u64,
    lat: Window,
    seen: Ns,
}

impl Worker {
    pub fn new(queue: Url, topic: &str) -> Worker {
        Worker {
            queue,
            topic: String::from(topic),
            concurrency: 1,
            time: 50 * MS,
            jitter: 0,
            error_ppm: 0,
            wait: SEC,
            idle: 200 * MS,
            timeout: 5 * SEC,
            slots: Vec::new(),
            taken_at: Vec::new(),
            agent: Agent::new(K_AGENT),
            wheel: Wheel::new(K_OWN),
            asks: BTreeMap::new(),
            rng: Pcg32::new(0, 0),
            taken: 0,
            done: 0,
            failed: 0,
            polls: 0,
            empty: 0,
            errors: 0,
            lat: Window::new(300, SEC),
            seen: 0,
        }
    }
    /// slots working on a message now
    pub fn busy(&self) -> u32 {
        self.slots.iter().filter(|s| matches!(s, SlotState::Working { .. } | SlotState::Acking { .. })).count() as u32
    }

    fn req(&self, method: &str, target: String) -> Req {
        let base = self.queue.target.trim_end_matches('/');
        let mut h = Headers::new();
        h.add("User-Agent", "jfn-worker/1.0");
        let timeout = if method == "GET" { self.wait + self.timeout } else { self.timeout };
        Req { method: String::from(method), url: Url { host: self.queue.host.clone(), port: self.queue.port, target: format!("{}{}", base, target) }, version: Version::Http11, headers: h, body: Vec::new(), timeout, connect_timeout: SEC, reuse: true, keep_body: true }
    }

    fn ask(&mut self, os: &mut OsApi, slot: usize) {
        if slot >= self.concurrency as usize || slot >= self.slots.len() {
            return;
        }
        self.polls += 1;
        let req = self.req("GET", format!("/topics/{}/next?wait_ms={}", self.topic, self.wait / MS));
        let id = self.agent.submit(os, req);
        self.asks.insert(id, slot);
        self.slots[slot] = SlotState::Asking;
    }

    /// a slot pauses, then asks again
    fn pause(&mut self, os: &mut OsApi, slot: usize, d: Ns) {
        let now = os.now();
        if let Some(s) = self.slots.get_mut(slot) {
            *s = SlotState::Idle;
        }
        self.wheel.add(now + d, WTimer::Ask(slot));
    }

    fn answered(&mut self, os: &mut OsApi, d: Done) {
        let now = os.now();
        let Some(slot) = self.asks.remove(&d.id) else {
            return;
        };
        let state = self.slots.get(slot).copied().unwrap_or(SlotState::Idle);
        match state {
            SlotState::Asking => match d.result {
                Ok(r) if r.head.status == 200 => {
                    let id = r.head.header("x-message-id").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                    let j = if self.jitter > 0 { self.rng.range(0, self.jitter as i64) as Ns } else { 0 };
                    let until = now + self.time + j;
                    self.taken += 1;
                    if let Some(t) = self.taken_at.get_mut(slot) {
                        *t = now;
                    }
                    self.slots[slot] = SlotState::Working { id, until };
                    self.wheel.add(until, WTimer::Done(slot));
                }
                Ok(r) if r.head.status == 204 => {
                    self.empty += 1;
                    if self.wait == 0 {
                        self.pause(os, slot, self.idle);
                    } else {
                        self.ask(os, slot);
                    }
                }
                _ => {
                    self.errors += 1;
                    self.pause(os, slot, self.idle.max(MS));
                }
            },
            SlotState::Acking { .. } => {
                let ok = d.result.as_ref().is_ok_and(|r| r.head.status == 204);
                if !ok {
                    self.errors += 1;
                }
                self.ask(os, slot);
            }
            _ => {}
        }
    }

    /// a slot's work on its message is done: acknowledge it, or give it back
    fn work_done(&mut self, os: &mut OsApi, slot: usize) {
        let now = os.now();
        let Some(SlotState::Working { id, .. }) = self.slots.get(slot).copied() else {
            return;
        };
        let fail = self.error_ppm > 0 && self.rng.chance_ppm(self.error_ppm);
        let what = if fail {
            self.failed += 1;
            "nack"
        } else {
            self.done += 1;
            if let Some(t) = self.taken_at.get(slot) {
                self.lat.record(now, now - *t);
            }
            "ack"
        };
        let req = self.req("POST", format!("/topics/{}/{}/{}", self.topic, what, id));
        let rid = self.agent.submit(os, req);
        self.asks.insert(rid, slot);
        self.slots[slot] = SlotState::Acking { id };
    }

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                WTimer::Ask(s) => {
                    if self.slots.get(s) == Some(&SlotState::Idle) {
                        self.ask(os, s);
                    }
                }
                WTimer::Done(s) => self.work_done(os, s),
            }
        }
        self.wheel.arm(os);
    }

    /// changes the worker while it runs (Action::WorkerSet): more slots start at once, fewer
    /// stop after their message
    pub fn set(&mut self, os: &mut OsApi, concurrency: Option<u32>, time: Option<Ns>, error_ppm: Option<u32>) {
        if let Some(t) = time {
            self.time = t;
        }
        if let Some(e) = error_ppm {
            self.error_ppm = e.min(1_000_000);
        }
        if let Some(c) = concurrency {
            let old = self.concurrency as usize;
            self.concurrency = c.min(10_000);
            while self.slots.len() < self.concurrency as usize {
                self.slots.push(SlotState::Idle);
                self.taken_at.push(0);
            }
            for s in old..self.concurrency as usize {
                if self.slots[s] == SlotState::Idle {
                    self.ask(os, s);
                }
            }
        }
        self.wheel.arm(os);
    }

    /// what the lab shows
    pub fn view(&self, now: Ns) -> WorkerView {
        WorkerView {
            queue: self.queue.text(),
            topic: self.topic.clone(),
            concurrency: self.concurrency,
            slots: self.slots.iter().take(self.concurrency as usize).copied().collect(),
            taken: self.taken,
            done: self.done,
            failed: self.failed,
            polls: self.polls,
            empty: self.empty,
            errors: self.errors,
            p50: self.lat.p50(now),
            p99: self.lat.p99(now),
        }
    }
}

/// a worker as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerView {
    pub queue: String,
    pub topic: String,
    pub concurrency: u32,
    pub slots: Vec<SlotState>,
    pub taken: u64,
    pub done: u64,
    pub failed: u64,
    pub polls: u64,
    pub empty: u64,
    pub errors: u64,
    /// taken to acknowledged
    pub p50: Option<Ns>,
    pub p99: Option<Ns>,
}

impl Service for Worker {
    fn kind(&self) -> &'static str {
        "worker"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        let a = os.rand() as u64;
        let b = os.rand() as u64;
        self.rng = Pcg32::new((a << 32) | b, os.svc_id() as u64);
        self.agent.reset();
        self.agent.idle_timeout = 30 * SEC;
        self.wheel.clear();
        self.asks.clear();
        self.slots = alloc::vec![SlotState::Idle; self.concurrency as usize];
        self.taken_at = alloc::vec![0; self.concurrency as usize];
        for s in 0..self.concurrency as usize {
            self.ask(os, s);
        }
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
                self.answered(os, d);
            }
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        // the messages it worked on stay unacknowledged: the queue hands them out again
        self.agent.reset();
        self.wheel.clear();
        self.asks.clear();
        for s in self.slots.iter_mut() {
            *s = SlotState::Idle;
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let v = |x: u64| x as i64;
        alloc::vec![
            Metric::new("taken", v(self.taken)),
            Metric::new("done", v(self.done)),
            Metric::new("failed", v(self.failed)),
            Metric::new("busy", self.busy() as i64),
            Metric::new("polls", v(self.polls)),
            Metric::new("empty", v(self.empty)),
            Metric::new("errors", v(self.errors)),
            Metric::new("p50_us", us(self.lat.p50(now))),
            Metric::new("p95_us", us(self.lat.p95(now))),
            Metric::new("p99_us", us(self.lat.p99(now))),
        ]
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LAT)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> jfn_model::msg::Msg {
        msg!("{} {}: {} von {} beschäftigt, {} erledigt, {} gescheitert", self.queue.text(), self.topic, self.busy(), self.concurrency, self.done, self.failed)
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

/// a queue or a worker from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(kind: &str, c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(kind, c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(kind: &str, c: &Value) -> Result<Box<dyn Service>, jfn_model::msg::Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    match kind {
        "mq" => {
            let mut q = Queue::new();
            q.port = match num("port", 9000) {
                p @ 1..=65535 => p as u16,
                p => return Err(msg!("Port {} gibt es nicht.", p)),
            };
            q.visibility = ms("visibility_ms", 5000).max(MS);
            q.max_deliveries = num("max_deliveries", 5).min(1_000_000) as u32;
            q.max_depth = num("max_depth", 0);
            q.durable = c.get("durable").and_then(|v| v.as_bool()).unwrap_or(true);
            for t in c.get("topics").and_then(|t| t.as_array()).unwrap_or(&[]) {
                if let Some(n) = t.as_str() {
                    q.topics.entry(String::from(n)).or_default();
                }
            }
            Ok(Box::new(q))
        }
        "worker" => {
            let u = c.get("queue").and_then(|v| v.as_str()).ok_or_else(|| msg!("\u{201e}queue\u{201c} fehlt."))?;
            let url = Url::parse(u).map_err(|_| msg!("\u{201e}{}\u{201c} ist keine http-URL.", u))?;
            let mut w = Worker::new(url, c.get("topic").and_then(|v| v.as_str()).unwrap_or("jobs"));
            w.concurrency = num("concurrency", 1).clamp(1, 10_000) as u32;
            w.time = ms("time_ms", 50);
            w.jitter = ms("jitter_ms", 0);
            w.error_ppm = c.get("error_rate").and_then(rate_ppm).unwrap_or(0);
            w.wait = ms("wait_ms", 1000);
            w.idle = ms("idle_ms", 200);
            w.timeout = ms("timeout_ms", 5000).max(MS);
            Ok(Box::new(w))
        }
        k => Err(msg!("Unbekannte Dienstart \u{201e}{}\u{201c}.", k)),
    }
}

impl Engine {
    /// a queue's view for the lab
    pub fn mq_view(&self, dev: u32, svc: &str) -> Option<QueueView> {
        Some(self.service::<Queue>(dev, svc)?.view(self.now))
    }
    /// a worker's view for the lab
    pub fn worker_view(&self, dev: u32, svc: &str) -> Option<WorkerView> {
        Some(self.service::<Worker>(dev, svc)?.view(self.now))
    }
}
