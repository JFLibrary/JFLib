// manifold -- the contact points between two bodies for the solver: up to four per normal
//
// GJK / EPA give one normal and one pair of nearest (or deepest) points.  A box resting on a face needs its
// four corners, or it rocks; so, as in Jolt, the face of each shape that looks most along the normal (a box's
// face, a hull's polygon of coplanar triangles, a triangle of a mesh, a capsule's axis lying flat, a point
// otherwise) is taken, one face is clipped against the side planes of the other, projected along the normal
// (Sutherland-Hodgman), and every clipped point closer than the speculative distance is a contact, its gap
// measured along the normal to the other face's plane.  The face more parallel to the normal is the one the
// other is clipped against.  Clipping that leaves nothing (a corner, a sphere) gives the one point of GJK.
// More than four points are reduced to the four that span the largest area, the deepest first.
//
// A convex body against a mesh: a manifold per triangle near it, with the triangle's own normal on an
// inactive edge (mesh.rs), then the manifolds of triangles with the same normal (within 2.6 degrees) are one
// (a box across the seam of two floor triangles has one manifold of four points, not two of three).
// Everything runs in the frame of the second body, by triangle index and in a fixed order: the same
// points on every target.

use crate::convex::{Core, Placed};
use crate::gjk::{self, Sep};
use crate::math::{Iso, Vec3, abs};
use crate::mesh::TriMesh;
use crate::query::NO_TRIANGLE;
use crate::shape::Shape;
use alloc::boxed::Box;
use alloc::vec::Vec;

/// at most this many points in a manifold
pub(crate) const MAX_POINTS: usize = 4;

/// the vertices of a face beyond which a hull's face falls back to its triangle
const MAXF: usize = 64;

/// a capsule's axis gives both its ends unless it stands along the normal within 2.6 degrees (the end that
/// is farther than the speculative distance is dropped with the clipped points): a tilted capsule that
/// tips over in a step has a point at its upper end too
const SEGMENT_ALONG: f64 = 0.999;

/// two capsule axes are parallel enough for two points when the cosine between them is at least this
const SEGMENT_PARALLEL: f64 = 0.998;

/// the manifolds of two triangles are one when their normals are within this cosine (2.6 degrees)
const MERGE_COS: f64 = 0.999;

/// a contact point: where (world, midway between the two surfaces) and the gap along the normal
/// (negative: penetration)
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MPoint {
    pub point: Vec3,
    pub separation: f64,
}

/// the contact points of a pair along one normal
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geom {
    /// unit, world, from the first body towards the second
    pub normal: Vec3,
    pub points: [MPoint; MAX_POINTS],
    pub count: usize,
    /// NO_TRIANGLE for two convex shapes; with a mesh the lowest triangle index of the manifold
    pub key: u32,
}

/// a point on both surfaces and on both cores in the frame of the second body
#[derive(Clone, Copy, Debug)]
struct Raw {
    pa: Vec3,
    pb: Vec3,
    ca: Vec3,
    cb: Vec3,
    s: f64,
}

impl Raw {
    /// from GJK / EPA's points on the surfaces, with its normal from b towards a
    fn from_sep(sep: &Sep, ma: f64, mb: f64) -> Raw {
        Raw { pa: sep.pa, pb: sep.pb, ca: sep.pa + sep.normal * ma, cb: sep.pb - sep.normal * mb, s: sep.dist }
    }
}

/// a face of a core in the common frame: 1 vertex (a point), 2 (a segment), 3 or more (a polygon,
/// counter-clockwise around its outward normal)
struct Face {
    p: [Vec3; MAXF],
    n: usize,
    normal: Vec3,
}

impl Face {
    const EMPTY: Face = Face { p: [Vec3::ZERO; MAXF], n: 0, normal: Vec3::ZERO };

    fn push(&mut self, v: Vec3) {
        if self.n < MAXF {
            self.p[self.n] = v;
            self.n += 1;
        }
    }
}

#[inline]
fn to_common(pl: &Placed, v: Vec3) -> Vec3 {
    match &pl.rot {
        Some(m) => m.mul_vec(v) + pl.pos,
        None => v + pl.pos,
    }
}

#[inline]
fn dir_to_common(pl: &Placed, v: Vec3) -> Vec3 {
    match &pl.rot {
        Some(m) => m.mul_vec(v),
        None => v,
    }
}

