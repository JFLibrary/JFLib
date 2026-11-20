// client -- the client side of the netcode, without I/O: netchan's events in, messages out
//
//   NetClient<M>   joins a room (HELLO -> WELCOME), runs its own ticks ahead of the server (SteeredClock),
//                  predicts its figure with M (Predictor) and sends every tick's input with the ones the
//                  server may still lack; decodes snapshots against the base they name (a ring of the last
//                  64 it decoded, the map's baseline), reconciles its figure with each newer one, and shows
//                  the other entities interp_ms behind the newest snapshot (InterpClock, remotes)
//   fetch          a UDP client whose full state does not fit a datagram is told so in WELCOME:
//                  fetch_request() names the server's WebSocket port and a ticket, the app opens a second
//                  netchan connection there and feeds its events to on_fetch_event (FETCH -> FULL)
//   events         send_event (to the room's RoomSim) and take_event (the room's): reliable, in order, each
//                  delivered once (event.rs); they ride in the INPUTs and SNAPSHOTs
//   smoothing      a correction of the own figure becomes an offset of the drawn position that decays in
//                  smooth_ms (predict::Smoother; above `snap` metres it jumps): drawn() is what a frame
//                  shows, predicted() stays the exact state
//   leave          LEAVE goes out, the server removes the figure at once (NetLink::leave also ends netchan)
//
// Per frame: on_event for every netchan event (main channel), advance(now) -> n, then n times step(map,
// input) with the input of that tick; take_out what to send.  The figure is drawn() (or predicted() and
// predicted_prev() with alpha() for a frame between two ticks); the others are remotes(); events come from
// take_event().

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::ClientEvent;
use physics::Vec3;

use super::clock::{InterpClock, SteeredClock};
use super::event::{self, Inbox, InboxStats, NetEvent, Outbox, OutboxStats};
use super::input::{NetInput, MAX_REDUNDANT};
use super::predict::{Movement, Outcome, Predictor, Smoother};
use super::proto::{self, Base, InputMsg, Welcome};
use super::quant::round_i64;
use super::room::RoomCode;
use super::snapshot::{self, tick_diff, NetState, Schema, Snapshot};

/// a client's settings
#[derive(Clone, Debug)]
pub struct ClientOptions {
    /// the player's name
    pub name: String,
    /// the room to join (None: make one)
    pub room: Option<RoomCode>,
    /// how far behind the newest snapshot the others are shown (ms)
    pub interp_ms: u32,
    /// the margin the inputs aim for (None: the server's)
    pub margin: Option<f64>,
    /// snapshots kept as bases and for interpolation
    pub ring: usize,
    /// inputs every packet repeats at least (the newest and the ones before it)
    pub min_redundancy: usize,
    /// the time constant in which a correction of the drawn own figure decays (ms; 0: no smoothing)
    pub smooth_ms: u32,
    /// a correction larger than this is not smoothed: the picture jumps (m)
    pub snap: f64,
    /// the bytes of the events an INPUT carries at most
    pub event_budget: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        ClientOptions { name: String::from("Spieler"), room: None, interp_ms: 100, margin: None, ring: 64, min_redundancy: 3, smooth_ms: 60, snap: 1.0, event_budget: 400 }
    }
}

/// where a client stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// the channel is not connected yet
    Connecting,
    /// HELLO went out, no WELCOME yet
    Joining,
    /// in a room
    Playing,
    /// the server refused (proto::REFUSE_*)
    Refused(u8),
    /// the channel ended
    Closed,
    /// the client left (leave)
    Left,
}

/// what a client counted
#[derive(Clone, Debug, Default)]
pub struct ClientStats {
    /// snapshots decoded (FULL included)
    pub snapshots: u64,
    /// FULLs decoded
    pub fulls: u64,
    /// snapshots older than the newest (kept as bases only)
    pub stale: u64,
    /// snapshots whose base was not in the ring
    pub no_base: u64,
    /// messages that did not decode
    pub bad: u64,
    /// snapshots before WELCOME (it was lost or overtaken: they are dropped)
    pub early: u64,
    /// payload bytes received (snapshots and FULLs)
    pub bytes_in: u64,
    /// INPUT messages sent and their payload bytes
    pub inputs_sent: u64,
    /// see inputs_sent
    pub bytes_out: u64,
    /// ticks between the server's tick and the client's when WELCOME came (the first estimate)
    pub start_lead: u32,
}

