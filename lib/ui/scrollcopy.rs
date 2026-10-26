// scrollcopy -- scroll copies: when only a scroll view's content moved, the pixels that stay
// valid are moved inside the framebuffer; the band that came into view and whatever really
// changed are drawn again
//
//   scroll_copy_begin(id, ox, oy)   the commands recorded until scroll_copy_end are the content
//                                   of scroll view `id` and move with it; (ox, oy) = window px
//                                   of the content's origin (its place minus the scroll offset).
//                                   Call right after the view's clip.  Scroll views, lists, data
//                                   views, vlist, document, code and multi-line text views do.
//   scroll_copy_end()
//   (dirty.rs) Pass: the content grids of a frame while dirty_compute hashes the commands;
//   sc_plan: the window cells a copy can make valid; sc_want: copy or draw; sc_copy: the copy
//   scopy_present(n) (render_live): the moved cells join the presented rectangles
//   scopy_done() (render_live, after drawing): the frame's cost for sc_want
//
// A view whose origin moved (its clip staying) in the last RECENT frames gets a grid of 32 x 32 px cells aligned
// at its content's origin, over its clip.  A cell's hash mixes, in drawing order, every content
// command touching it (hash and box relative to the origin, so a moved command keeps them) and
// every other command over the view that is flat there (a fill covering the cell's visible
// part: window background, cards; an opaque one forgets what was below).  Anything else over
// the cell (text or shapes of other layers, scroll bars, popups, the uneven edge of a fill) and
// content whose pixels may change when it moves (the rasterizer works in absolute f32
// coordinates: rounded corners, circles, lines; custom drawings, icons painted by the style or
// an icon hook, very large or left-cut text) marks the cell bad.  When the origin moved by
// (dx, dy) since the last frame and the view's clip stayed, a cell with its old hash that is
// bad in neither frame shows the same pixels moved by (dx, dy): the dirty window cells covered
// by such cells may be copied instead of drawn -- byte-identical to a full redraw
// (--verify-dirty checks it).
//
// A copy reads and writes a cell's pixels: filling a flat cell again is cheaper, drawing text
// or icons is not, and every rectangle left to draw replays the commands.  So only cells with
// detail are copied, small gaps between drawn cells are drawn too, a copy must take over a
// quarter of what would be drawn, and per view the measured cost of frames that copied and of
// frames that drew decides (the other way is tried again every EXPLORE frames; the first
// frames try both); a view that moved IDLE_MAX frames in a row with grids but without copying
// gets none for SKIP frames.  Frames checked by --verify-dirty always copy when they can (and
// never skip).  No copy: a horizontal move in a mirrored window, the inspector, pending canvas
// damage, a full redraw.  `expect scrollcopy` (auto.rs) counts the frames that copied,
// jfgui_DEBUG=1 prints them.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::cmd::{pos_mix, Cmd, ICON_PAINTED};
use super::Ui;

/// content cells: 32 x 32 px
const CS_SHIFT: u32 = 5;
const SEED: u64 = 0x243F_6A88_85A3_08D3;
const HK: u64 = 0x9E37_79B9_7F4A_7C15;
/// other commands over a view: their hashes are told apart from the content's
const OTHER: u64 = 0x5851_F42D_4C95_7F2D;
/// views smaller than this (px) get no grid
const AREA_MIN: i64 = 96 * 96;
/// grids per window and frame (further views get none)
const GRIDS_MAX: usize = 16;
/// a view gets grids while its origin moved within this many rendered frames
const RECENT: u32 = 60;
/// the way that costs more is tried again every this many frames (its cost may have changed)
const EXPLORE: u32 = 16;
/// views whose costs are kept per window
const STATS_MAX: usize = 16;
/// a view that could have copied in this many frames in a row but did not gets no grids for
/// SKIP frames (they cost a pass over its commands)
const IDLE_MAX: u32 = 4;
const SKIP: u32 = 60;
/// text and icons larger than this (px) may skip the glyph cache (drawn as paths): not copied
const TEXT_MAX_PX: i32 = 128;
/// copied cells between two drawn ones in a row: drawn too when at most this many
const GAP_MAX: usize = 2;

/// a scroll view of the pass: its content is records [i0, i1) of `layer`
#[derive(Clone, Copy)]
struct Rec {
    id: u32,
    layer: u8,
    i0: u32,
    i1: u32,
    ox: i64,
    oy: i64,
    /// the (logical) clip at its start: a view that moved with its clip was laid out anew
    clip: [i32; 4],
}

