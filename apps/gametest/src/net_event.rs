// net_event -- the netcode's part 2 (G7e): reliable events, leaving, smoothed corrections, rooms on threads, room
// codes from the system's random numbers
//
//   events    the outbox and inbox against a model channel written here (loss, duplicates, reordering, lost
//             acknowledgements, the u16 numbers wrapping): every event exactly once and in order, nothing left over;
//             then sessions on the simulated network with an arena whose room sends events to all, to one and to
//             all but one and echoes the clients' events, in clear, rough and stormy weather over UDP and WebSocket:
//             each client gets exactly the events meant for it since it joined, once and in order, and the room each
//             client's events once and in order; no snapshot over 1200 bytes; a client that never acknowledges is
//             dropped when its outbox is full
//   leave     LEAVE removes the figure in the tick after it came and the others see it go (UDP and WebSocket),
//             with the LEAVE lost netchan's DISCONNECT does it, a client that goes silent only after the timeout
//   smooth    the Smoother's bounds (the offset takes the jump back, never grows without a correction, never over
//             `snap`, falls below a millimetre in time, nearly the same for any frame rate); in rough sessions the
//             drawn position does not move at a correction, and after the weather the predictions are exact
//   threads   several rooms on the simulated network, side by side on the pool and one after another: every message
//             of the server bit for bit the same; seeded, secure and given room codes and tickets
//
// The references: the room's own log of what it sent to whom (the arena below), the clients' logs of what they sent,
// the server's states per tick (prediction), the messages of the single-threaded run.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::arena::{self, Arena, ArenaConfig};
use game::net::bot::{remote_feet, Bot, BotConfig};
use game::net::client::{ClientOptions, NetClient, Phase, Remote};
use game::net::event::{Inbox, NetEvent, Outbox, MAX_PENDING};
use game::net::input::NetInput;
use game::net::predict::Smoother;
use game::net::proto;
use game::net::server::{NetServer, RoomEvent, RoomFactory, RoomSim, ServerOptions, To};
use game::net::sim::{SimNet, Weather, Which};
use game::net::snapshot::{tick_diff, Ent, Schema, Snapshot};
use game::net::walker::{WalkState, Walker};
use game::net::RoomCode;
use game::Vec3;
use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};
use netchan::Transport;

use crate::net_session::PROTOCOL;
use crate::{Ctx, Rng};

// ---------------------------------------------------------------- an arena that talks

/// the arena with events: every `every` ticks one to all (channel 1), every 7 ticks one to a player in turn
/// (channel 2), every 11 ticks one to all but a player (channel 3), now and then a large one (channel 6, 200 bytes);
/// a player's event comes back to it (channel 4); everything it sent and got is logged
pub struct TalkArena {
    pub a: Arena,
    pub every: u32,
    pub players: Vec<u16>,
    /// (tick, recipients, the event's channel and bytes) in the order sent
    pub sent: Vec<(u32, Vec<u16>, u8, Vec<u8>)>,
    /// (player, event) in the order handed to the room
    pub got: Vec<(u16, NetEvent)>,
    out: Vec<RoomEvent>,
    n: u32,
    turn: usize,
}

impl TalkArena {
    pub fn new(cfg: ArenaConfig, every: u32) -> TalkArena {
        TalkArena { a: Arena::new(cfg), every, players: Vec::new(), sent: Vec::new(), got: Vec::new(), out: Vec::new(), n: 0, turn: 0 }
    }

    fn emit(&mut self, tick: u32, to: To, channel: u8, len: usize) {
        self.n += 1;
        let mut data = Vec::with_capacity(len.max(8));
        data.extend_from_slice(&tick.to_le_bytes());
        data.extend_from_slice(&self.n.to_le_bytes());
        while data.len() < len {
            data.push((data.len() as u32 * 7 + self.n) as u8);
        }
        let who: Vec<u16> = self.players.iter().copied().filter(|p| to.includes(*p)).collect();
        self.sent.push((tick, who, channel, data.clone()));
        self.out.push(RoomEvent { to, channel, data });
    }
}

impl RoomSim for TalkArena {
    fn schema(&self) -> &Schema {
        self.a.schema()
    }
    fn baseline(&self) -> &Snapshot {
        self.a.baseline()
    }
    fn join(&mut self, name: &str) -> Option<u16> {
        let id = self.a.join(name)?;
        self.players.push(id);
        Some(id)
    }
    fn leave(&mut self, player: u16) {
        self.a.leave(player);
        self.players.retain(|p| *p != player);
    }
    fn step(&mut self, tick: u32, dt: f64, inputs: &[(u16, NetInput, bool)]) {
        self.a.step(tick, dt, inputs);
        if self.players.is_empty() {
            return;
        }
        if tick % self.every == 0 {
            self.emit(tick, To::All, 1, 8);
        }
        if tick % 7 == 0 {
            self.turn = (self.turn + 1) % self.players.len();
            let p = self.players[self.turn];
            self.emit(tick, To::Player(p), 2, 12);
        }
        if tick % 11 == 0 {
            let p = self.players[(self.turn + 1) % self.players.len()];
            self.emit(tick, To::AllBut(p), 3, 9);
        }
        if tick % 97 == 0 {
            self.emit(tick, To::All, 6, 200);
        }
    }
    fn snapshot(&self, tick: u32, out: &mut Snapshot) {
        self.a.snapshot(tick, out)
    }
    fn priority(&self, viewer: u16, e: &Ent) -> u32 {
        self.a.priority(viewer, e)
    }
    fn on_event(&mut self, player: u16, e: &NetEvent) {
        self.got.push((player, e.clone()));
        self.out.push(RoomEvent { to: To::Player(player), channel: 4, data: e.data.clone() });
    }
    fn take_events(&mut self, out: &mut Vec<RoomEvent>) {
        out.append(&mut self.out);
    }
}

