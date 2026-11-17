// physics -- collision for games: shapes, broad phase, queries; deterministic, the same bits on every target
//
//   Shape          sphere, capsule, box, convex hull (ConvexHull::new: quickhull), static triangle mesh
//                  (TriMesh: f32 vertices, a binned-SAH tree, active edges against ghost collisions)
//   World          bodies (shape, Iso, layers) in a dynamic AABB tree; ray_cast, ray_cast_all, cast (a
//                  swept convex shape: time of impact, normal, contact point), overlap, contacts
//                  (penetration depth and normal per body and triangle), closest_point
//   query          the same for one shape against one placed shape, without a world
//   math           Vec3, Quat, Iso, Aabb in f64; sqrt correctly rounded, sin / cos / atan2 from rt's
//                  portable math
//
// Narrow phase: GJK for distances, EPA for penetration, GJK ray casting with conservative advancement for
// the casts (radii as margins outside GJK), watertight ray/triangle tests, analytic rays against the other
// primitives.  Deterministic: only + - * / and correctly rounded square roots in f64, never a fused
// multiply-add, no unordered containers, no threads, fixed iteration limits and tie-breaks; checked by the
// checksums of apps/physicstest on every target.  docs/PHYSICS.md describes the design, the tests and the
// measurements.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod bvh;
mod convex;
mod gjk;
mod hull;
pub mod math;
mod mesh;
pub mod query;
mod ray;
mod shape;
mod tree;
mod world;

pub use bvh::BvhNode;
pub use gjk::{CAST_SKIN, cast_fallbacks};
pub use hull::{ConvexHull, HullError, MAX_HULL_POINTS};
pub use math::{Aabb, Iso, Mat3, Quat, Vec3};
pub use mesh::{ACTIVE_EDGE_COS, MAX_TRIANGLES, MeshError, MeshOptions, TriMesh};
pub use query::{CastHit, Contact, NO_TRIANGLE, PointHit, RayHit, Separation};
pub use shape::Shape;
pub use tree::DynamicTree;
pub use world::{Body, BodyId, Filter, World};
