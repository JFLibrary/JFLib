// arena -- the netcode's reference game: a square arena with walls and crates, walking figures
// (walker::Walker), props (some move along circles) and a hitscan weapon with lag compensation.  The
// tests, the bots and the measurements of G7e run it; G7h can read it as an example of a RoomSim
//
//   ArenaConfig    its size, the props and how many of them move, the rate, the lag compensation's limit
//   schema()       KIND_PLAYER (13 fields: the walker's 7, yaw, pitch, health, kills, deaths, the tick of the
//                  last shot) and KIND_PROP (7: the position, the rotation's smallest three)
//   map(cfg)       the static map as a physics::World (the same on the server and every client)
//   baseline(cfg)  the props as the map places them (the clients' first base)
//   Arena          the RoomSim: join (a figure at a spawn point), leave, step (move every figure with its
//                  input, move the props, remember the hit volumes, fire: rewind the others to the shooter's
//                  view time, cast the ray), snapshot, priority (figures over props, near over far)
//
// The props are not in the physics world: they do not block the figures in this first part, so the map
// every client predicts against stays the server's.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use physics::math::{sincos, sqrt};
use physics::{Iso, Quat, Shape, Vec3};

use super::input::NetInput;
use super::lagcomp::{hitscan, view_time, Hit, HitHistory};
use super::predict::Movement;
use super::quant::{angle_f, pitch_f, pos_q, rot_q};
use super::server::RoomSim;
use super::snapshot::{Ent, NetState, Schema, Snapshot};
use super::walker::{WalkState, Walker, BTN_FIRE};

/// the kind of a figure
pub const KIND_PLAYER: u8 = 1;
/// the kind of a prop
pub const KIND_PROP: u8 = 2;
/// a figure's fields
pub const PLAYER_FIELDS: usize = 13;
/// a prop's fields
pub const PROP_FIELDS: usize = 7;
/// the fields of a figure after the walker's: yaw, pitch, health, kills, deaths, shot
pub const F_YAW: usize = 7;
/// see F_YAW
pub const F_PITCH: usize = 8;
/// see F_YAW
pub const F_HEALTH: usize = 9;
/// see F_YAW
pub const F_KILLS: usize = 10;
/// see F_YAW
pub const F_DEATHS: usize = 11;
/// see F_YAW
pub const F_SHOT: usize = 12;
/// the first net id of the props (figures have 1 ..= 255)
pub const FIRST_PROP: u16 = 1000;
/// a hit's damage, a figure's health
pub const DAMAGE: i32 = 25;
/// see DAMAGE
pub const HEALTH: i32 = 100;
/// ticks between two shots
pub const FIRE_TICKS: u32 = 6;
/// how far a shot reaches (m)
pub const RANGE: f64 = 100.0;

/// the arena's settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArenaConfig {
    /// half the side of the square (m)
    pub half: f64,
    /// props
    pub props: usize,
    /// of those, how many move (the first ones)
    pub moving: usize,
    /// of the others, how many stand away from the map's place from the start (moved earlier in a round:
    /// they make the full state of a join larger, not the snapshots after it)
    pub displaced: usize,
    /// crates on the floor (static, in the map)
    pub crates: usize,
    /// ticks a second (the props' motion)
    pub rate: u32,
    /// how far back hits are rewound at most (ticks)
    pub max_rewind: u32,
    /// figures may shoot
    pub fire: bool,
}

impl Default for ArenaConfig {
    fn default() -> Self {
        ArenaConfig { half: 30.0, props: 0, moving: 0, displaced: 0, crates: 12, rate: 60, max_rewind: 12, fire: true }
    }
}

/// the schema of the arena's entities
pub fn schema() -> Schema {
    Schema::new().with(KIND_PLAYER, PLAYER_FIELDS).with(KIND_PROP, PROP_FIELDS)
}

