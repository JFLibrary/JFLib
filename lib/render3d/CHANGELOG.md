# Changelog of jflib-render3d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- Removing meshes, materials and textures (G7f): `Renderer::remove_mesh`, `remove_material`, `remove_texture`; their places, vertex and index ranges and texture layers are used again, the ids of removed ones stay invalid (ids carry a generation in their high 8 bits; a renderer that removes nothing numbers 0, 1, 2 as before); refused while a scene's instances, a material or another mesh's detail levels use them, and a scene prepared before a removal does not record; `mesh_count`, `material_count`, `pool_use`. docs/RENDER3D.md section Entfernen
- Double-sided materials and glTF's BLEND (G7f): `MaterialDesc` (`#[non_exhaustive]`: `blend`, `double_sided`) with `Renderer::add_material_desc`, `set_material_desc`, `double_sided`; `AlphaClass::Blend`. Double-sided lists are drawn without culling, back faces lit with the shading normal turned round (`Scene::list_kinds`, `ListKind`); blended instances go into one list per frame, sorted back to front per instance on the GPU (sort.wgsl) and drawn by one indirect draw after the rest with premultiplied alpha, the depth tested and not written, without shadows and outside the occlusion test's pyramid (`Scene::blend_buffer`, `blend_drawn`). `gltf::upload` draws BLEND and doubleSided as the file says. docs/RENDER3D.md sections Doppelseitige Materialien, Durchsichtiges
- An orthographic camera (G7f part 2): `Camera::ortho` (`Some(h)`: the view h high) and `Camera::ortho_at`; culling, the occlusion test, detail levels (by size alone), blended instances (by depth) and the sun's cascades (the view's box split uniformly) work for it. docs/RENDER3D.md section Orthografische Kamera
- The cartoon look (G7f part 2): `MaterialDesc::toon` with `Toon { bands, softness, rim, rim_width }`: the sun's and the lights' n.l in bands (in shadow as if turned away), a flat ambient light, a rim; base colour, normal map, emission, MASK, BLEND and double-sided as for PBR. `render3dtest --toon-sheet DIR` draws four looks to choose from. docs/RENDER3D.md section Cartoon-Material
- Outlines and FXAA (G7f part 2): `Scene::set_outline` (`Outline { width, color, crease }`): lines on the edges of the depth buffer (silhouettes, creases), drawn over the opaque image before blended instances and particles; `Scene::set_antialias(Antialias::Fxaa)`: FXAA 3.11 (quality 12) from an image of the renderer's into the target. docs/RENDER3D.md sections Umrisse, Kantenglättung
- The viewmodel (G7f part 2): `Scene::set_viewmodel` (`Viewmodel { fov_y, near, far }`) and `Scene::add_viewmodel`: instances in the camera's space drawn with their own lens in front of the world (the world's depth takes 0 to 0.5, the viewmodel's 0.5 to 1), lit by the scene's sun, shadows, lights and environment; lists apart from the world's, no shadows cast. docs/RENDER3D.md section Waffe vor der Kamera
- Particles (G7f part 2): `Scene::set_particles`, `add_emitter` (`Emitter`: seed, additive or alpha, discs or streaks, life, speed, cone, size and colour over life, gravity, drag, the viewmodel's space), `emit`, `set_time`, `clear_particles`: born on the CPU from the emitter's seed (the same on every device), moved in closed form in a compute shader, sorted back to front on the GPU, one indirect draw; no read-back. docs/RENDER3D.md section Partikel

### Changed

- The camera's culling without the occlusion test and the shadow cascades claim the places of their lists in four turns by list: lists that share warps (opaque and MASK instances of a mesh, a few meshes in turn) no longer cost an atomicAdd per instance (10 % MASK: 0.145 to 0.067 ms for 1 million cubes). docs/RENDER3D.md section Zähler der Listen
- `render3d::Camera` has a field more, `ortho` (None: perspective as before); code that builds a `Camera` with all its fields adds `ortho: None` (`Camera::look_at` sets it)

### Fixed

- Textures taller than wide are resampled columns first: rows first held height x size floats (a 1 x 32768 texture: 1 GiB for 128 KB of texels).
- `Scene`: a buffer that could not be made while growing makes every capacity start again at the next frame (the raised ones no longer fit the buffers); `wait_slot` after a timeout waits again at the next call (it went on writing into a slot the GPU still used).
- A scene whose draw records grew without its list_first (once detail levels had made that grow first) drew nothing: the culling counted into the old records
- `FrameStats::visible` and `late` of a scene without blended instances added what the read-back's place of the blended draw happened to hold (G7f part 1; on llvmpipe garbage, elsewhere usually 0): it is cleared now

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

First release on crates.io.

### Added

- `Renderer` and `Scene`: meshes in shared pools (`Renderer::add_mesh`), a table of materials,
  scenes of instances (`Instance`, `Transform` with an `f64` position) relative to an origin
  (`Scene::set_origin`), a `Camera` with reverse Z, and one instance list per mesh, side and alpha
  class, filled by a compute shader: one multi-draw per side, class and pass on level `Voll`, one
  indirect draw per list on `Basis`. `RendererDesc` (`#[non_exhaustive]`, from `RendererDesc::new`)
  with `frames_in_flight` and `encode_srgb`.
- Materials after glTF 2.0's metallic-roughness model (`Material`) with its five textures and the
  alpha mode MASK (`AlphaClass`); textures in one array per size class from `CLASS_MIN` to
  `CLASS_MAX` and colour space (`Renderer::add_texture`, `TextureData`, `mip_chain`), glTF's wrap
  and filter modes in the shader (`Sampling`, `Wrap`, `Mip`).
- Mirrored instances: a transform with a negative determinant is drawn by a second pipeline with
  its faces turned the right way.
- Physically based shading after glTF 2.0 Appendix B (GGX, separable Smith, Schlick) with normal
  maps, exposure and Khronos PBR Neutral tone mapping (`Scene::set_exposure`,
  `Scene::set_tonemap`); a sun in lux (`Sun`), point and spot lights after `KHR_lights_punctual`
  (`Scene::set_lights`, `Light`), environment light from spherical harmonics and a prefiltered cube
  (`Renderer::set_environment` with a procedural `Sky` or six faces, `Scene::set_environment`).
- Cascaded shadow maps of the sun (`Scene::set_shadows`, `Shadows`): up to `CASCADES_MAX`
  cascades, culled on the GPU, snapped to the texel grid, filtered with PCF, with MASK's holes;
  `Shadows::DEFAULT` for level `Voll`, `Shadows::BASIS` and `Shadows::for_level(level)` for the
  rest.
- An occlusion test against a depth pyramid in two phases (`Scene::set_occlusion`, on by default
  when the depth buffer can be sampled), and detail levels per mesh (`Renderer::set_lods`, `Lod`,
  `LODS_MAX`): a simpler mesh, or none, below a projected size.
- sRGB encoding in the fragment shader for 8-bit colour targets without sRGB, such as a window's
  swapchain (`RendererDesc::encode_srgb`, on by default).
- `render3d::gltf::upload`: a jflib-gltf `Document` on the GPU, its primitives as meshes, its
  images as textures, its materials, and the instances, lights and suns of a scene (`Imported`).
- The same WGSL on Vulkan, Metal and WebGPU (`CULL_WGSL`, `DRAW_WGSL` and the others, compiled
  ahead as `*_SPV`); `FrameStats`, `UploadInfo` and `ListInfo` for measuring.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
