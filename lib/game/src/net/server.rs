// server -- the server-authoritative side of the netcode, without I/O: netchan's events in, messages out
//
//   RoomSim        what a game puts into a room: its schema, the map's baseline, join and leave, one tick
//                  with the players' inputs, the replicated world, the priority of an entity for a viewer,
//                  and the reliable events both ways (on_event, take_events)
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
// What was sent is remembered per client exactly as the client will hold it.  The events of the room
// (RoomSim::take_events) ride in every snapshot until the client acknowledges them (event.rs); the client's
// events come with its INPUTs and reach the RoomSim at the start of the next tick.
//
// Rooms are independent: a room's tick (its inputs, its step, its events, its snapshots) touches nothing of
// another room.  With set_parallel the rooms of a tick run side by side on plat's worker pool (one room per
// item: one room per core with enough cores); their messages are merged in the order of the room codes, so
// the server sends bit for bit what it sends on one thread.
//
// Joining: HELLO -> WELCOME.  The full state goes with the first snapshot when it fits (a WebSocket always:
// up to stream_budget).  A UDP client whose full state does not fit a datagram gets it filled in: its
// snapshots are cut by priority, so it sees its own figure and the most important things at once and the
// rest in the next snapshots (about 200 ms for 300 props, as fast as the fetch; the user's decision of
// 19.11.2026).  With ServerOptions::fetch_full it gets a ticket instead and fetches the full state over a
// WebSocket of its own (FETCH -> FULL at the next snapshot tick; the user's first decision of 18.11.2026),
// filled in by priority until the FULL arrives.  Leaving: LEAVE (or netchan's DISCONNECT) removes the
// figure at once; a client that goes silent is removed when netchan's timeout ends its connection.
//
// Room codes and tickets come from the server's generator: a seeded one (new: tests, the simulated network,
// a practice mode in the browser) or the system's cryptographic random numbers (secure, with_random): a
// ticket is sent to its client, and a seeded xorshift gives its state away with one output.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::ServerEvent;
use netchan::Transport;
use rt::sync::SpinLock;

use super::event::{self, Inbox, InboxStats, NetEvent, Outbox, OutboxStats};
use super::input::{InputBuffer, InputStats, NetInput};
use super::proto::{self, Base, SnapHeader, Welcome};
use super::room::{clean_name, unique_name, RoomCode};
use super::snapshot::{self, diff, encoded_bits, select, tick_diff, Change, Ent, Op, Schema, Snapshot};
use crate::tick::FixedTick;

/// who gets an event of a room
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum To {
    /// every player of the room
    All,
    /// the player with this figure
    Player(u16),
    /// every player but this one
    AllBut(u16),
}

impl To {
    /// whether the player with figure `id` gets it
    pub fn includes(&self, id: u16) -> bool {
        match self {
            To::All => true,
            To::Player(p) => *p == id,
            To::AllBut(p) => *p != id,
        }
    }
}

/// an event a room sends to its players (RoomSim::take_events): reliable, in order, each delivered once
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomEvent {
    /// who gets it
    pub to: To,
    /// the game's channel (0 ..= 15)
    pub channel: u8,
    /// the game's bytes (at most event::MAX_EVENT)
    pub data: Vec<u8>,
}