/// the mean cost (us, 0 = not known) of a view's frames that drew and that copied; frames
/// that could copy; frames in a row that compared grids but did not copy; frames left without
/// grids
#[derive(Clone, Copy, Default)]
struct Stat {
    id: u32,
    draw: u32,
    copy: u32,
    n: u32,
    idle: u32,
    skip: u32,
}

// the view's Stat (a new one replaces the oldest when STATS_MAX are kept)
fn stat(v: &mut Vec<Stat>, id: u32) -> &mut Stat {
    let k = match v.iter().position(|e| e.id == id) {
        Some(k) => k,
        None => {
            if v.len() >= STATS_MAX {
                v.remove(0);
            }
            v.push(Stat { id, ..Stat::default() });
            v.len() - 1
        }
    };
    &mut v[k]
}

/// a content grid: the cells of one view in one frame
#[derive(Default)]
pub(crate) struct Grid {
    id: u32,
    /// the view's clip at its content's start (x0, y0, x1, y1), cut to the window
    r: [i32; 4],
    ox: i64,
    oy: i64,
    /// the first cell (in cells from the origin), cells per row, rows
    kx: i64,
    ky: i64,
    nx: usize,
    ny: usize,
    h: Vec<u64>,
    bad: Vec<bool>,
    /// content beyond flat fills (text, icons, images, shapes): worth a copy
    det: Vec<bool>,
    // ---- while hashing
    layer: usize,
    i0: usize,
    i1: usize,
    live: bool,
    /// the clips opened inside the content (intersections; x0, y0, x1, y1)
    cs: Vec<[i32; 4]>,
}

/// per-window part: the views recorded by this pass, where they were (id, origin, clip, the
/// frame they last moved), the grids of the last frame, the cells copied by this frame (x, y,
/// w, h; presented), the frame being drawn (its start in us, what it did with a possible copy:
/// 0 none possible, 1 drew, 2 copied; about which view), the views' costs
#[derive(Default)]
pub(crate) struct W {
    recs: Vec<Rec>,
    open: Vec<usize>,
    seen: Vec<(u32, i64, i64, [i32; 4], u32)>,
    tick: u32,
    prev: Vec<Grid>,
    shown: Vec<[i32; 4]>,
    t0: u64,
    mode: u8,
    key: u32,
    stats: Vec<Stat>,
    /// the views whose grids met last frame's, the views that copied (this frame)
    tried: Vec<u32>,
    copied: Vec<u32>,
}

impl W {
    /// cmd_reset: a new pass records its views again
    pub(crate) fn reset(&mut self) {
        self.recs.clear();
        self.open.clear();
    }
}

/// shared: what the copies did since the last `expect scrollcopy`
#[derive(Default)]
pub(crate) struct S {
    pub(crate) frames: u32,
    pub(crate) cells: u64,
}

/// how a command may change pixels when moved
enum Odd {
    No,
    All,
    /// the four corner squares (side k) of the box
    Corners(i32, [i32; 4]),
}

/// a command's flat part (window px, before its clip): a fill of one colour (opaque: hides
/// what is below), or a hole where it draws nothing
enum Flat {
    No,
    Fill([i32; 4], bool),
    Hole([i32; 4]),
}

fn isect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

fn empty(a: &[i32; 4]) -> bool {
    a[2] <= a[0] || a[3] <= a[1]
}

#[inline]
fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(7) ^ v).wrapping_mul(HK)
}

// content commands whose pixels may change when they move by whole pixels: the rasterizer
// works in absolute f32 coordinates (curves), glyphs too large for the cache are paths, the
// "natural" pen rounding differs left of 0, custom drawings and painted icons are unknown
fn odd_moved(c: &Cmd, icons_exact: bool) -> Odd {
    match *c {
        Cmd::Rect { .. } | Cmd::Image { .. } | Cmd::Clip { .. } | Cmd::Unclip => Odd::No,
        Cmd::Text { x, size, .. } => {
            if x < 0 || size >> 8 > TEXT_MAX_PX { Odd::All } else { Odd::No }
        }
        Cmd::Icon { x, size, .. } => {
            if !icons_exact || x < 0 || size >> 8 > TEXT_MAX_PX { Odd::All } else { Odd::No }
        }
        Cmd::RRect { x, y, w, h, r, t, .. } => {
            // only the corner bands are paths (shapes.rs); without a radius plain fills
            let r = (r as i32).min(w.min(h) >> 1);
            if r <= 0 { Odd::No } else { Odd::Corners(r.max(t as i32) + 1, [x, y, x.wrapping_add(w), y.wrapping_add(h)]) }
        }
        Cmd::Shadow { x, y, w, h, r, depth: d, .. } => {
            if d <= 0 {
                return Odd::No;
            }
            // rings of fill_rrect(x - i, y - i + down, w + 2i, h + 2i, r + i), i = 1 .. d
            let down = (d as u32 / 3) as i32;
            Odd::Corners(r.max(0) + d + 2, [x - d, y - d + down, x + w + d, y + h + d + down])
        }
        Cmd::PanelFx { .. } | Cmd::Circle { .. } | Cmd::Line { .. } | Cmd::Custom { .. } => Odd::All,
    }
}

