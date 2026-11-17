// query -- one shape against one placed shape, without a world: rays, casts, distance and contacts, the
// nearest point.  The work happens in the frame of the target shape (positions far from the origin cancel
// once, in the relative placement); results come back in world coordinates.  The query shape of a cast,
// a distance or a contact must be convex: with a mesh as the query shape these give nothing.

use crate::convex::{Core, Placed};
use crate::gjk::{self, Cast};
use crate::math::{Iso, Vec3, clamp, max};
use crate::mesh::{APPROACH, LocalHit};
use crate::ray;
use crate::shape::Shape;
use alloc::vec::Vec;

/// `triangle` of a hit on anything but a mesh
pub const NO_TRIANGLE: u32 = u32::MAX;

/// where a ray hits
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// the ray parameter: point = origin + dir * t (metres when dir is a unit vector)
    pub t: f64,
    pub point: Vec3,
    /// the unit normal, against the ray (a ray starting inside a convex shape: t = 0, normal = -dir)
    pub normal: Vec3,
    /// the triangle's index in the mesh's index list, or [`NO_TRIANGLE`]
    pub triangle: u32,
}

/// where a moving shape first touches another
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CastHit {
    /// how far along the motion: the shape stops at iso.pos + motion * fraction (0..=1).  With a radius
    /// (sphere, capsule) on either side the gap there is at most 1e-9 m, between two shapes without one
    /// at most [`crate::CAST_SKIN`] + 1e-9 m; never a penetration that was not there at the start.
    pub fraction: f64,
    /// the contact point on the other shape's surface
    pub point: Vec3,
    /// the unit normal of that surface towards the moving shape
    pub normal: Vec3,
    pub triangle: u32,
    /// > 0: the shapes overlapped at the start by this much (then fraction = 0)
    pub depth: f64,
}

/// how two shapes touch or nearly touch
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// on the other shape's surface (with overlap: its deepest point)
    pub point: Vec3,
    /// the unit normal from the other shape towards the query shape: moving the query shape by
    /// normal * depth separates them (for a triangle of a mesh along the triangle's normal, unless the
    /// contact is on an active edge)
    pub normal: Vec3,
    /// the overlap; negative: the gap (contacts within a margin)
    pub depth: f64,
    pub triangle: u32,
}

/// the distance between two shapes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Separation {
    /// the gap; negative: the penetration depth
    pub distance: f64,
    /// the nearest (or deepest) points on the query shape and on the other one
    pub point_a: Vec3,
    pub point_b: Vec3,
    /// the unit normal from the other shape towards the query shape
    pub normal: Vec3,
    pub triangle: u32,
}

/// the point of a shape nearest to a given point
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointHit {
    /// the nearest point of the surface, or the point itself when it is inside a convex shape
    pub point: Vec3,
    /// 0 inside
    pub distance: f64,
    pub inside: bool,
    pub triangle: u32,
}

/// the convex query shape in the target's frame
fn query_in(shape: &Shape, iso: &Iso, target_iso: &Iso) -> Option<(Iso, Option<crate::math::Mat3>)> {
    if !shape.is_convex() {
        return None;
    }
    let rel = target_iso.inv_mul(iso);
    let rot = if rel.rot.is_identity() { None } else { Some(rel.rot.to_mat3()) };
    Some((rel, rot))
}

fn target_placed(target: &Shape) -> Option<Placed<'_>> {
    target.placed_rot(None, Vec3::ZERO)
}

/// the first hit of the ray origin + dir * t (0 <= t <= max_t) on `target` placed by `target_iso`
pub fn ray(target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64) -> Option<RayHit> {
    ray_b(false, target, target_iso, origin, dir, max_t)
}

fn ray_b(brute: bool, target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64) -> Option<RayHit> {
    let o = target_iso.inverse_transform_point(origin);
    let d = target_iso.inverse_transform_vector(dir);
    let (t, n, tri) = match target {
        Shape::Sphere { radius } => ray::sphere(o, d, *radius, max_t).map(|(t, n)| (t, n, NO_TRIANGLE))?,
        Shape::Capsule { half_height, radius } => ray::capsule(o, d, *half_height, *radius, max_t).map(|(t, n)| (t, n, NO_TRIANGLE))?,
        Shape::Box { half } => ray::cuboid(o, d, *half, max_t).map(|(t, n)| (t, n, NO_TRIANGLE))?,
        Shape::Hull(h) => ray::hull(o, d, h, max_t).map(|(t, n)| (t, n, NO_TRIANGLE))?,
        Shape::Mesh(m) => m.ray(brute, o, d, max_t).map(|h| (h.t, h.normal, h.tri))?,
    };
    Some(RayHit { t, point: origin + dir * t, normal: target_iso.transform_vector(n), triangle: tri })
}

/// every hit of the ray on `target` (one for a convex shape, one per triangle for a mesh), appended to
/// `out` sorted by t and then by triangle
pub fn ray_all(target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64, out: &mut Vec<RayHit>) {
    ray_all_b(false, target, target_iso, origin, dir, max_t, out)
}