/// what a game puts into a room (see the module header)
pub trait RoomSim {
    /// the kinds of the replicated entities and their field counts
    fn schema(&self) -> &Schema;
    /// the replicated entities as the map places them (every client builds the same from the map)
    fn baseline(&self) -> &Snapshot;
    /// a player joins with this (cleaned, unique) name -> the net id of its figure (None: no place)
    fn join(&mut self, name: &str) -> Option<u16>;
    /// the player's figure goes (it left, or its connection ended)
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
    /// an event of the player with figure `player` (reliable, in order): at the start of the tick after it came,
    /// before step, by the figures' ids and in the order they came
    fn on_event(&mut self, player: u16, e: &NetEvent) {
        let _ = (player, e);
    }
    /// the events of the last step for the players (asked after every step; their tick is the step's)
    fn take_events(&mut self, out: &mut Vec<RoomEvent>) {
        let _ = out;
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
    /// the bytes of the events a snapshot carries at most (out of `budget`; it always carries one)
    pub event_budget: usize,
    /// the margin the clients' inputs should arrive with (ticks)
    pub margin: u8,
    /// how far back hits are rewound at most (ms; 300 unless set: the 100 ms of the interpolation buffer and
    /// 200 ms beyond them, the user's decision of 2026-11-19)
    pub max_rewind_ms: u32,
    /// the WebSocket's port (WELCOME tells UDP clients where to fetch a full state; 0: none)
    pub ws_port: u16,
    /// a UDP client whose full state does not fit a datagram fetches it over a WebSocket of its own (ws_port, a
    /// ticket); false unless set: its snapshots fill it in by priority, as fast and without a second connection
    /// (the user's decision of 2026-11-19)
    pub fetch_full: bool,
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
            event_budget: 400,
            margin: 2,
            max_rewind_ms: 300,
            ws_port: 0,
            fetch_full: false,
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
    /// players who left with LEAVE
    pub left: u64,
    /// events of the rooms queued for players
    pub events_out: u64,
    /// events of players handed to their rooms
    pub events_in: u64,
    /// players dropped because they did not acknowledge their events (event::MAX_PENDING waiting)
    pub event_overflow: u64,
    /// the payload size of every snapshot (the first 200 000, for percentiles)
    pub sizes: Vec<u16>,
}

impl ServerStats {
    // a room's counts of a tick into the server's (in the order of the rooms: the same as on one thread)
    fn take_from(&mut self, o: &mut ServerStats) {
        self.snapshots += o.snapshots;
        self.snapshot_bytes += o.snapshot_bytes;
        self.max_snapshot = self.max_snapshot.max(o.max_snapshot);
        self.partial += o.partial;
        self.fulls += o.fulls;
        self.full_bytes += o.full_bytes;
        self.events_out += o.events_out;
        self.events_in += o.events_in;
        for s in o.sizes.drain(..) {
            if self.sizes.len() < 200_000 {
                self.sizes.push(s);
            }
        }
        *o = ServerStats { sizes: core::mem::take(&mut o.sizes), ..ServerStats::default() };
    }
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
    /// the events to the client
    pub events: OutboxStats,
    /// the events of the client
    pub events_in: InboxStats,
}

struct Sent {
    tick: u32,
    snap: Snapshot,
    complete: bool,
    acked: bool,
}

struct Player {
    conn: u32,
    name: String,
    welcome: Welcome,
    reliable: bool,
    id: u16,
    inputs: InputBuffer,
    sent: VecDeque<Sent>,
    prio: BTreeMap<u16, u32>,
    fetching: Option<u32>,
    /// a fetch's connection waiting for its FULL at the next snapshot tick
    fetch_conn: Option<u32>,
    out_ev: Outbox,
    in_ev: Inbox,
    stats: LinkStats,
}

enum Conn {
    Hello { since: u32, reliable: bool },
    Playing(RoomCode),
    Fetch,
}

// what a room's tick needs of the server (read only: rooms run side by side)
struct TickCtx {
    t: u32,
    dt: f64,
    snap: bool,
    opts: ServerOptions,
}

struct Room<S> {
    sim: S,
    /// the players in the order they joined
    players: Vec<Box<Player>>,
    empty_since: Option<u32>,
    // the results of a tick, merged by the server in the order of the rooms
    out: Vec<(u32, Vec<u8>)>,
    close: Vec<u32>,
    kicked: Vec<u32>,
    stats: ServerStats,
    scratch: Vec<Change>,
    events: Vec<RoomEvent>,
    inputs: Vec<(u16, NetInput, bool)>,
}

impl<S: RoomSim> Room<S> {
    fn new(sim: S, now: u32) -> Room<S> {
        Room {
            sim,
            players: Vec::new(),
            empty_since: Some(now),
            out: Vec::new(),
            close: Vec::new(),
            kicked: Vec::new(),
            stats: ServerStats::default(),
            scratch: Vec::new(),
            events: Vec::new(),
            inputs: Vec::new(),
        }
    }

