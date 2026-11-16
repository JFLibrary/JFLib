# jflib-diagram-core

The diagram model of [JFLib](https://jflib.org): documents of pages, layers, masters and shapes
in the way Visio stores them, with ShapeSheet cells, inheritance and an edit API with undo. It
is the model behind JFDiagrammer and serves any app that draws or reads such diagrams. It is
plain Rust, without the standard library and without a line of `unsafe` code.

The model is being built in stages; the plan is
[apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German). The document model is there; the formula engine, geometry, glue, connector routing
and layout follow.

## What it holds

- **The ShapeSheet is the model.** Every cell is kept as the file has it: formula, cached value,
  unit and error, and where its formula comes from (the sheet itself, the master shape, a style,
  the default, or a deleted row). Documents, pages and shapes are views on these cells, so a
  round trip through a file loses nothing.
- **Documents** with foreground and background pages, layers, masters (also stencils without
  pages), shapes, groups, foreign objects and guides, styles, theme parts, `Connect` entries,
  hyperlinks, shape data and user cells. What a file holds and the model does not know stays at
  the place it stood.
- **Inheritance** from shape to master shape to the styles of its line, fill and text, with
  rows and sections that a shape deletes for itself.
- **Units:** values in inches, radians and days inside, as in the file, and conversion for
  every unit code a cell can show (millimetres, points, degrees, feet, ciceros ...).
- **Edits in transactions:** each transaction is one undo step, kept as the inverse of what it
  changed, so undo costs what the change costs, not what the drawing weighs.

```rust
use jflib::diagram_core::{CellAddr, Document, SheetRef, Unit};

fn move_right(doc: &mut Document, shape: SheetRef) {
    let pin_x = CellAddr::single(doc.intern("PinX"));
    let x = doc.resolve(shape, &pin_x).num().unwrap_or(0.0);   // inches, inherited or own
    let mut tx = doc.transaction("Move");
    tx.set_length(shape, &pin_x, x * 25.4 + 10.0, Unit::Millimeters).unwrap();
    if let Some(mut change) = tx.commit() {
        change.undo(doc);   // and change.redo(doc)
    }
}
```

The file formats are a crate of their own,
[`jflib-diagram-io`](https://crates.io/crates/jflib-diagram-io); the canvas is the diagram
widget of [`jflib-ui`](https://crates.io/crates/jflib-ui), whose undo history takes a
transaction's change as one step.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::diagram_core`. All of JFLib's crates share
one version and are released together.

## What's new in 0.2.1

jflib-diagram-core is new: the document model of pages, layers, masters, shapes and styles on
ShapeSheet cells, with inheritance, units and an edit API with undo and redo. The formula engine,
geometry and routing are still to come.
Everything is in the [changelog](https://docs.rs/crate/jflib-diagram-core/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_core): every
  public item of `diagram_core`, also on [docs.rs](https://docs.rs/jflib-diagram-core)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  the stages, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
