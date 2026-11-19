// bot -- a player without a person for tests and load: it wanders the arena, turns, strafes, runs and
// jumps now and then, and (with fire) aims at the nearest other figure and shoots.  It sees what its
// client sees (its predicted figure, the others interpolated) and makes the input of one tick from it;
// deterministic from its seed
//
//   BotConfig    seed, whether it shoots, how often it jumps, how fast it turns
//   Bot::think   the input of a tick from the own figure's feet (None before the first snapshot) and the
//                others' feet

use alloc::vec::Vec;

use physics::math::{atan2, sqrt};
use physics::Vec3;

use super::input::NetInput;
use super::quant::{angle_q, axis_q, pitch_q};
use super::walker::{BTN_FIRE, BTN_JUMP, BTN_RUN};

/// a bot's settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotConfig {
    /// its random numbers
    pub seed: u64,
    /// it shoots at others within `range`
    pub fire: bool,
    /// see fire (m)
    pub range: f64,
    /// jumps a second on average
    pub jumps: f64,
    /// the fastest turn (rad/s)
    pub turn: f64,
    /// half the side of the arena (where it wanders)
    pub half: f64,
    /// ticks a second
    pub rate: u32,
}

impl Default for BotConfig {
    fn default() -> Self {
        BotConfig { seed: 1, fire: false, range: 25.0, jumps: 0.3, turn: 4.0, half: 30.0, rate: 60 }
    }
}

/// a bot (see the module header)
#[derive(Clone, Debug)]
pub struct Bot {
    cfg: BotConfig,
    rng: u64,
    yaw: f64,
    pitch: f64,
    target: Vec3,
    until: u32,
    strafe: i8,
    run: bool,
    ticks: u32,
}

/// the yaw that looks along (dx, dz) (yaw 0 looks along -Z, positive turns left)
fn yaw_to(dx: f64, dz: f64) -> f64 {
    atan2(-dx, -dz)
}

fn wrap(a: f64) -> f64 {
    crate::camera::wrap_angle(a)
}

impl Bot {
    /// a bot with these settings
    pub fn new(cfg: BotConfig) -> Bot {
        Bot { rng: cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1, cfg, yaw: 0.0, pitch: 0.0, target: Vec3::ZERO, until: 0, strafe: 0, run: false, ticks: 0 }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f64 {
        (self.rand() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// the input of one tick: `own` the figure's feet (None: not known yet), `others` the other figures'
    /// feet
    pub fn think(&mut self, own: Option<Vec3>, others: &[(u16, Vec3)]) -> NetInput {
        self.ticks = self.ticks.wrapping_add(1);
        let Some(me) = own else {
            return NetInput { yaw: angle_q(self.yaw), ..NetInput::default() };
        };
        let h = self.cfg.half * 0.8;
        let close = {
            let dx = self.target.x - me.x;
            let dz = self.target.z - me.z;
            dx * dx + dz * dz < 2.25
        };
        if self.ticks >= self.until || close {
            self.target = Vec3::new((self.unit() * 2.0 - 1.0) * h, 0.0, (self.unit() * 2.0 - 1.0) * h);
            self.until = self.ticks + self.cfg.rate * 2 + (self.unit() * (self.cfg.rate * 3) as f64) as u32;
            self.strafe = match self.rand() % 4 {
                0 => -80,
                1 => 80,
                _ => 0,
            };
            self.run = self.rand() % 3 == 0;
        }
        // an enemy in range: aim and shoot
        let mut want_yaw = yaw_to(self.target.x - me.x, self.target.z - me.z);
        let mut want_pitch = 0.0;
        let mut aim = None;
        if self.cfg.fire {
            let mut best = self.cfg.range * self.cfg.range;
            for (_, p) in others {
                let d = *p - me;
                let d2 = d.x * d.x + d.z * d.z;
                if d2 < best {
                    best = d2;
                    aim = Some(d);
                }
            }
        }
        if let Some(d) = aim {
            want_yaw = yaw_to(d.x, d.z);
            want_pitch = atan2(d.y, sqrt(d.x * d.x + d.z * d.z));
        }
        let max = self.cfg.turn / self.cfg.rate as f64;
        let dy = wrap(want_yaw - self.yaw).clamp(-max, max);
        self.yaw = wrap(self.yaw + dy);
        self.pitch += (want_pitch - self.pitch).clamp(-max, max);
        let mut buttons = 0u16;
        if self.run {
            buttons |= BTN_RUN;
        }
        if self.unit() < self.cfg.jumps / self.cfg.rate as f64 {
            buttons |= BTN_JUMP;
        }
        if aim.is_some() && wrap(want_yaw - self.yaw).abs() < 0.05 {
            buttons |= BTN_FIRE;
        }
        NetInput { buttons, move_x: self.strafe, move_z: axis_q(1.0), yaw: angle_q(self.yaw), pitch: pitch_q(self.pitch), view: 0 }
    }
}

/// the feet of the other figures of a client's view (KIND_PLAYER remotes, blended), for Bot::think
pub fn remote_feet(remotes: &[super::client::Remote], out: &mut Vec<(u16, Vec3)>) {
    use super::client::lerp_field;
    out.clear();
    for r in remotes {
        if r.a.kind != super::arena::KIND_PLAYER {
            continue;
        }
        let f = |i: usize| lerp_field(r.a.fields[i], r.b.fields[i], r.t) / super::quant::POS_STEPS;
        out.push((r.id, Vec3::new(f(0), f(1), f(2))));
    }
}
