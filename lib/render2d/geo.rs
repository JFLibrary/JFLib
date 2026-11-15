// geo -- exact geometry in f64 for picking, area selection and snapping (G6d, decision 30; docs/RENDER2D.md "Auswahl und
// Fang").  Pieces of a centre line in drawing units (Prim): segments, circular arcs and rational Bézier spans (curve.rs's
// Span, relative to an origin); their nearest points, crossings of a ray (even-odd), intersections, perpendicular feet,
// tangent points, and whether they lie inside or meet a region.  Curves stay curves: branch and bound by subdivision of
// the span, then Newton on the span itself; a line or a circle against a span through the roots of a Bernstein polynomial
// (the implicit line or circle in the span's homogeneous control points).  No libm: angles never come from atan2 (a
// direction is within an arc's sweep by a pseudo-angle), roots by bisection.

use alloc::vec::Vec;

use crate::curve::Span;
use crate::math::{sincos, sqrt};

pub(crate) type P = [f64; 2];
pub(crate) type Bx = [f64; 4];

const TAU: f64 = core::f64::consts::TAU;

#[inline]
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

#[inline]
pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

#[inline]
pub fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}

#[inline]
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

#[inline]
pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

#[inline]
pub fn len(a: P) -> f64 {
    sqrt(dot(a, a))
}

#[inline]
pub fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}

pub fn unit(a: P) -> P {
    let l = len(a);
    if l > 0.0 { [a[0] / l, a[1] / l] } else { [0.0, 0.0] }
}

/// the left normal
#[inline]
pub fn perp(a: P) -> P {
    [-a[1], a[0]]
}

