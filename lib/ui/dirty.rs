// dirty -- dirty regions: only redraw and present what changed        [src/ui/ui_dirty.s]
//
//   dirty_compute(w, h) -> n         number of dirty rectangles (Ui::drects: x, y, w, h,
//                                    clipped to w x h), -1 = everything.  Hashes the
//                                    recorded commands of every layer into 32 x 32 px cells
//                                    and compares with the previous frame.
//   dirty_force_full()               the next dirty_compute reports "everything"
//   dirty_free()                     forget the live window's cells (a closed window)
//   canvas_damage(id, x, y, w, h)    (U1) explicit damage: the part x, y, w, h (px,
//                                    relative to the canvas) of canvas `id` is redrawn with
//                                    the next render even when its command did not change
//   dmg_canvas_seen(id)              canvas() remembers its rectangle and window
//
// A command's hash covers all of its content (colours, coordinates, text, a custom draw
// function's identity and version) and the clip it is drawn in, so any visible change
// marks its cells.

use alloc::vec;
use alloc::vec::Vec;

use super::cmd::{cmd_bbox, cmd_hash, Cmd};
use super::Ui;

const CELL_SHIFT: u32 = 5; // 32 px cells
const CELL: i32 = 32;
pub(crate) const DRECTS_MAX: usize = 128;
const HASH_K: u64 = 0x9E37_79B9_7F4A_7C15;
const DCLIP_MAX: usize = 32;
const DMG_MAX: usize = 32; // pending damage rectangles (all windows)
pub(crate) const CVS_MAX: usize = 16; // canvases whose rectangle is known

/// a pending damage rectangle: window, x0, y0, x1, y1 (px)
#[derive(Clone, Copy, Default)]
pub(crate) struct Dmg {
    win: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// a canvas whose rectangle is known: id (0 = free), window, frame, rect
#[derive(Clone, Copy, Default)]
pub(crate) struct Cvs {
    id: u32,
    win: u32,
    frame: u32,
    rect: [i32; 4],
}

/// the per-window part (ui_dirty_wb0 .. ui_dirty_wb1)
#[derive(Default)]
pub(crate) struct W {
    pub(crate) cells: Option<Vec<u64>>,
    pub(crate) cells_prev: Option<Vec<u64>>,
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    pub(crate) full: bool,
}

// the clip tracked while hashing (C_CLIP / C_UNCLIP of a layer)
struct DClip {
    c: [i32; 4],
    stack: Vec<[i32; 4]>,
}

impl DClip {
    // dclip_reset: no clip (start of a layer's command list)
    fn new() -> DClip {
        DClip { c: [-0x4000_0000, -0x4000_0000, 0x4000_0000, 0x4000_0000], stack: Vec::new() }
    }

    // dclip_push: save the current clip (at most DCLIP_MAX levels), intersect it
    fn push(&mut self, r: [i32; 4]) {
        if self.stack.len() < DCLIP_MAX {
            self.stack.push(self.c);
        }
        let c = &mut self.c;
        c[0] = c[0].max(r[0]);
        c[1] = c[1].max(r[1]);
        c[2] = c[2].min(r[0].wrapping_add(r[2]));
        c[3] = c[3].min(r[1].wrapping_add(r[3]));
    }

    fn pop(&mut self) {
        if let Some(c) = self.stack.pop() {
            self.c = c;
        }
    }

    // dclip_cut: bb (x0, y0, x1, y1) intersected with the clip -> still has area
    fn cut(&self, bb: &mut [i32; 4]) -> bool {
        bb[0] = bb[0].max(self.c[0]);
        bb[1] = bb[1].max(self.c[1]);
        bb[2] = bb[2].min(self.c[2]);
        bb[3] = bb[3].min(self.c[3]);
        bb[2] > bb[0] && bb[3] > bb[1]
    }
}

impl Ui {
    /// dirty_force_full: the next frame redraws and presents everything
    pub fn dirty_force_full(&mut self) {
        self.wp.dirty.full = true;
    }

    /// dirty_free: release the live window's cell arrays (a closed window)
    #[allow(dead_code)] // ui-win
    pub(crate) fn dirty_free(&mut self) {
        let d = &mut self.wp.dirty;
        d.cells = None;
        d.cells_prev = None;
        d.cols = 0;
        d.rows = 0;
        d.full = true;
    }

