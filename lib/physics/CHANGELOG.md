# Changelog of jflib-physics

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- Collision for games, deterministic to the bit on every target. Vec3, Quat, Mat3, Iso, Aabb in f64; `math::sqrt` correctly rounded (the processor's on x86-64 and ARM64, `sqrt_portable` elsewhere), `sin`, `cos`, `sincos`, `atan2` from rt's portable math. Shapes: sphere, capsule (upright), box, convex hull (`ConvexHull::new`, quickhull) and static triangle mesh (`TriMesh`: f32 vertices in its own frame, a binned-SAH tree with 32-byte nodes, active edges against ghost collisions on seams, two-sided triangles). `World`: bodies with layers in three dynamic AABB trees (still meshes, still bodies, moving bodies); `ray_cast`, `ray_cast_all`, `cast` (a swept convex shape: fraction, contact point, normal, triangle, depth when it started inside; touching shapes moving apart or along are free), `overlap`, `contacts` (depth and normal per body and triangle), `closest_point`; `Filter` (layer mask, one body left out). `query`: the same for one shape against one placed shape, `query::brute` without a mesh's tree. GJK, EPA, GJK ray casting with conservative advancement (a Newton fallback on the convex gap), watertight ray/triangle tests, robust slab tests. docs/PHYSICS.md

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
- The kinematic character controller (G7c part 2): `CharacterController::new(CharacterParams)` and `step(&state, &world, &filter, &input, dt) -> (CharacterState, StepEvents)`, an upright capsule that walks, runs, jumps and crouches (standing up only with room), takes steps up to `step_height` (an edge carries when the surface beyond it is walkable), ramps up to `max_slope_deg` (steeper surfaces are walls it slides down), slides along walls and into corners with Quake 3's clip planes (no jitter), snaps down ramps and stairs, steers in the air (Quake's air acceleration, `air_control`) and pushes itself out of overlaps by casts. On the ground the velocity approaches the wish velocity (`accel`), without input Quake's friction brakes; gravity uses the tick's mean velocity (free flight is exact). The same bits on every target; a step reads only position, velocity and crouched. `CharacterInput`, `StepEvents` (landed, impact speed, jumped, left ground, ceiling, walls, stepped up, snapped down, stand blocked, pushed out, casts), `Touch`, `MAX_WALL_TOUCHES`; defaults for a fast cartoon arena shooter. docs/PHYSICS.md section Der Character-Controller

### Changed

- `math::sqrt_portable` is a thin wrapper of rt's one portable square root (`rt::fx::sqrt_portable`, before a copy of its own), `math::sqrt` is `rt::fx::sqrtd` on every target (the processor's root on x86-64 and ARM64, the portable one in the browser and on the x86-64 Android emulator); the same bits as before for numbers, and NaN results follow `sqrtsd` now (the negative default NaN for a negative number, a quieted NaN for a NaN), so `math::sqrt` gives the same bits on every target for NaN too

