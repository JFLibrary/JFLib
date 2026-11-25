// math -- the vectors, rotations and boxes of the crate, in f64, with operations that give the same bits on
// every target: + - * / and a correctly rounded square root, never a fused multiply-add (Rust does not
// contract a * b + c on its own); sin, cos and atan2 from rt's portable math (docs/ARM64.md, ARM2), which is
// the same code on every target.

use core::ops::{Add, AddAssign, Div, Index, Mul, MulAssign, Neg, Sub, SubAssign};

// ---------------------------------------------------------------- scalar functions

/// The square root, correctly rounded (IEEE 754), so the same bits on every target, NaN included:
/// `rt::fx::sqrtd`, the processor's instruction where rt has it (`sqrtsd` on x86-64, `fsqrt` on ARM64),
/// elsewhere (the browser, the x86-64 Android emulator) [`sqrt_portable`].  A negative argument gives NaN,
/// -0.0 gives -0.0.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    rt::fx::sqrtd(x)
}

/// The correctly rounded square root from f64 operations and one 64 x 64 bit product (`rt::fx::sqrt_portable`):
/// Newton's method gives an estimate within a few units in the last place, the integer test (R - 1/2)^2 < X
/// <= (R + 1/2)^2 on the significand picks the right neighbour.  Equal to the processor's square root
/// (`sqrtsd`) for every input, NaN included.  Public for the tests.
#[inline]
pub fn sqrt_portable(x: f64) -> f64 {
    rt::fx::sqrt_portable(x)
}

/// sin and cos of `x` (radians) from rt's portable math: the same bits on every target (within half an
/// ulp of the exact value)
#[inline]
pub fn sincos(x: f64) -> (f64, f64) {
    let (c, s) = pmath::sincos(x); // rt's order is the x87 unit's: cosine first
    (s, c)
}

/// sin of `x` (radians), the same bits on every target
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

// rt::x87 is the x87 unit on x86-64 and portable/math.rs elsewhere; rt::portable::math is portable/math.rs
// on every target but the browser, where rt::x87 already is it
#[cfg(not(target_arch = "wasm32"))]
use rt::portable::math as pmath;
#[cfg(target_arch = "wasm32")]
use rt::x87 as pmath;

/// the smaller of two numbers (`b` when they are equal or one is NaN), the same on every target
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

/// the larger of two numbers (`b` when they are equal or one is NaN)
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// `x` limited to [lo, hi] (lo when x is NaN)
#[inline]
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x > hi {
        hi
    } else if x >= lo {
        x
    } else {
        lo
    }
}

#[inline]
pub(crate) fn abs(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & !(1u64 << 63))
}

// ---------------------------------------------------------------- Vec3

/// A point or a direction in metres, `f64` (docs/PHYSICS.md, "f64").  Converts from and to `[f64; 3]`,
/// render3d's positions.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    /// metres (or a direction's component)
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    /// (0, 0, 0)
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    /// the unit vectors along the axes (Y is up)
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    /// (x, y, z)
    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// (v, v, v)
    #[inline]
    pub const fn splat(v: f64) -> Vec3 {
        Vec3 { x: v, y: v, z: v }
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

    /// the smaller component by component
    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(min(self.x, o.x), min(self.y, o.y), min(self.z, o.z))
    }

    /// the larger component by component
    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(max(self.x, o.x), max(self.y, o.y), max(self.z, o.z))
    }

    /// the absolute value of each component
    #[inline]
    pub fn abs(self) -> Vec3 {
        Vec3::new(abs(self.x), abs(self.y), abs(self.z))
    }

    /// component by component
    #[inline]
    pub fn mul_elem(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }

    /// a + (b - a) * t
    #[inline]
    pub fn lerp(self, b: Vec3, t: f64) -> Vec3 {
        self + (b - self) * t
    }

    /// the largest component
    #[inline]
    pub fn max_elem(self) -> f64 {
        max(self.x, max(self.y, self.z))
    }

    /// every component is finite
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// [x, y, z] (render3d's positions)
    #[inline]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// a unit vector perpendicular to this unit vector (chosen the same way on every target)
    pub fn any_perpendicular(self) -> Vec3 {
        let a = self.abs();
        let o = if a.x <= a.y && a.x <= a.z {
            Vec3::X
        } else if a.y <= a.z {
            Vec3::Y
        } else {
            Vec3::Z
        };
        self.cross(o).normalize_or_zero()
    }
}

