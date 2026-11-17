// gjk -- distance, penetration and time of impact between two convex shapes given as support functions
//
// GJK (Gilbert, Johnson, Keerthi) finds the point of the Minkowski difference D = A - B closest to the
// origin; the simplex is reduced by the Voronoi regions of its vertices, edges and faces (Ericson,
// "Real-Time Collision Detection", 5.1.5 and 9.5).  EPA expands a polytope inside D to find the shallowest
// way out when the cores overlap.  The cast is GJK ray casting with conservative advancement (van den
// Bergen 2004, as in Box2D's b2ShapeCast): the support plane of D in the direction of the current closest
// point bounds the distance at every later time, so the time advances to where that bound meets the target
// distance and never beyond the first contact.  The radii of spheres and capsules stay outside GJK as
// margins: the cores of two such shapes are apart at contact, which keeps the casts well conditioned.

use crate::convex::Placed;
use crate::math::{Vec3, max};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// GJK stops when the next support point brings the distance closer by at most this fraction
const REL: f64 = 1e-12;
/// at most so many support points per GJK and per cast
const MAX_ITERS: u32 = 64;
/// a cast stops when the shapes are this close to the target gap (metres)
pub(crate) const CAST_TOL: f64 = 1e-9;
/// two shapes without radius (boxes, hulls, triangles) stop a cast at this gap (metres): a positive
/// distance keeps the direction of the closest points well defined
pub const CAST_SKIN: f64 = 1e-6;
/// cores closer than this count as overlapping (metres)
const TOUCH: f64 = 1e-12;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sv {
    pub w: Vec3,
    pub a: Vec3,
    pub b: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Simplex {
    pub v: [Sv; 4],
    pub bary: [f64; 4],
    pub n: usize,
}

impl Simplex {
    pub(crate) fn new() -> Simplex {
        Simplex { v: [Sv::default(); 4], bary: [0.0; 4], n: 0 }
    }

    fn push(&mut self, s: Sv) {
        self.v[self.n] = s;
        self.n += 1;
    }

    fn contains(&self, w: Vec3) -> bool {
        self.v[..self.n].iter().any(|s| s.w == w)
    }

    fn keep1(&mut self, i: usize) -> Vec3 {
        self.v[0] = self.v[i];
        self.bary[0] = 1.0;
        self.n = 1;
        self.v[0].w
    }

    fn keep2(&mut self, i: usize, j: usize, t: f64) -> Vec3 {
        let (a, b) = (self.v[i], self.v[j]);
        self.v[0] = a;
        self.v[1] = b;
        self.bary[0] = 1.0 - t;
        self.bary[1] = t;
        self.n = 2;
        a.w + (b.w - a.w) * t
    }

    /// the point of the simplex closest to the origin; the simplex shrinks to the smallest face that holds
    /// it (4 points stay when the origin is inside: the cores overlap)
    pub(crate) fn closest(&mut self) -> Vec3 {
        match self.n {
            1 => {
                self.bary[0] = 1.0;
                self.v[0].w
            }
            2 => self.solve2(),
            3 => self.solve3(),
            _ => self.solve4(),
        }
    }

    fn solve2(&mut self) -> Vec3 {
        let (a, b) = (self.v[0].w, self.v[1].w);
        let ab = b - a;
        let num = -a.dot(ab);
        let den = ab.dot(ab);
        if num <= 0.0 {
            return self.keep1(0);
        }
        if num >= den {
            return self.keep1(1);
        }
        self.keep2(0, 1, num / den)
    }

    fn solve3(&mut self) -> Vec3 {
        let (a, b, c) = (self.v[0].w, self.v[1].w, self.v[2].w);
        let ab = b - a;
        let ac = c - a;
        let d1 = -ab.dot(a);
        let d2 = -ac.dot(a);
        if d1 <= 0.0 && d2 <= 0.0 {
            return self.keep1(0);
        }
        let d3 = -ab.dot(b);
        let d4 = -ac.dot(b);
        if d3 >= 0.0 && d4 <= d3 {
            return self.keep1(1);
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let den = d1 - d3;
            return self.keep2(0, 1, if den > 0.0 { d1 / den } else { 0.0 });
        }
        let d5 = -ab.dot(c);
        let d6 = -ac.dot(c);
        if d6 >= 0.0 && d5 <= d6 {
            return self.keep1(2);
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let den = d2 - d6;
            return self.keep2(0, 2, if den > 0.0 { d2 / den } else { 0.0 });
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let den = (d4 - d3) + (d5 - d6);
            return self.keep2(1, 2, if den > 0.0 { (d4 - d3) / den } else { 0.0 });
        }
        let sum = va + vb + vc;
        if !(sum > 0.0) || !sum.is_finite() {
            // a degenerate triangle: the best of its edges
            return self.best_edge();
        }
        let v = vb / sum;
        let w = vc / sum;
        self.bary = [1.0 - v - w, v, w, 0.0];
        a + ab * v + ac * w
    }

    fn best_edge(&mut self) -> Vec3 {
        let mut best: Option<(f64, Simplex, Vec3)> = None;
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            let mut s = Simplex::new();
            s.push(self.v[i]);
            s.push(self.v[j]);
            let p = s.solve2();
            let d = p.length_sq();
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, s, p));
            }
        }
        let (_, s, p) = best.unwrap_or((0.0, *self, self.v[0].w));
        *self = s;
        p
    }

    fn solve4(&mut self) -> Vec3 {
        const FACES: [[usize; 4]; 4] = [[0, 1, 2, 3], [0, 3, 1, 2], [0, 2, 3, 1], [1, 3, 2, 0]];
        let mut best: Option<(f64, Simplex, Vec3)> = None;
        let mut inside = true;
        for f in FACES {
            let (pi, pj, pk, pl) = (self.v[f[0]].w, self.v[f[1]].w, self.v[f[2]].w, self.v[f[3]].w);
            let n = (pj - pi).cross(pk - pi);
            let so = -n.dot(pi); // the origin's side
            let sl = n.dot(pl - pi); // the fourth point's side
            let outside = sl == 0.0 || (so != 0.0 && (so > 0.0) != (sl > 0.0));
            if !outside {
                continue;
            }
            inside = false;
            let mut s = Simplex::new();
            s.push(self.v[f[0]]);
            s.push(self.v[f[1]]);
            s.push(self.v[f[2]]);
            let p = s.solve3();
            let d = p.length_sq();
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, s, p));
            }
        }
        if inside {
            self.bary = [0.25; 4];
            return Vec3::ZERO;
        }
        let (_, s, p) = best.unwrap_or((0.0, *self, Vec3::ZERO));
        *self = s;
        p
    }

    /// the closest points on A and B for the current barycentric weights
    pub(crate) fn witness(&self) -> (Vec3, Vec3) {
        let mut a = Vec3::ZERO;
        let mut b = Vec3::ZERO;
        for i in 0..self.n {
            a += self.v[i].a * self.bary[i];
            b += self.v[i].b * self.bary[i];
        }
        (a, b)
    }
}