// the flat part of a command (see Flat)
fn flat(c: &Cmd) -> Flat {
    match *c {
        Cmd::Rect { x, y, w, h, c } => Flat::Fill([x, y, x.wrapping_add(w), y.wrapping_add(h)], c >> 24 == 255),
        Cmd::RRect { x, y, w, h, r, t, fill, .. } => {
            // inside the border and the corner bands: the plain fill rows of fill_rrect
            let (r, t) = ((r as i32).min(w.min(h) >> 1), t as i32);
            let k = if r <= 0 { t } else { r.max(t) + 1 };
            let b = [x + k, y + k, x + w - k, y + h - k];
            match fill >> 24 {
                0 => Flat::Hole(b),
                a => Flat::Fill(b, a == 255),
            }
        }
        Cmd::Shadow { x, y, w, h, r, depth: d, .. } => {
            if d <= 0 {
                return Flat::Hole([x, y, x.wrapping_add(w), y.wrapping_add(h)]);
            }
            // every ring's plain fill rows (translucent)
            let down = (d as u32 / 3) as i32;
            let r = r.max(0);
            Flat::Fill([x + r + 1, y + down + r + 1, x + w - r - 1, y + down + h - r - 1], false)
        }
        _ => Flat::No,
    }
}

// content worth a copy: drawing it costs more than moving its pixels (not a plain fill)
fn detail(c: &Cmd) -> bool {
    !matches!(c, Cmd::Rect { .. } | Cmd::RRect { .. } | Cmd::Shadow { .. } | Cmd::Clip { .. } | Cmd::Unclip)
}

impl Grid {
    // cells (grid indices [a, b) x [c, d)) touched by the window rectangle q
    fn cells(&self, q: &[i32; 4]) -> Option<(usize, usize, usize, usize)> {
        if empty(q) {
            return None;
        }
        let a = ((q[0] as i64 - self.ox) >> CS_SHIFT) - self.kx;
        let b = ((q[2] as i64 - 1 - self.ox) >> CS_SHIFT) - self.kx + 1;
        let c = ((q[1] as i64 - self.oy) >> CS_SHIFT) - self.ky;
        let d = ((q[3] as i64 - 1 - self.oy) >> CS_SHIFT) - self.ky + 1;
        let (a, b) = (a.max(0), b.min(self.nx as i64));
        let (c, d) = (c.max(0), d.min(self.ny as i64));
        if a >= b || c >= d {
            return None;
        }
        Some((a as usize, b as usize, c as usize, d as usize))
    }

    // the cells (grid indices [a, b) x [c, d), maybe empty) whose part inside `bound` lies inside
    // f: a run per axis
    fn inner(&self, f: &[i32; 4], bound: &[i32; 4]) -> (usize, usize, usize, usize) {
        let axis = |o: i64, k: i64, n: usize, f0: i32, f1: i32, b0: i32, b1: i32| {
            // cell i spans o + (k + i) * 32 ..+ 32; its part max(.., b0) .. min(.., b1)
            let a = if b0 >= f0 { 0 } else { -((o - f0 as i64) >> CS_SHIFT) - k };
            let e = if b1 <= f1 { n as i64 } else { ((f1 as i64 - o) >> CS_SHIFT) - k };
            (a.clamp(0, n as i64) as usize, e.clamp(0, n as i64) as usize)
        };
        let (a, b) = axis(self.ox, self.kx, self.nx, f[0], f[2], bound[0], bound[2]);
        let (c, d) = axis(self.oy, self.ky, self.ny, f[1], f[3], bound[1], bound[3]);
        (a, b.max(a), c, d.max(c))
    }