    fn player_mut(&mut self, conn: u32) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.conn == conn).map(|p| &mut **p)
    }

    // one tick of this room (see the module header)
    fn tick(&mut self, ctx: &TickCtx) {
        let t = ctx.t;
        // the players by figure id (and connection): their events, then their inputs
        let mut order: Vec<(u16, u32, usize)> = self.players.iter().enumerate().map(|(i, p)| (p.id, p.conn, i)).collect();
        order.sort();
        for &(pid, _, i) in &order {
            while let Some(e) = self.players[i].in_ev.take() {
                self.sim.on_event(pid, &e);
                self.stats.events_in += 1;
            }
        }
        self.inputs.clear();
        for &(pid, _, i) in &order {
            let (inp, own) = self.players[i].inputs.take(t);
            self.inputs.push((pid, inp, own));
        }
        self.sim.step(t, ctx.dt, &self.inputs);
        // the room's events into the players' outboxes
        self.sim.take_events(&mut self.events);
        for ev in self.events.drain(..) {
            for p in self.players.iter_mut() {
                if !ev.to.includes(p.id) {
                    continue;
                }
                if p.out_ev.push(NetEvent { tick: t, channel: ev.channel, data: ev.data.clone() }) {
                    self.stats.events_out += 1;
                } else if p.out_ev.pending() >= event::MAX_PENDING && !self.kicked.contains(&p.conn) {
                    self.kicked.push(p.conn);
                }
            }
        }
        if ctx.snap {
            let mut world = Snapshot::new(t);
            self.sim.snapshot(t, &mut world);
            for i in 0..self.players.len() {
                match self.players[i].fetch_conn.take() {
                    Some(fc) => self.send_full(i, fc, &world, &ctx.opts),
                    None => self.send_snapshot(i, &world, &ctx.opts),
                }
            }
        }
    }

    fn send_full(&mut self, i: usize, fetch_conn: u32, world: &Snapshot, opts: &ServerOptions) {
        let p = &mut self.players[i];
        let (schema, baseline) = (self.sim.schema(), self.sim.baseline());
        let mut changes = Vec::new();
        diff(schema, Some(baseline), world, Some(baseline), &mut changes);
        let h = SnapHeader::new(world.tick, Base::Map, p.id, p.inputs.have(), None);
        let mut w = proto::write_snap_header(proto::MSG_FULL, &h);
        let budget = opts.stream_budget.saturating_mul(8).saturating_sub(w.bit_len());
        let order = own_first(&changes, p.id, &p.prio);
        let chosen = select(&changes, &order, budget);
        snapshot::encode(&mut w, schema, Some(baseline), world, Some(baseline), &chosen);
        let as_sent = snapshot::apply(Some(baseline), world, &chosen);
        let complete = chosen.len() == changes.len();
        let bytes = w.finish();
        p.fetching = Some(world.tick);
        push_sent(&mut p.sent, Sent { tick: world.tick, snap: as_sent, complete, acked: false }, opts.history);
        p.stats.snapshots += 1;
        p.stats.bytes += bytes.len() as u64;
        self.stats.fulls += 1;
        self.stats.full_bytes += bytes.len() as u64;
        self.out.push((fetch_conn, bytes));
        self.close.push(fetch_conn);
    }

    fn send_snapshot(&mut self, i: usize, world: &Snapshot, opts: &ServerOptions) {
        let p = &mut self.players[i];
        let (schema, baseline) = (self.sim.schema(), self.sim.baseline());
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
                let add = world.get(c.id).map(|e| self.sim.priority(p.id, e)).unwrap_or(1);
                prio.insert(c.id, p.prio.get(&c.id).copied().unwrap_or(0).saturating_add(add));
            }
        }
        let order = own_first(&changes, p.id, &prio);
        let margin = p.inputs.take_margin().map(|m| m.clamp(-127, 127) as i8);
        // the event part: the acknowledgement of the client's events, a run of the room's
        let ev_ack = p.in_ev.ack_for_packet();
        let run = p.out_ev.take_run(t);
        let h = SnapHeader { tick: t, base, player: p.id, have: p.inputs.have(), margin, ev_ack, events: run.is_some() };
        let mut w = proto::write_snap_header(proto::MSG_SNAPSHOT, &h);
        if let Some((first, ev)) = &run {
            event::write_section(&mut w, *first, ev, t);
        }
        let first = p.sent.is_empty();
        let bytes_budget = if first && p.reliable { opts.stream_budget } else { opts.budget };
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
        push_sent(&mut p.sent, Sent { tick: t, snap: as_sent, complete, acked }, opts.history);
        p.stats.snapshots += 1;
        p.stats.bytes += bytes.len() as u64;
        if !complete {
            p.stats.partial += 1;
            self.stats.partial += 1;
        }
        self.stats.snapshots += 1;
        self.stats.snapshot_bytes += bytes.len() as u64;
        self.stats.max_snapshot = self.stats.max_snapshot.max(bytes.len());
        self.stats.sizes.push(bytes.len().min(u16::MAX as usize) as u16);
        self.out.push((p.conn, bytes));
    }
}

