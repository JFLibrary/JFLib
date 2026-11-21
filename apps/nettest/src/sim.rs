// sim -- lib/netchan's protocol on a simulated network: a virtual clock, loss, duplicates,
// reordering, delay and jitter from a seed.  The independent reference is the bookkeeping here:
// what each side sent under which sequence number, what arrived, what the acks claimed.
//
//   every message arrives at most once and unchanged; Delivered(seq) only for packets that did
//   arrive; every sent packet ends Delivered or Lost exactly once (but those still in flight);
//   reliable streams deliver everything in order and lose nothing; the handshake survives loss;
//   timeouts, keep-alives, the connection caps, the protocol id, rate limits, forged cookies and
//   tokens, unpadded handshakes, answers never larger than their request, a client that starts
//   again, streams that never send CONNECT; the seal: no payload readable on the wire, changed,
//   forged and replayed packets dropped without harm, forged DISCONNECTs ignored, keys without a
//   shared secret refused, the static key (pinned: connects; wrong or missing: ServerKey; a server
//   in the middle: refused with it, taken without it)

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::{ClientConfig, ClientCore, ClientEvent, Out, Route, ServerConfig, ServerCore, ServerEvent, State};
use netchan::packet::{self, Packet};
use netchan::Reason;
use plat::net::{IpAddr, SocketAddr};

use crate::{Ctx, Rng, PROTOCOL};

/// what the network does to a packet
#[derive(Clone, Copy)]
pub struct Weather {
    pub loss: f64,
    pub dup: f64,
    /// the chance of an extra delay of up to `extra_us` (packets overtake each other)
    pub reorder: f64,
    pub extra_us: u64,
    pub delay_us: u64,
    pub jitter_us: u64,
}

pub const CLEAR: Weather = Weather { loss: 0.0, dup: 0.0, reorder: 0.0, extra_us: 0, delay_us: 10_000, jitter_us: 0 };

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Server(SocketAddr),
    Client(usize),
}

/// a server and its clients on one simulated network
pub struct Sim {
    pub now: u64,
    pub server: ServerCore,
    pub clients: Vec<(SocketAddr, ClientCore)>,
    pub w: Weather,
    rng: Rng,
    q: Vec<(u64, u64, Dest, Vec<u8>)>,
    order: u64,
    /// the network carries nothing (a cut cable)
    pub down: bool,
    /// the largest answer the server sent an address without a connection, and the request size
    pub pre_conn: Vec<(usize, usize)>,
    last_in: BTreeMap<SocketAddr, usize>,
    /// every datagram put on the network, when set (what someone listening sees)
    pub tap: Option<Vec<Vec<u8>>>,
}

/// a client's 32 random bytes from one number
pub fn random32(x: u64) -> [u8; 32] {
    let mut r = [0u8; 32];
    r[..8].copy_from_slice(&x.to_le_bytes());
    r
}

/// X25519's base point: a public key that gives a shared secret with any key
pub const BASE_POINT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

pub fn client_addr(i: usize) -> SocketAddr {
    SocketAddr::new(IpAddr::V4([10, 0, (i / 250) as u8, (i % 250) as u8 + 1]), 40000 + i as u16)
}

pub fn server_cfg() -> ServerConfig {
    ServerConfig { protocol: PROTOCOL, ..ServerConfig::default() }
}

pub fn client_cfg() -> ClientConfig {
    ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }
}

impl Sim {
    pub fn new(seed: u64, w: Weather, scfg: ServerConfig) -> Sim {
        let mut secret = [0u8; 32];
        secret[..8].copy_from_slice(&seed.to_le_bytes());
        Sim {
            now: 1_000_000_000,
            server: ServerCore::new(scfg, secret, 1_000_000_000),
            clients: Vec::new(),
            w,
            rng: Rng::new(seed),
            q: Vec::new(),
            order: 0,
            down: false,
            pre_conn: Vec::new(),
            last_in: BTreeMap::new(),
            tap: None,
        }
    }

    /// a client from `addr` that starts its handshake now -> its index
    pub fn add_client(&mut self, addr: SocketAddr, cfg: ClientConfig) -> usize {
        let mut c = ClientCore::new(cfg, random32(self.rng.next_u64()), false);
        c.start(self.now);
        self.clients.push((addr, c));
        self.flush();
        self.clients.len() - 1
    }

    fn push(&mut self, d: Dest, b: Vec<u8>) {
        if let Some(tap) = self.tap.as_mut() {
            tap.push(b.clone());
        }
        if self.down || self.rng.chance(self.w.loss) {
            return;
        }
        let copies = if self.rng.chance(self.w.dup) { 2 } else { 1 };
        for _ in 0..copies {
            let mut at = self.now + self.w.delay_us + if self.w.jitter_us > 0 { self.rng.next_u64() % self.w.jitter_us } else { 0 };
            if self.rng.chance(self.w.reorder) && self.w.extra_us > 0 {
                at += self.rng.next_u64() % self.w.extra_us;
            }
            self.order += 1;
            self.q.push((at, self.order, d, b.clone()));
        }
    }