// ---------------------------------------------------------------- a harness for any room

/// a client of the rig
pub struct Cl {
    pub peer: usize,
    pub nc: NetClient<Walker>,
    pub ws: bool,
    pub weather: Weather,
    /// the room to join: None makes one; Some(k): the room of client k
    pub join: Option<usize>,
    pub started: bool,
    pub start_us: u64,
    pub bot: Bot,
    /// the events it sent (in order) and got (in order)
    pub sent: Vec<NetEvent>,
    pub got: Vec<NetEvent>,
    /// its first prediction of every tick
    pub first: BTreeMap<u32, WalkState>,
    /// it called leave (the harness then ends its link)
    pub leaving: bool,
    rem: Vec<Remote>,
    feet: Vec<(u16, Vec3)>,
}

/// a server with rooms of S and clients on the simulated network
pub struct Rig<S: RoomSim> {
    pub net: SimNet,
    pub server: NetServer<S>,
    pub cl: Vec<Cl>,
    pub map: physics::World,
    pub arena: ArenaConfig,
    /// every message of the server, when set
    pub record: Option<Vec<(u32, Vec<u8>)>>,
    /// how far the drawn position of a client moved at each correction it smoothed (m), and the snapped ones
    pub at_correction: Vec<f64>,
    pub snapped: u64,
    /// drop LEAVE messages of the clients (a LEAVE lost)
    pub lose_leave: bool,
    pub fire: bool,
    clock: game::FixedTick,
    seed: u64,
    sev: Vec<ServerEvent>,
    cev: Vec<ClientEvent>,
}

impl<S: RoomSim> Rig<S> {
    pub fn new(seed: u64, server: NetServer<S>, arena: ArenaConfig) -> Rig<S> {
        let chan = ServerConfig { protocol: PROTOCOL, max_per_ip: 255, max_connections: 255, packets_per_s: 100_000, bytes_per_s: 1 << 28, ..ServerConfig::default() };
        let rate = server.options().rate;
        Rig {
            net: SimNet::new(seed, chan),
            server,
            cl: Vec::new(),
            map: arena::map(&arena),
            arena,
            record: None,
            at_correction: Vec::new(),
            snapped: 0,
            lose_leave: false,
            fire: false,
            clock: game::FixedTick::new(rate).with_max_steps(64),
            seed,
            sev: Vec::new(),
            cev: Vec::new(),
        }
    }