    // mark the cells touched by q bad
    fn spoil(&mut self, q: &[i32; 4]) {
        if let Some((a, b, c, d)) = self.cells(q) {
            for j in c..d {
                self.bad[j * self.nx + a..j * self.nx + b].fill(true);
            }
        }
    }

    // a content command: its box inside the content's own clips
    fn content(&mut self, c: &Cmd, parts: (u64, i32, i32, u32), bb: [i32; 4], icons_exact: bool) {
        let e = match self.cs.last() {
            Some(k) => isect(bb, *k),
            None => bb,
        };
        let Some((a, b, cc, d)) = self.cells(&e) else { return };
        let (body, x, y, sh) = parts;
        let mut hc = pos_mix(body, x as i64 - (self.ox << sh), y as i64 - (self.oy << sh));
        hc = mix(hc, ((e[0] as i64 - self.ox) as u64) ^ ((e[1] as i64 - self.oy) as u64) << 32);
        hc = mix(hc, ((e[2] as i64 - self.ox) as u64) ^ ((e[3] as i64 - self.oy) as u64) << 32);
        let det = detail(c);
        for j in cc..d {
            let row = j * self.nx;
            for v in &mut self.h[row + a..row + b] {
                *v = mix(*v, hc);
            }
            if det {
                self.det[row + a..row + b].fill(true);
            }
        }
        match odd_moved(c, icons_exact) {
            Odd::No => {}
            Odd::All => self.spoil(&e),
            Odd::Corners(k, q) => {
                for s in [
                    [q[0], q[1], q[0] + k, q[1] + k],
                    [q[2] - k, q[1], q[2], q[1] + k],
                    [q[0], q[3] - k, q[0] + k, q[3]],
                    [q[2] - k, q[3] - k, q[2], q[3]],
                ] {
                    self.spoil(&isect(s, e));
                }
            }
        }
    }

    // another command over the view, drawn in `clip` (exact: all of its levels tracked)
    fn other(&mut self, c: &Cmd, bb: [i32; 4], clip: [i32; 4], exact: bool, hw: u64) {
        let e = isect(bb, clip);
        let ea = isect(e, self.r);
        let Some((a, b, cc, d)) = self.cells(&ea) else { return };
        let hw = hw ^ OTHER;
        let fl = if exact { flat(c) } else { Flat::No };
        // the cells it is flat over (or draws nothing over)
        let (ia, ib, ic, id) = match fl {
            Flat::Fill(f, _) => self.inner(&isect(f, clip), &self.r),
            Flat::Hole(f) => self.inner(&f, &self.r),
            Flat::No => (0, 0, 0, 0),
        };
        let opaque = matches!(fl, Flat::Fill(_, true));
        let fill = matches!(fl, Flat::Fill(..));
        for j in cc..d {
            let row = j * self.nx;
            let inj = j >= ic && j < id;
            for i in a..b {
                let k = row + i;
                if !(inj && i >= ia && i < ib) {
                    self.bad[k] = true;
                } else if fill {
                    if opaque {
                        self.h[k] = SEED;
                        self.bad[k] = false;
                        self.det[k] = false;
                    }
                    self.h[k] = mix(self.h[k], hw);
                }
            }
        }
    }
}

/// the grids of one frame while dirty_compute hashes the commands (in drawing order)
#[derive(Default)]
pub(crate) struct Pass {
    grids: Vec<Grid>,
    /// grid indices by (layer, i0): where each one starts
    starts: Vec<usize>,
    next: usize,
    icons_exact: bool,
    w: i32,
    h: i32,
}

impl Pass {
    /// any grid to fill
    pub(crate) fn on(&self) -> bool {
        !self.grids.is_empty()
    }

    /// before record i of layer l (clip: the clip there, exact: all of its levels tracked):
    /// the grids whose content starts here take their view's clip
    pub(crate) fn reach(&mut self, l: usize, i: usize, clip: [i32; 4], exact: bool) {
        while let Some(&g) = self.starts.get(self.next) {
            let gr = &mut self.grids[g];
            if (gr.layer, gr.i0) > (l, i) {
                break;
            }
            self.next += 1;
            if (gr.layer, gr.i0) < (l, i) {
                continue; // (never reached: an empty layer part)
            }
            let r = isect(clip, [0, 0, self.w, self.h]);
            if !exact || empty(&r) || ((r[2] - r[0]) as i64 * (r[3] - r[1]) as i64) < AREA_MIN {
                continue;
            }
            gr.r = r;
            gr.kx = (r[0] as i64 - gr.ox) >> CS_SHIFT;
            gr.ky = (r[1] as i64 - gr.oy) >> CS_SHIFT;
            gr.nx = (((r[2] as i64 - 1 - gr.ox) >> CS_SHIFT) - gr.kx + 1) as usize;
            gr.ny = (((r[3] as i64 - 1 - gr.oy) >> CS_SHIFT) - gr.ky + 1) as usize;
            gr.h = vec![SEED; gr.nx * gr.ny];
            gr.bad = vec![false; gr.nx * gr.ny];
            gr.det = vec![false; gr.nx * gr.ny];
            gr.live = true;
        }
    }

