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
// marks its cells.  The same pass fills the content grids of the scroll views
// (scrollcopy.rs): dirty cells whose pixels a scrolled view still shows elsewhere are copied
// there before the rectangles are drawn.
//
// While a window's frames go through the GPU (gpu_out.rs, W::split) the pass also keeps the hash
// of every cell after each layer (W::through: what changed in which layer) and notes the big
// images of the base layer and which cells they end up on top of (W::top, W::imgs).  A canvas whose
// view comes from tiles (canvas.rs, W::canv) is noted the same way; its hash stands only for where
// it lies: what it shows there is the GPU's.

use alloc::vec;
use alloc::vec::Vec;

use super::cmd::{cmd_bbox, cmd_parts, pos_mix, Cmd};
use super::consts::L_COUNT;
use super::Ui;

const CELL_SHIFT: u32 = 5; // 32 px cells
const CELL: i32 = 32;
/// dirty rectangles of a render; more: everything is redrawn (the same pixels, only slower)
pub(crate) const DRECTS_MAX: usize = 128;
/// a frame with a scroll copy may draw this many rectangles more than without it (each one
/// replays the commands)
const COPY_RECTS_MORE: usize = 16;
const HASH_K: u64 = 0x9E37_79B9_7F4A_7C15;
/// clip levels tracked while hashing (as ctx::UICLIP_MAX); deeper ones clip nothing, which
/// only marks more cells
const DCLIP_MAX: usize = super::ctx::UICLIP_MAX;
/// pending damage rectangles (all windows); more: merged into one of the same window
const DMG_MAX: usize = 32;
// (canvases whose rectangle is known: Ui::cvs grows; a record not drawn in the last two
// passes is reused, so it holds at most the canvases of two passes and none is forgotten
// while it is on the screen)

/// a pending damage rectangle: window, x0, y0, x1, y1 (px)
#[derive(Clone, Copy, Default)]
pub(crate) struct Dmg {
    win: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// images of the base layer from this many visible pixels on are noted for the GPU output
pub(crate) const IMG_MIN_PX: i64 = 256 * 256;
/// W::top: more than an image touches the cell, or something after it
pub(crate) const TOP_BLOCKED: u8 = 255;

/// a big image of the base layer in this frame: its record, the part of it the clips leave
/// (x0, y0, x1, y1)
#[derive(Clone, Copy)]
#[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))] // (read by gpu_out.rs)
pub(crate) struct ImgSeen {
    pub(crate) index: u32,
    pub(crate) vis: [i32; 4],
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
    /// (gpu_out.rs) the GPU output keeps the layers apart: the hashes through each layer, the
    /// big images
    pub(crate) split: bool,
    /// (gpu_out.rs) the last frame had layers or images apart: no scroll copies
    pub(crate) busy: bool,
    /// this frame: everything was asked for, or explicit damage went into a full redraw
    pub(crate) forced: bool,
    /// the rectangles of drects before the explicit damage
    pub(crate) n0: i32,
    /// the hashes through layer l, for the layers below a layer with commands (empty: as through
    /// l + 1), of this frame and the last
    pub(crate) pre: [Vec<u64>; L_COUNT - 1],
    pub(crate) pre_prev: [Vec<u64>; L_COUNT - 1],
    /// per cell: 0 nothing yet, k the k-th image of imgs is on top, TOP_BLOCKED
    pub(crate) top: Vec<u8>,
    pub(crate) imgs: Vec<ImgSeen>,
    /// (gpu_out.rs) the canvases of the base layer apart in this frame: command, rectangle
    pub(crate) canv: Vec<(u32, [i32; 4])>,
}

impl W {
    /// (after dirty_compute, whose swap made cells_prev this frame's) the hashes through layer l
    /// of this frame or of the last one
    #[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))] // (read by gpu_out.rs)
    pub(crate) fn through(&self, now: bool, l: usize) -> &[u64] {
        let (pre, all) = if now { (&self.pre_prev, &self.cells_prev) } else { (&self.pre, &self.cells) };
        match pre[l.min(L_COUNT - 1)..].iter().find(|k| !k.is_empty()) {
            Some(k) => k,
            None => all.as_deref().unwrap_or(&[]),
        }
    }
}

// the clip tracked while hashing (C_CLIP / C_UNCLIP of a layer)
struct DClip {
    c: [i32; 4],
    stack: Vec<[i32; 4]>,
    /// levels beyond DCLIP_MAX (they clip nothing)
    over: u32,
}

impl DClip {
    // dclip_reset: no clip (start of a layer's command list)
    fn new() -> DClip {
        DClip { c: [-0x4000_0000, -0x4000_0000, 0x4000_0000, 0x4000_0000], stack: Vec::new(), over: 0 }
    }

