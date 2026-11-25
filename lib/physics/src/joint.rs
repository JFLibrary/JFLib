// joint -- constraints between two bodies: ball (with a swing cone and a twist range, for ragdolls),
// hinge (limits, motor, spring), fixed, slider (limits, motor, spring), distance (rope, rod, spring)
//
// Every joint has a frame on each body: its origin is the anchor, its X axis the axis (of a hinge, a
// slider, the twist of a ball joint).  The parts are solved one after the other in every pass of the solver
// (solver.rs): motor and spring first, then the limits, the locked rotations, the anchors last (what must
// hold best).  Soft as the contacts: with the bias the error goes away like a damped spring of the
// simulation's joint frequency, the relax pass removes the velocity that leaves.  Limits are one-sided and
// speculative like contacts: a hinge that is still short of its limit may turn exactly up to it in a
// substep.  The impulses are kept from step to step (warm start).

use crate::math::{Iso, Mat3, Quat, Vec3, atan2, clamp, max};
use crate::solver::{NONE, SBody, Soft, apply, apply_angular, load, pair, relative_velocity};
use crate::world::BodyId;

/// a joint of a [`crate::Simulation`]: an index and a generation, like [`BodyId`]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JointId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl JointId {
    /// the slot (used again after a remove)
    pub fn index(self) -> u32 {
        self.index
    }

    /// how often the slot was used before
    pub fn generation(self) -> u32 {
        self.generation
    }
}

/// drives a hinge (rad/s) or a slider (m/s) towards a speed with at most `max_force` (N m or N)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motor {
    /// the speed it drives towards
    pub speed: f64,
    /// the largest torque or force it uses
    pub max_force: f64,
}

/// pulls a hinge's angle (rad), a slider's position (m) or a distance joint's length (m) towards `target`
/// like a damped spring
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// the angle, position or length it pulls towards
    pub target: f64,
    /// how fast it oscillates (Hz); 0 is no spring
    pub hertz: f64,
    /// 1 is critically damped, below oscillates, above creeps
    pub damping_ratio: f64,
}

/// what a joint allows
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// the anchors stay together, the bodies turn freely; optionally the X axes of the two frames stay within
    /// a cone of this half angle (radians, the swing) and body b turns about its X axis only within
    /// (low, high) radians relative to a (the twist): a shoulder, a hip, a neck
    Ball { cone: Option<f64>, twist: Option<(f64, f64)> },
    /// the anchors stay together, the bodies turn about the X axis only, by an angle that starts at 0 when the
    /// frames are aligned; within (low, high) radians, driven by a motor, pulled by a spring
    Hinge { limits: Option<(f64, f64)>, motor: Option<Motor>, spring: Option<Spring> },
    /// no relative motion at all
    Fixed,
    /// no rotation; b's anchor moves along a's X axis only, at a position that starts at 0 on a's anchor;
    /// within (low, high) metres, driven by a motor, pulled by a spring
    Slider { limits: Option<(f64, f64)>, motor: Option<Motor>, spring: Option<Spring> },
    /// the anchors stay between `min` and `max` apart (a rope: min 0; a rod: min = max; `max` may be
    /// infinite), free otherwise, with a spring towards its target length between them
    Distance { min: f64, max: f64, spring: Option<Spring> },
}

impl JointKind {
    /// a ball joint without limits
    pub const BALL: JointKind = JointKind::Ball { cone: None, twist: None };
    /// a hinge without limits, motor or spring
    pub const HINGE: JointKind = JointKind::Hinge { limits: None, motor: None, spring: None };
    /// a slider without limits, motor or spring
    pub const SLIDER: JointKind = JointKind::Slider { limits: None, motor: None, spring: None };

    /// a rod of length `length`
    pub const fn rod(length: f64) -> JointKind {
        JointKind::Distance { min: length, max: length, spring: None }
    }

    /// a rope of at most `length`
    pub const fn rope(length: f64) -> JointKind {
        JointKind::Distance { min: 0.0, max: length, spring: None }
    }
}

/// A joint between bodies `a` and `b`: its frame on each (in the body's own frame, the frame its shape is
/// in), what it allows, whether the two bodies still collide with each other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    /// the first body
    pub a: BodyId,
    /// the second body (limits, motors and positions are b relative to a)
    pub b: BodyId,
    /// the joint's frame in a's frame: the origin is the anchor, X the axis
    pub frame_a: Iso,
    /// the joint's frame in b's frame
    pub frame_b: Iso,
    /// what it allows
    pub kind: JointKind,
    /// do a and b still collide with each other (default false)
    pub collide_connected: bool,
    /// free for the caller
    pub user: u64,
}

