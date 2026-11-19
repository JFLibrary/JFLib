// character -- the kinematic character controller: an upright capsule that walks, runs, jumps and crouches
// on the world's queries.  Steps up to a step height, ramps up to a slope limit (steeper surfaces are walls
// it slides down), sliding along walls and into corners, ground snapping on the way down ramps and steps,
// steering in the air.  The feel hangs on CharacterParams; the defaults suit a fast cartoon arena shooter.
//
// One step is a function of (state, input, world, dt) alone: f64 with + - * /, the correctly rounded
// square root and rt's portable cosine (once, in new), no fused multiply-add, no unordered iteration.  So
// it gives the same bits on every target, and a client predicts exactly what the server computes (G7e).
// The step derives the ground itself; position, velocity and crouched are all the state it reads.
//
// A step (docs/PHYSICS.md, "Der Character-Controller"):
//   1. push out of anything the capsule overlaps (only after a teleport or a body moved into it: the
//      controller's own moves are casts and never end inside anything)
//   2. find the ground: a short cast down; it carries the figure when the normal is walkable (y >=
//      cos(max slope)), or when the contact is on the edge of a walkable surface (a step, a ledge: a ray
//      down just beyond the contact finds that surface), and the figure does not move away from it
//   3. crouch (on the ground the head comes down, in the air the legs come up) or stand up (only where a
//      cast of the crouched capsule finds room for the standing one)
//   4. jump (sets the vertical speed); on the ground with input the velocity moves towards the wish
//      velocity along the ground (accel), without input Quake's friction brakes it; in the air Quake's
//      acceleration (the speed along the wish direction grows up to the wish speed, scaled by air_control)
//   5. move: collide and slide with up to four casts, stopping `skin` before each contact; Quake 3's clip
//      planes (two planes: along their crease, three: stop), so corners neither jitter nor trap; on the
//      ground steep surfaces count as vertical walls (no climbing them by pushing), a walkable one turns
//      the velocity onto it at the same speed; blocked on the ground, a step: up, forward, down onto
//      walkable ground, taken when it gets farther
//   6. on the ground at the start and no jump: snap down onto ground up to step_height below (stairs and
//      ramps downwards); in the air: land on ground within the probe distance
//
// Gravity in the air uses the mean velocity of the tick (v - g dt / 2) for the move, so the height after k
// free ticks is y0 + v0 t - g t^2 / 2 at t = k dt (up to rounding), whatever the tick rate.

use crate::math::{Iso, Vec3, clamp, cos, max, min, sqrt};
use crate::query::{CastHit, Contact};
use crate::shape::Shape;
use crate::world::{BodyId, Filter, World};
use alloc::vec::Vec;

/// at most so many walls one step reports in [`StepEvents::walls`]
pub const MAX_WALL_TOUCHES: usize = 4;

/// casts of one slide (each contact costs one)
const MAX_BUMPS: usize = 4;
/// clip planes of one slide: the ground, the original direction and four contacts
const MAX_PLANES: usize = 6;
/// rounds of pushing out of overlaps
const MAX_PUSH_OUT: usize = 4;
/// a velocity component below this (m/s) counts as not going into a plane
const VEL_EPS: f64 = 1e-9;
/// moving away from the ground faster than this (m/s) leaves it (a jump, a launch from outside)
const LEAVE_SPEED: f64 = 0.25;
/// the push away from a plane that was hit twice in one slide (m/s; Quake's 1 unit/s)
const NUDGE: f64 = 1e-3;
/// an overlap deeper than this (m) is pushed out of
const OVERLAP_TOL: f64 = 1e-6;
/// an edge contact whose normal has a smaller y does not carry the figure (the capsule's side, not its foot)
const LEDGE_MIN_Y: f64 = 0.05;
/// the ray that looks for the surface beyond an edge starts this far above the contact (m)
const LEDGE_UP: f64 = 0.05;
/// a step is taken when it gets at least this much farther than the slide without it (m)
const STEP_GAIN: f64 = 1e-4;
/// a motion shorter than this (m) is not cast
const MIN_MOVE: f64 = 1e-9;
/// the ground is snapped to only when it is farther than the skin by more than this (m): a figure at rest
/// does not creep down by rounding
const SNAP_EPS: f64 = 1e-9;

