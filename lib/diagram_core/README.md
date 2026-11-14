# jflib-diagram-core

The diagram model of [JFLib](https://jflib.org): documents of pages, layers, masters and shapes
in the way Visio stores them, with ShapeSheet cells, a formula engine, geometry, connector
routing and layout. It is the model behind JFDiagrammer and serves any app that draws or reads
such diagrams. It is plain Rust, without the standard library and without a line of `unsafe`
code.

**The crate is a frame for now and has no API yet.** The model is being built in stages; the plan
is [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German).

## What it will hold

- **The ShapeSheet is the model.** Every cell is kept as the file has it: formula, cached value,
  unit and where it comes from (local, inherited from a master or a style, deleted). Documents,
  pages and shapes are views on these cells, so a round trip through a file loses nothing.
- Documents with foreground and background pages, layers, masters, shapes and groups, styles and
  theme data, and the inheritance from shape to master shape to style to document.
- An edit API with transactions, so that every change can be undone.
- A formula engine with incremental recalculation, units and inheritance.
- Geometry, glue, connectors routed around obstacles, and layout.

The file formats are a crate of their own,
[`jflib-diagram-io`](https://crates.io/crates/jflib-diagram-io); the canvas is the diagram
widget of [`jflib-ui`](https://crates.io/crates/jflib-ui).

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::diagram_core`. All of JFLib's crates share
one version and are released together.

## What's new in 0.2.1

jflib-diagram-core is new in this release, as an empty frame: the crate exists so that the model
can grow in it; it has no public items yet.
Everything is in the [changelog](https://docs.rs/crate/jflib-diagram-core/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_core): every
  public item of `diagram_core`, also on [docs.rs](https://docs.rs/jflib-diagram-core)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  the stages, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