impl From<[f64; 3]> for Vec3 {
    #[inline]
    fn from(a: [f64; 3]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
}

impl From<[f32; 3]> for Vec3 {
    #[inline]
    fn from(a: [f32; 3]) -> Vec3 {
        Vec3::new(a[0] as f64, a[1] as f64, a[2] as f64)
    }
}

impl From<Vec3> for [f64; 3] {
    #[inline]
    fn from(v: Vec3) -> [f64; 3] {
        v.to_array()
    }
}

impl Index<usize> for Vec3 {
    type Output = f64;
    #[inline]
    fn index(&self, i: usize) -> &f64 {
        match i {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Mul<Vec3> for f64 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(self * v.x, self * v.y, self * v.z)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, k: f64) -> Vec3 {
        Vec3::new(self.x / k, self.y / k, self.z / k)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

impl MulAssign<f64> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, k: f64) {
        *self = *self * k;
    }
}

// ---------------------------------------------------------------- Quat

/// A rotation as a unit quaternion (x, y, z, w), `f64`.  render3d keeps rotations as `[f32; 4]` in the same
/// order: [`Quat::from_f32`], [`Quat::to_f32`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    /// the vector part (sin of half the angle times the axis)
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// the scalar part (cos of half the angle)
    pub w: f64,
}

impl Default for Quat {
    fn default() -> Quat {
        Quat::IDENTITY
    }
}

impl Quat {
    /// no rotation
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    /// as given (normalise it with [`Quat::normalize`] unless it is a unit quaternion already)
    #[inline]
    pub const fn new(x: f64, y: f64, z: f64, w: f64) -> Quat {
        Quat { x, y, z, w }
    }

    /// the rotation by `angle` radians about the unit vector `axis` (right-handed)
    pub fn from_axis_angle(axis: Vec3, angle: f64) -> Quat {
        let (s, c) = sincos(angle * 0.5);
        Quat::new(axis.x * s, axis.y * s, axis.z * s, c)
    }

    /// the rotation about +Y by `yaw`, then about the rotated +X by `pitch` (a camera's or a figure's turn)
    pub fn from_yaw_pitch(yaw: f64, pitch: f64) -> Quat {
        Quat::from_axis_angle(Vec3::Y, yaw).mul(Quat::from_axis_angle(Vec3::X, pitch))
    }

    /// the shortest rotation that turns the unit vector `from` into the unit vector `to`
    pub fn from_rotation_arc(from: Vec3, to: Vec3) -> Quat {
        let d = from.dot(to);
        if d < -0.999999999999 {
            let axis = from.any_perpendicular();
            return Quat::new(axis.x, axis.y, axis.z, 0.0);
        }
        let c = from.cross(to);
        Quat::new(c.x, c.y, c.z, 1.0 + d).normalize()
    }

    /// from render3d's `[f32; 4]` (x, y, z, w)
    #[inline]
    pub fn from_f32(q: [f32; 4]) -> Quat {
        Quat::new(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64)
    }

    /// to render3d's `[f32; 4]` (x, y, z, w)
    #[inline]
    pub fn to_f32(self) -> [f32; 4] {
        [self.x as f32, self.y as f32, self.z as f32, self.w as f32]
    }

    /// the unit quaternion in this direction (the identity for a zero quaternion)
    pub fn normalize(self) -> Quat {
        let l = sqrt(self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w);
        if l > 0.0 && l < f64::INFINITY {
            Quat::new(self.x / l, self.y / l, self.z / l, self.w / l)
        } else {
            Quat::IDENTITY
        }
    }

    /// the inverse rotation of a unit quaternion
    #[inline]
    pub fn conjugate(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, self.w)
    }

    /// the product self * o, which rotates by `o` first and then by `self`
    #[allow(clippy::should_implement_trait)] // mul is the quaternion product; Mul<Vec3> would read as a scale
    pub fn mul(self, o: Quat) -> Quat {
        Quat::new(
            self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        )
    }