    // dclip_push: save the current clip (at most DCLIP_MAX levels), intersect it
    fn push(&mut self, r: [i32; 4]) {
        if self.stack.len() >= DCLIP_MAX {
            self.over += 1;
            return;
        }
        self.stack.push(self.c);
        let c = &mut self.c;
        c[0] = c[0].max(r[0]);
        c[1] = c[1].max(r[1]);
        c[2] = c[2].min(r[0].wrapping_add(r[2]));
        c[3] = c[3].min(r[1].wrapping_add(r[3]));
    }

    fn pop(&mut self) {
        if self.over != 0 {
            self.over -= 1;
        } else if let Some(c) = self.stack.pop() {
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

// rects_build(out, cols, rows, dirty) -> the number of dirty cells (dirty(row * cols + col)),
// as rectangles (px) in out: a run of dirty cells in a row extends a rectangle of the row above
// with the same columns; None when more than DRECTS_MAX
fn rects_build(out: &mut Vec<[i32; 4]>, cols: i32, rows: i32, dirty: impl Fn(usize) -> bool) -> Option<u32> {
    out.clear();
    let mut cells = 0u32;
    for row in 0..rows {
        let at = |col: i32| dirty((row * cols + col) as usize);
        let mut col = 0;
        while col < cols {
            if !at(col) {
                col += 1;
                continue;
            }
            // a run of dirty cells [col, end)
            let mut end = col + 1;
            while end < cols && at(end) {
                end += 1;
            }
            cells += (end - col) as u32;
            // extend a rectangle from the row above with the same columns
            let (x, w, y) = (col << CELL_SHIFT, (end - col) << CELL_SHIFT, row << CELL_SHIFT);
            if let Some(r) = out.iter_mut().find(|r| r[0] == x && r[2] == w && r[1] + r[3] == y) {
                r[3] += CELL;
            } else if out.len() < DRECTS_MAX {
                out.push([x, y, w, CELL]);
            } else {
                return None;
            }
            col = end;
        }
    }
    Some(cells)
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
        d.pre = Default::default();
        d.pre_prev = Default::default();
        d.full = true;
    }

    /// dmg_canvas_seen(id): remember the rectangle (last_rect) and window of canvas id (a
    /// new id takes the record seen longest ago when that canvas was not drawn in the last
    /// two passes, else a new record)
    pub(crate) fn dmg_canvas_seen(&mut self, id: u32) {
        let e = Cvs { id, win: self.uiw_cur, frame: self.frame, rect: self.wp.ctx.last_rect };
        let (mut old, mut age) = (0, 0u32);
        for (i, c) in self.cvs.iter_mut().enumerate() {
            if c.id == id {
                *c = e;
                return;
            }
            let a = self.frame.wrapping_sub(c.frame);
            if a > age {
                (old, age) = (i, a);
            }
        }
        if age >= 2 {
            self.cvs[old] = e;
        } else {
            self.cvs.push(e);
        }
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
        let full = self.dmg.len() >= DMG_MAX;
        if let Some(d) = self.dmg.iter_mut().rev().find(|d| d.win == e.win).filter(|_| full) {
            // full: grow the last entry of the same window to the union
            d.x0 = d.x0.min(x0);
            d.y0 = d.y0.min(y0);
            d.x1 = d.x1.max(x1);
            d.y1 = d.y1.max(y1);
        } else {
            self.dmg.push(Dmg { win: e.win, x0, y0, x1, y1 }); // (full: the window's first)
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

    /// dirty_compute(w, h) -> rectangles in drects, or -1 for a full redraw.  Scroll views whose
    /// content only moved copy their still valid cells first (scrollcopy.rs).
    pub(crate) fn dirty_compute(&mut self, w: i32, h: i32) -> i32 {
        self.cells_ensure(w, h);
        {
            let d = &mut self.wp.dirty;
            d.forced = d.full;
            d.top.clear();
            d.imgs.clear();
            d.canv.clear();
            for k in d.pre.iter_mut() {
                k.clear();
            }
        }
        if self.gp_canvas_wanted() {
            self.gp_canvas_begin(); // which canvases come from tiles in this frame (gpu_out.rs)
        }
        self.gp_views_begin(); // (G5e1) the views: their frames now, whether the output draws them (gpu_out.rs)
        let mut sc = self.sc_pass(w, h);
        if self.wp.dirty.cells.is_none() || self.wp.dirty.cells_prev.is_none() {
            self.sc_plan(sc, 0, 0, false);
            return self.dirty_full_take(w, h);
        }
        let cols = self.wp.dirty.cols as i32;
        let rows = self.wp.dirty.rows as i32;
        // ---- hash every command into its cells (and into the scroll views' content grids)
        {
            let Ui { wp, .. } = self;
            let d = &mut wp.dirty;
            let split = cfg!(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")) && d.split; // (only there: nothing of it in other targets)
            let cells = d.cells.as_mut().unwrap();
            cells.fill(0);
            let grids = sc.on();
            for (l, layer) in wp.cmd.layers.iter().enumerate() {
                if split && l > 0 && !layer.is_empty() {
                    d.pre[l - 1].extend_from_slice(cells);
                }
                let mut dc = DClip::new();
                for (i, c) in layer.iter().enumerate() {
                    if grids {
                        sc.reach(l, i, dc.c, dc.over == 0);
                    }
                    match c {
                        Cmd::Clip { x, y, w, h } => {
                            dc.push([*x, *y, *w, *h]);
                            if grids {
                                sc.clip(l, i, [*x, *y, x.wrapping_add(*w), y.wrapping_add(*h)]);
                            }
                            continue;
                        }
                        Cmd::Unclip => {
                            dc.pop();
                            if grids {
                                sc.unclip(l, i);
                            }
                            continue;
                        }
                        _ => {}
                    }
                    let Some(bb0) = cmd_bbox(c) else { continue };
                    // only the part inside the clip can change pixels
                    let mut bb = bb0;
                    let vis = dc.cut(&mut bb);
                    if !vis && !grids {
                        continue;
                    }
                    // hash of the record and of the clip (a moved clip edge changes pixels
                    // although the command stayed the same)
                    let parts = cmd_parts(c);
                    let mut hh = pos_mix(parts.0, parts.1 as i64, parts.2 as i64);
                    hh ^= (dc.c[0] as u32 as u64) | (dc.c[1] as u32 as u64) << 32;
                    hh = hh.wrapping_mul(HASH_K);
                    hh ^= (dc.c[2] as u32 as u64) | (dc.c[3] as u32 as u64) << 32;
                    hh = hh.wrapping_mul(HASH_K);
                    // a canvas apart: the part of its rectangle the clips leave stands for it
                    let mut apart = None;
                    if split && l == 0 && !d.canv.is_empty() && dc.over == 0 && d.imgs.len() < TOP_BLOCKED as usize - 1 && matches!(c, Cmd::Custom { .. }) {
                        if let Some(&(_, r)) = d.canv.iter().find(|x| x.0 == i as u32) {
                            let v = [bb[0].max(r[0]), bb[1].max(r[1]), bb[2].min(r[0] + r[2]), bb[3].min(r[1] + r[3])];
                            hh = pos_mix(0x4341_4E56, v[0] as i64, v[1] as i64) ^ ((v[2] as u32 as u64) | (v[3] as u32 as u64) << 32).wrapping_mul(HASH_K);
                            apart = Some(v);
                        }
                    }
                    if grids {
                        sc.cmd(l, i, c, parts, bb0, dc.c, dc.over == 0, hh);
                    }
                    if !vis {
                        continue;
                    }
                    // cell range (clamped)
                    let c0 = (bb[0] >> CELL_SHIFT).max(0);
                    let c1 = ((bb[2] - 1) >> CELL_SHIFT).min(cols - 1);
                    let r0 = (bb[1] >> CELL_SHIFT).max(0);
                    let r1 = ((bb[3] - 1) >> CELL_SHIFT).min(rows - 1);
                    // (the GPU output) the big images of the base layer and what lies over them
                    let mut tag = 0;
                    let seen = match apart {
                        Some(v) => (v[0] < v[2] && v[1] < v[3]).then_some(v),
                        None => (matches!(c, Cmd::Image { .. }) && (bb[2] - bb[0]) as i64 * (bb[3] - bb[1]) as i64 >= IMG_MIN_PX).then_some(bb),
                    };
                    if let Some(vis) = seen.filter(|_| split && l == 0 && dc.over == 0 && d.imgs.len() < TOP_BLOCKED as usize - 1) {
                        if d.top.is_empty() {
                            d.top.resize(cols as usize * rows as usize, 0);
                        }
                        d.imgs.push(ImgSeen { index: i as u32, vis });
                        tag = d.imgs.len() as u8;
                    }
                    let track = l == 0 && !d.top.is_empty();
                    let mut r = r0;
                    while r <= r1 {
                        let mut k = c0;
                        while k <= c1 {
                            let i = (r * cols + k) as usize;
                            cells[i] = (cells[i].rotate_left(7) ^ hh).wrapping_mul(HASH_K);
                            if track {
                                let t = &mut d.top[i];
                                *t = if *t != 0 { TOP_BLOCKED } else { tag };
                            }
                            k += 1;
                        }
                        r += 1;
                    }
                }
            }
        }
        // ---- compare with the previous frame, build rectangles
        let pending = self.dmg.iter().any(|d| d.win == self.uiw_cur);
        // the GPU output's images of this frame; scroll copies move pixels of the framebuffer, which
        // keeps no separate layers and may lack an image's pixels
        let split = cfg!(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")) && self.wp.dirty.split;
        let apart = split && {
            let views = self.wp.canv.views.iter().any(|v| v.apart);
            let quads = (!self.wp.dirty.imgs.is_empty() || self.wp.dirty.busy || views) && {
                let images = self.gp_plan_images();
                let canvases = self.gp_plan_canvases();
                self.gp_plan_views() | canvases | images
            };
            quads || self.wp.dirty.busy || self.wp.cmd.layers[1..].iter().any(|l| !l.is_empty())
        };
        let allowed = !self.wp.dirty.full && !pending && !apart;
        let mut plan = self.sc_plan(sc, cols, rows, allowed);
        let want = plan.as_ref().is_some_and(|p| self.sc_want(p));
        // (checked frames copy whenever they can, and so does a finger's scroll: touch.rs)
        let force = self.verify_dirty || self.m.touch.on;
        let (built, copy) = {
            let Ui { wp, drects, .. } = &mut *self;
            let d = &wp.dirty;
            let (cur, prev) = (d.cells.as_ref().unwrap(), d.cells_prev.as_ref().unwrap());
            let changed = |i: usize| cur[i] != prev[i];
            let mut built = rects_build(drects, cols, rows, changed);
            // a scroll copy (scrollcopy.rs) provides the cells it covers: when the view's costs
            // favour it (sc_want), it takes over at least a quarter of what would be drawn
            // without it and does not split the rest into many more rectangles (each one
            // replays the commands); frames checked by --verify-dirty take any copy
            let mut copy = false;
            if let Some(p) = plan.as_mut().filter(|_| want) {
                let copied = p.settle(cols as usize, changed);
                let total = cols as u32 * rows as u32;
                let draw = built.filter(|&n| n * 5 <= total * 3).unwrap_or(total);
                let mut alt = Vec::new();
                let cv = &p.covered;
                if let Some(n) = rects_build(&mut alt, cols, rows, |i| changed(i) && cv[i] == 0) {
                    if copied > 0 && (force || (copied * 4 >= draw as usize && alt.len() <= drects.len() + COPY_RECTS_MORE)) {
                        *drects = alt;
                        built = Some(n);
                        copy = true;
                    }
                }
            }
            (built, copy)
        };
        if !copy {
            plan = None;
        }
        let Some(dirty_cells) = built else {
            self.dirty_swap();
            return self.dirty_full_take(w, h);
        };
        // (U1) explicit damage (canvas_damage) of this window
        let n0 = self.drects.len() as i32;
        self.wp.dirty.n0 = n0;
        let n = self.dmg_take(n0, w, h);
        if n < 0 {
            self.wp.dirty.forced = true;
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
        if (dirty_cells.wrapping_mul(5) > total && !(force && plan.is_some())) || self.wp.dirty.full {
            self.dirty_swap();
            return self.dirty_full_take(w, h);
        }
        // the copies come first: the rectangles drawn next may cover their sources
        if let Some(p) = plan {
            self.sc_copy(&p, cols, rows);
        }
        self.dirty_swap();
        n
    }

    // .dc_full: everything; the pending damage of this window is covered
    fn dirty_full_take(&mut self, w: i32, h: i32) -> i32 {
        if self.dmg.iter().any(|d| d.win == self.uiw_cur) {
            self.wp.dirty.forced = true; // (the damage is not told apart any more)
        }
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
        core::mem::swap(&mut d.pre, &mut d.pre_prev);
    }
}
