// mass -- the mass, the centre of mass and the inertia tensor of a shape of uniform density
//
// Spheres, capsules and boxes in closed form; a convex hull from the tetrahedra between a point inside it
// and its triangles (the second moment of a tetrahedron with one corner at the origin is
// det / 120 (a a^T + b b^T + c c^T + s s^T), s = a + b + c), moved to the centre of mass with the
// parallel axis theorem.  A mesh has none: it is static.

use crate::hull::ConvexHull;
use crate::math::{abs, max, pow2, unit_exp, Iso, Mat3, Vec3};
use crate::shape::Shape;

const PI: f64 = core::f64::consts::PI;

/// The mass of a body: how much (kg), where its centre is (in the shape's frame) and how it resists turning
/// about that centre (the inertia tensor, kg m², in the shape's frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProps {
    /// kg
    pub mass: f64,
    /// the centre of mass in the shape's frame
    pub center: Vec3,
    /// the inertia tensor about the centre of mass, in the shape's frame
    pub inertia: Mat3,
}

impl MassProps {
    /// a mass `mass` at `center` with this inertia tensor
    pub const fn new(mass: f64, center: Vec3, inertia: Mat3) -> MassProps {
        MassProps { mass, center, inertia }
    }

    /// a point mass at the origin that does not turn (an infinite inertia is no number: the tensor is 0 and a
    /// body reads 0 as "does not turn")
    pub const fn point(mass: f64) -> MassProps {
        MassProps { mass, center: Vec3::ZERO, inertia: Mat3::ZERO }
    }

    /// the same body seen in the frame `iso` places it in (the centre moved, the tensor turned)
    pub fn transformed(&self, iso: &Iso) -> MassProps {
        MassProps { mass: self.mass, center: iso.transform_point(self.center), inertia: self.inertia.rotated(iso.rot) }
    }

    /// the inertia tensor about another point (the parallel axis theorem), in the same frame
    pub fn inertia_about(&self, p: Vec3) -> Mat3 {
        let d = p - self.center;
        self.inertia.add(&Mat3::IDENTITY.scale(d.length_sq()).sub(&Mat3::outer(d, d)).scale(self.mass))
    }

    /// the mass and tensor scaled by `factor` (another density)
    pub fn scaled(&self, factor: f64) -> MassProps {
        MassProps { mass: self.mass * factor, center: self.center, inertia: self.inertia.scale(factor) }
    }

    /// mass, centre and tensor finite, the mass positive, the tensor symmetric with principal moments that are
    /// non-negative and keep the triangle inequality, in its own axes: what a solid body can have, of any size.
    /// Each principal moment is below the sum of the other two plus 1e-9 times the sum of all three (so the
    /// moments of a rod or a plate, which meet the inequality exactly, pass in any frame); the zero tensor of
    /// [`MassProps::point`] is valid.
    pub fn is_valid(&self) -> bool {
        if !(self.mass > 0.0 && self.mass.is_finite() && self.center.is_finite() && self.inertia.is_finite()) {
            return false;
        }
        // brought to one size by a power of two (exact): the products below neither over- nor underflow
        let big = self.inertia.cols.iter().fold(0.0, |m, v| max(m, v.abs().max_elem()));
        let Some(k) = unit_exp(big) else {
            return true; // the zero tensor of a point
        };
        let i = self.inertia.scale(pow2(k));
        let t = i.trace();
        let tol = abs(t) * 1e-9;
        if !(abs(i.at(0, 1) - i.at(1, 0)) <= tol && abs(i.at(0, 2) - i.at(2, 0)) <= tol && abs(i.at(1, 2) - i.at(2, 1)) <= tol) {
            return false;
        }
        // a solid body's tensor is tr(S) 1 - S for the second moment S of its mass, which is positive
        // semidefinite: S = tr(I)/2 1 - I has the eigenvalues (I2 + I3 - I1) / 2 ... of the principal moments,
        // in any frame.  I1 < I2 + I3 + tol is (I2 + I3 - I1) / 2 > -tol / 2: S + tol/2 1 positive definite,
        // which the pivots of its LDL^T factors tell (each positive; their rounding is about 1e-16 of the sum,
        // far below tol / 2, also for a rod or a plate, whose S has an eigenvalue 0).
        let s = |r: usize, c: usize| -(i.at(r, c) + i.at(c, r)) * 0.5;
        let dg = t * 0.5 + tol * 0.5;
        let (s00, s11, s22) = (dg - i.at(0, 0), dg - i.at(1, 1), dg - i.at(2, 2));
        let (s01, s02, s12) = (s(0, 1), s(0, 2), s(1, 2));
        if !(s00 > 0.0) {
            return false;
        }
        let (l10, l20) = (s01 / s00, s02 / s00);
        let d1 = s11 - l10 * s01;
        if !(d1 > 0.0) {
            return false;
        }
        let e21 = s12 - l20 * s01;
        s22 - l20 * s02 - e21 * e21 / d1 > 0.0
    }
}