    /// `v` rotated (a unit quaternion)
    #[inline]
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }

    /// `v` rotated back (by the conjugate)
    #[inline]
    pub fn inverse_rotate(self, v: Vec3) -> Vec3 {
        self.conjugate().rotate(v)
    }

    /// the rotation as a matrix
    pub fn to_mat3(self) -> Mat3 {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, yy, zz) = (x * x2, y * y2, z * z2);
        let (xy, xz, yz) = (x * y2, x * z2, y * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Mat3 {
            cols: [
                Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy),
                Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx),
                Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)),
            ],
        }
    }

    /// is this the identity, bit for bit (no rotation work needed)
    #[inline]
    pub fn is_identity(self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.z == 0.0 && self.w == 1.0
    }

    /// the rotation by |v| radians about the direction of `v` (a rotation vector; zero gives the identity):
    /// exact, so a body spinning at a constant rate turns by its rate times the time
    pub fn from_scaled_axis(v: Vec3) -> Quat {
        let a2 = v.length_sq();
        if !(a2 > 0.0) {
            return Quat::IDENTITY;
        }
        let a = sqrt(a2);
        let (s, c) = sincos(a * 0.5);
        let k = s / a;
        Quat::new(v.x * k, v.y * k, v.z * k, c)
    }

    /// the rotation vector of a unit quaternion: its axis times its angle in [0, pi] (the inverse of
    /// [`Quat::from_scaled_axis`])
    pub fn to_scaled_axis(self) -> Vec3 {
        let q = if self.w < 0.0 { Quat::new(-self.x, -self.y, -self.z, -self.w) } else { self };
        let v = Vec3::new(q.x, q.y, q.z);
        let s = v.length();
        if !(s > 0.0) {
            return Vec3::ZERO;
        }
        v * (2.0 * atan2(s, q.w) / s)
    }

    /// the 4D dot product (the cosine of half the angle between two unit quaternions, up to the sign)
    #[inline]
    pub fn dot(self, o: Quat) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// every component finite
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }
}

// ---------------------------------------------------------------- Mat3

/// A 3 x 3 matrix by columns (a rotation from [`Quat::to_mat3`])
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    /// the columns: the images of X, Y and Z
    pub cols: [Vec3; 3],
}

impl Mat3 {
    /// the unit matrix
    pub const IDENTITY: Mat3 = Mat3 { cols: [Vec3::X, Vec3::Y, Vec3::Z] };