    // cells_ensure(w, h): (re)allocate both cell arrays for this size
    fn cells_ensure(&mut self, w: i32, h: i32) {
        let cols = (w as u32).wrapping_add(CELL as u32 - 1) >> CELL_SHIFT;
        let rows = (h as u32).wrapping_add(CELL as u32 - 1) >> CELL_SHIFT;
        let d = &mut self.wp.dirty;
        if cols == d.cols && rows == d.rows {
            return;
        }
        d.cols = cols;
        d.rows = rows;
        let n = cols as usize * rows as usize;
        d.cells = Some(vec![0; n]);
        d.cells_prev = Some(vec![0; n]);
        d.full = true;
    }

    /// dmg_canvas_seen(id): remember the rectangle (last_rect) and window of canvas id (a
    /// new id replaces a free entry or the one seen longest ago)
    pub(crate) fn dmg_canvas_seen(&mut self, id: u32) {
        let mut at = None;
        let mut old = 0;
        let mut old_frame = u32::MAX;
        for (i, e) in self.cvs.iter().enumerate() {
            if e.id == id {
                at = Some(i);
                break;
            }
            let f = if e.id == 0 { 0 } else { e.frame }; // a free entry counts as frame 0
            if f < old_frame {
                old_frame = f;
                old = i;
            }
        }
        let i = at.unwrap_or(old);
        self.cvs[i] = Cvs { id, win: self.uiw_cur, frame: self.frame, rect: self.wp.ctx.last_rect };
    }

    /// ui_canvas_damage(id, x, y, w, h): redraw part of a canvas with the next render (px
    /// relative to the canvas, cut to it)
    pub fn canvas_damage(&mut self, id: u32, x: i32, y: i32, w: i32, h: i32) {
        if id == 0 {
            return;
        }
        // never emitted: its first render draws it all
        let Some(e) = self.cvs.iter().find(|e| e.id == id).copied() else { return };
        let r = e.rect;
        let x0 = x.wrapping_add(r[0]).max(r[0]);
        let x1 = w.wrapping_add(x.wrapping_add(r[0])).min(r[0].wrapping_add(r[2]));
        let y0 = y.wrapping_add(r[1]).max(r[1]);
        let y1 = h.wrapping_add(y.wrapping_add(r[1])).min(r[1].wrapping_add(r[3]));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if self.dmg.len() < DMG_MAX {
            self.dmg.push(Dmg { win: e.win, x0, y0, x1, y1 });
        } else if let Some(d) = self.dmg.iter_mut().rev().find(|d| d.win == e.win) {
            // full: grow the last entry of the same window to the union
            d.x0 = d.x0.min(x0);
            d.y0 = d.y0.min(y0);
            d.x1 = d.x1.max(x1);
            d.y1 = d.y1.max(y1);
        }
        self.invalidate();
    }

    // dmg_take(n, w, h) -> n + the damage rectangles of the live window appended to drects
    // (cut to the w x h window), -1 when they do not fit; the window's entries are removed
    fn dmg_take(&mut self, n: i32, w: i32, h: i32) -> i32 {
        let mut n = n;
        let cur = self.uiw_cur;
        let mut keep = Vec::new();
        for d in core::mem::take(&mut self.dmg) {
            if d.win != cur {
                keep.push(d);
                continue;
            }
            if n < 0 {
                continue; // already overflowed
            }
            if n as usize >= DRECTS_MAX {
                n = -1;
                continue;
            }
            let x = d.x0.max(0);
            let rw = d.x1.min(w).wrapping_sub(x);
            let y = d.y0.max(0);
            let rh = d.y1.min(h).wrapping_sub(y);
            if rw > 0 && rh > 0 {
                self.drects.push([x, y, rw, rh]);
                n += 1;
            }
        }
        self.dmg = keep;
        n
    }