/// why a joint was refused
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointError {
    /// a body id is dead
    DeadBody,
    /// both ends are the same body
    SameBody,
    /// neither end is a dynamic body (nothing could move)
    NothingMoves,
    /// a frame, a limit, a motor or a spring is not finite
    NotFinite,
    /// a limit's low end above its high end, a negative cone, length, force, frequency or damping
    Range,
}

impl Joint {
    /// the joint as given with unit rotations, or why it cannot be one (the bodies are checked by the
    /// simulation)
    pub(crate) fn checked(&self) -> Result<Joint, JointError> {
        let fin = |i: &Iso| i.pos.is_finite() && i.rot.is_finite();
        if !fin(&self.frame_a) || !fin(&self.frame_b) {
            return Err(JointError::NotFinite);
        }
        let range = |r: Option<(f64, f64)>| match r {
            None => Ok(()),
            Some((lo, hi)) if lo.is_nan() || hi.is_nan() => Err(JointError::NotFinite),
            Some((lo, hi)) if lo > hi || lo == f64::INFINITY || hi == f64::NEG_INFINITY => Err(JointError::Range),
            Some(_) => Ok(()),
        };
        let motor = |m: Option<Motor>| match m {
            None => Ok(()),
            Some(m) if !m.speed.is_finite() || !m.max_force.is_finite() => Err(JointError::NotFinite),
            Some(m) if m.max_force < 0.0 => Err(JointError::Range),
            Some(_) => Ok(()),
        };
        let spring = |s: Option<Spring>| match s {
            None => Ok(()),
            Some(s) if !s.target.is_finite() || !s.hertz.is_finite() || !s.damping_ratio.is_finite() => Err(JointError::NotFinite),
            Some(s) if s.hertz < 0.0 || s.damping_ratio < 0.0 => Err(JointError::Range),
            Some(_) => Ok(()),
        };
        match self.kind {
            JointKind::Ball { cone, twist } => {
                if let Some(c) = cone {
                    if c.is_nan() {
                        return Err(JointError::NotFinite);
                    }
                    if c < 0.0 {
                        return Err(JointError::Range);
                    }
                }
                range(twist)?;
            }
            JointKind::Hinge { limits, motor: m, spring: s } | JointKind::Slider { limits, motor: m, spring: s } => {
                range(limits)?;
                motor(m)?;
                spring(s)?;
            }
            JointKind::Fixed => {}
            JointKind::Distance { min: lo, max: hi, spring: s } => {
                if lo.is_nan() || hi.is_nan() || !lo.is_finite() {
                    return Err(JointError::NotFinite);
                }
                if lo < 0.0 || hi < lo {
                    return Err(JointError::Range);
                }
                spring(s)?;
            }
        }
        let mut j = *self;
        j.frame_a.rot = j.frame_a.rot.normalize();
        j.frame_b.rot = j.frame_b.rot.normalize();
        Ok(j)
    }
}

/// what is left of a joint's impulses when they are applied again in the next substep.  Conflicting joints on
/// one body (a body welded to two others) build up large impulses against each other that cancel; when the
/// body turns they no longer do, and the whole of them would push it (as Bullet's warm start factor, which is
/// 0.85 there)
const WARM: f64 = 0.85;

/// the accumulated impulses of a joint's parts (per substep)
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Acc {
    pub point: Vec3,
    pub ang: Vec3,
    pub perp: [f64; 2],
    pub lower: f64,
    pub upper: f64,
    pub motor: f64,
    pub spring: f64,
    pub cone: f64,
}

/// a joint with what the solver keeps of it
#[derive(Clone, Copy, Debug)]
pub(crate) struct JointRt {
    pub joint: Joint,
    /// the slots of a and b
    pub ia: u32,
    pub ib: u32,
    /// their solver indices in this step
    pub sa: u32,
    pub sb: u32,
    /// the anchors relative to the centres of mass, the centres' offset, the frames' rotations (world, at
    /// the step's start)
    pub ra0: Vec3,
    pub rb0: Vec3,
    pub dc: Vec3,
    pub qa0: Quat,
    pub qb0: Quat,
    pub spring_soft: Soft,
    pub acc: Acc,
    /// the substep of the last step (for forces)
    pub h: f64,
    /// how far a and b moved and turned in the last step (the basis its last pass solved in, for forces)
    pub end: [(Vec3, Quat); 2],
}

