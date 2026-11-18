// convex -- the convex shapes as support functions for GJK, EPA and the casts: a core (point, segment,
// box, hull, triangle) and a margin (the radius of spheres and capsules), placed by a rotation matrix and
// a position.  The queries run in the frame of the second shape, so positions far from the world's origin
// cancel once (in the relative placement) and not in every support point.

use crate::hull::ConvexHull;
use crate::math::{Aabb, Mat3, Vec3};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Core<'a> {
    /// a sphere's centre
    Point,
    /// a capsule's axis: (0, -h, 0) .. (0, h, 0)
    Segment(f64),
    /// a box's half sizes
    Box(Vec3),
    Hull(&'a ConvexHull),
    /// a mesh's triangle, already in the frame of the query
    Triangle([Vec3; 3]),
}

impl Core<'_> {
    /// the farthest point of the core in direction d (local), the first of equals
    #[inline]
    pub(crate) fn support(&self, d: Vec3) -> Vec3 {
        match *self {
            Core::Point => Vec3::ZERO,
            Core::Segment(h) => Vec3::new(0.0, if d.y >= 0.0 { h } else { -h }, 0.0),
            Core::Box(h) => Vec3::new(
                if d.x >= 0.0 { h.x } else { -h.x },
                if d.y >= 0.0 { h.y } else { -h.y },
                if d.z >= 0.0 { h.z } else { -h.z },
            ),
            Core::Hull(hull) => hull.support(d),
            Core::Triangle(t) => {
                let (a, b, c) = (t[0].dot(d), t[1].dot(d), t[2].dot(d));
                if a >= b && a >= c {
                    t[0]
                } else if b >= c {
                    t[1]
                } else {
                    t[2]
                }
            }
        }
    }

    /// a point inside the core (the start of GJK)
    #[inline]
    pub(crate) fn center(&self) -> Vec3 {
        match *self {
            Core::Point | Core::Segment(_) | Core::Box(_) => Vec3::ZERO,
            Core::Hull(h) => h.aabb().center(),
            Core::Triangle(t) => (t[0] + t[1] + t[2]) * (1.0 / 3.0),
        }
    }
}

/// a core with its margin, placed: world = rot * local + pos (rot None: no rotation)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed<'a> {
    pub core: Core<'a>,
    pub margin: f64,
    pub rot: Option<Mat3>,
    pub pos: Vec3,
}

impl<'a> Placed<'a> {
    #[inline]
    pub(crate) fn new(core: Core<'a>, margin: f64, rot: Option<Mat3>, pos: Vec3) -> Placed<'a> {
        Placed { core, margin, rot, pos }
    }

    /// the support point of the core in the query frame
    #[inline]
    pub(crate) fn support(&self, d: Vec3) -> Vec3 {
        match &self.rot {
            Some(m) => m.mul_vec(self.core.support(m.mul_vec_t(d))) + self.pos,
            None => self.core.support(d) + self.pos,
        }
    }

    #[inline]
    pub(crate) fn center(&self) -> Vec3 {
        match &self.rot {
            Some(m) => m.mul_vec(self.core.center()) + self.pos,
            None => self.core.center() + self.pos,
        }
    }

    /// the box around the core and its margin in the query frame (from the support points along the axes)
    pub(crate) fn aabb(&self) -> Aabb {
        let lo = Vec3::new(self.support(-Vec3::X).x, self.support(-Vec3::Y).y, self.support(-Vec3::Z).z);
        let hi = Vec3::new(self.support(Vec3::X).x, self.support(Vec3::Y).y, self.support(Vec3::Z).z);
        Aabb::new(lo - Vec3::splat(self.margin), hi + Vec3::splat(self.margin))
    }
}
