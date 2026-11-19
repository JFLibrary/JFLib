// server -- the server-authoritative side of the netcode, without I/O: netchan's events in, messages out
//
//   RoomSim        what a game puts into a room: its schema, the map's baseline, join and leave, one tick
//                  with the players' inputs, the replicated world, the priority of an entity for a viewer
//   NetServer      rooms by code, connections by netchan id; on_event takes netchan's ServerEvents (of a
//                  netchan::Server with sockets or of an endpoint::ServerCore on a simulated network),
//                  advance(now) runs the ticks due at the fixed rate, take_out / take_close say what to send
//                  and which connections to end
//
// A tick: every room hands each player the input of this tick (InputBuffer: the client's own, or the last
// one repeated when it did not arrive in time) to its RoomSim; every snapshot_every ticks each client gets
// a snapshot.  A snapshot is a delta against a base the client holds: over a WebSocket the last one sent
// (reliable and ordered), over UDP the newest the client confirmed it decoded (INPUT's ack) or, when that
// is better, the newest complete one it confirmed (the FULL it fetched), else the map's baseline.  Its
// changes are ordered -- the client's own figure, what is gone, then by a priority added up per snapshot
// until the entity is sent -- and cut to the budget (1200 bytes unless set: netchan does not fragment).
// What was sent is remembered per client exactly as the client will hold it.
//
// Joining: HELLO -> WELCOME.  The full state goes with the first snapshot when it fits (a WebSocket always:
// up to stream_budget); a UDP client whose full state does not fit a datagram gets a ticket and fetches it
// over a WebSocket of its own (FETCH -> FULL at the next snapshot tick), the user's decision of 18.11.2026.
// Until the FULL arrives the client's snapshots are cut by priority, so it sees its own figure and the
// most important things at once.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::ServerEvent;
use netchan::Transport;

use super::input::{InputBuffer, InputStats, NetInput};
use super::proto::{self, Base, SnapHeader, Welcome};
use super::room::{clean_name, unique_name, RoomCode};
use super::snapshot::{self, diff, encoded_bits, select, tick_diff, Change, Ent, Op, Schema, Snapshot};
use crate::tick::FixedTick;

/// what a game puts into a room (see the module header)
pub trait RoomSim {
    /// the kinds of the replicated entities and their field counts
    fn schema(&self) -> &Schema;
    /// the replicated entities as the map places them (every client builds the same from the map)
    fn baseline(&self) -> &Snapshot;
    /// a player joins with this (cleaned, unique) name -> the net id of its figure (None: no place)
    fn join(&mut self, name: &str) -> Option<u16>;
    /// the player's figure goes
    fn leave(&mut self, player: u16);
    /// one tick: every player's input (net id, input, false when it is the last one repeated)
    fn step(&mut self, tick: u32, dt: f64, inputs: &[(u16, NetInput, bool)]);
    /// the replicated world after the tick (tick set, entities sorted by id)
    fn snapshot(&self, tick: u32, out: &mut Snapshot);
    /// how much `e` matters to the player `viewer` now (added up every snapshot until it is sent)
    fn priority(&self, viewer: u16, e: &Ent) -> u32 {
        let _ = (viewer, e);
        1
    }
}

/// a server's settings
#[derive(Clone, Copy, Debug)]
pub struct ServerOptions {
    /// ticks a second
    pub rate: u32,
    /// a snapshot every this many ticks
    pub snapshot_every: u32,
    /// players a room takes
    pub max_players: usize,
    /// rooms at once
    pub max_rooms: usize,
    /// the bytes of a snapshot (UDP's limit; netchan's max_payload)
    pub budget: usize,
    /// the bytes of a join's state over a WebSocket and of a FULL (below netchan's max_stream_payload)
    pub stream_budget: usize,
    /// the margin the clients' inputs should arrive with (ticks)
    pub margin: u8,
    /// how far back hits are rewound at most (ms; 300 unless set: the 100 ms of the interpolation buffer and
    /// 200 ms beyond them, the user's decision of 2026-11-19)
    pub max_rewind_ms: u32,
    /// the WebSocket's port (WELCOME tells UDP clients where to fetch a full state; 0: none)
    pub ws_port: u16,
    /// a connection must send HELLO within this long (ms)
    pub hello_ms: u32,
    /// a ticket stays good this long (ms)
    pub ticket_ms: u32,
    /// a room without players closes after this long (ms)
    pub empty_room_ms: u32,
    /// the first tick (tests start near the wrap)
    pub start_tick: u32,
    /// snapshots remembered per client
    pub history: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            rate: 60,
            snapshot_every: 2,
            max_players: 16,
            max_rooms: 64,
            budget: 1200,
            stream_budget: 60 * 1024,
            margin: 2,
            max_rewind_ms: 300,
            ws_port: 0,
            hello_ms: 5000,
            ticket_ms: 10_000,
            empty_room_ms: 30_000,
            start_tick: 1,
            history: 32,
        }
    }
}

