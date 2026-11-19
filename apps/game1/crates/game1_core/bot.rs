// bot -- a server-side player of game 1 (it fills rooms for tests and for people alone): it wanders between the
// map's way points, notices a figure it can see within its field of view, reacts after a delay, turns towards it
// with an aiming error that shrinks over a few tenths of a second, strafes and shoots while it can see it.  It
// reads the true state of the room (the server's) but only what a line of sight through the map allows; all its
// randomness comes from its seed, so a match with bots is the same on every target
//
//   Skill        reaction (ticks), aiming error at first (radians) and how fast it shrinks, turn rate, the
//                angle within which it fires, how far it sees
//   Brain::think the input of one tick (NetInput, as a client would send it; view 0: no lag to compensate)

use alloc::vec::Vec;

use physics::math::{atan2, sqrt};
use physics::Vec3;

use game::net::input::NetInput;
use game::net::quant::{angle_q, axis_q, pitch_q};

use crate::map::Map;
use crate::movement::{MoveState, Mover};
use crate::rules::{BTN_FIRE, BTN_JUMP, BTN_RELOAD, MAGAZINE};

/// how good a bot is
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skill {
    /// ticks from seeing a figure to reacting, at least and the random part on top
    pub react: u32,
    pub react_spread: u32,
    /// the aiming error when it starts aiming (radians, yaw; pitch half of it), and the factor it shrinks by a tick
    pub error: f64,
    pub settle: f64,
    /// a small error on top every tick (radians)
    pub jitter: f64,
    /// the fastest turn (radians a second)
    pub turn: f64,
    /// it fires when its aim is this close to the target (radians)
    pub fire_cone: f64,
    /// how far it sees (m), and half its field of view for noticing (radians)
    pub sight: f64,
    pub fov: f64,
}

impl Default for Skill {
    fn default() -> Self {
        Skill { react: 14, react_spread: 10, error: 0.10, settle: 0.94, jitter: 0.004, turn: 7.0, fire_cone: 0.025, sight: 60.0, fov: 1.2 }
    }
}

/// what a bot knows of another figure
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    pub id: u16,
    /// the centre of its hit volume
    pub centre: Vec3,
    /// alive and not protected: worth shooting at
    pub hittable: bool,
}

/// a bot's mind (see the module header)
#[derive(Clone, Debug)]
pub struct Brain {
    skill: Skill,
    rng: u64,
    rate: f64,
    yaw: f64,
    pitch: f64,
    goal: Option<usize>,
    from: Option<usize>,
    anchor: Vec3,
    anchor_tick: u32,
    enemy: Option<u16>,
    last_seen: u32,
    react_at: u32,
    err_yaw: f64,
    err_pitch: f64,
    strafe: i8,
    strafe_until: u32,
    jump: bool,
    reload_press: bool,
}

fn wrap(a: f64) -> f64 {
    game::camera::wrap_angle(a)
}

/// the yaw that looks along (dx, dz) (yaw 0 looks along -Z, positive turns left)
pub fn yaw_to(dx: f64, dz: f64) -> f64 {
    atan2(-dx, -dz)
}

