// walker -- the netcode's reference movement: a kinematic capsule that walks, runs, jumps, falls and
// slides along what it touches (like apps/gamedemo's walker, with gravity); deterministic and on the
// grid, so a client predicts exactly what the server computes.  G7h's character controller (lib/physics,
// G7c part 2) takes its place behind the same trait (predict::Movement)
//
//   WalkState    feet position and velocity on the grid of quant.rs (1/1024 m, 1/1024 m/s), flags (on the
//                ground) and the buttons of the tick before (for edges: a jump is a press, not a hold)
//   Walker       the parameters (capsule, speeds, jump, gravity, air control); Movement::step
//   BTN_*        the buttons the walker reads (and the arena's FIRE)
//
// A step: on the ground the horizontal velocity is the wish (walk or run along the view's yaw), a jump
// press lifts off; in the air the wish pulls the velocity a little (air control) and gravity pulls it
// down.  The capsule is cast through the map along the motion, up to four times: to the contact less a
// millimetre, the rest projected onto the surface, the velocity into the surface taken away.  Then a
// short cast down decides "on the ground" (a surface steeper than about 45 degrees is a wall), and the
// position and velocity are rounded onto the grid.

use physics::math::sqrt;
use physics::{Filter, Iso, Shape, Vec3};

use super::input::NetInput;
use super::predict::Movement;
use super::quant::{angle_f, axis_f, pos_f, pos_q};

/// a button: jump (a press lifts off)
pub const BTN_JUMP: u16 = 1;
/// a button: run (held)
pub const BTN_RUN: u16 = 2;
/// a button: fire (the arena's hitscan)
pub const BTN_FIRE: u16 = 4;

/// the walker's state flag: standing on the ground
pub const FLAG_GROUND: u32 = 1;

/// the walker's movement state (see the module header)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkState {
    /// the feet (the bottom of the capsule), in 1/1024 m
    pub pos: [i32; 3],
    /// the velocity in 1/1024 m/s
    pub vel: [i32; 3],
    /// FLAG_GROUND in bit 0, the buttons of the tick before in bits 16 ..= 31
    pub flags: u32,
}

impl WalkState {
    /// standing at feet position p (metres) on the ground
    pub fn at(p: Vec3) -> WalkState {
        WalkState { pos: [pos_q(p.x), pos_q(p.y), pos_q(p.z)], vel: [0; 3], flags: FLAG_GROUND }
    }

    /// the feet in metres
    pub fn feet(&self) -> Vec3 {
        Vec3::new(pos_f(self.pos[0]), pos_f(self.pos[1]), pos_f(self.pos[2]))
    }

    /// the velocity in m/s
    pub fn velocity(&self) -> Vec3 {
        Vec3::new(pos_f(self.vel[0]), pos_f(self.vel[1]), pos_f(self.vel[2]))
    }

    /// on the ground
    pub fn grounded(&self) -> bool {
        self.flags & FLAG_GROUND != 0
    }

    /// the buttons of the tick before
    pub fn last_buttons(&self) -> u16 {
        (self.flags >> 16) as u16
    }
}

/// the walker's parameters (see the module header)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    /// the capsule's half height (of its straight part) and radius
    pub half: f64,
    /// see half
    pub radius: f64,
    /// walking and running speed (m/s)
    pub walk: f64,
    /// see walk
    pub run: f64,
    /// the upward speed of a jump (m/s)
    pub jump: f64,
    /// gravity (m/s^2)
    pub gravity: f64,
    /// how fast the wish pulls the velocity in the air (1/s)
    pub air: f64,
    /// the fastest fall (m/s)
    pub max_fall: f64,
    /// the smallest normal.y of a surface to stand on (cos of the steepest slope)
    pub ground_cos: f64,
}

impl Default for Walker {
    fn default() -> Self {
        Walker { half: 0.6, radius: 0.3, walk: 4.5, run: 8.0, jump: 5.5, gravity: 18.0, air: 2.0, max_fall: 40.0, ground_cos: 0.7 }
    }
}

