// shape -- the collision shapes: sphere, capsule, box, convex hull (convex, solid) and the static triangle
// mesh (two-sided triangles)

use crate::convex::{Core, Placed};
use crate::hull::{ConvexHull, HullError};
use crate::math::{Aabb, Iso, Mat3, Vec3};
use crate::mesh::TriMesh;
use alloc::sync::Arc;

/// A collision shape in its own frame.  Hulls and meshes are shared ([`Arc`]): cloning a shape is cheap,
/// one map mesh can sit in several worlds (server, prediction, tests).
#[derive(Clone, Debug)]
pub enum Shape {
    /// a ball around the origin
    Sphere { radius: f64 },
    /// the points within `radius` of the segment (0, -half_height, 0) .. (0, half_height, 0): upright,
    /// total height 2 (half_height + radius)
    Capsule { half_height: f64, radius: f64 },
    /// a box around the origin with half sizes `half`
    Box { half: Vec3 },
    /// a convex hull (shared)
    Hull(Arc<ConvexHull>),
    /// a triangle mesh: a static or kinematic body (never dynamic), not a query shape
    Mesh(Arc<TriMesh>),
}

impl Shape {
    /// a ball of `radius`
    #[inline]
    pub fn sphere(radius: f64) -> Shape {
        Shape::Sphere { radius }
    }

    /// an upright capsule: the segment (0, ±half_height, 0) grown by `radius`
    #[inline]
    pub fn capsule(half_height: f64, radius: f64) -> Shape {
        Shape::Capsule { half_height, radius }
    }

    /// a box with half sizes `half`
    #[inline]
    pub fn cuboid(half: Vec3) -> Shape {
        Shape::Box { half }
    }

    /// a convex hull, shared
    #[inline]
    pub fn hull(hull: ConvexHull) -> Shape {
        Shape::Hull(Arc::new(hull))
    }

    /// the convex hull of `points`
    pub fn hull_from_points(points: &[Vec3]) -> Result<Shape, HullError> {
        Ok(Shape::hull(ConvexHull::new(points)?))
    }

    /// a triangle mesh (for a static or kinematic body), shared
    #[inline]
    pub fn mesh(mesh: TriMesh) -> Shape {
        Shape::Mesh(Arc::new(mesh))
    }

    /// everything but a mesh
    #[inline]
    pub fn is_convex(&self) -> bool {
        !matches!(self, Shape::Mesh(_))
    }

    /// the radius around the core (spheres and capsules), 0 for the others
    #[inline]
    pub fn margin(&self) -> f64 {
        match self {
            Shape::Sphere { radius } | Shape::Capsule { radius, .. } => *radius,
            _ => 0.0,
        }
    }

    /// the box around the shape placed by `iso`
    pub fn aabb(&self, iso: &Iso) -> Aabb {
        match self {
            Shape::Mesh(m) => {
                let b = m.aabb();
                if !b.is_valid() {
                    return Aabb::new(iso.pos, iso.pos);
                }
                if iso.rot.is_identity() {
                    return Aabb::new(b.min + iso.pos, b.max + iso.pos);
                }
                let mut out = Aabb::EMPTY;
                for k in 0..8 {
                    let p = Vec3::new(
                        if k & 1 != 0 { b.max.x } else { b.min.x },
                        if k & 2 != 0 { b.max.y } else { b.min.y },
                        if k & 4 != 0 { b.max.z } else { b.min.z },
                    );
                    out = out.add_point(iso.transform_point(p));
                }
                out
            }
            _ => match self.placed(iso) {
                Some(p) => p.aabb(),
                None => Aabb::new(iso.pos, iso.pos),
            },
        }
    }

    /// the support form of a convex shape placed by `iso` (None for a mesh)
    pub(crate) fn placed(&self, iso: &Iso) -> Option<Placed<'_>> {
        let rot = if iso.rot.is_identity() { None } else { Some(iso.rot.to_mat3()) };
        self.placed_rot(rot, iso.pos)
    }

    pub(crate) fn placed_rot(&self, rot: Option<Mat3>, pos: Vec3) -> Option<Placed<'_>> {
        let (core, margin) = match self {
            Shape::Sphere { radius } => (Core::Point, *radius),
            Shape::Capsule { half_height, radius } => (Core::Segment(*half_height), *radius),
            Shape::Box { half } => (Core::Box(*half), 0.0),
            Shape::Hull(h) => (Core::Hull(h), 0.0),
            Shape::Mesh(_) => return None,
        };
        Some(Placed::new(core, margin, rot, pos))
    }
}