/// the buffers of the manifolds: made once for a step and used for every pair, so no large array is cleared
/// or copied per pair
pub(crate) struct Scratch {
    fa: Face,
    fb: Face,
    clip: [[Vec3; CLIP_MAX]; 2],
    local: [Vec3; MAXF],
    raw: Vec<Raw>,
    near: Vec<(u32, [Vec3; 3], Vec3, Sep)>,
    /// (normal, lowest triangle, points) per normal of a mesh pair; the first `groups_used` are this pair's
    groups: Vec<(Vec3, u32, Vec<Raw>)>,
}

impl Scratch {
    pub(crate) fn new() -> Box<Scratch> {
        Box::new(Scratch {
            fa: Face::EMPTY,
            fb: Face::EMPTY,
            clip: [[Vec3::ZERO; CLIP_MAX]; 2],
            local: [Vec3::ZERO; MAXF],
            raw: Vec::new(),
            near: Vec::new(),
            groups: Vec::new(),
        })
    }
}

/// the face of `pl` that looks most along `dir` (a unit vector in the common frame), into `f`
fn face_of(pl: &Placed, dir: Vec3, f: &mut Face, local: &mut [Vec3; MAXF]) {
    f.n = 0;
    f.normal = Vec3::ZERO;
    let d = match &pl.rot {
        Some(m) => m.mul_vec_t(dir),
        None => dir,
    };
    match pl.core {
        Core::Point => f.push(pl.pos),
        Core::Segment(h) => {
            let u = dir_to_common(pl, Vec3::Y);
            let (e0, e1) = (pl.pos - u * h, pl.pos + u * h);
            let c = u.dot(dir);
            if abs(c) < SEGMENT_ALONG {
                f.push(e0);
                f.push(e1);
            } else {
                f.push(if c > 0.0 { e1 } else { e0 });
            }
        }
        Core::Box(h) => {
            let a = d.abs();
            let i = if a.x >= a.y && a.x >= a.z {
                0
            } else if a.y >= a.z {
                1
            } else {
                2
            };
            let s = if d[i] >= 0.0 { 1.0 } else { -1.0 };
            let (mut j, mut k) = ((i + 1) % 3, (i + 2) % 3);
            if s < 0.0 {
                core::mem::swap(&mut j, &mut k);
            }
            let axis = |n: usize| match n {
                0 => Vec3::X,
                1 => Vec3::Y,
                _ => Vec3::Z,
            };
            let (ei, ej, ek) = (axis(i), axis(j), axis(k));
            let c = ei * (s * h[i]);
            let (u, v) = (ej * h[j], ek * h[k]);
            for (su, sv) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
                f.push(to_common(pl, c + u * su + v * sv));
            }
            f.normal = dir_to_common(pl, ei * s);
        }
        Core::Hull(hull) => {
            let planes = hull.planes();
            let (mut best, mut bi) = (f64::NEG_INFINITY, 0usize);
            for (i, (n, _)) in planes.iter().enumerate() {
                let v = n.dot(d);
                if v > best {
                    best = v;
                    bi = i;
                }
            }
            let (n, pd) = planes[bi];
            let pts = hull.points();
            let ext = hull.aabb().half_extents().max_elem();
            let tol = 2.0 * hull.tolerance() + 1e-9 * ext;
            let mut on = 0usize;
            for p in pts {
                if n.dot(*p) >= pd - tol {
                    on += 1;
                }
            }
            if on <= 3 || on > MAXF {
                let t = hull.faces()[bi];
                for &v in &t {
                    f.push(to_common(pl, pts[v as usize]));
                }
            } else {
                let mut m = 0;
                for p in pts {
                    if n.dot(*p) >= pd - tol {
                        local[m] = *p;
                        m += 1;
                    }
                }
                sort_around(&mut local[..m], n);
                for p in &local[..m] {
                    f.push(to_common(pl, *p));
                }
            }
            f.normal = dir_to_common(pl, n);
        }
        Core::Triangle(t) => {
            let (a, b, c) = (to_common(pl, t[0]), to_common(pl, t[1]), to_common(pl, t[2]));
            let fnorm = (b - a).cross(c - a).normalize_or_zero();
            if fnorm.dot(dir) >= 0.0 {
                f.push(a);
                f.push(b);
                f.push(c);
                f.normal = fnorm;
            } else {
                f.push(a);
                f.push(c);
                f.push(b);
                f.normal = -fnorm;
            }
        }
    }
}

