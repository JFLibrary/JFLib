// endpoint -- the server's and the client's protocol without I/O (docs/TRANSPORT.md "Handschlag")
//
//   ServerCore::new(ServerConfig, secret)
//     .datagram(from, bytes, now)        a UDP datagram
//     .stream_open(id, addr, now) / .stream_message(id, bytes, now) / .stream_closed(id, code)
//                                        a WebSocket (or any reliable message stream)
//     .tick(now)                         keep-alives, timeouts, streams that never said CONNECT
//     .send(conn, payload, now) -> seq   .disconnect(conn, now)
//     .take_out() -> Out (what to send where, streams to close)   .poll_event() -> ServerEvent
//   ClientCore::new(ClientConfig, nonce, stream)
//     .start(now) / .packet(bytes, now) / .tick(now) / .send(payload, now) / .disconnect(now)
//     .transport_closed(code) / .take_out() -> bytes / .poll_event() -> ClientEvent
//
// Time is the caller's (microseconds), so tests run them on a virtual clock over a simulated
// network (apps/nettest).  The UDP handshake: CONNECT (padded) -> CHALLENGE with a stateless
// cookie, HMAC-SHA256 over the client's address, its nonce and the time -> RESPONSE (padded) with
// the cookie -> ACCEPT with a random token, which every DATA and DISCONNECT then carries.  Nothing
// is kept for an address before its RESPONSE proves it receives there, the answers before that
// are smaller than the requests, and challenges_per_s bounds the work for all of them.  A
// WebSocket sends CONNECT and gets ACCEPT (token 0); TCP has proved the address.  The token is no
// encryption: it keeps senders out that do not see the traffic.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use plat::net::{IpAddr, SocketAddr};

use crate::conn::{Bucket, Conn, Event, Settings, Stats};
use crate::packet::{self, Packet, DATA_HEADER, HANDSHAKE_SIZE, MAX_UDP_PAYLOAD};
use crate::{Error, Reason, Transport};

/// how long a cookie of a CHALLENGE stays good (seconds)
const COOKIE_S: u32 = 10;
/// DISCONNECT over UDP goes out this many times (it is not acknowledged)
const BYE_COPIES: usize = 3;

/// a server's limits and timing
#[derive(Clone, Copy, Debug)]
pub struct ServerConfig {
    /// the app's protocol id: clients with another one are refused (another game, another version)
    pub protocol: u32,
    /// connections at once (UDP and WebSocket together)
    pub max_connections: usize,
    /// connections from one IP address
    pub max_per_ip: usize,
    /// the largest payload of a UDP message, both ways (at most MAX_UDP_PAYLOAD)
    pub max_payload: usize,
    /// the largest payload of a message over a WebSocket, both ways
    pub max_stream_payload: usize,
    /// nothing received for this long ends a connection
    pub timeout_ms: u32,
    /// an empty DATA packet after this long without sending (carries the acks)
    pub keepalive_ms: u32,
    /// packets a second from one client (UDP: more are dropped; WebSocket: it is disconnected)
    pub packets_per_s: u32,
    /// bytes a second from one client (as packets_per_s)
    pub bytes_per_s: u32,
    /// answers to handshakes a second, for all addresses together
    pub challenges_per_s: u32,
    /// a WebSocket must send its CONNECT within this long
    pub handshake_ms: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            protocol: 0,
            max_connections: 64,
            max_per_ip: 16,
            max_payload: 1200,
            max_stream_payload: 64 * 1024,
            timeout_ms: 5000,
            keepalive_ms: 100,
            packets_per_s: 300,
            bytes_per_s: 256 * 1024,
            challenges_per_s: 2000,
            handshake_ms: 5000,
        }
    }
}

/// where a packet goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// a UDP datagram to this address
    Udp(SocketAddr),
    /// a message on this stream (WebSocket)
    Stream(u32),
}

/// what a core wants done
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// send these bytes
    Send(Route, Vec<u8>),
    /// close this stream (after what was sent on it)
    Close(u32),
}

/// what happened on the server
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    /// a client came (`addr`: its address, if the transport knows it)
    Connected { id: u32, transport: Transport, addr: Option<SocketAddr> },
    /// a message of a client
    Message { id: u32, seq: u16, data: Vec<u8> },
    /// a client has a packet the server sent
    Delivered { id: u32, seq: u16 },
    /// a packet the server sent did not reach the client (UDP)
    Lost { id: u32, seq: u16 },
    /// a client is gone
    Disconnected { id: u32, reason: Reason },
}