/// the factory of a room's simulation
pub type RoomFactory<S> = Box<dyn FnMut(&RoomCode) -> S + Send>;

// runs the rooms of a tick (set_parallel: on the worker pool)
type Runner<S> = for<'a> fn(&[SpinLock<&'a mut Room<S>>], &TickCtx);

enum Random {
    Seeded(u64),
    Source(fn() -> u64),
}

/// the server (see the module header)
pub struct NetServer<S: RoomSim> {
    opts: ServerOptions,
    factory: RoomFactory<S>,
    rooms: BTreeMap<RoomCode, Room<S>>,
    conns: BTreeMap<u32, Conn>,
    tickets: BTreeMap<u64, (u32, u32)>,
    random: Random,
    clock: FixedTick,
    tick: u32,
    out: VecDeque<(u32, Vec<u8>)>,
    close: VecDeque<u32>,
    stats: ServerStats,
    runner: Option<Runner<S>>,
}

impl<S: RoomSim> NetServer<S> {
    /// a server with these settings whose room codes and tickets come from a generator seeded with `seed` (the
    /// same codes every run: tests, the simulated network, a practice mode); `factory` makes a room's simulation.
    /// A server on the network takes secure() or with_random()
    pub fn new(opts: ServerOptions, seed: u64, factory: RoomFactory<S>) -> NetServer<S> {
        NetServer::with(opts, Random::Seeded(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1), factory)
    }

    /// a server whose room codes and tickets come from `random` (a cryptographic source, such as
    /// plat::random_u64; secure() takes that one)
    pub fn with_random(opts: ServerOptions, random: fn() -> u64, factory: RoomFactory<S>) -> NetServer<S> {
        NetServer::with(opts, Random::Source(random), factory)
    }

    /// a server whose room codes and tickets come from the system's cryptographic random numbers
    /// (plat::random_u64: Windows, Linux, Apple, Android)
    #[cfg(any(windows, target_os = "linux", target_vendor = "apple", target_os = "android"))]
    pub fn secure(opts: ServerOptions, factory: RoomFactory<S>) -> NetServer<S> {
        NetServer::with_random(opts, plat::random_u64, factory)
    }