/// the points of a convex polygon (on a plane with unit normal n) counter-clockwise around n: by half plane,
/// then by the sign of the cross product (no angles, the same order on every target)
fn sort_around(p: &mut [Vec3], n: Vec3) {
    let mut c = Vec3::ZERO;
    for q in p.iter() {
        c += *q;
    }
    c = c / p.len() as f64;
    let u = (p[0] - c).try_normalize().unwrap_or_else(|| n.any_perpendicular());
    let w = n.cross(u);
    let key = |q: Vec3| {
        let r = q - c;
        let (x, y) = (r.dot(u), r.dot(w));
        let half = if y > 0.0 || (y == 0.0 && x >= 0.0) { 0u8 } else { 1u8 };
        (half, x, y)
    };
    let before = |a: Vec3, b: Vec3| {
        let (ha, xa, ya) = key(a);
        let (hb, xb, yb) = key(b);
        ha < hb || (ha == hb && xa * yb - ya * xb > 0.0)
    };
    for i in 1..p.len() {
        let mut j = i;
        while j > 0 && before(p[j], p[j - 1]) {
            p.swap(j, j - 1);
            j -= 1;
        }
    }
}

const CLIP_MAX: usize = 2 * MAXF + 8;

/// clip the closed polygon `poly` (or the segment, `closed` false) to the half space side . (x - r0) <= 0
fn clip(poly: &[Vec3], closed: bool, r0: Vec3, side: Vec3, out: &mut [Vec3; CLIP_MAX]) -> usize {
    let mut m = 0;
    let n = poly.len();
    if n == 0 {
        return 0;
    }
    let dist = |p: Vec3| side.dot(p - r0);
    if !closed {
        // a segment
        let (a, b) = (poly[0], poly[n - 1]);
        let (da, db) = (dist(a), dist(b));
        if da > 0.0 && db > 0.0 {
            return 0;
        }
        let cut = |p: Vec3, q: Vec3, dp: f64, dq: f64| p + (q - p) * (dp / (dp - dq));
        out[0] = if da > 0.0 { cut(a, b, da, db) } else { a };
        out[1] = if db > 0.0 { cut(b, a, db, da) } else { b };
        return 2;
    }
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (dp, dq) = (dist(p), dist(q));
        if dp <= 0.0 {
            if m < CLIP_MAX {
                out[m] = p;
                m += 1;
            }
        }
        if (dp <= 0.0) != (dq <= 0.0) && m < CLIP_MAX {
            out[m] = p + (q - p) * (dp / (dp - dq));
            m += 1;
        }
    }
    m
}

