// movement -- a figure of game 1 behind game::net's Movement: lib/physics' CharacterController for the body and
// the weapon's timing (magazine, cooldown, reload, the shot), both on integers after every tick, so a client
// predicts its own figure and its shots bit for bit what the server computes (docs/NETCODE.md, "Bitgleich")
//
//   MoveState   feet and velocity on the grid of game::net::quant (1/1024 m, 1/1024 m/s), flags (crouched, on
//               the ground, dead, fired in this tick, jumped, landed; the buttons of the tick for the edge of a
//               reload press), the weapon (rounds, cooldown, reload, a counter of shots): FIELDS = 8 integers
//   Mover       the controller's parameters (CharacterParams::default(), the start values) and Movement::step
//
// A step: a dead figure stands still.  Otherwise the controller moves it (wish from the yaw and the two axes,
// running unless BTN_WALK, BTN_JUMP held jumps on every landing, BTN_CROUCH held crouches); then the weapon: the
// cooldown counts down, a reload counts down and fills the magazine, a reload press or an empty magazine starts
// one, BTN_FIRE with the cooldown over fires (F_FIRED, a round less, the counter up).  The server reads F_FIRED
// and casts the shot with lag compensation (sim.rs); the client shows its muzzle flash at once.  The state is
// rounded onto the grid at the end: the controller reads only position, velocity and crouched, and a rounding of
// under a millimetre never puts the capsule into anything (its skin is 5 mm).

use physics::{CharacterController, CharacterInput, CharacterParams, CharacterState, Filter, Shape, Vec3};

use game::net::input::NetInput;
use game::net::predict::Movement;
use game::net::quant::{angle_f, axis_f, pos_q, vec_f};

use crate::rules::{BTN_CROUCH, BTN_FIRE, BTN_JUMP, BTN_RELOAD, BTN_WALK, EYE_BELOW_TOP, FIRE_TICKS, MAGAZINE, RELOAD_TICKS};

/// a flag of MoveState: crouched
pub const F_CROUCH: u32 = 1;
/// on the ground after the tick
pub const F_GROUND: u32 = 2;
/// dead (stands still, waits for the server to bring it back)
pub const F_DEAD: u32 = 4;
/// fired in this tick
pub const F_FIRED: u32 = 8;
/// jumped in this tick
pub const F_JUMPED: u32 = 16;
/// landed in this tick
pub const F_LANDED: u32 = 32;

/// a figure's movement and weapon state (see the module header)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveState {
    /// the feet (the capsule's lowest point), in 1/1024 m
    pub pos: [i32; 3],
    /// the velocity in 1/1024 m/s
    pub vel: [i32; 3],
    /// F_* in bits 0-15, the buttons of the tick in bits 16-31
    pub flags: u32,
    /// rounds (bits 0-7), cooldown ticks (8-15), reload ticks left (16-23), shots fired (24-31, a counter)
    pub gun: u32,
}

impl MoveState {
    /// standing at feet position p with a full magazine (alive)
    pub fn at(p: Vec3) -> MoveState {
        MoveState { pos: [pos_q(p.x), pos_q(p.y), pos_q(p.z)], vel: [0; 3], flags: 0, gun: gun(MAGAZINE, 0, 0, 0) }
    }

    /// the feet in metres
    pub fn feet(&self) -> Vec3 {
        vec_f(self.pos)
    }

    /// the velocity in m/s
    pub fn velocity(&self) -> Vec3 {
        vec_f(self.vel)
    }

    pub fn crouched(&self) -> bool {
        self.flags & F_CROUCH != 0
    }

    pub fn dead(&self) -> bool {
        self.flags & F_DEAD != 0
    }

    pub fn fired(&self) -> bool {
        self.flags & F_FIRED != 0
    }

    pub fn grounded(&self) -> bool {
        self.flags & F_GROUND != 0
    }

    /// rounds in the magazine
    pub fn ammo(&self) -> u32 {
        self.gun & 0xff
    }

    /// reload ticks left (0: not reloading)
    pub fn reload(&self) -> u32 {
        (self.gun >> 16) & 0xff
    }

    /// shots fired (a counter that wraps at 256)
    pub fn shots(&self) -> u8 {
        (self.gun >> 24) as u8
    }

    /// the state killed: dead, still, the weapon as it was
    pub fn killed(&self) -> MoveState {
        MoveState { vel: [0; 3], flags: F_DEAD | (self.flags & F_CROUCH), ..*self }
    }
}

/// the weapon field of its parts
pub fn gun(ammo: u32, cool: u32, reload: u32, shots: u32) -> u32 {
    (ammo & 0xff) | (cool & 0xff) << 8 | (reload & 0xff) << 16 | (shots & 0xff) << 24
}

/// a figure's movement (see the module header)
#[derive(Clone, Debug)]
pub struct Mover {
    pub ctrl: CharacterController,
}

impl Default for Mover {
    fn default() -> Self {
        Mover::new(params())
    }
}

