// refs -- reference geometry that shares no code with lib/physics
//
// Closest points by projection onto the plane and the minimum over the edges, segment pairs by Ericson's
// clamped solution, segment/triangle crossings by Moeller-Trumbore.  A convex shape is a `Poly`: its core's
// vertices, face normals, edges and triangles, and a radius.  The signed distance of two of them: when they
// do not intersect, the minimum over every feature pair (vertex-triangle, edge-edge, vertex-edge,
// vertex-vertex), exact for separated convex polytopes; when they intersect (an edge crosses a triangle,
// a vertex lies inside, or features touch), the penetration from the separating axes (face normals of
// both, cross products of their edges), exact for convex polytopes.  The gap along a straight motion is a
// convex function of time, so its minimum comes from a ternary search and the first contact from
// bisection before it.

use alloc::vec;
use alloc::vec::Vec;
use physics::{ConvexHull, Iso, Shape, Vec3};

pub fn clamp01(x: f64) -> f64 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

pub fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let l = ab.length_sq();
    if l == 0.0 {
        return a;
    }
    a + ab * clamp01((p - a).dot(ab) / l)
}

/// the closest points of segments p1q1 and p2q2 (Ericson 5.1.9)
pub fn seg_seg(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let eps = 1e-300;
    let (s, t);
    if a <= eps && e <= eps {
        return (p1, p2);
    }
    if a <= eps {
        s = 0.0;
        t = clamp01(f / e);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = clamp01(-c / a);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            let mut ss = if den > 0.0 { clamp01((b * f - c * e) / den) } else { 0.0 };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 {
                tt = 0.0;
                ss = clamp01(-c / a);
            } else if tt > 1.0 {
                tt = 1.0;
                ss = clamp01((b - c) / a);
            }
            s = ss;
            t = tt;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// the closest point of triangle abc to p: the projection when it falls inside, else the nearest edge point
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let n = (b - a).cross(c - a);
    let nn = n.length_sq();
    if nn > 0.0 {
        let q = p - n * ((p - a).dot(n) / nn);
        let e0 = (b - a).cross(q - a).dot(n);
        let e1 = (c - b).cross(q - b).dot(n);
        let e2 = (a - c).cross(q - c).dot(n);
        if e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 {
            return q;
        }
    }
    let cands = [closest_on_segment(p, a, b), closest_on_segment(p, b, c), closest_on_segment(p, c, a)];
    let mut best = cands[0];
    for q in &cands[1..] {
        if (*q - p).length_sq() < (best - p).length_sq() {
            best = *q;
        }
    }
    best
}

/// Moeller-Trumbore: the parameter t of o + t d on triangle abc (any t), None when parallel or outside
pub fn ray_tri_mt(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f64, f64, f64)> {
    let e1 = b - a;
    let e2 = c - a;
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - a;
    let u = tv.dot(pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some((e2.dot(qv) * inv, u, v))
}

/// does segment pq cross triangle abc
pub fn seg_crosses_tri(p: Vec3, q: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    match ray_tri_mt(p, q - p, a, b, c) {
        Some((t, _, _)) => (0.0..=1.0).contains(&t),
        None => false,
    }
}

/// a convex core with a radius, as polytope features
#[derive(Clone, Debug)]
pub struct Poly {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub edges: Vec<(Vec3, Vec3)>,
    pub tris: Vec<[Vec3; 3]>,
    /// the planes of a solid core (n . x <= d inside); empty for points, segments and triangles
    pub planes: Vec<(Vec3, f64)>,
    pub radius: f64,
}

impl Poly {
    pub fn point(p: Vec3, r: f64) -> Poly {
        Poly { verts: vec![p], normals: vec![], edges: vec![], tris: vec![], planes: vec![], radius: r }
    }

    pub fn segment(p: Vec3, q: Vec3, r: f64) -> Poly {
        Poly { verts: vec![p, q], normals: vec![], edges: vec![(p, q)], tris: vec![], planes: vec![], radius: r }
    }

    pub fn triangle(a: Vec3, b: Vec3, c: Vec3) -> Poly {
        let n = (b - a).cross(c - a).normalize_or_zero();
        Poly { verts: vec![a, b, c], normals: vec![n], edges: vec![(a, b), (b, c), (c, a)], tris: vec![[a, b, c]], planes: vec![], radius: 0.0 }
    }

    /// a box (half sizes h) placed by iso
    pub fn cuboid(h: Vec3, iso: &Iso) -> Poly {
        let mut verts = Vec::new();
        for k in 0..8 {
            let p = Vec3::new(if k & 1 != 0 { h.x } else { -h.x }, if k & 2 != 0 { h.y } else { -h.y }, if k & 4 != 0 { h.z } else { -h.z });
            verts.push(iso.transform_point(p));
        }
        let ax = [iso.transform_vector(Vec3::X), iso.transform_vector(Vec3::Y), iso.transform_vector(Vec3::Z)];
        let mut edges = Vec::new();
        for i in 0..8usize {
            for bit in [1usize, 2, 4] {
                if i & bit == 0 {
                    edges.push((verts[i], verts[i | bit]));
                }
            }
        }
        // the 12 triangles: two per face
        let faces: [[usize; 4]; 6] = [[0, 2, 6, 4], [1, 5, 7, 3], [0, 4, 5, 1], [2, 3, 7, 6], [0, 1, 3, 2], [4, 6, 7, 5]];
        let mut tris = Vec::new();
        for f in faces {
            tris.push([verts[f[0]], verts[f[1]], verts[f[2]]]);
            tris.push([verts[f[0]], verts[f[2]], verts[f[3]]]);
        }
        let c = iso.pos;
        let mut planes = Vec::new();
        for (a, hv) in ax.iter().zip([h.x, h.y, h.z]) {
            planes.push((*a, a.dot(c) + hv));
            planes.push((-*a, -a.dot(c) + hv));
        }
        Poly { verts, normals: ax.to_vec(), edges, tris, planes, radius: 0.0 }
    }

    /// a hull placed by iso (its faces from lib/physics' quickhull; the hull itself is checked separately)
    pub fn hull(h: &ConvexHull, iso: &Iso) -> Poly {
        let verts: Vec<Vec3> = h.points().iter().map(|p| iso.transform_point(*p)).collect();
        let mut edges = Vec::new();
        let mut tris = Vec::new();
        let mut normals = Vec::new();
        let mut planes = Vec::new();
        let mut seen: Vec<(u32, u32)> = Vec::new();
        for f in h.faces() {
            let t = [verts[f[0] as usize], verts[f[1] as usize], verts[f[2] as usize]];
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            normals.push(n);
            planes.push((n, n.dot(t[0])));
            tris.push(t);
            for k in 0..3 {
                let (a, b) = (f[k].min(f[(k + 1) % 3]), f[k].max(f[(k + 1) % 3]));
                if !seen.contains(&(a, b)) {
                    seen.push((a, b));
                    edges.push((verts[a as usize], verts[b as usize]));
                }
            }
        }
        Poly { verts, normals, edges, tris, planes, radius: 0.0 }
    }

    /// any convex shape of lib/physics, placed
    pub fn of(shape: &Shape, iso: &Iso) -> Poly {
        match shape {
            Shape::Sphere { radius } => Poly::point(iso.pos, *radius),
            Shape::Capsule { half_height, radius } => Poly::segment(
                iso.transform_point(Vec3::new(0.0, -*half_height, 0.0)),
                iso.transform_point(Vec3::new(0.0, *half_height, 0.0)),
                *radius,
            ),
            Shape::Box { half } => Poly::cuboid(*half, iso),
            Shape::Hull(h) => Poly::hull(h, iso),
            Shape::Mesh(_) => panic!("a mesh is no Poly"),
        }
    }

    pub fn moved(&self, v: Vec3) -> Poly {
        let mut p = self.clone();
        for q in &mut p.verts {
            *q += v;
        }
        for e in &mut p.edges {
            e.0 += v;
            e.1 += v;
        }
        for t in &mut p.tris {
            for q in t.iter_mut() {
                *q += v;
            }
        }
        for pl in &mut p.planes {
            pl.1 += pl.0.dot(v);
        }
        p
    }

    fn contains(&self, p: Vec3) -> bool {
        !self.planes.is_empty() && self.planes.iter().all(|(n, d)| n.dot(p) <= *d)
    }
}

/// the minimum distance over every feature pair (exact when the cores do not intersect)
fn brute(a: &Poly, b: &Poly) -> f64 {
    let mut best = f64::INFINITY;
    let mut take = |d2: f64| {
        if d2 < best {
            best = d2;
        }
    };
    for p in &a.verts {
        for t in &b.tris {
            take((closest_on_triangle(*p, t[0], t[1], t[2]) - *p).length_sq());
        }
        for e in &b.edges {
            take((closest_on_segment(*p, e.0, e.1) - *p).length_sq());
        }
        for q in &b.verts {
            take((*q - *p).length_sq());
        }
    }
    for p in &b.verts {
        for t in &a.tris {
            take((closest_on_triangle(*p, t[0], t[1], t[2]) - *p).length_sq());
        }
        for e in &a.edges {
            take((closest_on_segment(*p, e.0, e.1) - *p).length_sq());
        }
    }
    for e in &a.edges {
        for f in &b.edges {
            let (x, y) = seg_seg(e.0, e.1, f.0, f.1);
            take((x - y).length_sq());
        }
    }
    physics::math::sqrt(best)
}

fn intersect(a: &Poly, b: &Poly) -> bool {
    for e in &a.edges {
        for t in &b.tris {
            if seg_crosses_tri(e.0, e.1, t[0], t[1], t[2]) {
                return true;
            }
        }
    }
    for e in &b.edges {
        for t in &a.tris {
            if seg_crosses_tri(e.0, e.1, t[0], t[1], t[2]) {
                return true;
            }
        }
    }
    a.verts.iter().any(|p| b.contains(*p)) || b.verts.iter().any(|p| a.contains(*p))
}

/// the largest gap over the separating axes (<= 0 when they overlap: minus the penetration depth)
fn sat(a: &Poly, b: &Poly) -> Option<f64> {
    let mut axes: Vec<Vec3> = Vec::new();
    axes.extend(a.normals.iter().copied());
    axes.extend(b.normals.iter().copied());
    for e in &a.edges {
        for f in &b.edges {
            if let Some(n) = (e.1 - e.0).cross(f.1 - f.0).try_normalize() {
                if (e.1 - e.0).cross(f.1 - f.0).length() > 1e-9 * (e.1 - e.0).length() * (f.1 - f.0).length() {
                    axes.push(n);
                }
            }
        }
    }
    if axes.is_empty() {
        return None;
    }
    let mut best = f64::NEG_INFINITY;
    for n in axes {
        let (mut amin, mut amax, mut bmin, mut bmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in &a.verts {
            let d = n.dot(*p);
            amin = amin.min(d);
            amax = amax.max(d);
        }
        for p in &b.verts {
            let d = n.dot(*p);
            bmin = bmin.min(d);
            bmax = bmax.max(d);
        }
        let gap = (bmin - amax).max(amin - bmax);
        if gap > best {
            best = gap;
        }
    }
    Some(best)
}

/// the signed distance of the cores: the gap, or minus the penetration depth
pub fn core_dist(a: &Poly, b: &Poly) -> f64 {
    let d = brute(a, b);
    if d <= 1e-12 || intersect(a, b) {
        return match sat(a, b) {
            Some(g) => g.min(0.0),
            None => 0.0,
        };
    }
    d
}

/// the signed distance with the radii: the gap, or minus the penetration depth
pub fn signed_dist(a: &Poly, b: &Poly) -> f64 {
    core_dist(a, b) - a.radius - b.radius
}

/// the signed distance from a point to a Poly (the closest point outside, minus the depth inside a solid)
pub fn point_dist(p: Vec3, b: &Poly) -> f64 {
    signed_dist(&Poly::point(p, 0.0), b)
}

/// for a convex gap function f on [0, 1]: (the first t with f(t) <= target, if any, and the minimum of f)
pub fn first_root(f: &dyn Fn(f64) -> f64, target: f64) -> (Option<f64>, f64) {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..64 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if f(m1) <= f(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let tmin = (lo + hi) * 0.5;
    let fmin = f(tmin).min(f(0.0)).min(f(1.0));
    let tmin = if f(0.0) <= fmin { 0.0 } else if f(1.0) <= fmin { 1.0 } else { tmin };
    if f(0.0) <= target {
        return (Some(0.0), fmin);
    }
    if fmin > target {
        return (None, fmin);
    }
    let (mut a, mut b) = (0.0f64, tmin);
    for _ in 0..64 {
        let m = (a + b) * 0.5;
        if f(m) <= target {
            b = m;
        } else {
            a = m;
        }
    }
    (Some(b), fmin)
}
