// sim -- a room of game 1 on the server: game::net's RoomSim for free-for-all with one hitscan weapon
//
//   MatchConfig   bots, figures at most, the seed, the lag compensation's limit, whether bots shoot
//   Match         the RoomSim: join (a figure at the spawn point farthest from the others; a bot makes room),
//                 leave (a bot takes the place again), step, snapshot (figures and their name tags), priority
//   Figure        a figure's state on the server: movement (movement::MoveState), view, health, kills, deaths,
//                 the hits counter, who ended it, when it comes back, how long it cannot be hurt, a bot's mind
//   Event         what happened in a tick (shots, kills, spawns): for tests and measurements
//   events        what the players learn reliably (game::net's events, rules::GameEvent): every kill to everybody
//                 (the kill feed), every hit to its shooter (the hit marker), players coming and going
//
// A tick (step): figures whose time has come are back at a spawn point; every bot thinks (from the state at the
// start of the tick); every figure moves with its input in id order (Mover::step: body and weapon); the hit volumes
// after the tick are remembered (standing and crouched capsules apart); then every shot of the tick (F_FIRED) is
// cast in id order: the others rewound to the shooter's view time (game::net::lagcomp, at most MAX_REWIND ticks;
// bots see without lag), a ray against the map and the capsules, the nearest hit counts; a hit on the upper half
// sphere is a head shot.  A figure at 0 health is dead (F_DEAD in its movement state: its client predicts it
// standing still) and comes back after RESPAWN_TICKS.  Falling below KILL_Y ends a figure too.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use physics::Vec3;

use game::net::input::NetInput;
use game::net::lagcomp::{hitscan, view_time, Hit, HitHistory};
use game::net::predict::Movement;
use game::net::quant::{angle_f, angle_q, pitch_f};
use game::net::server::{RoomEvent, RoomSim, To};
use game::net::snapshot::{Ent, NetState, Schema, Snapshot};

use crate::bot::{Brain, Seen, Skill};
use crate::map::Map;
use crate::movement::{MoveState, Mover, F_DEAD};
use crate::rules::*;

/// a room's settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchConfig {
    /// bots while the room has space (they make room for players)
    pub bots: usize,
    /// figures at most (players and bots)
    pub max_figures: usize,
    /// the bots' randomness
    pub seed: u64,
    /// the lag compensation's limit (ticks)
    pub max_rewind: u32,
    /// bots shoot
    pub bot_fire: bool,
    /// ticks a second
    pub rate: u32,
    /// hits hurt (false: they are counted and nothing else; the lag compensation's test)
    pub damage: bool,
}

impl Default for MatchConfig {
    fn default() -> Self {
        MatchConfig { bots: 0, max_figures: MAX_FIGURES, seed: 1, max_rewind: MAX_REWIND, bot_fire: true, rate: RATE, damage: true }
    }
}

/// a figure on the server (see the module header)
#[derive(Clone, Debug)]
pub struct Figure {
    pub name: String,
    pub state: MoveState,
    pub yaw: u16,
    pub pitch: i16,
    pub health: i32,
    pub kills: i32,
    pub deaths: i32,
    /// hits it scored and head shots (counters that wrap at 256)
    pub hits: u8,
    pub heads: u8,
    /// who ended it last (0: nobody), and whether by a head shot
    pub killer: u16,
    pub killed_by_head: bool,
    pub generation: u8,
    /// the tick it comes back (while dead)
    pub back_at: u32,
    /// it cannot be hurt before this tick
    pub safe_until: u32,
    /// the yaw of the spawn point it came back at (rules::spawn_dir)
    pub spawn_dir: i32,
    /// the mind of a bot (None: a player)
    pub bot: Option<Brain>,
}

impl Figure {
    pub fn is_bot(&self) -> bool {
        self.bot.is_some()
    }
}

/// what happened in a tick
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// a shot: who, from where, along what, and what it hit (a figure, whether on the head) where
    Shot { tick: u32, shooter: u16, from: Vec3, dir: Vec3, hit: Option<(u16, bool)>, at: Option<Vec3>, view: u16 },
    /// a figure was ended (by itself when it fell out of the map)
    Kill { tick: u32, killer: u16, victim: u16, head: bool },
    /// a figure came (back) at a spawn point
    Spawn { tick: u32, id: u16, spawn: usize },
}