    /// a client more that starts after `after_ms` -> its index
    pub fn add(&mut self, ws: bool, weather: Weather, join: Option<usize>, after_ms: u64) -> usize {
        let k = self.cl.len();
        let opts = ClientOptions { name: format!("Kunde {}", k + 1), ..ClientOptions::default() };
        let nc = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&self.arena));
        let bot = Bot::new(BotConfig { seed: self.seed.wrapping_mul(131).wrapping_add(k as u64 + 1), fire: self.fire, half: self.arena.half, rate: 60, ..BotConfig::default() });
        let start_us = self.net.now_us + after_ms * 1000;
        self.cl.push(Cl { peer: usize::MAX, nc, ws, weather, join, started: false, start_us, bot, sent: Vec::new(), got: Vec::new(), first: BTreeMap::new(), leaving: false, rem: Vec::new(), feet: Vec::new() });
        k
    }

    fn room_of(&self, k: usize) -> Option<RoomCode> {
        self.cl.get(k)?.nc.welcome().map(|w| w.room)
    }

    /// one millisecond; `hook(k, client)` runs for every playing client after its ticks (send and take events)
    pub fn step_ms(&mut self, hook: &mut dyn FnMut(usize, &mut Cl)) {
        self.net.step(1000);
        let now_us = self.net.now_us;
        let now_ns = now_us * 1000;
        let ccfg = ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() };
        for k in 0..self.cl.len() {
            if self.cl[k].started || now_us < self.cl[k].start_us {
                continue;
            }
            let room = match self.cl[k].join {
                Some(j) => match self.room_of(j) {
                    Some(r) => Some(r),
                    None => continue,
                },
                None => None,
            };
            let opts = ClientOptions { name: format!("Kunde {}", k + 1), room, ..ClientOptions::default() };
            let mut nc = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&self.arena));
            nc.set_reliable(self.cl[k].ws);
            let w = self.cl[k].weather;
            let peer = if self.cl[k].ws { self.net.add_ws(ccfg, w) } else { self.net.add_udp(ccfg, w) };
            let c = &mut self.cl[k];
            c.nc = nc;
            c.peer = peer;
            c.started = true;
        }
        self.net.server_events(&mut self.sev);
        for e in &self.sev {
            self.server.on_event(e);
        }
        for _ in 0..self.clock.advance_to(now_ns) {
            self.server.step();
        }
        while let Some((id, b)) = self.server.take_out() {
            if let Some(r) = &mut self.record {
                r.push((id, b.clone()));
            }
            let _ = self.net.server_send(id, &b);
        }
        while let Some(id) = self.server.take_close() {
            self.net.server_disconnect(id);
        }
        for k in 0..self.cl.len() {
            if !self.cl[k].started || self.cl[k].peer == usize::MAX {
                continue;
            }
            let peer = self.cl[k].peer;
            self.net.link_events(peer, Which::Main, &mut self.cev);
            let c = &mut self.cl[k];
            for e in &self.cev {
                // (at a correction the drawn position must stay where it was: measured event by event)
                let (b, m0, s0) = (c.nc.drawn(), c.nc.predictor().stats().mismatches, c.nc.smoother().snaps);
                c.nc.on_event(&self.map, e, now_ns);
                if c.nc.predictor().stats().mismatches > m0 {
                    if c.nc.smoother().snaps > s0 {
                        self.snapped += 1;
                    } else if let (Some(b), Some(a)) = (b, c.nc.drawn()) {
                        self.at_correction.push((a - b).length());
                    }
                }
            }
            for _ in 0..c.nc.advance(now_ns) {
                c.nc.remotes(&mut c.rem);
                remote_feet(&c.rem, &mut c.feet);
                let own = c.nc.predicted().filter(|_| c.nc.predictor().known()).map(|s| s.feet());
                let input = c.bot.think(own, &c.feet);
                let t = c.nc.step(&self.map, input);
                if let Some(s) = c.nc.predicted() {
                    c.first.insert(t, s);
                }
            }
            while c.first.len() > 2000 {
                c.first.pop_first();
            }
            if c.nc.phase() == Phase::Playing {
                hook(k, c);
            }
            while let Some(e) = c.nc.take_event() {
                c.got.push(e);
            }
            let mut out = Vec::new();
            while let Some(b) = self.cl[k].nc.take_out() {
                out.push(b);
            }
            for b in out {
                if self.lose_leave && proto::is_leave(&b) {
                    continue;
                }
                let _ = self.net.link_send(peer, Which::Main, &b);
            }
            if self.cl[k].leaving {
                self.cl[k].leaving = false;
                self.net.link_close(peer, Which::Main);
            }
        }
    }

    pub fn run(&mut self, ms: u64, hook: &mut dyn FnMut(usize, &mut Cl)) {
        for _ in 0..ms {
            self.step_ms(hook);
        }
    }

    pub fn weather(&mut self, w: Weather) {
        for c in self.cl.iter_mut() {
            c.weather = w;
            if c.peer != usize::MAX {
                self.net.set_weather(c.peer, w);
            }
        }
    }

    pub fn playing(&self) -> usize {
        self.cl.iter().filter(|c| c.nc.phase() == Phase::Playing).count()
    }
}

fn talk_server(seed: u64, every: u32) -> NetServer<TalkArena> {
    let cfg = ArenaConfig { fire: false, props: 20, moving: 4, ..ArenaConfig::default() };
    let factory: RoomFactory<TalkArena> = Box::new(move |_| TalkArena::new(cfg, every));
    NetServer::new(ServerOptions::default(), seed, factory)
}

// ---------------------------------------------------------------- events

pub fn events(c: &mut Ctx) {
    model(c, false);
    model(c, true);
    wrap(c);
    sessions(c);
    overflow(c);
}

