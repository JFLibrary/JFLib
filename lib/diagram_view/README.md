# jflib-diagram-view

Draws the pages of diagrams of the Visio kind, built by [JFLib](https://jflib.org). The model
and its display list come from [`jflib-diagram-core`](https://crates.io/crates/jflib-diagram-core):
a page as fills, lines, line ends, texts laid out and pictures in painter's order, in paper
inches. This crate puts that list on a scene of
[`jflib-render2d`](https://crates.io/crates/jflib-render2d) and draws what the scene cannot hold
-- gradient fills and pictures -- with [`jflib-gfx`](https://crates.io/crates/jflib-gfx) between
the scene's layers. It is plain Rust, without the standard library and without `unsafe` code.

## Why a crate of its own

The display list lives in `diagram_core`, which has no dependencies, so that the exports of
`diagram_io` (SVG, PDF) and the hit tests of an editor read the same list. Drawing it needs
render2d (and through it the GPU layer), gfx and the image codecs; neither the model nor the
file formats should pull those in, and the UI library knows nothing of diagrams. An app connects
a `PageView` to its canvas.

## How it is used

```rust
use jflib::diagram_core::render::Options;
use jflib::diagram_io::vsdx;
use jflib::diagram_view::{Class, FontSet, PageView};

let doc = vsdx::read(&bytes, &vsdx::Limits::VSDX)?.doc;
let mut fonts = FontSet::new();
fonts.add(Class::Sans, false, false, sans_regular);   // TrueType or OpenType bytes
fonts.add(Class::Sans, true, false, sans_bold);
let page = doc.foreground_pages().next().unwrap();
let mut view = PageView::new(&doc, page, &mut fonts, &Options::default());
let image = view.render(1600, 1200);                   // the page fitted, an XRGB surface
// or into a view of a window: view.draw(dc, sf, rect, &camera)
```

Fonts come from the app: its style's fonts, and installed fonts through a `FontSource` it
implements. A family the set does not have is drawn with the set's font of its class (sans,
serif, monospaced), stretched so that a sample line is as wide as in the missing font; italics
without their own face lean. `PageView::substitutions` names what was substituted.

Pictures of foreign shapes: PNG and JPEG are decoded, EMF, WMF and other formats show a
placeholder. A picture larger than 8192 pixels on a side is not decoded.

## What's new in 0.2.1

The crate is new: it draws the viewer stage (JD4) of
[JFDiagrammer](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_view): every
  public item of `diagram_view`
- [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
  (in German): the plan the crate belongs to

## License

MIT or Apache-2.0, at your choice.