/// what the walker leaves between itself and a surface (m)
const SKIN: f64 = 0.001;
/// how far it looks down for the ground (m)
const PROBE: f64 = 0.01;

impl Walker {
    /// the capsule
    pub fn shape(&self) -> Shape {
        Shape::capsule(self.half, self.radius)
    }

    /// the capsule's centre above the feet
    pub fn centre(&self) -> Vec3 {
        Vec3::new(0.0, self.half + self.radius, 0.0)
    }

    /// where the eye is above the feet
    pub fn eye(&self) -> f64 {
        2.0 * (self.half + self.radius) - 0.15
    }
}

impl Movement for Walker {
    type State = WalkState;
    const FIELDS: usize = 7;

    fn step(&self, map: &physics::World, s: &WalkState, input: &NetInput, dt: f64) -> WalkState {
        let mut pos = s.feet();
        let mut vel = s.velocity();
        let buttons = input.buttons;
        let jump = buttons & BTN_JUMP != 0 && s.last_buttons() & BTN_JUMP == 0;
        let speed = if buttons & BTN_RUN != 0 { self.run } else { self.walk };
        let wish = crate::camera::walk_dir(angle_f(input.yaw), axis_f(input.move_z), axis_f(input.move_x)) * speed;
        if s.grounded() {
            vel = Vec3::new(wish.x, 0.0, wish.z);
            if jump {
                vel.y = self.jump;
            }
        } else {
            let k = (self.air * dt).min(1.0);
            vel.x += (wish.x - vel.x) * k;
            vel.z += (wish.z - vel.z) * k;
            vel.y = (vel.y - self.gravity * dt).max(-self.max_fall);
        }
        let shape = self.shape();
        let c = self.centre();
        let mut motion = vel * dt;
        for _ in 0..4 {
            let len2 = motion.length_sq();
            if len2 < 1e-18 {
                break;
            }
            match map.cast(&shape, &Iso::at(pos + c), motion, &Filter::ALL) {
                Some((_, hit)) => {
                    let len = sqrt(len2);
                    let go = (hit.fraction * len - SKIN).max(0.0) / len;
                    pos += motion * go;
                    let rest = motion * (1.0 - go);
                    let n = hit.normal;
                    motion = rest - n * rest.dot(n);
                    let into = vel.dot(n);
                    if into < 0.0 {
                        vel -= n * into;
                    }
                }
                None => {
                    pos += motion;
                    break;
                }
            }
        }
        // the ground: a short cast down
        let down = Vec3::new(0.0, -PROBE, 0.0);
        let grounded = vel.y <= 0.0
            && match map.cast(&shape, &Iso::at(pos + c), down, &Filter::ALL) {
                Some((_, hit)) => hit.normal.y >= self.ground_cos,
                None => false,
            };
        if grounded {
            vel.y = 0.0;
        }
        WalkState {
            pos: [pos_q(pos.x), pos_q(pos.y), pos_q(pos.z)],
            vel: [pos_q(vel.x), pos_q(vel.y), pos_q(vel.z)],
            flags: (grounded as u32) | (buttons as u32) << 16,
        }
    }

    fn to_fields(s: &WalkState, out: &mut [i32]) {
        out[0] = s.pos[0];
        out[1] = s.pos[1];
        out[2] = s.pos[2];
        out[3] = s.vel[0];
        out[4] = s.vel[1];
        out[5] = s.vel[2];
        out[6] = s.flags as i32;
    }

    fn from_fields(f: &[i32]) -> WalkState {
        let g = |i: usize| f.get(i).copied().unwrap_or(0);
        WalkState { pos: [g(0), g(1), g(2)], vel: [g(3), g(4), g(5)], flags: g(6) as u32 }
    }

    fn position(s: &WalkState) -> Vec3 {
        s.feet()
    }
}