impl Shape {
    /// the mass properties of the shape filled with `density` (kg/m³), None for a mesh (never dynamic), for a
    /// density that is not positive and finite and for sizes that are not (a radius, a box's half sizes; a
    /// capsule's half height may be 0)
    pub fn mass_properties(&self, density: f64) -> Option<MassProps> {
        if !(density > 0.0 && density.is_finite()) {
            return None;
        }
        // the sizes: positive and finite (a capsule's half height may be 0, a ball)
        let ok = |x: f64| x > 0.0 && x.is_finite();
        let sizes = match self {
            Shape::Sphere { radius } => ok(*radius),
            Shape::Capsule { half_height, radius } => ok(*radius) && *half_height >= 0.0 && half_height.is_finite(),
            Shape::Box { half } => ok(half.x) && ok(half.y) && ok(half.z),
            _ => true,
        };
        if !sizes {
            return None;
        }
        let m = match self {
            Shape::Sphere { radius } => {
                let r = *radius;
                let mass = density * (4.0 / 3.0) * PI * r * r * r;
                MassProps::new(mass, Vec3::ZERO, Mat3::diagonal(Vec3::splat(0.4 * mass * r * r)))
            }
            Shape::Capsule { half_height, radius } => {
                let (h, r) = (*half_height, *radius);
                // the cylinder and the two half balls (together a ball), each half ball's centre of mass
                // 3r/8 beyond the end of the cylinder
                let mc = density * PI * r * r * 2.0 * h;
                let ms = density * (4.0 / 3.0) * PI * r * r * r;
                let iy = mc * r * r * 0.5 + ms * r * r * 0.4;
                let ix = mc * (h * h / 3.0 + r * r * 0.25) + ms * (r * r * 0.4 + h * h + 0.75 * h * r);
                MassProps::new(mc + ms, Vec3::ZERO, Mat3::diagonal(Vec3::new(ix, iy, ix)))
            }
            Shape::Box { half } => {
                let mass = density * 8.0 * half.x * half.y * half.z;
                let (x2, y2, z2) = (half.x * half.x, half.y * half.y, half.z * half.z);
                MassProps::new(mass, Vec3::ZERO, Mat3::diagonal(Vec3::new(y2 + z2, x2 + z2, x2 + y2) * (mass / 3.0)))
            }
            Shape::Hull(h) => hull_mass(h, density)?,
            Shape::Mesh(_) => return None,
        };
        if m.is_valid() { Some(m) } else { None }
    }
}

fn hull_mass(h: &ConvexHull, density: f64) -> Option<MassProps> {
    let o = h.aabb().center();
    let pts = h.points();
    let mut six_v = 0.0; // six times the volume
    let mut first = Vec3::ZERO; // 24 times the first moment
    let mut second = Mat3::ZERO; // 120 times the second moment
    for f in h.faces() {
        let (a, b, c) = (pts[f[0] as usize] - o, pts[f[1] as usize] - o, pts[f[2] as usize] - o);
        let det = a.dot(b.cross(c));
        six_v += det;
        let s = a + b + c;
        first += s * det;
        let m = Mat3::outer(a, a).add(&Mat3::outer(b, b)).add(&Mat3::outer(c, c)).add(&Mat3::outer(s, s));
        second = second.add(&m.scale(det));
    }
    if !(six_v > 0.0) {
        return None;
    }
    let volume = six_v / 6.0;
    let mass = density * volume;
    let c = first / (4.0 * six_v); // (first / 24) / volume
    // the second moment about o, then about the centre of mass
    let cov = second.scale(density / 120.0).sub(&Mat3::outer(c, c).scale(mass));
    let inertia = Mat3::IDENTITY.scale(cov.trace()).sub(&cov);
    // exactly symmetric (the sums above are, up to rounding)
    let sym = |i: usize, j: usize| (inertia.at(i, j) + inertia.at(j, i)) * 0.5;
    let (xy, xz, yz) = (sym(0, 1), sym(0, 2), sym(1, 2));
    let inertia = Mat3::from_cols(
        Vec3::new(inertia.at(0, 0), xy, xz),
        Vec3::new(xy, inertia.at(1, 1), yz),
        Vec3::new(xz, yz, inertia.at(2, 2)),
    );
    Some(MassProps::new(mass, c + o, inertia))
}
