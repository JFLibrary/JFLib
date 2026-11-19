// sim -- a simulated network in the process: netchan's protocol (endpoint::ServerCore, ClientCore) on a
// virtual clock, with delay, jitter, loss, duplicates and reordering per peer, from a seed
//
//   Weather        what the network does to a packet: loss, duplicates, a chance of extra delay (packets
//                  overtake each other), the delay and its jitter; CLEAR, TYPICAL, ROUGH, STORM
//   SimNet         one server and its peers; a peer has a main channel (UDP or a WebSocket) and maybe a
//                  fetch's WebSocket.  UDP datagrams meet the weather; a WebSocket is reliable and ordered
//                  (a loss is a retransmission: two delays and 20 ms later, and nothing behind it overtakes)
//   step(dt_us)    the clock moves on, what is due arrives, the cores tick, what they send goes out
//
// Nothing here reads a clock or a socket: the same seed gives the same packets on every target, so a whole
// networked session can be checked by a checksum (apps/gametest --netsum).  The template was apps/nettest's
// simulation of G7d.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use netchan::endpoint::{ClientConfig, ClientCore, ClientEvent, Out, Route, ServerConfig, ServerCore, ServerEvent, State};
use netchan::Error;
use plat::net::{IpAddr, SocketAddr};

/// what the network does to a packet
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    /// the chance a datagram is lost (on a WebSocket: retransmitted)
    pub loss: f64,
    /// the chance a datagram comes twice
    pub dup: f64,
    /// the chance of an extra delay of up to extra_us (packets overtake each other)
    pub reorder: f64,
    /// see reorder
    pub extra_us: u64,
    /// the delay one way
    pub delay_us: u64,
    /// up to this much more, evenly spread
    pub jitter_us: u64,
}

impl Weather {
    /// 20 ms each way, nothing else
    pub const CLEAR: Weather = Weather { loss: 0.0, dup: 0.0, reorder: 0.0, extra_us: 0, delay_us: 20_000, jitter_us: 0 };
    /// a good home connection: 30 ms +- 5 each way, 1 % loss, 2 % a little later
    pub const TYPICAL: Weather = Weather { loss: 0.01, dup: 0.0, reorder: 0.02, extra_us: 10_000, delay_us: 30_000, jitter_us: 5_000 };
    /// nettest's rough weather: 20 % loss, 10 % twice, 20 % up to 30 ms later, 20 ms +- 5
    pub const ROUGH: Weather = Weather { loss: 0.2, dup: 0.1, reorder: 0.2, extra_us: 30_000, delay_us: 20_000, jitter_us: 5_000 };
    /// nettest's storm: 50 % loss, 20 % twice, 30 % up to 60 ms later, 40 ms +- 20
    pub const STORM: Weather = Weather { loss: 0.5, dup: 0.2, reorder: 0.3, extra_us: 60_000, delay_us: 40_000, jitter_us: 20_000 };
}

/// which of a peer's links
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// the main channel
    Main,
    /// a fetch's WebSocket
    Fetch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Udp(SocketAddr),
    Stream(u32),
}

/// one link of a peer: a netchan client core and how it reaches the server
pub struct Link {
    /// netchan's client protocol
    pub core: ClientCore,
    kind: Kind,
    up_at: u64,
    down_at: u64,
    closed: bool,
}

impl Link {
    /// a reliable stream (a WebSocket)
    pub fn reliable(&self) -> bool {
        matches!(self.kind, Kind::Stream(_))
    }

    /// the stream ended or the core disconnected
    pub fn closed(&self) -> bool {
        self.closed || matches!(self.core.state(), State::Disconnected(_))
    }
}

/// what went over a peer's links (netchan's packets, without IP and UDP headers)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerStats {
    /// bytes to the server
    pub up_bytes: u64,
    /// packets to the server
    pub up_packets: u64,
    /// bytes from the server
    pub down_bytes: u64,
    /// packets from the server
    pub down_packets: u64,
}

