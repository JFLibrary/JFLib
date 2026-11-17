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
- Geometry and the display list of the viewer (stage JD4 of apps/JFDiagrammer/PLAN.md): `geometry` turns Geometry sections into paths of lines and cubic curves (MoveTo, LineTo, ArcTo, EllipticalArcTo, Ellipse, NURBSTo with Visio's NURBS(...) values, PolylineTo with POLYLINE(...), SplineStart and SplineKnot, InfiniteLine and the relative rows; NoFill, NoLine, NoShow and NoSnap per section; Rounding), `Xform` and the shape transforms (Pin, LocPin, Angle, FlipX and FlipY, groups: `local_xform`, `page_xform`); `render::build` makes a page's display list from the cached values: background pages first, groups by DisplayMode, fills of closed sub-paths even-odd, line patterns and caps, line ends 1 to 45 of its own drawing with their sizes, solid, translucent, hatched and gradient fills (FillPattern 25 to 40, the FillGradient section), simple shadows, texts laid out in their blocks (Character and Paragraph rows, fields with their cached text, wrapping, alignment, TxtAngle, vertical text, the text of mirrored shapes kept readable) through a `Measure` of the drawing side, pictures of Foreign shapes, layers (hidden ones, layer colours), the page's theme for values cached as Themed (`ThemeInfo`, picked by ThemeIndex and ColorSchemeIndex), the drawing scale; `Drawing::query` finds items through a grid
- The formula engine (stage JD6 of apps/JFDiagrammer/PLAN.md), `diagram_core::formula`: `parse` reads the universal ShapeSheet syntax (numbers with units, strings, operators in Visio's precedence, references to cells, sections, rows, other shapes, ThePage, TheDoc, pages, masters and styles, functions) with the byte position of an error; `Engine::calculate` computes every formula of a document, inherited ones in the inheriting sheet (`Sheet.N` of a master's formula names the instance's subshape), and `Results::write` stores what differs (an inherited formula gets an F="Inh" cell); `recalc` and `recalc_cells` recompute after an edit what depends on it, following the cells formulas really read, into the same transaction; `refresh` after undo and redo; `user_set` (GUARD, SETATREF redirection), `run_action` (SETF; OPENTEXTWIN, DOCMD, RUNADDON, GOTOPAGE ... as `Effect` for the app, macros never run), `evaluate`, `evaluate_text`, `guarded`, `theme_guarded`, `locks`. Functions: maths and logic, text, colours on Visio's HLS scale with TINT, SHADE, MSOTINT and the rest, points and coordinate systems (PAR, LOC, LOCTOLOC, ANGLETOPAR ...), NURBS and POLYLINE, BOUND, THEMEVAL, THEMEPROP and ISTHEMED over the document's themes (colour, font and format schemes chosen by their scheme numbers, quick style slices and variants, DrawingML colour modifiers), shape and page information, the ISERR family. Nothing is computed when a file is opened; a formula the engine cannot compute (an unknown function, a glue helper, a font number of the machine) keeps its stored value and is reported (`Keep`). Text sizes take the app's fonts through `TextMetrics` (else an estimate)

### Changed

- The theme cells (ThemeIndex, ColorSchemeIndex, EffectSchemeIndex, ConnectorSchemeIndex, FontSchemeIndex, the variation and embellishment indices) and the quick style cells inherit from the styles: the line and font ones with line and text, the others with fill (`Category::of`); shapes took them only from master shapes and so never got the document's theme

