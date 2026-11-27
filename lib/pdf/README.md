# jflib-pdf

A PDF writer and reader for [JFLib](https://jflib.org). The writer draws documents page by page
on a canvas, with real paths, embedded font subsets, images, links and bookmarks, instead of one
raster image per page; JFDiagrammer exports its drawings with it, jfWord can use it for its
documents as well. The reader (`pdf::read`) opens PDF files of every version, encrypted and
damaged ones too, and gives their pages, the text with the position of every glyph, outlines,
links, form fields and attachments, and draws the pages as pixels. It is plain Rust, without the
standard library and without a line of `unsafe` code.

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

## What it reads

- **Files** of PDF 1.0 to 2.0: classic cross-reference tables and cross-reference streams,
  object streams, incremental updates and hybrid files. A file whose table is wrong or missing,
  or that was cut off, is rebuilt from its objects, as Acrobat, pdf.js and Poppler do.
- **Encryption** of the standard security handler: RC4 with 40 to 128 bits, AES-128 and AES-256
  (revisions 2 to 6), with the user or the owner password, or without one when only the owner
  password is set. The permissions are reported, not enforced.
- **Streams** with FlateDecode (raw DEFLATE too, and damaged data up to the damage),
  LZWDecode, ASCIIHexDecode, ASCII85Decode, RunLengthDecode and the PNG and TIFF predictors.
  An image filter (DCT, JPX, JBIG2, CCITTFax) ends the chain: `decode_until_image` gives the JPEG
  or JPEG 2000 file as it is.
- **Pages** with their boxes, rotation and inherited resources, and their labels ("iv", "A-3").
- **Text**: every glyph with its characters, position, advance and size, and each page's text
  in reading order, laid out as Poppler's `pdftotext -raw` does (words apart where the glyphs
  are, Hebrew and Arabic in reading order, fake bold once). Simple fonts through their ToUnicode
  map, their encoding and the Adobe Glyph List; Type 0 fonts through their CMaps; Type 3 fonts;
  the 14 standard fonts with their widths; text in form XObjects.
- **Navigation**: the document information, the XMP metadata, the language, the outline with
  explicit and named destinations, links and other annotations with their actions.
- **Forms**: every field of the interactive form with its full name, kind, value, options and
  widgets; for a signature field what the signature states (it is not verified).
- **Attachments**: embedded files with their name, MIME type, description, size and
  relationship (as ZUGFeRD and Factur-X invoices carry their XML), and their data.

A damaged or hostile file gives an error or less than it claims, never a panic, a hang or memory
without end: nesting, the depth of references, page trees, outlines and form XObjects are
bounded, every decoded stream is at most `Options::max_stream` bytes (256 MiB). Not supported:
the predefined CMaps of East Asian fonts other than Identity and the Unicode ones, verifying
signatures, and changing and saving a document.

## What it draws

`Document::render` draws a page into a `gfx::Surface` at any resolution, the whole page or a
region of it (tiles for a viewer), over paper of any colour or none:

- **Paths**: fills with the non-zero and even-odd rule, strokes with caps, joins, miter limit and
  dashes in user space (also under a matrix that scales unevenly), thin rules and frames on whole
  pixels as Poppler draws them, clipping paths and text clipping.
- **Colours** in every colour space: Device, CalGray, CalRGB, Lab, ICCBased (by its component
  count or its alternate), Indexed, Separation and DeviceN through their tint transforms, and
  functions of all four types (sampled, exponential, stitching, PostScript calculator). CMYK as
  printing inks look (the Neugebauer model over the colours of the SWOP inks).
- **Text** through the glyph outlines of the embedded fonts -- Type 1, CFF (with seac accents),
  TrueType, OpenType, CID-keyed fonts, Type 3 -- and for fonts that are not embedded substitutes
  from a `FontSource` (by default the system's fonts: URW's Nimbus, Liberation or the Windows
  fonts for the 14 standard fonts); every text render mode.
- **Images** of every bit depth and colour space with Decode, stencil masks, colour-key and
  explicit masks, soft masks with Matte, inline images, JPEG (CMYK too) and CCITT Group 3 and 4;
  shrunk images averaged over the pixels they cover, enlarged ones sampled bilinearly.
- **Shadings** of all seven types, with `sh` and as patterns; tiling patterns, coloured and
  uncoloured.
- **Transparency**: alpha, the sixteen blend modes, soft masks (luminosity and alpha, with
  backdrop and transfer function), transparency groups.
- **Annotations** through their appearance streams (with their states and flags), and optional
  content (the default configuration, membership dictionaries).

Pages agree with Poppler's `pdftoppm` (tools/pdf_render_check.py), and drawing takes about as
long. JPEG 2000 images (`JPXDecode`) are decoded by
[`jflib-img-jp2`](https://crates.io/crates/jflib-img-jp2): their own size and bit depth, the colour
space of the JP2 header when the dictionary names none, `SMaskInData`. Not drawn: JBIG2 images,
knockout groups and groups that are not isolated (drawn as isolated ones), overprint. The work of a page is bounded: a hostile page gives
`Error::Limit`, never a hang.

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

And reading one:

```rust
use jflib::pdf::read::{Action, Document, Options};

let doc = Document::open(bytes)?;                        // Error::Password when it needs one
let doc = Document::open_with(bytes, &Options { password: "geheim".into(), ..Options::default() })?;
for i in 0..doc.page_count() {
    let (w, h) = doc.page(i)?.size();                    // points, as the page is shown
    let text = doc.text(i)?;                             // the text in reading order
}
for g in doc.page_text(0)?.items {                       // every glyph: text, x, y, size
    if g.text == "€" { /* g.x, g.y */ }
}
let title = doc.info().title;
for o in doc.outlines() {
    if let Some(Action::GoTo(dest)) = o.action { /* dest.page */ }
}
for f in doc.fields() { /* f.name, f.kind, f.value */ }
for a in doc.attachments() {
    let xml = doc.decode(a.stream.as_ref().unwrap())?;  // "factur-x.xml", "text/xml"
}
```

And drawing one:

```rust
use jflib::pdf::read::{Document, RenderOptions};

let doc = Document::open(bytes)?;
let (w, h) = doc.page_pixels(0, 150.0)?;                // the page's size at 150 dpi
let page: gfx::Surface = doc.render(0, &RenderOptions { dpi: 150.0, ..RenderOptions::default() })?;
let tile = doc.render(0, &RenderOptions { dpi: 300.0, region: Some([0, 0, 512, 512]), ..RenderOptions::default() })?;
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

MIT or Apache-2.0, at your choice. The reader carries the Adobe Glyph List (BSD 3-Clause License,
Copyright Adobe) and the advance widths of the Liberation fonts Sans and Serif (SIL Open Font
License 1.1); their texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/pdf/licenses) folder.