/// a client machine on the network
pub struct Peer {
    /// what its network does
    pub weather: Weather,
    /// the main channel
    pub main: Option<Link>,
    /// a fetch's WebSocket
    pub fetch: Option<Link>,
    /// what went over its links
    pub stats: PeerStats,
    addr: SocketAddr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    ServerUdp(SocketAddr),
    ServerStream(u32),
    ServerStreamClosed(u32),
    Peer(usize, Which),
    PeerClosed(usize, Which),
    PeerStart(usize, Which),
}

/// a server and its peers on one simulated network (see the module header)
pub struct SimNet {
    /// the virtual clock (microseconds)
    pub now_us: u64,
    /// netchan's server protocol
    pub server: ServerCore,
    /// the peers
    pub peers: Vec<Peer>,
    /// the network carries nothing (a cut cable)
    pub down: bool,
    rng: u64,
    q: Vec<(u64, u64, Dest, Vec<u8>)>,
    order: u64,
    next_stream: u32,
    by_addr: BTreeMap<SocketAddr, usize>,
    by_stream: BTreeMap<u32, (usize, Which)>,
}

fn peer_addr(i: usize) -> SocketAddr {
    SocketAddr::new(IpAddr::V4([10, 1 + (i / 250) as u8, 0, (i % 250) as u8 + 1]), 40000 + (i % 20000) as u16)
}

