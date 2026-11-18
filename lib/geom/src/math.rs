// math -- the vectors and affine maps of the crate, in f64, with operations that give the same bits on every
// target: + - * / and a correctly rounded square root, never a fused multiply-add (Rust does not contract
// a * b + c on its own); sin, cos and atan2 from rt's portable math (docs/ARM64.md, ARM2), the same code on
// every target.  Angles of the modelling API are degrees: a multiple of 90 degrees turns exactly (sin and cos
// are 0 and +-1 there), so a box turned by 90 degrees keeps exact corners and copies of a radial array in
// four meet without a gap.  The same scheme as lib/physics' math (docs/PHYSICS.md, "Bitgleich").

use core::ops::{Add, AddAssign, Div, Index, Mul, Neg, Sub, SubAssign};

// ---------------------------------------------------------------- scalar functions

/// The square root, correctly rounded (IEEE 754), so the same bits on every target, NaN included:
/// `rt::fx::sqrtd`, the processor's instruction where rt has it (`sqrtsd` on x86-64, `fsqrt` on ARM64),
/// elsewhere (the browser, the x86-64 Android emulator) [`sqrt_portable`].  A negative argument gives NaN,
/// -0.0 gives -0.0.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    rt::fx::sqrtd(x)
}

/// The correctly rounded square root from f64 operations and one 64 x 64 bit product (`rt::fx::sqrt_portable`,
/// tested against the processor in apps/physicstest): Newton's method gives an estimate within a few units
/// in the last place, the integer test (R - 1/2)^2 < X <= (R + 1/2)^2 on the significand picks the right
/// neighbour.  Equal to `sqrtsd` for every input, NaN included.  Public for the tests.
#[inline]
pub fn sqrt_portable(x: f64) -> f64 {
    rt::fx::sqrt_portable(x)
}

// rt::x87 is the x87 unit on x86-64 and portable/math.rs elsewhere; rt::portable::math is portable/math.rs
// on every target but the browser, where rt::x87 already is it
#[cfg(not(target_arch = "wasm32"))]
use rt::portable::math as pmath;
#[cfg(target_arch = "wasm32")]
use rt::x87 as pmath;

/// sin of `x` (radians), the same bits on every target (within half an ulp)
#[inline]
pub fn sin(x: f64) -> f64 {
    pmath::sin(x)
}

/// cos of `x` (radians), the same bits on every target
#[inline]
pub fn cos(x: f64) -> f64 {
    pmath::cos(x)
}

/// atan2(y, x) in radians, the same bits on every target
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    pmath::atan2(y, x)
}

/// pi / 180
const DEG: f64 = core::f64::consts::PI / 180.0;

/// The largest angle in degrees [`sin_cos_deg`] takes (beyond it NaN): far more than any modelling needs, and
/// small enough that the reduction stays exact.
pub const MAX_DEGREES: f64 = 1e12;

/// (sin, cos) of an angle in degrees, the same bits on every target and exact at every multiple of 90
/// degrees (0 and +-1): the angle is reduced exactly to the nearest multiple of 90 and a rest of at most 45
/// degrees, whose sine and cosine come from rt's portable math.  NaN for a NaN angle or one beyond
/// [`MAX_DEGREES`].
pub fn sin_cos_deg(deg: f64) -> (f64, f64) {
    if !(abs(deg) <= MAX_DEGREES) {
        return (f64::NAN, f64::NAN);
    }
    let q = floor(deg / 90.0 + 0.5); // the nearest multiple of 90, exact as an f64
    // exact: q * 90 is (|q| < 2^40), and deg lies within a factor of 2 of it when q != 0 (Sterbenz)
    let rest = deg - q * 90.0;
    let (s, c) = if rest == 0.0 {
        (0.0, 1.0)
    } else {
        let (c, s) = pmath::sincos(rest * DEG); // rt's order is the x87 unit's: cosine first
        (s, c)
    };
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// the largest integer not above `x` for |x| < 2^62 (the crate's reductions; larger ones as they are), the
/// same on every target
pub fn floor(x: f64) -> f64 {
    if !(abs(x) < 4.6e18) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// |x| (the sign bit cleared)
#[inline]
pub fn abs(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & !(1u64 << 63))
}

/// the angle between two directions in degrees, 0 ..= 180 (atan2 of the cross product's length and the dot
/// product: accurate at 0 and 180 too); 0 when one of them is zero
pub fn angle_deg(a: Vec3, b: Vec3) -> f64 {
    atan2(a.cross(b).length(), a.dot(b)) / DEG
}

/// the smaller of two numbers (`b` when they are equal or one is NaN)
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

/// the larger of two numbers (`b` when they are equal or one is NaN)
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// `x` with -0.0 turned into +0.0 (the same bits for the same point; welding and hashing compare bits)
#[inline]
pub fn unsign_zero(x: f64) -> f64 {
    x + 0.0
}

// ---------------------------------------------------------------- Vec3

/// A point or a direction in metres, `f64`; right-handed, Y up (render3d's and lib/physics' convention).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    /// metres (or a direction's component)
    pub x: f64,
    /// metres, up
    pub y: f64,
    /// metres, towards the viewer of the front view
    pub z: f64,
}