    /// raw bytes from `from` to the server, through the network
    pub fn inject(&mut self, from: SocketAddr, b: Vec<u8>) {
        self.push(Dest::Server(from), b);
    }

    pub fn flush(&mut self) {
        while let Some(o) = self.server.take_out() {
            if let Out::Send(Route::Udp(a), b) = o {
                if let Some(i) = self.clients.iter().position(|(ca, _)| *ca == a) {
                    if self.clients[i].1.state() != State::Connected {
                        self.pre_conn.push((b.len(), *self.last_in.get(&a).unwrap_or(&0)));
                    }
                    self.push(Dest::Client(i), b);
                } else {
                    self.pre_conn.push((b.len(), *self.last_in.get(&a).unwrap_or(&0)));
                }
            }
        }
        for i in 0..self.clients.len() {
            while let Some(b) = self.clients[i].1.take_out() {
                let a = self.clients[i].0;
                self.push(Dest::Server(a), b);
            }
        }
    }

    /// advance the clock by dt and deliver what is due
    pub fn step(&mut self, dt: u64) {
        self.now += dt;
        let now = self.now;
        let mut due: Vec<(u64, u64, Dest, Vec<u8>)> = Vec::new();
        let mut i = 0;
        while i < self.q.len() {
            if self.q[i].0 <= now {
                due.push(self.q.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|x| (x.0, x.1));
        for (_, _, d, b) in due {
            match d {
                Dest::Server(from) => {
                    self.last_in.insert(from, b.len());
                    self.server.datagram(from, &b, now);
                }
                Dest::Client(i) => self.clients[i].1.packet(&b, now),
            }
        }
        self.server.tick(now);
        for c in self.clients.iter_mut() {
            c.1.tick(now);
        }
        self.flush();
    }

    pub fn server_events(&mut self) -> Vec<ServerEvent> {
        let mut v = Vec::new();
        while let Some(e) = self.server.poll_event() {
            v.push(e);
        }
        v
    }

    pub fn client_events(&mut self, i: usize) -> Vec<ClientEvent> {
        let mut v = Vec::new();
        while let Some(e) = self.clients[i].1.poll_event() {
            v.push(e);
        }
        v
    }

    /// step until every client is connected or gave up (at most `ms`)
    pub fn settle(&mut self, ms: u64) {
        for _ in 0..ms {
            self.step(1000);
            if self.clients.iter().all(|c| c.1.state() != State::Connecting) {
                break;
            }
        }
    }
}

/// the bookkeeping of one direction: what went out under which number, what came, what the acks said
#[derive(Default)]
struct Ledger {
    sent: BTreeMap<u16, u64>,
    arrived: BTreeSet<u64>,
    delivered: BTreeSet<u64>,
    lost: BTreeSet<u64>,
    dup_arrivals: u64,
    corrupt: u64,
    false_delivered: u64,
    false_lost: u64,
    twice: u64,
}

impl Ledger {
    fn on_send(&mut self, seq: u16, id: u64) {
        self.sent.insert(seq, id);
        // a number comes again after 65536 packets (keep-alives too, which the ledger does not
        // see): what went out 49152 numbers ago is forgotten, long decided
        self.sent.remove(&seq.wrapping_add(16384));
    }

    fn on_arrive(&mut self, seq: u16, data: &[u8]) {
        let Some(id) = decode(data) else {
            self.corrupt += 1;
            return;
        };
        if self.sent.get(&seq) != Some(&id) {
            self.corrupt += 1;
        }
        if !self.arrived.insert(id) {
            self.dup_arrivals += 1;
        }
    }

    fn on_delivered(&mut self, seq: u16) {
        let Some(&id) = self.sent.get(&seq) else { return }; // a keep-alive
        if !self.arrived.contains(&id) {
            self.false_delivered += 1;
        }
        if self.lost.contains(&id) || !self.delivered.insert(id) {
            self.twice += 1;
        }
    }

    fn on_lost(&mut self, seq: u16) {
        let Some(&id) = self.sent.get(&seq) else { return };
        if self.arrived.contains(&id) {
            self.false_lost += 1;
        }
        if self.delivered.contains(&id) || !self.lost.insert(id) {
            self.twice += 1;
        }
    }
}

/// a message: its id and filler, checked by a sum
fn encode(id: u64, len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len.max(12));
    v.extend_from_slice(&id.to_le_bytes());
    let mut x = id as u32 ^ 0x5a5a_5a5a;
    while v.len() + 4 < len.max(12) {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        v.push((x >> 16) as u8);
    }
    let sum = v.iter().fold(0u32, |s, b| s.wrapping_mul(31).wrapping_add(*b as u32));
    v.extend_from_slice(&sum.to_le_bytes());
    v
}

fn decode(d: &[u8]) -> Option<u64> {
    if d.len() < 12 {
        return None;
    }
    let (body, sum) = d.split_at(d.len() - 4);
    let s = body.iter().fold(0u32, |s, b| s.wrapping_mul(31).wrapping_add(*b as u32));
    (s.to_le_bytes() == sum).then(|| u64::from_le_bytes([body[0], body[1], body[2], body[3], body[4], body[5], body[6], body[7]]))
}

/// one client and the server trade `n` messages each way under `w` -> the two ledgers and the
/// client's final stats; messages every `every_us`, a quarter of a millisecond steps
fn exchange(c: &mut Ctx, name: &str, seed: u64, w: Weather, n: usize, every_us: u64) {
    let mut sim = Sim::new(seed, w, ServerConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, timeout_ms: 30_000, ..server_cfg() });
    let ci = sim.add_client(client_addr(0), ClientConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, timeout_ms: 30_000, connect_ms: 30_000, ..client_cfg() });
    sim.settle(30_000);
    let mut sid = None;
    for e in sim.server_events() {
        if let ServerEvent::Connected { id, .. } = e {
            sid = Some(id);
        }
    }
    let connected = sim.clients[ci].1.state() == State::Connected && sid.is_some();
    c.check(connected, || format!("{}: no connection ({:?})", name, sim.clients[ci].1.state()));
    if !connected {
        return;
    }
    let sid = sid.unwrap_or(0);
    let (mut up, mut down) = (Ledger::default(), Ledger::default());
    let (mut next_up, mut next_down) = (0u64, 1u64 << 40);
    let mut t_next = sim.now;
    let mut rng = Rng::new(seed ^ 77);
    let mut first_seq = None;
    let mut last_seq = 0u16;
    let end_send = n as u64 * every_us;
    let t0 = sim.now;
    let mut rtt_flowing = 0.0f32;
    while sim.now < t0 + end_send + 2_000_000 {
        if sim.now >= t0 + end_send && rtt_flowing == 0.0 {
            rtt_flowing = sim.clients[ci].1.stats().rtt_ms;
        }
        if sim.now >= t_next && sim.now < t0 + end_send {
            t_next += every_us;
            let len = 12 + rng.below(1100);
            if let Ok(seq) = sim.clients[ci].1.send(&encode(next_up, len), sim.now) {
                up.on_send(seq, next_up);
                first_seq.get_or_insert(seq);
                last_seq = seq;
            }
            next_up += 1;
            if let Ok(seq) = sim.server.send(sid, &encode(next_down, 12 + rng.below(1100)), sim.now) {
                down.on_send(seq, next_down);
            }
            next_down += 1;
            sim.flush();
        }
        sim.step(250);
        for e in sim.server_events() {
            match e {
                ServerEvent::Message { seq, data, .. } => up.on_arrive(seq, &data),
                ServerEvent::Delivered { seq, .. } => down.on_delivered(seq),
                ServerEvent::Lost { seq, .. } => down.on_lost(seq),
                ServerEvent::Disconnected { reason, .. } => {
                    c.check(false, || format!("{}: the server lost the client: {:?}", name, reason));
                }
                _ => {}
            }
        }
        for e in sim.client_events(ci) {
            match e {
                ClientEvent::Message { seq, data } => down.on_arrive(seq, &data),
                ClientEvent::Delivered(seq) => up.on_delivered(seq),
                ClientEvent::Lost(seq) => up.on_lost(seq),
                ClientEvent::Disconnected(r) => {
                    c.check(false, || format!("{}: the client was disconnected: {:?}", name, r));
                }
                _ => {}
            }
        }
    }
    for (dir, l) in [("up", &up), ("down", &down)] {
        let decided = l.delivered.len() + l.lost.len();
        c.check(l.corrupt == 0, || format!("{} {}: {} messages changed or under a wrong number", name, dir, l.corrupt));
        c.check(l.dup_arrivals == 0, || format!("{} {}: {} messages arrived twice", name, dir, l.dup_arrivals));
        c.check(l.false_delivered == 0, || format!("{} {}: {} Delivered for packets that never arrived", name, dir, l.false_delivered));
        c.check(l.twice == 0, || format!("{} {}: {} packets decided twice", name, dir, l.twice));
        c.check(decided + 40 >= n && decided <= n, || format!("{} {}: {} of {} sent decided", name, dir, decided, n));
        // a loss report for a packet that came after all is possible only with reordering beyond the bits
        let false_lost_ok = if w.reorder == 0.0 { l.false_lost == 0 } else { (l.false_lost as usize) * 50 <= n };
        c.check(false_lost_ok, || format!("{} {}: {} Lost for packets that arrived", name, dir, l.false_lost));
        let arrived = l.arrived.len() as f64 / n as f64;
        let expect = 1.0 - w.loss;
        c.check(arrived > expect - 0.06 && arrived <= 1.0, || format!("{} {}: {:.3} arrived, {:.3} expected", name, dir, arrived, expect));
    }
    if n > 70_000 {
        c.check(first_seq.is_some_and(|f| last_seq < f || n > 65_536), || format!("{}: the numbers wrapped ({:?} .. {})", name, first_seq, last_seq));
    }
    let st = sim.clients[ci].1.stats();
    if w.loss == 0.0 && w.reorder == 0.0 && w.jitter_us == 0 {
        let rtt = rtt_flowing as f64;
        let want = 2.0 * w.delay_us as f64 / 1000.0;
        // (an ack rides on the peer's next packet: up to every_us later)
        let slack = every_us as f64 / 1000.0 + 0.5;
        c.check(rtt >= want - 0.3 && rtt <= want + slack, || format!("{}: rtt {:.2} ms, the network's {:.2}", name, rtt, want));
    }
    if w.dup > 0.0 {
        c.check(st.duplicates > 0, || format!("{}: no duplicate counted", name));
    }
}