/// what GJK found between the cores
#[derive(Clone, Copy, Debug)]
pub(crate) struct Gjk {
    /// the distance of the cores (0 when they overlap)
    pub dist: f64,
    /// the closest point of D to the origin (a - b)
    pub v: Vec3,
    pub simplex: Simplex,
    pub overlap: bool,
}

#[inline]
fn support(a: &Placed, b: &Placed, d: Vec3) -> Sv {
    let sa = a.support(d);
    let sb = b.support(-d);
    Sv { w: sa - sb, a: sa, b: sb }
}

/// GJK: the distance between the cores of a and b (both in the same frame)
pub(crate) fn distance(a: &Placed, b: &Placed) -> Gjk {
    let mut d0 = a.center() - b.center();
    if !(d0.length_sq() > 0.0) {
        d0 = Vec3::X;
    }
    let mut s = Simplex::new();
    s.push(support(a, b, -d0));
    let mut v = s.closest();
    let mut vv = v.dot(v);
    let mut overlap = vv <= TOUCH * TOUCH;
    let mut iter = 0;
    while !overlap && iter < MAX_ITERS {
        iter += 1;
        let w = support(a, b, -v);
        if vv - v.dot(w.w) <= REL * vv || s.contains(w.w) {
            break;
        }
        let prev = (s, v, vv);
        s.push(w);
        let nv = s.closest();
        if s.n == 4 {
            overlap = true;
            v = Vec3::ZERO;
            break;
        }
        let nvv = nv.dot(nv);
        if nvv <= TOUCH * TOUCH {
            overlap = true;
            v = nv;
            break;
        }
        if nvv >= vv {
            // no progress (rounding): keep the previous simplex
            (s, v, vv) = prev;
            break;
        }
        v = nv;
        vv = nvv;
    }
    Gjk { dist: if overlap { 0.0 } else { crate::math::sqrt(vv) }, v, simplex: s, overlap }
}