impl Vec3 {
    /// (0, 0, 0)
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    /// the unit vector along X
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    /// the unit vector along Y (up)
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    /// the unit vector along Z (towards the front view's camera)
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    /// (x, y, z)
    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// the dot product, summed x, y, z in this order
    #[inline]
    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// the cross product (right-handed)
    #[inline]
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    /// the squared length
    #[inline]
    pub fn length_sq(self) -> f64 {
        self.dot(self)
    }

    /// the length (correctly rounded root)
    #[inline]
    pub fn length(self) -> f64 {
        sqrt(self.dot(self))
    }

    /// the unit vector in this direction, None for a zero (or not finite) vector
    #[inline]
    pub fn try_normalize(self) -> Option<Vec3> {
        let l = self.length();
        if l > 0.0 && l < f64::INFINITY { Some(self / l) } else { None }
    }

    /// the unit vector in this direction; a zero vector stays zero
    #[inline]
    pub fn normalize_or_zero(self) -> Vec3 {
        self.try_normalize().unwrap_or(Vec3::ZERO)
    }

    /// the smaller coordinates of both
    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(min(self.x, o.x), min(self.y, o.y), min(self.z, o.z))
    }

    /// the larger coordinates of both
    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(max(self.x, o.x), max(self.y, o.y), max(self.z, o.z))
    }

    /// whether every coordinate is finite
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// the same point with -0.0 coordinates as +0.0
    #[inline]
    pub fn unsigned_zeros(self) -> Vec3 {
        Vec3::new(unsign_zero(self.x), unsign_zero(self.y), unsign_zero(self.z))
    }

    /// the bits of the coordinates with -0.0 as +0.0: equal for equal points (the key of welding)
    #[inline]
    pub fn key(self) -> [u64; 3] {
        let v = self.unsigned_zeros();
        [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
    }

    /// the coordinates as f32 (render3d, glTF)
    #[inline]
    pub fn to_f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }

    /// the coordinate along an axis
    #[inline]
    pub fn get(self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }
}

impl From<[f64; 3]> for Vec3 {
    fn from(a: [f64; 3]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
}

impl From<Vec3> for [f64; 3] {
    fn from(v: Vec3) -> [f64; 3] {
        [v.x, v.y, v.z]
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

impl Index<usize> for Vec3 {
    type Output = f64;
    /// 0: x, 1: y, 2: z
    fn index(&self, i: usize) -> &f64 {
        match i {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }
}

// ---------------------------------------------------------------- axes, boxes

/// one of the three axes (mirror planes, rotations, the dominant axis of a normal)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// left to right
    X,
    /// up
    Y,
    /// back to front
    Z,
}

/// An axis-aligned box in metres (the bounds of a mesh).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// the smallest coordinates
    pub min: Vec3,
    /// the largest coordinates
    pub max: Vec3,
}

impl Aabb {
    /// the box around one point
    pub fn point(p: Vec3) -> Aabb {
        Aabb { min: p, max: p }
    }

    /// the box grown to hold `p`
    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    /// the box around both
    pub fn union(self, o: Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    /// width, height, depth (max - min)
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// the middle
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

// ---------------------------------------------------------------- affine maps

/// An affine map p -> m p + t in f64: rotations, scales, mirrors and moves of the modelling operations.
/// `m` holds rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// the linear part, rows
    pub m: [[f64; 3]; 3],
    /// the move after it
    pub t: Vec3,
}

impl Affine {
    /// the map that changes nothing
    pub const IDENTITY: Affine = Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: Vec3::ZERO };

    /// a move by `t`
    pub fn translation(t: Vec3) -> Affine {
        Affine { t, ..Affine::IDENTITY }
    }

    /// a scale along the axes (a negative factor mirrors)
    pub fn scale(s: Vec3) -> Affine {
        Affine { m: [[s.x, 0.0, 0.0], [0.0, s.y, 0.0], [0.0, 0.0, s.z]], t: Vec3::ZERO }
    }