impl SimNet {
    /// a server with these limits; `seed` drives the network and the server's secret
    pub fn new(seed: u64, cfg: ServerConfig) -> SimNet {
        let mut secret = [0u8; 32];
        secret[..8].copy_from_slice(&seed.to_le_bytes());
        let now = 1_000_000_000;
        SimNet {
            now_us: now,
            server: ServerCore::new(cfg, secret, now),
            peers: Vec::new(),
            down: false,
            rng: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            q: Vec::new(),
            order: 0,
            next_stream: 1,
            by_addr: BTreeMap::new(),
            by_stream: BTreeMap::new(),
        }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && ((self.rand() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)) < p
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.rand() % n }
    }

    /// a peer whose main channel is UDP -> its index (the handshake starts now)
    pub fn add_udp(&mut self, cfg: ClientConfig, w: Weather) -> usize {
        let i = self.peers.len();
        let addr = peer_addr(i);
        let mut core = ClientCore::new(cfg, self.rand(), false);
        core.start(self.now_us);
        self.peers.push(Peer { weather: w, main: Some(Link { core, kind: Kind::Udp(addr), up_at: 0, down_at: 0, closed: false }), fetch: None, stats: PeerStats::default(), addr });
        self.by_addr.insert(addr, i);
        self.flush();
        i
    }

    fn open_stream(&mut self, i: usize, which: Which, cfg: ClientConfig) -> Link {
        let s = self.next_stream;
        self.next_stream += 1;
        self.by_stream.insert(s, (i, which));
        let addr = self.peers.get(i).map_or_else(|| peer_addr(i), |p| p.addr);
        self.server.stream_open(s, Some(addr), self.now_us);
        // the WebSocket is open after a round trip (TCP) and another (the upgrade): then CONNECT
        let w = self.peers.get(i).map_or(Weather::CLEAR, |p| p.weather);
        let at = self.now_us + 4 * w.delay_us;
        self.order += 1;
        self.q.push((at, self.order, Dest::PeerStart(i, which), Vec::new()));
        Link { core: ClientCore::new(cfg, self.rand(), true), kind: Kind::Stream(s), up_at: 0, down_at: 0, closed: false }
    }

    /// a peer whose main channel is a WebSocket -> its index
    pub fn add_ws(&mut self, cfg: ClientConfig, w: Weather) -> usize {
        let i = self.peers.len();
        let addr = peer_addr(i);
        self.peers.push(Peer { weather: w, main: None, fetch: None, stats: PeerStats::default(), addr });
        let l = self.open_stream(i, Which::Main, cfg);
        self.peers[i].main = Some(l);
        i
    }

    /// a fetch's WebSocket for peer `i` (replacing an old one)
    pub fn open_fetch(&mut self, i: usize, cfg: ClientConfig) {
        if i >= self.peers.len() {
            return;
        }
        let l = self.open_stream(i, Which::Fetch, cfg);
        self.peers[i].fetch = Some(l);
    }

    /// the peer's network from now on
    pub fn set_weather(&mut self, i: usize, w: Weather) {
        if let Some(p) = self.peers.get_mut(i) {
            p.weather = w;
        }
    }

    fn link(&mut self, i: usize, which: Which) -> Option<&mut Link> {
        let p = self.peers.get_mut(i)?;
        match which {
            Which::Main => p.main.as_mut(),
            Which::Fetch => p.fetch.as_mut(),
        }
    }

    // a packet into the network
    fn push(&mut self, d: Dest, b: Vec<u8>, w: Weather, stream: Option<(usize, Which, bool)>) {
        if self.down {
            return;
        }
        match stream {
            Some((i, which, up)) => {
                // reliable and ordered: a loss is a retransmission, nothing overtakes
                let mut at = self.now_us + w.delay_us + self.below(w.jitter_us);
                if self.chance(w.loss) {
                    at += 2 * w.delay_us + 20_000;
                }
                let Some(l) = self.link(i, which) else { return };
                let last = if up { &mut l.up_at } else { &mut l.down_at };
                at = at.max(*last);
                *last = at;
                self.order += 1;
                self.q.push((at, self.order, d, b));
            }
            None => {
                if self.chance(w.loss) {
                    return;
                }
                let copies = if self.chance(w.dup) { 2 } else { 1 };
                for _ in 0..copies {
                    let mut at = self.now_us + w.delay_us + self.below(w.jitter_us);
                    if self.chance(w.reorder) {
                        at += self.below(w.extra_us);
                    }
                    self.order += 1;
                    self.q.push((at, self.order, d, b.clone()));
                }
            }
        }
    }

    /// hand what the cores want sent to the network
    pub fn flush(&mut self) {
        while let Some(o) = self.server.take_out() {
            match o {
                Out::Send(Route::Udp(a), b) => {
                    if let Some(&i) = self.by_addr.get(&a) {
                        let w = self.peers[i].weather;
                        self.peers[i].stats.down_bytes += b.len() as u64;
                        self.peers[i].stats.down_packets += 1;
                        self.push(Dest::Peer(i, Which::Main), b, w, None);
                    }
                }
                Out::Send(Route::Stream(s), b) => {
                    if let Some(&(i, which)) = self.by_stream.get(&s) {
                        let w = self.peers[i].weather;
                        self.peers[i].stats.down_bytes += b.len() as u64;
                        self.peers[i].stats.down_packets += 1;
                        self.push(Dest::Peer(i, which), b, w, Some((i, which, false)));
                    }
                }
                Out::Close(s) => {
                    if let Some(&(i, which)) = self.by_stream.get(&s) {
                        let w = self.peers[i].weather;
                        self.push(Dest::PeerClosed(i, which), Vec::new(), w, Some((i, which, false)));
                    }
                }
            }
        }
        for i in 0..self.peers.len() {
            for which in [Which::Main, Which::Fetch] {
                loop {
                    let Some(l) = self.link(i, which) else { break };
                    let kind = l.kind;
                    let Some(b) = l.core.take_out() else { break };
                    let w = self.peers[i].weather;
                    self.peers[i].stats.up_bytes += b.len() as u64;
                    self.peers[i].stats.up_packets += 1;
                    match kind {
                        Kind::Udp(a) => self.push(Dest::ServerUdp(a), b, w, None),
                        Kind::Stream(s) => self.push(Dest::ServerStream(s), b, w, Some((i, which, true))),
                    }
                }
            }
        }
    }

    /// the clock moves on by dt_us: what is due arrives, the cores tick, what they send goes out
    pub fn step(&mut self, dt_us: u64) {
        self.now_us += dt_us;
        let now = self.now_us;
        let mut due: Vec<(u64, u64, Dest, Vec<u8>)> = Vec::new();
        let mut k = 0;
        while k < self.q.len() {
            if self.q[k].0 <= now {
                due.push(self.q.swap_remove(k));
            } else {
                k += 1;
            }
        }
        due.sort_by_key(|x| (x.0, x.1));
        for (_, _, d, b) in due {
            match d {
                Dest::ServerUdp(from) => self.server.datagram(from, &b, now),
                Dest::ServerStream(s) => self.server.stream_message(s, &b, now),
                Dest::ServerStreamClosed(s) => self.server.stream_closed(s, 1000),
                Dest::Peer(i, which) => {
                    if let Some(l) = self.link(i, which) {
                        if !l.closed {
                            l.core.packet(&b, now);
                        }
                    }
                }
                Dest::PeerClosed(i, which) => {
                    if let Some(l) = self.link(i, which) {
                        l.closed = true;
                        l.core.transport_closed(1000);
                    }
                }
                Dest::PeerStart(i, which) => {
                    if let Some(l) = self.link(i, which) {
                        l.core.start(now);
                    }
                }
            }
        }
        self.server.tick(now);
        for p in self.peers.iter_mut() {
            for l in [p.main.as_mut(), p.fetch.as_mut()].into_iter().flatten() {
                l.core.tick(now);
            }
        }
        self.flush();
    }

    /// the server's events since the last call
    pub fn server_events(&mut self, out: &mut Vec<ServerEvent>) {
        out.clear();
        while let Some(e) = self.server.poll_event() {
            out.push(e);
        }
    }

    /// a link's events since the last call
    pub fn link_events(&mut self, i: usize, which: Which, out: &mut Vec<ClientEvent>) {
        out.clear();
        if let Some(l) = self.link(i, which) {
            while let Some(e) = l.core.poll_event() {
                out.push(e);
            }
        }
    }

    /// the server sends to a connection
    pub fn server_send(&mut self, id: u32, data: &[u8]) -> Result<u16, Error> {
        let r = self.server.send(id, data, self.now_us);
        self.flush();
        r
    }

    /// the server ends a connection
    pub fn server_disconnect(&mut self, id: u32) {
        self.server.disconnect(id, netchan::packet::BYE_NORMAL);
        self.flush();
    }

    /// a link sends to the server
    pub fn link_send(&mut self, i: usize, which: Which, data: &[u8]) -> Result<u16, Error> {
        let now = self.now_us;
        let r = match self.link(i, which) {
            Some(l) => l.core.send(data, now),
            None => Err(Error::NotConnected),
        };
        self.flush();
        r
    }

    /// a link ends (DISCONNECT goes out; a WebSocket's stream closes behind it)
    pub fn link_close(&mut self, i: usize, which: Which) {
        let Some(l) = self.link(i, which) else { return };
        l.core.disconnect();
        let kind = l.kind;
        l.closed = true;
        self.flush();
        if let Kind::Stream(s) = kind {
            let w = self.peers[i].weather;
            self.push(Dest::ServerStreamClosed(s), Vec::new(), w, Some((i, which, true)));
        }
    }

    /// a link's netchan state
    pub fn link_state(&self, i: usize, which: Which) -> Option<State> {
        let p = self.peers.get(i)?;
        let l = match which {
            Which::Main => p.main.as_ref(),
            Which::Fetch => p.fetch.as_ref(),
        }?;
        Some(l.core.state())
    }

    /// packets on their way
    pub fn in_flight(&self) -> usize {
        self.q.len()
    }
}