/// how the cores overlap: the direction from B towards A that separates them soonest, the depth along it
/// and the deepest points of the cores
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pen {
    pub normal: Vec3,
    pub depth: f64,
    pub pa: Vec3,
    pub pb: Vec3,
}

const EPA_MAX_VERTS: usize = 64;
const EPA_TOL: f64 = 1e-10;

#[derive(Clone, Copy)]
struct EFace {
    i: [usize; 3],
    n: Vec3,
    d: f64,
}

fn eface(v: &[Sv], i: [usize; 3]) -> EFace {
    let (a, b, c) = (v[i[0]].w, v[i[1]].w, v[i[2]].w);
    match (b - a).cross(c - a).try_normalize() {
        Some(n) => EFace { i, n, d: n.dot(a) },
        None => EFace { i, n: Vec3::ZERO, d: f64::INFINITY },
    }
}

/// the barycentric weights of the point of triangle (a, b, c) closest to p
pub(crate) fn tri_bary(a: Vec3, b: Vec3, c: Vec3, p: Vec3) -> [f64; 3] {
    let mut s = Simplex::new();
    for q in [a, b, c] {
        s.push(Sv { w: q - p, a: q, b: Vec3::ZERO });
    }
    // remember which input each kept vertex was
    let ids = [a, b, c];
    let _ = s.solve3();
    let mut out = [0.0; 3];
    for k in 0..s.n {
        for (j, q) in ids.iter().enumerate() {
            if s.v[k].a == *q && out[j] == 0.0 {
                out[j] = s.bary[k];
                break;
            }
        }
    }
    out
}

