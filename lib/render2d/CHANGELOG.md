# Changelog of jflib-render2d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- Text (G6c): fonts from TrueType or OpenType bytes (`Scene::add_font`, parsed by lib/text), text styles like DXF's STYLE (`TextStyle`: font, fallback fonts, a fixed height, width factor, oblique angle, drawing units or pixels), single lines with DXF TEXT's alignments (`Shape::Text`, `Text`, `HAlign`, `VAlign`, backward, upside down, aligned and fit between two points) and the basics of MTEXT (`Shape::MText`, `MText`: line breaks, a wrapping width, line spacing, the nine attachment points), shaped by HarfRust through lib/text_shape (kerning, ligatures, bidi, Arabic joining, fallback fonts per run). Glyphs come from an atlas of multi-channel signed distance fields made on the CPU from the outlines (two size classes, tiles on demand, the same bytes on every target); the coverage comes from the distance through the map's Jacobian, so width factors, oblique angles and uneven inserts stay sharp, and every pixel blends once per text; below 7 px per em a text is drawn as one bar per word, from 160 px per em as a fill of its outlines (texts in blocks stay glyphs); texts work in blocks like any entity. `text_layout` and the free `layout_text` / `layout_mtext` give the layout (the extent for picking, the outlines for a CPU path), `text_info`, `atlas_hash`. docs/RENDER2D.md section Text

## [0.2.0] - 2026-11-14

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