/// what a match counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchStats {
    pub shots: u64,
    pub hits: u64,
    pub heads: u64,
    pub kills: u64,
    pub falls: u64,
    pub spawns: u64,
    /// shots whose view lay further back than the limit (clamped)
    pub clamped: u64,
}

/// a room of game 1 (see the module header)
pub struct Match {
    cfg: MatchConfig,
    map: Arc<Map>,
    mover: Mover,
    schema: Schema,
    baseline: Snapshot,
    figs: BTreeMap<u16, Figure>,
    gens: BTreeMap<u16, u8>,
    stand: HitHistory,
    crouch: HitHistory,
    spawned: u32,
    bot_names: u32,
    tick: u32,
    stats: MatchStats,
    /// events of the ticks since the last take_events (kept only when record is on)
    events: Vec<Event>,
    record: bool,
    scratch: Vec<(u16, Vec3)>,
    seen: Vec<Seen>,
    /// the players' events until the server takes them (take_events)
    net: Vec<RoomEvent>,
}

impl Match {
    /// a room on `map` with these settings; its bots come at once
    pub fn new(cfg: MatchConfig, map: Arc<Map>) -> Match {
        let cap = cfg.max_rewind as usize + 8;
        let mut m = Match {
            cfg,
            map,
            mover: Mover::default(),
            schema: schema(),
            baseline: Snapshot::new(0),
            figs: BTreeMap::new(),
            gens: BTreeMap::new(),
            stand: HitHistory::new(cap),
            crouch: HitHistory::new(cap),
            spawned: 0,
            bot_names: 0,
            tick: 0,
            stats: MatchStats::default(),
            events: Vec::new(),
            record: false,
            scratch: Vec::new(),
            seen: Vec::new(),
            net: Vec::new(),
        };
        m.fill_bots();
        m
    }

    /// the map's baseline: empty (part 1 replicates no props; every client builds the same)
    pub fn empty_baseline() -> Snapshot {
        Snapshot::new(0)
    }

    pub fn config(&self) -> &MatchConfig {
        &self.cfg
    }

    pub fn map(&self) -> &Arc<Map> {
        &self.map
    }

    pub fn mover(&self) -> &Mover {
        &self.mover
    }

    /// the figures by net id
    pub fn figures(&self) -> &BTreeMap<u16, Figure> {
        &self.figs
    }

    /// the figures, to change (tests: place a figure, end its protection)
    pub fn figures_mut(&mut self) -> &mut BTreeMap<u16, Figure> {
        &mut self.figs
    }

    pub fn stats(&self) -> MatchStats {
        self.stats
    }

    /// keep the events of every tick (take_events hands them out)
    pub fn set_record(&mut self, on: bool) {
        self.record = on;
        self.events.clear();
    }

