// exact -- arithmetic without rounding error for the decisions of the Boolean operations (docs/GEOM.md,
// "Boolesche Operationen"): expansions after Shewchuk (a number as a sum of f64 components that do not overlap,
// smallest first; sums and products of them are exact), the orientation of three points in the plane, the
// side of a plane at the point where three other planes meet, and the f64 nearest to a quotient of two
// expansions.  Only + - * on f64 (never a fused multiply-add: Rust does not contract a * b + c), so the same
// bits and the same answers on every target.  Each predicate first computes in f64 and decides there when the
// result is farther from 0 than its error bound; only near 0 (and exactly at 0) it computes exactly.

use alloc::vec::Vec;

/// 2^27 + 1: splits an f64 into two halves of 26 bits (Dekker)
const SPLITTER: f64 = 134_217_729.0;

/// half an ulp of 1 (2^-53): the unit roundoff of f64
const EPS: f64 = 1.0 / 9_007_199_254_740_992.0;

/// a + b as (rounded sum, its error): exact
#[inline]
pub(crate) fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let bv = x - a;
    let av = x - bv;
    (x, (a - av) + (b - bv))
}

/// a + b for |a| >= |b| (or a = 0): exact
#[inline]
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    (x, b - (x - a))
}

/// a - b as (rounded difference, its error): exact
#[inline]
pub(crate) fn two_diff(a: f64, b: f64) -> (f64, f64) {
    let x = a - b;
    let bv = a - x;
    let av = x + bv;
    (x, (a - av) + (bv - b))
}

/// the two halves of a (Dekker's split)
#[inline]
fn split(a: f64) -> (f64, f64) {
    let c = SPLITTER * a;
    let hi = c - (c - a);
    (hi, a - hi)
}

/// a * b as (rounded product, its error): exact (without underflow and overflow)
#[inline]
pub(crate) fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    two_product_presplit(a, ah, al, b, bh, bl, x)
}

#[inline]
fn two_product_presplit(_a: f64, ah: f64, al: f64, _b: f64, bh: f64, bl: f64, x: f64) -> (f64, f64) {
    let e1 = x - ah * bh;
    let e2 = e1 - al * bh;
    let e3 = e2 - ah * bl;
    (x, al * bl - e3)
}

/// An exact number: f64 components that do not overlap, smallest magnitude first, no zeros (an empty list is 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exp(pub Vec<f64>);

impl Exp {
    /// 0
    pub fn zero() -> Exp {
        Exp(Vec::new())
    }

    /// one f64
    pub fn of(a: f64) -> Exp {
        if a == 0.0 { Exp::zero() } else { Exp(alloc::vec![a]) }
    }

    /// a * b, exact
    pub fn product(a: f64, b: f64) -> Exp {
        let (x, y) = two_product(a, b);
        let mut v = Vec::with_capacity(2);
        if y != 0.0 {
            v.push(y);
        }
        if x != 0.0 {
            v.push(x);
        }
        Exp(v)
    }

    /// a - b, exact
    pub fn diff(a: f64, b: f64) -> Exp {
        let (x, y) = two_diff(a, b);
        let mut v = Vec::with_capacity(2);
        if y != 0.0 {
            v.push(y);
        }
        if x != 0.0 {
            v.push(x);
        }
        Exp(v)
    }

    /// -1, 0 or 1: the sign of the largest component
    pub fn sign(&self) -> i32 {
        match self.0.last() {
            Some(&x) if x > 0.0 => 1,
            Some(&x) if x < 0.0 => -1,
            _ => 0,
        }
    }

    /// the components summed in f64, smallest first: close to the value (after compress within an ulp)
    pub fn estimate(&self) -> f64 {
        self.0.iter().fold(0.0, |s, &x| s + x)
    }

