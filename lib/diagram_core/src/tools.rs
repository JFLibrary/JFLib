// tools -- (JD21) shape tools in place of Visio's add-ons (AS-11, RA-09): numbering shapes, arrays
// of copies, dimension lines
//
//   number::*      Shapes nummerieren: the order (as selected, in rows, in columns, along the
//                  connectors), the number written into the text before or after it or instead
//                  of it and kept in User.JFD_Number / JFD_NumberPlace, so a second run replaces
//                  it instead of adding one more; a page that goes on numbering shapes as they
//                  are dropped (User.JFD_NumberNext ... of the page sheet)
//   array::*       Array: copies of shapes in rows and columns, spaced between the shapes or from
//                  middle to middle, to the right or left and down or up; copies as a paste makes
//                  them (instances stay instances of their masters)
//   dimension::*   Maßlinien: the two ends of a dimension shape on two points, a shape's extent
//                  or between two shapes (horizontal, vertical, aligned): glued to a connection
//                  point where one is there (static glue, a Connect entry), else tied to a point of
//                  the shape's box by the same PAR(PNT(...)) formula; both follow the shape
//
// Every position is on the page in drawing units, y up.  Everything runs inside the caller's
// transaction (one undo step).  Visio's own add-ons are programs, not cells: nothing of them is
// in a file, so these tools write only ordinary cells (text, User rows, glue formulas) that Visio
// keeps and draws as they are.

pub mod array;
pub mod dimension;
pub mod number;

#[cfg(test)]
mod tests;

use crate::document::{Document, ShapeRef};
use crate::shape::ShapeKind;
use crate::{glue, structure};

/// whether the tools take a shape as one of their own: a 2-D shape that is no guide, container,
/// list or callout (connectors, lines and dimensions are left out)
pub fn plain(doc: &Document, s: ShapeRef) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    !sh.is_deleted() && sh.header().kind != ShapeKind::Guide && !glue::is_one_d(doc, s) && structure::kind(doc, s).is_none()
}

// the box of a shape on the page (x0, y0, x1, y1)
fn bbox(doc: &Document, s: ShapeRef) -> [f64; 4] {
    crate::ops::bounds(doc, &[s]).unwrap_or([0.0; 4])
}