/// what the server counted beyond its connections
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// CHALLENGE answers sent
    pub challenges: u64,
    /// connections taken
    pub accepted: u64,
    /// clients refused (full, protocol, address)
    pub denied: u64,
    /// packets that were no packet, a wrong kind or a wrong cookie or token
    pub bad: u64,
    /// handshakes over challenges_per_s, left without an answer
    pub throttled: u64,
    /// DATA from addresses without a connection
    pub unknown: u64,
}

struct Peer {
    conn: Conn,
    route: Route,
    nonce: u64,
    addr: Option<SocketAddr>,
}

/// the server's protocol (see the top of this file)
pub struct ServerCore {
    cfg: ServerConfig,
    secret: [u8; 32],
    tokens: u64,
    peers: BTreeMap<u32, Peer>,
    by_addr: BTreeMap<SocketAddr, u32>,
    by_stream: BTreeMap<u32, u32>,
    pending: BTreeMap<u32, (u64, Option<SocketAddr>)>,
    next_id: u32,
    events: VecDeque<ServerEvent>,
    out: VecDeque<Out>,
    answers: Bucket,
    stats: ServerStats,
}

impl ServerCore {
    /// a server with these limits; `secret` keys the cookies and tokens (random in a real server,
    /// fixed in tests)
    pub fn new(cfg: ServerConfig, secret: [u8; 32], now: u64) -> ServerCore {
        let mut cfg = cfg;
        cfg.max_payload = cfg.max_payload.clamp(1, MAX_UDP_PAYLOAD);
        cfg.max_stream_payload = cfg.max_stream_payload.max(1);
        ServerCore {
            cfg,
            secret,
            tokens: 0,
            peers: BTreeMap::new(),
            by_addr: BTreeMap::new(),
            by_stream: BTreeMap::new(),
            pending: BTreeMap::new(),
            next_id: 1,
            events: VecDeque::new(),
            out: VecDeque::new(),
            answers: Bucket::new(cfg.challenges_per_s, now),
            stats: ServerStats::default(),
        }
    }

    /// the limits in force (max_payload clamped)
    pub fn config(&self) -> &ServerConfig {
        &self.cfg
    }

    fn settings(&self) -> Settings {
        Settings { timeout_ms: self.cfg.timeout_ms, keepalive_ms: self.cfg.keepalive_ms, packets_per_s: self.cfg.packets_per_s, bytes_per_s: self.cfg.bytes_per_s }
    }

    fn cookie(&self, from: &SocketAddr, nonce: u64, time: u32) -> [u8; 16] {
        let mut d = [0u8; 34];
        d[..16].copy_from_slice(&ip16(&from.ip));
        d[16..18].copy_from_slice(&from.port.to_le_bytes());
        d[18..26].copy_from_slice(&nonce.to_le_bytes());
        d[26..30].copy_from_slice(&time.to_le_bytes());
        d[30..34].copy_from_slice(b"ck01");
        let h = crypto::hmac::hmac_sha256(&self.secret, &d);
        let mut m = [0u8; 16];
        m.copy_from_slice(&h[..16]);
        m
    }

    fn new_token(&mut self) -> u32 {
        loop {
            self.tokens += 1;
            let mut d = [0u8; 12];
            d[..8].copy_from_slice(&self.tokens.to_le_bytes());
            d[8..].copy_from_slice(b"tk01");
            let h = crypto::hmac::hmac_sha256(&self.secret, &d);
            let t = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
            if t != 0 {
                return t;
            }
        }
    }

    fn send_raw(&mut self, route: Route, p: Packet<'_>) {
        self.out.push_back(Out::Send(route, p.to_vec(false)));
    }

    fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    // take a peer's events into the server's
    fn drain(&mut self, id: u32) {
        let Some(p) = self.peers.get_mut(&id) else { return };
        while let Some(e) = p.conn.take_event() {
            self.events.push_back(match e {
                Event::Message { seq, data } => ServerEvent::Message { id, seq, data },
                Event::Delivered(seq) => ServerEvent::Delivered { id, seq },
                Event::Lost(seq) => ServerEvent::Lost { id, seq },
            });
        }
    }

