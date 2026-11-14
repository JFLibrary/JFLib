// diagram_core -- the diagram model: ShapeSheet cells, formulas, geometry, routing and layout
//
// The model behind JFDiagrammer (apps/JFDiagrammer, plan in its PLAN.md), usable by any app that
// draws or reads diagrams of the Visio kind.  It holds no file format (that is diagram_io) and no
// widget (the canvas is ui::diagram); it is plain data and the rules that compute it.
//
// The ShapeSheet is the model (decision V1): every cell is kept as the file has it -- formula,
// cached value, unit and where it comes from (local, inherited from the master or from a style,
// deleted) -- and documents, pages and shapes are views on these cells.  So a round trip through
// a file loses nothing, and the behaviour Visio files carry works without translation.
//
// What is to come here, by stage of the plan (none of it exists yet):
//   JD2   document, foreground and background pages, layers, masters, shapes and groups, styles,
//         theme data; sections, rows and cells; the inheritance shape -> master shape -> style ->
//         document; NameU and Name; units (inches inside, as in the file); hyperlinks, shape data
//         and user cells; an edit API with transactions for undo and redo
//   JD4   geometry rows and transforms (pin, local pin, angle, flips, groups) for drawing
//   JD6   the formula engine: parser of the universal syntax, evaluation over a dependency graph
//         with incremental recalculation, units, inheritance, GUARD and locks
//   JD7  glue, the dynamic connector with right-angle routing around obstacles, layout
//
// Rules that hold from the start: no std (only alloc), no unsafe code, nothing recalculated on
// opening a file (connectors, layout and containers move only when something moves), and a
// formula with an unknown function keeps its cached value instead of failing.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