    /// a clip record (x0, y0, x1, y1) at record i of layer l
    pub(crate) fn clip(&mut self, l: usize, i: usize, q: [i32; 4]) {
        for g in self.grids.iter_mut().filter(|g| g.live && g.layer == l && i >= g.i0 && i < g.i1) {
            let k = g.cs.last().map_or(q, |k| isect(*k, q));
            g.cs.push(k);
        }
    }

    /// an unclip record at record i of layer l
    pub(crate) fn unclip(&mut self, l: usize, i: usize) {
        for g in self.grids.iter_mut().filter(|g| g.live && g.layer == l && i >= g.i0 && i < g.i1) {
            g.cs.pop();
        }
    }

    /// a drawing record i of layer l: parts (cmd_parts), its box bb (x0, y0, x1, y1), the clip
    /// it is drawn in (exact: all levels tracked), its window hash (with the clip)
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cmd(&mut self, l: usize, i: usize, c: &Cmd, parts: (u64, i32, i32, u32), bb: [i32; 4], clip: [i32; 4], exact: bool, hw: u64) {
        let icons_exact = self.icons_exact;
        for g in self.grids.iter_mut().filter(|g| g.live) {
            if g.layer == l && i >= g.i0 && i < g.i1 {
                g.content(c, parts, bb, icons_exact);
            } else {
                g.other(c, bb, clip, exact, hw);
            }
        }
    }
}

/// a planned copy: per window cell (row-major) the copy index + 1 of a copy that would make
/// it valid (dirty_compute clears the cells it draws), each copy's view and move
pub(crate) struct Plan {
    pub(crate) covered: Vec<u8>,
    moves: Vec<(u32, i32, i32)>,
}

impl Plan {
    /// (dirty_compute) changed(i): the hash of window cell i changed (cols per row).  Unchanged
    /// cells need no copy; a few covered cells between two drawn ones in a row are drawn too
    /// (fewer, longer rectangles) -> the cells left to copy
    pub(crate) fn settle(&mut self, cols: usize, changed: impl Fn(usize) -> bool) -> usize {
        let cv = &mut self.covered;
        for (i, t) in cv.iter_mut().enumerate() {
            if !changed(i) {
                *t = 0; // the pixels are there already
            }
        }
        let mut left = 0;
        for start in (0..cv.len()).step_by(cols.max(1)) {
            let drawn = |cv: &[u8], i: usize| cv[start + i] == 0 && changed(start + i);
            let mut i = 0;
            while i < cols {
                if cv[start + i] == 0 {
                    i += 1;
                    continue;
                }
                // a run of covered cells [i, e)
                let mut e = i + 1;
                while e < cols && cv[start + e] != 0 {
                    e += 1;
                }
                if e - i <= GAP_MAX && i > 0 && e < cols && drawn(cv, i - 1) && drawn(cv, e) {
                    cv[start + i..start + e].fill(0);
                } else {
                    left += e - i;
                }
                i = e;
            }
        }
        left
    }
}

impl Ui {
    /// ui_scroll_copy_begin(id, ox, oy): the commands recorded until scroll_copy_end are the
    /// content of scroll view `id`; (ox, oy) = the window px of the content's origin (its
    /// position minus the scroll offset).  Call right after the view's clip.  When only the
    /// origin moved between two frames, pixels still shown may be moved in the framebuffer
    /// instead of drawn again (content that does not move with the origin is simply drawn).
    pub fn scroll_copy_begin(&mut self, id: u32, ox: i64, oy: i64) {
        let layer = self.wp.cmd.layer;
        let i0 = self.wp.cmd.layers[layer].len() as u32;
        let clip = self.wp.ctx.uiclip;
        let s = &mut self.wp.scopy;
        s.recs.push(Rec { id, layer: layer as u8, i0, i1: u32::MAX, ox, oy, clip });
        s.open.push(s.recs.len() - 1);
    }