    // a peer goes: its bye (UDP several times), its stream closed, the event
    fn remove(&mut self, id: u32, bye: Option<u8>, reason: Reason) {
        let Some(p) = self.peers.remove(&id) else { return };
        match p.route {
            Route::Udp(a) => {
                self.by_addr.remove(&a);
                if let Some(r) = bye {
                    for _ in 0..BYE_COPIES {
                        self.send_raw(p.route, Packet::Disconnect { token: p.conn.token(), reason: r });
                    }
                }
            }
            Route::Stream(s) => {
                self.by_stream.remove(&s);
                if let Some(r) = bye {
                    self.send_raw(p.route, Packet::Disconnect { token: 0, reason: r });
                }
                self.out.push_back(Out::Close(s));
            }
        }
        self.events.push_back(ServerEvent::Disconnected { id, reason });
    }

    fn count_ip(&self, ip: &IpAddr) -> usize {
        self.peers.values().filter(|p| p.addr.is_some_and(|a| a.ip == *ip)).count()
    }

    /// a UDP datagram from `from`
    pub fn datagram(&mut self, from: SocketAddr, data: &[u8], now: u64) {
        let Some(p) = packet::parse(data) else {
            self.stats.bad += 1;
            return;
        };
        match p {
            Packet::Connect { protocol, nonce } => {
                if data.len() < HANDSHAKE_SIZE {
                    self.stats.bad += 1;
                    return;
                }
                if !self.answers.take(1, now) {
                    self.stats.throttled += 1;
                    return;
                }
                if protocol != self.cfg.protocol {
                    self.stats.denied += 1;
                    self.send_raw(Route::Udp(from), Packet::Deny { nonce, reason: packet::DENY_PROTOCOL });
                    return;
                }
                let time = (now / 1_000_000) as u32;
                let mac = self.cookie(&from, nonce, time);
                self.stats.challenges += 1;
                self.send_raw(Route::Udp(from), Packet::Challenge { nonce, time, mac });
            }
            Packet::Response { protocol, nonce, time, mac } => {
                if data.len() < HANDSHAKE_SIZE || protocol != self.cfg.protocol {
                    self.stats.bad += 1;
                    return;
                }
                if let Some(&id) = self.by_addr.get(&from) {
                    if self.peers.get(&id).is_some_and(|p| p.nonce == nonce) {
                        // the ACCEPT was lost: once more
                        let (token, max) = (self.peers[&id].conn.token(), self.cfg.max_payload as u16);
                        self.send_raw(Route::Udp(from), Packet::Accept { nonce, token, max_payload: max });
                        return;
                    }
                }
                if !self.answers.take(1, now) {
                    self.stats.throttled += 1;
                    return;
                }
                let now_s = (now / 1_000_000) as u32;
                if time > now_s || now_s - time > COOKIE_S || !crypto::ct_eq(&mac, &self.cookie(&from, nonce, time)) {
                    self.stats.bad += 1;
                    return;
                }
                // a new handshake from an address with a connection: the client started again
                if let Some(&old) = self.by_addr.get(&from) {
                    self.remove(old, Some(packet::BYE_REPLACED), Reason::Replaced);
                }
                if self.peers.len() >= self.cfg.max_connections {
                    self.stats.denied += 1;
                    self.send_raw(Route::Udp(from), Packet::Deny { nonce, reason: packet::DENY_FULL });
                    return;
                }
                if self.count_ip(&from.ip) >= self.cfg.max_per_ip {
                    self.stats.denied += 1;
                    self.send_raw(Route::Udp(from), Packet::Deny { nonce, reason: packet::DENY_ADDRESS });
                    return;
                }
                let id = self.new_id();
                let token = self.new_token();
                let conn = Conn::new(token, false, self.settings(), now);
                self.peers.insert(id, Peer { conn, route: Route::Udp(from), nonce, addr: Some(from) });
                self.by_addr.insert(from, id);
                self.stats.accepted += 1;
                let max = self.cfg.max_payload as u16;
                self.send_raw(Route::Udp(from), Packet::Accept { nonce, token, max_payload: max });
                self.events.push_back(ServerEvent::Connected { id, transport: Transport::Udp, addr: Some(from) });
            }
            Packet::Data { token, seq, ack, bits, payload } => {
                let Some(&id) = self.by_addr.get(&from) else {
                    self.stats.unknown += 1;
                    return;
                };
                let max = self.cfg.max_payload;
                let Some(peer) = self.peers.get_mut(&id) else { return };
                if token != peer.conn.token() {
                    self.stats.bad += 1;
                    return;
                }
                if payload.len() > max {
                    peer.conn.count_dropped();
                    return;
                }
                if !peer.conn.rate_ok(data.len(), now) {
                    return;
                }
                peer.conn.on_data(seq, ack, bits, payload, now);
                if peer.conn.ack_due() {
                    let mut b = Vec::with_capacity(DATA_HEADER);
                    peer.conn.write_data(&[], now, &mut b);
                    self.out.push_back(Out::Send(Route::Udp(from), b));
                }
                self.drain(id);
            }
            Packet::Disconnect { token, reason } => {
                let Some(&id) = self.by_addr.get(&from) else { return };
                if self.peers.get(&id).is_some_and(|p| p.conn.token() == token) {
                    self.remove(id, None, Reason::Peer(reason));
                } else {
                    self.stats.bad += 1;
                }
            }
            _ => self.stats.bad += 1,
        }
    }

