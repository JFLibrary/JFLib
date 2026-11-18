# jflib-game-render3d

The bridge from JFLib's game core ([jflib-game](https://crates.io/crates/jflib-game)) to its 3D
renderer ([jflib-render3d](https://crates.io/crates/jflib-render3d)), part of
[JFLib](https://jflib.org): entities with a mesh become instances of a render3d scene, and every
frame writes only what changed. It is plain Rust, without the standard library and without a line of
`unsafe` code.

## How it is used

```rust
use jflib::game::{Pose, Vec3, World};
use jflib::game_render3d::{Bridge, MeshRef, camera};

// an entity with a Pose and a MeshRef is drawn
let crate_box = world.spawn_with((Pose::at(Vec3::new(2.0, 0.5, -3.0)), MeshRef { mesh, material }));

// every frame, before render3d's prepare
let mut bridge = Bridge::new();
scene.set_camera(&camera(&first_person.view(), 1.1, width as f64 / height as f64, 0.05, f64::INFINITY));
bridge.follow(&mut scene, first_person.eye.to_array());   // the scene's origin follows the camera
let stats = bridge.sync(&world, &mut scene, game.alpha())?;
scene.prepare(&mut renderer)?;
```

`sync` adds instances for new meshes, switches meshes and materials that changed, removes instances
of entities that are gone and writes the transforms of poses that changed, at the frame's place
between two ticks: a pose that moved in the latest tick is written every frame, a standing one once.
`retire_mesh` and `collect` remove meshes and materials from the renderer once no instance uses them
(when a map is left). `reset` starts over on a new scene after the GPU device was lost.

## Why a crate of its own

A game's server is built from the same workspace as its client. Cargo unifies a crate's features
over everything one build builds, so a renderer behind a feature of the game core would end up in
the server's program; a crate the server does not name never does. The game core stays free of any
GPU code.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::game_render3d`. All of JFLib's crates share
one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release. The
[changelog](https://docs.rs/crate/jflib-game-render3d/latest/source/CHANGELOG.md) will say so.

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#game_render3d): every
  public item of `game_render3d`, also on [docs.rs](https://docs.rs/jflib-game-render3d)
- [GAMECORE.md](https://github.com/JFLibrary/JFLib/blob/main/docs/GAMECORE.md) (in German): the
  design of the game core and the bridge, the tests
- [RENDER3D.md](https://github.com/JFLibrary/JFLib/blob/main/docs/RENDER3D.md) (in German): the
  renderer

## License

MIT or Apache-2.0, at your choice.
