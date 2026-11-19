// index -- a uniform grid over a drawing's items: which items touch a rectangle, without
// looking at all of them (culling for the drawing side, hit tests for JD7)
//
//   Grid::build(&items, bounds)     up to 128 x 128 cells over the bounds; an item lands in
//                                   every cell its box touches, an item over more than 1024
//                                   cells in a list that every query looks at
//   grid.query(rect, &mut out)      the items touching rect, ascending (painter's order)
//   grid.replace(&changes, &items)  (JD7) ranges of items made anew (render::rebuild): the
//                                   grid keeps its cells, the items after a range that grew or
//                                   shrank are renumbered
//
// Building is linear in the items times their cells (bounded per item); a query costs the
// cells of its rectangle plus what lies in them.  Nothing is quadratic in the number of items.
// A cell's list need not be sorted (a query sorts what it found).  An item outside the bounds
// the grid was built for lies in the cells at the edge, where a query outside looks too.

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
            g.boxes.push(it.bbox);
            g.place(i as u32, it.bbox);
        }
        g
    }

    /// item i with box b into the cells it touches (or the big list)
    fn place(&mut self, i: u32, b: [f64; 4]) {
        let Some((x0, y0, x1, y1)) = self.range(b) else { return };
        if (x1 - x0 + 1) * (y1 - y0 + 1) > BIG {
            self.big.push(i);
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.nx + x].push(i);
            }
        }
    }

    /// item i with box b out of the cells place put it in
    fn unplace(&mut self, i: u32, b: [f64; 4]) {
        let Some((x0, y0, x1, y1)) = self.range(b) else { return };
        let take = |v: &mut Vec<u32>| {
            if let Some(k) = v.iter().position(|&x| x == i) {
                v.swap_remove(k);
            }
        };
        if (x1 - x0 + 1) * (y1 - y0 + 1) > BIG {
            take(&mut self.big);
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                take(&mut self.cells[y * self.nx + x]);
            }
        }
    }

    /// ranges of items made anew: (first item, old count, new count) each, ascending and apart,
    /// in the old numbering; `items` the list as it is now
    pub fn replace(&mut self, changes: &[(usize, usize, usize)], items: &[Item]) {
        if self.nx == 0 {
            let bounds = items.iter().map(|i| i.bbox).filter(|b| b.iter().all(|v| v.is_finite())).reduce(super::union).unwrap_or([0.0, 0.0, 1.0, 1.0]);
            *self = Grid::build(items, bounds);
            return;
        }
        if changes.iter().all(|c| c.1 == c.2) {
            // the same numbering: each item out of its old cells, into its new ones
            for &(s, n, _) in changes {
                for i in s..(s + n).min(items.len()).min(self.boxes.len()) {
                    let old = self.boxes[i];
                    self.unplace(i as u32, old);
                    self.boxes[i] = items[i].bbox;
                    self.place(i as u32, items[i].bbox);
                }
            }
            return;
        }
        // old number -> new number (u32::MAX: made anew)
        let old_len = self.boxes.len();
        let mut map = alloc::vec![u32::MAX; old_len];
        let (mut i, mut shift) = (0usize, 0i64);
        for &(s, n, m) in changes {
            while i < s.min(old_len) {
                map[i] = (i as i64 + shift) as u32;
                i += 1;
            }
            i = (s + n).max(i);
            shift += m as i64 - n as i64;
        }
        while i < old_len {
            map[i] = (i as i64 + shift) as u32;
            i += 1;
        }
        for v in self.cells.iter_mut().chain(core::iter::once(&mut self.big)) {
            v.retain_mut(|x| match map.get(*x as usize) {
                Some(&m) if m != u32::MAX => {
                    *x = m;
                    true
                }
                _ => false,
            });
        }
        self.boxes = items.iter().map(|it| it.bbox).collect();
        let mut shift = 0i64;
        for &(s, n, m) in changes {
            let ns = (s as i64 + shift) as usize;
            for k in ns..(ns + m).min(items.len()) {
                self.place(k as u32, items[k].bbox);
            }
            shift += m as i64 - n as i64;
        }
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