    fn with(opts: ServerOptions, random: Random, factory: RoomFactory<S>) -> NetServer<S> {
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
            random,
            out: VecDeque::new(),
            close: VecDeque::new(),
            stats: ServerStats::default(),
            runner: None,
        }
    }

    fn random(&mut self) -> u64 {
        match &mut self.random {
            Random::Seeded(state) => {
                let mut x = *state;
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                *state = x;
                x.wrapping_mul(0x2545_f491_4f6c_dd1d)
            }
            Random::Source(f) => f(),
        }
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

    /// whether the rooms run side by side (set_parallel)
    pub fn parallel(&self) -> bool {
        self.runner.is_some()
    }

    fn player(&self, conn: u32) -> Option<&Player> {
        match self.conns.get(&conn)? {
            Conn::Playing(code) => self.rooms.get(code)?.players.iter().find(|p| p.conn == conn).map(|p| &**p),
            _ => None,
        }
    }

    fn player_mut(&mut self, conn: u32) -> Option<&mut Player> {
        match self.conns.get(&conn)? {
            Conn::Playing(code) => self.rooms.get_mut(code)?.player_mut(conn),
            _ => None,
        }
    }

    /// a client's counts (None: no player on that connection)
    pub fn link_stats(&self, conn: u32) -> Option<LinkStats> {
        let p = self.player(conn)?;
        let mut s = p.stats;
        s.inputs = p.inputs.stats();
        s.events = p.out_ev.stats();
        s.events_in = p.in_ev.stats();
        Some(s)
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

    /// a room's players: (connection, figure id, name), in the order they joined
    pub fn players(&self, code: &RoomCode) -> Vec<(u32, u16, String)> {
        self.rooms.get(code).map(|r| r.players.iter().map(|p| (p.conn, p.id, p.name.clone())).collect()).unwrap_or_default()
    }

    /// the room and figure of a connection
    pub fn player_of(&self, conn: u32) -> Option<(RoomCode, u16)> {
        match self.conns.get(&conn)? {
            Conn::Playing(code) => self.player(conn).map(|p| (*code, p.id)),
            _ => None,
        }
    }

    /// queue an event for the player on connection `conn` (besides those of its room) -> false when refused (no
    /// player there, too long, a bad channel, its outbox full)
    pub fn send_event(&mut self, conn: u32, channel: u8, data: &[u8]) -> bool {
        let tick = self.tick;
        let ok = self.player_mut(conn).is_some_and(|p| p.out_ev.push(NetEvent::new(tick, channel, data)));
        if ok {
            self.stats.events_out += 1;
        }
        ok
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
        self.rooms.insert(code, Room::new(sim, self.tick));
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
        match c {
            Conn::Playing(code) => {
                if let Some(room) = self.rooms.get_mut(&code) {
                    if let Some(i) = room.players.iter().position(|p| p.conn == id) {
                        let p = room.players.remove(i);
                        room.sim.leave(p.id);
                    }
                    if room.players.is_empty() {
                        room.empty_since = Some(self.tick);
                    }
                }
                self.tickets.retain(|_, v| v.0 != id);
            }
            Conn::Fetch => {
                for room in self.rooms.values_mut() {
                    for p in room.players.iter_mut() {
                        if p.fetch_conn == Some(id) {
                            p.fetch_conn = None;
                        }
                    }
                }
            }
            Conn::Hello { .. } => {}
        }
    }

    fn on_message(&mut self, id: u32, data: &[u8]) {
        match self.conns.get(&id) {
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
            Some(Conn::Playing(_)) => {
                if proto::is_leave(data) {
                    // the player leaves: the figure goes now, the connection after what was sent
                    self.stats.left += 1;
                    self.drop_conn(id);
                    self.close.push_back(id);
                    return;
                }
                let tick = self.tick;
                let Some(p) = self.player_mut(id) else { return };
                if proto::msg_type(data) == Some(proto::MSG_HELLO) {
                    // (UDP) the WELCOME was lost: once more, with the tick of now
                    let w = Welcome { tick, ..p.welcome.clone() };
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
                if let Some(a) = m.ev_ack {
                    p.out_ev.ack(a);
                }
                if let Some((first, ev)) = m.events {
                    p.in_ev.receive(first, ev);
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
        let taken: Vec<&str> = room.players.iter().map(|p| p.name.as_str()).collect();
        let name = unique_name(&name, &taken);
        let Some(pid) = room.sim.join(&name) else { return self.refuse(id, proto::REFUSE_ROOM_FULL) };
        room.empty_since = None;
        // the size of the full state now: does it fit a datagram?
        let mut world = Snapshot::new(tick.wrapping_sub(1));
        room.sim.snapshot(tick.wrapping_sub(1), &mut world);
        let bits = snapshot::full_bits(room.sim.schema(), Some(room.sim.baseline()), &world, Some(room.sim.baseline()));
        let full_bytes = 16 + bits.div_ceil(8);
        let fetch = !reliable && full_bytes > self.opts.budget && self.opts.fetch_full && self.opts.ws_port != 0;
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
            conn: id,
            name,
            welcome: w,
            reliable,
            id: pid,
            inputs: InputBuffer::new(tick),
            sent: VecDeque::new(),
            prio: BTreeMap::new(),
            fetching: None,
            fetch_conn: None,
            out_ev: Outbox::new(reliable, self.opts.event_budget),
            // (the client may not know its transport: it waits for acknowledgements over a WebSocket too)
            in_ev: Inbox::new(true),
            stats: LinkStats::default(),
        };
        if let Some(room) = self.rooms.get_mut(&code) {
            room.players.push(Box::new(p));
        }
        self.conns.insert(id, Conn::Playing(code));
        self.stats.joins += 1;
    }

    fn on_fetch(&mut self, id: u32, ticket: u64) {
        let ok = self.tickets.remove(&ticket).filter(|(owner, until)| tick_diff(*until, self.tick) >= 0 && matches!(self.conns.get(owner), Some(Conn::Playing(_))));
        match ok {
            Some((owner, _)) => {
                self.conns.insert(id, Conn::Fetch);
                if let Some(p) = self.player_mut(owner) {
                    p.fetch_conn = Some(id);
                }
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
        let ctx = TickCtx { t, dt: 1.0 / self.opts.rate as f64, snap: t % self.opts.snapshot_every == 0, opts: self.opts };
        match self.runner {
            Some(run) if self.rooms.len() > 1 => {
                let jobs: Vec<SpinLock<&mut Room<S>>> = self.rooms.values_mut().map(SpinLock::new).collect();
                run(&jobs, &ctx);
            }
            _ => {
                for room in self.rooms.values_mut() {
                    room.tick(&ctx);
                }
            }
        }
        // the rooms' results in the order of their codes (the same whatever thread ran a room)
        let mut kicked = Vec::new();
        for room in self.rooms.values_mut() {
            self.out.extend(room.out.drain(..));
            self.close.extend(room.close.drain(..));
            self.stats.take_from(&mut room.stats);
            kicked.append(&mut room.kicked);
        }
        for conn in kicked {
            if matches!(self.conns.get(&conn), Some(Conn::Playing(_))) {
                self.drop_conn(conn);
                self.close.push_back(conn);
                self.stats.event_overflow += 1;
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
}

impl<S: RoomSim + Send> NetServer<S> {
    /// run the rooms of a tick side by side on plat's worker pool (plat::parallel_for, one room per item; the pool
    /// has a thread per core unless `--threads N`), or one after another (false, the default).  Every room is
    /// deterministic as on one thread and the results are merged in the order of the room codes: the messages are
    /// the same bit for bit.  Without threads (the browser without shared memory) the pool runs the rooms in turn.
    pub fn set_parallel(&mut self, on: bool) {
        self.runner = if on { Some(run_parallel::<S>) } else { None };
    }
}

fn run_parallel<S: RoomSim + Send>(jobs: &[SpinLock<&mut Room<S>>], ctx: &TickCtx) {
    let _ = plat::parallel_for(jobs.len(), &|i, _| {
        if let Some(j) = jobs.get(i) {
            j.lock().tick(ctx);
        }
    });
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