    /// whether it keeps the events (set_record)
    pub fn recording(&self) -> bool {
        self.record
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    /// the tick of the last step
    pub fn tick(&self) -> u32 {
        self.tick
    }

    // an event for the players (kept until the server takes it; a room without a server keeps the last 1024)
    fn tell(&mut self, to: To, e: GameEvent) {
        let (channel, data) = e.encode();
        if self.net.len() >= 1024 {
            self.net.remove(0);
        }
        self.net.push(RoomEvent { to, channel, data });
    }

    fn free_id(&self) -> Option<u16> {
        (1..=255u16).find(|i| !self.figs.contains_key(i))
    }

    fn bots(&self) -> usize {
        self.figs.values().filter(|f| f.is_bot()).count()
    }

    /// the spawn point farthest from every living figure (ties: the next in turn)
    fn pick_spawn(&mut self, me: u16) -> usize {
        let n = self.map.spawns.len();
        self.spawned = self.spawned.wrapping_add(1);
        let start = self.spawned as usize % n;
        let mut best = (f64::NEG_INFINITY, start);
        for j in 0..n {
            let k = (start + j) % n;
            let p = self.map.spawns[k].feet;
            let mut near = f64::INFINITY;
            for (id, f) in &self.figs {
                if *id == me || f.state.dead() {
                    continue;
                }
                near = near.min((f.state.feet() - p).length_sq());
            }
            if near > best.0 {
                best = (near, k);
            }
        }
        best.1
    }

    fn place(&mut self, id: u16) {
        let k = self.pick_spawn(id);
        let sp = self.map.spawns[k];
        let tick = self.tick;
        let rate = self.cfg.rate;
        if let Some(f) = self.figs.get_mut(&id) {
            f.state = MoveState::at(sp.feet);
            f.health = HEALTH;
            f.yaw = angle_q(sp.yaw);
            f.pitch = 0;
            f.safe_until = tick.wrapping_add(PROTECT_TICKS * rate / RATE);
            f.spawn_dir = spawn_dir(sp.yaw);
            if let Some(b) = &mut f.bot {
                b.respawned(sp.yaw);
            }
        }
        self.stats.spawns += 1;
        if self.record {
            self.events.push(Event::Spawn { tick, id, spawn: k });
        }
    }

    fn add_figure(&mut self, name: String, bot: Option<Brain>) -> Option<u16> {
        let id = self.free_id()?;
        let generation = self.gens.get(&id).map_or(0, |g| g.wrapping_add(1));
        self.gens.insert(id, generation);
        let f = Figure {
            name,
            state: MoveState::default(),
            yaw: 0,
            pitch: 0,
            health: HEALTH,
            kills: 0,
            deaths: 0,
            hits: 0,
            heads: 0,
            killer: 0,
            killed_by_head: false,
            generation,
            back_at: 0,
            safe_until: 0,
            spawn_dir: 0,
            bot,
        };
        self.figs.insert(id, f);
        self.place(id);
        Some(id)
    }

    /// bots up to cfg.bots while there is space
    fn fill_bots(&mut self) {
        while self.bots() < self.cfg.bots && self.figs.len() < self.cfg.max_figures {
            let k = self.bot_names;
            self.bot_names += 1;
            let name = format!("Bot {}", BOT_NAMES[k as usize % BOT_NAMES.len()]);
            let seed = self.cfg.seed.wrapping_mul(0x2545_f491).wrapping_add(k as u64 * 7919 + 1);
            let brain = Brain::new(seed, Skill::default(), 0.0, self.cfg.rate);
            if self.add_figure(name, Some(brain)).is_none() {
                break;
            }
        }
    }

    /// a bot leaves (the one with the highest id) -> whether there was one
    fn drop_bot(&mut self) -> bool {
        let Some(id) = self.figs.iter().rev().find(|(_, f)| f.is_bot()).map(|(id, _)| *id) else { return false };
        self.figs.remove(&id);
        true
    }

    /// add a player directly (tests): as RoomSim::join
    pub fn join_player(&mut self, name: &str) -> Option<u16> {
        self.join(name)
    }

    fn kill(&mut self, victim: u16, killer: u16, head: bool) {
        let tick = self.tick;
        let back = tick.wrapping_add(RESPAWN_TICKS * self.cfg.rate / RATE);
        if let Some(v) = self.figs.get_mut(&victim) {
            v.state = v.state.killed();
            v.health = 0;
            v.deaths += 1;
            v.killer = killer;
            v.killed_by_head = head;
            v.back_at = back;
        }
        if killer != victim {
            if let Some(k) = self.figs.get_mut(&killer) {
                k.kills += 1;
            }
            self.stats.kills += 1;
        } else {
            self.stats.falls += 1;
        }
        self.tell(To::All, GameEvent::Kill { killer, victim, head });
        if self.record {
            self.events.push(Event::Kill { tick, killer, victim, head });
        }
    }

    fn shoot(&mut self, shooter: u16, view: u16) {
        let tick = self.tick;
        let Some(f) = self.figs.get(&shooter) else { return };
        let from = self.mover.eye(&f.state);
        let (dir, _, _) = game::camera::basis(angle_f(f.yaw), pitch_f(f.pitch));
        if view as u32 > self.cfg.max_rewind * 256 {
            self.stats.clamped += 1;
        }
        let (vt, frac) = view_time(tick, view, self.cfg.max_rewind);
        let mut figs = core::mem::take(&mut self.scratch);
        let mut best: Option<(f64, u16, Vec3, bool)> = None;
        let mut wall: Option<f64> = None;
        for crouched in [false, true] {
            let hist = if crouched { &self.crouch } else { &self.stand };
            hist.rewind(vt, frac, &mut figs);
            let shape = self.mover.shape(crouched);
            match hitscan(&self.map.world, &figs, &shape, shooter, from, dir, RANGE) {
                Some(Hit::Figure { id, t, point }) => {
                    if best.is_none_or(|b| t < b.0 || (t == b.0 && id < b.1)) {
                        let c = figs.iter().find(|x| x.0 == id).map_or(point, |x| x.1);
                        best = Some((t, id, point, self.mover.head(crouched, c, point)));
                    }
                }
                Some(Hit::World { t, .. }) => wall = Some(t),
                None => {}
            }
        }
        self.scratch = figs;
        self.stats.shots += 1;
        if let Some(s) = self.figs.get_mut(&shooter) {
            s.safe_until = tick; // shooting ends the protection
        }
        let hit = best.filter(|b| wall.is_none_or(|w| b.0 <= w));
        let mut result = None;
        if let Some((_, victim, _, head)) = hit {
            let alive = self.figs.get(&victim).is_some_and(|v| !v.state.dead() && (tick.wrapping_sub(v.safe_until) as i32) >= 0);
            if alive {
                result = Some((victim, head));
                self.stats.hits += 1;
                if head {
                    self.stats.heads += 1;
                }
                if let Some(s) = self.figs.get_mut(&shooter) {
                    s.hits = s.hits.wrapping_add(1);
                    if head {
                        s.heads = s.heads.wrapping_add(1);
                    }
                }
                let damage = self.cfg.damage;
                let dead = match self.figs.get_mut(&victim).filter(|_| damage) {
                    Some(v) => {
                        v.health -= if head { HEAD_DAMAGE } else { DAMAGE };
                        v.health <= 0
                    }
                    None => false,
                };
                self.tell(To::Player(shooter), GameEvent::Hit { victim, head, kill: dead });
                if dead {
                    self.kill(victim, shooter, head);
                }
            }
        }
        if self.record {
            let at = hit.map(|b| b.2).or_else(|| wall.map(|w| from + dir * w));
            self.events.push(Event::Shot { tick, shooter, from, dir, hit: result, at, view });
        }
    }

    /// a figure's replicated state
    pub fn figure_state(&self, f: &Figure) -> NetState {
        let mut fields = [0i32; FIG_FIELDS];
        Mover::to_fields(&f.state, &mut fields);
        fields[F_YAW] = f.yaw as i32;
        fields[F_PITCH] = f.pitch as i32;
        fields[F_HEALTH] = f.health;
        fields[F_KILLS] = f.kills;
        fields[F_DEATHS] = f.deaths;
        let mut info = (f.spawn_dir & 63) << INFO_SPAWN_SHIFT;
        if (self.tick.wrapping_sub(f.safe_until) as i32) < 0 {
            info |= INFO_PROTECTED;
        }
        fields[F_INFO] = info;
        NetState::new(KIND_FIG, &fields)
    }
}

impl RoomSim for Match {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn baseline(&self) -> &Snapshot {
        &self.baseline
    }