    /// M v
    #[inline]
    pub fn mul_vec(&self, v: Vec3) -> Vec3 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z
    }

    /// M^T v (the inverse of a rotation)
    #[inline]
    pub fn mul_vec_t(&self, v: Vec3) -> Vec3 {
        Vec3::new(self.cols[0].dot(v), self.cols[1].dot(v), self.cols[2].dot(v))
    }

    /// the zero matrix
    pub const ZERO: Mat3 = Mat3 { cols: [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO] };

    /// the matrix with `d` on the diagonal
    #[inline]
    pub const fn diagonal(d: Vec3) -> Mat3 {
        Mat3 { cols: [Vec3::new(d.x, 0.0, 0.0), Vec3::new(0.0, d.y, 0.0), Vec3::new(0.0, 0.0, d.z)] }
    }

    /// the matrix with these columns
    #[inline]
    pub const fn from_cols(x: Vec3, y: Vec3, z: Vec3) -> Mat3 {
        Mat3 { cols: [x, y, z] }
    }

    /// the element in row `r` and column `c`
    #[inline]
    pub fn at(&self, r: usize, c: usize) -> f64 {
        self.cols[c][r]
    }

    /// M^T
    pub fn transpose(&self) -> Mat3 {
        let [a, b, c] = self.cols;
        Mat3 { cols: [Vec3::new(a.x, b.x, c.x), Vec3::new(a.y, b.y, c.y), Vec3::new(a.z, b.z, c.z)] }
    }

    /// the product M N
    #[allow(clippy::should_implement_trait)] // with mul_vec beside it, Mul would hide which product is meant
    pub fn mul(&self, n: &Mat3) -> Mat3 {
        Mat3 { cols: [self.mul_vec(n.cols[0]), self.mul_vec(n.cols[1]), self.mul_vec(n.cols[2])] }
    }

    /// M + N
    #[allow(clippy::should_implement_trait)]
    pub fn add(&self, n: &Mat3) -> Mat3 {
        Mat3 { cols: [self.cols[0] + n.cols[0], self.cols[1] + n.cols[1], self.cols[2] + n.cols[2]] }
    }

    /// M - N
    #[allow(clippy::should_implement_trait)]
    pub fn sub(&self, n: &Mat3) -> Mat3 {
        Mat3 { cols: [self.cols[0] - n.cols[0], self.cols[1] - n.cols[1], self.cols[2] - n.cols[2]] }
    }

    /// s M
    pub fn scale(&self, s: f64) -> Mat3 {
        Mat3 { cols: [self.cols[0] * s, self.cols[1] * s, self.cols[2] * s] }
    }

    /// the determinant
    pub fn determinant(&self) -> f64 {
        self.cols[0].dot(self.cols[1].cross(self.cols[2]))
    }

    /// the inverse, None when the matrix is singular (or not finite)
    pub fn inverse(&self) -> Option<Mat3> {
        let [a, b, c] = self.cols;
        let (bc, ca, ab) = (b.cross(c), c.cross(a), a.cross(b));
        let det = a.dot(bc);
        if !(det != 0.0 && det.is_finite()) {
            return None;
        }
        let inv = 1.0 / det;
        // the rows of the inverse are the cross products of the columns
        Some(Mat3 { cols: [bc * inv, ca * inv, ab * inv] }.transpose())
    }

    /// R M R^T: a tensor of a body's frame (its inertia) in the frame rotated by `q`
    pub fn rotated(&self, q: Quat) -> Mat3 {
        let r = q.to_mat3();
        r.mul(self).mul(&r.transpose())
    }

    /// the outer product a b^T
    pub fn outer(a: Vec3, b: Vec3) -> Mat3 {
        Mat3 { cols: [a * b.x, a * b.y, a * b.z] }
    }

    /// the matrix of the cross product with `v` (skew(v) w = v x w)
    pub fn skew(v: Vec3) -> Mat3 {
        Mat3 { cols: [Vec3::new(0.0, v.z, -v.y), Vec3::new(-v.z, 0.0, v.x), Vec3::new(v.y, -v.x, 0.0)] }
    }

    /// the sum of the diagonal
    pub fn trace(&self) -> f64 {
        self.cols[0].x + self.cols[1].y + self.cols[2].z
    }

    /// every element finite
    pub fn is_finite(&self) -> bool {
        self.cols[0].is_finite() && self.cols[1].is_finite() && self.cols[2].is_finite()
    }
}

// ---------------------------------------------------------------- Iso

/// A placement: rotate, then move (no scale).  The pose of a body and of a query shape.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Iso {
    /// where the local origin lies
    pub pos: Vec3,
    /// the rotation (a unit quaternion)
    pub rot: Quat,
}

impl Iso {
    /// at the origin, not rotated
    pub const IDENTITY: Iso = Iso { pos: Vec3::ZERO, rot: Quat::IDENTITY };

    /// rotated by `rot`, then moved to `pos`
    #[inline]
    pub const fn new(pos: Vec3, rot: Quat) -> Iso {
        Iso { pos, rot }
    }

    /// moved to `pos`, not rotated
    #[inline]
    pub const fn at(pos: Vec3) -> Iso {
        Iso { pos, rot: Quat::IDENTITY }
    }

    /// a local point in world coordinates
    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.rot.rotate(p) + self.pos
    }

    /// a world point in local coordinates
    #[inline]
    pub fn inverse_transform_point(&self, p: Vec3) -> Vec3 {
        self.rot.inverse_rotate(p - self.pos)
    }

    /// a local direction in world coordinates
    #[inline]
    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot.rotate(v)
    }

    /// a world direction in local coordinates
    #[inline]
    pub fn inverse_transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot.inverse_rotate(v)
    }

    /// self * o: first o, then self
    #[allow(clippy::should_implement_trait)]
    pub fn mul(&self, o: &Iso) -> Iso {
        Iso { pos: self.transform_point(o.pos), rot: self.rot.mul(o.rot) }
    }

    /// the placement of `o` relative to self: self^-1 * o
    pub fn inv_mul(&self, o: &Iso) -> Iso {
        Iso { pos: self.inverse_transform_point(o.pos), rot: self.rot.conjugate().mul(o.rot) }
    }
}

// ---------------------------------------------------------------- Aabb

/// An axis-aligned box in metres
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// the corner with the smallest coordinates
    pub min: Vec3,
    /// the corner with the largest coordinates
    pub max: Vec3,
}