    /// self + b, exact (Shewchuk's Grow-Expansion with zero elimination)
    pub fn add_f64(&self, b: f64) -> Exp {
        let mut q = b;
        let mut h = Vec::with_capacity(self.0.len() + 1);
        for &e in &self.0 {
            let (s, err) = two_sum(q, e);
            q = s;
            if err != 0.0 {
                h.push(err);
            }
        }
        if q != 0.0 {
            h.push(q);
        }
        Exp(h)
    }

    /// self + o, exact: o's components grown in one by one (correct for any two expansions that do not overlap)
    pub fn add(&self, o: &Exp) -> Exp {
        let (small, big) = if self.0.len() < o.0.len() { (self, o) } else { (o, self) };
        let mut r = big.clone();
        for &x in &small.0 {
            r = r.add_f64(x);
        }
        r
    }

    /// -self
    pub fn neg(&self) -> Exp {
        Exp(self.0.iter().map(|&x| -x).collect())
    }

    /// self - o, exact
    pub fn sub(&self, o: &Exp) -> Exp {
        self.add(&o.neg())
    }

    /// self * b, exact (Shewchuk's Scale-Expansion with zero elimination)
    pub fn scale(&self, b: f64) -> Exp {
        if self.0.is_empty() || b == 0.0 {
            return Exp::zero();
        }
        let (bh, bl) = split(b);
        let mut h = Vec::with_capacity(2 * self.0.len());
        let e0 = self.0[0];
        let (eh, el) = split(e0);
        let (mut q, hh) = two_product_presplit(e0, eh, el, b, bh, bl, e0 * b);
        if hh != 0.0 {
            h.push(hh);
        }
        for &e in &self.0[1..] {
            let (eh, el) = split(e);
            let (p1, p0) = two_product_presplit(e, eh, el, b, bh, bl, e * b);
            let (s, hh) = two_sum(q, p0);
            if hh != 0.0 {
                h.push(hh);
            }
            let (qn, hh) = fast_two_sum(p1, s);
            q = qn;
            if hh != 0.0 {
                h.push(hh);
            }
        }
        if q != 0.0 {
            h.push(q);
        }
        Exp(h)
    }

    /// self * o, exact: the sum of self scaled by each component of o
    pub fn mul(&self, o: &Exp) -> Exp {
        let (a, b) = if self.0.len() < o.0.len() { (o, self) } else { (self, o) };
        let mut r = Exp::zero();
        for &x in &b.0 {
            r = r.add(&a.scale(x));
        }
        r.compress()
    }

    /// the same value in as few components as possible (Shewchuk's Compress): the largest component is then
    /// the value rounded to within an ulp
    pub fn compress(&self) -> Exp {
        let e = &self.0;
        let m = e.len();
        if m < 2 {
            return self.clone();
        }
        let mut g = alloc::vec![0.0; m];
        let mut bottom = m - 1;
        let mut q = e[m - 1];
        for i in (0..m - 1).rev() {
            let (qn, ql) = fast_two_sum(q, e[i]);
            if ql != 0.0 {
                g[bottom] = qn;
                bottom -= 1;
                q = ql;
            } else {
                q = qn;
            }
        }
        g[bottom] = q;
        let mut h = Vec::with_capacity(m - bottom);
        for &gi in &g[bottom + 1..] {
            let (qn, ql) = fast_two_sum(gi, q);
            if ql != 0.0 {
                h.push(ql);
            }
            q = qn;
        }
        if q != 0.0 {
            h.push(q);
        }
        Exp(h)
    }
}

// ---------------------------------------------------------------- orientation in the plane

/// Shewchuk's bound for orient2d's f64 evaluation: (3 + 16 eps) eps
const CCW_BOUND: f64 = (3.0 + 16.0 * EPS) * EPS;

/// The orientation of a, b, c in the plane: 1 counter-clockwise (c left of a -> b), -1 clockwise, 0 on one
/// line -- exact: the f64 determinant decides when it is clearly away from 0, else the exact one.
pub fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> i32 {
    let l = (a[0] - c[0]) * (b[1] - c[1]);
    let r = (a[1] - c[1]) * (b[0] - c[0]);
    let det = l - r;
    let bound = CCW_BOUND * (crate::math::abs(l) + crate::math::abs(r));
    if det > bound {
        return 1;
    }
    if -det > bound {
        return -1;
    }
    orient2d_exact(a, b, c)
}

