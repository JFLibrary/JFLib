// tools::array -- Array (AS-11, RA-09): copies of shapes in rows and columns
//
//   Grid                    rows, columns, the spacing, how it is measured, the directions
//   offsets(size, &grid)    the offset of every cell but the first (row by row): where the copy
//                           of that cell goes, from the shapes themselves
//   make(tx, eng, owner, shapes, &grid) -> Vec<ShapeRef>   the copies (new top-level shapes in
//                           the order of the cells)
//
// The shapes copied stand in the first cell (top left with the default directions); the cell is
// the box around all of them.  Spacing between the shapes: from the box's edge to the next box's
// edge; from middle to middle: the step itself (smaller than the box: the copies overlap, as the
// user asked).  Copies are made the way a paste makes them (ops::paste): masters stay masters,
// connectors between copied shapes come along, glue to shapes left behind is let go.

use alloc::vec::Vec;

use crate::document::{Owner, ShapeRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::ops;

/// the most cells an array makes (rows x columns)
pub const MAX_CELLS: u32 = 10_000;

/// an array of copies
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Grid {
    /// rows (1 or more)
    pub rows: u32,
    /// columns (1 or more)
    pub cols: u32,
    /// x: between columns, y: between rows (drawing units, 0 or more)
    pub gap: [f64; 2],
    /// the gap is the step from middle to middle (else from edge to edge)
    pub centres: bool,
    /// the columns go to the left (else to the right)
    pub left: bool,
    /// the rows go up (else down)
    pub up: bool,
}

impl Default for Grid {
    fn default() -> Grid {
        Grid { rows: 1, cols: 1, gap: [0.25, 0.25], centres: false, left: false, up: false }
    }
}

/// (row, column, offset) of every cell but the first, row by row, for shapes whose box is
/// `size` wide and high
pub fn offsets(size: [f64; 2], g: &Grid) -> Vec<(u32, u32, [f64; 2])> {
    let step = |k: usize| if g.centres { g.gap[k].max(0.0) } else { size[k] + g.gap[k].max(0.0) };
    let (sx, sy) = (step(0) * if g.left { -1.0 } else { 1.0 }, step(1) * if g.up { 1.0 } else { -1.0 });
    let mut out = Vec::new();
    for r in 0..g.rows.max(1) {
        for c in 0..g.cols.max(1) {
            if r == 0 && c == 0 {
                continue;
            }
            out.push((r, c, [c as f64 * sx, r as f64 * sy]));
        }
    }
    out
}

/// the copies of `shapes` (top-level shapes of `owner`) for every cell but the first -> the new
/// top-level shapes, cell by cell.  Invalid: no rows or columns, more than MAX_CELLS
pub fn make(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], g: &Grid) -> Result<Vec<ShapeRef>, EditError> {
    if g.rows == 0 || g.cols == 0 || g.rows.saturating_mul(g.cols) > MAX_CELLS || !g.gap.iter().all(|v| v.is_finite()) {
        return Err(EditError::Invalid);
    }
    let b = ops::bounds(tx.doc(), shapes).ok_or(EditError::NoShape)?;
    let clip = ops::copy(tx.doc(), shapes);
    let mut made = Vec::new();
    for (_, _, d) in offsets([b[2] - b[0], b[3] - b[1]], g) {
        made.extend(ops::paste(tx, eng, owner, &clip, d)?);
    }
    Ok(made)
}