    /// a stream (WebSocket) opened; it must send CONNECT within handshake_ms
    pub fn stream_open(&mut self, stream: u32, addr: Option<SocketAddr>, now: u64) {
        self.pending.insert(stream, (now, addr));
    }

    /// a message on a stream
    pub fn stream_message(&mut self, stream: u32, data: &[u8], now: u64) {
        if let Some(&id) = self.by_stream.get(&stream) {
            match packet::parse(data) {
                Some(Packet::Data { seq, ack, bits, payload, .. }) => {
                    let max = self.cfg.max_stream_payload;
                    let Some(peer) = self.peers.get_mut(&id) else { return };
                    if payload.len() > max {
                        self.remove(id, Some(packet::BYE_PROTOCOL), Reason::Protocol);
                        return;
                    }
                    if !peer.conn.rate_ok(data.len(), now) {
                        self.remove(id, Some(packet::BYE_RATE), Reason::RateLimit);
                        return;
                    }
                    peer.conn.on_data(seq, ack, bits, payload, now);
                    self.drain(id);
                }
                Some(Packet::Disconnect { reason, .. }) => self.remove(id, None, Reason::Peer(reason)),
                _ => {
                    self.stats.bad += 1;
                    self.remove(id, Some(packet::BYE_PROTOCOL), Reason::Protocol);
                }
            }
            return;
        }
        let Some((_, addr)) = self.pending.remove(&stream) else { return };
        let Some(Packet::Connect { protocol, nonce }) = packet::parse(data) else {
            self.stats.bad += 1;
            self.out.push_back(Out::Close(stream));
            return;
        };
        let deny = if protocol != self.cfg.protocol {
            Some(packet::DENY_PROTOCOL)
        } else if self.peers.len() >= self.cfg.max_connections {
            Some(packet::DENY_FULL)
        } else if addr.is_some_and(|a| self.count_ip(&a.ip) >= self.cfg.max_per_ip) {
            Some(packet::DENY_ADDRESS)
        } else {
            None
        };
        if let Some(reason) = deny {
            self.stats.denied += 1;
            self.send_raw(Route::Stream(stream), Packet::Deny { nonce, reason });
            self.out.push_back(Out::Close(stream));
            return;
        }
        let id = self.new_id();
        let conn = Conn::new(0, true, self.settings(), now);
        self.peers.insert(id, Peer { conn, route: Route::Stream(stream), nonce, addr });
        self.by_stream.insert(stream, id);
        self.stats.accepted += 1;
        let max = self.cfg.max_stream_payload.min(u16::MAX as usize) as u16;
        self.send_raw(Route::Stream(stream), Packet::Accept { nonce, token: 0, max_payload: max });
        self.events.push_back(ServerEvent::Connected { id, transport: Transport::WebSocket, addr });
    }

    /// a stream ended (`code`: its close code, 1006 without one)
    pub fn stream_closed(&mut self, stream: u32, code: u16) {
        self.pending.remove(&stream);
        if let Some(&id) = self.by_stream.get(&stream) {
            self.peers.remove(&id);
            self.by_stream.remove(&stream);
            self.events.push_back(ServerEvent::Disconnected { id, reason: Reason::Closed(code) });
        }
    }

