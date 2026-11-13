# jflib-img-svg

SVG images and icons for [JFLib](https://jflib.org). A document is parsed once and then drawn as
often as needed, at any size and sharp at every one, through the path rasterizer of
[`jflib-gfx`](https://crates.io/crates/jflib-gfx). It is plain Rust, without the standard library
and without a line of `unsafe` code.

## What it draws

A practical subset of SVG 1.1 and SVG Tiny. Structure:
nested `svg` viewports with `viewBox` and `preserveAspectRatio`, `g`, `defs`, `use` with `symbol`,
`switch` and `a`. Shapes: `path` with every command including arcs, `rect` with rounded corners,
`circle`, `ellipse`, `line`, `polyline` and `polygon`, under any transform. Paint: colours as hex,
`rgb()` and `hsl()` values, the 148 names and `currentColor`, opacities, fill rules, stroke width,
caps, joins, miter limit and dashes, linear and radial gradients with spread modes, focal points and
inheritance through `href`. Group opacity is drawn in layers, `clipPath` clips with shapes, text or
`use`. Style comes from presentation attributes, the `style` attribute and `<style>` sheets with
type, class, id and attribute selectors joined by descendant and child combinators. Lengths may be
given in px, pt, pc, mm, cm, in, em, ex and percent. `.svgz` files are inflated first.

Text is drawn through a font engine the app supplies (`TextHook`): one line per text chunk, with
`tspan`, the first x, y, dx and dy values, `text-anchor`, and the font's size, family, weight and
style. Without one, text is left out. Images embedded as `data:` URIs are decoded by whatever
raster codecs are registered in [`jflib-img-core`](https://crates.io/crates/jflib-img-core).

What it does not do is left out of the picture rather than guessed: patterns (their fallback
colour is used), masks, markers, filters, text on a path, per-glyph positions, letter spacing, CSS
pseudo-classes and `@media`, animation and scripts. It never fetches anything: external files are
not loaded.

## Safe with files from anywhere

Malformed input never panics. The reader is forgiving (unclosed tags, unquoted attributes, a path
broken halfway draws up to the error, and `Svg::truncated` tells you), and it is bounded: 32 MB per
document, also after inflating, 250,000 elements, 256 levels of nesting and 4,096 CSS rules; entity
references expand only once. Selector matching has a step budget, and a cascade that would copy more
than 128 MB of declarations makes parsing fail with `ERR_LIMIT`. Drawing is bounded too: a million
element visits, 64 levels of nesting, cycles of `use` cut, and a work budget that grows with the
area drawn (pixels painted, layers, masks and images allocated, path points and elements walked), so
the cost of a render stays in proportion to its size. Embedded images share one pixel budget per
document.

## How it is used

In an app with a window the document is parsed with `Svg::parse` (also reachable as `ui::Svg`) and
handed to the UI: `ui.svg` of [`jflib-ui`](https://crates.io/crates/jflib-ui) draws it as a picture
or an icon at its layout size, coloured with the theme's text colour where it says `currentColor`,
and `ui.image_view` lets the user zoom and pan it, redrawing only what is visible. The runtime also
registers SVG in img_core's format table at start-up, so `img_core::io::load_ex` returns an `.svg`
or `.svgz` file as a bitmap of its intrinsic size; a program without that runtime calls
`img_svg::register()` once.

To draw it yourself:

```rust
use jflib::img_svg::{Opts, Svg};

let Ok(doc) = Svg::parse(&bytes) else { return };            // an ERR_* code of img_core
let (w, h) = doc.size();                                     // the intrinsic size in px
let icon = doc.rasterize(48, 48, &mut Opts::default());      // Ok(a new premultiplied surface)
doc.render_rect(&mut surface, 10.0, 10.0, 96.0, 96.0, None, &mut Opts::default());
```

`render_rect` maps the `viewBox` into a rectangle of a surface you already have, `render` draws
through any matrix (zoomed, rotated), and both can be limited to a clip rectangle. `Opts` carries
the colour of `currentColor` and the font engine for text; in a UI, `ui::SvgText` sets text with
the app's fonts. `img_svg::rasterize(bytes, w, h)` parses and draws in one call.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_svg`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Two limits close ways in which a small hostile file could cost minutes and gigabytes. The
selectors of a style sheet's list now share one copy of their declarations, every declaration
counts toward the cascade's budget, and a cascade beyond 128 MB fails with `ERR_LIMIT`. Rendering
is bounded by its work, not only by the number of elements it visits. Everything is in the
[changelog](https://docs.rs/crate/jflib-img-svg/latest/source/CHANGELOG.md).

## Documentation

- [SVG images and the image viewer](https://docs.rs/jflib/latest/jflib/guide/index.html#31-svg-images-and-the-image-viewer),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_svg), also on
  [docs.rs](https://docs.rs/jflib-img-svg)

## License

MIT or Apache-2.0, at your choice.