/// How a figure moves: its capsule, speeds, acceleration, friction, gravity, jump, steps and slopes.
/// Metres, seconds, degrees.  [`CharacterParams::default`] is tuned for a fast cartoon arena shooter
/// (docs/PHYSICS.md lists the values and why); [`CharacterController::new`] limits values that make no sense.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterParams {
    /// the capsule's radius (default 0.4 m)
    pub radius: f64,
    /// standing height, feet to the top of the head (1.8 m), at least twice the radius
    pub height: f64,
    /// crouched height (1.2 m), between twice the radius and `height`
    pub crouch_height: f64,
    /// wish speed without `run` (5 m/s)
    pub walk_speed: f64,
    /// wish speed with `run` (9 m/s)
    pub run_speed: f64,
    /// wish speed while crouched (3 m/s)
    pub crouch_speed: f64,
    /// ground acceleration (12 per second): with input the velocity moves towards the wish velocity (wish
    /// direction along the ground times wish speed) by up to `accel` times the wish speed per second; 0 to
    /// full speed in 1/12 s, sliding along a wall at the wish velocity's part along it
    pub accel: f64,
    /// ground friction without input (6 per second, Quake's): each second the speed drops by `friction`
    /// times max(speed, `stop_speed`); from run speed to a stop in about 0.4 s
    pub friction: f64,
    /// below this speed friction brakes as if the figure moved this fast, so it stops (2.5 m/s)
    pub stop_speed: f64,
    /// acceleration in the air as a fraction of `accel` (0.3; 0: no steering in the air), Quake's model:
    /// the speed along the wish direction grows by up to air_control times accel times the wish speed per
    /// second, not beyond the wish speed; steering at an angle to the velocity can gain speed (strafe
    /// jumping) up to `max_speed`
    pub air_control: f64,
    /// downwards (20 m/s², about twice the earth's: short, snappy jumps)
    pub gravity: f64,
    /// height of a jump's apex above the take-off (1.2 m: onto a crate)
    pub jump_height: f64,
    /// the highest step the figure walks up and the deepest it snaps down to (0.4 m)
    pub step_height: f64,
    /// the steepest walkable slope (45 degrees); steeper surfaces are walls it slides down
    pub max_slope_deg: f64,
    /// the gap kept between the capsule and everything (0.005 m)
    pub skin: f64,
    /// the horizontal speed never gets above this (30 m/s)
    pub max_speed: f64,
    /// falling never gets faster than this (50 m/s)
    pub max_fall_speed: f64,
}

impl Default for CharacterParams {
    fn default() -> CharacterParams {
        CharacterParams {
            radius: 0.4,
            height: 1.8,
            crouch_height: 1.2,
            walk_speed: 5.0,
            run_speed: 9.0,
            crouch_speed: 3.0,
            accel: 12.0,
            friction: 6.0,
            stop_speed: 2.5,
            air_control: 0.3,
            gravity: 20.0,
            jump_height: 1.2,
            step_height: 0.4,
            max_slope_deg: 45.0,
            skin: 0.005,
            max_speed: 30.0,
            max_fall_speed: 50.0,
        }
    }
}

/// What a figure is: all a step reads is `position`, `velocity` and `crouched`; `grounded`,
/// `ground_normal` and `ground_body` are what the last step found (a step finds them again).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterState {
    /// the feet: the lowest point of the capsule (m)
    pub position: Vec3,
    /// m/s; on the ground along the ground
    pub velocity: Vec3,
    /// the capsule has the crouched height
    pub crouched: bool,
    /// stands on walkable ground (output)
    pub grounded: bool,
    /// the normal of that ground, [`Vec3::Y`] in the air (output)
    pub ground_normal: Vec3,
    /// the body the figure stands on (output)
    pub ground_body: Option<BodyId>,
}

impl CharacterState {
    /// standing at `position` (the feet), not moving, in the air until the first step finds ground
    pub const fn new(position: Vec3) -> CharacterState {
        CharacterState { position, velocity: Vec3::ZERO, crouched: false, grounded: false, ground_normal: Vec3::Y, ground_body: None }
    }
}

/// What the player wants in one tick
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CharacterInput {
    /// the direction to move in, in world space; y is ignored.  Length 1 moves at the full wish speed,
    /// shorter moves slower (a stick), longer counts as 1.  From a yaw: `math::sincos` (rt's portable sine
    /// and cosine), never std's, which differs between targets.
    pub wish: Vec3,
    /// the run speed instead of the walk speed
    pub run: bool,
    /// jump when on the ground (held: again on every landing)
    pub jump: bool,
    /// crouch while held; on release stand up as soon as there is room
    pub crouch: bool,
}