/// the controller's parameters of game 1: physics' defaults, tuned for a fast cartoon arena shooter (radius 0.4 m,
/// 1.8 m tall, 1.2 m crouched, run 9 m/s, walk 5, crouch 3, accel 12/s, friction 6/s, air control 0.3, gravity
/// 20 m/s^2, jump 1.2 m, steps 0.4 m, slopes 45 degrees); the start values the user tunes by hand
pub fn params() -> CharacterParams {
    CharacterParams::default()
}

impl Mover {
    pub fn new(p: CharacterParams) -> Mover {
        Mover { ctrl: CharacterController::new(p) }
    }

    /// the capsule's height (standing or crouched)
    pub fn height(&self, crouched: bool) -> f64 {
        self.ctrl.height(crouched)
    }

    /// where the eye is
    pub fn eye(&self, s: &MoveState) -> Vec3 {
        s.feet() + Vec3::new(0.0, self.height(s.crouched()) - EYE_BELOW_TOP, 0.0)
    }

    /// the centre of the hit volume (the capsule's centre)
    pub fn centre(&self, s: &MoveState) -> Vec3 {
        s.feet() + Vec3::new(0.0, 0.5 * self.height(s.crouched()), 0.0)
    }

    /// the hit volume (the capsule)
    pub fn shape(&self, crouched: bool) -> Shape {
        self.ctrl.capsule(crouched)
    }

    /// does a hit at `point` on the capsule centred at `centre` strike the head (the upper half sphere)
    pub fn head(&self, crouched: bool, centre: Vec3, point: Vec3) -> bool {
        let r = self.ctrl.params().radius;
        point.y >= centre.y + 0.5 * self.height(crouched) - r
    }
}

impl Movement for Mover {
    type State = MoveState;
    const FIELDS: usize = 8;

    fn step(&self, map: &physics::World, s: &MoveState, input: &NetInput, dt: f64) -> MoveState {
        let buttons = input.buttons as u32;
        let prev = s.flags >> 16;
        if s.dead() {
            return MoveState { pos: s.pos, vel: [0; 3], flags: F_DEAD | (s.flags & F_CROUCH) | buttons << 16, gun: s.gun };
        }
        let cs = CharacterState { velocity: s.velocity(), crouched: s.crouched(), ..CharacterState::new(s.feet()) };
        let wish = game::camera::walk_dir(angle_f(input.yaw), axis_f(input.move_z), axis_f(input.move_x));
        let ci = CharacterInput { wish, run: buttons & BTN_WALK as u32 == 0, jump: buttons & BTN_JUMP as u32 != 0, crouch: buttons & BTN_CROUCH as u32 != 0 };
        let (n, ev) = self.ctrl.step(&cs, map, &Filter::ALL, &ci, dt);
        // the weapon
        let (mut ammo, mut cool, mut reload, mut shots) = (s.gun & 0xff, (s.gun >> 8) & 0xff, (s.gun >> 16) & 0xff, s.gun >> 24);
        cool = cool.saturating_sub(1);
        let mut fired = false;
        if reload > 0 {
            reload -= 1;
            if reload == 0 {
                ammo = MAGAZINE;
            }
        } else if (buttons & BTN_RELOAD as u32 != 0 && prev & BTN_RELOAD as u32 == 0 && ammo < MAGAZINE) || ammo == 0 {
            reload = RELOAD_TICKS;
        } else if buttons & BTN_FIRE as u32 != 0 && cool == 0 {
            fired = true;
            ammo -= 1;
            cool = FIRE_TICKS;
            shots = shots.wrapping_add(1);
        }
        let mut flags = buttons << 16;
        if n.crouched {
            flags |= F_CROUCH;
        }
        if n.grounded {
            flags |= F_GROUND;
        }
        if fired {
            flags |= F_FIRED;
        }
        if ev.jumped {
            flags |= F_JUMPED;
        }
        if ev.landed {
            flags |= F_LANDED;
        }
        let p = n.position;
        let v = n.velocity;
        MoveState { pos: [pos_q(p.x), pos_q(p.y), pos_q(p.z)], vel: [pos_q(v.x), pos_q(v.y), pos_q(v.z)], flags, gun: gun(ammo, cool, reload, shots) }
    }

    fn to_fields(s: &MoveState, out: &mut [i32]) {
        out[0] = s.pos[0];
        out[1] = s.pos[1];
        out[2] = s.pos[2];
        out[3] = s.vel[0];
        out[4] = s.vel[1];
        out[5] = s.vel[2];
        out[6] = s.flags as i32;
        out[7] = s.gun as i32;
    }

    fn from_fields(f: &[i32]) -> MoveState {
        let g = |i: usize| f.get(i).copied().unwrap_or(0);
        MoveState { pos: [g(0), g(1), g(2)], vel: [g(3), g(4), g(5)], flags: g(6) as u32, gun: g(7) as u32 }
    }

    fn position(s: &MoveState) -> Vec3 {
        s.feet()
    }
}