    /// keep-alives, timeouts, streams without a CONNECT
    pub fn tick(&mut self, now: u64) {
        let ids: Vec<u32> = self.peers.keys().copied().collect();
        for id in ids {
            let Some(p) = self.peers.get_mut(&id) else { continue };
            if p.conn.timed_out(now) {
                self.remove(id, Some(packet::BYE_TIMEOUT), Reason::Timeout);
            } else if p.conn.keepalive_due(now) {
                let mut b = Vec::with_capacity(DATA_HEADER);
                p.conn.write_data(&[], now, &mut b);
                let route = p.route;
                self.out.push_back(Out::Send(route, b));
                self.drain(id);
            }
        }
        let limit = self.cfg.handshake_ms as u64 * 1000;
        let late: Vec<u32> = self.pending.iter().filter(|(_, (t, _))| now.saturating_sub(*t) >= limit).map(|(s, _)| *s).collect();
        for s in late {
            self.pending.remove(&s);
            self.out.push_back(Out::Close(s));
        }
    }

    /// send a message to a client -> its sequence number (Size: empty or over the limit of its
    /// transport; NotConnected: no such client)
    pub fn send(&mut self, id: u32, payload: &[u8], now: u64) -> Result<u16, Error> {
        let (max_udp, max_stream) = (self.cfg.max_payload, self.cfg.max_stream_payload);
        let p = self.peers.get_mut(&id).ok_or(Error::NotConnected)?;
        let max = if matches!(p.route, Route::Udp(_)) { max_udp } else { max_stream };
        if payload.is_empty() || payload.len() > max {
            return Err(Error::Size);
        }
        let mut b = Vec::with_capacity(DATA_HEADER + payload.len());
        let seq = p.conn.write_data(payload, now, &mut b);
        let route = p.route;
        self.out.push_back(Out::Send(route, b));
        self.drain(id);
        Ok(seq)
    }

    /// end a client's connection (DISCONNECT goes out, Disconnected(Local) follows)
    pub fn disconnect(&mut self, id: u32, reason: u8) {
        self.remove(id, Some(reason), Reason::Local);
    }

    /// end every connection (the server stops)
    pub fn disconnect_all(&mut self) {
        let ids: Vec<u32> = self.peers.keys().copied().collect();
        for id in ids {
            self.remove(id, Some(packet::BYE_SHUTDOWN), Reason::Local);
        }
    }

    /// the next thing to send or close
    pub fn take_out(&mut self) -> Option<Out> {
        self.out.pop_front()
    }

    /// the next event
    pub fn poll_event(&mut self) -> Option<ServerEvent> {
        self.events.pop_front()
    }

    /// connections now
    pub fn connections(&self) -> usize {
        self.peers.len()
    }

    /// the ids of the connections
    pub fn ids(&self) -> Vec<u32> {
        self.peers.keys().copied().collect()
    }

    /// a connection's counts and round-trip time
    pub fn stats(&self, id: u32) -> Option<Stats> {
        self.peers.get(&id).map(|p| p.conn.stats())
    }

    /// a connection's transport and address
    pub fn peer(&self, id: u32) -> Option<(Transport, Option<SocketAddr>)> {
        self.peers.get(&id).map(|p| (if matches!(p.route, Route::Udp(_)) { Transport::Udp } else { Transport::WebSocket }, p.addr))
    }

    /// where a connection's packets go
    pub fn route(&self, id: u32) -> Option<Route> {
        self.peers.get(&id).map(|p| p.route)
    }

    /// what the server counted beyond its connections
    pub fn server_stats(&self) -> ServerStats {
        self.stats
    }
}

/// an address as 16 bytes (IPv4 mapped)
fn ip16(ip: &IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(a) => [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a[0], a[1], a[2], a[3]],
        IpAddr::V6(a) => *a,
    }
}

// ------------------------------------------------------------------ the client

/// a client's limits and timing
#[derive(Clone, Copy, Debug)]
pub struct ClientConfig {
    /// the app's protocol id (the server's must be the same)
    pub protocol: u32,
    /// the largest payload this side takes from the server (UDP at most MAX_UDP_PAYLOAD)
    pub max_payload: usize,
    /// nothing received for this long ends the connection
    pub timeout_ms: u32,
    /// an empty DATA packet after this long without sending
    pub keepalive_ms: u32,
    /// the handshake within this long
    pub connect_ms: u32,
    /// UDP: CONNECT / RESPONSE again after this long without an answer
    pub resend_ms: u32,
    /// packets a second from the server, on average (more are dropped)
    pub packets_per_s: u32,
    /// bytes a second from the server, on average (more are dropped)
    pub bytes_per_s: u32,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            protocol: 0,
            max_payload: 64 * 1024,
            timeout_ms: 5000,
            keepalive_ms: 100,
            connect_ms: 5000,
            resend_ms: 100,
            packets_per_s: 4000,
            bytes_per_s: 16 << 20,
        }
    }
}