/// one contact of a step with a wall
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    /// the body
    pub body: BodyId,
    /// the contact point on its surface
    pub point: Vec3,
    /// the surface normal towards the figure
    pub normal: Vec3,
    /// the triangle of a mesh, or [`crate::NO_TRIANGLE`]
    pub triangle: u32,
}

/// What happened in one step
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StepEvents {
    /// was in the air (after a jump of this step too) and stands at the end
    pub landed: bool,
    /// the speed into the ground at the landing (m/s, for sounds and damage)
    pub impact_speed: f64,
    /// jumped at the start of this step
    pub jumped: bool,
    /// stood at the start, did not jump and is in the air at the end (walked off a ledge)
    pub left_ground: bool,
    /// touched a surface facing down (the normal's y at most -cos(max slope))
    pub hit_ceiling: bool,
    /// the walls touched, in order, each body and triangle once, at most [`MAX_WALL_TOUCHES`]
    pub walls: [Option<Touch>; MAX_WALL_TOUCHES],
    /// how far a step lifted the figure (m)
    pub stepped_up: f64,
    /// how far ground snapping lowered it (m)
    pub snapped_down: f64,
    /// wanted to stand up and had no room
    pub stand_blocked: bool,
    /// how far it was pushed out of an overlap at the start (m)
    pub pushed_out: f64,
    /// the casts this step made (for statistics)
    pub casts: u32,
}

impl StepEvents {
    /// the walls touched
    pub fn walls(&self) -> impl Iterator<Item = &Touch> + '_ {
        self.walls.iter().flatten()
    }

    /// how many walls were touched
    pub fn wall_count(&self) -> usize {
        self.walls().count()
    }
}

/// The controller: the parameters checked and the values derived from them.  One controller serves any
/// number of figures; [`CharacterController::step`] takes `&self`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterController {
    p: CharacterParams,
    cos_slope: f64,
    jump_speed: f64,
    ledge_out: f64,
}

impl Default for CharacterController {
    fn default() -> CharacterController {
        CharacterController::new(CharacterParams::default())
    }
}

/// the ground under a figure
#[derive(Clone, Copy, Debug)]
struct Ground {
    /// what the figure walks along: the surface's normal (beyond an edge: the surface beyond)
    normal: Vec3,
    body: BodyId,
    /// the free distance down to it
    dist: f64,
}

/// the walls and ceilings one slide touched
#[derive(Clone, Copy, Default)]
struct Touches {
    walls: [Option<Touch>; MAX_WALL_TOUCHES],
    ceiling: bool,
}

impl Touches {
    fn wall(&mut self, t: Touch) {
        for w in self.walls.iter_mut() {
            match w {
                Some(o) if o.body == t.body && o.triangle == t.triangle => return,
                Some(_) => {}
                None => {
                    *w = Some(t);
                    return;
                }
            }
        }
    }
}

/// the result of one slide
struct Slid {
    pos: Vec3,
    vel: Vec3,
    /// on the ground and stopped by something that is no ground and no ceiling: try a step
    blocked: bool,
    touches: Touches,
    /// the fastest approach to walkable ground in the air (m/s)
    impact: f64,
}

/// the result of the move with its step
struct Moved {
    pos: Vec3,
    vel: Vec3,
    touches: Touches,
    impact: f64,
    /// the ground a step landed on
    stepped: Option<Ground>,
    stepped_up: f64,
}