/// the face-to-face points of a and b (common frame) along n_ab (unit, from a towards b), gaps up to `spec`;
/// false when the faces give none (the caller takes GJK's point)
fn faces(a: &Placed, b: &Placed, n_ab: Vec3, spec: f64, sc: &mut Scratch, out: &mut Vec<Raw>) -> bool {
    let Scratch { fa, fb, clip: buf, local, .. } = sc;
    face_of(a, n_ab, fa, local);
    face_of(b, -n_ab, fb, local);
    let (fa, fb) = (&*fa, &*fb);
    let (ma, mb) = (a.margin, b.margin);
    let start = out.len();
    if fa.n == 2 && fb.n == 2 {
        // two capsules lying side by side
        let (a0, a1) = (fa.p[0], fa.p[1]);
        let (b0, b1) = (fb.p[0], fb.p[1]);
        let (ua, la) = match (a1 - a0).try_normalize() {
            Some(u) => (u, (a1 - a0).length()),
            None => return false,
        };
        let Some(ub) = (b1 - b0).try_normalize() else {
            return false;
        };
        if abs(ua.dot(ub)) < SEGMENT_PARALLEL {
            return false;
        }
        let buf = &mut buf[0];
        let mut seg = [b0, b1];
        let k = clip(&seg, false, a0, -ua, buf);
        if k < 2 {
            return false;
        }
        seg = [buf[0], buf[1]];
        let k = clip(&seg, false, a1, ua, buf);
        if k < 2 {
            return false;
        }
        // the axes overlap in one point only: one contact, not the same twice
        let pts: &[Vec3] = if (buf[1] - buf[0]).length_sq() > 0.0 { &buf[..2] } else { &buf[..1] };
        for p in pts {
            let qa = a0 + ua * crate::math::clamp((*p - a0).dot(ua), 0.0, la);
            let g = (*p - qa).dot(n_ab);
            let s = g - ma - mb;
            if s <= spec {
                out.push(Raw { pa: qa + n_ab * ma, pb: *p - n_ab * mb, ca: qa, cb: *p, s });
            }
        }
        return out.len() > start;
    }
    // the reference face: a polygon, the one more parallel to the normal
    let a_ref = if fa.n >= 3 && fb.n >= 3 {
        abs(fa.normal.dot(n_ab)) >= abs(fb.normal.dot(n_ab))
    } else if fa.n >= 3 {
        true
    } else if fb.n >= 3 {
        false
    } else {
        return false;
    };
    let (rf, inc, nref, m_ref, m_inc) = if a_ref { (fa, fb, n_ab, ma, mb) } else { (fb, fa, -n_ab, mb, ma) };
    if inc.n < 2 {
        return false;
    }
    let along = rf.normal.dot(nref);
    if !(along > 1e-3) {
        return false;
    }
    let mut cur = 0usize;
    let mut len = inc.n;
    buf[0][..len].copy_from_slice(&inc.p[..len]);
    let closed = inc.n >= 3;
    for i in 0..rf.n {
        let (r0, r1) = (rf.p[i], rf.p[(i + 1) % rf.n]);
        let side = (r1 - r0).cross(nref);
        if !(side.length_sq() > 0.0) {
            continue;
        }
        let (src, dst) = if cur == 0 {
            let (x, y) = buf.split_at_mut(1);
            (&x[0], &mut y[0])
        } else {
            let (x, y) = buf.split_at_mut(1);
            (&y[0], &mut x[0])
        };
        len = clip(&src[..len], closed, r0, side, dst);
        cur ^= 1;
        if len == 0 {
            return false;
        }
    }
    let r0 = rf.p[0];
    for p in &buf[cur][..len] {
        let g = (*p - r0).dot(rf.normal) / along;
        let s = g - ma - mb;
        if s <= spec {
            let q = *p - nref * g;
            let (p_ref, p_inc) = (q + nref * m_ref, *p - nref * m_inc);
            out.push(if a_ref { Raw { pa: p_ref, pb: p_inc, ca: q, cb: *p, s } } else { Raw { pa: p_inc, pb: p_ref, ca: *p, cb: q, s } });
        }
    }
    out.len() > start
}

/// keep the four points that span the most: the deepest, the farthest from it, the one making the largest
/// triangle with these, the one adding the most area to that triangle
fn reduce(pts: &mut Vec<Raw>, n: Vec3) {
    if pts.len() <= MAX_POINTS {
        return;
    }
    let mid = |r: &Raw| (r.pa + r.pb) * 0.5;
    let mut pick = [0usize; MAX_POINTS];
    let mut k = 1;
    for (i, r) in pts.iter().enumerate() {
        if r.s < pts[pick[0]].s {
            pick[0] = i;
        }
    }
    let m0 = mid(&pts[pick[0]]);
    let (mut best, mut bi) = (-1.0, usize::MAX);
    for (i, r) in pts.iter().enumerate() {
        let d = (mid(r) - m0).length_sq();
        if i != pick[0] && d > best {
            best = d;
            bi = i;
        }
    }
    if bi == usize::MAX || !(best > 0.0) {
        pts.truncate(1);
        return;
    }
    pick[1] = bi;
    k = k.max(2);
    let m1 = mid(&pts[pick[1]]);
    let area = |a: Vec3, b: Vec3, p: Vec3| (b - a).cross(p - a).dot(n);
    let (mut best, mut bi) = (0.0, usize::MAX);
    for (i, r) in pts.iter().enumerate() {
        let a = abs(area(m0, m1, mid(r)));
        if i != pick[0] && i != pick[1] && a > best {
            best = a;
            bi = i;
        }
    }
    if bi != usize::MAX {
        pick[2] = bi;
        k = 3;
        let m2 = mid(&pts[bi]);
        let sigma = if area(m0, m1, m2) > 0.0 { 1.0 } else { -1.0 };
        let (mut best, mut bi) = (0.0, usize::MAX);
        for (i, r) in pts.iter().enumerate() {
            if pick[..3].contains(&i) {
                continue;
            }
            let p = mid(r);
            let gain = [(m0, m1), (m1, m2), (m2, m0)].iter().map(|&(x, y)| -sigma * area(x, y, p)).fold(0.0, crate::math::max);
            if gain > best {
                best = gain;
                bi = i;
            }
        }
        if bi != usize::MAX {
            pick[3] = bi;
            k = 4;
        }
    }
    let kept = [pts[pick[0]], pts[pick[1]], pts[pick[2]], pts[pick[3]]];
    pts.clear();
    pts.extend_from_slice(&kept[..k]);
}

