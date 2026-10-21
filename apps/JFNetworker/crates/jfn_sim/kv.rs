// kv -- the key-value store of the lab: nodes on a hash ring with virtual nodes, a replication
// factor, a leader per key, read and write quorums in two consistency modes, read repair,
// heartbeats between the nodes and a disk with latency and capacity; KvClient lets other
// services (web servers) use the store: it knows the ring and fails over to the next replica
//
//   kind  config (JSON; every member may be left out but "nodes")
//   kv    {"port": 7000, "nodes": ["10.0.2.21", "10.0.2.22", "10.0.2.23"], "vnodes": 16,
//          "rf": 2, "consistency": "strong", "w": 0, "r": 0, "read_repair": true,
//          "timeout_ms": 500, "heartbeat_ms": 500, "down_ms": 1500, "hb_port": 7946,
//          "disk": {"latency_us": 100, "mbps": 500, "iops": 50000, "capacity_mb": 1024}}
//
// The ring: every node puts "vnodes" tokens on it (made from its address); the replicas of a
// key are the first "rf" distinct nodes from the hash of the key on (its preference list),
// and its leader is the first of them this node believes alive.  Every node coordinates what
// a client asks: a write goes to the leader (forwarded with X-Kv-Forwarded unless this node
// leads it), which gives it the next version of its Lamport counter (n.node), stores it and
// sends it to the other live replicas; the write succeeds when W replicas stored it.  A read
// asks every live replica and answers with the newest version once R of them replied; with
// read repair the replicas that had an older version (or none) get the newest one afterwards.
// "strong" uses W and R (0: a majority of rf each, so that W + R > rf), "eventual" W = R = 1,
// and a request may say ?consistency=, ?w=, ?r=.  Too few live replicas: 503 at once; a
// quorum not reached within timeout_ms: 503; the disk full: 507.  Versions decide (the
// newer wins), a deletion is a version without value (a tombstone).  When the leader does not
// answer a forwarded write, a replica leads it itself; a node that does not answer a request
// counts as down until its next heartbeat.
//
// Heartbeats: every node sends a datagram ("kvhb N") to every other one each heartbeat_ms on
// UDP hb_port; a node it has not heard from for down_ms counts as down until it is heard
// again.  The data stays on the disk of the device over power off; writes the disk had not
// finished are lost.
//
//   HTTP   GET /kv/KEY (200 with X-Version, 404), PUT|POST /kv/KEY (201), DELETE /kv/KEY (204);
//          between the nodes GET /_kv/KEY and PUT /_kv/KEY with X-Version (X-Deleted: 1 for a
//          tombstone, X-Kv-Repair: 1 for read repair; 204 with X-Stored: 1 when it was newer)
//
// Metrics: requests, reads, writes, ok, not_found, unavailable, full, forwards, led,
// replica_writes, replica_reads, stale_reads (a read answered an older version than another
// replica had), read_repairs (sent), repaired (received), keys, bytes, members_up,
// member_downs, p50_us, p95_us, p99_us (client requests at this node; the time series take them
// per interval from the window).
// Views: Engine::kv_view, kv_ring, kv_where (the replicas of a key and what each holds).  The
// status line is a message for the app's catalogue (status_msg; status() is its German text).

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
use crate::trace::Note;

/// timer keys: its own deadlines (heartbeats, disk, timeouts), the server side, the agent
const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
const K_AGENT: u32 = 2;

/// how strictly reads and writes agree
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consistency {
    /// W and R replicas (a majority each by default)
    Strong,
    /// one replica each
    Eventual,
}

impl Consistency {
    pub fn name(self) -> &'static str {
        match self {
            Consistency::Strong => "strong",
            Consistency::Eventual => "eventual",
        }
    }
    pub fn parse(s: &str) -> Option<Consistency> {
        match s {
            "strong" | "quorum" => Some(Consistency::Strong),
            "eventual" | "one" => Some(Consistency::Eventual),
            _ => None,
        }
    }
}

/// a version: the leader's counter and its member number (1, 2, ... in address order)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ver {
    pub n: u64,
    pub node: u16,
}

impl Ver {
    pub fn text(self) -> String {
        format!("{}.{}", self.n, self.node)
    }
    pub fn parse(s: &str) -> Option<Ver> {
        let (a, b) = s.trim().split_once('.')?;
        Some(Ver { n: a.parse().ok()?, node: b.parse().ok()? })
    }
}

/// a stored value
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub value: Vec<u8>,
    pub ver: Ver,
    /// a deletion (no value)
    pub deleted: bool,
}

/// the bytes an entry takes on the disk
fn size_of(key: &str, e: &Entry) -> u64 {
    (key.len() + e.value.len() + 16) as u64
}

/// a key in a path: unusual bytes as %XX
pub fn enc(key: &str) -> String {
    let mut s = String::new();
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:@!$'()*+,;=".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{:02X}", b));
        }
    }
    s
}

/// a key from a path (%XX decoded)
pub fn dec(path: &str) -> String {
    let b = path.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// the value of a query parameter of a request target
pub fn query_value<'a>(target: &'a str, name: &str) -> Option<&'a str> {
    let q = target.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k == name { Some(v) } else { None }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KTimer {
    /// send heartbeats and look who went quiet
    Beat,
    /// a disk access is done
    Io(u64),
    /// a coordinated request runs out of time
    Timeout(u64),
}

/// a request this node coordinates or a part of one
#[derive(Clone, Debug)]
enum Op {
    Read { sid: u64, key: String, need: u8, targets: Vec<Ipv4>, got: Vec<(Ipv4, Option<Entry>)>, failed: Vec<Ipv4>, answered: Option<Ver>, timer: u64, start: Ns },
    Write { sid: u64, key: String, ver: Ver, deleted: bool, need: u8, targets: usize, acks: u8, failed: u8, full: bool, answered: bool, timer: u64, start: Ns },
    /// a client's write on its way to the leader (what it needs to lead it here when the
    /// leader does not answer)
    Forward { sid: u64, key: String, deleted: bool, need: u8, body: Vec<u8>, start: Ns },
}

/// what a local disk access is for
#[derive(Clone, Debug)]
enum IoFor {
    /// this node's replica in a read it coordinates
    Read(u64),
    /// the leader's own copy of a write
    Write(u64),
    /// another node asked for the entry (GET /_kv)
    ReplicaRead(u64),
    /// another node sent a version (PUT /_kv); repair: it came from read repair
    ReplicaWrite { sid: u64, repair: bool },
    /// read repair of this node's own copy
    Repair,
}

#[derive(Clone, Debug)]
struct Io {
    why: IoFor,
    key: String,
    /// a write: the entry and the bytes reserved for it
    entry: Option<Entry>,
    grow: i64,
}