impl CharacterController {
    /// a controller with `params`, limited to what makes sense: radius at least 1 cm, heights at least twice
    /// the radius (crouched not above standing), skin between 0.1 mm and a quarter of the radius, slope 0 to
    /// 89 degrees, no negative speeds; a value that is not finite takes the default
    pub fn new(params: CharacterParams) -> CharacterController {
        let d = CharacterParams::default();
        let f = |v: f64, dflt: f64| if v.is_finite() { v } else { dflt };
        let mut p = params;
        p.radius = max(f(p.radius, d.radius), 0.01);
        p.height = max(f(p.height, d.height), 2.0 * p.radius);
        p.crouch_height = clamp(f(p.crouch_height, d.crouch_height), 2.0 * p.radius, p.height);
        p.walk_speed = max(f(p.walk_speed, d.walk_speed), 0.0);
        p.run_speed = max(f(p.run_speed, d.run_speed), 0.0);
        p.crouch_speed = max(f(p.crouch_speed, d.crouch_speed), 0.0);
        p.accel = max(f(p.accel, d.accel), 0.0);
        p.friction = max(f(p.friction, d.friction), 0.0);
        p.stop_speed = max(f(p.stop_speed, d.stop_speed), 0.0);
        p.air_control = max(f(p.air_control, d.air_control), 0.0);
        p.gravity = max(f(p.gravity, d.gravity), 0.0);
        p.jump_height = max(f(p.jump_height, d.jump_height), 0.0);
        p.step_height = clamp(f(p.step_height, d.step_height), 0.0, p.height);
        p.max_slope_deg = clamp(f(p.max_slope_deg, d.max_slope_deg), 0.0, 89.0);
        p.skin = clamp(f(p.skin, d.skin), 1e-4, 0.25 * p.radius);
        p.max_speed = max(f(p.max_speed, d.max_speed), 0.0);
        p.max_fall_speed = max(f(p.max_fall_speed, d.max_fall_speed), 0.0);
        CharacterController {
            p,
            cos_slope: cos(p.max_slope_deg * (core::f64::consts::PI / 180.0)),
            jump_speed: sqrt(2.0 * p.gravity * p.jump_height),
            ledge_out: min(0.02, 0.1 * p.radius),
        }
    }

    /// the parameters as the controller uses them
    pub fn params(&self) -> &CharacterParams {
        &self.p
    }

    /// the vertical speed a jump starts with: sqrt(2 gravity jump_height)
    pub fn jump_speed(&self) -> f64 {
        self.jump_speed
    }

    /// cos(max_slope_deg): a normal with at least this y is walkable
    pub fn walkable_y(&self) -> f64 {
        self.cos_slope
    }

    /// can a figure stand on a surface with this unit normal
    pub fn walkable(&self, normal: Vec3) -> bool {
        normal.y >= self.cos_slope
    }

    /// the capsule's total height
    pub fn height(&self, crouched: bool) -> f64 {
        if crouched { self.p.crouch_height } else { self.p.height }
    }

    /// the capsule (upright, its centre half the height above the feet)
    pub fn capsule(&self, crouched: bool) -> Shape {
        Shape::capsule(0.5 * self.height(crouched) - self.p.radius, self.p.radius)
    }

    /// where the capsule of `state` is: for [`World::set_iso`] when the figure is a body of the world
    pub fn capsule_iso(&self, state: &CharacterState) -> Iso {
        let p = state.position;
        Iso::at(Vec3::new(p.x, p.y + 0.5 * self.height(state.crouched), p.z))
    }