/// the geometry of a joint now (in the substep)
struct Geo {
    ra: Vec3,
    rb: Vec3,
    /// from a's anchor to b's anchor
    d: Vec3,
    qa: Quat,
    qb: Quat,
    xa: Vec3,
    ya: Vec3,
    za: Vec3,
    xb: Vec3,
}

/// the angle of b's frame about a's X axis (radians in [-pi, pi]) from the frames' rotations
fn twist_angle(qa: Quat, qb: Quat) -> f64 {
    let mut r = qa.conjugate().mul(qb);
    if r.w < 0.0 {
        r = Quat::new(-r.x, -r.y, -r.z, -r.w);
    }
    2.0 * atan2(r.x, r.w)
}

/// the rotation error of b's frame against a's (a rotation vector, small)
fn rotation_error(qa: Quat, qb: Quat) -> Vec3 {
    let mut e = qb.mul(qa.conjugate());
    if e.w < 0.0 {
        e = Quat::new(-e.x, -e.y, -e.z, -e.w);
    }
    Vec3::new(e.x, e.y, e.z) * 2.0
}

impl Acc {
    fn scale(&mut self, k: f64) {
        self.point *= k;
        self.ang *= k;
        self.perp = [self.perp[0] * k, self.perp[1] * k];
        self.lower *= k;
        self.upper *= k;
        self.motor *= k;
        self.spring *= k;
        self.cone *= k;
    }
}

impl JointRt {
    pub(crate) fn new(joint: Joint) -> JointRt {
        JointRt {
            joint,
            ia: joint.a.index(),
            ib: joint.b.index(),
            sa: NONE,
            sb: NONE,
            ra0: Vec3::ZERO,
            rb0: Vec3::ZERO,
            dc: Vec3::ZERO,
            qa0: Quat::IDENTITY,
            qb0: Quat::IDENTITY,
            spring_soft: Soft::RIGID,
            acc: Acc::default(),
            h: 0.0,
            end: [(Vec3::ZERO, Quat::IDENTITY); 2],
        }
    }

    /// the frames at the step's start: each body's (centre of mass, rotation, local centre of mass)
    pub(crate) fn prepare(&mut self, a: (Vec3, Quat, Vec3), b: (Vec3, Quat, Vec3), h: f64) {
        let j = &self.joint;
        self.ra0 = a.1.rotate(j.frame_a.pos - a.2);
        self.rb0 = b.1.rotate(j.frame_b.pos - b.2);
        self.dc = b.0 - a.0;
        self.qa0 = a.1.mul(j.frame_a.rot);
        self.qb0 = b.1.mul(j.frame_b.rot);
        let spring = match j.kind {
            JointKind::Hinge { spring, .. } | JointKind::Slider { spring, .. } | JointKind::Distance { spring, .. } => spring,
            _ => None,
        };
        self.spring_soft = match spring {
            Some(s) if s.hertz > 0.0 => Soft::new(s.hertz, s.damping_ratio, h),
            _ => Soft::RIGID,
        };
        self.h = h;
        self.end = [(Vec3::ZERO, Quat::IDENTITY); 2];
    }

    /// the axis of a distance joint: from a's anchor to b's, with the anchors together from a's centre to b's,
    /// with those together too the frame's X axis
    fn distance_axis(&self, g: &Geo, a: &SBody, b: &SBody) -> Vec3 {
        if let Some(u) = g.d.try_normalize() {
            return u;
        }
        (self.dc + (b.dp - a.dp)).try_normalize().unwrap_or(g.xa)
    }

    /// keep what the last pass solved in: the motion of a and b in the step (`joint_force` reads it)
    pub(crate) fn finish(&mut self, bodies: &[SBody]) {
        let (a, b) = (load(bodies, self.sa), load(bodies, self.sb));
        self.end = [(a.dp, a.dq), (b.dp, b.dq)];
    }

