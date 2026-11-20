# jflib-geom

The modelling kernel of [JFLib](https://jflib.org)'s game engine: indexed triangle meshes built from
primitives, profiles and operations, and the checks a model must pass, with the same bits on every
platform. Boxes, cylinders, cones, spheres, tori, stairs and arches; profiles extruded, raised from a
floor plan, swept along a path or turned about an axis; moves, rotations, scales, mirrors, arrays,
merging and welding; flat or smooth normals with an angle threshold; texture coordinates by box
projection in metres. Union, difference and intersection of convex solids, as brushes in a map editor,
decided with exact arithmetic (results closed, faces outward, ready for the next operation), and
chamfered or rounded edges. The checks say whether a model is closed and manifold, whether its faces
point outward, whether it has degenerate triangles and where the first problem lies, how large it is,
its area and volume. It is plain Rust, without the standard library and without a line of `unsafe`
code.

## How it is used

```rust
use jflib::geom::{self, Axis, Mesh, Vec3};

// a table: a top on four legs; every primitive stands on y = 0, centred over the origin
let top = geom::cuboid(1.2, 0.04, 0.8)?.translated(Vec3::new(0.0, 0.71, 0.0));
let leg = geom::cylinder(0.025, 0.71, 24)?.translated(Vec3::new(0.55, 0.0, 0.35));
let legs = Mesh::merge(&[&leg, &leg.mirrored(Axis::X)])?;
let legs = Mesh::merge(&[&legs, &legs.mirrored(Axis::Z)])?;
let table = Mesh::merge(&[&top, &legs])?.uv_box(1.0);

// a door frame: a profile swept along the edge of the opening, mitred at the corners
let profile = [[-0.07, -0.07], [0.0, -0.07], [0.0, 0.02], [0.015, 0.02], [0.015, 0.07], [-0.07, 0.07]];
let edge = [Vec3::new(-0.44, 0.0, 0.0), Vec3::new(-0.44, 2.0, 0.0), Vec3::new(0.44, 2.0, 0.0), Vec3::new(0.44, 0.0, 0.0)];
let frame = geom::sweep(&profile, &edge, false, 0.0)?;

// a wall with a window: a difference of convex solids, the sill with rounded edges
let wall = geom::cuboid(3.0, 2.5, 0.3)?;
let window = geom::cuboid(1.2, 1.3, 1.0)?.translated(Vec3::new(0.0, 0.9, 0.0));
let wall = geom::difference(&wall, &[&window])?;
let sill = geom::bevel(&geom::cuboid(1.4, 0.05, 0.35)?, 0.01, 3, None)?;

let report = table.check();
assert!(report.closed() && report.outward() && report.degenerate == 0);
println!("{} triangles, {:?} m, {}", report.triangles, report.size(), report.json());
```

A mesh holds positions, normals and UV in `f64`, three indices per triangle (counter-clockwise from
outside, as in render3d and glTF) and the topology: each vertex belongs to a point, so a hard edge is
two vertices of one point and does not open the surface, while two parts that only touch stay two
closed parts. `Mesh::weld` joins the points of vertices at the same place. Angles are degrees, exact
at every multiple of 90. Meshes convert to `f32` arrays for render3d and lib/gltf.

## Made for scripts

The kernel is the geometry behind JFLib's modelling with JFScript: a script describes a model from
building blocks and parameters, `apps/modeltool` binds the script's functions to this crate, draws the
model from fixed cameras and writes the checks as JSON. Error texts are German, like JFScript's,
because they are read by whoever wrote the script. The crate itself knows no script.

## The same bits everywhere

Only `+ - * /` and correctly rounded square roots on `f64` (never a fused multiply-add), `sin`,
`cos` and `atan2` from JFLib's own portable math, sorting with unique keys, no unordered containers
and no threads: the same calls give the same mesh bits on Windows, Linux, macOS and in the browser,
which the checksum of `apps/modeltool` shows. So the same script gives the same model everywhere,
and a hash of the script and its inputs is enough as a cache key.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::geom`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release. The
[changelog](https://docs.rs/crate/jflib-geom/latest/source/CHANGELOG.md) will say so.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#geom): every public
  item of `geom`, also on [docs.rs](https://docs.rs/jflib-geom)
- [GEOM.md](https://github.com/JFLibrary/JFLib/blob/main/docs/GEOM.md) (in German): the design and
  its reasons, the script API of modeltool, the tests and checksums, what JFScript still needs

## License

MIT or Apache-2.0, at your choice.
