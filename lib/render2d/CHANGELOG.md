# Changelog of jflib-render2d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The CPU path (G6e1): a scene draws without a GPU through lib/gfx (`Scene::draw_cpu` into a `gfx::Surface`, `cpu_info`, `CpuInfo`, `set_cpu_single`) with the camera, the painter's order, the visible layers and the look of a frame on the GPU, highlights and the overlay included: entities found through the R-tree, every point placed relative to the view in f64 and clipped to a guard band before lib/gfx (exact at 1e7 and 1e-6 units per pixel), strokes as convex pieces adding up to the GPU's union (joins cut at the bisectors, else the union of both strips less their intersection), arcs within 1/128 px near the view, curves and fills as the GPU flattens them, dash patterns and hatches cut in f64 with the GPU's flat tones, text from its glyphs' outlines and bars below 7 px per em. The same bytes on every target, also in row bands on lib/gfx's parallel loop. A scene without a device (`Scene::new_cpu`) does everything but a GPU frame -- picking, snapping and highlighting included -- and is put on a renderer's device and taken off again (`attach`, `detach`, `attached`): the drawing survives a lost device. docs/RENDER2D.md section Der CPU-Weg
- `render2d::dxf` (G6e2): a DXF drawing into a scene as lib/dxf reads it (`load`, `read`, `Options`, `Loaded`): a render2d layer per run of entities on one DXF layer (the file's order is the painter's order, as in AutoCAD; `set_layer_visible` switches all runs of a layer), styles from colours (index 7 after the background), linetypes ($LTSCALE times the entity's scale; `linetype_pattern`), lineweights ($LWDISPLAY) and transparency resolved by layer and by block (entities on layer 0 of a block take what is by layer from the insert), blocks as render2d blocks (DIMENSION and ACAD_TABLE through their anonymous blocks, MINSERT arrays, attributes as texts), object coordinate systems projected onto the plane, polyline bulges as arcs (one rational quadratic spline where PLINEGEN runs a pattern across the vertices, under a tilted normal and for a constant width), varying widths as fills, hatches with their boundary edges and pattern lines (more than four families: several fills of one boundary, at most 64 families), SOLID, TRACE, 3DFACE, WIPEOUT as fills, SHX and TrueType fonts drawn in substitutes of the repository with a width correction, MTEXT's format codes laid out into single-line texts (runs of font, height, width, slant and colour; under-, over- and strike-through; stacked fractions; paragraph alignment, indents and tabs; columns; background fill); per DXF kind what was drawn, skipped in paper space, hidden, unsupported, refused and approximated; `group_of` / `members` (the render2d entities one DXF entity became). docs/RENDER2D.md section DXF

### Fixed

- In Firefox (WebGPU through wgpu: Naga, HLSL, DXC without IEEE strictness, which may regroup float sums) the f32 pairs lost their low bits: the compiler regrouped the error terms of the two-sums and of Dekker's products, lines at 1e-6 units per pixel vanished, the drawing at 1e7 moved by pixels. Every step of the exact arithmetic now goes through opq (common.wgsl: two_sum, split_f, two_prod, dd_add, dd_mul, rel2, dd_sqrt; stroke.wgsl: arc_from); the SPIR-V of the visibility and vertex shaders grows, the images on Vulkan, Metal and in Chromium stay byte for byte (G6e3)
- A frame with more changed records than one dispatch of the scatter shader reaches (Metal, WebGPU and many phones: 65 535 workgroups of 64, 4 194 240 records; the first frame of a city's drawing) was refused at finish; the change buffer is now bound and dispatched in pieces of what a dispatch and a storage binding reach, with a barrier between them. Where one dispatch reaches it is one piece and the same commands as before (G6e3)

## [0.2.1] - 2026-11-16

### Added

- Text (G6c): fonts from TrueType or OpenType bytes (`Scene::add_font`, parsed by lib/text), text styles like DXF's STYLE (`TextStyle`: font, fallback fonts, a fixed height, width factor, oblique angle, drawing units or pixels), single lines with DXF TEXT's alignments (`Shape::Text`, `Text`, `HAlign`, `VAlign`, backward, upside down, aligned and fit between two points) and the basics of MTEXT (`Shape::MText`, `MText`: line breaks, a wrapping width, line spacing, the nine attachment points), shaped by HarfRust through lib/text_shape (kerning, ligatures, bidi, Arabic joining, fallback fonts per run). Glyphs come from an atlas of multi-channel signed distance fields made on the CPU from the outlines (two size classes, tiles on demand, the same bytes on every target); the coverage comes from the distance through the map's Jacobian, so width factors, oblique angles and uneven inserts stay sharp, and every pixel blends once per text; below 7 px per em a text is drawn as one bar per word, from 160 px per em as a fill of its outlines (texts in blocks stay glyphs); texts work in blocks like any entity. `text_layout` and the free `layout_text` / `layout_mtext` give the layout (the extent for picking, the outlines for a CPU path), `text_info`, `atlas_hash`. docs/RENDER2D.md section Text
- Picking, area selection, object snaps, highlighting and an overlay (G6d): a spatial index of its own (an R-tree of f64 boxes, built in one go by the first query and kept in step with every change; each block indexed once in its own units, queries go down through the inserts' inverse transforms, arrays and nesting included) behind `Scene::pick` (exact against what is drawn: widths in pixels or units, caps, joins and miters, arcs and curves themselves rather than their flattening, fills even-odd with holes, hatched areas, texts by their quad; topmost first in the painter's order; inside blocks the insert and the path of nested inserts down to the entity; hidden layers skipped, a filter for an app's layer lock), `Scene::select` (window and crossing by rectangles and polygons, an insert as a whole) and `Scene::snap` / `snaps` (endpoint, midpoint, centre, quadrant, intersection, perpendicular, tangent, nearest and insertion; aperture, kinds, a base point, AutoCAD's priority), all in f64 and exact at 1e7 and 1e-6 units per pixel; `grip_points`; highlighting of selected and hovered entities and of inserts as a whole (`set_highlight`, `HL_SELECTED`, `HL_HOVER`) through a mask of two bits per record and instance, of which only the changed words go up, never geometry (without a highlight every pipeline, its SPIR-V and every image stay as they were); an overlay made anew every frame (`Scene::overlay`: rubber bands, window and crossing boxes, grips, snap markers, fills; in drawing units or pixels) drawn after the scene with the same anti-aliased stroke and fill code from its own per-frame buffers. docs/RENDER2D.md sections Auswahl und Fang, Hervorheben, Überlagerung

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
