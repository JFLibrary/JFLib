# jflib-diagram-io

The file formats of [JFLib](https://jflib.org)'s diagram model: Visio's VSDX family (drawings,
stencils and templates) in both directions, later the older XML formats and the binary stencils,
and exports of diagrams. It reads and writes the documents of
[`jflib-diagram-core`](https://crates.io/crates/jflib-diagram-core). It is plain Rust, without
the standard library and without a line of `unsafe` code.

**The crate is a frame for now and has no API yet.** The codecs are being built in stages; the
plan is [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German).

## What it will hold

- Reading `.vsdx`, `.vssx` and `.vstx`, and the macro-enabled `.vsdm` and `.vssm`, whose macro
  part is passed through unchanged and never run.
- Writing the model back with every part it does not know, so a file keeps what it had.
- Writing new drawings and stencils from scratch.
- Exports of pages as SVG, PNG and PDF.
- Later: `.vdx` and `.vsx` (the XML formats of 2003 to 2010) and importing `.vss` and `.vsd`.

The format follows the public specification [MS-VSDX]. Every file is treated as untrusted input:
sizes, counts and nesting are bounded, external links are kept as data and never followed, and
nothing in a file is ever run.

[MS-VSDX]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-vsdx/

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::diagram_io`. All of JFLib's crates share
one version and are released together.

## What's new in 0.2.1

jflib-diagram-io is new in this release, as an empty frame: the crate exists so that the codecs
can grow in it; it has no public items yet.
Everything is in the [changelog](https://docs.rs/crate/jflib-diagram-io/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_io): every
  public item of `diagram_io`, also on [docs.rs](https://docs.rs/jflib-diagram-io)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  the stages, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