    fn join(&mut self, name: &str) -> Option<u16> {
        if self.figs.len() >= self.cfg.max_figures && !self.drop_bot() {
            return None;
        }
        let id = self.add_figure(String::from(name), None)?;
        self.tell(To::AllBut(id), GameEvent::Joined { id, name: String::from(name) });
        Some(id)
    }

    fn leave(&mut self, player: u16) {
        if let Some(f) = self.figs.remove(&player) {
            self.tell(To::All, GameEvent::Left { id: player, name: f.name });
        }
        self.fill_bots();
    }

    fn take_events(&mut self, out: &mut Vec<RoomEvent>) {
        out.append(&mut self.net);
    }

    fn step(&mut self, tick: u32, dt: f64, inputs: &[(u16, NetInput, bool)]) {
        self.tick = tick;
        // back from the dead
        let due: Vec<u16> = self.figs.iter().filter(|(_, f)| f.state.dead() && (tick.wrapping_sub(f.back_at) as i32) >= 0).map(|(id, _)| *id).collect();
        for id in due {
            self.place(id);
        }
        // the bots think (from the state at the start of the tick)
        let mut bot_inputs: Vec<(u16, NetInput)> = Vec::new();
        let ids: Vec<u16> = self.figs.iter().filter(|(_, f)| f.is_bot()).map(|(id, _)| *id).collect();
        for id in ids {
            self.seen.clear();
            for (oid, o) in &self.figs {
                if *oid == id {
                    continue;
                }
                let hittable = !o.state.dead() && (tick.wrapping_sub(o.safe_until) as i32) >= 0;
                self.seen.push(Seen { id: *oid, centre: self.mover.centre(&o.state), hittable });
            }
            let fire = self.cfg.bot_fire;
            let (map, mover, seen) = (&self.map, &self.mover, &self.seen);
            if let Some(f) = self.figs.get_mut(&id) {
                if let Some(b) = &mut f.bot {
                    let mut i = b.think(tick, &f.state, mover, map, seen);
                    if !fire {
                        i.buttons &= !BTN_FIRE;
                    }
                    bot_inputs.push((id, i));
                }
            }
        }
        // every figure moves (id order)
        let ids: Vec<u16> = self.figs.keys().copied().collect();
        let mut views: Vec<(u16, u16)> = Vec::new();
        for id in ids {
            let input = inputs.iter().find(|x| x.0 == id).map(|x| x.1).or_else(|| bot_inputs.iter().find(|x| x.0 == id).map(|x| x.1));
            let Some(input) = input else { continue };
            let (map, mover) = (&self.map, &self.mover);
            let Some(f) = self.figs.get_mut(&id) else { continue };
            if !f.state.dead() {
                f.yaw = input.yaw;
                f.pitch = input.pitch;
            }
            f.state = mover.step(&map.world, &f.state, &input, dt);
            if f.state.fired() {
                views.push((id, input.view));
            }
        }
        // out of the map
        let fallen: Vec<u16> = self.figs.iter().filter(|(_, f)| !f.state.dead() && f.state.feet().y < KILL_Y).map(|(id, _)| *id).collect();
        for id in fallen {
            self.kill(id, id, false);
        }
        // the hit volumes after the tick
        let (mut s, mut c) = (Vec::new(), Vec::new());
        for (id, f) in &self.figs {
            if f.state.flags & F_DEAD != 0 {
                continue;
            }
            let p = self.mover.centre(&f.state);
            if f.state.crouched() { c.push((*id, p)) } else { s.push((*id, p)) }
        }
        self.stand.record(tick, &s);
        self.crouch.record(tick, &c);
        // the shots of the tick
        for (id, view) in views {
            let alive = self.figs.get(&id).is_some_and(|f| !f.state.dead());
            if alive {
                self.shoot(id, view);
            }
        }
    }

    fn snapshot(&self, tick: u32, out: &mut Snapshot) {
        out.tick = tick;
        out.ents.clear();
        for (id, f) in &self.figs {
            out.ents.push(Ent { id: *id, generation: f.generation, state: self.figure_state(f) });
        }
        for (id, f) in &self.figs {
            let mut fields = [0i32; TAG_FIELDS];
            fields[0] = *id as i32;
            name_fields(&f.name, &mut fields[1..]);
            out.ents.push(Ent { id: TAG_BASE + *id, generation: f.generation, state: NetState::new(KIND_TAG, &fields) });
        }
    }

    fn priority(&self, _viewer: u16, e: &Ent) -> u32 {
        if e.state.kind == KIND_FIG { 64 } else { 4 }
    }
}
