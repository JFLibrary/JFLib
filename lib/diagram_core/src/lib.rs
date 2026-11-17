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
//   Document            pages (foreground, background), masters, styles, the document sheet,
//                       themes and parts kept as bytes; a stencil is a document without pages
//   Page, Master, Style  their attributes, sheets, shape trees and Connect entries
//   ShapeTree, ShapeData, Shape   shapes, groups, foreign objects, guides and their subshapes
//   Sheet, Section, Row, Cell     the ShapeSheet store; Value, Formula, Unit, Origin
//   doc.resolve(sheet, addr)      a cell with inheritance: shape -> master shape -> styles
//   doc.rows / shape_data / user_cells / hyperlinks / layers   rows as they really are
//   doc.transaction(label)        edits with undo and redo (Change; ui::history wraps it)
//   Unit, convert, Implicit       unit codes and conversion to the internal units (inches,
//                                 radians, days)
//   Strings, Sym                  the document's string table (compact cells)
//   Extras                        what the file had and the model does not know (DF-03)
//   formula::Engine               the formula engine: parse, evaluate, recalculate (JD6)
//
//   geometry::*                   (JD4) Geometry rows as paths of lines and cubic curves,
//                                 transforms (pin, local pin, angle, flips, groups)
//   render::build(..) -> Drawing  (JD4) a page as a display list: fills, lines, line ends,
//                                 texts laid out, pictures, in painter's order with a grid
//
// What is to come here, by stage of the plan:
//   JD7   glue, the dynamic connector with right-angle routing around obstacles, layout
//
// Rules that hold from the start: no std (only alloc), no unsafe code, nothing recalculated on
// opening a file (connectors, layout and containers move only when something moves), and a
// formula with an unknown function keeps its cached value instead of failing.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod cell;
mod document;
mod edit;
pub mod formula;
pub mod geometry;
mod inherit;
mod names;
mod page;
pub mod render;
mod shape;
mod sheet;
mod strings;
mod units;
mod views;

pub use cell::{Cell, CellError, Formula, Origin, Value};
pub use document::{
    ColorEntry, DocError, DocKind, DocSettings, DocSheet, Document, FaceName, MasterRef, Owner, PageRef, Part,
    Relationship, Shape, ShapeRef, SheetRef, StyleRef, Theme,
};
pub use edit::{Change, EditError, Transaction};
pub use inherit::{guarded, EffRow, Resolved};
pub use names::{named_rows, Category};
pub use page::{attr, Connect, Master, MasterHeader, Page, PageHeader, Style, StyleHeader};
pub use shape::{
    ForeignData, ForeignPayload, ShapeData, ShapeHeader, ShapeKind, ShapeTree, StyleRefs, Text, TextItem, TreeError,
    TreeIter, NO_ID,
};
pub use sheet::{CellAddr, Extra, ExtraItem, Extras, Local, Place, Row, RowKey, SecKey, Section, Sheet, Trigger};
pub use strings::{Strings, Sym};
pub use units::{convert, Dimension, Implicit, Unit};
pub use views::{parse_members, RowView};

#[cfg(test)]
mod tests;