/// what a KV node counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KvStats {
    pub requests: u64,
    pub reads: u64,
    pub writes: u64,
    pub ok: u64,
    pub not_found: u64,
    pub unavailable: u64,
    pub full: u64,
    pub forwards: u64,
    pub led: u64,
    pub replica_writes: u64,
    pub replica_reads: u64,
    pub stale_reads: u64,
    pub read_repairs: u64,
    pub repaired: u64,
    pub member_downs: u64,
}

/// a member as this node sees it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub addr: Ipv4,
    pub up: bool,
    /// when it was last heard (or this node started)
    pub heard: Ns,
    /// when its state last changed
    pub since: Ns,
}

/// a node of the store
#[derive(Clone, Debug)]
pub struct KvNode {
    pub port: u16,
    pub hb_port: u16,
    /// the members, sorted
    pub nodes: Vec<Ipv4>,
    pub vnodes: u32,
    pub rf: usize,
    /// the quorums of "strong" (0: a majority)
    pub w: u8,
    pub r: u8,
    pub consistency: Consistency,
    pub read_repair: bool,
    pub timeout: Ns,
    pub beat: Ns,
    pub down_after: Ns,
    disk_cfg: Option<(Ns, u64, u64, u64)>,
    ring: Ring<Ipv4>,
    me: Option<Ipv4>,
    my_no: u16,
    clock: u64,
    data: BTreeMap<String, Entry>,
    members: BTreeMap<Ipv4, Member>,
    serve: Serve,
    agent: Agent,
    wheel: Wheel<KTimer>,
    ops: BTreeMap<u64, Op>,
    ios: BTreeMap<u64, Io>,
    next: u64,
    /// agent request -> (op, replica); op 0: nothing waits for it (read repair)
    by_agent: BTreeMap<u64, (u64, Ipv4)>,
    beats: u64,
    lat: Window,
    pub stats: KvStats,
    seen: Ns,
}

/// the default port of the store's HTTP side
pub const KV_PORT: u16 = 7000;
/// the default port of the heartbeats
pub const HB_PORT: u16 = 7946;

impl KvNode {
    pub fn new(nodes: Vec<Ipv4>) -> KvNode {
        let mut n = nodes;
        n.sort();
        n.dedup();
        let cfg = ServeCfg { port: KV_PORT, server: String::from("jfn-kv/1.0"), max_requests: 0, max_body: 16 << 20, ..ServeCfg::default() };
        let mut k = KvNode {
            port: KV_PORT,
            hb_port: HB_PORT,
            nodes: n,
            vnodes: 16,
            rf: 2,
            w: 0,
            r: 0,
            consistency: Consistency::Strong,
            read_repair: true,
            timeout: 500 * MS,
            beat: 500 * MS,
            down_after: 1500 * MS,
            disk_cfg: None,
            ring: Ring::new(),
            me: None,
            my_no: 0,
            clock: 0,
            data: BTreeMap::new(),
            members: BTreeMap::new(),
            serve: Serve::new(cfg, K_SERVE),
            agent: Agent::new(K_AGENT),
            wheel: Wheel::new(K_OWN),
            ops: BTreeMap::new(),
            ios: BTreeMap::new(),
            next: 0,
            by_agent: BTreeMap::new(),
            beats: 0,
            lat: Window::new(300, SEC),
            stats: KvStats::default(),
            seen: 0,
        };
        k.rering();
        k
    }

    fn rering(&mut self) {
        let mut r = Ring::new();
        for a in &self.nodes {
            r.add(*a, &format!("{}", a), self.vnodes);
        }
        self.ring = r;
    }