/// what the server counted
#[derive(Clone, Debug, Default)]
pub struct ServerStats {
    /// ticks run
    pub ticks: u64,
    /// snapshots sent (FULL included)
    pub snapshots: u64,
    /// their payload bytes
    pub snapshot_bytes: u64,
    /// the largest payload
    pub max_snapshot: usize,
    /// snapshots cut by the budget
    pub partial: u64,
    /// FULL sent over a fetch's WebSocket, and their bytes
    pub fulls: u64,
    /// the bytes of the FULLs
    pub full_bytes: u64,
    /// players who joined, refusals
    pub joins: u64,
    /// HELLOs refused
    pub refused: u64,
    /// the payload size of every snapshot (the first 200 000, for percentiles)
    pub sizes: Vec<u16>,
}

/// what the server counted for one client
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkStats {
    /// snapshots sent
    pub snapshots: u64,
    /// their payload bytes
    pub bytes: u64,
    /// cut by the budget
    pub partial: u64,
    /// based on the map's baseline
    pub from_map: u64,
    /// the inputs
    pub inputs: InputStats,
}

struct Sent {
    tick: u32,
    snap: Snapshot,
    complete: bool,
    acked: bool,
}

struct Player {
    welcome: Welcome,
    reliable: bool,
    room: RoomCode,
    id: u16,
    inputs: InputBuffer,
    sent: VecDeque<Sent>,
    prio: BTreeMap<u16, u32>,
    fetching: Option<u32>,
    stats: LinkStats,
}

enum Conn {
    Hello { since: u32, reliable: bool },
    Playing(Box<Player>),
    Fetch,
}

struct Room<S> {
    sim: S,
    /// (connection, figure id, name), in the order they joined
    players: Vec<(u32, u16, String)>,
    empty_since: Option<u32>,
}

/// the factory of a room's simulation
pub type RoomFactory<S> = Box<dyn FnMut(&RoomCode) -> S + Send>;

/// the server (see the module header)
pub struct NetServer<S: RoomSim> {
    opts: ServerOptions,
    factory: RoomFactory<S>,
    rooms: BTreeMap<RoomCode, Room<S>>,
    conns: BTreeMap<u32, Conn>,
    tickets: BTreeMap<u64, (u32, u32)>,
    fetches: Vec<(u32, u32)>,
    rng: u64,
    clock: FixedTick,
    tick: u32,
    out: VecDeque<(u32, Vec<u8>)>,
    close: VecDeque<u32>,
    stats: ServerStats,
    scratch: Vec<Change>,
}

impl<S: RoomSim> NetServer<S> {
    /// a server with these settings; `seed` drives room codes and tickets (random in a real server),
    /// `factory` makes a room's simulation
    pub fn new(opts: ServerOptions, seed: u64, factory: RoomFactory<S>) -> NetServer<S> {
        let mut opts = opts;
        opts.snapshot_every = opts.snapshot_every.clamp(1, 255);
        opts.history = opts.history.clamp(2, 255);
        NetServer {
            clock: FixedTick::new(opts.rate),
            tick: opts.start_tick,
            opts,
            factory,
            rooms: BTreeMap::new(),
            conns: BTreeMap::new(),
            tickets: BTreeMap::new(),
            fetches: Vec::new(),
            rng: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            out: VecDeque::new(),
            close: VecDeque::new(),
            stats: ServerStats::default(),
            scratch: Vec::new(),
        }
    }

    fn random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn ms_ticks(&self, ms: u32) -> u32 {
        ((ms as u64 * self.opts.rate as u64).div_ceil(1000)) as u32
    }

    /// the settings
    pub fn options(&self) -> &ServerOptions {
        &self.opts
    }

    /// the next tick to run
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// what it counted
    pub fn stats(&self) -> &ServerStats {
        &self.stats
    }