    /// ui_scroll_copy_end: the end of the content of scroll_copy_begin
    pub fn scroll_copy_end(&mut self) {
        let Ui { wp, .. } = self;
        if let Some(k) = wp.scopy.open.pop() {
            if let Some(r) = wp.scopy.recs.get_mut(k) {
                r.i1 = wp.cmd.layers[r.layer as usize].len() as u32;
            }
        }
    }

    /// (dirty_compute) the grids to fill this frame: the closed views of the last pass whose
    /// origin moved lately
    pub(crate) fn sc_pass(&mut self, w: i32, h: i32) -> Pass {
        let icons_exact = self.icon_hook.is_none() && !ICON_PAINTED.load(Ordering::Relaxed);
        let (ins, verify) = (self.m.ins.on, self.verify_dirty);
        let s = &mut self.wp.scopy;
        s.shown.clear();
        s.tick = s.tick.wrapping_add(1);
        s.t0 = plat::time_us();
        s.mode = 0;
        // the last frame: views that moved since their last grid without copying idle, IDLE_MAX
        // frames in a row and they skip SKIP frames
        for id in core::mem::take(&mut s.tried) {
            let copied = s.copied.contains(&id);
            let st = stat(&mut s.stats, id);
            st.idle = if copied { 0 } else { st.idle + 1 };
            if st.idle >= IDLE_MAX {
                st.idle = 0;
                st.skip = SKIP;
            }
        }
        s.copied.clear();
        let mut p = Pass { w, h, icons_exact, ..Pass::default() };
        // where the views are (scrolled: the origin moved, the clip stayed); those not built in
        // this pass are forgotten
        let mut seen = Vec::with_capacity(s.recs.len());
        for r in s.recs.iter().filter(|r| r.i1 != u32::MAX && r.i1 > r.i0) {
            let moved = match s.seen.iter().find(|e| e.0 == r.id) {
                Some(e) if (e.1 == r.ox && e.2 == r.oy) || e.3 != r.clip => e.4,
                Some(_) => s.tick,
                None => s.tick.wrapping_sub(RECENT + 1),
            };
            seen.push((r.id, r.ox, r.oy, r.clip, moved));
            let skip = match s.stats.iter_mut().find(|e| e.id == r.id) {
                Some(st) if st.skip > 0 && !verify => {
                    st.skip -= 1;
                    true
                }
                _ => false,
            };
            // the overlay is drawn into the framebuffer: no copies under the inspector
            if !ins && !skip && s.tick.wrapping_sub(moved) <= RECENT && p.grids.len() < GRIDS_MAX {
                p.grids.push(Grid { id: r.id, ox: r.ox, oy: r.oy, layer: r.layer as usize, i0: r.i0 as usize, i1: r.i1 as usize, ..Grid::default() });
            }
        }
        s.seen = seen;
        let mut st: Vec<usize> = (0..p.grids.len()).collect();
        st.sort_by_key(|&g| (p.grids[g].layer, p.grids[g].i0));
        p.starts = st;
        p
    }