    /// One tick: the state after `dt` seconds of `input` among the bodies of `world` that `filter` lets
    /// through (leave the figure's own body out: `Filter::ALL.excluding(id)`), and what happened.  The same
    /// bits on every target for the same arguments.  A `dt` that is not positive and finite, or a position
    /// that is not finite, changes nothing; a velocity that is not finite counts as 0.
    pub fn step(&self, state: &CharacterState, world: &World, filter: &Filter, input: &CharacterInput, dt: f64) -> (CharacterState, StepEvents) {
        let p = &self.p;
        let mut ev = StepEvents::default();
        if !(dt > 0.0 && dt < f64::INFINITY) || !state.position.is_finite() {
            return (*state, ev);
        }
        let mut m = Mover { c: self, world, filter, crouched: state.crouched, casts: 0 };
        let mut pos = state.position;
        let mut vel = if state.velocity.is_finite() { state.velocity } else { Vec3::ZERO };

        // 1. out of overlaps
        let (np, pushed) = m.push_out(pos);
        pos = np;
        ev.pushed_out = pushed;

        // 2. the ground at the start
        let probe = 3.0 * p.skin;
        let mut ground = m.probe(pos, vel, probe, true);

        // 3. crouch or stand up
        if input.crouch && !m.crouched {
            if ground.is_none() {
                pos.y += p.height - p.crouch_height; // in the air the legs come up, the head stays
            }
            m.crouched = true;
        } else if !input.crouch && m.crouched {
            match m.stand_up(pos, ground.is_some()) {
                Some(np) => {
                    let lowered = np.y != pos.y;
                    pos = np;
                    m.crouched = false;
                    if lowered {
                        ground = m.probe(pos, vel, probe, true);
                    }
                }
                None => ev.stand_blocked = true,
            }
        }

        // 4. jump, friction, acceleration
        if ground.is_some() && input.jump && self.jump_speed > 0.0 {
            vel.y = self.jump_speed;
            ground = None;
            ev.jumped = true;
        }
        let (dir, amount) = wish_of(input.wish);
        let base = if m.crouched {
            p.crouch_speed
        } else if input.run {
            p.run_speed
        } else {
            p.walk_speed
        };
        let wish_speed = base * amount;
        match ground {
            Some(g) => {
                if amount > 0.0 {
                    let d = along(dir, g.normal).unwrap_or(dir);
                    approach(&mut vel, d * wish_speed, p.accel * wish_speed * dt);
                } else {
                    self.friction(&mut vel, dt);
                }
                vel = reorient(vel, g.normal);
            }
            None => accelerate(&mut vel, dir, wish_speed, p.accel * p.air_control, dt),
        }
        self.limit(&mut vel);

        // 5. move (in the air with the mean velocity of the tick)
        let airborne = ground.is_none();
        let half_g = 0.5 * p.gravity * dt;
        let mut mvel = vel;
        if airborne {
            mvel.y = max(mvel.y - half_g, -p.max_fall_speed);
        }
        let moved = m.move_and_step(pos, mvel, dt, ground);
        pos = moved.pos;
        vel = moved.vel;
        ev.walls = moved.touches.walls;
        ev.hit_ceiling = moved.touches.ceiling;
        ev.stepped_up = moved.stepped_up;

        // 6. the ground at the end: snapped down to, landed on, or what a step landed on
        let end = if moved.stepped.is_some() {
            moved.stepped
        } else {
            let g = if airborne { m.probe(pos, vel, probe, true) } else { m.probe(pos, vel, p.step_height + 2.0 * p.skin, false) };
            if let Some(g) = g {
                let down = g.dist - p.skin;
                if down > SNAP_EPS {
                    pos.y -= down;
                    if !airborne {
                        ev.snapped_down = down;
                    }
                }
            }
            g
        };
        match end {
            Some(g) => {
                if airborne {
                    // landing keeps the horizontal speed, along the ground
                    ev.landed = true;
                    ev.impact_speed = max(moved.impact, max(-vel.dot(g.normal), 0.0));
                    let h = Vec3::new(vel.x, 0.0, vel.z);
                    vel = match along(h, g.normal) {
                        Some(t) => t * h.length(),
                        None => Vec3::ZERO,
                    };
                } else {
                    vel = reorient(vel, g.normal);
                }
            }
            None => {
                if airborne {
                    vel.y = max(vel.y - half_g, -p.max_fall_speed);
                } else {
                    ev.left_ground = true;
                }
            }
        }
        ev.casts = m.casts;
        let out = CharacterState {
            position: pos,
            velocity: vel,
            crouched: m.crouched,
            grounded: end.is_some(),
            ground_normal: match end {
                Some(g) => g.normal,
                None => Vec3::Y,
            },
            ground_body: end.map(|g| g.body),
        };
        (out, ev)
    }

    /// Quake's friction (on the ground without input): the speed drops by friction * max(speed, stop_speed)
    /// * dt, not below 0
    fn friction(&self, vel: &mut Vec3, dt: f64) {
        let speed = vel.length();
        let drop = max(speed, self.p.stop_speed) * self.p.friction * dt;
        if !(speed > drop) {
            *vel = Vec3::ZERO;
            return;
        }
        *vel = *vel * ((speed - drop) / speed);
    }

    /// the horizontal speed at most max_speed, the fall at most max_fall_speed
    fn limit(&self, v: &mut Vec3) {
        let h = sqrt(v.x * v.x + v.z * v.z);
        if h > self.p.max_speed {
            let k = self.p.max_speed / h;
            v.x *= k;
            v.z *= k;
        }
        if v.y < -self.p.max_fall_speed {
            v.y = -self.p.max_fall_speed;
        }
    }
}

/// the unit wish direction (horizontal) and how much of the wish speed (0..1)
fn wish_of(w: Vec3) -> (Vec3, f64) {
    let h = Vec3::new(w.x, 0.0, w.z);
    let l = h.length();
    if !(l > 1e-9 && l < f64::INFINITY) {
        return (Vec3::ZERO, 0.0);
    }
    (h / l, min(l, 1.0))
}