/// a number from an index, the same everywhere (splitmix64)
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn unit(x: u64) -> f64 {
    (x >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
}

/// the crates of the map: (centre, half extents)
pub fn crates(cfg: &ArenaConfig) -> Vec<(Vec3, Vec3)> {
    let mut v = Vec::new();
    for i in 0..cfg.crates {
        let r = mix(i as u64 * 3 + 1);
        let x = (unit(r) * 2.0 - 1.0) * cfg.half * 0.7;
        let z = (unit(mix(r)) * 2.0 - 1.0) * cfg.half * 0.7;
        let h = 0.4 + 0.6 * unit(mix(r ^ 7));
        // (the middle stays free for the spawn ring)
        let (x, z) = if x * x + z * z < 16.0 { (x + 6.0, z) } else { (x, z) };
        v.push((Vec3::new(x, h, z), Vec3::new(0.7, h, 0.7)));
    }
    v
}

/// the static map: a floor (its top at y = 0), four walls, the crates
pub fn map(cfg: &ArenaConfig) -> physics::World {
    let mut w = physics::World::new();
    let h = cfg.half;
    w.add(Shape::cuboid(Vec3::new(h + 1.0, 0.5, h + 1.0)), Iso::at(Vec3::new(0.0, -0.5, 0.0)));
    for (c, e) in [
        (Vec3::new(h + 0.5, 1.5, 0.0), Vec3::new(0.5, 1.5, h + 1.0)),
        (Vec3::new(-h - 0.5, 1.5, 0.0), Vec3::new(0.5, 1.5, h + 1.0)),
        (Vec3::new(0.0, 1.5, h + 0.5), Vec3::new(h + 1.0, 1.5, 0.5)),
        (Vec3::new(0.0, 1.5, -h - 0.5), Vec3::new(h + 1.0, 1.5, 0.5)),
    ] {
        w.add(Shape::cuboid(e), Iso::at(c));
    }
    for (c, e) in crates(cfg) {
        w.add(Shape::cuboid(e), Iso::at(c));
    }
    w
}

/// where the k-th figure appears (a ring round the middle)
pub fn spawn_point(cfg: &ArenaConfig, k: u32) -> Vec3 {
    let a = (k % 16) as f64 * (core::f64::consts::TAU / 16.0) + (k / 16) as f64 * 0.2;
    let (s, c) = sincos(a);
    let r = (cfg.half * 0.15).min(4.0);
    Vec3::new(r * s, 0.001, r * c)
}

struct Prop {
    id: u16,
    home: Vec3,
    turn: f64,
    /// (radius, angular speed rad/s, phase) of a moving prop
    path: Option<(f64, f64, f64)>,
    fields: [i32; PROP_FIELDS],
}

fn prop_fields(p: Vec3, q: Quat) -> [i32; PROP_FIELDS] {
    let r = rot_q(q);
    [pos_q(p.x), pos_q(p.y), pos_q(p.z), r[0], r[1], r[2], r[3]]
}

fn props(cfg: &ArenaConfig) -> Vec<Prop> {
    let mut v = Vec::with_capacity(cfg.props);
    let n = cfg.props.max(1);
    let side = sqrt(n as f64) as usize + 1;
    for i in 0..cfg.props {
        let r = mix(0x5eed_0000 + i as u64);
        let gx = (i % side) as f64 + 0.5;
        let gz = (i / side) as f64 + 0.5;
        let x = (gx / side as f64 * 2.0 - 1.0) * cfg.half * 0.9;
        let z = (gz / side as f64 * 2.0 - 1.0) * cfg.half * 0.9;
        let home = Vec3::new(x, 0.25 + unit(r) * 2.0, z);
        let turn = unit(mix(r)) * core::f64::consts::TAU;
        let path = (i < cfg.moving).then(|| (0.5 + unit(mix(r ^ 1)) * 2.0, 0.5 + unit(mix(r ^ 2)) * 1.5, unit(mix(r ^ 3)) * core::f64::consts::TAU));
        let fields = prop_fields(home, Quat::from_axis_angle(Vec3::Y, turn));
        v.push(Prop { id: FIRST_PROP + i as u16, home, turn, path, fields });
    }
    v
}

// the displaced props: half a metre to three metres from their place, turned
fn displace(mut v: Vec<Prop>, cfg: &ArenaConfig) -> Vec<Prop> {
    let from = cfg.moving.min(v.len());
    let to = (from + cfg.displaced).min(v.len());
    for (k, p) in v[from..to].iter_mut().enumerate() {
        let r = mix(0xd15b_0000 + k as u64);
        let (s, c) = sincos(unit(r) * core::f64::consts::TAU);
        let d = 0.5 + 2.5 * unit(mix(r));
        p.home = Vec3::new(p.home.x + d * c, p.home.y, p.home.z + d * s);
        p.turn += 1.0 + unit(mix(r ^ 5));
        p.fields = prop_fields(p.home, Quat::from_axis_angle(Vec3::Y, p.turn));
    }
    v
}

/// the props as the map places them: every client's first base
pub fn baseline(cfg: &ArenaConfig) -> Snapshot {
    let mut s = Snapshot::new(0);
    for p in props(cfg) {
        s.ents.push(Ent { id: p.id, generation: 0, state: NetState::new(KIND_PROP, &p.fields) });
    }
    s
}

/// a figure of the arena
#[derive(Clone, Debug)]
pub struct Figure {
    /// its name
    pub name: String,
    /// the walker's state
    pub state: WalkState,
    /// the view (quant steps)
    pub yaw: u16,
    /// see yaw
    pub pitch: i16,
    /// health 0 ..= 100
    pub health: i32,
    /// hits that ended another figure
    pub kills: i32,
    /// how often it was ended
    pub deaths: i32,
    /// the tick of its last shot
    pub shot: u32,
    /// its generation (the net id's)
    pub generation: u8,
    spawn: u32,
}

/// what the arena counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaStats {
    /// shots fired
    pub shots: u64,
    /// of those, hits on a figure
    pub hits: u64,
    /// shots whose view time lay further back than max_rewind (clamped)
    pub clamped: u64,
    /// figures ended
    pub kills: u64,
    /// the shooters' view lag summed over the shots (1/256 ticks) and the largest
    pub view_sum: u64,
    /// see view_sum
    pub view_max: u32,
}

