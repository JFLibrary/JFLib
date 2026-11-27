// solver -- the velocities of the bodies of one step, soft constraints and the contact constraints
//
// The soft step of Box2D v3 (Catto, "Solver2D", 2024), here in 3D: a step is cut into substeps; in each the
// velocities take gravity and forces, the impulses of the last substep are applied again (warm start), the
// constraints are solved once with a soft bias that pushes overlaps apart (a damped spring of a given
// frequency, so the push never overshoots and never exceeds `max_push`), the positions move, and the
// constraints are solved once more without the bias (relax), so the push leaves no velocity behind.  After
// the substeps restitution is applied once with the approach speeds from before the step.  The separation
// of a contact point follows the bodies through the substeps (the anchors turn with them), so a gap that
// is still open lets the bodies come exactly that close (speculative contacts) and nothing is pushed
// before it touches.  Jacobians keep the anchors of the step's start, as in Box2D.
//
// Friction is a cone: the two tangent impulses together at most friction x the normal impulse.

use crate::manifold::MAX_POINTS;
use crate::math::{Mat3, Quat, Vec3, max, min, sqrt};

pub(crate) const NONE: u32 = u32::MAX;

/// a body as the solver sees it in one step: velocities and the motion so far
#[derive(Clone, Copy, Debug)]
pub(crate) struct SBody {
    pub v: Vec3,
    pub w: Vec3,
    /// how far the centre of mass moved in this step
    pub dp: Vec3,
    /// how far the body turned in this step (world frame, applied before the rotation of the step's start)
    pub dq: Quat,
    pub inv_mass: f64,
    /// the inverse inertia in the world frame of the step's start
    pub inv_i: Mat3,
}

impl SBody {
    pub(crate) const STATIC: SBody = SBody { v: Vec3::ZERO, w: Vec3::ZERO, dp: Vec3::ZERO, dq: Quat::IDENTITY, inv_mass: 0.0, inv_i: Mat3::ZERO };
}

#[inline]
pub(crate) fn load(bodies: &[SBody], i: u32) -> SBody {
    if i == NONE { SBody::STATIC } else { bodies[i as usize] }
}

/// the solver bodies i and j in place, no copy (184 bytes each, eight passes a substep).  The step keeps two
/// static bodies at the end of `bodies` for what does not move in the solver (NONE): nothing changes them, their
/// inverse mass and inertia are 0
#[inline]
pub(crate) fn pair(bodies: &mut [SBody], i: u32, j: u32) -> (&mut SBody, &mut SBody) {
    let n = bodies.len();
    let i = if i == NONE { n - 2 } else { i as usize };
    let j = if j == NONE || j as usize == i { n - 1 } else { j as usize };
    if i < j {
        let (x, y) = bodies.split_at_mut(j);
        (&mut x[i], &mut y[0])
    } else {
        let (x, y) = bodies.split_at_mut(i);
        (&mut y[0], &mut x[j])
    }
}

/// a soft constraint: the bias towards zero error, how much of the effective mass is used, how much of the
/// accumulated impulse is taken back (Catto, "Soft Step")
#[derive(Clone, Copy, Debug)]
pub(crate) struct Soft {
    pub bias_rate: f64,
    pub mass_scale: f64,
    pub impulse_scale: f64,
}

impl Soft {
    pub(crate) const RIGID: Soft = Soft { bias_rate: 0.0, mass_scale: 1.0, impulse_scale: 0.0 };

    /// a spring of `hertz` with damping ratio `zeta`, solved implicitly over a substep of `h` seconds.  A
    /// frequency above half the substep rate is that (a stiffer spring cannot be followed); a damping ratio so
    /// large that the coefficients overflow is a rigid constraint without bias
    pub(crate) fn new(hertz: f64, zeta: f64, h: f64) -> Soft {
        if !(hertz > 0.0) {
            return Soft { bias_rate: 0.0, mass_scale: 1.0, impulse_scale: 0.0 };
        }
        let hertz = if hertz * h > 0.5 { 0.5 / h } else { hertz };
        let omega = 2.0 * core::f64::consts::PI * hertz;
        let a1 = 2.0 * zeta + h * omega;
        let a2 = h * omega * a1;
        if !(a1.is_finite() && a2.is_finite()) {
            return Soft { bias_rate: 0.0, mass_scale: 1.0, impulse_scale: 0.0 };
        }
        let a3 = 1.0 / (1.0 + a2);
        Soft { bias_rate: omega / a1, mass_scale: a2 * a3, impulse_scale: a3 }
    }
}

