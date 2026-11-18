# jflib-pdf

A vector PDF writer for [JFLib](https://jflib.org): documents drawn page by page on a canvas,
with real paths, embedded font subsets, images, links and bookmarks, instead of one raster image
per page. It is plain Rust, without the standard library and without a line of `unsafe` code.
JFDiagrammer exports its drawings with it; jfWord can use it for its documents as well.

## What it writes

- **Pages** of any size (A4, Letter or your own), each with a canvas whose origin is the top left
  corner with y downwards, or PDF's own bottom left with y upwards.
- **Vector graphics:** paths of lines, cubic and quadratic curves, rectangles, rounded
  rectangles, ellipses and arcs; fills with the non-zero or even-odd rule; strokes with width,
  dash pattern, caps, joins and miter limit; transforms, clipping, separate alpha for fills and
  strokes, RGB colours.
- **Gradients and hatches:** axial and radial gradients with any number of colour stops (hard
  edges too) and alpha per stop, either filling a path or painting the whole clip; varying alpha
  becomes a soft mask. Hatches fill a path with parallel lines of any angle, spacing, width and
  colour; two hatches cross.
- **Images:** JPEG files go in unchanged (DCTDecode, also progressive and CMYK); any other pixels
  (gray, RGB, RGBA or a `gfx::Surface`) are compressed with PNG predictors and DEFLATE, their
  alpha becomes a soft mask.
- **Text** in embedded subsets of TrueType and OpenType fonts (glyf and CFF outlines), as CID
  fonts with Identity-H encoding. Lines are shaped with JFLib's shaper (kerning, ligatures, marks,
  Arabic joining, bidi, fallback fonts), every glyph lands where the shaper put it, and a
  ToUnicode map keeps search and copy working. Text can be filled, stroked or invisible.
- **Links** to URIs and to places in the document, and nested **bookmarks** (outlines).
- The **document information** (title, author, subject, keywords, dates) and the same as XMP
  metadata, and the document's language.

The output is PDF 1.7 with a classic cross-reference table. It is deterministic: the same
document always gives the same bytes, with no time stamp unless you set one. Not supported:
PDF/A, encryption and forms. Fonts whose licence (OS/2 `fsType`) forbids embedding are refused.

## How it is used

```rust
use jflib::pdf::{Dest, Document, FillRule, Hatch, Origin, Rgb, Shading, A4};

let mut doc = Document::new();
let page = doc.add_page(A4.0, A4.1);                     // points, 1/72 inch
let font = doc.add_font(font_bytes, 0)?;                 // a .ttf, .otf or a collection's face
{
    let mut c = doc.canvas(page, Origin::TopLeft);
    c.set_fill_color(Rgb::hex(0x2060C0));
    c.rounded_rect(50.0, 50.0, 200.0, 80.0, 8.0);
    c.fill(FillRule::NonZero);
    c.rect(50.0, 200.0, 200.0, 40.0);                    // a gradient from left to right
    c.fill_shading(FillRule::NonZero, &Shading::axial([50.0, 0.0], [250.0, 0.0],
        &[(0.0, Rgb::hex(0x2060C0), 1.0), (1.0, Rgb::hex(0x2060C0), 0.0)]));
    c.rect(50.0, 260.0, 200.0, 40.0);                    // hatched at 45 degrees
    c.fill_hatch(FillRule::NonZero, &Hatch::new(0.785, 6.0, 1.0, Rgb::BLACK));
    c.set_fill_color(Rgb::BLACK);
    let w = c.text(font, 12.0, 50.0, 160.0, "Hallo, Welt");
    c.link_uri(50.0, 148.0, w, 16.0, "https://jflib.org");
}
doc.add_outline(None, "Start", Dest::Fit(page));
let bytes = doc.write()?;
```

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::pdf`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

jflib-pdf is new: the first version of the writer with pages, vector graphics, images, embedded
font subsets, links, bookmarks and metadata.
Everything is in the [changelog](https://docs.rs/crate/jflib-pdf/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#pdf): every public
  item of `pdf`, also on [docs.rs](https://docs.rs/jflib-pdf)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  decision V5 and milestone JD12

## License

MIT or Apache-2.0, at your choice.