/// `vel` moved towards `target` by at most `step` (the ground with input)
fn approach(vel: &mut Vec3, target: Vec3, step: f64) {
    let dv = target - *vel;
    let l = dv.length();
    if l <= step {
        *vel = target;
    } else if l > 0.0 {
        *vel += dv * (step / l);
    }
}

/// Quake's acceleration (in the air): the speed along `dir` (a unit vector) grows by up to accel * wish_speed * dt,
/// not beyond wish_speed
fn accelerate(vel: &mut Vec3, dir: Vec3, wish_speed: f64, accel: f64, dt: f64) {
    if !(wish_speed > 0.0 && accel > 0.0) {
        return;
    }
    let add = wish_speed - vel.dot(dir);
    if !(add > 0.0) {
        return;
    }
    *vel += dir * min(accel * dt * wish_speed, add);
}

/// the unit direction in the plane of the unit normal `n` with the horizontal heading of `d`
fn along(d: Vec3, n: Vec3) -> Option<Vec3> {
    n.cross(d.cross(Vec3::Y)).try_normalize()
}

/// `v` turned into the plane of `n` with its length and its horizontal heading (nearly straight up or
/// down: only its part in the plane, so a vertical speed never turns into a horizontal one)
fn reorient(v: Vec3, n: Vec3) -> Vec3 {
    let speed = v.length();
    if !(speed > 0.0) {
        return Vec3::ZERO;
    }
    let h = Vec3::new(v.x, 0.0, v.z);
    if !(h.length() > 1e-3 * speed) {
        return clip(v, n);
    }
    match along(h, n) {
        Some(t) => t * speed,
        None => clip(v, n),
    }
}

/// `v` without its part along the unit normal `n`
#[inline]
fn clip(v: Vec3, n: Vec3) -> Vec3 {
    v - n * v.dot(n)
}

/// the horizontal distance of two points
fn horizontal(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    sqrt(dx * dx + dz * dz)
}

/// Quake 3's clip planes (PM_SlideMove): `vel` turned so that it goes into none of `planes`: clipped by the
/// first plane it goes into, along the crease when that sends it into a second, None (stop) when the
/// crease goes into a third
fn clip_planes(planes: &[Vec3], vel: Vec3) -> Option<Vec3> {
    for i in 0..planes.len() {
        if vel.dot(planes[i]) >= -VEL_EPS {
            continue;
        }
        let mut cv = clip(vel, planes[i]);
        for j in 0..planes.len() {
            if j == i || cv.dot(planes[j]) >= -VEL_EPS {
                continue;
            }
            cv = clip(cv, planes[j]);
            if cv.dot(planes[i]) >= -VEL_EPS {
                continue;
            }
            // along the crease of the two planes
            let dir = planes[i].cross(planes[j]).try_normalize()?;
            cv = dir * dir.dot(vel);
            for (k, pk) in planes.iter().enumerate() {
                if k != i && k != j && cv.dot(*pk) < -VEL_EPS {
                    return None;
                }
            }
        }
        return Some(cv);
    }
    Some(vel)
}

/// the queries of one step and their count
struct Mover<'a> {
    c: &'a CharacterController,
    world: &'a World,
    filter: &'a Filter,
    crouched: bool,
    casts: u32,
}