/// orient2d without the f64 shortcut
pub fn orient2d_exact(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> i32 {
    let acx = Exp::diff(a[0], c[0]);
    let bcy = Exp::diff(b[1], c[1]);
    let acy = Exp::diff(a[1], c[1]);
    let bcx = Exp::diff(b[0], c[0]);
    acx.mul(&bcy).sub(&acy.mul(&bcx)).sign()
}

// ---------------------------------------------------------------- planes and their points

/// a plane n . x + w = 0 as [nx, ny, nz, w]; its inside is where n . x + w < 0
pub type Pl = [f64; 4];

/// The point where three planes meet, as the cofactors x~ of the 3 x 4 matrix of their rows: for any plane
/// p, det[p1; p2; p3; p] = p . x~, and x~ = det3(normals) * (x, y, z, 1).  Exact (expansions) on request,
/// always in f64 with error bounds for the shortcut.
#[derive(Clone, Debug)]
pub struct Meet {
    /// the cofactors in f64
    pub approx: [f64; 4],
    /// the permanents (the same sums of products, every term positive): the scale of each cofactor's error
    pub perm: [f64; 4],
    /// the exact cofactors, compressed (computed the first time a decision needs them)
    pub exact: Option<[Exp; 4]>,
}

/// det3 of rows (a b c / d e f / g h i) in f64 and its permanent
#[inline]
fn det3_f64(m: [[f64; 3]; 3]) -> (f64, f64) {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let abs = crate::math::abs;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let perm = abs(a) * (abs(e) * abs(i) + abs(f) * abs(h)) + abs(b) * (abs(d) * abs(i) + abs(f) * abs(g)) + abs(c) * (abs(d) * abs(h) + abs(e) * abs(g));
    (det, perm)
}

/// det3 exactly
fn det3_exact(m: [[f64; 3]; 3]) -> Exp {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let t = |x: f64, y: f64, z: f64| Exp::product(x, y).scale(z);
    let pos = t(a, e, i).add(&t(b, f, g)).add(&t(c, d, h));
    let neg = t(a, f, h).add(&t(b, d, i)).add(&t(c, e, g));
    pos.sub(&neg).compress()
}

/// the 3 x 3 minor of rows p, q, r without column k
#[inline]
fn minor(p: &Pl, q: &Pl, r: &Pl, k: usize) -> [[f64; 3]; 3] {
    let pick = |row: &Pl| -> [f64; 3] {
        let mut o = [0.0; 3];
        let mut j = 0;
        for (c, &v) in row.iter().enumerate() {
            if c != k {
                o[j] = v;
                j += 1;
            }
        }
        o
    };
    [pick(p), pick(q), pick(r)]
}

/// the sign the cofactor k carries: x~_k = (-1)^(3 + k) M_k
const COF_SIGN: [f64; 4] = [-1.0, 1.0, -1.0, 1.0];

impl Meet {
    /// where planes p, q, r meet (x~ in f64; exact later on request)
    pub fn new(p: &Pl, q: &Pl, r: &Pl) -> Meet {
        let mut approx = [0.0; 4];
        let mut perm = [0.0; 4];
        for k in 0..4 {
            let (d, pm) = det3_f64(minor(p, q, r, k));
            approx[k] = d * COF_SIGN[k];
            perm[k] = pm;
        }
        Meet { approx, perm, exact: None }
    }

    /// the exact cofactors (computed once)
    pub fn exact(&mut self, p: &Pl, q: &Pl, r: &Pl) -> &[Exp; 4] {
        if self.exact.is_none() {
            let e = core::array::from_fn(|k| {
                let d = det3_exact(minor(p, q, r, k));
                if COF_SIGN[k] < 0.0 { d.neg() } else { d }
            });
            self.exact = Some(e);
        }
        self.exact.as_ref().unwrap()
    }

    /// the sign of x~_w = det3 of the three normals: 0 when the planes do not meet in one point
    pub fn w_sign(&mut self, p: &Pl, q: &Pl, r: &Pl) -> i32 {
        let a = self.approx[3];
        if crate::math::abs(a) > 1e-15 * self.perm[3] {
            return if a > 0.0 { 1 } else { -1 };
        }
        self.exact(p, q, r)[3].sign()
    }

    /// The sign of p . x~ (det[p1; p2; p3; s]): with w_sign, the side of plane s at the point (exact).
    pub fn dot_sign(&mut self, s: &Pl, p: &Pl, q: &Pl, r: &Pl) -> i32 {
        let abs = crate::math::abs;
        let mut v = 0.0;
        let mut b = 0.0;
        for k in 0..4 {
            v += s[k] * self.approx[k];
            b += abs(s[k]) * self.perm[k];
        }
        // the f64 cofactors are within 1e-15 of their permanents, the sum of four products adds 4 eps: 4e-15 is
        // twice that (docs/GEOM.md)
        let bound = 4e-15 * b;
        if v > bound {
            return 1;
        }
        if -v > bound {
            return -1;
        }
        let e = self.exact(p, q, r);
        let mut sum = Exp::zero();
        for k in 0..4 {
            sum = sum.add(&e[k].scale(s[k]));
        }
        sum.sign()
    }

    /// the point's coordinates, each the f64 nearest to the exact quotient (ties to even): equal points give
    /// equal bits however their planes were chosen
    pub fn point(&mut self, p: &Pl, q: &Pl, r: &Pl) -> [f64; 3] {
        let e = self.exact(p, q, r).clone();
        [div_round(&e[0], &e[3]), div_round(&e[1], &e[3]), div_round(&e[2], &e[3])]
    }
}

/// the sign of n - q * d (exact)
fn residual_sign(n: &Exp, d: &Exp, q: f64) -> i32 {
    n.sub(&d.scale(q)).sign()
}

/// The f64 nearest to n / d (ties to even), d not 0: an estimate, moved by ulps until n / d lies between two
/// neighbours, then the side of their midpoint decides (2n against (lo + hi) d, exact).
pub fn div_round(n: &Exp, d: &Exp) -> f64 {
    let sd = d.sign();
    if n.sign() == 0 || sd == 0 {
        return 0.0;
    }
    let (n, d) = (n.compress(), d.compress());
    let mut q = n.estimate() / d.estimate();
    if !q.is_finite() {
        q = 0.0;
    }
    // r(q) > 0: n / d > q
    let r = |q: f64| residual_sign(&n, &d, q) * sd;
    let mut lo;
    let mut hi;
    let mut steps = 0;
    loop {
        steps += 1;
        let s = r(q);
        if s == 0 {
            return q + 0.0;
        }
        if s > 0 {
            let u = q.next_up();
            let su = r(u);
            if su == 0 {
                return u + 0.0;
            }
            if su < 0 || steps > 200 {
                lo = q;
                hi = u;
                break;
            }
            q = u;
        } else {
            let dn = q.next_down();
            let sdn = r(dn);
            if sdn == 0 {
                return dn + 0.0;
            }
            if sdn > 0 || steps > 200 {
                lo = dn;
                hi = q;
                break;
            }
            q = dn;
        }
    }
    if lo > hi {
        core::mem::swap(&mut lo, &mut hi);
    }
    // 2n - (lo + hi) d
    let m = n.add(&n).sub(&d.scale(lo)).sub(&d.scale(hi)).sign() * sd;
    let out = if m > 0 {
        hi
    } else if m < 0 {
        lo
    } else if lo.to_bits() & 1 == 0 {
        lo
    } else {
        hi
    };
    out + 0.0
}