    /// a client's counts (None: no player on that connection)
    pub fn link_stats(&self, conn: u32) -> Option<LinkStats> {
        match self.conns.get(&conn)? {
            Conn::Playing(p) => {
                let mut s = p.stats;
                s.inputs = p.inputs.stats();
                Some(s)
            }
            _ => None,
        }
    }

    /// the codes of the rooms
    pub fn rooms(&self) -> Vec<RoomCode> {
        self.rooms.keys().copied().collect()
    }

    /// a room's simulation
    pub fn room(&self, code: &RoomCode) -> Option<&S> {
        self.rooms.get(code).map(|r| &r.sim)
    }

    /// a room's simulation for changing it
    pub fn room_mut(&mut self, code: &RoomCode) -> Option<&mut S> {
        self.rooms.get_mut(code).map(|r| &mut r.sim)
    }

    /// a room's players: (connection, figure id, name)
    pub fn players(&self, code: &RoomCode) -> Vec<(u32, u16, String)> {
        self.rooms.get(code).map(|r| r.players.clone()).unwrap_or_default()
    }

    /// the room and figure of a connection
    pub fn player_of(&self, conn: u32) -> Option<(RoomCode, u16)> {
        match self.conns.get(&conn)? {
            Conn::Playing(p) => Some((p.room, p.id)),
            _ => None,
        }
    }

    /// make an empty room (a lobby could hand its code out) -> its code; None when the server has its rooms
    pub fn create_room(&mut self) -> Option<RoomCode> {
        if self.rooms.len() >= self.opts.max_rooms {
            return None;
        }
        let code = loop {
            let c = RoomCode::from_random(self.random());
            if !self.rooms.contains_key(&c) {
                break c;
            }
        };
        let sim = (self.factory)(&code);
        self.rooms.insert(code, Room { sim, players: Vec::new(), empty_since: Some(self.tick) });
        Some(code)
    }

    /// the next message to send: (connection, payload)
    pub fn take_out(&mut self) -> Option<(u32, Vec<u8>)> {
        self.out.pop_front()
    }

    /// the next connection to end (after what was sent to it)
    pub fn take_close(&mut self) -> Option<u32> {
        self.close.pop_front()
    }

    fn refuse(&mut self, conn: u32, reason: u8) {
        self.out.push_back((conn, proto::write_refuse(reason)));
        self.close.push_back(conn);
        self.stats.refused += 1;
    }

    /// one of netchan's server events
    pub fn on_event(&mut self, e: &ServerEvent) {
        match e {
            ServerEvent::Connected { id, transport, .. } => {
                self.conns.insert(*id, Conn::Hello { since: self.tick, reliable: *transport == Transport::WebSocket });
            }
            ServerEvent::Message { id, data, .. } => self.on_message(*id, data),
            ServerEvent::Disconnected { id, .. } => self.drop_conn(*id),
            ServerEvent::Delivered { .. } | ServerEvent::Lost { .. } => {}
        }
    }

    fn drop_conn(&mut self, id: u32) {
        let Some(c) = self.conns.remove(&id) else { return };
        if let Conn::Playing(p) = c {
            if let Some(room) = self.rooms.get_mut(&p.room) {
                room.sim.leave(p.id);
                room.players.retain(|x| x.0 != id);
                if room.players.is_empty() {
                    room.empty_since = Some(self.tick);
                }
            }
            self.tickets.retain(|_, v| v.0 != id);
            self.fetches.retain(|f| f.1 != id);
        }
        self.fetches.retain(|f| f.0 != id);
    }

    fn on_message(&mut self, id: u32, data: &[u8]) {
        match self.conns.get_mut(&id) {
            Some(Conn::Hello { reliable, .. }) => {
                let reliable = *reliable;
                match proto::msg_type(data) {
                    Some(proto::MSG_HELLO) => match proto::read_hello(data) {
                        Some(h) => self.on_hello(id, reliable, h),
                        None => self.refuse(id, proto::REFUSE_PROTOCOL),
                    },
                    Some(proto::MSG_FETCH) => match proto::read_fetch(data) {
                        Some(t) => self.on_fetch(id, t),
                        None => self.refuse(id, proto::REFUSE_PROTOCOL),
                    },
                    _ => self.refuse(id, proto::REFUSE_PROTOCOL),
                }
            }
            Some(Conn::Playing(p)) => {
                if proto::msg_type(data) == Some(proto::MSG_HELLO) {
                    // (UDP) the WELCOME was lost: once more, with the tick of now
                    let w = Welcome { tick: self.tick, ..p.welcome.clone() };
                    self.out.push_back((id, proto::write_welcome(&w)));
                    return;
                }
                let Some(m) = proto::read_input(data) else { return };
                p.inputs.put_packet(m.newest, &m.inputs);
                for t in [m.ack, m.full].into_iter().flatten() {
                    for s in p.sent.iter_mut() {
                        if s.tick == t {
                            s.acked = true;
                        }
                    }
                }
                if m.full.is_some_and(|f| p.fetching == Some(f)) && m.ack.is_some() {
                    p.fetching = None;
                }
            }
            _ => {}
        }
    }