/// EPA from the simplex GJK ended with (cores overlapping or touching).  `hint` orients the normal when
/// D is flat (a sphere's centre in a triangle's plane): from B towards A along it.
pub(crate) fn epa(a: &Placed, b: &Placed, g: &Gjk, hint: Vec3) -> Pen {
    let mut verts: Vec<Sv> = Vec::with_capacity(EPA_MAX_VERTS);
    verts.extend_from_slice(&g.simplex.v[..g.simplex.n]);
    let tiny = 1e-12 * (1.0 + verts.iter().fold(0.0, |m, s| max(m, s.w.abs().max_elem())));
    // blow the simplex up to a tetrahedron
    if verts.len() == 1 {
        for d in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            let w = support(a, b, d);
            if (w.w - verts[0].w).length() > tiny {
                verts.push(w);
                break;
            }
        }
    }
    if verts.len() == 2 {
        let e = (verts[1].w - verts[0].w).normalize_or_zero();
        let p1 = e.any_perpendicular();
        let p2 = e.cross(p1);
        const CS: [(f64, f64); 6] = [(1.0, 0.0), (0.5, 0.8660254037844386), (-0.5, 0.8660254037844386), (-1.0, 0.0), (-0.5, -0.8660254037844386), (0.5, -0.8660254037844386)];
        for (c, s) in CS {
            let d = p1 * c + p2 * s;
            let w = support(a, b, d);
            let off = w.w - verts[0].w;
            if (off - e * off.dot(e)).length() > tiny {
                verts.push(w);
                break;
            }
        }
    }
    let mut flat_n = hint.normalize_or_zero();
    if verts.len() == 3 {
        let n = (verts[1].w - verts[0].w).cross(verts[2].w - verts[0].w).normalize_or_zero();
        if n != Vec3::ZERO {
            flat_n = n;
            let up = support(a, b, n);
            let down = support(a, b, -n);
            let du = n.dot(up.w - verts[0].w);
            let dd = -n.dot(down.w - verts[0].w);
            if du > tiny || dd > tiny {
                verts.push(if du >= dd { up } else { down });
            }
        }
    }
    if verts.len() < 4 {
        // D is flat (or a segment or a point): the cores leave along its normal, at no depth; from B towards
        // A on the side of the hint
        let mut n = flat_n;
        if n == Vec3::ZERO {
            n = Vec3::Y;
        }
        if n.dot(hint) < 0.0 {
            n = -n;
        }
        let (pa, pb) = g.simplex.witness();
        return Pen { normal: n, depth: 0.0, pa, pb };
    }
    let mut faces: Vec<EFace> = Vec::with_capacity(4 * EPA_MAX_VERTS);
    for f in [[0, 1, 2, 3], [0, 3, 1, 2], [0, 2, 3, 1], [1, 3, 2, 0]] {
        let (i, j, k, l) = (f[0], f[1], f[2], f[3]);
        let n = (verts[j].w - verts[i].w).cross(verts[k].w - verts[i].w);
        let ids = if n.dot(verts[l].w - verts[i].w) > 0.0 { [i, k, j] } else { [i, j, k] };
        faces.push(eface(&verts, ids));
    }
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut best = 0usize;
    for _ in 0..MAX_ITERS {
        best = 0;
        for (i, f) in faces.iter().enumerate() {
            if f.d < faces[best].d {
                best = i;
            }
        }
        let f = faces[best];
        if !f.d.is_finite() {
            break;
        }
        let w = support(a, b, f.n);
        let dw = f.n.dot(w.w);
        if dw - f.d <= EPA_TOL * (1.0 + crate::math::abs(dw)) || verts.len() >= EPA_MAX_VERTS {
            break;
        }
        let wi = verts.len();
        verts.push(w);
        edges.clear();
        let mut removed = false;
        let mut k = 0;
        while k < faces.len() {
            let ff = faces[k];
            let seen = if ff.d.is_finite() { ff.n.dot(w.w - verts[ff.i[0]].w) > tiny } else { false };
            if seen {
                removed = true;
                for e in 0..3 {
                    let (p, q) = (ff.i[e], ff.i[(e + 1) % 3]);
                    if let Some(at) = edges.iter().position(|&(x, y)| x == q && y == p) {
                        edges.swap_remove(at);
                    } else {
                        edges.push((p, q));
                    }
                }
                faces.swap_remove(k);
            } else {
                k += 1;
            }
        }
        if !removed {
            verts.pop();
            break;
        }
        for &(p, q) in &edges {
            faces.push(eface(&verts, [p, q, wi]));
        }
        if faces.is_empty() {
            break;
        }
    }
    if faces.is_empty() {
        let (pa, pb) = g.simplex.witness();
        return Pen { normal: if hint == Vec3::ZERO { Vec3::Y } else { hint.normalize_or_zero() }, depth: 0.0, pa, pb };
    }
    let f = faces[best.min(faces.len() - 1)];
    let depth = max(f.d, 0.0);
    let (va, vb, vc) = (verts[f.i[0]], verts[f.i[1]], verts[f.i[2]]);
    let bc = tri_bary(va.w, vb.w, vc.w, f.n * f.d);
    let pa = va.a * bc[0] + vb.a * bc[1] + vc.a * bc[2];
    let pb = va.b * bc[0] + vb.b * bc[1] + vc.b * bc[2];
    // D's surface is nearest the origin along n: A leaves along -n
    Pen { normal: -f.n, depth, pa, pb }
}

/// the signed distance of the shapes (with margins): negative = penetration depth
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sep {
    pub dist: f64,
    /// from B towards A
    pub normal: Vec3,
    /// the surface points of A and B (for overlap: the deepest points)
    pub pa: Vec3,
    pub pb: Vec3,
}

/// distance or penetration of a and b, margins included
pub(crate) fn separation(a: &Placed, b: &Placed, hint: Vec3) -> Sep {
    let g = distance(a, b);
    let r = a.margin + b.margin;
    if !g.overlap && g.dist > 0.0 {
        let n = g.v / g.dist;
        let (ca, cb) = g.simplex.witness();
        return Sep { dist: g.dist - r, normal: n, pa: ca - n * a.margin, pb: cb + n * b.margin };
    }
    let p = epa(a, b, &g, hint);
    Sep { dist: -(p.depth + r), normal: p.normal, pa: p.pa - p.normal * a.margin, pb: p.pb + p.normal * b.margin }
}

/// the outcome of a cast of `a` by `r` against `b`
#[derive(Clone, Copy, Debug)]
pub(crate) enum Cast {
    Miss,
    /// they touch or overlap at the start (the caller decides with the contact's normal)
    Start,
    /// first contact at `lambda` (0..=1): the normal from B towards A, B's core point there
    Hit { lambda: f64, normal: Vec3, pb: Vec3 },
}