/// the outbox and inbox over a model channel: packets one way with the runs, the other way with the acknowledgements,
/// each lost, doubled and late by chance; then a clean channel drains what is left
fn model(c: &mut Ctx, reliable: bool) {
    let mut r = Rng::new(if reliable { 72 } else { 71 });
    let seeds = c.n(300);
    let (mut bad, mut total, mut sends, mut firsts) = (0u64, 0u64, 0u64, 0u64);
    for s in 0..seeds {
        let (loss, dup, late) = if reliable { (0.0, 0.0, 0.0) } else { [(0.0, 0.0, 0.0), (0.2, 0.1, 0.2), (0.5, 0.2, 0.3), (0.05, 0.0, 0.5)][s % 4] };
        let budget = 16 + r.below(400);
        let mut out = Outbox::new(reliable, budget);
        let mut inb = Inbox::new(!reliable);
        let mut sent: Vec<NetEvent> = Vec::new();
        let mut got: Vec<NetEvent> = Vec::new();
        // (arrival step, order, run) and (arrival step, ack)
        let mut fwd: Vec<(u32, u32, (u16, Vec<NetEvent>))> = Vec::new();
        let mut back: Vec<(u32, u32, u16)> = Vec::new();
        let mut order = 0u32;
        let steps = 600 + r.below(600) as u32;
        // (then a clean channel until everything is through: a small budget carries one event a packet)
        for step in 0..steps + 20_000 {
            let clean = step >= steps;
            if clean && out.pending() == 0 && fwd.is_empty() && back.is_empty() && step > steps + 50 {
                break;
            }
            if !clean {
                for _ in 0..r.below(4) {
                    let len = if r.below(10) == 0 { r.below(256) } else { r.below(12) };
                    let e = NetEvent { tick: step.wrapping_mul(3), channel: r.below(16) as u8, data: (0..len).map(|_| r.next_u64() as u8).collect() };
                    if out.push(e.clone()) {
                        sent.push(e);
                    }
                }
            }
            if let Some(run) = out.take_run(step) {
                let ch = |p: f64, r: &mut Rng| !clean && r.f() < p;
                if !ch(loss, &mut r) {
                    for _ in 0..if ch(dup, &mut r) { 2 } else { 1 } {
                        let at = step + 1 + if ch(late, &mut r) { r.below(8) as u32 } else { 0 };
                        order += 1;
                        fwd.push((at, order, run.clone()));
                    }
                }
            }
            fwd.sort_by_key(|x| (x.0, x.1));
            while fwd.first().is_some_and(|x| x.0 <= step) {
                let (_, _, (first, ev)) = fwd.remove(0);
                inb.receive(first, ev);
            }
            while let Some(e) = inb.take() {
                got.push(e);
            }
            if let Some(a) = inb.ack_for_packet() {
                if clean || r.f() >= loss {
                    order += 1;
                    back.push((step + 1 + if !clean && r.f() < late { r.below(8) as u32 } else { 0 }, order, a));
                }
            }
            back.sort_by_key(|x| (x.0, x.1));
            while back.first().is_some_and(|x| x.0 <= step) {
                let (_, _, a) = back.remove(0);
                out.ack(a);
            }
        }
        total += sent.len() as u64;
        let st = out.stats();
        sends += st.sent + st.resent;
        firsts += st.sent;
        if got != sent || inb.stats().gaps != 0 || out.pending() != 0 {
            bad += 1;
            if bad <= 3 {
                c.check(false, || format!("model {} seed {}: sent {}, got {}, gaps {}, pending {}", if reliable { "reliable" } else { "udp" }, s, sent.len(), got.len(), inb.stats().gaps, out.pending()));
            }
        }
    }
    c.check(bad == 0, || format!("model: {} of {} runs not exactly once in order", bad, seeds));
    crate::out(&format!(
        "  model ({}): {} runs, {} events exactly once and in order, each in {:.2} packets on average\n",
        if reliable { "WebSocket" } else { "UDP: loss up to 50 %, doubled, late" },
        seeds,
        total,
        sends as f64 / firsts.max(1) as f64
    ));
}

/// the numbers wrap: 70 000 events through a lossy model channel
fn wrap(c: &mut Ctx) {
    let mut r = Rng::new(73);
    let mut out = Outbox::new(false, 100);
    let mut inb = Inbox::new(true);
    let n = if c.quick { 70_000 } else { 140_000 };
    let mut next_data = 0u32;
    let mut expect = 0u32;
    let mut bad = 0u64;
    let mut step = 0u32;
    while expect < n {
        step += 1;
        while next_data < n && out.pending() < 40 {
            out.push(NetEvent { tick: step, channel: (next_data % 16) as u8, data: next_data.to_le_bytes().to_vec() });
            next_data += 1;
        }
        if let Some((first, ev)) = out.take_run(step) {
            if r.f() >= 0.1 {
                inb.receive(first, ev);
            }
        }
        while let Some(e) = inb.take() {
            if e.data != expect.to_le_bytes() {
                bad += 1;
            }
            expect += 1;
        }
        if let Some(a) = inb.ack_for_packet() {
            if r.f() >= 0.1 {
                out.ack(a);
            }
        }
        if step > n * 4 {
            break;
        }
    }
    c.check(expect == n && bad == 0, || format!("wrap: {} of {} events in order, {} out of order", expect, n, bad));
}