    fn on_hello(&mut self, id: u32, reliable: bool, h: proto::Hello) {
        if h.version != proto::VERSION {
            return self.refuse(id, proto::REFUSE_VERSION);
        }
        let Some(name) = clean_name(&h.name) else { return self.refuse(id, proto::REFUSE_NAME) };
        let code = match h.room {
            Some(c) => {
                if !self.rooms.contains_key(&c) {
                    return self.refuse(id, proto::REFUSE_NO_ROOM);
                }
                c
            }
            None => match self.create_room() {
                Some(c) => c,
                None => return self.refuse(id, proto::REFUSE_SERVER_FULL),
            },
        };
        let max_players = self.opts.max_players;
        let tick = self.tick;
        let Some(room) = self.rooms.get_mut(&code) else { return };
        if room.players.len() >= max_players {
            return self.refuse(id, proto::REFUSE_ROOM_FULL);
        }
        let taken: Vec<&str> = room.players.iter().map(|p| p.2.as_str()).collect();
        let name = unique_name(&name, &taken);
        let Some(pid) = room.sim.join(&name) else { return self.refuse(id, proto::REFUSE_ROOM_FULL) };
        room.players.push((id, pid, name));
        room.empty_since = None;
        // the size of the full state now: does it fit a datagram?
        let mut world = Snapshot::new(tick.wrapping_sub(1));
        room.sim.snapshot(tick.wrapping_sub(1), &mut world);
        let bits = snapshot::full_bits(room.sim.schema(), Some(room.sim.baseline()), &world, Some(room.sim.baseline()));
        let full_bytes = 16 + bits.div_ceil(8);
        let fetch = !reliable && full_bytes > self.opts.budget && self.opts.ws_port != 0;
        let ticket = if fetch { self.random() } else { 0 };
        if fetch {
            let until = tick.wrapping_add(self.ms_ticks(self.opts.ticket_ms));
            self.tickets.insert(ticket, (id, until));
        }
        let w = Welcome {
            room: code,
            player: pid,
            rate: self.opts.rate as u16,
            snapshot_every: self.opts.snapshot_every as u8,
            tick,
            margin: self.opts.margin,
            max_rewind: self.max_rewind_ticks() as u8,
            fetch,
            ticket,
            ws_port: self.opts.ws_port,
            full_bytes: full_bytes as u32,
        };
        self.out.push_back((id, proto::write_welcome(&w)));
        let p = Player {
            welcome: w,
            reliable,
            room: code,
            id: pid,
            inputs: InputBuffer::new(tick),
            sent: VecDeque::new(),
            prio: BTreeMap::new(),
            fetching: None,
            stats: LinkStats::default(),
        };
        self.conns.insert(id, Conn::Playing(Box::new(p)));
        self.stats.joins += 1;
    }

    fn on_fetch(&mut self, id: u32, ticket: u64) {
        let ok = self.tickets.remove(&ticket).filter(|(owner, until)| tick_diff(*until, self.tick) >= 0 && matches!(self.conns.get(owner), Some(Conn::Playing(_))));
        match ok {
            Some((owner, _)) => {
                self.conns.insert(id, Conn::Fetch);
                self.fetches.push((id, owner));
            }
            None => self.refuse(id, proto::REFUSE_TICKET),
        }
    }

    /// the lag compensation's limit in ticks (max_rewind_ms at the rate)
    pub fn max_rewind_ticks(&self) -> u32 {
        (self.opts.max_rewind_ms as u64 * self.opts.rate as u64 / 1000).min(255) as u32
    }

    /// the clock stands at now_ns (any monotonic clock): the ticks due run -> how many
    pub fn advance(&mut self, now_ns: u64) -> u32 {
        let n = self.clock.advance_to(now_ns);
        for _ in 0..n {
            self.step();
        }
        n
    }

