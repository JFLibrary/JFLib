# Changelog of jflib-render3d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