    /// dirty_compute(w, h) -> rectangles in drects, or -1 for a full redraw
    pub(crate) fn dirty_compute(&mut self, w: i32, h: i32) -> i32 {
        self.cells_ensure(w, h);
        if self.wp.dirty.cells.is_none() || self.wp.dirty.cells_prev.is_none() {
            return self.dirty_full_take(w, h);
        }
        let cols = self.wp.dirty.cols as i32;
        let rows = self.wp.dirty.rows as i32;
        // ---- hash every command into its cells
        {
            let Ui { wp, .. } = self;
            let cells = wp.dirty.cells.as_mut().unwrap();
            cells.fill(0);
            for layer in wp.cmd.layers.iter() {
                let mut dc = DClip::new();
                for c in layer {
                    match c {
                        Cmd::Clip { x, y, w, h } => {
                            dc.push([*x, *y, *w, *h]);
                            continue;
                        }
                        Cmd::Unclip => {
                            dc.pop();
                            continue;
                        }
                        _ => {}
                    }
                    let Some(mut bb) = cmd_bbox(c) else { continue };
                    // only the part inside the clip can change pixels
                    if !dc.cut(&mut bb) {
                        continue;
                    }
                    // hash of the record and of the clip (a moved clip edge changes pixels
                    // although the command stayed the same)
                    let mut hh = cmd_hash(c);
                    hh ^= (dc.c[0] as u32 as u64) | (dc.c[1] as u32 as u64) << 32;
                    hh = hh.wrapping_mul(HASH_K);
                    hh ^= (dc.c[2] as u32 as u64) | (dc.c[3] as u32 as u64) << 32;
                    hh = hh.wrapping_mul(HASH_K);
                    // cell range (clamped)
                    let c0 = (bb[0] >> CELL_SHIFT).max(0);
                    let c1 = ((bb[2] - 1) >> CELL_SHIFT).min(cols - 1);
                    let r0 = (bb[1] >> CELL_SHIFT).max(0);
                    let r1 = ((bb[3] - 1) >> CELL_SHIFT).min(rows - 1);
                    let mut r = r0;
                    while r <= r1 {
                        let mut k = c0;
                        while k <= c1 {
                            let i = (r * cols + k) as usize;
                            cells[i] = (cells[i].rotate_left(7) ^ hh).wrapping_mul(HASH_K);
                            k += 1;
                        }
                        r += 1;
                    }
                }
            }
        }
        // ---- compare with the previous frame, build rectangles
        self.drects.clear();
        let mut dirty_cells: u32 = 0;
        let mut overflow = false;
        {
            let d = &self.wp.dirty;
            let (cur, prev) = (d.cells.as_ref().unwrap(), d.cells_prev.as_ref().unwrap());
            let is_dirty = |row: i32, col: i32| {
                let i = (row * cols + col) as usize;
                cur[i] != prev[i]
            };
            'rows: for row in 0..rows {
                let mut col = 0;
                while col < cols {
                    if !is_dirty(row, col) {
                        col += 1;
                        continue;
                    }
                    // a run of dirty cells [col, end)
                    let mut end = col + 1;
                    while end < cols && is_dirty(row, end) {
                        end += 1;
                    }
                    dirty_cells += (end - col) as u32;
                    // extend a rectangle from the row above with the same columns
                    let (x, w, y) = (col << CELL_SHIFT, (end - col) << CELL_SHIFT, row << CELL_SHIFT);
                    if let Some(r) = self.drects.iter_mut().find(|r| r[0] == x && r[2] == w && r[1] + r[3] == y) {
                        r[3] += CELL;
                    } else if self.drects.len() < DRECTS_MAX {
                        self.drects.push([x, y, w, CELL]);
                    } else {
                        overflow = true;
                        break 'rows;
                    }
                    col = end;
                }
            }
        }
        if overflow {
            self.dirty_swap();
            return self.dirty_full_take(w, h);
        }
        // (U1) explicit damage (canvas_damage) of this window
        let n0 = self.drects.len() as i32;
        let n = self.dmg_take(n0, w, h);
        if n < 0 {
            self.dirty_swap();
            return self.dirty_full_take(w, h);
        }
        // clip the rectangles to the window
        for r in self.drects.iter_mut() {
            let mw = w.wrapping_sub(r[0]);
            if r[2] > mw {
                r[2] = mw;
            }
            let mh = h.wrapping_sub(r[1]);
            if r[3] > mh {
                r[3] = mh;
            }
        }
        // more than 60 % of the cells dirty: one full redraw is cheaper
        let total = (cols as u32).wrapping_mul(rows as u32).wrapping_mul(3);
        if dirty_cells.wrapping_mul(5) > total || self.wp.dirty.full {
            self.dirty_swap();
            return self.dirty_full_take(w, h);
        }
        self.dirty_swap();
        n
    }

    // .dc_full: everything; the pending damage of this window is covered
    fn dirty_full_take(&mut self, w: i32, h: i32) -> i32 {
        self.wp.dirty.full = false;
        self.drects.clear();
        self.dmg_take(0, w, h);
        self.drects.clear();
        -1
    }

    // .dc_swap: this frame's hashes become the previous ones
    fn dirty_swap(&mut self) {
        let d = &mut self.wp.dirty;
        core::mem::swap(&mut d.cells, &mut d.cells_prev);
    }
}