/// sessions: the talking arena, clients over UDP and WebSocket that send events too, in weather
fn sessions(c: &mut Ctx) {
    let secs = if c.quick { 4 } else { 10 };
    for (name, w, seed) in [("clear", Weather::CLEAR, 81u64), ("rough", Weather::ROUGH, 82), ("storm", Weather::STORM, 83)] {
        let mut rig = Rig::new(seed, talk_server(seed, 3), ArenaConfig { fire: false, props: 20, moving: 4, ..ArenaConfig::default() });
        for k in 0..6 {
            rig.add(k % 3 == 2, w, if k == 0 { None } else { Some(0) }, k as u64 * 80);
        }
        let mut rng = Rng::new(seed);
        let mut hook = |k: usize, cl: &mut Cl| {
            if rng.below(9) == 0 {
                let n = cl.sent.len() as u32;
                let big = rng.below(20) == 0;
                let len = 4 + rng.below(if big { 250 } else { 20 });
                let mut d = Vec::with_capacity(len);
                d.extend_from_slice(&(k as u16).to_le_bytes());
                d.extend_from_slice(&n.to_le_bytes());
                while d.len() < len {
                    d.push(rng.next_u64() as u8);
                }
                let ch = 5 + (rng.below(3) as u8) * 4;
                if cl.nc.send_event(ch, &d) {
                    cl.sent.push(NetEvent { tick: cl.nc.tick(), channel: ch, data: d });
                }
            }
        };
        rig.run(secs * 1000, &mut hook);
        // the weather clears; nothing new is sent, everything pending arrives
        rig.weather(Weather::CLEAR);
        rig.run(3000, &mut |_, _| {});
        let code = rig.room_of(0);
        let room = code.and_then(|r| rig.server.room(&r));
        let mut lines = String::new();
        c.check(rig.playing() == 6 && room.is_some(), || format!("events {}: {} of 6 play", name, rig.playing()));
        let Some(room) = room else { continue };
        let max = rig.server.stats().max_snapshot;
        c.check(max <= 1200, || format!("events {}: a snapshot of {} bytes", name, max));
        let (mut to_clients, mut from_clients) = (0usize, 0usize);
        for (k, cl) in rig.cl.iter().enumerate() {
            let id = cl.nc.own_id();
            // what the room sent to it (its echoes included), in order
            let want: Vec<(u8, &Vec<u8>)> = room
                .sent
                .iter()
                .filter(|s| s.1.contains(&id))
                .map(|s| (s.2, &s.3))
                .chain(core::iter::empty())
                .collect();
            let got_room: Vec<(u8, &Vec<u8>)> = cl.got.iter().filter(|e| e.channel != 4).map(|e| (e.channel, &e.data)).collect();
            c.check(got_room == want, || format!("events {}: client {} got {} of the room's {} events (or in another order)", name, k, got_room.len(), want.len()));
            // its own events: the room got each once and in order, and echoed them back
            let at_room: Vec<&NetEvent> = room.got.iter().filter(|g| g.0 == id).map(|g| &g.1).collect();
            let same = at_room.len() == cl.sent.len() && at_room.iter().zip(&cl.sent).all(|(a, b)| a.channel == b.channel && a.data == b.data && a.tick == b.tick);
            c.check(same, || format!("events {}: client {} sent {}, the room got {}", name, k, cl.sent.len(), at_room.len()));
            let echoes: Vec<&Vec<u8>> = cl.got.iter().filter(|e| e.channel == 4).map(|e| &e.data).collect();
            c.check(echoes.len() == cl.sent.len() && echoes.iter().zip(&cl.sent).all(|(a, b)| **a == b.data), || format!("events {}: client {} got {} echoes of {}", name, k, echoes.len(), cl.sent.len()));
            let st = cl.nc.events_in();
            c.check(st.gaps == 0 && cl.nc.events_pending() == 0, || format!("events {}: client {}: {:?}, {} pending", name, k, st, cl.nc.events_pending()));
            to_clients += cl.got.len();
            from_clients += cl.sent.len();
            if k < 3 {
                lines += &format!(" {}:{} dup {}", if cl.ws { "ws" } else { "udp" }, cl.got.len(), st.duplicates);
            }
        }
        crate::out(&format!("  events {:<5} {} s: {} events to 6 clients, {} from them, exactly once and in order;{}\n", name, secs, to_clients, from_clients, lines));
    }
}

/// a client that never acknowledges is dropped once its outbox is full (driven by hand, no network)
fn overflow(c: &mut Ctx) {
    let mut s = talk_server(9, 1);
    s.on_event(&ServerEvent::Connected { id: 1, transport: Transport::Udp, addr: None });
    let hello = proto::write_hello(&proto::Hello { version: proto::VERSION, room: None, name: String::from("Stumm") });
    s.on_event(&ServerEvent::Message { id: 1, seq: 0, data: hello });
    let mut ticks = 0;
    while s.stats().event_overflow == 0 && ticks < 3000 {
        s.step();
        ticks += 1;
    }
    let mut closed = false;
    while let Some(id) = s.take_close() {
        closed |= id == 1;
    }
    c.check(s.stats().event_overflow == 1 && closed && s.player_of(1).is_none(), || format!("overflow: {} after {} ticks, closed {}", s.stats().event_overflow, ticks, closed));
    c.check(ticks as usize >= MAX_PENDING / 2, || format!("overflow: dropped after {} ticks already", ticks));
}