/// another entity as a frame shows it: its states in the two snapshots around the render time and the
/// fraction between them (a == b and t == 0 when the newer one is missing: it holds)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remote {
    /// the net id
    pub id: u16,
    /// the generation
    pub generation: u8,
    /// the state before the render time
    pub a: NetState,
    /// the state after it
    pub b: NetState,
    /// the fraction from a to b
    pub t: f64,
}

/// a field blended linearly
pub fn lerp_field(a: i32, b: i32, t: f64) -> f64 {
    a as f64 + (b as f64 - a as f64) * t
}

/// a field holding an angle in u16 steps (quant::angle_q), blended the short way -> the angle's step as f64
pub fn lerp_angle_field(a: i32, b: i32, t: f64) -> f64 {
    let d = (b as u16).wrapping_sub(a as u16) as i16 as f64;
    a as u16 as f64 + d * t
}

/// the client (see the module header)
pub struct NetClient<M: Movement> {
    opts: ClientOptions,
    mover: M,
    schema: Schema,
    baseline: Snapshot,
    phase: Phase,
    welcome: Option<Welcome>,
    hello_ns: u64,
    now_ns: u64,
    rate: u32,
    dt: f64,
    clock: SteeredClock,
    interp: InterpClock,
    pred: Predictor<M>,
    sent: VecDeque<(u32, NetInput)>,
    have: Option<u32>,
    ring: Vec<Snapshot>,
    newest: Option<u32>,
    full: Option<(u32, u32)>,
    out: VecDeque<Vec<u8>>,
    fetch_out: VecDeque<Vec<u8>>,
    fetch_req: Option<(u16, u64)>,
    stats: ClientStats,
    last_outcome: Option<Outcome>,
    out_ev: Outbox,
    in_ev: Inbox,
    smooth: Smoother,
    smooth_ns: u64,
}

impl<M: Movement> NetClient<M> {
    /// a client of a game with this schema and map baseline (both as the server has them), predicting with
    /// `mover`
    pub fn new(opts: ClientOptions, mover: M, schema: Schema, baseline: Snapshot) -> NetClient<M> {
        NetClient {
            out_ev: Outbox::new(false, opts.event_budget),
            in_ev: Inbox::new(true),
            smooth: Smoother::new(opts.smooth_ms as f64, opts.snap),
            smooth_ns: 0,
            opts,
            mover,
            schema,
            baseline,
            phase: Phase::Connecting,
            welcome: None,
            hello_ns: 0,
            now_ns: 0,
            rate: 60,
            dt: 1.0 / 60.0,
            clock: SteeredClock::new(60, 2.0),
            interp: InterpClock::new(60, 6.0),
            pred: Predictor::new(),
            sent: VecDeque::new(),
            have: None,
            ring: Vec::new(),
            newest: None,
            full: None,
            out: VecDeque::new(),
            fetch_out: VecDeque::new(),
            fetch_req: None,
            stats: ClientStats::default(),
            last_outcome: None,
        }
    }

    /// where it stands
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// the server's WELCOME (in a room)
    pub fn welcome(&self) -> Option<&Welcome> {
        self.welcome.as_ref()
    }

    /// the net id of the own figure (0 before WELCOME)
    pub fn own_id(&self) -> u16 {
        self.welcome.as_ref().map_or(0, |w| w.player)
    }

    /// the movement
    pub fn mover(&self) -> &M {
        &self.mover
    }

    /// the schema
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// what it counted
    pub fn stats(&self) -> &ClientStats {
        &self.stats
    }

    /// the prediction (its counts: steps, mismatches, errors)
    pub fn predictor(&self) -> &Predictor<M> {
        &self.pred
    }

    /// the prediction clock
    pub fn clock(&self) -> &SteeredClock {
        &self.clock
    }

    /// the interpolation clock
    pub fn interp(&self) -> &InterpClock {
        &self.interp
    }

    /// what the last reconciliation found
    pub fn last_outcome(&self) -> Option<Outcome> {
        self.last_outcome
    }

    /// the channel is reliable (a WebSocket): its events go once instead of until acknowledged.  NetLink and the
    /// simulated network set it; without it a client resends over a WebSocket too (harmless: duplicates are dropped)
    pub fn set_reliable(&mut self, on: bool) {
        if self.out_ev.pending() == 0 {
            self.out_ev = Outbox::new(on, self.opts.event_budget);
        }
    }