    /// the replicas of a key (its preference list)
    pub fn replicas(&self, key: &str) -> Vec<Ipv4> {
        self.ring.preference(hash64(key.as_bytes()), self.rf)
    }
    /// this node believes the member alive
    pub fn is_up(&self, a: Ipv4) -> bool {
        Some(a) == self.me || self.members.get(&a).is_some_and(|m| m.up)
    }
    /// the quorums a request gets: (W, R)
    pub fn quorums(&self, cons: Option<Consistency>, w: Option<u8>, r: Option<u8>) -> (u8, u8) {
        let rf = self.rf.max(1) as u8;
        let maj = rf / 2 + 1;
        let (mut qw, mut qr) = match cons.unwrap_or(self.consistency) {
            Consistency::Strong => (if self.w > 0 { self.w } else { maj }, if self.r > 0 { self.r } else { maj }),
            Consistency::Eventual => (1, 1),
        };
        if let Some(x) = w {
            qw = x;
        }
        if let Some(x) = r {
            qr = x;
        }
        (qw.clamp(1, rf), qr.clamp(1, rf))
    }
    /// this node's own address in the cluster
    pub fn me(&self) -> Option<Ipv4> {
        self.me
    }
    /// the entry this node holds for a key
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.data.get(key)
    }
    /// keys with a value (no deletions)
    pub fn keys(&self) -> usize {
        self.data.values().filter(|e| !e.deleted).count()
    }
    /// the bytes of what it stores
    pub fn bytes(&self) -> u64 {
        self.data.iter().map(|(k, e)| size_of(k, e)).sum()
    }
    /// the stored keys (with deletions), in order
    pub fn stored(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.data.iter()
    }
    /// the members as this node sees them (itself included, always up)
    pub fn member_list(&self) -> Vec<Member> {
        self.nodes
            .iter()
            .map(|a| match self.members.get(a) {
                Some(m) => *m,
                None => Member { addr: *a, up: true, heard: self.seen, since: 0 },
            })
            .collect()
    }
    pub fn ring(&self) -> &Ring<Ipv4> {
        &self.ring
    }

    /// changes the consistency and quorums while it runs (Action::KvSet)
    pub fn set(&mut self, cons: Option<Consistency>, w: Option<u8>, r: Option<u8>) {
        if let Some(c) = cons {
            self.consistency = c;
        }
        if let Some(x) = w {
            self.w = x.min(self.rf as u8);
        }
        if let Some(x) = r {
            self.r = x.min(self.rf as u8);
        }
    }

    fn op_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn answer(&mut self, os: &mut OsApi, sid: u64, status: u16, body: Vec<u8>, h: Headers, start: Option<Ns>) {
        let now = os.now();
        let mut h = h;
        if h.get("content-type").is_none() {
            h.add("Content-Type", if status == 200 { "application/octet-stream" } else { "text/plain" });
        }
        let rep = Reply { status, headers: h, body, chunk: 0, close: false };
        if self.serve.respond(os, sid, rep)
            && let Some(t) = start
        {
            self.lat.record(now, now - t);
            match status {
                200..=299 => self.stats.ok += 1,
                404 => {
                    self.stats.ok += 1;
                    self.stats.not_found += 1;
                }
                503 => self.stats.unavailable += 1,
                507 => self.stats.full += 1,
                _ => {}
            }
        }
    }

    fn text(&mut self, os: &mut OsApi, sid: u64, status: u16, text: &str, start: Option<Ns>) {
        self.answer(os, sid, status, Vec::from(text.as_bytes()), Headers::new(), start);
    }

    // ---------------------------------------------------------------- requests

    fn request(&mut self, os: &mut OsApi, r: ReqIn) {
        let path = String::from(r.head.path());
        let m = r.head.method.clone();
        if let Some(k) = path.strip_prefix("/_kv/") {
            let key = dec(k);
            match m.as_str() {
                "GET" => self.replica_read(os, r.id, key),
                "PUT" => self.replica_write(os, &r, key),
                _ => self.text(os, r.id, 405, "405 Method Not Allowed\n", None),
            }
            return;
        }
        let Some(k) = path.strip_prefix("/kv/").filter(|k| !k.is_empty()) else {
            if path == "/kv" || path == "/kv/" {
                self.text(os, r.id, 400, "Es fehlt der Schlüssel: /kv/SCHLÜSSEL\n", None);
            } else {
                self.text(os, r.id, 404, "404 Not Found\n", None);
            }
            return;
        };
        let key = dec(k);
        self.stats.requests += 1;
        let t = &r.head.target;
        let cons = query_value(t, "consistency").and_then(Consistency::parse);
        let qw = query_value(t, "w").and_then(|v| v.parse::<u8>().ok());
        let qr = query_value(t, "r").and_then(|v| v.parse::<u8>().ok());
        let (w, rq) = self.quorums(cons, qw, qr);
        match m.as_str() {
            "GET" | "HEAD" => self.read(os, r.id, key, rq, r.at),
            "PUT" | "POST" => self.write(os, &r, key, false, w),
            "DELETE" => self.write(os, &r, key, true, w),
            _ => {
                let mut h = Headers::new();
                h.add("Allow", "GET, HEAD, PUT, POST, DELETE");
                self.answer(os, r.id, 405, Vec::from(&b"405 Method Not Allowed\n"[..]), h, Some(r.at));
            }
        }
    }

    /// a read this node coordinates
    fn read(&mut self, os: &mut OsApi, sid: u64, key: String, need: u8, start: Ns) {
        let now = os.now();
        self.stats.reads += 1;
        let live: Vec<Ipv4> = self.replicas(&key).into_iter().filter(|a| self.is_up(*a)).collect();
        os.note(Note::KvCoordinate { key: key.clone(), op: "GET", replicas: live.len() as u8 });
        if (live.len() as u8) < need {
            os.note(Note::KvQuorum { key: key.clone(), got: live.len() as u8, need, ok: false });
            self.text(os, sid, 503, &format!("Zu wenige Replikate erreichbar: {} von {}\n", live.len(), need), Some(start));
            return;
        }
        let id = self.op_id();
        let timer = self.wheel.add(now + self.timeout, KTimer::Timeout(id));
        self.ops.insert(id, Op::Read { sid, key: key.clone(), need, targets: live.clone(), got: Vec::new(), failed: Vec::new(), answered: None, timer, start });
        for a in live {
            if Some(a) == self.me {
                let len = self.data.get(&key).map(|e| size_of(&key, e)).unwrap_or(key.len() as u64);
                let at = os.disk_read(len);
                let io = self.op_id();
                self.ios.insert(io, Io { why: IoFor::Read(id), key: key.clone(), entry: None, grow: 0 });
                self.wheel.add(at, KTimer::Io(io));
            } else {
                let aid = self.send(os, a, "GET", &format!("/_kv/{}", enc(&key)), Headers::new(), Vec::new());
                self.by_agent.insert(aid, (id, a));
            }
        }
    }

    /// a write (or deletion) of a client: forwarded to the leader, or led here
    fn write(&mut self, os: &mut OsApi, r: &ReqIn, key: String, deleted: bool, need: u8) {
        self.stats.writes += 1;
        let leader = self.replicas(&key).into_iter().find(|a| self.is_up(*a));
        let forwarded = r.head.header("x-kv-forwarded").is_some();
        if !forwarded
            && let Some(l) = leader
            && Some(l) != self.me
        {
            // the leader orders the writes of its keys
            self.stats.forwards += 1;
            os.note(Note::KvForward { key: key.clone(), to: l });
            let mut h = Headers::new();
            h.add("X-Kv-Forwarded", "1");
            let aid = self.send(os, l, &r.head.method, &r.head.target, h, r.body.clone());
            let id = self.op_id();
            self.ops.insert(id, Op::Forward { sid: r.id, key, deleted, need, body: r.body.clone(), start: r.at });
            self.by_agent.insert(aid, (id, l));
            return;
        }
        self.lead(os, r.id, key, deleted, need, r.body.clone(), r.at);
    }

    /// leads a write: the next version, stored here and sent to the other live replicas
    fn lead(&mut self, os: &mut OsApi, sid: u64, key: String, deleted: bool, need: u8, body: Vec<u8>, start: Ns) {
        let now = os.now();
        let live: Vec<Ipv4> = self.replicas(&key).into_iter().filter(|a| self.is_up(*a)).collect();
        os.note(Note::KvCoordinate { key: key.clone(), op: if deleted { "DELETE" } else { "PUT" }, replicas: live.len() as u8 });
        if (live.len() as u8) < need || live.is_empty() {
            os.note(Note::KvQuorum { key: key.clone(), got: live.len() as u8, need, ok: false });
            self.text(os, sid, 503, &format!("Zu wenige Replikate erreichbar: {} von {}\n", live.len(), need), Some(start));
            return;
        }
        self.stats.led += 1;
        self.clock += 1;
        let ver = Ver { n: self.clock, node: self.my_no };
        let entry = Entry { value: if deleted { Vec::new() } else { body }, ver, deleted };
        os.note(Note::KvWrite { key: key.clone(), version: ver.text(), need });
        let id = self.op_id();
        let timer = self.wheel.add(now + self.timeout, KTimer::Timeout(id));
        self.ops.insert(id, Op::Write { sid, key: key.clone(), ver, deleted, need, targets: live.len(), acks: 0, failed: 0, full: false, answered: false, timer, start });
        for a in live {
            if Some(a) == self.me {
                if self.local_write(os, &key, &entry, IoFor::Write(id)).is_none()
                    && let Some(Op::Write { failed, full, .. }) = self.ops.get_mut(&id)
                {
                    *failed += 1;
                    *full = true;
                }
            } else {
                let mut h = Headers::new();
                h.add("X-Version", &ver.text());
                if deleted {
                    h.add("X-Deleted", "1");
                }
                let aid = self.send(os, a, "PUT", &format!("/_kv/{}", enc(&key)), h, entry.value.clone());
                self.by_agent.insert(aid, (id, a));
            }
        }
        self.write_progress(os, id);
    }

    /// starts a local write of an entry on the disk: the access, None when the disk is full
    fn local_write(&mut self, os: &mut OsApi, key: &str, e: &Entry, why: IoFor) -> Option<u64> {
        let old = self.data.get(key).map(|x| size_of(key, x)).unwrap_or(0) as i64;
        let new = size_of(key, e) as i64;
        let grow = new - old;
        match os.disk_write(new as u64, grow) {
            Some(at) => {
                let io = self.op_id();
                self.ios.insert(io, Io { why, key: String::from(key), entry: Some(e.clone()), grow });
                self.wheel.add(at, KTimer::Io(io));
                Some(io)
            }
            None => {
                os.note(Note::DiskFull);
                None
            }
        }
    }

    /// stores an entry whose write finished, if it is newer; true when it was
    fn apply(&mut self, os: &mut OsApi, key: &str, e: Entry, reserved: i64) -> bool {
        let cur = self.data.get(key);
        let newer = cur.is_none_or(|c| e.ver > c.ver);
        if newer {
            let old = cur.map(|c| size_of(key, c)).unwrap_or(0) as i64;
            let actual = size_of(key, &e) as i64 - old;
            let corr = actual - reserved;
            if corr > 0 {
                let _ = os.disk_reserve(corr as u64);
            } else if corr < 0 {
                os.disk_free(corr.unsigned_abs());
            }
            self.clock = self.clock.max(e.ver.n);
            self.data.insert(String::from(key), e);
        } else if reserved > 0 {
            os.disk_free(reserved as u64);
        } else if reserved < 0 {
            let _ = os.disk_reserve(reserved.unsigned_abs());
        }
        newer
    }

    /// another node asks for this node's entry
    fn replica_read(&mut self, os: &mut OsApi, sid: u64, key: String) {
        self.stats.replica_reads += 1;
        let len = self.data.get(&key).map(|e| size_of(&key, e)).unwrap_or(key.len() as u64);
        let at = os.disk_read(len);
        let io = self.op_id();
        self.ios.insert(io, Io { why: IoFor::ReplicaRead(sid), key, entry: None, grow: 0 });
        self.wheel.add(at, KTimer::Io(io));
    }

    /// another node sends a version of a key
    fn replica_write(&mut self, os: &mut OsApi, r: &ReqIn, key: String) {
        let Some(ver) = r.head.header("x-version").and_then(Ver::parse) else {
            self.text(os, r.id, 400, "X-Version fehlt\n", None);
            return;
        };
        self.stats.replica_writes += 1;
        self.clock = self.clock.max(ver.n);
        let deleted = r.head.header("x-deleted") == Some("1");
        let repair = r.head.header("x-kv-repair").is_some();
        let e = Entry { value: if deleted { Vec::new() } else { r.body.clone() }, ver, deleted };
        if self.data.get(&key).is_some_and(|c| c.ver >= ver) {
            os.note(Note::KvReplica { key, version: ver.text(), stored: false });
            let mut h = Headers::new();
            h.add("X-Stored", "0");
            self.answer(os, r.id, 204, Vec::new(), h, None);
            return;
        }
        if self.local_write(os, &key, &e, IoFor::ReplicaWrite { sid: r.id, repair }).is_none() {
            self.stats.full += 1;
            self.text(os, r.id, 507, "Disk voll\n", None);
        }
    }

    /// sends a request to another node
    fn send(&mut self, os: &mut OsApi, to: Ipv4, method: &str, target: &str, h: Headers, body: Vec<u8>) -> u64 {
        let url = Url { host: format!("{}", to), port: self.port, target: String::from(target) };
        let req = Req { method: String::from(method), url, version: Version::Http11, headers: h, body, timeout: self.timeout, connect_timeout: (self.timeout / 2).max(MS), reuse: true, keep_body: true };
        self.agent.submit(os, req)
    }

    /// read repair: the newest version to a replica that had an older one
    fn repair(&mut self, os: &mut OsApi, to: Ipv4, key: &str, e: &Entry) {
        self.stats.read_repairs += 1;
        os.note(Note::KvReadRepair { key: String::from(key), to });
        if Some(to) == self.me {
            let _ = self.local_write(os, key, e, IoFor::Repair);
            return;
        }
        let mut h = Headers::new();
        h.add("X-Version", &e.ver.text());
        h.add("X-Kv-Repair", "1");
        if e.deleted {
            h.add("X-Deleted", "1");
        }
        let aid = self.send(os, to, "PUT", &format!("/_kv/{}", enc(key)), h, e.value.clone());
        self.by_agent.insert(aid, (0, to));
    }

    // ---------------------------------------------------------------- progress

    /// a reply of a replica to a read
    fn read_reply(&mut self, os: &mut OsApi, id: u64, from: Ipv4, got: Option<Option<Entry>>) {
        if let Some(Op::Read { got: g, failed, .. }) = self.ops.get_mut(&id) {
            match got {
                Some(e) => g.push((from, e)),
                None => failed.push(from),
            }
        }
        self.read_progress(os, id, false);
    }

    fn read_progress(&mut self, os: &mut OsApi, id: u64, timeout: bool) {
        let Some(Op::Read { sid, key, need, targets, got, failed, answered, timer, start }) = self.ops.get(&id).cloned() else {
            return;
        };
        let newest = got.iter().filter_map(|g| g.1.as_ref()).max_by_key(|e| e.ver).cloned();
        let mut answered = answered;
        if answered.is_none() && got.len() as u8 >= need {
            os.note(Note::KvQuorum { key: key.clone(), got: got.len() as u8, need, ok: true });
            match &newest {
                Some(e) if !e.deleted => {
                    let mut h = Headers::new();
                    h.add("X-Version", &e.ver.text());
                    self.answer(os, sid, 200, e.value.clone(), h, Some(start));
                    answered = Some(e.ver);
                }
                Some(e) => {
                    self.text(os, sid, 404, "404 Not Found\n", Some(start));
                    answered = Some(e.ver);
                }
                None => {
                    self.text(os, sid, 404, "404 Not Found\n", Some(start));
                    answered = Some(Ver::default());
                }
            }
            if let Some(Op::Read { answered: a, .. }) = self.ops.get_mut(&id) {
                *a = answered;
            }
        }
        let all = got.len() + failed.len() >= targets.len();
        if !all && !timeout {
            return;
        }
        // the end: an answer if there was none, then read repair
        self.wheel.cancel(timer);
        self.ops.remove(&id);
        match answered {
            None => {
                os.note(Note::KvQuorum { key: key.clone(), got: got.len() as u8, need, ok: false });
                self.text(os, sid, 503, &format!("Lese-Quorum verfehlt: {} von {}\n", got.len(), need), Some(start));
            }
            Some(v) => {
                if newest.as_ref().is_some_and(|e| e.ver > v) {
                    self.stats.stale_reads += 1;
                }
            }
        }
        if self.read_repair
            && let Some(e) = newest
        {
            for (a, have) in &got {
                if have.as_ref().is_none_or(|h| h.ver < e.ver) {
                    self.repair(os, *a, &key, &e);
                }
            }
        }
    }

    fn write_progress(&mut self, os: &mut OsApi, id: u64) {
        let Some(Op::Write { sid, key, ver, deleted, need, targets, acks, failed, full, answered, timer, start }) = self.ops.get(&id).cloned() else {
            return;
        };
        if !answered && acks >= need {
            os.note(Note::KvQuorum { key: key.clone(), got: acks, need, ok: true });
            let mut h = Headers::new();
            h.add("X-Version", &ver.text());
            h.add("X-Acks", &format!("{}", acks));
            let (status, body) = if deleted { (204, Vec::new()) } else { (201, Vec::from(&b"gespeichert\n"[..])) };
            self.answer(os, sid, status, body, h, Some(start));
            if let Some(Op::Write { answered: a, .. }) = self.ops.get_mut(&id) {
                *a = true;
            }
        }
        let answered = answered || acks >= need;
        if (acks + failed) as usize >= targets {
            self.wheel.cancel(timer);
            self.ops.remove(&id);
            if !answered {
                os.note(Note::KvQuorum { key, got: acks, need, ok: false });
                if full && acks == 0 {
                    self.text(os, sid, 507, "Disk voll\n", Some(start));
                } else {
                    self.text(os, sid, 503, &format!("Schreib-Quorum verfehlt: {} von {}\n", acks, need), Some(start));
                }
            }
        }
    }

    fn timed_out(&mut self, os: &mut OsApi, id: u64) {
        match self.ops.get(&id) {
            Some(Op::Read { .. }) => self.read_progress(os, id, true),
            Some(Op::Write { sid, key, acks, need, answered, start, .. }) => {
                let (sid, key, acks, need, answered, start) = (*sid, key.clone(), *acks, *need, *answered, *start);
                self.ops.remove(&id);
                if !answered {
                    os.note(Note::KvQuorum { key, got: acks, need, ok: false });
                    self.text(os, sid, 503, &format!("Schreib-Quorum verfehlt: {} von {}\n", acks, need), Some(start));
                }
            }
            _ => {}
        }
    }

    /// a disk access is done
    fn io_done(&mut self, os: &mut OsApi, io: u64) {
        let Some(x) = self.ios.remove(&io) else {
            return;
        };
        match x.why {
            IoFor::Read(id) => {
                let e = self.data.get(&x.key).cloned();
                if let Some(me) = self.me {
                    self.read_reply(os, id, me, Some(e));
                }
            }
            IoFor::Write(id) => {
                if let Some(e) = x.entry {
                    self.apply(os, &x.key, e, x.grow);
                }
                if let Some(Op::Write { acks, .. }) = self.ops.get_mut(&id) {
                    *acks += 1;
                }
                self.write_progress(os, id);
            }
            IoFor::ReplicaRead(sid) => match self.data.get(&x.key).cloned() {
                Some(e) => {
                    let mut h = Headers::new();
                    h.add("X-Version", &e.ver.text());
                    if e.deleted {
                        h.add("X-Deleted", "1");
                    }
                    self.answer(os, sid, 200, e.value, h, None);
                }
                None => self.text(os, sid, 404, "404 Not Found\n", None),
            },
            IoFor::ReplicaWrite { sid, repair } => {
                let ver = x.entry.as_ref().map(|e| e.ver).unwrap_or_default();
                let stored = match x.entry {
                    Some(e) => self.apply(os, &x.key, e, x.grow),
                    None => false,
                };
                if stored && repair {
                    self.stats.repaired += 1;
                }
                os.note(Note::KvReplica { key: x.key.clone(), version: ver.text(), stored });
                let mut h = Headers::new();
                h.add("X-Stored", if stored { "1" } else { "0" });
                self.answer(os, sid, 204, Vec::new(), h, None);
            }
            IoFor::Repair => {
                if let Some(e) = x.entry
                    && self.apply(os, &x.key, e, x.grow)
                {
                    self.stats.repaired += 1;
                }
            }
        }
    }

    /// an answer of another node
    fn agent_done(&mut self, os: &mut OsApi, d: Done) {
        let Some((id, from)) = self.by_agent.remove(&d.id) else {
            return;
        };
        if id == 0 {
            return;
        }
        if d.result.is_err() {
            self.suspect(os, from);
        }
        match self.ops.get(&id) {
            Some(Op::Read { .. }) => {
                let got = match &d.result {
                    Ok(resp) if resp.head.status == 200 => {
                        let ver = resp.head.header("x-version").and_then(Ver::parse).unwrap_or_default();
                        let deleted = resp.head.header("x-deleted") == Some("1");
                        Some(Some(Entry { value: resp.body.clone(), ver, deleted }))
                    }
                    Ok(resp) if resp.head.status == 404 => Some(None),
                    _ => None,
                };
                self.read_reply(os, id, from, got);
            }
            Some(Op::Write { .. }) => {
                let ok = d.result.as_ref().is_ok_and(|r| (200..300).contains(&r.head.status));
                let full = d.result.as_ref().is_ok_and(|r| r.head.status == 507);
                if let Some(Op::Write { acks, failed, full: f, .. }) = self.ops.get_mut(&id) {
                    if ok {
                        *acks += 1;
                    } else {
                        *failed += 1;
                        *f |= full;
                    }
                }
                self.write_progress(os, id);
            }
            Some(Op::Forward { .. }) => {
                let Some(Op::Forward { sid, key, deleted, need, body, start }) = self.ops.remove(&id) else {
                    return;
                };
                match d.result {
                    Ok(resp) => {
                        let mut h = Headers::new();
                        for k in ["x-version", "x-acks", "content-type"] {
                            if let Some(v) = resp.head.header(k) {
                                h.add(k, v);
                            }
                        }
                        self.answer(os, sid, resp.head.status, resp.body, h, Some(start));
                    }
                    Err(f) => {
                        // the leader is gone: a replica leads the write itself
                        if self.me.is_some_and(|m| self.replicas(&key).contains(&m)) {
                            self.lead(os, sid, key, deleted, need, body, start);
                        } else {
                            let text = format!("Leader {} nicht erreichbar: {}\n", from, f.text());
                            self.text(os, sid, 503, &text, Some(start));
                        }
                    }
                }
            }
            None => {}
        }
    }

    // ---------------------------------------------------------------- heartbeats

    fn beat(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.beats += 1;
        let msg = format!("kvhb {}", self.beats);
        for a in self.nodes.clone() {
            if Some(a) != self.me {
                os.send(self.hb_port, a, self.hb_port, msg.as_bytes());
            }
        }
        let down_after = self.down_after;
        let mut gone = Vec::new();
        for m in self.members.values_mut() {
            if Some(m.addr) != self.me && m.up && now.saturating_sub(m.heard) >= down_after {
                m.up = false;
                m.since = now;
                gone.push(m.addr);
            }
        }
        for a in gone {
            self.stats.member_downs += 1;
            os.note(Note::KvMember { member: a, up: false });
            os.log(4, "kv", &format!("{} antwortet nicht mehr und gilt als ausgefallen", a));
        }
        self.wheel.add(now + self.beat, KTimer::Beat);
    }

    fn heard(&mut self, os: &mut OsApi, from: Ipv4) {
        let now = os.now();
        let Some(m) = self.members.get_mut(&from) else {
            return;
        };
        m.heard = now;
        if !m.up {
            m.up = true;
            m.since = now;
            os.note(Note::KvMember { member: from, up: true });
            os.log(5, "kv", &format!("{} antwortet wieder", from));
        }
    }

    /// a member did not answer a request: down until its next heartbeat
    fn suspect(&mut self, os: &mut OsApi, a: Ipv4) {
        let now = os.now();
        if Some(a) == self.me {
            return;
        }
        if let Some(m) = self.members.get_mut(&a)
            && m.up
        {
            m.up = false;
            m.since = now;
            self.stats.member_downs += 1;
            os.note(Note::KvMember { member: a, up: false });
            os.log(4, "kv", &format!("{} antwortet nicht und gilt als ausgefallen", a));
        }
    }

    fn timers(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                KTimer::Beat => self.beat(os),
                KTimer::Io(io) => self.io_done(os, io),
                KTimer::Timeout(id) => self.timed_out(os, id),
            }
        }
        self.wheel.arm(os);
    }

    /// what the lab shows
    pub fn view(&self, now: Ns) -> KvView {
        let (w, r) = self.quorums(None, None, None);
        KvView {
            me: self.me,
            port: self.port,
            members: self.member_list(),
            vnodes: self.vnodes,
            rf: self.rf,
            w,
            r,
            consistency: self.consistency,
            read_repair: self.read_repair,
            keys: self.keys() as u64,
            bytes: self.bytes(),
            stats: self.stats,
            pending: self.ops.len() as u64,
            p50: self.lat.p50(now),
            p95: self.lat.p95(now),
            p99: self.lat.p99(now),
            shares: self.ring.shares(),
        }
    }
}