// ---------------------------------------------------------------- leave

pub fn leave(c: &mut Ctx) {
    for (name, ws, lose) in [("UDP", false, false), ("UDP, LEAVE lost", false, true), ("WebSocket", true, false)] {
        let factory: RoomFactory<Arena> = Box::new(|_| Arena::new(ArenaConfig { fire: false, ..ArenaConfig::default() }));
        let mut rig = Rig::new(91, NetServer::new(ServerOptions::default(), 91, factory), ArenaConfig { fire: false, ..ArenaConfig::default() });
        rig.lose_leave = lose;
        rig.add(false, Weather::CLEAR, None, 0);
        rig.add(ws, Weather::CLEAR, Some(0), 100);
        rig.add(false, Weather::CLEAR, Some(0), 200);
        rig.run(1500, &mut |_, _| {});
        let code = rig.room_of(0);
        let id = rig.cl[1].nc.own_id();
        let before = code.map(|r| rig.server.players(&r).len()).unwrap_or(0);
        rig.cl[1].nc.leave();
        rig.cl[1].leaving = true;
        let t0 = rig.net.now_us;
        let (mut gone_server, mut gone_others) = (None, None);
        for _ in 0..1000 {
            rig.step_ms(&mut |_, _| {});
            let on_server = code.is_some_and(|r| rig.server.players(&r).iter().any(|p| p.1 == id));
            if gone_server.is_none() && !on_server && code.and_then(|r| rig.server.room(&r)).is_some_and(|a| !a.figures().contains_key(&id)) {
                gone_server = Some((rig.net.now_us - t0) / 1000);
            }
            let seen = |k: usize| rig.cl[k].nc.newest().is_some_and(|s| s.get(id).is_some());
            if gone_others.is_none() && gone_server.is_some() && !seen(0) && !seen(2) {
                gone_others = Some((rig.net.now_us - t0) / 1000);
            }
        }
        c.check(before == 3 && rig.cl[1].nc.phase() == Phase::Left, || format!("leave {}: {} players before, phase {:?}", name, before, rig.cl[1].nc.phase()));
        c.check(gone_server.is_some_and(|ms| ms <= 60) && gone_others.is_some_and(|ms| ms <= 150), || format!("leave {}: gone on the server after {:?} ms, for the others after {:?} ms", name, gone_server, gone_others));
        let left = rig.server.stats().left;
        c.check(left == if lose { 0 } else { 1 }, || format!("leave {}: {} LEAVEs counted", name, left));
        crate::out(&format!("  leave {:<16} the figure gone on the server after {} ms, for the others after {} ms\n", name, gone_server.unwrap_or(9999), gone_others.unwrap_or(9999)));
    }
    // a client that goes silent: only netchan's timeout ends it (5 s)
    let factory: RoomFactory<Arena> = Box::new(|_| Arena::new(ArenaConfig { fire: false, ..ArenaConfig::default() }));
    let mut rig = Rig::new(92, NetServer::new(ServerOptions::default(), 92, factory), ArenaConfig { fire: false, ..ArenaConfig::default() });
    rig.add(false, Weather::CLEAR, None, 0);
    rig.add(false, Weather::CLEAR, Some(0), 100);
    rig.run(1500, &mut |_, _| {});
    let code = rig.room_of(0);
    let id = rig.cl[1].nc.own_id();
    let peer = rig.cl[1].peer;
    rig.net.set_weather(peer, Weather { loss: 1.0, ..Weather::CLEAR });
    let t0 = rig.net.now_us;
    let mut gone = None;
    for _ in 0..7000 {
        rig.step_ms(&mut |_, _| {});
        if gone.is_none() && code.is_some_and(|r| !rig.server.players(&r).iter().any(|p| p.1 == id)) {
            gone = Some((rig.net.now_us - t0) / 1000);
        }
    }
    c.check(gone.is_some_and(|ms| (4900..=5600).contains(&ms)), || format!("silent: gone after {:?} ms (the timeout is 5000)", gone));
    crate::out(&format!("  leave silent           gone after {} ms (netchan's timeout of 5 s)\n", gone.unwrap_or(0)));
}

// ---------------------------------------------------------------- smooth