    /// queue an event for the room's RoomSim (reliable, in order) -> false when refused: not in a room, a channel
    /// over 15, more than event::MAX_EVENT bytes, event::MAX_PENDING not yet acknowledged
    pub fn send_event(&mut self, channel: u8, data: &[u8]) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        self.out_ev.push(NetEvent::new(self.clock.tick(), channel, data))
    }

    /// the next event of the room (each once, in the order the room sent them)
    pub fn take_event(&mut self) -> Option<NetEvent> {
        self.in_ev.take()
    }

    /// what the outbox of the client's events counted
    pub fn events_out(&self) -> OutboxStats {
        self.out_ev.stats()
    }

    /// what the inbox of the room's events counted
    pub fn events_in(&self) -> InboxStats {
        self.in_ev.stats()
    }

    /// events of the client not yet acknowledged by the server
    pub fn events_pending(&self) -> usize {
        self.out_ev.pending()
    }

    /// leave the room: LEAVE goes out (take_out), the server removes the figure at once; the phase is Left and
    /// nothing more is sent.  (NetLink::leave sends it and ends netchan's connection too.)
    pub fn leave(&mut self) {
        if matches!(self.phase, Phase::Joining | Phase::Playing) {
            self.out.push_back(proto::write_leave());
        }
        self.phase = Phase::Left;
    }

    /// what a frame adds to the predicted position of the own figure: the rest of the corrections, decaying
    pub fn smoothing(&self) -> Vec3 {
        self.smooth.offset()
    }

    /// the smoothing's counts: (corrections, of those snapped)
    pub fn smoother(&self) -> &Smoother {
        &self.smooth
    }

    /// where a frame draws the own figure: the predicted position between the last two ticks (alpha) plus the
    /// decaying rest of the corrections (None before the first snapshot)
    pub fn drawn(&self) -> Option<Vec3> {
        Some(self.blended()? + self.smooth.offset())
    }

    // the predicted position between the last two ticks (alpha)
    fn blended(&self) -> Option<Vec3> {
        let cur = M::position(&self.pred.current()?);
        let prev = self.pred.previous().map(|s| M::position(&s)).unwrap_or(cur);
        Some(prev + (cur - prev) * self.clock.alpha())
    }

    /// one of netchan's client events of the main channel at time now_ns
    pub fn on_event(&mut self, map: &physics::World, e: &ClientEvent, now_ns: u64) {
        self.now_ns = now_ns;
        match e {
            ClientEvent::Connected => {
                self.phase = Phase::Joining;
                self.send_hello(now_ns);
            }
            ClientEvent::Message { data, .. } => self.on_message(map, data, false),
            ClientEvent::Disconnected(_) => {
                if !matches!(self.phase, Phase::Refused(_) | Phase::Left) {
                    self.phase = Phase::Closed;
                }
            }
            ClientEvent::Delivered(_) | ClientEvent::Lost(_) => {}
        }
    }

    fn send_hello(&mut self, now_ns: u64) {
        let h = proto::Hello { version: proto::VERSION, room: self.opts.room, name: self.opts.name.clone() };
        self.out.push_back(proto::write_hello(&h));
        self.hello_ns = now_ns;
    }

    /// (UDP whose full state does not fit) the server's WebSocket port and the ticket, once: open a second
    /// netchan connection there (Endpoint::WebSocket) and feed its events to on_fetch_event
    pub fn fetch_request(&mut self) -> Option<(u16, u64)> {
        self.fetch_req.take()
    }

    /// one of netchan's client events of the fetch's connection
    pub fn on_fetch_event(&mut self, map: &physics::World, e: &ClientEvent, now_ns: u64) {
        self.now_ns = now_ns;
        match e {
            ClientEvent::Connected => {
                if let Some(w) = &self.welcome {
                    self.fetch_out.push_back(proto::write_fetch(w.ticket));
                }
            }
            ClientEvent::Message { data, .. } => self.on_message(map, data, true),
            _ => {}
        }
    }

    /// the next message for the main channel
    pub fn take_out(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }

    /// the next message for the fetch's connection
    pub fn take_fetch_out(&mut self) -> Option<Vec<u8>> {
        self.fetch_out.pop_front()
    }

    fn on_message(&mut self, map: &physics::World, data: &[u8], fetch: bool) {
        match proto::msg_type(data) {
            Some(proto::MSG_WELCOME) if !fetch && self.phase == Phase::Joining => match proto::read_welcome(data) {
                Some(w) => self.on_welcome(w),
                None => self.stats.bad += 1,
            },
            Some(proto::MSG_REFUSE) => {
                let r = proto::read_refuse(data).unwrap_or(proto::REFUSE_PROTOCOL);
                if !fetch {
                    self.phase = Phase::Refused(r);
                }
            }
            Some(proto::MSG_SNAPSHOT) | Some(proto::MSG_FULL) if self.phase == Phase::Playing => self.on_snapshot(map, data),
            Some(proto::MSG_SNAPSHOT) if self.phase == Phase::Joining => self.stats.early += 1,
            Some(proto::MSG_WELCOME) if self.phase == Phase::Playing => {}
            _ => self.stats.bad += 1,
        }
    }

    fn on_welcome(&mut self, w: Welcome) {
        self.rate = w.rate as u32;
        self.dt = 1.0 / self.rate as f64;
        let target = self.opts.margin.unwrap_or(w.margin as f64);
        self.clock = SteeredClock::new(self.rate, target);
        let delay = self.opts.interp_ms as f64 * self.rate as f64 / 1000.0;
        self.interp = InterpClock::new(self.rate, delay);
        // the server's tick when an input sent now arrives: its tick then, plus the round trip
        let rtt_ticks = (self.now_ns.saturating_sub(self.hello_ns) as u128 * self.rate as u128).div_ceil(1_000_000_000) as u32;
        let lead = rtt_ticks + target.max(0.0) as u32 + 1;
        self.clock.set_tick(w.tick.wrapping_add(lead));
        self.stats.start_lead = lead;
        if w.fetch {
            self.fetch_req = Some((w.ws_port, w.ticket));
        }
        self.welcome = Some(w);
        self.phase = Phase::Playing;
        self.pred.clear();
        self.sent.clear();
        self.ring.clear();
        self.newest = None;
        self.have = None;
        self.smooth.reset();
    }

    fn ring_get(&self, tick: u32) -> Option<&Snapshot> {
        self.ring.iter().find(|s| s.tick == tick)
    }

    fn on_snapshot(&mut self, map: &physics::World, data: &[u8]) {
        let Some((kind, h, mut r)) = proto::read_snap_header(data) else {
            self.stats.bad += 1;
            return;
        };
        // the event part (whatever the base: the events do not depend on it, and a late snapshot's run is either
        // known already or the continuation)
        if let Some(a) = h.ev_ack {
            self.out_ev.ack(a);
        }
        if h.events {
            match event::read_section(&mut r, h.tick) {
                Some((first, ev)) => self.in_ev.receive(first, ev),
                None => {
                    self.stats.bad += 1;
                    return;
                }
            }
        }
        let base: Option<&Snapshot> = match h.base {
            Base::None => None,
            Base::Map => Some(&self.baseline),
            Base::Back(k) => match self.ring_get(h.tick.wrapping_sub(k as u32)) {
                Some(s) => Some(s),
                None => {
                    self.stats.no_base += 1;
                    return;
                }
            },
        };
        let Some(snap) = snapshot::decode(&mut r, &self.schema, base, Some(&self.baseline), h.tick) else {
            self.stats.bad += 1;
            return;
        };
        self.stats.snapshots += 1;
        self.stats.bytes_in += data.len() as u64;
        if kind == proto::MSG_FULL {
            self.stats.fulls += 1;
            self.full = Some((h.tick, 0));
        }
        let newer = self.newest.is_none_or(|n| tick_diff(h.tick, n) > 0);
        // keep it as a base (a late one too), replacing one of the same tick
        if let Some(i) = self.ring.iter().position(|s| s.tick == h.tick) {
            self.ring[i] = snap;
        } else {
            let at = self.ring.iter().position(|s| tick_diff(s.tick, h.tick) > 0).unwrap_or(self.ring.len());
            self.ring.insert(at, snap);
            while self.ring.len() > self.opts.ring.max(2) {
                self.ring.remove(0);
            }
        }
        if !newer {
            self.stats.stale += 1;
            return;
        }
        self.newest = Some(h.tick);
        self.interp.on_snapshot(h.tick);
        if kind == proto::MSG_SNAPSHOT {
            self.have = Some(h.have);
            if let Some(m) = h.margin {
                self.clock.on_margin(m as i32);
            }
        }
        // the own figure
        let own = self.own_id();
        if own != 0 {
            let st = self.ring_get(h.tick).and_then(|s| s.get(own)).map(|e| e.state);
            if let Some(st) = st {
                let server = M::from_fields(&st.fields[..M::FIELDS.min(st.fields.len())]);
                // (the jump of what a frame shows now: the drawn position stays where it is)
                let before = self.blended();
                let o = self.pred.reconcile(&self.mover, map, h.tick, server, self.dt);
                let after = self.blended();
                match (o, before, after) {
                    (Outcome::Corrected { .. }, Some(b), Some(a)) => self.smooth.correct(a - b),
                    (Outcome::Reset, _, _) => self.smooth.reset(),
                    _ => {}
                }
                self.last_outcome = Some(o);
            }
        }
    }

    /// the clock stands at now_ns -> the ticks due (call step once for each)
    pub fn advance(&mut self, now_ns: u64) -> u32 {
        self.now_ns = now_ns;
        self.interp.advance(now_ns);
        if self.smooth_ns != 0 && now_ns > self.smooth_ns {
            self.smooth.advance((now_ns - self.smooth_ns) as f64 * 1e-9);
        }
        self.smooth_ns = now_ns;
        // (UDP) HELLO or WELCOME lost: HELLO again every 250 ms
        if self.phase == Phase::Joining && now_ns.saturating_sub(self.hello_ns) >= 250_000_000 {
            self.send_hello(now_ns);
        }
        if self.phase != Phase::Playing {
            return 0;
        }
        self.clock.advance(now_ns)
    }

    /// the next tick step predicts
    pub fn tick(&self) -> u32 {
        self.clock.tick()
    }

    /// the clock's place between two ticks (a frame blends predicted_prev and predicted by it)
    pub fn alpha(&self) -> f64 {
        self.clock.alpha()
    }

    /// one tick with this input: predict, queue the INPUT (its view time is set here) -> the tick
    pub fn step(&mut self, map: &physics::World, input: NetInput) -> u32 {
        let t = self.clock.next_tick();
        let mut input = input;
        input.view = match self.interp.time() {
            Some((a, f)) => {
                let lag = tick_diff(t, a) as f64 - f;
                round_i64(lag * 256.0).clamp(0, u16::MAX as i64) as u16
            }
            None => 0,
        };
        self.pred.step(&self.mover, map, t, input, self.dt);
        self.sent.push_back((t, input));
        if let Some(h) = self.have {
            while self.sent.front().is_some_and(|x| tick_diff(x.0, h) <= 0) && self.sent.len() > self.opts.min_redundancy {
                self.sent.pop_front();
            }
        }
        while self.sent.len() > MAX_REDUNDANT {
            self.sent.pop_front();
        }
        let inputs: Vec<NetInput> = self.sent.iter().rev().map(|x| x.1).collect();
        let full = match &mut self.full {
            Some((tick, n)) if *n < 120 => {
                *n += 1;
                Some(*tick)
            }
            _ => None,
        };
        let m = InputMsg { newest: t, inputs, ack: self.newest, full, ev_ack: self.in_ev.ack_for_packet(), events: self.out_ev.take_run(t) };
        let b = proto::write_input(&m);
        self.stats.inputs_sent += 1;
        self.stats.bytes_out += b.len() as u64;
        self.out.push_back(b);
        t
    }

    /// the predicted state of the own figure after the last step (None before the first snapshot)
    pub fn predicted(&self) -> Option<M::State> {
        self.pred.current()
    }

    /// the predicted state a tick before (for alpha)
    pub fn predicted_prev(&self) -> Option<M::State> {
        self.pred.previous()
    }

    /// the newest snapshot decoded
    pub fn newest(&self) -> Option<&Snapshot> {
        self.newest.and_then(|t| self.ring_get(t))
    }

    /// the render time of the others (a tick and the fraction), None before the first snapshot
    pub fn render_time(&self) -> Option<(u32, f64)> {
        self.interp.time()
    }

    /// the other entities at the render time (the own figure left out), in id order
    pub fn remotes(&self, out: &mut Vec<Remote>) {
        out.clear();
        let Some((rt, frac)) = self.interp.time() else { return };
        // the newest snapshot at or before the render time and the oldest after it
        let rel = |s: &Snapshot| tick_diff(s.tick, rt) as f64 - frac;
        let s0 = self.ring.iter().rev().find(|s| rel(s) <= 0.0);
        let s1 = self.ring.iter().find(|s| rel(s) > 0.0);
        let Some(s0) = s0.or(s1) else { return };
        let own = self.own_id();
        for e in &s0.ents {
            if e.id == own {
                continue;
            }
            let pair = match s1 {
                Some(s1) if s1.tick != s0.tick => s1.get(e.id).filter(|x| x.generation == e.generation && x.state.kind == e.state.kind).map(|x| {
                    let span = tick_diff(s1.tick, s0.tick) as f64;
                    let t = (-rel(s0) / span).clamp(0.0, 1.0);
                    (x.state, t)
                }),
                _ => None,
            };
            let (b, t) = pair.unwrap_or((e.state, 0.0));
            out.push(Remote { id: e.id, generation: e.generation, a: e.state, b, t });
        }
    }
}