impl Brain {
    /// a bot with this seed and skill, facing `yaw`
    pub fn new(seed: u64, skill: Skill, yaw: f64, rate: u32) -> Brain {
        Brain {
            skill,
            rng: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            rate: rate as f64,
            yaw,
            pitch: 0.0,
            goal: None,
            from: None,
            anchor: Vec3::ZERO,
            anchor_tick: 0,
            enemy: None,
            last_seen: 0,
            react_at: 0,
            err_yaw: 0.0,
            err_pitch: 0.0,
            strafe: 0,
            strafe_until: 0,
            jump: false,
            reload_press: false,
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

    /// uniform in [0, 1)
    fn unit(&mut self) -> f64 {
        (self.rand() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// uniform in [-1, 1)
    fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }

    /// the bot came back somewhere else: forget where it went and whom it saw
    pub fn respawned(&mut self, yaw: f64) {
        self.yaw = yaw;
        self.pitch = 0.0;
        self.goal = None;
        self.from = None;
        self.enemy = None;
    }

    /// the yaw and pitch it looks along
    pub fn view(&self) -> (f64, f64) {
        (self.yaw, self.pitch)
    }

    fn nearest_point(map: &Map, at: Vec3) -> Option<usize> {
        let eye = at + Vec3::new(0.0, 1.0, 0.0);
        let mut best: Option<(f64, usize)> = None;
        for (k, p) in map.points.iter().enumerate() {
            let d = (*p - at).length_sq();
            if best.is_some_and(|b| b.0 <= d) {
                continue;
            }
            if map.clear(eye, *p + Vec3::new(0.0, 1.0, 0.0)) {
                best = Some((d, k));
            }
        }
        best.map(|b| b.1).or_else(|| {
            // nothing in sight: the nearest at all
            (0..map.points.len()).min_by(|&a, &b| (map.points[a] - at).length_sq().partial_cmp(&(map.points[b] - at).length_sq()).unwrap_or(core::cmp::Ordering::Equal))
        })
    }

    fn next_goal(&mut self, map: &Map, at: usize) -> usize {
        let n = &map.neighbours[at];
        if n.is_empty() {
            return at;
        }
        let choices: Vec<usize> = n.iter().copied().filter(|&k| Some(k) != self.from || n.len() == 1).collect();
        let k = (self.rand() % choices.len() as u64) as usize;
        choices[k]
    }

    /// the input of one tick (see the module header)
    pub fn think(&mut self, tick: u32, me: &MoveState, mover: &Mover, map: &Map, others: &[Seen]) -> NetInput {
        let s = self.skill;
        let feet = me.feet();
        let eye = mover.eye(me);
        if me.dead() {
            return NetInput { yaw: angle_q(self.yaw), pitch: pitch_q(self.pitch), ..NetInput::default() };
        }
        // whom it sees: the nearest hittable figure in sight (and, for a new one, in its field of view)
        let mut target: Option<(u16, Vec3, f64)> = None;
        for o in others {
            if !o.hittable {
                continue;
            }
            let aim = o.centre + Vec3::new(0.0, 0.25, 0.0);
            let d = aim - eye;
            let dist = d.length();
            if dist > s.sight || target.is_some_and(|t| t.2 <= dist) {
                continue;
            }
            let known = self.enemy == Some(o.id);
            if !known {
                let off = wrap(yaw_to(d.x, d.z) - self.yaw).abs();
                if off > s.fov {
                    continue;
                }
            }
            if map.clear(eye, aim) {
                target = Some((o.id, aim, dist));
            }
        }
        let mut buttons = 0u16;
        let (mut want_yaw, mut want_pitch) = (self.yaw, 0.0);
        let mut fight = None;
        match target {
            Some((id, aim, dist)) => {
                if self.enemy != Some(id) {
                    // a new one: react after a delay, aim off at first
                    self.enemy = Some(id);
                    self.react_at = tick.wrapping_add(s.react + (self.rand() % (s.react_spread as u64 + 1)) as u32);
                    self.err_yaw = self.signed() * s.error;
                    self.err_pitch = self.signed() * s.error * 0.5;
                }
                self.last_seen = tick;
                let d = aim - eye;
                want_yaw = yaw_to(d.x, d.z);
                want_pitch = atan2(d.y, sqrt(d.x * d.x + d.z * d.z));
                fight = Some(dist);
            }
            None => {
                if self.enemy.is_some() && tick.wrapping_sub(self.last_seen) > 60 {
                    self.enemy = None;
                }
            }
        }
        let reacting = fight.is_some() && (tick.wrapping_sub(self.react_at) as i32) >= 0;
        // where to go: the way points while nothing is in sight
        let go = if map.points.is_empty() {
            Vec3::new(-feet.x, 0.0, -feet.z)
        } else {
            let goal = match self.goal {
                Some(g) => g,
                None => {
                    let g = Brain::nearest_point(map, feet).unwrap_or(0);
                    self.goal = Some(g);
                    g
                }
            };
            let p = map.points[goal];
            let flat = Vec3::new(p.x - feet.x, 0.0, p.z - feet.z);
            if flat.length_sq() < 1.44 {
                let next = self.next_goal(map, goal);
                self.from = Some(goal);
                self.goal = Some(next);
            }
            flat
        };
        // stuck: barely moved for 0.75 s
        if tick.wrapping_sub(self.anchor_tick) >= (self.rate * 0.75) as u32 {
            let moved = (feet - self.anchor).length();
            if moved < 0.4 && fight.is_none() {
                self.jump = true;
                if let Some(g) = self.goal {
                    let n = &map.neighbours[g];
                    if !n.is_empty() {
                        let k = (self.rand() % n.len() as u64) as usize;
                        self.from = Some(g);
                        self.goal = Some(n[k]);
                    }
                }
            }
            self.anchor = feet;
            self.anchor_tick = tick;
        }
        if fight.is_none() {
            if go.length_sq() > 1e-6 {
                want_yaw = yaw_to(go.x, go.z);
            }
            want_pitch = 0.0;
        }
        // aim: towards the target plus an error that settles
        if reacting {
            self.err_yaw = self.err_yaw * s.settle + self.signed() * s.jitter;
            self.err_pitch = self.err_pitch * s.settle + self.signed() * s.jitter * 0.5;
            want_yaw = wrap(want_yaw + self.err_yaw);
            want_pitch += self.err_pitch;
        } else if fight.is_some() {
            // (not yet: it keeps looking where it went)
            want_yaw = self.yaw;
            want_pitch = self.pitch;
        }
        let max = s.turn / self.rate;
        self.yaw = wrap(self.yaw + wrap(want_yaw - self.yaw).clamp(-max, max));
        self.pitch = (self.pitch + (want_pitch - self.pitch).clamp(-max, max)).clamp(-1.4, 1.4);
        // move: towards the goal, or in a fight strafe and keep a distance
        let (sn, cs) = physics::math::sincos(self.yaw);
        let fwd = Vec3::new(-sn, 0.0, -cs);
        let right = Vec3::new(cs, 0.0, -sn);
        let (mut mx, mut mz) = (0.0, 0.0);
        match fight {
            Some(dist) if reacting => {
                if tick.wrapping_sub(self.strafe_until) as i32 >= 0 {
                    self.strafe = if self.rand() % 2 == 0 { -1 } else { 1 };
                    self.strafe_until = tick.wrapping_add((self.rate * (0.4 + 0.8 * self.unit())) as u32);
                    if self.rand() % 5 == 0 {
                        self.jump = true;
                    }
                }
                mx = self.strafe as f64;
                mz = if dist > 18.0 { 1.0 } else if dist < 8.0 { -1.0 } else { 0.0 };
                let aim_err = wrap(want_yaw - self.yaw).abs().max((want_pitch - self.pitch).abs());
                let ready = (self.err_yaw.abs() + self.err_pitch.abs()) < s.fire_cone * 2.0;
                if aim_err < s.fire_cone && ready {
                    buttons |= BTN_FIRE;
                }
            }
            _ => {
                let l = go.length();
                if l > 1e-6 {
                    let dir = go * (1.0 / l);
                    mz = dir.dot(fwd);
                    mx = dir.dot(right);
                }
                // reload in a quiet moment
                if me.ammo() < MAGAZINE / 3 && me.reload() == 0 && fight.is_none() {
                    self.reload_press = !self.reload_press;
                    if self.reload_press {
                        buttons |= BTN_RELOAD;
                    }
                }
            }
        }
        if self.jump {
            buttons |= BTN_JUMP;
            self.jump = false;
        }
        NetInput { buttons, move_x: axis_q(mx), move_z: axis_q(mz), yaw: angle_q(self.yaw), pitch: pitch_q(self.pitch), view: 0 }
    }
}