    /// forget the impulses of parts the kind no longer has (a motor, a spring, limits taken away), so the
    /// warm start does not apply them again; the others stay
    pub(crate) fn drop_unused(&mut self) {
        let acc = &mut self.acc;
        let spring_on = |s: Option<Spring>| s.is_some_and(|s| s.hertz > 0.0);
        match self.joint.kind {
            JointKind::Ball { cone, twist } => {
                acc.ang = Vec3::ZERO;
                acc.perp = [0.0; 2];
                acc.motor = 0.0;
                acc.spring = 0.0;
                if cone.is_none() {
                    acc.cone = 0.0;
                }
                if twist.is_none() {
                    acc.lower = 0.0;
                    acc.upper = 0.0;
                }
            }
            JointKind::Hinge { limits, motor, spring } | JointKind::Slider { limits, motor, spring } => {
                acc.cone = 0.0;
                if let JointKind::Hinge { .. } = self.joint.kind {
                    acc.ang = Vec3::ZERO;
                } else {
                    acc.point = Vec3::ZERO;
                }
                if limits.is_none() {
                    acc.lower = 0.0;
                    acc.upper = 0.0;
                }
                if motor.is_none() {
                    acc.motor = 0.0;
                }
                if !spring_on(spring) {
                    acc.spring = 0.0;
                }
            }
            JointKind::Fixed => {
                acc.perp = [0.0; 2];
                acc.lower = 0.0;
                acc.upper = 0.0;
                acc.motor = 0.0;
                acc.spring = 0.0;
                acc.cone = 0.0;
            }
            JointKind::Distance { min, max, spring } => {
                acc.ang = Vec3::ZERO;
                acc.perp = [0.0; 2];
                acc.motor = 0.0;
                acc.cone = 0.0;
                acc.point.y = 0.0;
                acc.point.z = 0.0;
                if min < max {
                    acc.point.x = 0.0;
                    if !spring_on(spring) {
                        acc.spring = 0.0;
                    }
                    if !(min > 0.0) {
                        acc.lower = 0.0;
                    }
                    if !(max < f64::INFINITY) {
                        acc.upper = 0.0;
                    }
                } else {
                    acc.lower = 0.0;
                    acc.upper = 0.0;
                    acc.spring = 0.0;
                }
            }
        }
    }

    fn geo(&self, a: &SBody, b: &SBody) -> Geo {
        let ra = a.dq.rotate(self.ra0);
        let rb = b.dq.rotate(self.rb0);
        let d = self.dc + (b.dp - a.dp) + rb - ra;
        let qa = a.dq.mul(self.qa0);
        let qb = b.dq.mul(self.qb0);
        Geo { ra, rb, d, qa, qb, xa: qa.rotate(Vec3::X), ya: qa.rotate(Vec3::Y), za: qa.rotate(Vec3::Z), xb: qb.rotate(Vec3::X) }
    }

    /// the joint's coordinate now: a hinge's angle, a slider's position, a distance joint's length, a ball
    /// joint's twist (0 for a fixed joint), from the poses of the step's start
    pub(crate) fn position(&self) -> f64 {
        let g = self.geo(&SBody::STATIC, &SBody::STATIC);
        match self.joint.kind {
            JointKind::Hinge { .. } | JointKind::Ball { .. } => twist_angle(g.qa, g.qb),
            JointKind::Slider { .. } => g.xa.dot(g.d),
            JointKind::Distance { .. } => g.d.length(),
            JointKind::Fixed => 0.0,
        }
    }