fn ray_all_b(brute: bool, target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64, out: &mut Vec<RayHit>) {
    match target {
        Shape::Mesh(m) => {
            let o = target_iso.inverse_transform_point(origin);
            let d = target_iso.inverse_transform_vector(dir);
            let mut hits: Vec<LocalHit> = Vec::new();
            m.ray_all(brute, o, d, max_t, &mut hits);
            hits.sort_unstable_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(core::cmp::Ordering::Equal).then(a.tri.cmp(&b.tri)));
            out.extend(hits.iter().map(|h| RayHit { t: h.t, point: origin + dir * h.t, normal: target_iso.transform_vector(h.normal), triangle: h.tri }));
        }
        _ => {
            if let Some(h) = ray(target, target_iso, origin, dir, max_t) {
                out.push(h);
            }
        }
    }
}

/// the first contact of convex `shape`, moved from `iso` by `motion`, with `target`.  Shapes that touch or
/// overlap at the start give a hit at fraction 0 only when the motion goes into the other shape (normal .
/// motion < 0); a motion along or away from it is free, so sliding along a surface works.
pub fn cast(shape: &Shape, iso: &Iso, motion: Vec3, target: &Shape, target_iso: &Iso) -> Option<CastHit> {
    cast_b(false, shape, iso, motion, target, target_iso)
}

fn cast_b(brute: bool, shape: &Shape, iso: &Iso, motion: Vec3, target: &Shape, target_iso: &Iso) -> Option<CastHit> {
    let (rel, rot) = query_in(shape, iso, target_iso)?;
    let a = shape.placed_rot(rot, rel.pos)?;
    let r = target_iso.inverse_transform_vector(motion);
    let h = cast_local(brute, &a, r, target)?;
    Some(CastHit {
        fraction: h.t,
        point: target_iso.transform_point(h.point),
        normal: target_iso.transform_vector(h.normal),
        triangle: h.tri,
        depth: h.depth,
    })
}

/// a cast in the target's frame
pub(crate) fn cast_local(brute: bool, a: &Placed, r: Vec3, target: &Shape) -> Option<LocalHit> {
    if let Shape::Mesh(m) = target {
        return m.cast(brute, a, r);
    }
    let b = target_placed(target)?;
    match gjk::cast(a, &b, r) {
        Cast::Miss => None,
        Cast::Start => {
            let s = gjk::separation(a, &b, a.center() - b.center());
            if s.normal.dot(r) < -APPROACH * r.length() {
                Some(LocalHit { t: 0.0, point: s.pb, normal: s.normal, tri: NO_TRIANGLE, depth: max(0.0, -s.dist) })
            } else {
                None
            }
        }
        Cast::Hit { lambda, normal, pb } => Some(LocalHit { t: lambda, point: pb + normal * b.margin, normal, tri: NO_TRIANGLE, depth: 0.0 }),
    }
}

/// the gap (negative: depth) between convex `shape` and `target` if it is at most `max_dist`; for a mesh the
/// nearest triangle
pub fn separation(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, max_dist: f64) -> Option<Separation> {
    separation_b(false, shape, iso, target, target_iso, max_dist)
}

fn separation_b(brute: bool, shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, max_dist: f64) -> Option<Separation> {
    let (rel, rot) = query_in(shape, iso, target_iso)?;
    let a = shape.placed_rot(rot, rel.pos)?;
    let (s, tri) = match target {
        Shape::Mesh(m) => m.separation(brute, &a, max_dist)?,
        _ => {
            let b = target_placed(target)?;
            (gjk::separation(&a, &b, a.center() - b.center()), NO_TRIANGLE)
        }
    };
    if s.dist > max_dist {
        return None;
    }
    Some(Separation {
        distance: s.dist,
        point_a: target_iso.transform_point(s.pa),
        point_b: target_iso.transform_point(s.pb),
        normal: target_iso.transform_vector(s.normal),
        triangle: tri,
    })
}

/// do convex `shape` and `target` touch or overlap
pub fn intersects(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso) -> bool {
    intersects_b(false, shape, iso, target, target_iso)
}

fn intersects_b(brute: bool, shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso) -> bool {
    let Some((rel, rot)) = query_in(shape, iso, target_iso) else {
        return false;
    };
    let Some(a) = shape.placed_rot(rot, rel.pos) else {
        return false;
    };
    match target {
        Shape::Mesh(m) => m.intersects(brute, &a),
        _ => match target_placed(target) {
            Some(b) => {
                let g = gjk::distance(&a, &b);
                g.overlap || g.dist <= a.margin + b.margin
            }
            None => false,
        },
    }
}

/// the contacts of convex `shape` with `target` whose gap is at most `margin` (0: only touching and
/// overlapping), appended to `out` (a mesh: one per triangle, by triangle)
pub fn contacts(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, margin: f64, out: &mut Vec<Contact>) {
    contacts_b(false, shape, iso, target, target_iso, margin, out)
}