/// a reliable stream: the client over a lossless ordered queue to the server's stream
fn stream(c: &mut Ctx, seed: u64, n: usize) {
    let mut server = ServerCore::new(ServerConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, ..server_cfg() }, [3u8; 32], 0);
    let mut cl = ClientCore::new(ClientConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, ..client_cfg() }, [99; 32], true);
    let mut rng = Rng::new(seed);
    let mut now = 1_000_000u64;
    let delay = 5_000u64;
    let mut to_server: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut to_client: Vec<(u64, Vec<u8>)> = Vec::new();
    server.stream_open(7, Some(client_addr(0)), now);
    cl.start(now);
    let (mut sent, mut got, mut delivered, mut lost) = (0usize, Vec::new(), 0usize, 0usize);
    let mut sid = 0;
    let mut order_ok = true;
    for step in 0..(n as u64 * 4 + 4000) {
        now += 250;
        while let Some(b) = cl.take_out() {
            to_server.push((now + delay, b));
        }
        while let Some(o) = server.take_out() {
            if let Out::Send(Route::Stream(7), b) = o {
                to_client.push((now + delay, b));
            }
        }
        while to_server.first().is_some_and(|x| x.0 <= now) {
            let (_, b) = to_server.remove(0);
            server.stream_message(7, &b, now);
        }
        while to_client.first().is_some_and(|x| x.0 <= now) {
            let (_, b) = to_client.remove(0);
            cl.packet(&b, now);
        }
        server.tick(now);
        cl.tick(now);
        while let Some(e) = server.poll_event() {
            match e {
                ServerEvent::Connected { id, transport, .. } => {
                    sid = id;
                    order_ok &= transport == netchan::Transport::WebSocket;
                }
                ServerEvent::Message { data, .. } => got.push(decode(&data).unwrap_or(u64::MAX)),
                _ => {}
            }
        }
        while let Some(e) = cl.poll_event() {
            match e {
                ClientEvent::Delivered(_) => delivered += 1,
                ClientEvent::Lost(_) => lost += 1,
                _ => {}
            }
        }
        if step % 4 == 0 && sent < n && cl.state() == State::Connected {
            if cl.send(&encode(sent as u64, 12 + rng.below(3000)), now).is_ok() {
                sent += 1;
            }
        }
    }
    let _ = sid;
    let in_order = got.iter().enumerate().all(|(i, id)| *id == i as u64);
    c.check(order_ok && got.len() == n && in_order, || format!("stream: {} of {} messages, in order {}", got.len(), n, in_order));
    c.check(lost == 0 && delivered >= n, || format!("stream: delivered {} (keep-alives included), lost {}", delivered, lost));
    c.check(cl.reliable(), || String::from("stream: reliable"));
}