fn geom(raw: &[Raw], n_ab: Vec3, iso_b: &Iso, key: u32) -> Geom {
    let mut g = Geom { normal: iso_b.transform_vector(n_ab), points: [MPoint::default(); MAX_POINTS], count: 0, key };
    for r in raw.iter().take(MAX_POINTS) {
        g.points[g.count] = MPoint { point: iso_b.transform_point((r.pa + r.pb) * 0.5), separation: r.s };
        g.count += 1;
    }
    g
}

/// the convex query shape in the frame of the second body (as query.rs places it)
fn placed_in<'s>(shape: &'s Shape, iso: &Iso, iso_b: &Iso) -> Option<Placed<'s>> {
    let rel = iso_b.inv_mul(iso);
    let rot = if rel.rot.is_identity() { None } else { Some(rel.rot.to_mat3()) };
    shape.placed_rot(rot, rel.pos)
}

/// the manifold of two convex shapes whose gap is at most `spec`
pub(crate) fn convex_pair(sa: &Shape, ia: &Iso, sb: &Shape, ib: &Iso, spec: f64, sc: &mut Scratch) -> Option<Geom> {
    let a = placed_in(sa, ia, ib)?;
    let b = sb.placed_rot(None, Vec3::ZERO)?;
    let sep = gjk::separation(&a, &b, a.center() - b.center());
    if !(sep.dist <= spec) {
        return None;
    }
    let n_ab = -sep.normal;
    let mut raw = core::mem::take(&mut sc.raw);
    raw.clear();
    if !faces(&a, &b, n_ab, spec, sc, &mut raw) {
        raw.push(Raw::from_sep(&sep, a.margin, b.margin));
    }
    reduce(&mut raw, n_ab);
    let g = geom(&raw, n_ab, ib, NO_TRIANGLE);
    sc.raw = raw;
    Some(g)
}

/// the manifolds of convex `sa` against a mesh: per normal (triangles merged), sorted by their lowest
/// triangle index
pub(crate) fn mesh_pair(sa: &Shape, ia: &Iso, mesh: &TriMesh, ib: &Iso, spec: f64, sc: &mut Scratch, out: &mut Vec<Geom>) {
    let Some(a) = placed_in(sa, ia, ib) else {
        return;
    };
    let mut near = core::mem::take(&mut sc.near);
    near.clear();
    mesh.near_triangles(&a, spec, |id, tri, n, s| near.push((id, tri, n, s)));
    near.sort_unstable_by_key(|t| t.0);
    let mut groups = core::mem::take(&mut sc.groups);
    let mut used = 0usize;
    let mut raw = core::mem::take(&mut sc.raw);
    for (id, tri, n_tri, sep) in &near {
        let n_ab = -*n_tri;
        let b = Placed::new(Core::Triangle(*tri), 0.0, None, Vec3::ZERO);
        raw.clear();
        if !faces(&a, &b, n_ab, spec, sc, &mut raw) {
            if !(sep.dist <= spec) {
                continue;
            }
            raw.push(Raw::from_sep(sep, a.margin, 0.0));
        }
        match groups[..used].iter_mut().find(|g| g.0.dot(n_ab) >= MERGE_COS) {
            Some(g) => {
                if g.0 != n_ab {
                    // the points of a triangle of a slightly other normal, along the group's: on the line
                    // through the convex's core point (a sphere's centre) along it, with the gap along it, so
                    // the solver finds the core where it is and pushes a sphere through its centre
                    let ng = g.0;
                    for r in raw.iter_mut() {
                        let d = (r.cb - r.ca).dot(ng);
                        *r = Raw { pa: r.ca + ng * a.margin, pb: r.ca + ng * d, ca: r.ca, cb: r.ca + ng * d, s: d - a.margin };
                    }
                }
                g.2.extend_from_slice(&raw);
            }
            None => {
                if used == groups.len() {
                    groups.push((n_ab, *id, Vec::new()));
                }
                let g = &mut groups[used];
                g.0 = n_ab;
                g.1 = *id;
                g.2.clear();
                g.2.extend_from_slice(&raw);
                used += 1;
            }
        }
    }
    for g in &mut groups[..used] {
        reduce(&mut g.2, g.0);
        out.push(geom(&g.2, g.0, ib, g.1));
    }
    sc.near = near;
    sc.groups = groups;
    sc.raw = raw;
}