pub fn bx_of(pts: &[P]) -> Bx {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

pub fn bx_union(a: &Bx, b: &Bx) -> Bx {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

pub fn bx_grow(b: &Bx, m: f64) -> Bx {
    [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
}

pub fn bx_meets(a: &Bx, b: &Bx) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

pub fn bx_holds(a: &Bx, b: &Bx) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && b[2] <= a[2] && b[3] <= a[3]
}

pub fn bx_has(b: &Bx, p: P) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// the squared distance from p to a box (0 inside)
pub fn bx_dist2(b: &Bx, p: P) -> f64 {
    let dx = (b[0] - p[0]).max(p[0] - b[2]).max(0.0);
    let dy = (b[1] - p[1]).max(p[1] - b[3]).max(0.0);
    dx * dx + dy * dy
}

/// a monotone measure of a direction's angle counter-clockwise from +x, in [0, 4) (no atan2)
pub fn pseudo_angle(v: P) -> f64 {
    let s = v[0].abs() + v[1].abs();
    if s == 0.0 {
        return 0.0;
    }
    let p = v[1] / s;
    if v[0] >= 0.0 {
        if v[1] >= 0.0 { p } else { 4.0 + p }
    } else {
        2.0 - p
    }
}

/// the direction v (from an arc's centre) lies within the arc from angle a0 over sweep sw (radians, signed), ends included
pub fn arc_has(a0: f64, sw: f64, v: P) -> bool {
    if sw.abs() >= TAU {
        return true;
    }
    let (s0, c0) = sincos(a0);
    let (se, ce) = sincos(a0 + sw);
    let rot = |u: P| [c0 * u[0] + s0 * u[1], -s0 * u[0] + c0 * u[1]];
    let mut r = rot(v);
    let mut e = rot([ce, se]);
    if sw < 0.0 {
        r[1] = -r[1];
        e[1] = -e[1];
    }
    let mut x = pseudo_angle(r);
    if x > 4.0 - 1e-12 {
        x = 0.0;
    }
    x <= pseudo_angle(e) + 1e-12
}

/// a point of an arc at an angle
pub fn arc_at(c: P, r: f64, a: f64) -> P {
    let (s, co) = sincos(a);
    [c[0] + r * co, c[1] + r * s]
}

// ---------------------------------------------------------------- rational Bézier spans

/// the homogeneous point and derivative of a span at t (de Casteljau)
pub fn sp_h(s: &Span, t: f64) -> ([f64; 3], [f64; 3]) {
    let n = s.deg as usize;
    let mut q = s.p;
    let lerp = |a: [f64; 3], b: [f64; 3]| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    for lvl in (1..n).rev() {
        for k in 0..lvl + 1 {
            q[k] = lerp(q[k], q[k + 1]);
        }
    }
    let pt = lerp(q[0], q[1]);
    let d = [(q[1][0] - q[0][0]) * n as f64, (q[1][1] - q[0][1]) * n as f64, (q[1][2] - q[0][2]) * n as f64];
    (pt, d)
}

/// the span's point at t (relative to its origin; the ends exactly its end control points)
pub fn sp_pt(s: &Span, t: f64) -> P {
    let n = s.deg as usize;
    if t <= 0.0 {
        return [s.p[0][0] / s.p[0][2], s.p[0][1] / s.p[0][2]];
    }
    if t >= 1.0 {
        return [s.p[n][0] / s.p[n][2], s.p[n][1] / s.p[n][2]];
    }
    let (p, _) = sp_h(s, t);
    [p[0] / p[2], p[1] / p[2]]
}

/// the span's derivative at t
pub fn sp_d(s: &Span, t: f64) -> P {
    let (p, d) = sp_h(s, t.clamp(0.0, 1.0));
    let w = p[2];
    [(d[0] * w - p[0] * d[2]) / (w * w), (d[1] * w - p[1] * d[2]) / (w * w)]
}

/// the span split at t
pub fn sp_split(s: &Span, t: f64) -> (Span, Span) {
    let n = s.deg as usize;
    let mut l = *s;
    let mut r = *s;
    let mut q = s.p;
    l.p[0] = q[0];
    r.p[n] = q[n];
    for lvl in 1..n + 1 {
        for k in 0..n + 1 - lvl {
            q[k] = [q[k][0] + (q[k + 1][0] - q[k][0]) * t, q[k][1] + (q[k + 1][1] - q[k][1]) * t, q[k][2] + (q[k + 1][2] - q[k][2]) * t];
        }
        l.p[lvl] = q[0];
        r.p[n - lvl] = q[n - lvl];
    }
    (l, r)
}

/// the box of the projected control points (the curve lies inside: positive weights), relative to the origin
pub fn sp_hull(s: &Span) -> Bx {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for k in 0..s.deg as usize + 1 {
        let p = [s.p[k][0] / s.p[k][2], s.p[k][1] / s.p[k][2]];
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// how far the projected inner control points lie from the chord (the span's flatness: the curve lies within that)
fn sp_dev(s: &Span) -> f64 {
    let n = s.deg as usize;
    let a = sp_pt(s, 0.0);
    let b = sp_pt(s, 1.0);
    let ab = sub(b, a);
    let ll = dot(ab, ab);
    let mut d2 = 0.0f64;
    for k in 1..n {
        let p = [s.p[k][0] / s.p[k][2], s.p[k][1] / s.p[k][2]];
        let ap = sub(p, a);
        let t = if ll > 0.0 { (dot(ap, ab) / ll).clamp(0.0, 1.0) } else { 0.0 };
        let e = sub(ap, mul(ab, t));
        d2 = d2.max(dot(e, e));
    }
    sqrt(d2)
}

/// the span's arc length from 0 to t (Gauss-Legendre, 8 points on halves until it settles)
pub fn sp_length(s: &Span, t: f64) -> f64 {
    fn gl(s: &Span, a: f64, b: f64) -> f64 {
        const X: [f64; 4] = [0.183_434_642_495_649_8, 0.525_532_409_916_329, 0.796_666_477_413_626_7, 0.960_289_856_497_536_3];
        const W: [f64; 4] = [0.362_683_783_378_362, 0.313_706_645_877_887_3, 0.222_381_034_453_374_5, 0.101_228_536_290_376_3];
        let (m, r) = (0.5 * (a + b), 0.5 * (b - a));
        let mut sum = 0.0;
        for k in 0..4 {
            sum += W[k] * (len(sp_d(s, m - r * X[k])) + len(sp_d(s, m + r * X[k])));
        }
        sum * r
    }
    fn adapt(s: &Span, a: f64, b: f64, whole: f64, depth: u32) -> f64 {
        let m = 0.5 * (a + b);
        let (l, r) = (gl(s, a, m), gl(s, m, b));
        if depth >= 30 || (l + r - whole).abs() <= 1e-13 * (l + r).abs() + 1e-300 {
            return l + r;
        }
        adapt(s, a, m, l, depth + 1) + adapt(s, m, b, r, depth + 1)
    }
    if t <= 0.0 {
        return 0.0;
    }
    if s.deg == 1 {
        return dist(sp_pt(s, 0.0), sp_pt(s, t));
    }
    let w = gl(s, 0.0, t);
    adapt(s, 0.0, t, w, 0)
}

/// the parameter at which the span's arc length from 0 reaches l (bisection)
pub fn sp_at_length(s: &Span, l: f64) -> f64 {
    let (mut a, mut b) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let m = 0.5 * (a + b);
        if sp_length(s, m) < l { a = m } else { b = m }
    }
    0.5 * (a + b)
}

// ---------------------------------------------------------------- Bernstein polynomials

/// the coefficients of the product of two polynomials in Bernstein form (degrees m and n: m + n)
pub fn bern_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let (m, n) = (a.len() - 1, b.len() - 1);
    let mut c = alloc::vec![0.0f64; m + n + 1];
    let binom = |n: usize, k: usize| -> f64 {
        let mut r = 1.0;
        for j in 0..k {
            r = r * (n - j) as f64 / (j + 1) as f64;
        }
        r
    };
    for i in 0..=m {
        for j in 0..=n {
            c[i + j] += binom(m, i) * binom(n, j) / binom(m + n, i + j) * a[i] * b[j];
        }
    }
    c
}

fn bern_eval(c: &[f64], t: f64) -> f64 {
    let mut q: Vec<f64> = c.to_vec();
    let n = q.len();
    for lvl in 1..n {
        for k in 0..n - lvl {
            q[k] += (q[k + 1] - q[k]) * t;
        }
    }
    q[0]
}

fn bern_split(c: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = c.len();
    let mut q = c.to_vec();
    let mut l = alloc::vec![0.0; n];
    let mut r = alloc::vec![0.0; n];
    l[0] = q[0];
    r[n - 1] = q[n - 1];
    for lvl in 1..n {
        for k in 0..n - lvl {
            q[k] = 0.5 * (q[k] + q[k + 1]);
        }
        l[lvl] = q[0];
        r[n - 1 - lvl] = q[n - 1 - lvl];
    }
    (l, r)
}

/// the roots in [0, 1] of a polynomial in Bernstein form (none for one that is 0 throughout); sorted, close ones once
pub fn bern_roots(c: &[f64], out: &mut Vec<f64>) {
    let scale = c.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if !(scale > 0.0) {
        return;
    }
    let mut found = Vec::new();
    roots_in(c, 0.0, 1.0, scale * 1e-300, &mut found, 0);
    found.sort_unstable_by(|a, b| a.total_cmp(b));
    for t in found {
        if out.last().is_none_or(|&l: &f64| t - l > 1e-12) {
            out.push(t);
        }
    }
}

fn sign_changes(c: &[f64]) -> u32 {
    let mut last = 0.0f64;
    let mut n = 0;
    for &v in c {
        if v != 0.0 {
            if last != 0.0 && (v > 0.0) != (last > 0.0) {
                n += 1;
            }
            last = v;
        }
    }
    n
}

fn roots_in(c: &[f64], t0: f64, t1: f64, tiny: f64, out: &mut Vec<f64>, depth: u32) {
    if out.len() > 16 {
        return;
    }
    let n = c.len();
    if c[0] == 0.0 {
        out.push(t0);
    }
    if c[n - 1] == 0.0 {
        out.push(t1);
    }
    let v = sign_changes(c);
    if v == 0 {
        return;
    }
    if v == 1 && c[0] != 0.0 && c[n - 1] != 0.0 {
        // exactly one root (Descartes): bisection on the local parameter
        let (mut a, mut b) = (0.0f64, 1.0f64);
        let sa = c[0] > 0.0;
        for _ in 0..64 {
            let m = 0.5 * (a + b);
            let f = bern_eval(c, m);
            if f == 0.0 {
                a = m;
                b = m;
                break;
            }
            if (f > 0.0) == sa { a = m } else { b = m }
        }
        out.push(t0 + (t1 - t0) * 0.5 * (a + b));
        return;
    }
    if depth >= 56 || t1 - t0 < 1e-15 {
        if c.iter().all(|v| v.abs() <= tiny) {
            return;
        }
        out.push(0.5 * (t0 + t1));
        return;
    }
    let (l, r) = bern_split(c);
    let m = 0.5 * (t0 + t1);
    roots_in(&l, t0, m, tiny, out, depth + 1);
    roots_in(&r, m, t1, tiny, out, depth + 1);
}

// ---------------------------------------------------------------- pieces of a centre line

/// a piece of a centre line in drawing units: a segment, a circular arc (centre, radius, start angle, signed sweep; a full
/// turn: a circle), a rational Bézier span relative to an origin
#[derive(Clone, Copy, Debug)]
pub enum Prim {
    Seg(P, P),
    Arc(P, f64, f64, f64),
    Span(P, Span),
}

impl Prim {
    pub fn start(&self) -> P {
        match self {
            Prim::Seg(a, _) => *a,
            Prim::Arc(c, r, a0, _) => arc_at(*c, *r, *a0),
            Prim::Span(o, s) => add(*o, sp_pt(s, 0.0)),
        }
    }

    pub fn end(&self) -> P {
        match self {
            Prim::Seg(_, b) => *b,
            Prim::Arc(c, r, a0, sw) => arc_at(*c, *r, a0 + sw),
            Prim::Span(o, s) => add(*o, sp_pt(s, 1.0)),
        }
    }

    pub fn at(&self, t: f64) -> P {
        match self {
            Prim::Seg(a, b) => add(*a, mul(sub(*b, *a), t)),
            Prim::Arc(c, r, a0, sw) => arc_at(*c, *r, a0 + sw * t),
            Prim::Span(o, s) => add(*o, sp_pt(s, t)),
        }
    }

    /// the direction of travel at t (not normalised)
    pub fn tangent(&self, t: f64) -> P {
        match self {
            Prim::Seg(a, b) => sub(*b, *a),
            Prim::Arc(_, r, a0, sw) => {
                let (s, c) = sincos(a0 + sw * t);
                mul([-s, c], r * sw)
            }
            Prim::Span(_, s) => sp_d(s, t),
        }
    }

    pub fn bx(&self) -> Bx {
        match self {
            Prim::Seg(a, b) => bx_of(&[*a, *b]),
            Prim::Arc(c, r, a0, sw) => arc_box(*c, *r, *a0, *sw),
            Prim::Span(o, s) => {
                let h = sp_hull(s);
                [h[0] + o[0], h[1] + o[1], h[2] + o[0], h[3] + o[1]]
            }
        }
    }
}

/// an arc's box: its ends and the points of its circle on the axes within its sweep
pub fn arc_box(c: P, r: f64, a0: f64, sw: f64) -> Bx {
    let mut pts = alloc::vec![arc_at(c, r, a0), arc_at(c, r, a0 + sw)];
    for d in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
        if arc_has(a0, sw, d) {
            pts.push([c[0] + r * d[0], c[1] + r * d[1]]);
        }
    }
    bx_of(&pts)
}

/// the nearest point of a piece to q: (distance, parameter, point)
pub fn nearest(q: P, p: &Prim) -> (f64, f64, P) {
    match p {
        Prim::Seg(a, b) => {
            let ab = sub(*b, *a);
            let ll = dot(ab, ab);
            let t = if ll > 0.0 { (dot(sub(q, *a), ab) / ll).clamp(0.0, 1.0) } else { 0.0 };
            let x = add(*a, mul(ab, t));
            (dist(q, x), t, x)
        }
        Prim::Arc(c, r, a0, sw) => {
            let v = sub(q, *c);
            let l = len(v);
            if l > 0.0 && arc_has(*a0, *sw, v) {
                let x = add(*c, mul(v, r / l));
                // (the parameter of that direction: the pseudo-angle's share is enough for an order, the exact angle not needed)
                let t = arc_param(*a0, *sw, v);
                return (dist(q, x), t, x);
            }
            let (s, e) = (arc_at(*c, *r, *a0), arc_at(*c, *r, a0 + sw));
            let (ds, de) = (dist(q, s), dist(q, e));
            if ds <= de { (ds, 0.0, s) } else { (de, 1.0, e) }
        }
        Prim::Span(o, s) => {
            let ql = sub(q, *o);
            let mut best = (f64::MAX, 0.0f64);
            let size = {
                let h = sp_hull(s);
                (h[2] - h[0]).max(h[3] - h[1])
            };
            span_near(ql, s, s, 0.0, 1.0, size * 1e-7, &mut best, 0);
            let t = polish_near(ql, s, best.1);
            let x = sp_pt(s, t);
            let d = dist(ql, x);
            // (Newton only where it got nearer)
            let (t, x, d) = if d <= sqrt(best.0) { (t, x, d) } else { (best.1, sp_pt(s, best.1), sqrt(best.0)) };
            (d, t, add(*o, x))
        }
    }
}

/// the parameter (0..1) of a direction within an arc (by the angle, from sincos-free arithmetic: exact enough for an order
/// and for points; the point itself comes from the direction)
pub fn arc_param(a0: f64, sw: f64, v: P) -> f64 {
    let (s0, c0) = sincos(a0);
    let mut r = [c0 * v[0] + s0 * v[1], -s0 * v[0] + c0 * v[1]];
    if sw < 0.0 {
        r[1] = -r[1];
    }
    // the angle from the start in [0, TAU) by a few steps of the half-angle formula: atan2 without libm
    let ang = angle_of(r);
    (ang / sw.abs()).clamp(0.0, 1.0)
}

/// the angle of a direction counter-clockwise from +x, in [0, TAU) (atan2 by its series near the axes)
pub fn angle_of(v: P) -> f64 {
    let (x, y) = (v[0], v[1]);
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    // reduce to |y| <= |x|, x > 0, then atan(y/x) by its series after halving the angle twice
    let a = atan_unit(x.abs(), y.abs());
    let a = if x >= 0.0 { a } else { core::f64::consts::PI - a };
    if y >= 0.0 { a } else { TAU - a }.rem_tau()
}

trait RemTau {
    fn rem_tau(self) -> f64;
}

impl RemTau for f64 {
    fn rem_tau(self) -> f64 {
        if self >= TAU { self - TAU } else { self }
    }
}

/// atan(y / x) for x, y >= 0
fn atan_unit(x: f64, y: f64) -> f64 {
    if y > x {
        return core::f64::consts::FRAC_PI_2 - atan_unit(y, x);
    }
    let mut z = y / x;
    // twice z -> z / (1 + sqrt(1 + z^2)): the angle halves (z <= tan(pi/16) then)
    let mut k = 1.0;
    for _ in 0..2 {
        z = z / (1.0 + sqrt(1.0 + z * z));
        k *= 2.0;
    }
    let z2 = z * z;
    let (mut term, mut sum) = (z, 0.0);
    for n in 0..30 {
        sum += term / (2 * n + 1) as f64;
        term *= -z2;
    }
    k * sum
}

fn span_near(q: P, orig: &Span, s: &Span, t0: f64, t1: f64, eps: f64, best: &mut (f64, f64), depth: u32) {
    let h = sp_hull(s);
    if bx_dist2(&h, q) >= best.0 {
        return;
    }
    if sp_dev(s) <= eps || depth >= 50 {
        let (a, b) = (sp_pt(s, 0.0), sp_pt(s, 1.0));
        let ab = sub(b, a);
        let ll = dot(ab, ab);
        let u = if ll > 0.0 { (dot(sub(q, a), ab) / ll).clamp(0.0, 1.0) } else { 0.0 };
        let t = t0 + (t1 - t0) * u;
        let x = sp_pt(orig, t);
        let d2 = dot(sub(q, x), sub(q, x));
        if d2 < best.0 {
            *best = (d2, t);
        }
        return;
    }
    let (l, r) = sp_split(s, 0.5);
    let m = 0.5 * (t0 + t1);
    let (dl, dr) = (bx_dist2(&sp_hull(&l), q), bx_dist2(&sp_hull(&r), q));
    if dl <= dr {
        span_near(q, orig, &l, t0, m, eps, best, depth + 1);
        span_near(q, orig, &r, m, t1, eps, best, depth + 1);
    } else {
        span_near(q, orig, &r, m, t1, eps, best, depth + 1);
        span_near(q, orig, &l, t0, m, eps, best, depth + 1);
    }
}

/// a few Newton steps on |C(t) - q|^2 from t (within [0, 1])
fn polish_near(q: P, s: &Span, t: f64) -> f64 {
    let mut t = t;
    for _ in 0..6 {
        let c = sp_pt(s, t);
        let d1 = sp_d(s, t);
        let h = 1e-6;
        let d2 = mul(sub(sp_d(s, (t + h).min(1.0)), sp_d(s, (t - h).max(0.0))), 1.0 / ((t + h).min(1.0) - (t - h).max(0.0)));
        let e = sub(c, q);
        let g = dot(e, d1);
        let hh = dot(d1, d1) + dot(e, d2);
        if !(hh > 0.0) {
            break;
        }
        let nt = (t - g / hh).clamp(0.0, 1.0);
        if (nt - t).abs() < 1e-16 {
            t = nt;
            break;
        }
        t = nt;
    }
    t
}

/// how often the piece crosses the ray from q to +x (its parity: even-odd); a piece through q's height at an end counts
/// on its upper side only (half-open, as for segments)
pub fn crossings(q: P, p: &Prim) -> bool {
    match p {
        Prim::Seg(a, b) => seg_crosses(q, *a, *b),
        Prim::Arc(c, r, a0, sw) => {
            // (as spans: exact)
            let cv = crate::curve::Curve::ellipse(*c, [*r, *r], 0.0, *a0, *sw);
            let mut odd = false;
            for s in &cv.spans {
                odd ^= span_cross(sub(q, cv.origin), s, 0);
            }
            odd
        }
        Prim::Span(o, s) => span_cross(sub(q, *o), s, 0),
    }
}

fn seg_crosses(q: P, a: P, b: P) -> bool {
    if (a[1] > q[1]) != (b[1] > q[1]) {
        let x = a[0] + (q[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
        return x > q[0];
    }
    false
}

fn span_cross(q: P, s: &Span, depth: u32) -> bool {
    let h = sp_hull(s);
    if h[1] > q[1] || h[3] <= q[1] || h[2] <= q[0] {
        return false;
    }
    let (a, b) = (sp_pt(s, 0.0), sp_pt(s, 1.0));
    if h[0] > q[0] {
        return (a[1] > q[1]) != (b[1] > q[1]);
    }
    if depth >= 52 {
        return seg_crosses(q, a, b);
    }
    let (l, r) = sp_split(s, 0.5);
    span_cross(q, &l, depth + 1) ^ span_cross(q, &r, depth + 1)
}

/// q inside the area the closed rings of pieces enclose (even-odd); on the boundary: within eps of a piece
pub fn inside(q: P, rings: &[Prim]) -> bool {
    let mut odd = false;
    for p in rings {
        odd ^= crossings(q, p);
    }
    odd
}

// ---------------------------------------------------------------- intersections

/// the points where two pieces cross or touch (overlapping collinear parts give none); out: points
pub fn intersect(p: &Prim, q: &Prim, eps: f64, out: &mut Vec<P>) {
    if !bx_meets(&bx_grow(&p.bx(), eps), &q.bx()) {
        return;
    }
    match (p, q) {
        (Prim::Seg(a, b), Prim::Seg(c, d)) => {
            if let Some(x) = seg_seg(*a, *b, *c, *d) {
                out.push(x);
            }
        }
        (Prim::Seg(a, b), Prim::Arc(c, r, a0, sw)) | (Prim::Arc(c, r, a0, sw), Prim::Seg(a, b)) => {
            for (t, x) in line_circle(*a, *b, *c, *r) {
                if (-1e-12..=1.0 + 1e-12).contains(&t) && arc_has(*a0, *sw, sub(x, *c)) {
                    out.push(x);
                }
            }
        }
        (Prim::Arc(c1, r1, a1, s1), Prim::Arc(c2, r2, a2, s2)) => {
            for x in circle_circle(*c1, *r1, *c2, *r2) {
                if arc_has(*a1, *s1, sub(x, *c1)) && arc_has(*a2, *s2, sub(x, *c2)) {
                    out.push(x);
                }
            }
        }
        (Prim::Seg(a, b), Prim::Span(o, s)) | (Prim::Span(o, s), Prim::Seg(a, b)) => {
            let (al, bl) = (sub(*a, *o), sub(*b, *o));
            let n = perp(sub(bl, al));
            let c0 = -dot(n, al);
            let co: Vec<f64> = (0..s.deg as usize + 1).map(|k| n[0] * s.p[k][0] + n[1] * s.p[k][1] + c0 * s.p[k][2]).collect();
            let mut ts = Vec::new();
            bern_roots(&co, &mut ts);
            let ab = sub(bl, al);
            let ll = dot(ab, ab);
            for t in ts {
                let x = sp_pt(s, t);
                let u = if ll > 0.0 { dot(sub(x, al), ab) / ll } else { 0.0 };
                if (-1e-12..=1.0 + 1e-12).contains(&u) {
                    out.push(add(x, *o));
                }
            }
        }
        (Prim::Arc(c, r, a0, sw), Prim::Span(o, s)) | (Prim::Span(o, s), Prim::Arc(c, r, a0, sw)) => {
            let cl = sub(*c, *o);
            let n = s.deg as usize + 1;
            let u: Vec<f64> = (0..n).map(|k| s.p[k][0] - cl[0] * s.p[k][2]).collect();
            let v: Vec<f64> = (0..n).map(|k| s.p[k][1] - cl[1] * s.p[k][2]).collect();
            let w: Vec<f64> = (0..n).map(|k| s.p[k][2]).collect();
            let uu = bern_mul(&u, &u);
            let vv = bern_mul(&v, &v);
            let ww = bern_mul(&w, &w);
            let co: Vec<f64> = (0..uu.len()).map(|k| uu[k] + vv[k] - r * r * ww[k]).collect();
            let mut ts = Vec::new();
            bern_roots(&co, &mut ts);
            for t in ts {
                let x = sp_pt(s, t);
                if arc_has(*a0, *sw, sub(x, cl)) {
                    out.push(add(x, *o));
                }
            }
        }
        (Prim::Span(o1, s1), Prim::Span(o2, s2)) => {
            let first = out.len();
            span_span(*o1, s1, s1, 0.0, 1.0, *o2, s2, s2, 0.0, 1.0, eps, out, 0);
            // (one point per crossing: those within eps of each other once)
            let mut k = first;
            while k < out.len() {
                if out[first..k].iter().any(|y| dist(*y, out[k]) <= eps) {
                    out.remove(k);
                } else {
                    k += 1;
                }
            }
        }
    }
}

/// two segments' common point (none when parallel)
pub fn seg_seg(a: P, b: P, c: P, d: P) -> Option<P> {
    let r = sub(b, a);
    let s = sub(d, c);
    let den = cross(r, s);
    if den == 0.0 {
        // (parallel, or collinear overlapping: no single point; touching ends are a point)
        if cross(sub(c, a), r) == 0.0 {
            for (x, y) in [(a, c), (a, d), (b, c), (b, d)] {
                if x == y {
                    return Some(x);
                }
            }
        }
        return None;
    }
    let ca = sub(c, a);
    let t = cross(ca, s) / den;
    let u = cross(ca, r) / den;
    let e = 1e-12;
    if t >= -e && t <= 1.0 + e && u >= -e && u <= 1.0 + e {
        return Some(add(a, mul(r, t)));
    }
    None
}

/// the line through a, b against a circle: (parameter along a -> b, point)
pub fn line_circle(a: P, b: P, c: P, r: f64) -> Vec<(f64, P)> {
    let d = sub(b, a);
    let f = sub(a, c);
    let aa = dot(d, d);
    if aa == 0.0 {
        return Vec::new();
    }
    let bb = dot(f, d);
    let cc = dot(f, f) - r * r;
    let disc = bb * bb - aa * cc;
    if disc < 0.0 {
        return Vec::new();
    }
    let sq = sqrt(disc);
    // (the root of larger magnitude from the sum, the other from the product: no cancellation)
    let q = -(bb + if bb >= 0.0 { sq } else { -sq });
    let mut v = Vec::new();
    if q != 0.0 {
        let t1 = q / aa;
        let t2 = cc / q;
        v.push((t1, add(a, mul(d, t1))));
        if disc > 0.0 {
            v.push((t2, add(a, mul(d, t2))));
        }
    } else {
        v.push((0.0, a));
    }
    v
}

/// two circles' common points
pub fn circle_circle(c1: P, r1: f64, c2: P, r2: f64) -> Vec<P> {
    let d = sub(c2, c1);
    let l = len(d);
    if l == 0.0 || l > r1 + r2 || l < (r1 - r2).abs() {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + l * l) / (2.0 * l);
    let h2 = r1 * r1 - a * a;
    let u = mul(d, 1.0 / l);
    let m = add(c1, mul(u, a));
    if h2 <= 0.0 {
        return alloc::vec![m];
    }
    let h = sqrt(h2);
    alloc::vec![add(m, mul(perp(u), h)), sub(m, mul(perp(u), h))]
}

#[allow(clippy::too_many_arguments)]
fn span_span(o1: P, f1: &Span, s1: &Span, a0: f64, a1: f64, o2: P, f2: &Span, s2: &Span, b0: f64, b1: f64, eps: f64, out: &mut Vec<P>, depth: u32) {
    if out.len() > 64 {
        return;
    }
    let h1 = sp_hull(s1);
    let h2 = sp_hull(s2);
    let g1 = [h1[0] + o1[0], h1[1] + o1[1], h1[2] + o1[0], h1[3] + o1[1]];
    let g2 = [h2[0] + o2[0], h2[1] + o2[1], h2[2] + o2[0], h2[3] + o2[1]];
    if !bx_meets(&bx_grow(&g1, eps), &g2) {
        return;
    }
    let d1 = (g1[2] - g1[0]).max(g1[3] - g1[1]);
    let d2 = (g2[2] - g2[0]).max(g2[3] - g2[1]);
    if (d1 <= eps && d2 <= eps) || depth >= 48 {
        // a candidate from the chords, then Newton on both spans
        let (p0, p1) = (add(o1, sp_pt(s1, 0.0)), add(o1, sp_pt(s1, 1.0)));
        let (q0, q1) = (add(o2, sp_pt(s2, 0.0)), add(o2, sp_pt(s2, 1.0)));
        let (mut t, mut u) = (0.5 * (a0 + a1), 0.5 * (b0 + b1));
        let r = sub(p1, p0);
        let s = sub(q1, q0);
        let den = cross(r, s);
        if den != 0.0 {
            let ca = sub(q0, p0);
            let tt = (cross(ca, s) / den).clamp(0.0, 1.0);
            let uu = (cross(ca, r) / den).clamp(0.0, 1.0);
            t = a0 + (a1 - a0) * tt;
            u = b0 + (b1 - b0) * uu;
        }
        for _ in 0..12 {
            let e = sub(add(o1, sp_pt(f1, t)), add(o2, sp_pt(f2, u)));
            let j1 = sp_d(f1, t);
            let j2 = mul(sp_d(f2, u), -1.0);
            let det = cross(j1, j2);
            if det == 0.0 {
                break;
            }
            let dt = cross(e, j2) / det;
            let du = cross(j1, e) / det;
            t = (t - dt).clamp(0.0, 1.0);
            u = (u - du).clamp(0.0, 1.0);
            if dt.abs() < 1e-17 && du.abs() < 1e-17 {
                break;
            }
        }
        let x = add(o1, sp_pt(f1, t));
        let y = add(o2, sp_pt(f2, u));
        if dist(x, y) <= eps {
            out.push(mul(add(x, y), 0.5));
        }
        return;
    }
    if d1 >= d2 {
        let (l, r) = sp_split(s1, 0.5);
        let m = 0.5 * (a0 + a1);
        span_span(o1, f1, &l, a0, m, o2, f2, s2, b0, b1, eps, out, depth + 1);
        span_span(o1, f1, &r, m, a1, o2, f2, s2, b0, b1, eps, out, depth + 1);
    } else {
        let (l, r) = sp_split(s2, 0.5);
        let m = 0.5 * (b0 + b1);
        span_span(o1, f1, s1, a0, a1, o2, f2, &l, b0, m, eps, out, depth + 1);
        span_span(o1, f1, s1, a0, a1, o2, f2, &r, m, b1, eps, out, depth + 1);
    }
}

// ---------------------------------------------------------------- perpendicular feet, tangent points

/// the points of a piece where the line from b meets it at a right angle
pub fn perp_feet(b: P, p: &Prim, out: &mut Vec<P>) {
    match p {
        Prim::Seg(a, e) => {
            let ab = sub(*e, *a);
            let ll = dot(ab, ab);
            if ll > 0.0 {
                let t = dot(sub(b, *a), ab) / ll;
                if (-1e-12..=1.0 + 1e-12).contains(&t) {
                    out.push(add(*a, mul(ab, t.clamp(0.0, 1.0))));
                }
            }
        }
        Prim::Arc(c, r, a0, sw) => {
            let v = sub(b, *c);
            let l = len(v);
            if l > 0.0 {
                for s in [1.0, -1.0] {
                    let d = mul(v, s / l);
                    if arc_has(*a0, *sw, d) {
                        out.push(add(*c, mul(d, *r)));
                    }
                }
            }
        }
        Prim::Span(o, s) => {
            let bl = sub(b, *o);
            span_roots(s, &|t| dot(sub(sp_pt(s, t), bl), sp_d(s, t)), &mut |t| out.push(add(*o, sp_pt(s, t))));
        }
    }
}

/// the points of a piece where the line from b touches it (none on segments)
pub fn tangents(b: P, p: &Prim, out: &mut Vec<P>) {
    match p {
        Prim::Seg(..) => {}
        Prim::Arc(c, r, a0, sw) => {
            let v = sub(b, *c);
            let d = len(v);
            if d > *r && d > 0.0 {
                let u = mul(v, 1.0 / d);
                let co = r / d;
                let si = sqrt((1.0 - co * co).max(0.0));
                for sg in [1.0, -1.0] {
                    let dir = add(mul(u, co), mul(perp(u), sg * si));
                    if arc_has(*a0, *sw, dir) {
                        out.push(add(*c, mul(dir, *r)));
                    }
                }
            }
        }
        Prim::Span(o, s) => {
            let bl = sub(b, *o);
            span_roots(s, &|t| cross(sub(sp_pt(s, t), bl), sp_d(s, t)), &mut |t| out.push(add(*o, sp_pt(s, t))));
        }
    }
}

/// the roots of f on [0, 1] (a span's function): sign changes between 32 samples, then bisection
fn span_roots(_s: &Span, f: &dyn Fn(f64) -> f64, put: &mut dyn FnMut(f64)) {
    let n = 32;
    let mut ta = 0.0f64;
    let mut fa = f(0.0);
    if fa == 0.0 {
        put(0.0);
    }
    for k in 1..=n {
        let tb = k as f64 / n as f64;
        let fb = f(tb);
        if fb == 0.0 {
            put(tb);
        } else if fa != 0.0 && (fa > 0.0) != (fb > 0.0) {
            let (mut a, mut b) = (ta, tb);
            let sa = fa > 0.0;
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                let fm = f(m);
                if fm == 0.0 {
                    a = m;
                    b = m;
                    break;
                }
                if (fm > 0.0) == sa { a = m } else { b = m }
            }
            put(0.5 * (a + b));
        }
        ta = tb;
        fa = fb;
    }
}

// ---------------------------------------------------------------- regions

/// an area selection's region in drawing units: an axis-aligned rectangle or a closed polygon (even-odd)
#[derive(Clone, Debug)]
pub enum Area {
    Rect(Bx),
    Poly(Vec<P>),
}

impl Area {
    pub fn bx(&self) -> Bx {
        match self {
            Area::Rect(b) => *b,
            Area::Poly(v) => bx_of(v),
        }
    }

    /// a point of the region (a crossing region inside an area meets it)
    pub fn some_point(&self) -> P {
        match self {
            Area::Rect(b) => [b[0], b[1]],
            Area::Poly(v) => v[0],
        }
    }

    fn edges(&self) -> Vec<(P, P)> {
        match self {
            Area::Rect(b) => {
                let c = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
                (0..4).map(|k| (c[k], c[(k + 1) % 4])).collect()
            }
            Area::Poly(v) => (0..v.len()).map(|k| (v[k], v[(k + 1) % v.len()])).collect(),
        }
    }

    /// p in the closed region
    pub fn has(&self, p: P) -> bool {
        match self {
            Area::Rect(b) => bx_has(b, p),
            Area::Poly(v) => {
                let n = v.len();
                let mut odd = false;
                for k in 0..n {
                    let (a, b) = (v[k], v[(k + 1) % n]);
                    // (on an edge: inside)
                    let ab = sub(b, a);
                    let ll = dot(ab, ab);
                    if ll > 0.0 {
                        let t = (dot(sub(p, a), ab) / ll).clamp(0.0, 1.0);
                        let e = sub(p, add(a, mul(ab, t)));
                        if dot(e, e) <= 1e-24 * ll {
                            return true;
                        }
                    } else if p == a {
                        return true;
                    }
                    odd ^= seg_crosses(p, a, b);
                }
                odd
            }
        }
    }
}

/// the piece lies wholly in the region (its points in it, boundary included)
pub fn prim_in(p: &Prim, area: &Area) -> bool {
    if let Area::Rect(b) = area {
        return match p {
            Prim::Seg(a, e) => bx_has(b, *a) && bx_has(b, *e),
            Prim::Arc(..) => bx_holds(b, &p.bx()),
            Prim::Span(o, s) => span_in_rect(&[b[0] - o[0], b[1] - o[1], b[2] - o[0], b[3] - o[1]], s, 0),
        };
    }
    // a polygon: where the piece crosses the polygon's edges it may leave it; the piece's ends and the middles between
    // those crossings all inside
    if !bx_meets(&area.bx(), &p.bx()) {
        return false;
    }
    let mut ts = alloc::vec![0.0, 1.0];
    for (a, b) in area.edges() {
        params_on_seg(p, a, b, &mut ts);
    }
    ts.sort_unstable_by(|a, b| a.total_cmp(b));
    if !(area.has(p.start()) && area.has(p.end())) {
        return false;
    }
    for w in ts.windows(2) {
        if w[1] - w[0] > 1e-12 && !area.has(p.at(0.5 * (w[0] + w[1]))) {
            return false;
        }
    }
    true
}

/// the piece has a point in the region
pub fn prim_meets(p: &Prim, area: &Area) -> bool {
    if !bx_meets(&area.bx(), &p.bx()) {
        return false;
    }
    if area.has(p.start()) || area.has(p.end()) {
        return true;
    }
    if let (Area::Rect(b), Prim::Span(o, s)) = (area, p) {
        return span_meets_rect(&[b[0] - o[0], b[1] - o[1], b[2] - o[0], b[3] - o[1]], s, 0);
    }
    let mut ts = Vec::new();
    for (a, b) in area.edges() {
        params_on_seg(p, a, b, &mut ts);
        if !ts.is_empty() {
            return true;
        }
    }
    false
}

/// the parameters of the piece where it meets the segment a b
fn params_on_seg(p: &Prim, a: P, b: P, ts: &mut Vec<f64>) {
    match p {
        Prim::Seg(c, d) => {
            let r = sub(*d, *c);
            let s = sub(b, a);
            let den = cross(r, s);
            if den == 0.0 {
                return;
            }
            let ac = sub(a, *c);
            let t = cross(ac, s) / den;
            let u = cross(ac, r) / den;
            if (-1e-12..=1.0 + 1e-12).contains(&t) && (-1e-12..=1.0 + 1e-12).contains(&u) {
                ts.push(t.clamp(0.0, 1.0));
            }
        }
        Prim::Arc(c, r, a0, sw) => {
            for (u, x) in line_circle(a, b, *c, *r) {
                if (-1e-12..=1.0 + 1e-12).contains(&u) && arc_has(*a0, *sw, sub(x, *c)) {
                    ts.push(arc_param(*a0, *sw, sub(x, *c)));
                }
            }
        }
        Prim::Span(o, s) => {
            let (al, bl) = (sub(a, *o), sub(b, *o));
            let n = perp(sub(bl, al));
            let c0 = -dot(n, al);
            let co: Vec<f64> = (0..s.deg as usize + 1).map(|k| n[0] * s.p[k][0] + n[1] * s.p[k][1] + c0 * s.p[k][2]).collect();
            let mut roots = Vec::new();
            bern_roots(&co, &mut roots);
            let ab = sub(bl, al);
            let ll = dot(ab, ab);
            for t in roots {
                let x = sp_pt(s, t);
                let u = if ll > 0.0 { dot(sub(x, al), ab) / ll } else { 0.0 };
                if (-1e-12..=1.0 + 1e-12).contains(&u) {
                    ts.push(t);
                }
            }
        }
    }
}

fn span_in_rect(b: &Bx, s: &Span, depth: u32) -> bool {
    let h = sp_hull(s);
    if bx_holds(b, &h) {
        return true;
    }
    let (a, e) = (sp_pt(s, 0.0), sp_pt(s, 1.0));
    if !bx_has(b, a) || !bx_has(b, e) {
        return false;
    }
    if depth >= 50 {
        return true;
    }
    let (l, r) = sp_split(s, 0.5);
    span_in_rect(b, &l, depth + 1) && span_in_rect(b, &r, depth + 1)
}

fn span_meets_rect(b: &Bx, s: &Span, depth: u32) -> bool {
    let h = sp_hull(s);
    if !bx_meets(b, &h) {
        return false;
    }
    let (a, e) = (sp_pt(s, 0.0), sp_pt(s, 1.0));
    if bx_has(b, a) || bx_has(b, e) {
        return true;
    }
    if depth >= 50 {
        return seg_meets_rect(b, a, e);
    }
    let (l, r) = sp_split(s, 0.5);
    span_meets_rect(b, &l, depth + 1) || span_meets_rect(b, &r, depth + 1)
}

/// a segment meets a box (Liang-Barsky)
pub fn seg_meets_rect(b: &Bx, a: P, e: P) -> bool {
    let d = sub(e, a);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for k in 0..2 {
        if d[k] == 0.0 {
            if a[k] < b[k] || a[k] > b[k + 2] {
                return false;
            }
        } else {
            let (ta, tb) = ((b[k] - a[k]) / d[k], (b[k + 2] - a[k]) / d[k]);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        }
    }
    t0 <= t1
}

// ---------------------------------------------------------------- stroke pieces (picking)

/// the distance from q to a rectangle along a -> b, h on either side (infinite for a = b: no body)
pub fn rect_dist(q: P, a: P, b: P, h: f64) -> f64 {
    let ab = sub(b, a);
    let l = len(ab);
    if !(l > 0.0) {
        return f64::MAX;
    }
    let u = mul(ab, 1.0 / l);
    let v = sub(q, a);
    let s = dot(v, u);
    let n = cross(u, v).abs();
    let dx = (-s).max(s - l).max(0.0);
    let dy = (n - h).max(0.0);
    sqrt(dx * dx + dy * dy)
}

/// the distance from q to a convex polygon (counter-clockwise or clockwise; 0 inside)
pub fn convex_dist(q: P, pts: &[P]) -> f64 {
    let n = pts.len();
    let mut sign = 0.0f64;
    let mut inside = true;
    for k in 0..n {
        let c = cross(sub(pts[(k + 1) % n], pts[k]), sub(q, pts[k]));
        if c != 0.0 {
            if sign == 0.0 {
                sign = c.signum();
            } else if c.signum() != sign {
                inside = false;
            }
        }
    }
    if inside {
        return 0.0;
    }
    let mut d = f64::MAX;
    for k in 0..n {
        d = d.min(nearest(q, &Prim::Seg(pts[k], pts[(k + 1) % n])).0);
    }
    d
}

/// the distance from q to what a join adds beyond the two rectangles at vertex v (d1: the direction into v, d2: out of
/// it, both unit; h the half width): a miter's quadrilateral up to the limit, else a bevel's triangle, or a disk.
/// join: 0 miter, 1 round, 2 bevel (Join's order)
pub fn join_dist(q: P, v: P, d1: P, d2: P, h: f64, join: u32, limit: f64) -> f64 {
    let turn = cross(d1, d2);
    if join == 1 {
        return (dist(q, v) - h).max(0.0);
    }
    if turn == 0.0 {
        return f64::MAX;
    }
    // the outer side: right of a left turn, left of a right turn
    let (o1, o2) = if turn > 0.0 { (mul(perp(d1), -1.0), mul(perp(d2), -1.0)) } else { (perp(d1), perp(d2)) };
    let c1 = add(v, mul(o1, h));
    let c2 = add(v, mul(o2, h));
    let c = dot(d1, d2);
    let d1c = c + 1.0;
    if join == 0 && d1c > 1e-9 && 2.0 <= limit * limit * d1c {
        let s = add(o1, o2);
        let ss = dot(s, s);
        let tip = add(v, mul(s, 2.0 * h / ss));
        return convex_dist(q, &[v, c1, tip, c2]);
    }
    convex_dist(q, &[v, c1, c2])
}

/// what a cap adds beyond an open end e (out: the unit direction out of the stroke there): 0 butt, 1 round, 2 square
pub fn cap_dist(q: P, e: P, out: P, h: f64, cap: u32) -> f64 {
    match cap {
        1 => (dist(q, e) - h).max(0.0),
        2 => rect_dist(q, e, add(e, mul(out, h)), h),
        _ => f64::MAX,
    }
}

/// the largest and smallest singular values of a 2 x 2 map and the unit direction the larger one stretches (its right
/// singular vector); for a similarity the direction is (1, 0)
pub fn svd2(m: [[f64; 2]; 2]) -> (f64, f64, P) {
    let (a, b, c, d) = (m[0][0], m[0][1], m[1][0], m[1][1]);
    let p = a * a + c * c;
    let qv = a * b + c * d;
    let s = b * b + d * d;
    let half = 0.5 * (p - s);
    let r = sqrt(half * half + qv * qv);
    let l1 = 0.5 * (p + s) + r;
    let l2 = (0.5 * (p + s) - r).max(0.0);
    let v1 = if r <= 1e-12 * (p + s) {
        [1.0, 0.0]
    } else {
        let x = [qv, l1 - p];
        let y = [l1 - s, qv];
        unit(if dot(x, x) >= dot(y, y) { x } else { y })
    };
    (sqrt(l1), sqrt(l2), v1)
}