    /// (dirty_compute, after hashing) compare the grids with the last frame's (they become the
    /// last frame's): the window cells (cols x rows) whose pixels a copy can provide; None: no
    /// copy (allowed = false: this frame copies nothing)
    pub(crate) fn sc_plan(&mut self, p: Pass, cols: i32, rows: i32, allowed: bool) -> Option<Plan> {
        let grids: Vec<Grid> = p
            .grids
            .into_iter()
            .filter(|g| g.live)
            .map(|mut g| {
                g.cs = Vec::new();
                g
            })
            .collect();
        let prev = core::mem::replace(&mut self.wp.scopy.prev, grids);
        if !allowed || prev.is_empty() {
            return None;
        }
        let (w, h) = (p.w, p.h);
        let rtl = self.rtl;
        let grids = &self.wp.scopy.prev;
        // (grid, move, the part of the view whose pixels come from inside it)
        let mut cand: Vec<(&Grid, &Grid, i32, i32, [i32; 4])> = Vec::new();
        let mut tried = Vec::new();
        for g in grids {
            let Some(o) = prev.iter().find(|o| o.id == g.id) else { continue };
            let (dx, dy) = (g.ox - o.ox, g.oy - o.oy);
            let r = g.r;
            if o.r != r || (dx == 0 && dy == 0) || dx.abs() >= (r[2] - r[0]) as i64 || dy.abs() >= (r[3] - r[1]) as i64 || (rtl && dx != 0) {
                continue;
            }
            let (dx, dy) = (dx as i32, dy as i32);
            let q = [r[0].max(r[0] + dx), r[1].max(r[1] + dy), r[2].min(r[2] + dx), r[3].min(r[3] + dy)];
            cand.push((g, o, dx, dy, q));
            tried.push(g.id); // (moved since its last grid: the back-off of sc_pass)
        }
        self.wp.scopy.tried = tried;
        // views over each other: neither copies
        let keep: Vec<bool> = (0..cand.len()).map(|a| (0..cand.len()).all(|b| a == b || empty(&isect(cand[a].0.r, cand[b].0.r)))).collect();
        let mut covered = vec![0u8; cols as usize * rows as usize];
        let mut moves = Vec::new();
        for (m, &(g, o, dx, dy, q)) in cand.iter().enumerate() {
            if !keep[m] {
                continue;
            }
            // the content cells with their old hash, good in both frames, with detail
            let (sx, sy) = (g.kx - o.kx, g.ky - o.ky);
            let mut v = vec![false; g.h.len()];
            for j in 0..g.ny {
                let pj = j as i64 + sy;
                if pj < 0 || pj >= o.ny as i64 {
                    continue;
                }
                for i in 0..g.nx {
                    let pi = i as i64 + sx;
                    if pi < 0 || pi >= o.nx as i64 {
                        continue;
                    }
                    let (k, pk) = (j * g.nx + i, pj as usize * o.nx + pi as usize);
                    v[k] = !g.bad[k] && !o.bad[pk] && o.h[pk] == g.h[k];
                }
            }
            // the window cells inside q made of valid content cells only, one with detail (a
            // window cell meets one or two content cells per axis)
            let tag = moves.len() as u8 + 1;
            let span = |c: i32, lim: i32, o: i64, k: i64| {
                let (a, b) = (c << 5, ((c + 1) << 5).min(lim));
                ((((a as i64 - o) >> CS_SHIFT) - k), (((b as i64 - 1 - o) >> CS_SHIFT) - k), a, b)
            };
            let (c0, c1) = ((q[0] >> 5).max(0), ((q[2] + 31) >> 5).min(cols));
            let (r0, r1) = ((q[1] >> 5).max(0), ((q[3] + 31) >> 5).min(rows));
            let xs: Vec<(i64, i64, i32, i32)> = (c0..c1).map(|c| span(c, w, g.ox, g.kx)).collect();
            let mut any = false;
            for row in r0..r1 {
                let (j0, j1, y0, y1) = span(row, h, g.oy, g.ky);
                if y0 < q[1] || y1 > q[3] || j0 < 0 || j1 >= g.ny as i64 {
                    continue;
                }
                for (n, &(i0, i1, x0, x1)) in xs.iter().enumerate() {
                    if x0 < q[0] || x1 > q[2] || i0 < 0 || i1 >= g.nx as i64 {
                        continue;
                    }
                    let ks = [j0 as usize * g.nx + i0 as usize, j0 as usize * g.nx + i1 as usize, j1 as usize * g.nx + i0 as usize, j1 as usize * g.nx + i1 as usize];
                    if ks.iter().all(|&k| v[k]) && ks.iter().any(|&k| g.det[k]) {
                        covered[(row * cols + c0 + n as i32) as usize] = tag;
                        any = true;
                    }
                }
            }
            if any {
                moves.push((g.id, dx, dy));
            }
        }
        if moves.is_empty() {
            return None;
        }
        Some(Plan { covered, moves })
    }

    /// (dirty_compute) a copy is possible this frame: take it?  Checked frames (--verify-dirty)
    /// always do; else the view's measured costs decide (see the module header).  The frame
    /// counts as one that drew until sc_copy copies.
    pub(crate) fn sc_want(&mut self, plan: &Plan) -> bool {
        let verify = self.verify_dirty;
        let s = &mut self.wp.scopy;
        s.mode = 1;
        s.key = plan.moves.first().map_or(0, |m| m.0);
        if verify {
            return true;
        }
        let st = stat(&mut s.stats, s.key);
        st.n = st.n.wrapping_add(1);
        if st.draw == 0 || st.copy == 0 {
            return st.draw != 0; // the first frames: draw once, copy once
        }
        let better = st.copy < st.draw;
        if st.n % EXPLORE == 0 { !better } else { better }
    }

