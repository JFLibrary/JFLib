# jflib-diagram-core

The diagram model of [JFLib](https://jflib.org): documents of pages, layers, masters and shapes
in the way Visio stores them, with ShapeSheet cells, inheritance and an edit API with undo. It
is the model behind JFDiagrammer and serves any app that draws or reads such diagrams. It is
plain Rust, without the standard library and without a line of `unsafe` code.

The model is being built in stages; the plan is
[apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)
(in German). The document model, the formula engine, and the geometry and display list a
viewer draws are there; glue, connector routing and layout follow.

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
- **Geometry:** every row type of a Geometry section (lines, circular and elliptical arcs,
  ellipses, NURBS, polylines, splines, infinite lines, the relative rows) as paths of lines and
  cubic curves, and the transforms of shapes and groups (pin, local pin, angle, flips).
- **A display list per page** (`render::build`): what a viewer draws, in order, from the values
  the file cached -- fills, lines with patterns and line ends, texts laid out in their blocks,
  pictures, background pages, layers, the page's theme -- with a grid to find items by place.
  The drawing itself is someone else's ([`jflib-diagram-view`](https://crates.io/crates/jflib-diagram-view)
  draws it on render2d; exports read the same list).
- **Formulas** (`formula`): the universal ShapeSheet syntax, its functions (maths, logic, text,
  colours, points, THEMEVAL over the document's themes, GUARD, SETATREF, SETF ...) and a
  recalculation that follows the cells a formula really read. Inherited formulas run in the
  inheriting shape; results go into the same transaction as the edit that caused them. Opening
  a file computes nothing, and a formula the engine cannot compute keeps its stored value.
- **Designs** (`design`): a theme as typed data -- the DrawingML theme with Visio's extensions
  (variants, connector scheme, line ends, font styles) -- written as a theme part, JFLib's own
  designs (NetLab, Contour, Contrast, four variants each), and applying a design or variant to a
  page or the document in one undoable step; themed cells follow by recalculation.

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
ShapeSheet cells, with inheritance, units and an edit API with undo and redo; the formula
engine that recalculates them; the geometry of shapes and a page's display list for viewers.
Routing is still to come.
Everything is in the [changelog](https://docs.rs/crate/jflib-diagram-core/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#diagram_core): every
  public item of `diagram_core`, also on [docs.rs](https://docs.rs/jflib-diagram-core)
- [The plan](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md) (in German):
  the stages, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
