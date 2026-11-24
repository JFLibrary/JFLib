# jflib-physics

Collision and rigid bodies for games, part of [JFLib](https://jflib.org): shapes, a broad phase,
the queries a game needs and bodies that move under gravity, contacts and joints, with the same bits
on every platform. Spheres, capsules, boxes, convex hulls and static triangle meshes; a fixed bounding
volume hierarchy for a map and a dynamic AABB tree for the bodies that move; rays (first hit and all
hits), swept shapes with time of impact, normal and contact point, overlap tests, contacts with
penetration depth and the nearest point; a kinematic character controller; static, kinematic and
dynamic rigid bodies with friction, restitution and rolling resistance, ball, hinge, fixed, slider and
distance joints with limits, motors and springs (enough for ragdolls), sleeping islands, continuous
collision for fast bodies, contact and sensor events. It needs no GPU and no operating system: a game
server runs it headless. It is plain Rust, without the standard library and without a line of
`unsafe` code.

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

## Rigid bodies

A `Simulation` moves the bodies of its `World`: static ones (a map), kinematic ones that only their
velocity moves (a door, a platform, a lift) and dynamic ones that gravity, forces, contacts and joints
move. Their mass comes from the shape and a density (or is given), friction, restitution and rolling
resistance are per body, layers say what collides with what.

```rust
use jflib::physics::{ContactEvent, Iso, JointKind, Quat, RigidBody, Shape, Simulation, Vec3};

let mut sim = Simulation::new(); // gravity 9.81 m/s² down -Y, four substeps a step
sim.add(RigidBody::fixed(Shape::mesh(map), Iso::IDENTITY))?;
let crate_box = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(0.0, 5.0, 0.0))))?;
let barrel = sim.add(
    RigidBody::dynamic(Shape::capsule(0.4, 0.3), Iso::at(Vec3::new(1.0, 6.0, 0.0)))
        .with_density(400.0)
        .with_rolling_resistance(0.05),
)?;

// a door on a hinge (the joint's X axis is the hinge's axis), open up to 100 degrees
let post = sim.add(RigidBody::fixed(Shape::sphere(0.05), Iso::at(Vec3::new(4.0, 1.0, 0.0))).with_layers(0, 0))?;
let door = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 1.0, 0.05)), Iso::at(Vec3::new(4.5, 1.0, 0.0))))?;
let up = Quat::from_axis_angle(Vec3::Z, core::f64::consts::FRAC_PI_2);
sim.add_joint(post, door, Iso::new(Vec3::new(4.0, 1.0, 0.0), up), JointKind::Hinge { limits: Some((0.0, 1.75)), motor: None, spring: None })?;

sim.step(1.0 / 60.0); // a fixed step
let pose = sim.iso(crate_box); // for the renderer
for e in sim.events() {
    if let ContactEvent::Begin { speed, .. } = e { /* a sound as loud as the impact */ }
}
// sim.world(): rays, casts and the character controller see every body where it is now
```

Joints: ball (with a swing cone and a twist range, for shoulders and hips), hinge (limits, a motor, a
spring), fixed, slider (limits, motor, spring) and distance (rope, rod, spring); a ragdoll is a set of
them. Islands of bodies at rest fall asleep and cost nothing until something touches them. Fast bodies
are swept against what does not move (a `bullet` also against what does), so they do not pass through a
wall between two steps. A `Simulation` is `Clone`: a copy is a snapshot that steps on to the same bits,
for a client that rolls back and predicts again.

## The same bits everywhere

Client prediction in a networked game only works when the client computes what the server computes.
This crate uses only `+ - * /` and correctly rounded square roots on `f64` (never a fused
multiply-add), `sin`, `cos` and `atan2` from JFLib's own portable math, no unordered containers and
no threads; every iteration has a fixed limit and every choice a tie-break. A first hit is the
smallest (time, body, triangle), so the answer does not depend on the shape of the trees. The test
program prints one checksum over the bits of all results of a fixed set of scenarios; it is the
same on Windows, Linux, macOS, in the browser and on Android, and so is a second one over figures
moved by the character controller. A third one, over rigid bodies, contacts and joints, is the same on
Windows, Linux and in the browser (macOS and Android are still to run).

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

The rigid bodies use the soft step of Box2D v3, in 3D: each step is cut into substeps, contacts and
joints are damped springs that push overlaps apart without overshooting, and a relax pass takes the push
out of the velocities again. Contact points come from GJK and EPA and the faces of both shapes along
their normal, clipped against each other (up to four points, as in Jolt); speculative contacts reach as
far as a body can move in the step, so a falling box stops exactly on the ground. Inertia tensors come
from the shapes in closed form and, for hulls, from their tetrahedra.

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