    /// (render_live, after drawing) the cost of a frame that could copy: what it took since
    /// dirty_compute began joins the view's mean for the way it went (not for checked frames)
    pub(crate) fn scopy_done(&mut self) {
        let verify = self.verify_dirty;
        let s = &mut self.wp.scopy;
        let mode = core::mem::take(&mut s.mode);
        if mode == 0 || verify {
            return;
        }
        let dt = plat::time_us().saturating_sub(s.t0).clamp(1, 1 << 28) as u32;
        let key = s.key;
        if let Some(st) = s.stats.iter_mut().find(|e| e.id == key) {
            let v = if mode == 2 { &mut st.copy } else { &mut st.draw };
            *v = if *v == 0 { dt } else { (*v * 3 + dt) / 4 };
        }
    }

    /// (dirty_compute, no full redraw) do the planned copies of the covered cells: rows in the
    /// order that reads every source before it is overwritten; the moved areas are presented
    /// (scopy_present)
    pub(crate) fn sc_copy(&mut self, plan: &Plan, cols: i32, rows: i32) {
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let stride = self.wp.rt.fb.stride as usize;
        let debug = self.ui_debug;
        let mut done = 0u64;
        for (m, &(id, dx, dy)) in plan.moves.iter().enumerate() {
            let tag = m as u8 + 1;
            // the cells of this copy as runs per cell row (row, col0, col1)
            let mut runs: Vec<(i32, i32, i32)> = Vec::new();
            for row in 0..rows {
                let cv = &plan.covered[(row * cols) as usize..((row + 1) * cols) as usize];
                let mut col = 0;
                while col < cols as usize {
                    if cv[col] != tag {
                        col += 1;
                        continue;
                    }
                    let mut end = col + 1;
                    while end < cols as usize && cv[end] == tag {
                        end += 1;
                    }
                    runs.push((row, col as i32, end as i32));
                    col = end;
                }
            }
            if runs.is_empty() {
                continue;
            }
            // cell rows downwards when the content moves up (the source is below), else upwards;
            // along a row away from the source
            if dy > 0 {
                runs.reverse();
            }
            let mut bbox = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
            let ncell: u64 = runs.iter().map(|r| (r.2 - r.1) as u64).sum();
            let px = &mut self.wp.rt.fb.px;
            let mut k = 0;
            while k < runs.len() {
                let row = runs[k].0;
                let mut e = k;
                while e < runs.len() && runs[e].0 == row {
                    e += 1;
                }
                let mut rr: Vec<(i32, i32)> = runs[k..e].iter().map(|r| (r.1, r.2)).collect();
                rr.sort_unstable();
                if dx > 0 {
                    rr.reverse();
                }
                let (y0, y1) = (row << 5, ((row + 1) << 5).min(h));
                for t in 0..(y1 - y0) {
                    let y = if dy > 0 { y1 - 1 - t } else { y0 + t };
                    let sy = y - dy;
                    for &(c0, c1) in &rr {
                        let (x0, x1) = (c0 << 5, (c1 << 5).min(w));
                        let src = sy as usize * stride + (x0 - dx) as usize * 4;
                        let dst = y as usize * stride + x0 as usize * 4;
                        px.copy_within(src..src + (x1 - x0) as usize * 4, dst);
                    }
                }
                for &(c0, c1) in &rr {
                    bbox = [bbox[0].min(c0 << 5), bbox[1].min(y0), bbox[2].max((c1 << 5).min(w)), bbox[3].max(y1)];
                }
                k = e;
            }
            self.wp.scopy.shown.push([bbox[0], bbox[1], bbox[2] - bbox[0], bbox[3] - bbox[1]]);
            self.wp.scopy.copied.push(id);
            done += ncell;
            if debug {
                plat::write_stderr(alloc::format!("render: scroll copy #{:x} by {},{}: {} cells\n", id, dx, dy, ncell).as_bytes());
            }
        }
        if done != 0 {
            self.wp.scopy.mode = 2;
            self.m.scopy.frames = self.m.scopy.frames.wrapping_add(1);
            self.m.scopy.cells = self.m.scopy.cells.wrapping_add(done);
        }
    }

    /// (render_live) the presented rectangles: n dirty ones (-1 = all) and the cells a scroll
    /// copy moved (appended to drects) -> their count
    pub(crate) fn scopy_present(&mut self, n: i32) -> i32 {
        let s = &self.wp.scopy.shown;
        if n < 0 || s.is_empty() {
            return n;
        }
        self.drects.truncate(n as usize);
        self.drects.extend_from_slice(s);
        n + s.len() as i32
    }
}