/// a KV node as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvView {
    pub me: Option<Ipv4>,
    pub port: u16,
    pub members: Vec<Member>,
    pub vnodes: u32,
    pub rf: usize,
    /// the quorums of the node's consistency
    pub w: u8,
    pub r: u8,
    pub consistency: Consistency,
    pub read_repair: bool,
    pub keys: u64,
    pub bytes: u64,
    pub stats: KvStats,
    /// requests it coordinates now
    pub pending: u64,
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    /// each member's part of the ring (parts per million)
    pub shares: Vec<(Ipv4, u32)>,
}

/// a replica of a key as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaView {
    pub addr: Ipv4,
    /// the asking node believes it alive
    pub up: bool,
    /// the first live replica: it leads the writes of the key
    pub leader: bool,
    /// what that node holds (None: nothing, or no KV node found at the address)
    pub entry: Option<Entry>,
    /// the device of the address
    pub dev: Option<u32>,
}

fn us(v: Option<u64>) -> i64 {
    v.map(|x| (x / 1000) as i64).unwrap_or(0)
}

impl Service for KvNode {
    fn kind(&self) -> &'static str {
        "kv"
    }
    fn start(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.seen = now;
        self.serve.cfg.port = self.port;
        self.serve.stop();
        self.agent.reset();
        self.agent.idle_timeout = 30 * SEC;
        self.wheel.clear();
        self.ops.clear();
        self.ios.clear();
        self.by_agent.clear();
        if let Some((lat, bps, iops, cap)) = self.disk_cfg {
            os.disk_setup(lat, bps, iops, cap);
        }
        self.me = self.nodes.iter().copied().find(|a| os.is_mine(*a));
        self.my_no = self.me.and_then(|m| self.nodes.iter().position(|a| *a == m)).map(|i| i as u16 + 1).unwrap_or(0);
        self.members = self.nodes.iter().map(|a| (*a, Member { addr: *a, up: true, heard: now, since: now })).collect();
        self.serve.start(os, "kv");
        if os.bind(self.hb_port, None).is_err() {
            os.note(Note::PortInUse { port: self.hb_port, svc: String::from("kv") });
        }
        if self.me.is_none() {
            os.log(4, "kv", "keine eigene Adresse unter den Knoten: dieser Knoten speichert nichts");
        }
        self.beat(os);
        self.wheel.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        match ev {
            SvcEvent::Timer(K_OWN) => {
                self.timers(os);
                return;
            }
            SvcEvent::Udp(ref d) if d.dport == self.hb_port => {
                if d.data.starts_with(b"kvhb ") {
                    self.heard(os, d.src);
                }
                return;
            }
            _ => {}
        }
        if self.serve.on(&ev, os) {
            for r in self.serve.take() {
                self.request(os, r);
            }
        } else if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.agent_done(os, d);
            }
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        // writes the disk had not finished are lost
        for (_, x) in core::mem::take(&mut self.ios) {
            if x.grow > 0 {
                os.disk_free(x.grow as u64);
            } else if x.grow < 0 {
                let _ = os.disk_reserve(x.grow.unsigned_abs());
            }
        }
        self.serve.stop();
        self.agent.reset();
        self.wheel.clear();
        self.ops.clear();
        self.by_agent.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let s = &self.stats;
        let v = |x: u64| x as i64;
        alloc::vec![
            Metric::new("requests", v(s.requests)),
            Metric::new("reads", v(s.reads)),
            Metric::new("writes", v(s.writes)),
            Metric::new("ok", v(s.ok)),
            Metric::new("not_found", v(s.not_found)),
            Metric::new("unavailable", v(s.unavailable)),
            Metric::new("full", v(s.full)),
            Metric::new("forwards", v(s.forwards)),
            Metric::new("led", v(s.led)),
            Metric::new("replica_writes", v(s.replica_writes)),
            Metric::new("replica_reads", v(s.replica_reads)),
            Metric::new("stale_reads", v(s.stale_reads)),
            Metric::new("read_repairs", v(s.read_repairs)),
            Metric::new("repaired", v(s.repaired)),
            Metric::new("keys", self.keys() as i64),
            Metric::new("bytes", v(self.bytes())),
            Metric::new("members_up", self.nodes.iter().filter(|a| self.is_up(**a)).count() as i64),
            Metric::new("member_downs", v(s.member_downs)),
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
    fn status_msg(&self) -> Msg {
        let up = self.nodes.iter().filter(|a| self.is_up(**a)).count();
        let (w, r) = self.quorums(None, None, None);
        let me = match self.me {
            Some(a) => Msg::plain(format!("{}", a)),
            None => jfn_model::msg::phrase("nur Koordinator"),
        };
        msg!("{}: {} Schlüssel, {} von {} Knoten erreichbar, RF {}, {} W{}/R{}", me, self.keys(), up, self.nodes.len(), self.rf, self.consistency.name(), w, r)
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

/// addresses from a JSON list
fn addrs_of(v: Option<&Value>) -> Result<Vec<Ipv4>, Msg> {
    let mut out = Vec::new();
    for x in v.and_then(|x| x.as_array()).unwrap_or(&[]) {
        let s = x.as_str().unwrap_or("");
        out.push(Ipv4::parse(s.trim()).ok_or_else(|| msg!("\u{201e}{}\u{201c} ist keine IPv4-Adresse.", s))?);
    }
    Ok(out)
}

/// a KV node from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let nodes = addrs_of(c.get("nodes"))?;
    if nodes.is_empty() {
        return Err(msg!("\u{201e}nodes\u{201c} fehlt."));
    }
    let mut k = KvNode::new(nodes);
    k.port = match num("port", KV_PORT as u64) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    k.hb_port = match num("hb_port", HB_PORT as u64) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    k.vnodes = num("vnodes", 16).clamp(1, 1000) as u32;
    k.rf = (num("rf", 2) as usize).clamp(1, k.nodes.len());
    k.w = num("w", 0).min(k.rf as u64) as u8;
    k.r = num("r", 0).min(k.rf as u64) as u8;
    if let Some(s) = c.get("consistency").and_then(|v| v.as_str()) {
        k.consistency = Consistency::parse(s).ok_or_else(|| msg!("\u{201e}consistency\u{201c} kennt {} nicht (strong, eventual).", s))?;
    }
    k.read_repair = c.get("read_repair").and_then(|v| v.as_bool()).unwrap_or(true);
    k.timeout = ms("timeout_ms", 500).max(MS);
    k.beat = ms("heartbeat_ms", 500).max(10 * MS);
    k.down_after = ms("down_ms", 1500).max(k.beat);
    k.disk_cfg = crate::disk::setup_of(c);
    k.rering();
    Ok(Box::new(k))
}

// ---------------------------------------------------------------- the client

/// a finished request of a KvClient
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvDone {
    pub id: u64,
    /// the status of the answer (502 / 504 when no node answered)
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// the node that answered
    pub node: Option<Ipv4>,
    /// the nodes it tried
    pub tries: u32,
    pub start: Ns,
    pub end: Ns,
    pub fail: Option<Fail>,
}

/// what a KvClient counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientStats {
    pub requests: u64,
    pub answered: u64,
    pub errors: u64,
    /// a node did not answer: the next replica was asked
    pub failovers: u64,
    pub timeouts: u64,
}