/// how often a cast fell back to Newton's method on the distance (for the tests and tuning)
static FALLBACKS: AtomicU32 = AtomicU32::new(0);

/// how many casts so far fell back from GJK ray casting to Newton's method on the distance (GJK at one time
/// stalled by rounding, or its iterations ran out); the result is right either way, only slower
pub fn cast_fallbacks() -> u32 {
    FALLBACKS.load(Ordering::Relaxed)
}

/// GJK ray casting with conservative advancement: the first time a + lambda r comes within the target gap of b
pub(crate) fn cast(a: &Placed, b: &Placed, r: Vec3) -> Cast {
    let radius = a.margin + b.margin;
    let sigma = max(radius, CAST_SKIN);
    let mut lambda = 0.0f64;
    let mut s = Simplex::new();
    let mut v = a.support(-r) - b.support(r);
    let mut n = Vec3::ZERO;
    let mut iter = 0u32;
    let mut last_vv = f64::INFINITY;
    let mut converged = false;
    while iter < MAX_ITERS {
        let vv = v.dot(v);
        let vl = crate::math::sqrt(vv);
        if vl - sigma <= CAST_TOL {
            converged = true;
            break;
        }
        let wa = a.support(-v);
        let wb = b.support(v);
        let p = wa - wb;
        let vn = v / vl;
        let vp = vn.dot(p);
        let vr = vn.dot(r);
        let mut advanced = false;
        if vp + lambda * vr > sigma {
            if vr >= 0.0 {
                return Cast::Miss;
            }
            let l = (vp - sigma) / -vr;
            if l > 1.0 {
                return Cast::Miss;
            }
            // a step forward starts a new simplex at the new time; a plane that allows no step (rounding)
            // changes nothing, GJK goes on at this time
            if l > lambda {
                lambda = l;
                advanced = true;
                n = vn;
                s.n = 0;
                last_vv = f64::INFINITY;
            }
        }
        let al = wa + r * lambda;
        let w = al - wb;
        if !advanced && s.contains(w) {
            break; // nothing new at this time
        }
        s.push(Sv { w, a: al, b: wb });
        v = s.closest();
        iter += 1;
        if s.n == 4 {
            converged = true; // the cores overlap at lambda (rounding): a contact without a closest direction
            break;
        }
        let nvv = v.dot(v);
        if !advanced && nvv >= last_vv {
            break; // GJK at a fixed time made no progress
        }
        last_vv = nvv;
    }
    if !converged {
        FALLBACKS.fetch_add(1, Ordering::Relaxed);
        return cast_newton(a, b, r, sigma, lambda);
    }
    if lambda == 0.0 {
        return Cast::Start;
    }
    let (_, pb) = s.witness();
    let normal = if s.n < 4 { v.try_normalize().unwrap_or(n) } else { n };
    Cast::Hit { lambda, normal, pb }
}

/// the slow and sure way from `lambda` on: the gap of two convex shapes along a straight motion is a convex
/// function of time whose slope is normal . r, so Newton's steps from below never pass the first contact
/// and converge quadratically (each step a full GJK distance)
fn cast_newton(a: &Placed, b: &Placed, r: Vec3, sigma: f64, mut lambda: f64) -> Cast {
    let mut last: Option<(Vec3, Vec3)> = None;
    for _ in 0..MAX_ITERS {
        let at = Placed { pos: a.pos + r * lambda, ..*a };
        let g = distance(&at, b);
        if g.overlap || g.dist - sigma <= CAST_TOL {
            if lambda == 0.0 {
                return Cast::Start;
            }
            let (_, pb) = g.simplex.witness();
            let normal = if g.overlap { last.map(|l| l.0).unwrap_or(-r.normalize_or_zero()) } else { g.v / g.dist };
            return Cast::Hit { lambda, normal, pb };
        }
        let nrm = g.v / g.dist;
        let slope = nrm.dot(r);
        if slope >= 0.0 {
            return Cast::Miss;
        }
        let next = lambda + (g.dist - sigma) / -slope;
        if next > 1.0 {
            return Cast::Miss;
        }
        if !(next > lambda) {
            break; // no step left (rounding): as close as it gets
        }
        lambda = next;
        last = Some((nrm, g.simplex.witness().1));
    }
    match last {
        Some((normal, pb)) => Cast::Hit { lambda, normal, pb },
        None => Cast::Start,
    }
}