/// the reference RoomSim (see the module header)
pub struct Arena {
    cfg: ArenaConfig,
    map: physics::World,
    walker: Walker,
    schema: Schema,
    baseline: Snapshot,
    figures: BTreeMap<u16, Figure>,
    gens: BTreeMap<u16, u8>,
    props: Vec<Prop>,
    history: HitHistory,
    joined: u32,
    stats: ArenaStats,
    scratch: Vec<(u16, Vec3)>,
}

impl Arena {
    /// an arena with these settings
    pub fn new(cfg: ArenaConfig) -> Arena {
        Arena {
            map: map(&cfg),
            walker: Walker::default(),
            schema: schema(),
            baseline: baseline(&cfg),
            figures: BTreeMap::new(),
            gens: BTreeMap::new(),
            props: displace(props(&cfg), &cfg),
            history: HitHistory::new(cfg.max_rewind as usize + 8),
            joined: 0,
            stats: ArenaStats::default(),
            scratch: Vec::new(),
            cfg,
        }
    }

    /// the settings
    pub fn config(&self) -> &ArenaConfig {
        &self.cfg
    }

    /// the map
    pub fn map(&self) -> &physics::World {
        &self.map
    }

    /// the movement
    pub fn walker(&self) -> &Walker {
        &self.walker
    }

    /// the figures by net id
    pub fn figures(&self) -> &BTreeMap<u16, Figure> {
        &self.figures
    }

    /// the remembered hit volumes
    pub fn history(&self) -> &HitHistory {
        &self.history
    }

    /// what it counted
    pub fn stats(&self) -> ArenaStats {
        self.stats
    }

    /// put a figure somewhere else (a teleport: its client reconciles)
    pub fn teleport(&mut self, id: u16, feet: Vec3) {
        if let Some(f) = self.figures.get_mut(&id) {
            f.state = WalkState::at(feet);
        }
    }

    fn respawn(&mut self, id: u16) {
        let cfg = self.cfg;
        self.joined += 1;
        let k = self.joined;
        if let Some(f) = self.figures.get_mut(&id) {
            f.state = WalkState::at(spawn_point(&cfg, k));
            f.health = HEALTH;
            f.spawn = k;
        }
    }

    fn fire(&mut self, tick: u32, id: u16, input: &NetInput) {
        let Some(f) = self.figures.get(&id) else { return };
        if tick.wrapping_sub(f.shot) < FIRE_TICKS && f.shot != 0 {
            return;
        }
        let eye = f.state.feet() + Vec3::new(0.0, self.walker.eye(), 0.0);
        let (forward, _, _) = crate::camera::basis(angle_f(input.yaw), pitch_f(input.pitch));
        if input.view as u32 > self.cfg.max_rewind * 256 {
            self.stats.clamped += 1;
        }
        self.stats.view_sum += input.view as u64;
        self.stats.view_max = self.stats.view_max.max(input.view as u32);
        let (vt, frac) = view_time(tick, input.view, self.cfg.max_rewind);
        let mut figs = core::mem::take(&mut self.scratch);
        self.history.rewind(vt, frac, &mut figs);
        let hit = hitscan(&self.map, &figs, &self.walker.shape(), id, eye, forward, RANGE);
        self.scratch = figs;
        self.stats.shots += 1;
        if let Some(f) = self.figures.get_mut(&id) {
            f.shot = tick;
        }
        if let Some(Hit::Figure { id: victim, .. }) = hit {
            self.stats.hits += 1;
            let dead = match self.figures.get_mut(&victim) {
                Some(v) => {
                    v.health -= DAMAGE;
                    v.health <= 0
                }
                None => false,
            };
            if dead {
                self.stats.kills += 1;
                if let Some(v) = self.figures.get_mut(&victim) {
                    v.deaths += 1;
                }
                if let Some(s) = self.figures.get_mut(&id) {
                    s.kills += 1;
                }
                self.respawn(victim);
            }
        }
    }