#[derive(Clone, Debug)]
struct CReq {
    method: String,
    /// a store request: the key; else a plain URL
    key: Option<String>,
    url: Option<Url>,
    headers: Headers,
    body: Vec<u8>,
    prefs: Vec<Ipv4>,
    tried: Vec<Ipv4>,
    start: Ns,
}

/// requests of a service to the store (and plain HTTP requests over the same agent): the
/// replicas of a key from the ring, the first one not known to be down; when a node does not
/// answer (refused, unreachable, timeout) it counts as down for `down_for` and the next
/// replica gets the request
#[derive(Clone, Debug)]
pub struct KvClient {
    pub nodes: Vec<Ipv4>,
    pub port: u16,
    pub vnodes: u32,
    pub rf: usize,
    pub timeout: Ns,
    pub connect_timeout: Ns,
    pub down_for: Ns,
    /// sent as ?consistency= with every store request
    pub consistency: Option<Consistency>,
    ring: Ring<Ipv4>,
    agent: Agent,
    reqs: BTreeMap<u64, CReq>,
    by_agent: BTreeMap<u64, u64>,
    down: BTreeMap<Ipv4, Ns>,
    done: Vec<KvDone>,
    next: u64,
    pub stats: ClientStats,
    lat: Window,
}

impl KvClient {
    /// a client that uses timer key `key` of its service
    pub fn new(key: u32) -> KvClient {
        KvClient {
            nodes: Vec::new(),
            port: KV_PORT,
            vnodes: 16,
            rf: 2,
            timeout: SEC,
            connect_timeout: 200 * MS,
            down_for: 2 * SEC,
            consistency: None,
            ring: Ring::new(),
            agent: Agent::new(key),
            reqs: BTreeMap::new(),
            by_agent: BTreeMap::new(),
            down: BTreeMap::new(),
            done: Vec::new(),
            next: 0,
            stats: ClientStats::default(),
            lat: Window::new(300, SEC),
        }
    }
    /// the nodes of the store (their ring must be the nodes' own: same addresses and vnodes)
    pub fn setup(&mut self, nodes: Vec<Ipv4>, port: u16, vnodes: u32, rf: usize) {
        let mut n = nodes;
        n.sort();
        n.dedup();
        self.nodes = n;
        self.port = port;
        self.vnodes = vnodes.max(1);
        self.rf = rf.clamp(1, self.nodes.len().max(1));
        let mut r = Ring::new();
        for a in &self.nodes {
            r.add(*a, &format!("{}", a), self.vnodes);
        }
        self.ring = r;
    }
    /// forgets everything (the service stopped)
    pub fn reset(&mut self) {
        self.agent.reset();
        self.agent.idle_timeout = 30 * SEC;
        self.reqs.clear();
        self.by_agent.clear();
        self.done.clear();
        self.down.clear();
    }
    /// requests not finished yet
    pub fn active(&self) -> usize {
        self.reqs.len()
    }
    pub fn take_done(&mut self) -> Vec<KvDone> {
        core::mem::take(&mut self.done)
    }
    /// latencies of the answered requests (p50 / p95 / p99 at `now`)
    pub fn latency(&self, now: Ns) -> (Option<Ns>, Option<Ns>, Option<Ns>) {
        (self.lat.p50(now), self.lat.p95(now), self.lat.p99(now))
    }
    /// the latencies of its requests (300 slots of one second; the time series of its service)
    pub fn window(&self) -> &Window {
        &self.lat
    }
    /// the nodes this client counts as down now
    pub fn down_now(&self, now: Ns) -> Vec<Ipv4> {
        self.down.iter().filter(|(_, t)| **t > now).map(|(a, _)| *a).collect()
    }

