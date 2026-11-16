# jflib-diagram-io

The file formats of [JFLib](https://jflib.org)'s diagram model: Visio's VSDX family (drawings,
stencils and templates), later the older XML formats and the binary stencils, and exports of
diagrams. It reads and writes the documents of
[`jflib-diagram-core`](https://crates.io/crates/jflib-diagram-core). It is plain Rust, without
the standard library and without a line of `unsafe` code.

The codecs are being built in stages; the plan is
[apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German). Reading is there; writing and the exports follow.

## What it holds

- **Reading the VSDX family:** `.vsdx`, `.vstx` and `.vssx`, and the macro-enabled `.vsdm`,
  `.vstm` and `.vssm`, told apart by the main part's content type. The styles, the document
  sheet, colours and fonts, the pages with their shapes, texts and `Connect` entries, the
  masters with their icons, the themes, `windows.xml`, the document properties, pictures,
  comments and data recordsets all come into the model; what it does not know (unknown
  attributes, elements, text, parts and relationships) is kept at the place it stood.
- **Nothing recomputed on opening:** values stay the cached values of the file, connectors are
  not rerouted, so a drawing looks as it was saved.
- **Macros are never run.** The macro project of a macro-enabled file is kept as bytes; a file
  that may not carry macros but does is refused.

```rust
use jflib::diagram_io::vsdx::{self, Limits};

/// pages and masters of a .vsdx / .vssx / ... file, or why it was refused
fn summary(bytes: &[u8]) -> Result<(usize, usize), vsdx::Error> {
    let loaded = vsdx::read(bytes, &Limits::VSDX)?;   // loaded.format: which of the six files
    let doc = &loaded.doc;                              // a diagram_core::Document
    Ok((doc.page_count(), doc.masters().count()))      // loaded.warnings: what was odd but kept
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