    /// the force (N) and torque (N m) the joint applied to b in the last substep (world; a got the opposite)
    pub(crate) fn reaction(&self) -> (Vec3, Vec3) {
        if !(self.h > 0.0) {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let ea = SBody { dp: self.end[0].0, dq: self.end[0].1, ..SBody::STATIC };
        let eb = SBody { dp: self.end[1].0, dq: self.end[1].1, ..SBody::STATIC };
        let g = self.geo(&ea, &eb);
        let acc = &self.acc;
        let (lin, ang) = match self.joint.kind {
            JointKind::Ball { .. } => {
                let cone = g.xa.cross(g.xb).try_normalize().map(|n| -n * acc.cone).unwrap_or(Vec3::ZERO);
                let t = (g.xa + g.xb).normalize_or_zero();
                (acc.point, cone + t * (acc.lower - acc.upper))
            }
            JointKind::Hinge { .. } => {
                (acc.point, g.ya * acc.perp[0] + g.za * acc.perp[1] + g.xa * (acc.spring + acc.motor + acc.lower - acc.upper))
            }
            JointKind::Fixed => (acc.point, acc.ang),
            JointKind::Slider { .. } => {
                (g.ya * acc.perp[0] + g.za * acc.perp[1] + g.xa * (acc.spring + acc.motor + acc.lower - acc.upper), acc.ang)
            }
            JointKind::Distance { .. } => (self.distance_axis(&g, &ea, &eb) * (acc.spring + acc.lower - acc.upper + acc.point.x), Vec3::ZERO),
        };
        (lin / self.h, ang / self.h)
    }

    pub(crate) fn warm_start(&mut self, bodies: &mut [SBody]) {
        let (a, b) = pair(bodies, self.sa, self.sb);
        let g = self.geo(a, b);
        self.acc.scale(WARM);
        let acc = &self.acc;
        match self.joint.kind {
            JointKind::Ball { .. } => {
                if let Some(n) = g.xa.cross(g.xb).try_normalize() {
                    apply_angular(a, b, -n * acc.cone);
                }
                let t = (g.xa + g.xb).normalize_or_zero();
                apply_angular(a, b, t * (acc.lower - acc.upper));
                apply(a, b, g.ra, g.rb, acc.point);
            }
            JointKind::Hinge { .. } => {
                apply_angular(a, b, g.xa * (acc.spring + acc.motor + acc.lower - acc.upper));
                apply_angular(a, b, g.ya * acc.perp[0] + g.za * acc.perp[1]);
                apply(a, b, g.ra, g.rb, acc.point);
            }
            JointKind::Fixed => {
                apply_angular(a, b, acc.ang);
                apply(a, b, g.ra, g.rb, acc.point);
            }
            JointKind::Slider { .. } => {
                let ra = g.ra + g.d;
                apply(a, b, ra, g.rb, g.xa * (acc.spring + acc.motor + acc.lower - acc.upper));
                apply_angular(a, b, acc.ang);
                apply(a, b, ra, g.rb, g.ya * acc.perp[0] + g.za * acc.perp[1]);
            }
            JointKind::Distance { .. } => {
                let u = self.distance_axis(&g, a, b);
                apply(a, b, g.ra, g.rb, u * (acc.spring + acc.lower - acc.upper + acc.point.x));
            }
        }
    }

    /// one pass of the solver; `soft` is the joints' softness (used with the bias)
    pub(crate) fn solve(&mut self, bodies: &mut [SBody], soft: &Soft, inv_h: f64, use_bias: bool) {
        let (a, b) = pair(bodies, self.sa, self.sb);
        let bias = if use_bias { *soft } else { Soft::RIGID };
        let h = if inv_h > 0.0 { 1.0 / inv_h } else { 0.0 };
        let spring_soft = self.spring_soft;
        match self.joint.kind {
            JointKind::Ball { cone, twist } => {
                let g = self.geo(a, b);
                if let Some(cmax) = cone {
                    let c = g.xa.cross(g.xb);
                    let sn = c.length();
                    if sn > 1e-12 {
                        let phi = atan2(sn, g.xa.dot(g.xb));
                        angular_limit(a, b, -(c / sn), cmax - phi, &mut self.acc.cone, &bias, inv_h);
                    } else if g.xa.dot(g.xb) < 0.0 {
                        // the axes point against each other: any axis across a's turns b back
                        let n = g.xa.any_perpendicular();
                        angular_limit(a, b, -n, cmax - core::f64::consts::PI, &mut self.acc.cone, &bias, inv_h);
                    }
                }
                if let Some((lo, hi)) = twist {
                    if let Some(t) = (g.xa + g.xb).try_normalize() {
                        let th = twist_angle(g.qa, g.qb);
                        angular_limit(a, b, t, th - lo, &mut self.acc.lower, &bias, inv_h);
                        angular_limit(a, b, -t, hi - th, &mut self.acc.upper, &bias, inv_h);
                    }
                }
                let g = self.geo(a, b);
                point(a, b, g.ra, g.rb, g.d, &mut self.acc.point, &bias);
            }
            JointKind::Hinge { limits, motor, spring } => {
                let g = self.geo(a, b);
                let axis = g.xa;
                if let Some(s) = spring {
                    if s.hertz > 0.0 {
                        let th = twist_angle(g.qa, g.qb);
                        angular_spring(a, b, axis, th - s.target, &mut self.acc.spring, &spring_soft);
                    }
                }
                if let Some(m) = motor {
                    angular_motor(a, b, axis, m.speed, m.max_force * h, &mut self.acc.motor);
                }
                if let Some((lo, hi)) = limits {
                    let th = twist_angle(g.qa, g.qb);
                    angular_limit(a, b, axis, th - lo, &mut self.acc.lower, &bias, inv_h);
                    angular_limit(a, b, -axis, hi - th, &mut self.acc.upper, &bias, inv_h);
                }
                hinge_lock(a, b, &g, &mut self.acc.perp, &bias);
                let g = self.geo(a, b);
                point(a, b, g.ra, g.rb, g.d, &mut self.acc.point, &bias);
            }
            JointKind::Fixed => {
                let g = self.geo(a, b);
                rotation_lock(a, b, &g, &mut self.acc.ang, &bias);
                point(a, b, g.ra, g.rb, g.d, &mut self.acc.point, &bias);
            }
            JointKind::Slider { limits, motor, spring } => {
                let g = self.geo(a, b);
                let ra = g.ra + g.d;
                let x = g.xa.dot(g.d);
                if let Some(s) = spring {
                    if s.hertz > 0.0 {
                        linear_spring(a, b, ra, g.rb, g.xa, x - s.target, &mut self.acc.spring, &spring_soft);
                    }
                }
                if let Some(m) = motor {
                    linear_motor(a, b, ra, g.rb, g.xa, m.speed, m.max_force * h, &mut self.acc.motor);
                }
                if let Some((lo, hi)) = limits {
                    linear_limit(a, b, ra, g.rb, g.xa, x - lo, &mut self.acc.lower, &bias, inv_h);
                    linear_limit(a, b, ra, g.rb, -g.xa, hi - x, &mut self.acc.upper, &bias, inv_h);
                }
                rotation_lock(a, b, &g, &mut self.acc.ang, &bias);
                slider_lock(a, b, &g, &mut self.acc.perp, &bias);
            }
            JointKind::Distance { min: lo, max: hi, spring } => {
                let g = self.geo(a, b);
                let len = g.d.length();
                // anchors together: a rope (min 0) pulls nothing, a rod or a minimum pushes apart along a fixed
                // axis (from a's centre to b's, else the frame's X)
                if len > 1e-9 || lo > 0.0 {
                    let u = self.distance_axis(&g, a, b);
                    if lo < hi {
                        if let Some(s) = spring {
                            if s.hertz > 0.0 {
                                let target = clamp(s.target, lo, hi);
                                linear_spring(a, b, g.ra, g.rb, u, len - target, &mut self.acc.spring, &spring_soft);
                            }
                        }
                        if lo > 0.0 {
                            linear_limit(a, b, g.ra, g.rb, u, len - lo, &mut self.acc.lower, &bias, inv_h);
                        }
                        if hi < f64::INFINITY {
                            linear_limit(a, b, g.ra, g.rb, -u, hi - len, &mut self.acc.upper, &bias, inv_h);
                        }
                    } else {
                        // a rod: both ways (the impulse is kept in point.x)
                        let err = crate::math::clamp(len - lo, -MAX_LINEAR, MAX_LINEAR);
                        linear_spring(a, b, g.ra, g.rb, u, err, &mut self.acc.point.x, &bias);
                    }
                }
            }
        }
    }
}

/// the effective mass about `axis` (1 / (axis (Ia + Ib) axis)), 0 when neither can turn
#[inline]
fn angular_mass(a: &SBody, b: &SBody, axis: Vec3) -> f64 {
    let k = axis.dot(a.inv_i.mul_vec(axis)) + axis.dot(b.inv_i.mul_vec(axis));
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

#[inline]
fn linear_mass(a: &SBody, b: &SBody, ra: Vec3, rb: Vec3, u: Vec3) -> f64 {
    crate::solver::axis_mass(a, b, ra, rb, u)
}

/// at most this much of an error goes into the bias of a pass (m; rad): a joint that starts far off its limits
/// or is torn apart comes back steadily, not in one jolt (as Box2D 2.4's maxLinearCorrection and
/// maxAngularCorrection); springs are not limited
const MAX_LINEAR: f64 = 0.2;
const MAX_ANGULAR: f64 = 8.0 * core::f64::consts::PI / 180.0;

/// `v` no longer than `cap`
#[inline]
fn capped(v: Vec3, cap: f64) -> Vec3 {
    let l = v.length_sq();
    if l > cap * cap { v * (cap / crate::math::sqrt(l)) } else { v }
}

/// the bias, mass scale and impulse scale of a one-sided limit with error `c` (>= 0 wanted): speculative while
/// it is open, the overshoot taken back at most `cap` at a time
#[inline]
fn limit_terms(c: f64, soft: &Soft, inv_h: f64, cap: f64) -> (f64, f64, f64) {
    if c > 0.0 { (c * inv_h, 1.0, 0.0) } else { (soft.bias_rate * max(c, -cap), soft.mass_scale, soft.impulse_scale) }
}

/// c >= 0 with dc/dt = axis . (wb - wa)
fn angular_limit(a: &mut SBody, b: &mut SBody, axis: Vec3, c: f64, acc: &mut f64, soft: &Soft, inv_h: f64) {
    let m = angular_mass(a, b, axis);
    if !(m > 0.0) {
        return;
    }
    let (bias, ms, is) = limit_terms(c, soft, inv_h, MAX_ANGULAR);
    let cdot = axis.dot(b.w - a.w);
    let imp = -m * ms * (cdot + bias) - is * *acc;
    let total = max(*acc + imp, 0.0);
    let imp = total - *acc;
    *acc = total;
    apply_angular(a, b, axis * imp);
}

/// pull c towards 0 (two-sided, soft)
fn angular_spring(a: &mut SBody, b: &mut SBody, axis: Vec3, c: f64, acc: &mut f64, soft: &Soft) {
    let m = angular_mass(a, b, axis);
    if !(m > 0.0) {
        return;
    }
    let cdot = axis.dot(b.w - a.w);
    let imp = -m * soft.mass_scale * (cdot + soft.bias_rate * c) - soft.impulse_scale * *acc;
    *acc += imp;
    apply_angular(a, b, axis * imp);
}

fn angular_motor(a: &mut SBody, b: &mut SBody, axis: Vec3, speed: f64, max_imp: f64, acc: &mut f64) {
    let m = angular_mass(a, b, axis);
    if !(m > 0.0) {
        return;
    }
    let cdot = axis.dot(b.w - a.w) - speed;
    let total = clamp(*acc - m * cdot, -max_imp, max_imp);
    let imp = total - *acc;
    *acc = total;
    apply_angular(a, b, axis * imp);
}

/// c >= 0 with dc/dt = u . (velocity of b's anchor - velocity of a's point ra)
#[allow(clippy::too_many_arguments)]
fn linear_limit(a: &mut SBody, b: &mut SBody, ra: Vec3, rb: Vec3, u: Vec3, c: f64, acc: &mut f64, soft: &Soft, inv_h: f64) {
    let m = linear_mass(a, b, ra, rb, u);
    if !(m > 0.0) {
        return;
    }
    let (bias, ms, is) = limit_terms(c, soft, inv_h, MAX_LINEAR);
    let cdot = relative_velocity(a, b, ra, rb).dot(u);
    let imp = -m * ms * (cdot + bias) - is * *acc;
    let total = max(*acc + imp, 0.0);
    let imp = total - *acc;
    *acc = total;
    apply(a, b, ra, rb, u * imp);
}

#[allow(clippy::too_many_arguments)]
fn linear_spring(a: &mut SBody, b: &mut SBody, ra: Vec3, rb: Vec3, u: Vec3, c: f64, acc: &mut f64, soft: &Soft) {
    let m = linear_mass(a, b, ra, rb, u);
    if !(m > 0.0) {
        return;
    }
    let cdot = relative_velocity(a, b, ra, rb).dot(u);
    let imp = -m * soft.mass_scale * (cdot + soft.bias_rate * c) - soft.impulse_scale * *acc;
    *acc += imp;
    apply(a, b, ra, rb, u * imp);
}

#[allow(clippy::too_many_arguments)]
fn linear_motor(a: &mut SBody, b: &mut SBody, ra: Vec3, rb: Vec3, u: Vec3, speed: f64, max_imp: f64, acc: &mut f64) {
    let m = linear_mass(a, b, ra, rb, u);
    if !(m > 0.0) {
        return;
    }
    let cdot = relative_velocity(a, b, ra, rb).dot(u) - speed;
    let total = clamp(*acc - m * cdot, -max_imp, max_imp);
    let imp = total - *acc;
    *acc = total;
    apply(a, b, ra, rb, u * imp);
}

/// the anchors together: K = (ma + mb) 1 - [ra] Ia [ra] - [rb] Ib [rb]
fn point(a: &mut SBody, b: &mut SBody, ra: Vec3, rb: Vec3, c: Vec3, acc: &mut Vec3, soft: &Soft) {
    let (sa, sb) = (Mat3::skew(ra), Mat3::skew(rb));
    let k = Mat3::IDENTITY.scale(a.inv_mass + b.inv_mass).sub(&sa.mul(&a.inv_i).mul(&sa)).sub(&sb.mul(&b.inv_i).mul(&sb));
    let Some(kinv) = k.inverse() else {
        return;
    };
    let cdot = relative_velocity(a, b, ra, rb);
    let c = capped(c, MAX_LINEAR);
    let imp = -kinv.mul_vec(cdot + c * soft.bias_rate) * soft.mass_scale - *acc * soft.impulse_scale;
    *acc += imp;
    apply(a, b, ra, rb, imp);
}

/// all three rotations locked: K = Ia + Ib
fn rotation_lock(a: &mut SBody, b: &mut SBody, g: &Geo, acc: &mut Vec3, soft: &Soft) {
    let Some(kinv) = a.inv_i.add(&b.inv_i).inverse() else {
        return;
    };
    let c = capped(rotation_error(g.qa, g.qb), MAX_ANGULAR);
    let cdot = b.w - a.w;
    let imp = -kinv.mul_vec(cdot + c * soft.bias_rate) * soft.mass_scale - *acc * soft.impulse_scale;
    *acc += imp;
    apply_angular(a, b, imp);
}

/// solve the 2 x 2 system [k00 k01; k01 k11] x = r
#[inline]
fn solve2(k00: f64, k01: f64, k11: f64, r: [f64; 2]) -> Option<[f64; 2]> {
    let det = k00 * k11 - k01 * k01;
    if !(det > 0.0) || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    Some([(k11 * r[0] - k01 * r[1]) * inv, (k00 * r[1] - k01 * r[0]) * inv])
}

/// a hinge's two locked rotations: b's X axis stays on a's (the error a.x x b.x on a's Y and Z)
fn hinge_lock(a: &mut SBody, b: &mut SBody, g: &Geo, acc: &mut [f64; 2], soft: &Soft) {
    let k = a.inv_i.add(&b.inv_i);
    let (p1, p2) = (g.ya, g.za);
    let (kp1, kp2) = (k.mul_vec(p1), k.mul_vec(p2));
    let e = capped(g.xa.cross(g.xb), MAX_ANGULAR);
    let dw = b.w - a.w;
    let r = [p1.dot(dw) + soft.bias_rate * p1.dot(e), p2.dot(dw) + soft.bias_rate * p2.dot(e)];
    let Some(x) = solve2(p1.dot(kp1), p1.dot(kp2), p2.dot(kp2), r) else {
        return;
    };
    let imp = [-x[0] * soft.mass_scale - acc[0] * soft.impulse_scale, -x[1] * soft.mass_scale - acc[1] * soft.impulse_scale];
    acc[0] += imp[0];
    acc[1] += imp[1];
    apply_angular(a, b, p1 * imp[0] + p2 * imp[1]);
}

/// a slider's two locked directions: b's anchor stays on a's X axis
fn slider_lock(a: &mut SBody, b: &mut SBody, g: &Geo, acc: &mut [f64; 2], soft: &Soft) {
    let ra = g.ra + g.d;
    let (p1, p2) = (g.ya, g.za);
    let (ca1, ca2, cb1, cb2) = (ra.cross(p1), ra.cross(p2), g.rb.cross(p1), g.rb.cross(p2));
    let m = a.inv_mass + b.inv_mass;
    let k00 = m + ca1.dot(a.inv_i.mul_vec(ca1)) + cb1.dot(b.inv_i.mul_vec(cb1));
    let k01 = ca1.dot(a.inv_i.mul_vec(ca2)) + cb1.dot(b.inv_i.mul_vec(cb2));
    let k11 = m + ca2.dot(a.inv_i.mul_vec(ca2)) + cb2.dot(b.inv_i.mul_vec(cb2));
    let dv = relative_velocity(a, b, ra, g.rb);
    let off = capped(p1 * p1.dot(g.d) + p2 * p2.dot(g.d), MAX_LINEAR);
    let r = [p1.dot(dv) + soft.bias_rate * p1.dot(off), p2.dot(dv) + soft.bias_rate * p2.dot(off)];
    let Some(x) = solve2(k00, k01, k11, r) else {
        return;
    };
    let imp = [-x[0] * soft.mass_scale - acc[0] * soft.impulse_scale, -x[1] * soft.mass_scale - acc[1] * soft.impulse_scale];
    acc[0] += imp[0];
    acc[1] += imp[1];
    apply(a, b, ra, g.rb, p1 * imp[0] + p2 * imp[1]);
}