    /// a store request: GET, PUT or DELETE of a key; its id
    pub fn kv(&mut self, os: &mut OsApi, method: &str, key: &str, body: Vec<u8>) -> u64 {
        let prefs = self.ring.preference(hash64(key.as_bytes()), self.rf);
        self.submit(os, CReq { method: String::from(method), key: Some(String::from(key)), url: None, headers: Headers::new(), body, prefs, tried: Vec::new(), start: os.now() })
    }
    /// a plain HTTP request over the same connections; its id
    pub fn http(&mut self, os: &mut OsApi, method: &str, url: Url, headers: Headers, body: Vec<u8>) -> u64 {
        self.submit(os, CReq { method: String::from(method), key: None, url: Some(url), headers, body, prefs: Vec::new(), tried: Vec::new(), start: os.now() })
    }

    fn submit(&mut self, os: &mut OsApi, r: CReq) -> u64 {
        self.next += 1;
        let id = self.next;
        self.stats.requests += 1;
        self.reqs.insert(id, r);
        self.dispatch(os, id);
        id
    }

    /// sends a request to its next node
    fn dispatch(&mut self, os: &mut OsApi, id: u64) {
        let now = os.now();
        let Some(r) = self.reqs.get(&id) else {
            return;
        };
        let (url, node) = match &r.key {
            Some(k) => {
                let untried: Vec<Ipv4> = r.prefs.iter().copied().filter(|a| !r.tried.contains(a)).collect();
                let pick = untried.iter().copied().find(|a| self.down.get(a).is_none_or(|t| *t <= now)).or(untried.first().copied());
                let Some(node) = pick else {
                    self.finish(os, id, 502, Headers::new(), Vec::new(), None, None);
                    return;
                };
                let mut target = format!("/kv/{}", enc(k));
                if let Some(c) = self.consistency {
                    target.push_str(&format!("?consistency={}", c.name()));
                }
                (Url { host: format!("{}", node), port: self.port, target }, Some(node))
            }
            None => match &r.url {
                Some(u) => (u.clone(), Ipv4::parse(&u.host)),
                None => return,
            },
        };
        let req = Req { method: r.method.clone(), url, version: Version::Http11, headers: r.headers.clone(), body: r.body.clone(), timeout: self.timeout, connect_timeout: self.connect_timeout, reuse: true, keep_body: true };
        if let (Some(n), Some(r)) = (node, self.reqs.get_mut(&id)) {
            r.tried.push(n);
        }
        let aid = self.agent.submit(os, req);
        self.by_agent.insert(aid, id);
    }