pub fn smooth(c: &mut Ctx) {
    // the offset takes the jump back and decays
    let mut s = Smoother::new(60.0, 1.0);
    let jump = Vec3::new(0.3, -0.1, 0.2);
    s.correct(jump);
    c.check(s.offset() == Vec3::ZERO - jump, || format!("the offset after a jump: {:?}", s.offset()));
    let mut last = s.offset().length();
    let mut mono = true;
    let mut t = 0;
    while s.offset() != Vec3::ZERO && t < 2000 {
        s.advance(0.016);
        t += 16;
        mono &= s.offset().length() < last || s.offset() == Vec3::ZERO;
        last = s.offset().length();
    }
    c.check(mono && (400..=800).contains(&t), || format!("decay: falling {}, below 0.1 mm after {} ms (time constant 60 ms, 0.37 m)", mono, t));
    // nearly the same for any frame rate: 100 ms in steps of 1, 7, 16, 33 ms
    let mut rest = Vec::new();
    for step in [1u32, 7, 16, 33, 50, 100] {
        let mut s = Smoother::new(60.0, 1.0);
        s.correct(Vec3::new(0.5, 0.0, 0.0));
        let mut ms = 0;
        while ms + step <= 100 {
            s.advance(step as f64 / 1000.0);
            ms += step;
        }
        s.advance((100 - ms) as f64 / 1000.0);
        rest.push(s.offset().length());
    }
    let (lo, hi) = rest.iter().fold((f64::MAX, 0.0f64), |a, b| (a.0.min(*b), a.1.max(*b)));
    let exact = 0.5 * libm_exp(-100.0 / 60.0);
    c.check(hi / lo < 1.01 && (lo - exact).abs() < 0.005 * exact, || format!("frame rates: rest after 100 ms {:?} (exp: {:.4})", rest, exact));
    // a jump over snap is not smoothed; neither is what adds up over it; the offset never exceeds snap
    let mut s = Smoother::new(60.0, 1.0);
    s.correct(Vec3::new(2.0, 0.0, 0.0));
    c.check(s.offset() == Vec3::ZERO && s.snaps == 1, || String::from("a jump of 2 m snaps"));
    s.correct(Vec3::new(0.6, 0.0, 0.0));
    s.correct(Vec3::new(0.6, 0.0, 0.0));
    c.check(s.offset() == Vec3::ZERO && s.snaps == 2, || format!("two of 0.6 m add up over 1 m: {:?}", s.offset()));
    let mut r = Rng::new(5);
    let mut worst = 0.0f64;
    let mut grew = 0;
    for _ in 0..c.n(20_000) {
        if r.below(3) == 0 {
            s.correct(Vec3::new(r.range(-0.8, 0.8), r.range(-0.3, 0.3), r.range(-0.8, 0.8)));
        } else {
            let before = s.offset().length();
            s.advance(r.range(0.0, 0.05));
            if s.offset().length() > before {
                grew += 1;
            }
        }
        worst = worst.max(s.offset().length());
    }
    c.check(worst <= 1.0 && grew == 0, || format!("random corrections: largest offset {:.3} m, grew without a correction {} times", worst, grew));
    let mut off = Smoother::new(0.0, 1.0);
    off.correct(Vec3::new(0.1, 0.0, 0.0));
    c.check(off.offset() == Vec3::ZERO, || String::from("smooth_ms 0: no smoothing"));
    // sessions: the drawn position does not move at a correction; the predictions stay exact
    let factory: RoomFactory<Arena> = Box::new(|_| Arena::new(ArenaConfig { fire: false, props: 20, moving: 4, ..ArenaConfig::default() }));
    let mut rig = Rig::new(95, NetServer::new(ServerOptions::default(), 95, factory), ArenaConfig { fire: false, props: 20, moving: 4, ..ArenaConfig::default() });
    for k in 0..6 {
        rig.add(k == 5, Weather::ROUGH, if k == 0 { None } else { Some(0) }, k as u64 * 60);
    }
    let secs = if c.quick { 4 } else { 10 };
    let mut largest = 0.0f64;
    for _ in 0..secs * 1000 {
        rig.step_ms(&mut |_, _| {});
        largest = rig.cl.iter().map(|k| k.nc.smoothing().length()).fold(largest, f64::max);
    }
    let worst = rig.at_correction.iter().fold(0.0f64, |a, b| a.max(*b));
    c.check(!rig.at_correction.is_empty() && worst < 1e-9, || format!("rough: {} corrections smoothed, the drawn position moved up to {:.2e} m at one", rig.at_correction.len(), worst));
    c.check(largest <= 1.0, || format!("rough: an offset of {:.3} m", largest));
    rig.weather(Weather::CLEAR);
    rig.run(2000, &mut |_, _| {});
    let healed = rig.cl.iter().map(|k| k.nc.predictor().stats().mismatches).sum::<u64>();
    rig.run(1000, &mut |_, _| {});
    let off: f64 = rig.cl.iter().map(|k| k.nc.smoothing().length()).fold(0.0, f64::max);
    let after = rig.cl.iter().map(|k| k.nc.predictor().stats().mismatches).sum::<u64>() - healed;
    c.check(off == 0.0 && after == 0, || format!("after the weather: {:.4} m of offset left, {} corrections", off, after));
    crate::out(&format!("  smooth: rough {} s, 6 clients: {} corrections smoothed (the drawn position moved {:.1e} m at most at one, the largest offset {:.3} m), {} snapped\n", secs, rig.at_correction.len(), worst, largest, rig.snapped));
}

// exp for the reference (a series: the test does not use the smoother's own approximation)
fn libm_exp(x: f64) -> f64 {
    let n = 64;
    let y = x / n as f64;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..20 {
        term *= y / k as f64;
        sum += term;
    }
    let mut r = sum;
    for _ in 0..6 {
        r *= r;
    }
    r
}