impl Aabb {
    /// the empty box (min above max), the start of a union
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f64::INFINITY), max: Vec3::splat(f64::NEG_INFINITY) };

    /// the box between two corners
    #[inline]
    pub const fn new(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }

    /// the box around `c` with half sizes `h`
    #[inline]
    pub fn from_center(c: Vec3, h: Vec3) -> Aabb {
        Aabb { min: c - h, max: c + h }
    }

    /// the box around both
    #[inline]
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    /// the box around this one and `p`
    #[inline]
    pub fn add_point(&self, p: Vec3) -> Aabb {
        Aabb { min: self.min.min(p), max: self.max.max(p) }
    }

    /// grown by `m` on every side
    #[inline]
    pub fn expanded(&self, m: f64) -> Aabb {
        Aabb { min: self.min - Vec3::splat(m), max: self.max + Vec3::splat(m) }
    }

    /// the boxes touch or overlap
    #[inline]
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.x <= o.max.x
            && self.max.x >= o.min.x
            && self.min.y <= o.max.y
            && self.max.y >= o.min.y
            && self.min.z <= o.max.z
            && self.max.z >= o.min.z
    }

    /// `o` lies inside (or on the border)
    #[inline]
    pub fn contains(&self, o: &Aabb) -> bool {
        self.min.x <= o.min.x
            && self.min.y <= o.min.y
            && self.min.z <= o.min.z
            && self.max.x >= o.max.x
            && self.max.y >= o.max.y
            && self.max.z >= o.max.z
    }

    /// `p` lies inside (or on the border)
    #[inline]
    pub fn contains_point(&self, p: Vec3) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y && p.z >= self.min.z && p.z <= self.max.z
    }

    /// the middle
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// half the size on each axis
    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// half the surface (the cost measure of the trees; 0 for the empty box)
    #[inline]
    pub fn half_area(&self) -> f64 {
        let d = self.max - self.min;
        if d.x < 0.0 || d.y < 0.0 || d.z < 0.0 {
            return 0.0;
        }
        d.x * d.y + d.y * d.z + d.z * d.x
    }

    /// the box is not empty and finite
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }

    /// where the ray o + t d (inv = 1 / d per axis) enters the box, if it does within [0, tmax]
    #[inline]
    pub fn ray_enter(&self, o: Vec3, inv: Vec3, tmax: f64) -> Option<f64> {
        ray_box(o, inv, self.min, self.max, tmax)
    }
}

/// the slab test: the entry parameter of o + t d into [lo, hi] within [0, tmax].  A zero component of d
/// has an infinite inverse; with the origin inside the slab that gives -inf and +inf, with the origin on a
/// slab plane 0 * inf = NaN: the axis then sets no limit, so a ray in the plane of a face counts as
/// touching it (conservative for the trees).
#[inline]
pub(crate) fn ray_box(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3, tmax: f64) -> Option<f64> {
    let mut t0 = 0.0f64;
    let mut t1 = tmax;
    let mut axis = |o: f64, inv: f64, lo: f64, hi: f64| {
        let a = (lo - o) * inv;
        let b = (hi - o) * inv;
        if a.is_nan() || b.is_nan() {
            return;
        }
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        // widened by the rounding of the subtraction, the reciprocal and the product (Ize, "Robust BVH
        // Ray Traversal", JCGT 2013: 2 gamma(3) < 1e-15): a ray through a box's face or edge is never
        // turned away by one ulp, so no hit on a triangle at the border of a node is lost
        // (an infinite value stays as it is: inf - inf would be NaN and drop the axis)
        let near = if near.is_finite() { near - abs(near) * 1e-15 } else { near };
        let far = if far.is_finite() { far + abs(far) * 1e-15 } else { far };
        if near > t0 {
            t0 = near;
        }
        if far < t1 {
            t1 = far;
        }
    };
    axis(o.x, inv.x, lo.x, hi.x);
    axis(o.y, inv.y, lo.y, hi.y);
    axis(o.z, inv.z, lo.z, hi.z);
    if t0 <= t1 { Some(t0) } else { None }
}

/// 1 / d per axis with the sign of a zero kept (so +0 gives +inf and -0 gives -inf)
#[inline]
pub(crate) fn inverse_dir(d: Vec3) -> Vec3 {
    Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z)
}