    fn finish(&mut self, os: &mut OsApi, id: u64, status: u16, headers: Headers, body: Vec<u8>, node: Option<Ipv4>, fail: Option<Fail>) {
        let now = os.now();
        let Some(r) = self.reqs.remove(&id) else {
            return;
        };
        if fail.is_some() || status >= 500 {
            self.stats.errors += 1;
        } else {
            self.stats.answered += 1;
        }
        if fail.as_ref().is_some_and(|f| f.is_timeout()) {
            self.stats.timeouts += 1;
        }
        self.lat.record(now, now - r.start);
        self.done.push(KvDone { id, status, headers, body, node, tries: r.tried.len() as u32, start: r.start, end: now, fail });
    }

    /// handles an event of its agent; false when it is not one of them
    pub fn on(&mut self, ev: &SvcEvent, os: &mut OsApi) -> bool {
        if !self.agent.on(ev, os) {
            return false;
        }
        for d in self.agent.take_done() {
            let Some(id) = self.by_agent.remove(&d.id) else {
                continue;
            };
            let node = d.remote.map(|r| r.0);
            match d.result {
                Ok(resp) => self.finish(os, id, resp.head.status, resp.head.headers, resp.body, node, None),
                Err(f) => {
                    let store = self.reqs.get(&id).is_some_and(|r| r.key.is_some());
                    let left = self.reqs.get(&id).is_some_and(|r| r.prefs.iter().any(|a| !r.tried.contains(a)));
                    if store {
                        if let Some(n) = node.or_else(|| self.reqs.get(&id).and_then(|r| r.tried.last().copied())) {
                            let until = os.now() + self.down_for;
                            self.down.insert(n, until);
                        }
                        if left {
                            self.stats.failovers += 1;
                            self.dispatch(os, id);
                            continue;
                        }
                    }
                    let status = if matches!(f, Fail::Timeout { .. }) { 504 } else { 502 };
                    self.finish(os, id, status, Headers::new(), Vec::new(), node, Some(f));
                }
            }
        }
        true
    }
}

