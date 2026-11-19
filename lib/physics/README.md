# jflib-physics

Collision for games, part of [JFLib](https://jflib.org): shapes, a broad phase and the queries a
game needs, with the same bits on every platform. Spheres, capsules, boxes, convex hulls and static
triangle meshes; a fixed bounding volume hierarchy for a map and a dynamic AABB tree for the bodies
that move; rays (first hit and all hits), swept shapes with time of impact, normal and contact point,
overlap tests, contacts with penetration depth and the nearest point. It needs no GPU and no
operating system: a game server runs it headless. It is plain Rust, without the standard library and
without a line of `unsafe` code.

## How it is used

```rust
use jflib::physics::{Filter, Iso, Shape, TriMesh, Vec3, World};

// the map: the same f32 positions and indices a render3d mesh is made of
let map = TriMesh::new(&positions, &indices).expect("valid indices");
let mut world = World::new();
world.add(Shape::mesh(map), Iso::IDENTITY);
let crate_box = world.add(Shape::cuboid(Vec3::new(0.5, 0.5, 0.5)), Iso::at(Vec3::new(3.0, 0.5, 2.0)));

// hitscan: the first body on the line of fire, 100 m
let eye = Vec3::new(0.0, 1.7, 0.0);
let aim = Vec3::new(0.0, 0.0, -1.0);
if let Some((body, hit)) = world.ray_cast(eye, aim, 100.0, &Filter::ALL) {
    // hit.point, hit.normal, hit.t (metres), hit.triangle for a mesh
}

// a figure's capsule moved by this tick's motion: where it first touches something
let figure = Shape::capsule(0.5, 0.3);
let motion = Vec3::new(0.1, -0.05, 0.0);
if let Some((body, hit)) = world.cast(&figure, &Iso::at(eye), motion, &Filter::ALL.excluding(crate_box)) {
    // stop at hit.fraction of the motion, slide along hit.normal
}

// what pushes a figure out of the level after a teleport: one contact per body and triangle
let mut contacts = Vec::new();
world.contacts(&figure, &Iso::at(eye), 0.0, &Filter::ALL, &mut contacts);
```

`physics::query` has the same queries for one shape against one placed shape, without a world.

A figure of a game moves with the kinematic character controller: an upright capsule that walks,
runs, jumps and crouches, takes steps and ramps up to a slope limit, slides along walls and into
corners and stays on the ground on the way down ramps and stairs. One step is a function of the
state, the input, the world and the tick's length, with the same bits on every platform:

```rust
use jflib::physics::{CharacterController, CharacterInput, CharacterParams, CharacterState};

let controller = CharacterController::new(CharacterParams::default()); // tuned for a fast arena shooter
let mut figure = CharacterState::new(Vec3::new(10.0, 2.0, 10.0)); // its feet
let input = CharacterInput { wish: Vec3::new(0.0, 0.0, -1.0), run: true, jump: false, crouch: false };
let (next, events) = controller.step(&figure, &world, &Filter::ALL, &input, 1.0 / 60.0);
figure = next; // events.landed, events.jumped, events.walls(), ...
```
Positions are `f64` (large worlds need no shifting of the origin for the physics); a mesh keeps its
vertices in `f32` in its own frame, as a render3d mesh does.

## The same bits everywhere

Client prediction in a networked game only works when the client computes what the server computes.
This crate uses only `+ - * /` and correctly rounded square roots on `f64` (never a fused
multiply-add), `sin`, `cos` and `atan2` from JFLib's own portable math, no unordered containers and
no threads; every iteration has a fixed limit and every choice a tie-break. A first hit is the
smallest (time, body, triangle), so the answer does not depend on the shape of the trees. The test
program prints one checksum over the bits of all results of a fixed set of scenarios; it is the
same on Windows, Linux, macOS, in the browser and on Android, and so is a second one over figures
moved by the character controller.

## How it works

GJK for distances, EPA for penetration depths, GJK ray casting with conservative advancement for the
swept shapes (the radii of spheres and capsules stay outside GJK as margins, so a cast stops within
a nanometre of contact), watertight ray/triangle tests, quickhull for the convex hulls, a binned SAH
tree with 32-byte nodes for the meshes, dynamic AABB trees for the bodies (one for the meshes that stand still, one
for the other bodies that stand still, one for those that move). Triangle meshes
know their active edges: a shape that slides over the seam between two triangles of a flat or concave
surface is not stopped by the edge ("ghost collisions"). The character controller slides with Quake 3's
clip planes (no jitter in corners), steps up, forward and down, and lets the edge of a walkable surface
carry the capsule.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::physics`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release. The
[changelog](https://docs.rs/crate/jflib-physics/latest/source/CHANGELOG.md) will say so.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#physics): every public
  item of `physics`, also on [docs.rs](https://docs.rs/jflib-physics)
- [PHYSICS.md](https://github.com/JFLibrary/JFLib/blob/main/docs/PHYSICS.md) (in German): the
  design and its reasons, precision and range, the tests, the measurements

## License

MIT or Apache-2.0, at your choice.
