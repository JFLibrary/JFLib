# jflib-diagram-io

The file formats of [JFLib](https://jflib.org)'s diagram model: Visio's VSDX family (drawings,
stencils and templates), the older XML formats of Visio 2003-2010 (read), the binary stencils
of Visio 2003-2010 (import), and exports of diagrams. It reads and writes the documents of
[`jflib-diagram-core`](https://crates.io/crates/jflib-diagram-core). It is plain Rust, without
the standard library and without a line of `unsafe` code.

The codecs are being built in stages; the plan is
[apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German). Reading and writing are there; the exports follow.

## What it holds

- **Reading the VSDX family:** `.vsdx`, `.vstx` and `.vssx`, and the macro-enabled `.vsdm`,
  `.vstm` and `.vssm`, told apart by the main part's content type. The styles, the document
  sheet, colours and fonts, the pages with their shapes, texts and `Connect` entries, the
  masters with their icons, the themes, `windows.xml`, the document properties, pictures,
  comments and data recordsets all come into the model; what it does not know (unknown
  attributes, elements, text, parts and relationships) is kept at the place it stood.
- **Writing the VSDX family:** the model goes back into any of the six files with everything
  it did not know at its place, relationship ids kept, deterministic and DEFLATE-compressed; a
  file read and written unchanged reads back as the same model. A drawing can be saved as a
  template, a stencil's masters as a `.vssx`. A document without `windows.xml` gets one, since
  desktop Visio opens no file without it.
- **Reading the XML formats of 2003-2010:** `vdx::read` takes a `.vdx` drawing, `.vsx`
  stencil or `.vtx` template ("DatadiagramML", one XML file with base64 pictures) into the same
  model, so that it is saved as a file of the VSDX family: cells with formula and stored value,
  sections and rows by VSDX's names, font IDs as font names, pictures as parts, masters, pages,
  document properties, windows and the macro project. What a VSDX file has no place for (tab
  stops, solution XML, data connections) is left out with a warning.
- **Importing the binary stencils of 2003-2010:** `vss::read` takes a `.vss` (Visio 2003-2010,
  version 11) into the same model: styles, masters and their shapes with cells, formulas as
  Visio writes them, texts and fields. Microsoft published no specification of this format;
  the reader comes from our own analysis of files, the container (`archive::cfb`) from the open
  specification [MS-CFB]. Nothing is ever written in the binary formats: a stencil read this way
  is saved as `.vssx`.
- **Document properties and the window state:** `props` reads and writes the core, extended and
  custom properties (title, author, company, dates, custom fields), `windows` the drawing window
  (page, zoom, centre, what is shown); `vsdx::remove_macros` takes a macro project out.
- **New documents of its own:** `vsdx::new_document` makes a drawing, template or stencil (A4
  and millimetres or Letter and inches) with JFLib's own styles and document sheet and the own
  DrawingML theme "NetLab"; nothing in it comes from Visio's files.
- **Nothing recomputed on opening:** values stay the cached values of the file, connectors are
  not rerouted, so a drawing looks as it was saved.
- **Macros are never run.** The macro project of a macro-enabled file is kept as bytes and
  written back unchanged; a file that may not carry macros but does is refused.
- **Stencils from text sources:** `stencil::build` compiles the sources of a stencil -- a
  `stencil.txt`, one `.master` file per master with its ShapeSheet sections, optional SVG
  drawings -- into a stencil, and `stencil::build_vssx` into `.vssx` bytes, metric or US: masters
  with names, QuickInfos, keywords and translated cell texts in five languages, every formula
  computed by jflib-diagram-core's formula engine, icons rendered from the shapes themselves,
  name-based GUIDs, the same bytes for the same sources. A fault names the file and the line.
  [JFDiagrammer](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/stencils/README.md)
  builds its stencils with it.

```rust
use jflib::diagram_io::vsdx::{self, Limits};

/// pages and masters of a .vsdx / .vssx / ... file, or why it was refused
fn summary(bytes: &[u8]) -> Result<(usize, usize), vsdx::Error> {
    let loaded = vsdx::read(bytes, &Limits::VSDX)?;   // loaded.format: which of the six files
    let doc = &loaded.doc;                              // a diagram_core::Document
    Ok((doc.page_count(), doc.masters().count()))      // loaded.warnings: what was odd but kept
}

/// a new metric drawing as .vsdx bytes
fn new_drawing() -> Result<Vec<u8>, vsdx::WriteError> {
    let doc = vsdx::new_document(jflib::diagram_core::DocKind::Drawing, vsdx::Measure::Metric);
    vsdx::write(&doc, &vsdx::WriteOptions::DEFAULT)
}
```

The format follows the public specification [MS-VSDX]. Every file is treated as untrusted input:
the ZIP container, the XML nesting and the counts of shapes, cells, rows and sections are bounded
(`Limits`), cycles of styles, masters and background pages are refused, a broken file is an
error and never a panic, and external links are kept as data and never followed.

[MS-VSDX]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-vsdx/

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::diagram_io`. All of JFLib's crates share
one version and are released together.

## What's new in 0.2.1

jflib-diagram-io is new: it reads the six files of Visio's VSDX family into the document model of
`jflib-diagram-core`, keeps what it does not know, and refuses hostile files with a clear error.
Writing and exports are still to come.
Everything is in the [changelog](https://docs.rs/crate/jflib-diagram-io/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_io): every
  public item of `diagram_io`, also on [docs.rs](https://docs.rs/jflib-diagram-io)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  the stages, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