/// a client's settings from a service configuration: {"nodes": [...], "port": 7000,
/// "vnodes": 16, "rf": 2, "timeout_ms": 1000, "connect_timeout_ms": 200, "down_ms": 2000,
/// "consistency": "eventual"}
pub fn client_of(c: &Value, key: u32) -> Result<KvClient, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let nodes = addrs_of(c.get("nodes"))?;
    if nodes.is_empty() {
        return Err(msg!("\u{201e}kv.nodes\u{201c} fehlt."));
    }
    let mut k = KvClient::new(key);
    let port = match num("port", KV_PORT as u64) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    k.setup(nodes, port, num("vnodes", 16).clamp(1, 1000) as u32, num("rf", 2) as usize);
    k.timeout = ms("timeout_ms", 1000).max(MS);
    k.connect_timeout = ms("connect_timeout_ms", 200).max(MS);
    k.down_for = ms("down_ms", 2000);
    if let Some(s) = c.get("consistency").and_then(|v| v.as_str()) {
        k.consistency = Some(Consistency::parse(s).ok_or_else(|| msg!("\u{201e}consistency\u{201c} kennt {} nicht (strong, eventual).", s))?);
    }
    Ok(k)
}

// ---------------------------------------------------------------- the engine side

impl Engine {
    /// a KV node's view for the lab
    pub fn kv_view(&self, dev: u32, svc: &str) -> Option<KvView> {
        Some(self.service::<KvNode>(dev, svc)?.view(self.now))
    }
    /// the ring of a KV node: (token, member), sorted
    pub fn kv_ring(&self, dev: u32, svc: &str) -> Vec<(u64, Ipv4)> {
        self.service::<KvNode>(dev, svc).map(|k| k.ring().tokens().to_vec()).unwrap_or_default()
    }
    /// the device whose KV node has the address `a`
    pub fn kv_at(&self, a: Ipv4) -> Option<u32> {
        self.devices().find(|&d| self.node_index(d).and_then(|ni| self.stack(ni)).is_some_and(|s| s.is_mine(a)) && self.service_of::<KvNode>(d).is_some())
    }
    /// the replicas of a key as a KV node sees them, with what each one holds
    pub fn kv_where(&self, dev: u32, svc: &str, key: &str) -> Vec<ReplicaView> {
        let Some(k) = self.service::<KvNode>(dev, svc) else {
            return Vec::new();
        };
        let reps = k.replicas(key);
        let leader = reps.iter().copied().find(|a| k.is_up(*a));
        reps.into_iter()
            .map(|a| {
                let d = self.kv_at(a);
                let entry = d.and_then(|d| self.service_of::<KvNode>(d)).and_then(|n| n.entry(key).cloned());
                ReplicaView { addr: a, up: k.is_up(a), leader: Some(a) == leader, entry, dev: d }
            })
            .collect()
    }
}
