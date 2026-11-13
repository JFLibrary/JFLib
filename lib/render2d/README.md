# jflib-render2d

The core of a 2D renderer for CAD drawings, built by [JFLib](https://jflib.org) on its GPU layer
[`jflib-gpu`](https://crates.io/crates/jflib-gpu). It draws plans made of hundreds of thousands of
lines, arcs, curves, fills and hatches: the scene stays on the GPU and only what changed goes up,
every stroke gets its coverage computed from its geometry instead of from samples, coordinates
are `f64` even far from the origin, and the GPU decides what is in view. It is plain Rust, without
the standard library and without a line of `unsafe` code.

## The scene

A `Scene` holds entities with stable ids on layers, each drawn with a style. Layers are drawn by
their `order` and can be hidden or made transparent; within a layer the entities keep the order
they were inserted in, also when one is updated.

- **Shapes:** lines, open and closed polylines, arcs and circles, quadratic and cubic Bézier
  curves, ellipses and elliptic arcs, B-splines up to degree 3 (rational ones and DXF's knot
  vectors included), and fills made of any number of rings of those shapes, even-odd between the
  rings.
- **Styles:** a colour, a width in pixels or drawing units, butt, round or square caps, mitre,
  round or bevel joins, dash patterns of up to eight entries with a phase, hatches of up to four
  line families as in DXF, and AutoCAD's transparency per layer, per entity or by block.
- **Blocks:** a drawing defined once and inserted many times, as DXF's blocks and inserts,
  including arrays of inserts. Inserts nest, move, turn, scale unevenly and mirror. A block's
  records lie on the GPU once and every insert is an instance with its transform, so moving a
  thousand inserts sends only their transforms. Styles can take their colour, width or dash
  pattern from the insert.

The camera has a centre in `f64`, a scale in drawing units per pixel and a rotation; its y axis
points up on the screen.

## How it is used

```rust
use jflib::gpu;
use jflib::render2d::{Camera, Layer, Renderer, RendererDesc, Scene, Shape, Style, Unit};

let mut r = Renderer::new(&dev, &RendererDesc::new(gpu::Format::Bgra8Unorm))?;
let mut scene = Scene::new(&r)?;
let walls = scene.add_layer(&Layer::new(0))?;
let axes = scene.add_layer(&Layer::new(1))?;               // drawn above `walls`

let mut thick = Style::new(0xFF20_2020);                   // 0xAARRGGBB, sRGB
thick.width = 0.5;                                         // half a drawing unit
thick.unit = Unit::Drawing;
let thick = scene.add_style(&thick)?;
let dashed = scene.add_style(&Style::new(0xFF80_2020).dashed(&[6.0, 1.5, 0.8, 1.5], Unit::Drawing, 0.0))?;

let outline = [[1e7, 0.0], [1e7 + 40.0, 0.0], [1e7 + 40.0, 30.0], [1e7, 30.0]];
scene.insert(walls, thick, &Shape::Polyline { points: &outline, closed: true })?;
scene.insert(axes, dashed, &Shape::Line { a: [1e7 + 20.0, -5.0], b: [1e7 + 20.0, 35.0] })?;
scene.insert(axes, dashed, &Shape::Circle { center: [1e7 + 20.0, 15.0], radius: 4.0 })?;
scene.set_camera(&Camera::new([1e7 + 20.0, 15.0], 0.05))?;  // 20 pixels per unit

// every frame
scene.prepare(&mut r, (width, height))?;   // waits for this slot's last frame, uploads what changed
let mut e = dev.encoder()?;
r.record(&mut e, &scene, color)?;          // color: a gpu::ColorAttachment of that size
let id = dev.queue().submit([e.finish()?])?;
scene.submitted(id);
```

Colours are sRGB codes with straight alpha, blended the way
[`jflib-gfx`](https://crates.io/crates/jflib-gfx) blends them, so the target is a format without
sRGB, such as the `Bgra8Unorm` of a window. Where the segments of a translucent stroke meet at a
corner, the pixels there are blended once, not twice.

## How it draws

Strokes are quads whose coverage the fragment shader computes from the geometry, with exact caps
and joins and without multisampling. Curves are flattened on the CPU per zoom band, within 1/32 of
a pixel, and only where the view needs them when a curve is too large to flatten whole. Fills are
triangulated on the CPU in `f64`, and their edges are smoothed as strokes are. Hatches are computed
in the fragment shader from the camera's place in `f64` and turn into a flat tone where their lines
come closer than two pixels.

Coordinates are `f64` on the CPU and a pair of `f32` per axis on the GPU, with the camera
subtracted first. In JFLib's tests, strokes, curves, hatches and inserts stay within 1/16 of a
pixel at a scale of a millionth of a unit per pixel at coordinates near 10 million units.

Visibility runs in compute shaders that keep the painter's order and write the indirect draws. On
level `Voll` one multi-draw draws every visible layer, on `Basis` there is one indirect draw per
layer.

## Platforms and getting it

The same WGSL shaders run on every path: compiled ahead to SPIR-V for Vulkan and Metal, and handed
to WebGPU as text in the browser. It has been tested on Vulkan under Windows and Linux, on NVIDIA
and Intel GPUs, and on WebGPU in Chromium; Android and Apple's Metal have not been tested yet.
Inside a window it is the content of a `ui.gpu_view` of
[`jflib-ui`](https://crates.io/crates/jflib-ui): the app connects the two with a `GpuContent` of
its own. Text, selection and DXF import are not part of it yet.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::render2d`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

jflib-render2d is new in this release: the scene of lines, polylines, arcs, circles, curves, fills
and hatches with layers and styles, dash patterns, blocks inserted many times and transparency,
drawn on jflib-gpu with coverage from the geometry and `f64` coordinates. Everything is in the
[changelog](https://docs.rs/crate/jflib-render2d/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#render2d): every public
  item of `render2d`, also on [docs.rs](https://docs.rs/jflib-render2d)
- [RENDER2D.md](https://github.com/JFLibrary/JFLib/blob/main/docs/RENDER2D.md) (in German): an
  example, the structure, the shaders, coordinates, curves, fills, hatches, blocks, the browser,
  the measurements and what is not built yet

## License

MIT or Apache-2.0, at your choice.