/// where a client stands
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// the handshake is on its way
    Connecting,
    /// the server took the connection
    Connected,
    /// ended, and why
    Disconnected(Reason),
}

/// what happened on the client
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    /// the server took the connection
    Connected,
    /// a message of the server
    Message { seq: u16, data: Vec<u8> },
    /// the server has a packet this side sent
    Delivered(u16),
    /// a packet this side sent did not reach the server (UDP)
    Lost(u16),
    /// the connection ended
    Disconnected(Reason),
}

enum Phase {
    Idle,
    Connecting { started: u64, last: u64, cookie: Option<(u32, [u8; 16])> },
    Connected { conn: Conn, max_send: usize },
    Done(Reason),
}

/// the client's protocol (see the top of this file)
pub struct ClientCore {
    cfg: ClientConfig,
    nonce: u64,
    stream: bool,
    phase: Phase,
    out: VecDeque<Vec<u8>>,
    events: VecDeque<ClientEvent>,
}

impl ClientCore {
    /// a client with a random `nonce` (it names this attempt); `stream`: over a WebSocket
    pub fn new(cfg: ClientConfig, nonce: u64, stream: bool) -> ClientCore {
        let mut cfg = cfg;
        if !stream {
            cfg.max_payload = cfg.max_payload.clamp(1, MAX_UDP_PAYLOAD);
        }
        ClientCore { cfg, nonce, stream, phase: Phase::Idle, out: VecDeque::new(), events: VecDeque::new() }
    }

    fn connect_packet(&self) -> Vec<u8> {
        Packet::Connect { protocol: self.cfg.protocol, nonce: self.nonce }.to_vec(!self.stream)
    }

    /// begin the handshake (CONNECT goes out)
    pub fn start(&mut self, now: u64) {
        if matches!(self.phase, Phase::Idle) {
            self.out.push_back(self.connect_packet());
            self.phase = Phase::Connecting { started: now, last: now, cookie: None };
        }
    }

    fn end(&mut self, reason: Reason) {
        if !matches!(self.phase, Phase::Done(_)) {
            self.phase = Phase::Done(reason.clone());
            self.events.push_back(ClientEvent::Disconnected(reason));
        }
    }

    fn drain(&mut self) {
        let Phase::Connected { conn, .. } = &mut self.phase else { return };
        while let Some(e) = conn.take_event() {
            self.events.push_back(match e {
                Event::Message { seq, data } => ClientEvent::Message { seq, data },
                Event::Delivered(s) => ClientEvent::Delivered(s),
                Event::Lost(s) => ClientEvent::Lost(s),
            });
        }
    }

    /// a packet from the server (the caller checked that it came from the server)
    pub fn packet(&mut self, data: &[u8], now: u64) {
        let Some(p) = packet::parse(data) else { return };
        let nonce = self.nonce;
        match (&mut self.phase, p) {
            (Phase::Connecting { last, cookie, .. }, Packet::Challenge { nonce: n, time, mac }) if n == nonce && !self.stream => {
                *cookie = Some((time, mac));
                *last = now;
                let r = Packet::Response { protocol: self.cfg.protocol, nonce, time, mac }.to_vec(true);
                self.out.push_back(r);
            }
            (Phase::Connecting { .. }, Packet::Accept { nonce: n, token, max_payload }) if n == nonce => {
                let set = Settings { timeout_ms: self.cfg.timeout_ms, keepalive_ms: self.cfg.keepalive_ms, packets_per_s: self.cfg.packets_per_s, bytes_per_s: self.cfg.bytes_per_s };
                let cap = if self.stream { usize::MAX } else { MAX_UDP_PAYLOAD };
                self.phase = Phase::Connected { conn: Conn::new(token, self.stream, set, now), max_send: (max_payload as usize).clamp(1, cap) };
                self.events.push_back(ClientEvent::Connected);
            }
            (Phase::Connecting { .. }, Packet::Deny { nonce: n, reason }) if n == nonce => {
                self.end(if reason == packet::DENY_PROTOCOL { Reason::Protocol } else { Reason::Full });
            }
            (Phase::Connected { conn, .. }, Packet::Data { token, seq, ack, bits, payload }) if token == conn.token() => {
                if payload.len() > self.cfg.max_payload || !conn.rate_ok(data.len(), now) {
                    if payload.len() > self.cfg.max_payload {
                        conn.count_dropped();
                    }
                    return;
                }
                conn.on_data(seq, ack, bits, payload, now);
                if conn.ack_due() {
                    let mut b = Vec::with_capacity(DATA_HEADER);
                    conn.write_data(&[], now, &mut b);
                    self.out.push_back(b);
                }
                self.drain();
            }
            (Phase::Connected { conn, .. }, Packet::Disconnect { token, reason }) if token == conn.token() => self.end(Reason::Peer(reason)),
            _ => {}
        }
    }