impl Mover<'_> {
    fn center(&self, feet: Vec3) -> Vec3 {
        Vec3::new(feet.x, feet.y + 0.5 * self.c.height(self.crouched), feet.z)
    }

    /// the capsule with its feet at `feet` moved by `motion`: the first contact
    fn cast(&mut self, feet: Vec3, motion: Vec3) -> Option<(BodyId, CastHit)> {
        self.casts += 1;
        let shape = self.c.capsule(self.crouched);
        self.world.cast(&shape, &Iso::at(self.center(feet)), motion, self.filter)
    }

    /// the normal a figure walks along when a cast down from `feet` hit `h`: the surface's own when it is
    /// walkable; on the edge of a walkable surface (a step, a ledge) that surface's, found by a ray down
    /// just beyond the contact, away from the capsule's axis; None: it does not carry the figure
    fn support(&mut self, feet: Vec3, h: &CastHit) -> Option<Vec3> {
        if self.c.walkable(h.normal) {
            return Some(h.normal);
        }
        if h.normal.y < LEDGE_MIN_Y {
            return None;
        }
        let c = self.center(feet);
        let out = Vec3::new(h.point.x - c.x, 0.0, h.point.z - c.z).try_normalize()?;
        let o = h.point + out * self.c.ledge_out + Vec3::new(0.0, LEDGE_UP, 0.0);
        let (_, r) = self.world.ray_cast(o, Vec3::new(0.0, -1.0, 0.0), 2.0 * LEDGE_UP, self.filter)?;
        if self.c.walkable(r.normal) { Some(r.normal) } else { None }
    }

    /// the ground within `dist` below `feet`; with `leave` not when the velocity moves away from it
    fn probe(&mut self, feet: Vec3, vel: Vec3, dist: f64, leave: bool) -> Option<Ground> {
        let (body, h) = self.cast(feet, Vec3::new(0.0, -dist, 0.0))?;
        let normal = self.support(feet, &h)?;
        if leave && vel.dot(normal) > LEAVE_SPEED {
            return None;
        }
        Some(Ground { normal, body, dist: h.fraction * dist })
    }

    /// pushed out of overlaps: the deepest contact first, along its normal by its depth and the skin, as a
    /// cast (so the push goes through nothing else), up to four rounds -> (feet, distance moved)
    fn push_out(&mut self, mut feet: Vec3) -> (Vec3, f64) {
        let skin = self.c.p.skin;
        let mut moved = 0.0;
        let mut cs: Vec<(BodyId, Contact)> = Vec::new();
        for _ in 0..MAX_PUSH_OUT {
            cs.clear();
            let shape = self.c.capsule(self.crouched);
            self.world.contacts(&shape, &Iso::at(self.center(feet)), 0.0, self.filter, &mut cs);
            let mut deepest: Option<Contact> = None;
            for (_, c) in &cs {
                if c.depth > OVERLAP_TOL && deepest.is_none_or(|d| c.depth > d.depth) {
                    deepest = Some(*c);
                }
            }
            let Some(c) = deepest else {
                break;
            };
            let push = c.normal * (c.depth + skin);
            let len = push.length();
            let go = match self.cast(feet, push) {
                None => 1.0,
                Some((_, h)) => max(h.fraction - skin / len, 0.0),
            };
            if !(go > 0.0) {
                break; // wedged: the casts of the move sort it out or leave it
            }
            feet += push * go;
            moved += len * go;
        }
        (feet, moved)
    }

    /// the feet after standing up from crouched at `feet`, None without room: on the ground the head goes
    /// up; in the air the legs go down first, as far as there is room, and the head up by the rest.  The
    /// cast of the crouched capsule up by the height difference covers exactly the standing capsule.
    fn stand_up(&mut self, feet: Vec3, grounded: bool) -> Option<Vec3> {
        let p = self.c.p;
        let dh = p.height - p.crouch_height;
        if !(dh > 0.0) {
            return Some(feet);
        }
        let mut down = 0.0;
        if !grounded {
            let d = dh + p.skin;
            down = match self.cast(feet, Vec3::new(0.0, -d, 0.0)) {
                None => dh,
                Some((_, h)) => clamp(h.fraction * d - p.skin, 0.0, dh),
            };
        }
        let lowered = Vec3::new(feet.x, feet.y - down, feet.z);
        if self.cast(lowered, Vec3::new(0.0, dh, 0.0)).is_some() {
            return None;
        }
        Some(lowered)
    }

    /// the move of one tick with `vel`, on the ground with a step when that gets farther
    fn move_and_step(&mut self, pos: Vec3, vel: Vec3, dt: f64, ground: Option<Ground>) -> Moved {
        let first = self.slide(pos, vel, dt, ground, false);
        let plain = Moved { pos: first.pos, vel: first.vel, touches: first.touches, impact: first.impact, stepped: None, stepped_up: 0.0 };
        let p = self.c.p;
        if ground.is_none() || !first.blocked || !(p.step_height > p.skin) {
            return plain;
        }
        // up as far as there is room, at most step_height
        let up = match self.cast(pos, Vec3::new(0.0, p.step_height, 0.0)) {
            None => p.step_height,
            Some((_, h)) => h.fraction * p.step_height - p.skin,
        };
        if !(up > p.skin) {
            return plain;
        }
        // forward horizontally at the same speed, riding up nothing
        let h = Vec3::new(vel.x, 0.0, vel.z);
        let hl = h.length();
        if !(hl > 0.0) {
            return plain;
        }
        let raised = Vec3::new(pos.x, pos.y + up, pos.z);
        let second = self.slide(raised, h * (vel.length() / hl), dt, None, true);
        // down onto walkable ground
        let down = up + 2.0 * p.skin;
        let Some((body, dh)) = self.cast(second.pos, Vec3::new(0.0, -down, 0.0)) else {
            return plain;
        };
        // carried at most step_height above the feet (the capsule's sphere could hang on a higher edge)
        if !(dh.point.y - pos.y <= p.step_height) {
            return plain;
        }
        let Some(normal) = self.support(second.pos, &dh) else {
            return plain;
        };
        let gap = dh.fraction * down;
        let landed = Vec3::new(second.pos.x, second.pos.y - max(gap - p.skin, 0.0), second.pos.z);
        if !(horizontal(landed, pos) > horizontal(first.pos, pos) + STEP_GAIN) {
            return plain;
        }
        Moved {
            pos: landed,
            vel: second.vel,
            touches: second.touches,
            impact: 0.0,
            stepped: Some(Ground { normal, body, dist: min(gap, p.skin) }),
            stepped_up: max(landed.y - pos.y, 0.0),
        }
    }

    /// collide and slide: up to four casts, each stopping `skin` before the contact along the motion, the
    /// velocity turned by Quake 3's clip planes; `flat`: every surface counts as a vertical wall (the
    /// forward move of a step)
    fn slide(&mut self, mut pos: Vec3, mut vel: Vec3, dt: f64, ground: Option<Ground>, flat: bool) -> Slid {
        let c = self.c;
        let skin = c.p.skin;
        let mut planes = [Vec3::ZERO; MAX_PLANES];
        let mut np = 0;
        let on_ground = ground.is_some();
        // never turn against the ground or the original direction
        if let Some(g) = ground {
            planes[np] = g.normal;
            np += 1;
        }
        if let Some(d) = vel.try_normalize() {
            planes[np] = d;
            np += 1;
        }
        let mut out = Slid { pos, vel, blocked: false, touches: Touches::default(), impact: 0.0 };
        let mut time_left = dt;
        for _ in 0..MAX_BUMPS {
            let motion = vel * time_left;
            let len = motion.length();
            if !(len > MIN_MOVE) {
                break;
            }
            let Some((body, h)) = self.cast(pos, motion) else {
                pos += motion;
                break;
            };
            let go = max(h.fraction - skin / len, 0.0);
            pos += motion * go;
            time_left -= time_left * go;
            let n = h.normal;
            let walkable = c.walkable(n);
            if !walkable {
                if n.y <= -c.cos_slope {
                    out.touches.ceiling = true;
                } else {
                    out.touches.wall(Touch { body, point: h.point, normal: n, triangle: h.triangle });
                }
            }
            if walkable && on_ground {
                // walking onto walkable ground: along it at the same speed
                vel = reorient(vel, n);
                np = 0;
                planes[np] = n;
                np += 1;
                if let Some(d) = vel.try_normalize() {
                    planes[np] = d;
                    np += 1;
                }
                continue;
            }
            if walkable && !flat {
                out.impact = max(out.impact, -vel.dot(n));
            }
            let mut cn = n;
            if on_ground || flat {
                // on the ground a steep surface is a vertical wall: pushing into it climbs nothing; the
                // forward move of a step is horizontal, it rides up nothing
                if on_ground && n.y > -c.cos_slope {
                    out.blocked = true;
                }
                if let Some(f) = Vec3::new(n.x, 0.0, n.z).try_normalize() {
                    cn = f;
                }
            }
            if planes[..np].iter().any(|q| q.dot(cn) > 0.99) {
                // the same plane again (rounding): push off it a little
                vel = clip(vel, cn) + cn * NUDGE;
                continue;
            }
            if np == MAX_PLANES {
                vel = Vec3::ZERO;
                break;
            }
            planes[np] = cn;
            np += 1;
            match clip_planes(&planes[..np], vel) {
                Some(v) => vel = v,
                None => {
                    vel = Vec3::ZERO;
                    break;
                }
            }
        }
        out.pos = pos;
        out.vel = vel;
        out
    }
}
