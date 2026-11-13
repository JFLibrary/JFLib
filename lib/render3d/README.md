# jflib-render3d

The core of [JFLib](https://jflib.org)'s 3D engine, built on its GPU layer
[`jflib-gpu`](https://crates.io/crates/jflib-gpu): meshes in shared buffers, glTF's physically
based materials with their textures, scenes of very many instances with `f64` positions, a sun,
point and spot lights, light from the environment, cascaded shadows, and visibility decided on the
GPU. There is no draw call per object anywhere: a frame is a few compute dispatches and indirect
draws, and their number does not grow with the instances the scene holds. It is plain Rust,
without the standard library and without a line of `unsafe` code.

## What it draws

- **Meshes** with positions, normals, texture coordinates and indices go into shared pools of the
  `Renderer`. A mesh can have detail levels: below a projected size an instance takes a simpler
  mesh, or none at all (`Renderer::set_lods`).
- **Materials** are glTF 2.0's metallic-roughness materials with all five textures (base colour,
  metallic-roughness, normal, occlusion, emission) and the alpha mode MASK. Textures live in one
  texture array per size class, from 128 to 2048 texels, with mip levels made on the CPU, and the
  shader follows glTF's wrap and filter modes exactly.
- **Instances** place a mesh with a material: a position in `f64`, a rotation and a scale per
  axis. Mirrored instances (the product of their scales negative) are drawn by their own pipeline
  with their faces turned the right way.
- **Light** follows glTF 2.0's reference model (GGX, Smith, Schlick): a sun whose colour is
  illuminance in lux, point and spot lights in the units of `KHR_lights_punctual`, environment
  light from spherical harmonics and a prefiltered cube made from a procedural sky or six faces,
  then exposure and Khronos PBR Neutral tone mapping in the fragment shader.
- **Shadows** of the sun come from up to four cascades of shadow maps, culled on the GPU and
  snapped to their texel grid. `Shadows::for_level` gives the default for a device's level: fewer
  and smaller maps for the GPUs of level `Basis`, such as phones and the browser.

Visibility runs in a compute shader that writes one instance list per mesh, side and alpha class.
On level `Voll` one multi-draw per side, class and pass draws all of them, on `Basis` one indirect
draw per list. An occlusion test against a depth pyramid, in two phases per frame, leaves out what
lies hidden behind other things; it is on by default when the depth buffer can be sampled.

## How it is used

```rust
use jflib::{gpu, render3d};
use jflib::render3d::{Camera, Instance, Material, MeshData, Renderer, RendererDesc, Scene, Transform};

let mut r = Renderer::new(&dev, &RendererDesc::new(gpu::Format::Rgba8Unorm))?;
let mesh = r.add_mesh(&MeshData { positions: &pos, normals: &nrm, uvs: &uv, indices: &idx })?;
let red = r.add_material(&Material::color(0.8, 0.1, 0.1))?;
r.set_size(1920, 1080)?;                        // a depth buffer the renderer keeps

let mut scene = Scene::new(&r)?;
scene.add(Instance { mesh, material: red, transform: Transform::at([1e7, 0.0, 1e7]) })?;
scene.set_origin([1e7, 0.0, 1e7]);              // (0, 0, 0) on the GPU; keep it near the camera
scene.set_camera(&Camera::look_at([1e7 + 5.0, 2.0, 1e7], [1e7, 0.0, 1e7], [0.0, 1.0, 0.0],
                                  1.0, 16.0 / 9.0, 0.1, f64::INFINITY));

// every frame
scene.prepare(&mut r)?;                         // waits for this slot's last frame, uploads what changed
let mut e = dev.encoder()?;
r.record(&mut e, &scene, color, None)?;         // None: the renderer's own depth buffer
let id = dev.queue().submit([e.finish()?])?;
scene.submitted(id);

// a glTF file: read without a GPU by jflib-gltf, then meshes, textures, materials and instances
let imported = render3d::gltf::upload(&mut r, &doc, None)?;
for i in &imported.instances {
    scene.add(*i)?;
}
```

The conventions: reverse Z (depth 1 at the near plane, cleared to `DEPTH_CLEAR`, no far plane by
default), a right-handed world, faces counter-clockwise from outside, and linear colours. An sRGB
target encodes them in hardware; for an 8-bit target without sRGB, such as a window's swapchain,
the fragment shaders encode them (`RendererDesc::encode_srgb`). Positions are `f64` on the CPU
and `f32` relative to the scene's origin on the GPU, so a scene far from (0, 0, 0) keeps its
precision near the camera.

[`jflib-gltf`](https://crates.io/crates/jflib-gltf) reads `.gltf` and `.glb` files;
`render3d::gltf::upload` puts such a document on the GPU and returns its instances, lights and
suns for a scene.

## Platforms and getting it

The same WGSL shaders run on every path: compiled ahead to SPIR-V for Vulkan and Metal, and handed
to WebGPU as text in the browser, where each list gets a draw of its own and there are no GPU
timings. Inside a window, a scene is the content of a `ui.gpu_view` of
[`jflib-ui`](https://crates.io/crates/jflib-ui), next to the widgets: the app connects the two with
a `GpuContent` of its own. Not built: translucent materials (glTF's BLEND is drawn opaque), an
orthographic camera, multisampling, and removing meshes.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::render3d`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

jflib-render3d is new in this release: meshes, glTF's PBR materials with textures, scenes of
instances with `f64` positions, sun, point and spot lights, environment light, cascaded shadows,
visibility and occlusion on the GPU, detail levels, and glTF scenes put on the GPU in one call.
Everything is in the [changelog](https://docs.rs/crate/jflib-render3d/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#render3d): every public
  item of `render3d`, also on [docs.rs](https://docs.rs/jflib-render3d)
- [RENDER3D.md](https://github.com/JFLibrary/JFLib/blob/main/docs/RENDER3D.md) (in German): an
  example, the structure, textures, lights, shadows, occlusion, detail levels, WebGPU, sRGB, the
  measurements and what is not built yet

## License

MIT or Apache-2.0, at your choice.
