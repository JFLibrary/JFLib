# Changelog of jflib-diagram-core

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The crate, with the document model of stage JD2 of the plan
  [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md):
  - `Document` with foreground and background pages, masters (a stencil is a document without
    pages), styles, the document sheet, theme parts and other package parts kept as bytes;
    `Page`, `Master` and `Style` with their attributes, sheets and `Connect` entries
  - `ShapeTree` of shapes, groups, foreign objects and guides with their subshapes, file IDs
    found through a map, slots (`ShapeRef`) that stay valid across undo
  - the ShapeSheet store `Sheet`, `Section`, `Row` and `Cell`: formula, cached value, unit and
    error as the file has them (`Value`, `Formula`, `CellError`), indexed and named rows, Del
    marks; `Extras` keeps what the model does not know at the place it stood
  - `Document::resolve` reads a cell through the master shape and the line, fill and text
    styles and says where its formula comes from (`Origin`); `rows` and `section_indices`
    merge inherited, overridden and deleted rows; `shape_data`, `user_cells`, `hyperlinks`,
    `layers` and `layer_members` show them
  - `Unit` knows every unit code of a cell and converts to inches, radians and days, `Implicit`
    resolves the page's drawing, text, angle and duration units
  - `Strings` and `Sym` keep cell names, formulas and texts once per document (an ordered index:
    texts from a file cannot make a lookup slow)
  - edits in a `Transaction` become one `Change` that undoes and redoes by inverse steps, sized
    by what it changes; `ui::history` takes it as one step. Formula and value stay a pair:
    `set_formula` leaves the cell stale for the formula engine, `set_constant` writes a value
    whose formula is the value, `set_result` stores the engine's result, GUARD is respected
  - for loaders (stage JD3, diagram_io): `Relationship` and `Document::rels` (every relationship
    of a package as read), `Document::colors_extras`, `faces_extras` and `styles_extras`,
    `Place::InShapes` and `Place::InConnects` (what stood inside the Shapes and Connects lists),
    `Document::strings_mut` (the table for `Value::parse`, `Formula::parse`, `ShapeKind::parse`)
  - `NO_ID`, the ID 4294967295 Visio gives the deleted subshapes of an instance: a `ShapeTree`
    takes any number of shapes with it, `by_id` does not find them, they do not use up the IDs,
    and `next_id` never hands it out

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