    /// resends of the handshake, keep-alives, timeouts
    pub fn tick(&mut self, now: u64) {
        match &mut self.phase {
            Phase::Connecting { started, last, cookie } => {
                if now.saturating_sub(*started) >= self.cfg.connect_ms as u64 * 1000 {
                    self.end(Reason::ConnectTimeout);
                } else if !self.stream && now.saturating_sub(*last) >= self.cfg.resend_ms as u64 * 1000 {
                    *last = now;
                    let p = match cookie {
                        Some((time, mac)) => Packet::Response { protocol: self.cfg.protocol, nonce: self.nonce, time: *time, mac: *mac }.to_vec(true),
                        None => Packet::Connect { protocol: self.cfg.protocol, nonce: self.nonce }.to_vec(true),
                    };
                    self.out.push_back(p);
                }
            }
            Phase::Connected { conn, .. } => {
                if conn.timed_out(now) {
                    self.end(Reason::Timeout);
                } else if conn.keepalive_due(now) {
                    let mut b = Vec::with_capacity(DATA_HEADER);
                    conn.write_data(&[], now, &mut b);
                    self.out.push_back(b);
                    self.drain();
                }
            }
            _ => {}
        }
    }

    /// send a message -> its sequence number (Size: empty or over the server's limit;
    /// NotConnected before Connected and after the end)
    pub fn send(&mut self, payload: &[u8], now: u64) -> Result<u16, Error> {
        let Phase::Connected { conn, max_send } = &mut self.phase else { return Err(Error::NotConnected) };
        if payload.is_empty() || payload.len() > *max_send {
            return Err(Error::Size);
        }
        let mut b = Vec::with_capacity(DATA_HEADER + payload.len());
        let seq = conn.write_data(payload, now, &mut b);
        self.out.push_back(b);
        self.drain();
        Ok(seq)
    }

    /// end the connection (DISCONNECT goes out; UDP several times)
    pub fn disconnect(&mut self) {
        if let Phase::Connected { conn, .. } = &self.phase {
            let b = Packet::Disconnect { token: conn.token(), reason: packet::BYE_NORMAL }.to_vec(false);
            for _ in 0..if self.stream { 1 } else { BYE_COPIES } {
                self.out.push_back(b.clone());
            }
        }
        self.end(Reason::Local);
    }

    /// the transport under a stream ended (the WebSocket closed with `code`)
    pub fn transport_closed(&mut self, code: u16) {
        self.end(Reason::Closed(code));
    }

    /// the next packet to send to the server
    pub fn take_out(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }

    /// the next event
    pub fn poll_event(&mut self) -> Option<ClientEvent> {
        self.events.pop_front()
    }

    /// where it stands
    pub fn state(&self) -> State {
        match &self.phase {
            Phase::Idle | Phase::Connecting { .. } => State::Connecting,
            Phase::Connected { .. } => State::Connected,
            Phase::Done(r) => State::Disconnected(r.clone()),
        }
    }

    /// the largest payload the server takes (0 before Connected)
    pub fn max_payload(&self) -> usize {
        match &self.phase {
            Phase::Connected { max_send, .. } => *max_send,
            _ => 0,
        }
    }

    /// whether the transport is reliable (a WebSocket)
    pub fn reliable(&self) -> bool {
        self.stream
    }

    /// the connection's counts (zeros before Connected)
    pub fn stats(&self) -> Stats {
        match &self.phase {
            Phase::Connected { conn, .. } => conn.stats(),
            _ => Stats::default(),
        }
    }
}