/// the effective mass of an impulse along `n` at anchors ra, rb: 1 / (J M^-1 J^T), 0 when nothing can move
#[inline]
pub(crate) fn axis_mass(a: &SBody, b: &SBody, ra: Vec3, rb: Vec3, n: Vec3) -> f64 {
    let (ca, cb) = (ra.cross(n), rb.cross(n));
    let k = a.inv_mass + b.inv_mass + ca.dot(a.inv_i.mul_vec(ca)) + cb.dot(b.inv_i.mul_vec(cb));
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

/// apply impulse `p` at anchors ra (to a, negative) and rb (to b)
#[inline]
pub(crate) fn apply(a: &mut SBody, b: &mut SBody, ra: Vec3, rb: Vec3, p: Vec3) {
    a.v -= p * a.inv_mass;
    a.w -= a.inv_i.mul_vec(ra.cross(p));
    b.v += p * b.inv_mass;
    b.w += b.inv_i.mul_vec(rb.cross(p));
}

/// apply angular impulse `l` (to a negative)
#[inline]
pub(crate) fn apply_angular(a: &mut SBody, b: &mut SBody, l: Vec3) {
    a.w -= a.inv_i.mul_vec(l);
    b.w += b.inv_i.mul_vec(l);
}

/// the velocity of b's point rb relative to a's point ra
#[inline]
pub(crate) fn relative_velocity(a: &SBody, b: &SBody, ra: Vec3, rb: Vec3) -> Vec3 {
    b.v + b.w.cross(rb) - a.v - a.w.cross(ra)
}

/// one contact point of a constraint
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CPoint {
    /// the point (world) at the step's start, for events and the public view
    pub point: Vec3,
    /// the anchors relative to the centres of mass at the step's start (world)
    pub ra: Vec3,
    pub rb: Vec3,
    /// the anchor in a's own frame (for matching with the last step's points)
    pub local_a: Vec3,
    /// where the gap is followed: the points of the cores under the contact (a sphere's centre, a point of a
    /// capsule's axis, the surface of the others), relative to the centres of mass (world).  A ball that rolls
    /// does not open a gap by turning its contact point away.
    pub ta: Vec3,
    pub tb: Vec3,
    /// the gap at the step's start, and that minus the tracked points' offset along the normal
    pub separation: f64,
    pub base: f64,
    pub normal_impulse: f64,
    pub tangent_impulse: [f64; 2],
    pub max_normal_impulse: f64,
    /// the smallest gap the passes saw (restitution only where the contact closed)
    pub min_separation: f64,
    pub normal_mass: f64,
    pub tangent_mass: [f64; 2],
    /// the normal velocity at the step's start (negative: approaching), for restitution and events
    pub approach: f64,
}

/// the constraint of a manifold: up to four points along one normal
#[derive(Clone, Copy, Debug)]
pub(crate) struct Constraint {
    /// the slots of the bodies (a towards b along the normal) and their solver indices
    pub a: u32,
    pub b: u32,
    pub sa: u32,
    pub sb: u32,
    pub key: u32,
    pub normal: Vec3,
    pub tangent: [Vec3; 2],
    pub friction: f64,
    pub restitution: f64,
    pub count: usize,
    pub points: [CPoint; MAX_POINTS],
    /// against something that does not move in the solver (static, kinematic, asleep): the stiffer softness
    pub rigid_other: bool,
    /// the largest rolling torque impulse per unit of normal impulse (m; 0: none)
    pub rolling: f64,
    pub rolling_impulse: Vec3,
    pub rolling_mass: Mat3,
    /// against such a body, an overlap deeper than this is held rigidly (a quarter of the moving body's
    /// smallest size): a light body squeezed by a heavy one onto the ground is not pushed through it
    pub deep: f64,
}

/// the tangent plane of a normal, chosen the same way on every target
pub(crate) fn tangents(n: Vec3) -> [Vec3; 2] {
    let t1 = n.any_perpendicular();
    [t1, n.cross(t1)]
}

/// the masses and approach speeds of a constraint at the step's start
pub(crate) fn prepare(c: &mut Constraint, bodies: &[SBody]) {
    let (a, b) = (&load(bodies, c.sa), &load(bodies, c.sb));
    for p in &mut c.points[..c.count] {
        p.base = p.separation - (p.tb - p.ta).dot(c.normal);
        p.normal_mass = axis_mass(a, b, p.ra, p.rb, c.normal);
        p.tangent_mass = [axis_mass(a, b, p.ra, p.rb, c.tangent[0]), axis_mass(a, b, p.ra, p.rb, c.tangent[1])];
        p.approach = relative_velocity(a, b, p.ra, p.rb).dot(c.normal);
        p.max_normal_impulse = 0.0;
        p.min_separation = p.separation;
    }
    c.rolling_mass = if c.rolling > 0.0 { a.inv_i.add(&b.inv_i).inverse().unwrap_or(Mat3::ZERO) } else { Mat3::ZERO };
}

pub(crate) fn warm_start(c: &Constraint, bodies: &mut [SBody]) {
    let (a, b) = pair(bodies, c.sa, c.sb);
    for p in &c.points[..c.count] {
        let imp = c.normal * p.normal_impulse + c.tangent[0] * p.tangent_impulse[0] + c.tangent[1] * p.tangent_impulse[1];
        apply(a, b, p.ra, p.rb, imp);
    }
    if c.rolling > 0.0 {
        crate::solver::apply_angular(a, b, c.rolling_impulse);
    }
}

/// one pass over a constraint: the normal impulses (with the soft bias when `use_bias`), then friction
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve(c: &mut Constraint, bodies: &mut [SBody], soft: &Soft, inv_h: f64, max_push: f64, use_bias: bool) {
    let (a, b) = pair(bodies, c.sa, c.sb);
    let n = c.normal;
    let dp = b.dp - a.dp;
    for p in &mut c.points[..c.count] {
        // the gap now: the cores' points moved and turned with their bodies
        let pa = a.dq.rotate(p.ta);
        let pb = b.dq.rotate(p.tb);
        let s = (dp + (pb - pa)).dot(n) + p.base;
        p.min_separation = min(p.min_separation, s);
        let (bias, mass_scale, impulse_scale) = if s > 0.0 {
            (s * inv_h, 1.0, 0.0) // speculative: come exactly this close in this substep
        } else if c.deep > 0.0 && s < -c.deep {
            // too deep for a soft contact: rigid, and a fifth of the excess out per substep
            (if use_bias { max(0.2 * (s + c.deep) * inv_h, -max_push) } else { 0.0 }, 1.0, 0.0)
        } else if use_bias {
            (max(soft.bias_rate * s, -max_push), soft.mass_scale, soft.impulse_scale)
        } else {
            (0.0, 1.0, 0.0)
        };
        let vn = relative_velocity(a, b, p.ra, p.rb).dot(n);
        let imp = -p.normal_mass * mass_scale * (vn + bias) - impulse_scale * p.normal_impulse;
        let total = max(p.normal_impulse + imp, 0.0);
        let imp = total - p.normal_impulse;
        p.normal_impulse = total;
        p.max_normal_impulse = max(p.max_normal_impulse, imp);
        apply(a, b, p.ra, p.rb, n * imp);
    }
    for p in &mut c.points[..c.count] {
        let dv = relative_velocity(a, b, p.ra, p.rb);
        let t = c.tangent;
        let mut l0 = p.tangent_impulse[0] - p.tangent_mass[0] * dv.dot(t[0]);
        let mut l1 = p.tangent_impulse[1] - p.tangent_mass[1] * dv.dot(t[1]);
        let limit = c.friction * p.normal_impulse;
        let ll = l0 * l0 + l1 * l1;
        if ll > limit * limit {
            let k = if ll > 0.0 { limit / sqrt(ll) } else { 0.0 };
            l0 *= k;
            l1 *= k;
        }
        let d0 = l0 - p.tangent_impulse[0];
        let d1 = l1 - p.tangent_impulse[1];
        p.tangent_impulse = [l0, l1];
        apply(a, b, p.ra, p.rb, t[0] * d0 + t[1] * d1);
    }
    if c.rolling > 0.0 {
        // rolling resistance: the relative spin is braked with at most rolling x the normal impulse
        let total: f64 = c.points[..c.count].iter().map(|p| p.normal_impulse).sum();
        let limit = c.rolling * total;
        let old = c.rolling_impulse;
        let mut l = old - c.rolling_mass.mul_vec(b.w - a.w);
        let ll = l.length_sq();
        if ll > limit * limit {
            l = if ll > 0.0 { l * (limit / sqrt(ll)) } else { Vec3::ZERO };
        }
        c.rolling_impulse = l;
        apply_angular(a, b, l - old);
    }
}

/// restitution once after the substeps: points that approached faster than `threshold`, were pushed and came
/// within `slop` of touching (a speculative point that something else stopped short of the surface does not
/// bounce)
pub(crate) fn restitution(c: &mut Constraint, bodies: &mut [SBody], threshold: f64, slop: f64) {
    if !(c.restitution > 0.0) {
        return;
    }
    let (a, b) = pair(bodies, c.sa, c.sb);
    for p in &mut c.points[..c.count] {
        if p.approach > -threshold || !(p.max_normal_impulse > 0.0) || p.min_separation > slop {
            continue;
        }
        let vn = relative_velocity(a, b, p.ra, p.rb).dot(c.normal);
        let imp = -p.normal_mass * (vn + c.restitution * p.approach);
        let total = max(p.normal_impulse + imp, 0.0);
        let imp = total - p.normal_impulse;
        p.normal_impulse = total;
        p.max_normal_impulse = max(p.max_normal_impulse, imp);
        apply(a, b, p.ra, p.rb, c.normal * imp);
    }
}