    /// a figure's replicated state
    pub fn figure_state(&self, f: &Figure) -> NetState {
        let mut fields = [0i32; PLAYER_FIELDS];
        Walker::to_fields(&f.state, &mut fields);
        fields[F_YAW] = f.yaw as i32;
        fields[F_PITCH] = f.pitch as i32;
        fields[F_HEALTH] = f.health;
        fields[F_KILLS] = f.kills;
        fields[F_DEATHS] = f.deaths;
        fields[F_SHOT] = (f.shot & 0xffff) as i32;
        NetState::new(KIND_PLAYER, &fields)
    }
}

impl RoomSim for Arena {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn baseline(&self) -> &Snapshot {
        &self.baseline
    }

    fn join(&mut self, name: &str) -> Option<u16> {
        let id = (1..=255u16).find(|i| !self.figures.contains_key(i))?;
        let generation = self.gens.get(&id).map_or(0, |g| g.wrapping_add(1));
        self.gens.insert(id, generation);
        self.joined += 1;
        let k = self.joined;
        let f = Figure { name: String::from(name), state: WalkState::at(spawn_point(&self.cfg, k)), yaw: 0, pitch: 0, health: HEALTH, kills: 0, deaths: 0, shot: 0, generation, spawn: k };
        self.figures.insert(id, f);
        Some(id)
    }

    fn leave(&mut self, player: u16) {
        self.figures.remove(&player);
    }

    fn step(&mut self, tick: u32, dt: f64, inputs: &[(u16, NetInput, bool)]) {
        for (id, input, _) in inputs {
            let Some(f) = self.figures.get_mut(id) else { continue };
            f.yaw = input.yaw;
            f.pitch = input.pitch;
            f.state = self.walker.step(&self.map, &f.state, input, dt);
            if f.state.feet().y < -50.0 {
                let k = f.spawn;
                f.state = WalkState::at(spawn_point(&self.cfg, k));
            }
        }
        let t = tick as f64 / self.cfg.rate as f64;
        for p in self.props.iter_mut() {
            if let Some((r, w, ph)) = p.path {
                let (s, c) = sincos(w * t + ph);
                let at = Vec3::new(p.home.x + r * c, p.home.y, p.home.z + r * s);
                let q = Quat::from_axis_angle(Vec3::Y, p.turn + w * t);
                p.fields = prop_fields(at, q);
            }
        }
        let c = self.walker.centre();
        let figs: Vec<(u16, Vec3)> = self.figures.iter().map(|(id, f)| (*id, f.state.feet() + c)).collect();
        self.history.record(tick, &figs);
        if self.cfg.fire {
            for (id, input, _) in inputs {
                if input.buttons & BTN_FIRE != 0 {
                    self.fire(tick, *id, input);
                }
            }
        }
    }

    fn snapshot(&self, tick: u32, out: &mut Snapshot) {
        out.tick = tick;
        out.ents.clear();
        for (id, f) in &self.figures {
            out.ents.push(Ent { id: *id, generation: f.generation, state: self.figure_state(f) });
        }
        for p in &self.props {
            out.ents.push(Ent { id: p.id, generation: 0, state: NetState::new(KIND_PROP, &p.fields) });
        }
    }

    fn priority(&self, viewer: u16, e: &Ent) -> u32 {
        if e.state.kind == KIND_PLAYER {
            return 64;
        }
        let Some(v) = self.figures.get(&viewer) else { return 1 };
        let p = v.state.pos;
        let dx = (e.state.fields[0] as f64 - p[0] as f64) / 1024.0;
        let dz = (e.state.fields[2] as f64 - p[2] as f64) / 1024.0;
        let d2 = dx * dx + dz * dz;
        if d2 < 400.0 {
            16
        } else if d2 < 2500.0 {
            4
        } else {
            1
        }
    }
}