    /// the mirror at the plane through the origin across `axis` (x -> -x for Axis::X)
    pub fn mirror(axis: Axis) -> Affine {
        match axis {
            Axis::X => Affine::scale(Vec3::new(-1.0, 1.0, 1.0)),
            Axis::Y => Affine::scale(Vec3::new(1.0, -1.0, 1.0)),
            Axis::Z => Affine::scale(Vec3::new(1.0, 1.0, -1.0)),
        }
    }

    /// a rotation by `deg` degrees about an axis through the origin, counter-clockwise when the axis points
    /// at the viewer (right-handed); exact at multiples of 90 degrees
    pub fn rotation(axis: Axis, deg: f64) -> Affine {
        let (s, c) = sin_cos_deg(deg);
        let m = match axis {
            Axis::X => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
            Axis::Y => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
            Axis::Z => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        };
        Affine { m, t: Vec3::ZERO }
    }

    /// a rotation by `deg` degrees about the direction `axis` (any length) through the origin, right-handed
    /// (Rodrigues); None for a zero axis
    pub fn rotation_about(axis: Vec3, deg: f64) -> Option<Affine> {
        let a = axis.try_normalize()?;
        let (s, c) = sin_cos_deg(deg);
        let k = 1.0 - c;
        let m = [
            [c + a.x * a.x * k, a.x * a.y * k - a.z * s, a.x * a.z * k + a.y * s],
            [a.y * a.x * k + a.z * s, c + a.y * a.y * k, a.y * a.z * k - a.x * s],
            [a.z * a.x * k - a.y * s, a.z * a.y * k + a.x * s, c + a.z * a.z * k],
        ];
        Some(Affine { m, t: Vec3::ZERO })
    }

    /// `self` after `first`: p -> self(first(p))
    pub fn then_after(&self, first: &Affine) -> Affine {
        let a = &self.m;
        let b = &first.m;
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        Affine { m, t: self.apply_vector(first.t) + self.t }
    }

    /// m p + t, with -0.0 coordinates as +0.0
    #[inline]
    pub fn apply(&self, p: Vec3) -> Vec3 {
        (self.apply_vector(p) + self.t).unsigned_zeros()
    }

    /// m v (a direction: without the move)
    #[inline]
    pub fn apply_vector(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }

    /// the determinant of the linear part (negative: the map mirrors, faces must turn)
    pub fn determinant(&self) -> f64 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// the map of normals: the inverse transposed linear part up to a positive factor (the cofactor matrix,
    /// times the determinant's sign), so that a normal maps to a normal pointing the same way out
    pub fn normal_matrix(&self) -> [[f64; 3]; 3] {
        let m = &self.m;
        let c = [
            [m[1][1] * m[2][2] - m[1][2] * m[2][1], m[1][2] * m[2][0] - m[1][0] * m[2][2], m[1][0] * m[2][1] - m[1][1] * m[2][0]],
            [m[0][2] * m[2][1] - m[0][1] * m[2][2], m[0][0] * m[2][2] - m[0][2] * m[2][0], m[0][1] * m[2][0] - m[0][0] * m[2][1]],
            [m[0][1] * m[1][2] - m[0][2] * m[1][1], m[0][2] * m[1][0] - m[0][0] * m[1][2], m[0][0] * m[1][1] - m[0][1] * m[1][0]],
        ];
        let sign = if self.determinant() < 0.0 { -1.0 } else { 1.0 };
        let mut out = c;
        for row in out.iter_mut() {
            for v in row.iter_mut() {
                *v *= sign;
            }
        }
        out
    }

    /// whether every number is finite
    pub fn is_finite(&self) -> bool {
        self.m.iter().all(|r| r.iter().all(|v| v.is_finite())) && self.t.is_finite()
    }

    /// whether the linear part keeps angles: a rotation, a mirror or a uniform scale (m^T m = s^2 I to 1e-12),
    /// so normals can simply be turned with it
    pub fn is_similarity(&self) -> bool {
        let m = &self.m;
        let mut c = [[0.0; 3]; 3];
        for (i, row) in c.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = m[0][i] * m[0][j] + m[1][i] * m[1][j] + m[2][i] * m[2][j];
            }
        }
        let s2 = (c[0][0] + c[1][1] + c[2][2]) / 3.0;
        if !(s2 > 0.0) {
            return false;
        }
        (0..3).all(|i| (0..3).all(|j| abs(c[i][j] - if i == j { s2 } else { 0.0 }) <= 1e-12 * s2))
    }
}

/// a 3 x 3 matrix (rows) times a vector
#[inline]
pub(crate) fn mat_vec(m: &[[f64; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}