    /// one tick now
    pub fn step(&mut self) {
        let t = self.tick;
        let dt = 1.0 / self.opts.rate as f64;
        let codes: Vec<RoomCode> = self.rooms.keys().copied().collect();
        let mut inputs: Vec<(u16, NetInput, bool)> = Vec::new();
        for code in &codes {
            inputs.clear();
            let Some(room) = self.rooms.get(code) else { continue };
            let mut conns: Vec<(u16, u32)> = room.players.iter().map(|p| (p.1, p.0)).collect();
            conns.sort();
            for (pid, conn) in conns {
                if let Some(Conn::Playing(p)) = self.conns.get_mut(&conn) {
                    let (i, own) = p.inputs.take(t);
                    inputs.push((pid, i, own));
                }
            }
            if let Some(room) = self.rooms.get_mut(code) {
                room.sim.step(t, dt, &inputs);
            }
        }
        if t % self.opts.snapshot_every == 0 {
            for code in &codes {
                self.snapshots(code, t);
            }
        }
        self.tick = t.wrapping_add(1);
        self.stats.ticks += 1;
        self.expire();
    }

    fn expire(&mut self) {
        let now = self.tick;
        let hello = self.ms_ticks(self.opts.hello_ms) as i32;
        let late: Vec<u32> = self.conns.iter().filter(|(_, c)| matches!(c, Conn::Hello { since, .. } if tick_diff(now, *since) > hello)).map(|(id, _)| *id).collect();
        for id in late {
            self.conns.remove(&id);
            self.close.push_back(id);
        }
        self.tickets.retain(|_, (_, until)| tick_diff(*until, now) >= 0);
        let empty = self.ms_ticks(self.opts.empty_room_ms) as i32;
        self.rooms.retain(|_, r| !r.players.is_empty() || r.empty_since.is_none_or(|s| tick_diff(now, s) <= empty));
    }

    // every client of a room gets its snapshot of tick t
    fn snapshots(&mut self, code: &RoomCode, t: u32) {
        let Some(room) = self.rooms.get(code) else { return };
        let mut world = Snapshot::new(t);
        room.sim.snapshot(t, &mut world);
        let conns: Vec<u32> = room.players.iter().map(|p| p.0).collect();
        for conn in conns {
            let fetch = self.fetches.iter().position(|f| f.1 == conn).map(|i| self.fetches.remove(i).0);
            match fetch {
                Some(fc) => self.send_full(code, conn, fc, &world),
                None => self.send_snapshot(code, conn, &world),
            }
        }
    }

    fn send_full(&mut self, code: &RoomCode, conn: u32, fetch_conn: u32, world: &Snapshot) {
        let Some(room) = self.rooms.get(code) else { return };
        let Some(Conn::Playing(p)) = self.conns.get_mut(&conn) else { return };
        let (schema, baseline) = (room.sim.schema(), room.sim.baseline());
        let mut changes = Vec::new();
        diff(schema, Some(baseline), world, Some(baseline), &mut changes);
        let h = SnapHeader { tick: world.tick, base: Base::Map, player: p.id, have: p.inputs.have(), margin: None };
        let mut w = proto::write_snap_header(proto::MSG_FULL, &h);
        let budget = self.opts.stream_budget.saturating_mul(8).saturating_sub(w.bit_len());
        let order = own_first(&changes, p.id, &p.prio);
        let chosen = select(&changes, &order, budget);
        snapshot::encode(&mut w, schema, Some(baseline), world, Some(baseline), &chosen);
        let as_sent = snapshot::apply(Some(baseline), world, &chosen);
        let complete = chosen.len() == changes.len();
        let bytes = w.finish();
        p.fetching = Some(world.tick);
        push_sent(&mut p.sent, Sent { tick: world.tick, snap: as_sent, complete, acked: false }, self.opts.history);
        p.stats.snapshots += 1;
        p.stats.bytes += bytes.len() as u64;
        self.stats.fulls += 1;
        self.stats.full_bytes += bytes.len() as u64;
        self.out.push_back((fetch_conn, bytes));
        self.close.push_back(fetch_conn);
    }