// ---------------------------------------------------------------- threads

/// rooms side by side on the pool against one after another: the same messages bit for bit; the room codes
pub fn threads(c: &mut Ctx) {
    let rooms = 6;
    let per = 3;
    let secs = if c.quick { 3 } else { 8 };
    let mut sums = Vec::new();
    for parallel in [false, true] {
        let factory: RoomFactory<Arena> = Box::new(|_| Arena::new(ArenaConfig { fire: true, props: 40, moving: 8, ..ArenaConfig::default() }));
        let mut server = NetServer::new(ServerOptions::default(), 97, factory);
        server.set_parallel(parallel);
        let mut rig = Rig::new(97, server, ArenaConfig { fire: true, props: 40, moving: 8, ..ArenaConfig::default() });
        rig.fire = true;
        rig.record = Some(Vec::new());
        for r in 0..rooms {
            let first = rig.add(r % 2 == 1, Weather::ROUGH, None, r as u64 * 30);
            for k in 1..per {
                rig.add(k == 2, Weather::ROUGH, Some(first), 300 + k as u64 * 40);
            }
        }
        rig.run(secs * 1000, &mut |_, _| {});
        rig.weather(Weather::CLEAR);
        rig.run(1000, &mut |_, _| {});
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let rec = rig.record.take().unwrap_or_default();
        for (id, b) in &rec {
            for x in id.to_le_bytes().iter().chain(b.iter()) {
                h ^= *x as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let bytes: usize = rec.iter().map(|m| m.1.len()).sum();
        sums.push((h, rec.len(), bytes, rig.server.rooms().len(), rig.playing()));
    }
    c.check(sums[0].3 == rooms && sums[0].4 == rooms * per, || format!("threads: {} rooms, {} playing", sums[0].3, sums[0].4));
    c.check(sums[0] == sums[1], || format!("threads: one thread {:?}, side by side {:?}", sums[0], sums[1]));
    crate::out(&format!(
        "  threads: {} rooms of {}, rough: {} messages ({} bytes) of the server, the same bit for bit side by side ({} workers) as on one thread ({:016x})\n",
        rooms,
        per,
        sums[0].1,
        sums[0].2,
        plat::thr_count(),
        sums[0].0
    ));
    codes(c);
}

/// room codes and tickets: seeded (the same every run), from a given source, from the system's random numbers
fn codes(c: &mut Ctx) {
    let mk = || -> RoomFactory<Arena> { Box::new(|_| Arena::new(ArenaConfig::default())) };
    let a: Vec<RoomCode> = (0..2).map(|_| NetServer::new(ServerOptions::default(), 5, mk()).create_room().unwrap_or(RoomCode(*b"AAAAAA"))).collect();
    c.check(a[0] == a[1], || format!("seeded: {} and {}", a[0], a[1]));
    fn fixed() -> u64 {
        0x0123_4567_89ab_cdef
    }
    let g = NetServer::with_random(ServerOptions::default(), fixed, mk()).create_room();
    c.check(g == Some(RoomCode::from_random(fixed())), || format!("a given source: {:?}", g));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut seen = alloc::collections::BTreeSet::new();
        let mut ok = true;
        for _ in 0..20 {
            let mut s = NetServer::secure(ServerOptions::default(), mk());
            for _ in 0..3 {
                let code = s.create_room();
                ok &= code.is_some_and(|x| x.valid());
                if let Some(x) = code {
                    seen.insert(x);
                }
            }
        }
        c.check(ok && seen.len() >= 59, || format!("secure: {} distinct codes of 60", seen.len()));
        // tickets of a secure server (a full state that does not fit, fetch_full): two joins, two different tickets
        let big = ArenaConfig { props: 300, displaced: 150, ..ArenaConfig::default() };
        let f: RoomFactory<Arena> = Box::new(move |_| Arena::new(big));
        let mut s = NetServer::secure(ServerOptions { fetch_full: true, ws_port: 9, ..ServerOptions::default() }, f);
        let mut tickets = Vec::new();
        let mut room = None;
        for id in 1..=2u32 {
            s.on_event(&ServerEvent::Connected { id, transport: Transport::Udp, addr: None });
            let hello = proto::write_hello(&proto::Hello { version: proto::VERSION, room, name: format!("T{}", id) });
            s.on_event(&ServerEvent::Message { id, seq: 0, data: hello });
            while let Some((to, b)) = s.take_out() {
                if let Some(w) = proto::read_welcome(&b) {
                    if to == id {
                        tickets.push(w.ticket);
                        room = Some(w.room);
                    }
                }
            }
        }
        c.check(tickets.len() == 2 && tickets[0] != tickets[1] && tickets.iter().all(|t| *t != 0), || format!("secure tickets: {:x?}", tickets));
    }
    let _ = tick_diff(0, 0);
}
