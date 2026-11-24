// physics -- collision and rigid bodies for games: shapes, broad phase, queries, a character controller,
// bodies with contacts and joints; deterministic, the same bits on every target
//
//   Shape          sphere, capsule, box, convex hull (ConvexHull::new: quickhull), static triangle mesh
//                  (TriMesh: f32 vertices, a binned-SAH tree, active edges against ghost collisions)
//   World          bodies (shape, Iso, layers) in dynamic AABB trees; ray_cast, ray_cast_all, cast (a
//                  swept convex shape: time of impact, normal, contact point), overlap, contacts
//                  (penetration depth and normal per body and triangle), closest_point
//   query          the same for one shape against one placed shape, without a world
//   CharacterController  the kinematic figure: an upright capsule that walks, runs, jumps, crouches, takes
//                  steps and ramps up to a slope limit, slides along walls, snaps down; one step a pure
//                  function of (state, input, world, dt)
//   Simulation     rigid bodies (RigidBody: static, kinematic, dynamic; MassProps from the shape and a
//                  density) in a World: gravity, forces, contacts with friction, restitution and rolling
//                  resistance (manifold.rs: faces clipped along the GJK / EPA normal, up to four points),
//                  joints (joint.rs: ball with cone and twist, hinge, fixed, slider, distance; limits,
//                  motors, springs), the soft step of Box2D v3 with substeps (solver.rs), speculative
//                  contacts, sweeps of fast bodies, sleeping islands, contact and sensor events; Clone is
//                  a snapshot that steps on to the same bits
//   math           Vec3, Quat, Mat3, Iso, Aabb in f64; sqrt correctly rounded, sin / cos / atan2 from rt's
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
mod character;
mod convex;
mod gjk;
mod hull;
mod joint;
mod manifold;
mod mass;
/// Vec3, Quat, Mat3, Iso, Aabb in f64 and the scalar functions, the same bits on every target
pub mod math;
mod mesh;
/// one shape against one placed shape, without a world
pub mod query;
mod ray;
mod shape;
mod sim;
mod solver;
mod tree;
mod world;

pub use bvh::BvhNode;
pub use character::{CharacterController, CharacterInput, CharacterParams, CharacterState, MAX_WALL_TOUCHES, StepEvents, Touch};
pub use gjk::{CAST_SKIN, cast_fallbacks};
pub use hull::{ConvexHull, HullError, MAX_HULL_POINTS};
pub use joint::{Joint, JointError, JointId, JointKind, Motor, Spring};
pub use mass::MassProps;
pub use math::{Aabb, Iso, Mat3, Quat, Vec3};
pub use mesh::{ACTIVE_EDGE_COS, MAX_TRIANGLES, MeshError, MeshOptions, TriMesh};
pub use query::{CastHit, Contact, NO_TRIANGLE, PointHit, RayHit, Separation};
pub use shape::Shape;
pub use sim::{BodyError, BodyKind, BodyState, ContactEvent, ContactManifold, ContactPoint, RigidBody, SimParams, SimStats, Simulation};
pub use tree::DynamicTree;
pub use world::{Body, BodyId, Filter, World};