    fn send_snapshot(&mut self, code: &RoomCode, conn: u32, world: &Snapshot) {
        let Some(room) = self.rooms.get(code) else { return };
        let Some(Conn::Playing(p)) = self.conns.get_mut(&conn) else { return };
        let (schema, baseline) = (room.sim.schema(), room.sim.baseline());
        let t = world.tick;
        // the base: candidates the client holds
        let mut cands: Vec<Option<usize>> = Vec::new();
        if p.reliable {
            cands.push(p.sent.len().checked_sub(1));
        } else {
            let usable = |s: &Sent| s.acked && (1..=255).contains(&tick_diff(t, s.tick));
            let newest = p.sent.iter().rposition(usable);
            let complete = p.sent.iter().rposition(|s| usable(s) && s.complete);
            cands.push(newest);
            if complete != newest {
                cands.push(complete);
            }
            if newest.is_none() {
                cands.push(None);
            }
        }
        if p.reliable && cands[0].is_some_and(|i| !(1..=255).contains(&tick_diff(t, p.sent[i].tick))) {
            cands[0] = None;
        }
        let changes = &mut self.scratch;
        let mut best: Option<(usize, Option<usize>, Vec<Change>)> = None;
        for c in cands {
            let base = c.map(|i| &p.sent[i].snap).unwrap_or(baseline);
            diff(schema, Some(base), world, Some(baseline), changes);
            let bits = encoded_bits(changes);
            if best.as_ref().is_none_or(|b| bits < b.0) {
                best = Some((bits, c, changes.clone()));
            }
        }
        let Some((_, base_i, changes)) = best else { return };
        let (base_snap, base) = match base_i {
            Some(i) => (&p.sent[i].snap, Base::Back(tick_diff(t, p.sent[i].tick) as u8)),
            None => (baseline, Base::Map),
        };
        // the order: own figure, removals, then the priorities added up
        let mut prio: BTreeMap<u16, u32> = BTreeMap::new();
        for c in &changes {
            if c.op != Op::Remove {
                let add = world.get(c.id).map(|e| room.sim.priority(p.id, e)).unwrap_or(1);
                prio.insert(c.id, p.prio.get(&c.id).copied().unwrap_or(0).saturating_add(add));
            }
        }
        let order = own_first(&changes, p.id, &prio);
        let margin = p.inputs.take_margin().map(|m| m.clamp(-127, 127) as i8);
        let h = SnapHeader { tick: t, base, player: p.id, have: p.inputs.have(), margin };
        let mut w = proto::write_snap_header(proto::MSG_SNAPSHOT, &h);
        let first = p.sent.is_empty();
        let bytes_budget = if first && p.reliable { self.opts.stream_budget } else { self.opts.budget };
        let budget = bytes_budget.saturating_mul(8).saturating_sub(w.bit_len());
        let chosen = select(&changes, &order, budget);
        snapshot::encode(&mut w, schema, Some(base_snap), world, Some(baseline), &chosen);
        let as_sent = snapshot::apply(Some(base_snap), world, &chosen);
        let complete = chosen.len() == changes.len();
        for c in &chosen {
            prio.remove(&c.id);
        }
        p.prio = prio;
        let bytes = w.finish();
        if base == Base::Map {
            p.stats.from_map += 1;
        }
        // (over a WebSocket everything arrives: the last sent is the base, no ack needed)
        let acked = p.reliable;
        push_sent(&mut p.sent, Sent { tick: t, snap: as_sent, complete, acked }, self.opts.history);
        p.stats.snapshots += 1;
        p.stats.bytes += bytes.len() as u64;
        if !complete {
            p.stats.partial += 1;
            self.stats.partial += 1;
        }
        self.stats.snapshots += 1;
        self.stats.snapshot_bytes += bytes.len() as u64;
        self.stats.max_snapshot = self.stats.max_snapshot.max(bytes.len());
        if self.stats.sizes.len() < 200_000 {
            self.stats.sizes.push(bytes.len().min(u16::MAX as usize) as u16);
        }
        self.out.push_back((conn, bytes));
    }
}

fn push_sent(q: &mut VecDeque<Sent>, s: Sent, cap: usize) {
    q.push_back(s);
    while q.len() > cap {
        q.pop_front();
    }
}

// the order of the changes: the viewer's own figure, removals, then by priority (highest first), id
fn own_first(changes: &[Change], own: u16, prio: &BTreeMap<u16, u32>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..changes.len()).collect();
    let class = |c: &Change| if c.id == own { 0u8 } else if c.op == Op::Remove { 1 } else { 2 };
    order.sort_by(|&a, &b| {
        let (x, y) = (&changes[a], &changes[b]);
        class(x).cmp(&class(y)).then_with(|| prio.get(&y.id).copied().unwrap_or(0).cmp(&prio.get(&x.id).copied().unwrap_or(0))).then(x.id.cmp(&y.id))
    });
    order
}