pub fn run(quick: bool) -> Ctx {
    let mut c = Ctx::new("sim", quick);
    let n = c.n(3000).max(400);
    exchange(&mut c, "clear", 1, CLEAR, n, 2000);
    let rough = Weather { loss: 0.2, dup: 0.1, reorder: 0.2, extra_us: 30_000, delay_us: 20_000, jitter_us: 5_000 };
    for seed in 0..if quick { 2 } else { 6 } {
        exchange(&mut c, "rough", 10 + seed, rough, n, 2000);
    }
    let storm = Weather { loss: 0.5, dup: 0.2, reorder: 0.3, extra_us: 60_000, delay_us: 40_000, jitter_us: 20_000 };
    exchange(&mut c, "storm", 30, storm, n, 3000);
    let dup_only = Weather { dup: 0.5, ..CLEAR };
    exchange(&mut c, "duplicates", 31, dup_only, n, 2000);
    // past the wrap of the 16-bit numbers
    let wrap_n = if quick { 8000 } else { 70_000 };
    exchange(&mut c, "wrap", 40, Weather { loss: 0.05, ..CLEAR }, wrap_n, 300);
    stream(&mut c, 50, n);

    // the handshake under loss: every attempt connects within connect_ms
    for seed in 0..if quick { 5 } else { 30 } {
        let mut sim = Sim::new(100 + seed, Weather { loss: 0.4, ..CLEAR }, server_cfg());
        let i = sim.add_client(client_addr(0), client_cfg());
        sim.settle(5000);
        c.check(sim.clients[i].1.state() == State::Connected, || format!("handshake with 40% loss, seed {}: {:?}", seed, sim.clients[i].1.state()));
    }
    // answers before a connection are never larger than the request that caused them
    let mut sim = Sim::new(5, Weather { loss: 0.3, ..CLEAR }, server_cfg());
    for k in 0..20 {
        sim.add_client(client_addr(k), client_cfg());
    }
    sim.settle(3000);
    let worst = sim.pre_conn.iter().filter(|(a, r)| a > r).count();
    c.check(!sim.pre_conn.is_empty() && worst == 0, || format!("answers before ACCEPT: {} larger than their request (of {})", worst, sim.pre_conn.len()));

    // keep-alives hold a quiet connection, a cut cable ends it on both sides
    let mut sim = Sim::new(6, CLEAR, ServerConfig { timeout_ms: 1000, ..server_cfg() });
    let i = sim.add_client(client_addr(0), ClientConfig { timeout_ms: 1000, ..client_cfg() });
    sim.settle(1000);
    for _ in 0..3000 {
        sim.step(1000);
    }
    let quiet_ok = sim.clients[i].1.state() == State::Connected && sim.server.connections() == 1;
    c.check(quiet_ok, || String::from("3 s without messages, timeout 1 s: still connected (keep-alives)"));
    let ka = sim.clients[i].1.stats().sent;
    c.check((25..=35).contains(&ka), || format!("keep-alives of the client in 3 s at 100 ms: {}", ka));
    sim.server_events();
    sim.down = true;
    let t_cut = sim.now;
    let mut t_server = None;
    for _ in 0..3000 {
        sim.step(1000);
        if t_server.is_none() && sim.server_events().iter().any(|e| matches!(e, ServerEvent::Disconnected { reason: Reason::Timeout, .. })) {
            t_server = Some(sim.now - t_cut);
        }
    }
    c.check(t_server.is_some_and(|t| (900_000..1_200_000).contains(&t)), || format!("server timeout after the cut: {:?} us", t_server));
    c.check(sim.clients[i].1.state() == State::Disconnected(Reason::Timeout), || format!("client after the cut: {:?}", sim.clients[i].1.state()));

    // the caps: connections and per address; the protocol id
    let mut sim = Sim::new(7, CLEAR, ServerConfig { max_connections: 3, ..server_cfg() });
    for k in 0..5 {
        sim.add_client(client_addr(k), client_cfg());
    }
    sim.settle(2000);
    let full = sim.clients.iter().filter(|c| c.1.state() == State::Disconnected(Reason::Full)).count();
    c.check(sim.server.connections() == 3 && full == 2, || format!("max_connections 3 of 5: {} in, {} full", sim.server.connections(), full));
    let mut sim = Sim::new(8, CLEAR, ServerConfig { max_per_ip: 2, ..server_cfg() });
    for k in 0..3 {
        sim.add_client(SocketAddr::new(IpAddr::V4([10, 9, 9, 9]), 5000 + k), client_cfg());
    }
    sim.settle(2000);
    c.check(sim.server.connections() == 2, || format!("max_per_ip 2 of 3: {}", sim.server.connections()));
    let mut sim = Sim::new(9, CLEAR, server_cfg());
    let i = sim.add_client(client_addr(0), ClientConfig { protocol: PROTOCOL + 1, ..client_cfg() });
    sim.settle(2000);
    c.check(sim.clients[i].1.state() == State::Disconnected(Reason::Protocol), || format!("another protocol id: {:?}", sim.clients[i].1.state()));
    // nobody answers: ConnectTimeout
    let mut sim = Sim::new(10, CLEAR, server_cfg());
    sim.down = true;
    let i = sim.add_client(client_addr(0), ClientConfig { connect_ms: 800, ..client_cfg() });
    sim.settle(2000);
    c.check(sim.clients[i].1.state() == State::Disconnected(Reason::ConnectTimeout), || format!("no server: {:?}", sim.clients[i].1.state()));

    // the rate limit over UDP: the excess is dropped, the connection stays
    let mut sim = Sim::new(11, CLEAR, ServerConfig { packets_per_s: 100, ..server_cfg() });
    let i = sim.add_client(client_addr(0), client_cfg());
    sim.settle(1000);
    sim.server_events();
    for k in 0..1000u64 {
        let _ = sim.clients[i].1.send(&encode(k, 20), sim.now);
    }
    sim.flush();
    for _ in 0..200 {
        sim.step(1000);
    }
    let got = sim.server_events().iter().filter(|e| matches!(e, ServerEvent::Message { .. })).count();
    let dropped = sim.server.ids().first().and_then(|&id| sim.server.stats(id)).map(|s| s.dropped).unwrap_or(0);
    c.check(got <= 230 && dropped >= 750 && sim.server.connections() == 1, || format!("1000 packets against 100 a second: {} taken, {} dropped", got, dropped));

    // forged and stale cookies, foreign tokens, unpadded handshakes
    let mut sim = Sim::new(12, CLEAR, server_cfg());
    let i = sim.add_client(client_addr(0), client_cfg());
    sim.settle(1000);
    sim.server_events();
    let bad0 = sim.server.server_stats().bad;
    let forged = Packet::Response { protocol: PROTOCOL, nonce: 5, time: (sim.now / 1_000_000) as u32, mac: [7; 16], flags: 0, key: BASE_POINT }.to_vec(true);
    sim.inject(client_addr(50), forged);
    let unpadded = Packet::Connect { protocol: PROTOCOL, nonce: 6 }.to_vec(false);
    sim.inject(client_addr(51), unpadded);
    let token_wrong = Packet::Data { token: 0x1234_5678, seq: 1, ack: 0, bits: 0, payload: b"intruder msg" }.to_vec(false);
    sim.inject(client_addr(0), token_wrong);
    for _ in 0..100 {
        sim.step(1000);
    }
    let msgs = sim.server_events().iter().filter(|e| matches!(e, ServerEvent::Message { .. } | ServerEvent::Connected { .. })).count();
    c.check(sim.server.connections() == 1 && msgs == 0 && sim.server.server_stats().bad >= bad0 + 3, || format!("forged cookie, unpadded CONNECT, wrong token: {} connections, {} events, bad {}", sim.server.connections(), msgs, sim.server.server_stats().bad - bad0));
    // a cookie from another address and one too old
    let probe = client_addr(60);
    let mut s2 = Sim::new(13, CLEAR, server_cfg());
    let t = s2.now;
    s2.server.datagram(probe, &Packet::Connect { protocol: PROTOCOL, nonce: 77 }.to_vec(true), t);
    let mut cookie = None;
    while let Some(o) = s2.server.take_out() {
        if let Out::Send(_, b) = o {
            if let Some(Packet::Challenge { time, mac, .. }) = packet::parse(&b) {
                cookie = Some((time, mac));
            }
        }
    }
    if let Some((time, mac)) = cookie {
        let resp = Packet::Response { protocol: PROTOCOL, nonce: 77, time, mac, flags: 0, key: BASE_POINT }.to_vec(true);
        s2.server.datagram(client_addr(61), &resp, t);
        c.check(s2.server.connections() == 0, || String::from("a cookie used from another address"));
        s2.server.datagram(probe, &resp, t + 11_000_000);
        c.check(s2.server.connections() == 0, || String::from("a cookie of 11 s"));
        s2.server.datagram(probe, &resp, t + 1_000_000);
        c.check(s2.server.connections() == 1, || String::from("the cookie in time from its address"));
    } else {
        c.check(false, || String::from("no CHALLENGE to the probe"));
    }
    let _ = i;

    // a client that starts again from the same address replaces its old connection
    let mut sim = Sim::new(14, CLEAR, server_cfg());
    sim.add_client(client_addr(0), client_cfg());
    sim.settle(1000);
    sim.server_events();
    let mut again = ClientCore::new(client_cfg(), random32(4242), false);
    again.start(sim.now);
    sim.clients[0].1 = again;
    sim.flush();
    sim.settle(1000);
    let ev = sim.server_events();
    let replaced = ev.iter().any(|e| matches!(e, ServerEvent::Disconnected { reason: Reason::Replaced, .. }));
    let connected = ev.iter().any(|e| matches!(e, ServerEvent::Connected { .. }));
    c.check(replaced && connected && sim.server.connections() == 1, || format!("a client starting again: replaced {}, connected {}", replaced, connected));

    // streams: no CONNECT in time, too fast, too large
    let mut s = ServerCore::new(ServerConfig { handshake_ms: 500, packets_per_s: 50, ..server_cfg() }, [1; 32], 0);
    s.stream_open(1, None, 0);
    s.tick(600_000);
    let mut closed = false;
    while let Some(o) = s.take_out() {
        closed |= o == Out::Close(1);
    }
    c.check(closed, || String::from("a stream without CONNECT is closed after handshake_ms"));
    s.stream_open(2, None, 1_000_000);
    s.stream_message(2, &Packet::Connect { protocol: PROTOCOL, nonce: 1 }.to_vec(false), 1_000_000);
    for k in 0..400u16 {
        s.stream_message(2, &Packet::Data { token: 0, seq: k, ack: 0xffff, bits: 0, payload: b"x" }.to_vec(false), 1_000_000);
    }
    let mut rate = false;
    while let Some(e) = s.poll_event() {
        rate |= matches!(e, ServerEvent::Disconnected { reason: Reason::RateLimit, .. });
    }
    c.check(rate, || String::from("a stream over packets_per_s is disconnected"));
    s.stream_open(3, None, 2_000_000);
    s.stream_message(3, &Packet::Connect { protocol: PROTOCOL, nonce: 1 }.to_vec(false), 2_000_000);
    let huge = alloc::vec![1u8; 64 * 1024 + 1];
    s.stream_message(3, &Packet::Data { token: 0, seq: 0, ack: 0xffff, bits: 0, payload: &huge }.to_vec(false), 2_000_000);
    let mut too_big = false;
    while let Some(e) = s.poll_event() {
        too_big |= matches!(e, ServerEvent::Disconnected { reason: Reason::Protocol, .. });
    }
    c.check(too_big, || String::from("a stream message over max_stream_payload ends the connection"));
    seal(&mut c);
    c
}

