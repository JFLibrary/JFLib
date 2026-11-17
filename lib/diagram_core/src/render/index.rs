// index -- a uniform grid over a drawing's items: which items touch a rectangle, without
// looking at all of them (culling for the drawing side, hit tests for JD7)
//
//   Grid::build(&items, bounds)     up to 128 x 128 cells over the bounds; an item lands in
//                                   every cell its box touches, an item over more than 1024
//                                   cells in a list that every query looks at
//   grid.query(rect, &mut out)      the items touching rect, ascending (painter's order)
//
// Building is linear in the items times their cells (bounded per item); a query costs the
// cells of its rectangle plus what lies in them.  Nothing is quadratic in the number of items.

use alloc::vec::Vec;

use super::Item;
use crate::geometry::{ceil, floor};

const CELLS: usize = 128;
const BIG: usize = 1024;

#[derive(Clone, Debug, Default)]
pub struct Grid {
    origin: [f64; 2],
    cell: [f64; 2],
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
    big: Vec<u32>,
    boxes: Vec<[f64; 4]>,
}

impl Grid {
    pub fn build(items: &[Item], bounds: [f64; 4]) -> Grid {
        let w = (bounds[2] - bounds[0]).max(1e-6);
        let h = (bounds[3] - bounds[1]).max(1e-6);
        // square-ish cells, at most CELLS along the longer side
        let side = w.max(h) / CELLS as f64;
        let nx = (ceil(w / side) as usize).clamp(1, CELLS);
        let ny = (ceil(h / side) as usize).clamp(1, CELLS);
        let mut g = Grid { origin: [bounds[0], bounds[1]], cell: [w / nx as f64, h / ny as f64], nx, ny, cells: alloc::vec![Vec::new(); nx * ny], big: Vec::new(), boxes: Vec::with_capacity(items.len()) };
        for (i, it) in items.iter().enumerate() {
            let b = it.bbox;
            g.boxes.push(b);
            let Some((x0, y0, x1, y1)) = g.range(b) else { continue };
            if (x1 - x0 + 1) * (y1 - y0 + 1) > BIG {
                g.big.push(i as u32);
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g.cells[y * nx + x].push(i as u32);
                }
            }
        }
        g
    }

    // the cells a box touches (clamped to the grid; None: not a finite box)
    fn range(&self, b: [f64; 4]) -> Option<(usize, usize, usize, usize)> {
        if !b.iter().all(|v| v.is_finite()) || b[2] < b[0] || b[3] < b[1] {
            return None;
        }
        let cx = |x: f64| (floor((x - self.origin[0]) / self.cell[0]).max(0.0) as usize).min(self.nx - 1);
        let cy = |y: f64| (floor((y - self.origin[1]) / self.cell[1]).max(0.0) as usize).min(self.ny - 1);
        Some((cx(b[0]), cy(b[1]), cx(b[2]), cy(b[3])))
    }

    pub fn query(&self, rect: [f64; 4], out: &mut Vec<u32>) {
        out.clear();
        if self.nx == 0 {
            return;
        }
        let touches = |b: &[f64; 4]| b[0] <= rect[2] && b[2] >= rect[0] && b[1] <= rect[3] && b[3] >= rect[1];
        if let Some((x0, y0, x1, y1)) = self.range(rect) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    out.extend(self.cells[y * self.nx + x].iter().copied().filter(|&i| touches(&self.boxes[i as usize])));
                }
            }
        }
        out.extend(self.big.iter().copied().filter(|&i| touches(&self.boxes[i as usize])));
        out.sort_unstable();
        out.dedup();
    }
}
