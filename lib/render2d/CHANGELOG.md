# Changelog of jflib-render2d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

First release on crates.io.

### Added

- `Renderer` and `Scene`: a retained scene of entities with stable ids on layers (`Layer`: order,
  visibility, transparency) and styles, of which only what changed goes up (`Scene::prepare`,
  `Renderer::record`, `Scene::submitted`); a `Camera` with an `f64` centre, a scale in drawing
  units per pixel and a rotation.
- `Shape`: lines, open and closed polylines, arcs and circles, quadratic and cubic Bézier curves,
  ellipses and elliptic arcs, B-splines up to degree 3 (rational ones and DXF's knots included) and
  fills of any number of rings with the even-odd rule.
- Strokes whose coverage the fragment shader computes from the geometry: widths in pixels or
  drawing units (`Unit`), butt, round and square caps, mitre, round and bevel joins, dash patterns
  of up to `DASH_MAX` entries (`Style::dashed`); hatches of up to `HATCH_MAX` line families
  (`HatchLine`, `Style::hatched`), a flat tone below 2 px.
- Blocks (`Scene::add_block`, `Scene::insert_in`, `Shape::Insert` with `Insert`): defined once,
  inserted many times as instances with their own transform, nested up to `BLOCK_DEPTH` levels,
  with arrays, uneven scales and mirroring; styles by block (`BY_BLOCK_COLOR`, `BY_BLOCK_WIDTH`,
  `BY_BLOCK_DASH`) and AutoCAD's transparency per layer, per entity and by block
  (`Transparency`).
- `f64` coordinates as pairs of `f32` on the GPU with the camera subtracted first, visibility in
  compute shaders that keep the painter's order, one multi-draw on level `Voll` and one indirect
  draw per layer on `Basis`; the same WGSL on Vulkan, Metal and WebGPU (`*_WGSL`, `*_SPV`).
- The modules `curve` (curves flattened per zoom band within `curve::FLAT_TOL`) and `fill` (rings
  cleaned, nested, oriented and triangulated in `f64`); `FrameStats`, `UploadInfo` and
  `InstanceInfo` for measuring.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