// the client's one connection and the server's id for it, after the handshake
fn connected(seed: u64, scfg: ServerConfig, ccfg: ClientConfig, key: Option<[u8; 32]>) -> (Sim, usize, Option<u32>) {
    let mut sim = Sim::new(seed, CLEAR, scfg);
    if let Some(k) = key {
        sim.server.set_static_key(k);
    }
    let i = sim.add_client(client_addr(0), ccfg);
    sim.settle(1000);
    let sid = sim.server_events().iter().find_map(|e| if let ServerEvent::Connected { id, .. } = e { Some(*id) } else { None });
    (sim, i, sid)
}

// what the server received from its client as messages, and whether it lost it
fn server_got(sim: &mut Sim) -> (Vec<Vec<u8>>, bool) {
    let mut got = Vec::new();
    let mut gone = false;
    for e in sim.server_events() {
        match e {
            ServerEvent::Message { data, .. } => got.push(data),
            ServerEvent::Disconnected { .. } => gone = true,
            _ => {}
        }
    }
    (got, gone)
}

/// the seal over UDP: what listening, changing, forging and replaying get, and the static key
fn seal(c: &mut Ctx) {
    // someone listening: no payload in clear, the keys differ per connection
    let marker = b"nobody on the wire may read this sentence";
    let (mut sim, i, sid) = connected(20, server_cfg(), client_cfg(), None);
    sim.tap = Some(Vec::new());
    let sid = sid.unwrap_or(0);
    let _ = sim.clients[i].1.send(marker, sim.now);
    let _ = sim.server.send(sid, marker, sim.now);
    sim.flush();
    for _ in 0..50 {
        sim.step(1000);
    }
    let (got, _) = server_got(&mut sim);
    let down = sim.client_events(i).iter().any(|e| matches!(e, ClientEvent::Message { data, .. } if data == marker));
    let wire = sim.tap.take().unwrap_or_default();
    let readable = wire.iter().filter(|b| b.windows(8).any(|w| marker.windows(8).any(|m| m == w))).count();
    c.check(got.iter().any(|d| d == marker) && down, || format!("seal: the message both ways ({} up, down {})", got.len(), down));
    c.check(!wire.is_empty() && readable == 0, || format!("seal: {} of {} datagrams carry 8 bytes of the payload in clear", readable, wire.len()));
    c.check(wire.iter().filter(|b| b.first().is_some_and(|k| k & 0x0f == packet::KIND_DATA)).all(|b| b.len() >= packet::DATA_HEADER + packet::TAG),
        || String::from("seal: a DATA packet without its tag"));

    // changed, forged and replayed packets: dropped, counted, the connection unharmed
    let fast = ServerConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, ..server_cfg() };
    let (mut sim, i, sid) = connected(21, fast, client_cfg(), None);
    let from = client_addr(0);
    let _ = sim.clients[i].1.send(b"the genuine message 1", sim.now);
    let genuine = sim.clients[i].1.take_out().unwrap_or_default();
    let token = match packet::parse(&genuine) {
        Some(Packet::Data { token, .. }) => token,
        _ => 0,
    };
    let now = sim.now;
    let mut flipped = 0u64;
    for k in 0..genuine.len() {
        // every byte changed once: header (authenticated), payload (encrypted), tag
        let mut m = genuine.clone();
        m[k] ^= 0x40;
        sim.server.datagram(from, &m, now);
        flipped += 1;
    }
    let forged = Packet::Data { token, seq: 1, ack: 0, bits: 0, payload: &[0x55; 40] }.to_vec(false);
    sim.server.datagram(from, &forged, now);
    let bye = Packet::Disconnect { token, seq: 2, reason: packet::BYE_NORMAL, tag: &[0; 16] }.to_vec(false);
    sim.server.datagram(from, &bye, now);
    let bye_plain = Packet::Disconnect { token, seq: 2, reason: packet::BYE_NORMAL, tag: &[] }.to_vec(false);
    sim.server.datagram(from, &bye_plain, now);
    let (got, gone) = server_got(&mut sim);
    let forged_n = sid.and_then(|s| sim.server.stats(s)).map_or(0, |s| s.forged);
    c.check(got.is_empty() && !gone && sim.server.connections() == 1, || format!("seal: changed and forged packets: {} messages, gone {}", got.len(), gone));
    // (a changed kind or token byte, the first 5, makes no packet of this connection: bad, not forged)
    let want = flipped - 5 + 3;
    c.check(forged_n == want, || format!("seal: {} forged counted, {} expected ({} changed, 3 forged)", forged_n, want, flipped));
    sim.server.datagram(from, &genuine, now);
    sim.server.datagram(from, &genuine, now + 1000);
    let (got, _) = server_got(&mut sim);
    let dups = sid.and_then(|s| sim.server.stats(s)).map_or(0, |s| s.duplicates);
    c.check(got.len() == 1 && got[0] == b"the genuine message 1" && dups == 1, || format!("seal: the genuine packet after the forgeries: {} messages, {} duplicates", got.len(), dups));
    // an old packet replayed after more than the window: stale (or its tag fails), never delivered
    for k in 0..1100u32 {
        let _ = sim.clients[i].1.send(&k.to_le_bytes(), sim.now);
    }
    sim.flush();
    for _ in 0..50 {
        sim.step(1000);
    }
    server_got(&mut sim);
    sim.server.datagram(from, &genuine, sim.now);
    let (got, _) = server_got(&mut sim);
    c.check(got.is_empty() && sim.server.connections() == 1, || format!("seal: a packet replayed 1100 packets later: {} messages", got.len()));
    // the client's side: a forged DISCONNECT with the connection's token
    let fake = Packet::Disconnect { token, seq: 9, reason: packet::BYE_SHUTDOWN, tag: &[1; 16] }.to_vec(false);
    let t1 = sim.now;
    sim.clients[i].1.packet(&fake, t1);
    c.check(sim.clients[i].1.state() == State::Connected, || format!("seal: a forged DISCONNECT to the client: {:?}", sim.clients[i].1.state()));

    // a RESPONSE whose key gives no shared secret (a low-order point): no connection
    let mut s2 = Sim::new(22, CLEAR, server_cfg());
    let probe = client_addr(70);
    let t = s2.now;
    s2.server.datagram(probe, &Packet::Connect { protocol: PROTOCOL, nonce: 3 }.to_vec(true), t);
    let mut cookie = None;
    while let Some(o) = s2.server.take_out() {
        if let Out::Send(_, b) = o {
            if let Some(Packet::Challenge { time, mac, .. }) = packet::parse(&b) {
                cookie = Some((time, mac));
            }
        }
    }
    let bad0 = s2.server.server_stats().bad;
    if let Some((time, mac)) = cookie {
        s2.server.datagram(probe, &Packet::Response { protocol: PROTOCOL, nonce: 3, time, mac, flags: 0, key: [0; 32] }.to_vec(true), t);
    }
    c.check(s2.server.connections() == 0 && s2.server.server_stats().bad == bad0 + 1, || String::from("seal: a RESPONSE with an all-zero key"));

    // the static key: pinned, it connects; a wrong one or a server without one: ServerKey
    let secret = [0x42u8; 32];
    let mut probe_srv = ServerCore::new(server_cfg(), [1; 32], 0);
    probe_srv.set_static_key(secret);
    let public = probe_srv.public_key().unwrap_or([0; 32]);
    let (mut sim, i, sid) = connected(23, server_cfg(), ClientConfig { server_key: Some(public), ..client_cfg() }, Some(secret));
    let _ = sim.clients[i].1.send(b"pinned", sim.now);
    sim.flush();
    for _ in 0..30 {
        sim.step(1000);
    }
    let (got, _) = server_got(&mut sim);
    c.check(sid.is_some() && got.len() == 1, || format!("static key pinned: {:?}, {} messages", sim.clients[i].1.state(), got.len()));
    let (sim, i, _) = connected(24, server_cfg(), ClientConfig { server_key: Some(BASE_POINT), ..client_cfg() }, Some(secret));
    c.check(sim.clients[i].1.state() == State::Disconnected(Reason::ServerKey) && sim.server.connections() == 1,
        || format!("a wrong static key: {:?}", sim.clients[i].1.state()));
    let (sim, i, _) = connected(25, server_cfg(), ClientConfig { server_key: Some(public), ..client_cfg() }, None);
    c.check(sim.clients[i].1.state() == State::Disconnected(Reason::ServerKey) && sim.server.connections() == 0,
        || format!("a server without a static key: {:?}", sim.clients[i].1.state()));
    let (sim, i, _) = connected(26, server_cfg(), client_cfg(), Some(secret));
    c.check(sim.clients[i].1.state() == State::Connected, || format!("a static key the client does not pin: {:?}", sim.clients[i].1.state()));

    // a server in the middle (someone on the path answers with keys of its own): with the static
    // key the client refuses it; without, it cannot tell (what ClientConfig::server_key is for)
    for pinned in [true, false] {
        let mut mitm = ServerCore::new(server_cfg(), [0x66; 32], 1_000_000_000);
        mitm.set_static_key([0x67; 32]);
        let cfg = ClientConfig { server_key: pinned.then_some(public), ..client_cfg() };
        let mut cl = ClientCore::new(cfg, random32(27), false);
        let from = client_addr(0);
        let mut now = 1_000_000_000u64;
        cl.start(now);
        for _ in 0..6 {
            now += 1000;
            while let Some(b) = cl.take_out() {
                mitm.datagram(from, &b, now);
            }
            while let Some(o) = mitm.take_out() {
                if let Out::Send(_, b) = o {
                    cl.packet(&b, now);
                }
            }
        }
        let want = if pinned { State::Disconnected(Reason::ServerKey) } else { State::Connected };
        c.check(cl.state() == want, || format!("a server in the middle, pinned {}: {:?}", pinned, cl.state()));
    }
}