fn contacts_b(brute: bool, shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, margin: f64, out: &mut Vec<Contact>) {
    let Some((rel, rot)) = query_in(shape, iso, target_iso) else {
        return;
    };
    let Some(a) = shape.placed_rot(rot, rel.pos) else {
        return;
    };
    let mut hits: Vec<LocalHit> = Vec::new();
    contacts_local(brute, &a, target, margin, &mut hits);
    hits.sort_unstable_by(|x, y| x.tri.cmp(&y.tri));
    out.extend(hits.iter().map(|h| Contact {
        point: target_iso.transform_point(h.point),
        normal: target_iso.transform_vector(h.normal),
        depth: -h.t,
        triangle: h.tri,
    }));
}

pub(crate) fn contacts_local(brute: bool, a: &Placed, target: &Shape, margin: f64, out: &mut Vec<LocalHit>) {
    match target {
        Shape::Mesh(m) => m.contacts(brute, a, margin, out),
        _ => {
            if let Some(b) = target_placed(target) {
                let s = gjk::separation(a, &b, a.center() - b.center());
                if s.dist <= margin {
                    out.push(LocalHit { t: s.dist, point: s.pb, normal: s.normal, tri: NO_TRIANGLE, depth: max(0.0, -s.dist) });
                }
            }
        }
    }
}

/// the point of `target` nearest to `point`, if within `max_dist`
pub fn closest_point(target: &Shape, target_iso: &Iso, point: Vec3, max_dist: f64) -> Option<PointHit> {
    closest_point_b(false, target, target_iso, point, max_dist)
}

fn closest_point_b(brute: bool, target: &Shape, target_iso: &Iso, point: Vec3, max_dist: f64) -> Option<PointHit> {
    let p = target_iso.inverse_transform_point(point);
    let (q, d, inside, tri) = match target {
        Shape::Sphere { radius } => around(p, Vec3::ZERO, *radius),
        Shape::Capsule { half_height, radius } => around(p, Vec3::new(0.0, clamp(p.y, -*half_height, *half_height), 0.0), *radius),
        Shape::Box { half } => {
            let q = Vec3::new(clamp(p.x, -half.x, half.x), clamp(p.y, -half.y, half.y), clamp(p.z, -half.z, half.z));
            if q == p { (p, 0.0, true, NO_TRIANGLE) } else { (q, (p - q).length(), false, NO_TRIANGLE) }
        }
        Shape::Hull(h) => {
            if h.planes().iter().all(|&(n, pd)| n.dot(p) <= pd) {
                (p, 0.0, true, NO_TRIANGLE)
            } else {
                let a = Placed::new(Core::Point, 0.0, None, p);
                let b = Placed::new(Core::Hull(h), 0.0, None, Vec3::ZERO);
                let g = gjk::distance(&a, &b);
                let (_, pb) = g.simplex.witness();
                (pb, g.dist, false, NO_TRIANGLE)
            }
        }
        Shape::Mesh(m) => {
            let (q, d, tri) = m.closest_point(brute, p, max_dist)?;
            (q, d, false, tri)
        }
    };
    if d > max_dist {
        return None;
    }
    Some(PointHit { point: if inside { point } else { target_iso.transform_point(q) }, distance: d, inside, triangle: tri })
}

fn around(p: Vec3, c: Vec3, r: f64) -> (Vec3, f64, bool, u32) {
    let v = p - c;
    let l = v.length();
    if l <= r {
        return (p, 0.0, true, NO_TRIANGLE);
    }
    (c + v * (r / l), l - r, false, NO_TRIANGLE)
}

/// The same queries without a mesh's tree: every triangle in turn, through the same per-triangle code and
/// tie-breaks, so the results must equal those of the tree bit for bit.  For tests (apps/physicstest) and
/// for checking a mesh that behaves oddly; for convex shapes they are the queries above.
pub mod brute {
    use super::*;

    pub fn ray(target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64) -> Option<RayHit> {
        ray_b(true, target, target_iso, origin, dir, max_t)
    }

    pub fn ray_all(target: &Shape, target_iso: &Iso, origin: Vec3, dir: Vec3, max_t: f64, out: &mut Vec<RayHit>) {
        ray_all_b(true, target, target_iso, origin, dir, max_t, out)
    }

    pub fn cast(shape: &Shape, iso: &Iso, motion: Vec3, target: &Shape, target_iso: &Iso) -> Option<CastHit> {
        cast_b(true, shape, iso, motion, target, target_iso)
    }

    pub fn separation(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, max_dist: f64) -> Option<Separation> {
        separation_b(true, shape, iso, target, target_iso, max_dist)
    }

    pub fn intersects(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso) -> bool {
        intersects_b(true, shape, iso, target, target_iso)
    }

    pub fn contacts(shape: &Shape, iso: &Iso, target: &Shape, target_iso: &Iso, margin: f64, out: &mut Vec<Contact>) {
        contacts_b(true, shape, iso, target, target_iso, margin, out)
    }

    pub fn closest_point(target: &Shape, target_iso: &Iso, point: Vec3, max_dist: f64) -> Option<PointHit> {
        closest_point_b(true, target, target_iso, point, max_dist)
    }
}
