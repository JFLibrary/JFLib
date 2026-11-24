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
//   instance::*                   (JD9) masters from stencils into documents and onto pages:
//                                 import_master, place, the texts in five languages
//   geometry::*                   (JD4) Geometry rows as paths of lines and cubic curves,
//                                 transforms (pin, local pin, angle, flips, groups)
//   render::build(..) -> Drawing  (JD4) a page as a display list: fills, lines, line ends,
//                                 texts laid out, pictures, in painter's order with a grid;
//                                 render::rebuild (JD7) builds some top-level shapes again
//   ops::*                        (JD7) gestures as cell writes: move, resize, rotate, flip,
//                                 pin, control points, vertices, groups, z-order, new shapes,
//                                 text, the clipboard inside an app
//   glue::* / route::*            (JD7) connection points, static and dynamic glue (Connect
//                                 entries), the dynamic connector, right-angle routing around
//                                 obstacles
//   hit::*                        (JD7) the shape under a point of a display list, shapes in a
//                                 rectangle
//
//   structure::*                  (JD10) containers, lists, callouts: membership in the
//                                 Relationships cell, automatic size, list order and layout
//   dock::*                       (JD13) 2-D shapes that snap with an outward connection point
//                                 onto an inward one (equipment into racks)
//   pagesetup::*                  (JD8) a page's size against its drawing and its paper:
//                                 orientation, sizes, fit to the drawing, automatic size in
//                                 tiles of the printed sheet; Transaction::add_page_like and
//                                 duplicate_page insert pages with settings or content
//   data::*                       (JD18) external data: connections, recordsets with their
//                                 cached rows, rows bound to shapes (Prop rows, DataLinked),
//                                 refresh with conflicts, automatic linking (DA-04, DA-05)
//   shapeops::*                   (JD21) shape operations: Vereinigen, Kombinieren,
//                                 Fragmentieren, Schneiden, Subtrahieren (SP-08) on geom's plane
//                                 Booleans, arcs rebuilt as arcs; fit_frame (the box follows the
//                                 geometry after vertex edits, SP-05), update_alignment_box (SP-16)
//   field::*                      (JD8) text fields: a Field row's value in its format picture,
//                                 new fields, the text with its fields, the document's
//                                 properties (Document::fields: the app's locale and clock)
//   layout::*                     (JD21) the page laid out anew in Visio's styles (flowchart,
//                                 hierarchy, compact tree, radial, circular; CN-13), connected
//                                 shapes aligned and spaced (CN-14), shapes off the page
//                                 breaks (PR-03), the diagram turned or flipped (RS-25)
//   autoconn::*                   (JD21) a shape dropped on a connector splits it and the shapes
//                                 after it make room (CN-08), a deleted shape's connectors go
//                                 and the flow through it heals (CN-09), drops push shapes
//                                 away (PlowCode)
//   pointglue::*                  (JD21) control points that glue to connection points and
//                                 follow them (SP-04), glue to vertices, handles, control points
//                                 and guides (CN-02), the connection point tool (CN-01)
//
// Rules that hold from the start: no std (only alloc), no unsafe code, nothing recalculated on
// opening a file (connectors, layout and containers move only when something moves), and a
// formula with an unknown function keeps its cached value instead of failing.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod autoconn;
mod cell;
pub mod data;
pub mod design;
pub mod dock;
mod document;
mod edit;
pub mod field;
pub mod formula;
pub mod geometry;
pub mod glue;
pub mod hit;
mod inherit;
pub mod instance;
pub mod layout;
mod names;
pub mod ops;
mod page;
pub mod pagesetup;
pub mod pointglue;
pub mod render;
pub mod route;
pub mod shapeops;
mod shape;
pub mod structure;
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
