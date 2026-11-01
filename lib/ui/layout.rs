// layout -- box layout (flex-like, measured over passes)              [src/ui/ui_layout.s]
//
//   box_begin(id, flags, gap)          open a container (id 0 = automatic)
//   box_end()                          close it
//   next_size(w, h)                    size of the next widget / box in DIP: > 0 fixed,
//                                      0 natural, < 0 fill weight
//   next_rect(x, y, w, h)              absolute placement of the next widget in DIP,
//                                      relative to the current box's content origin
//   next_rect_px(x, y, w, h)           absolute placement in window px (w / h 0 = natural)
//   next_flags(ST_*)   next_icon(cp)   style flags / icon for the next widget
//   spacer(dip)                        empty space along the main axis (< 0 = fill)
//   last_rect() -> [x, y, w, h]        the last placed widget (px)
//   box_rect() -> [x, y, w, h]         content rectangle of the current box (px)
//   box_id() -> id                     id of the current box
//   box_offset(dx, dy)                 scroll the current box's content
//   avail_width(iw) -> (px, fixed)     width the next widget will get (TX_WRAP)
// Internal: place(iw, ih), take_flags(), take_icon(), layout_root(), layout_end_root()
//
// A box lays its children out along its main axis (BOX_H: x, BOX_V: y).  Fill children
// share the space left by the others; because that is only known after all children were
// placed, the previous pass's totals are used and a pass is repeated when they change (the
// runtime caps this at 4 passes).
//
// BOX_WRAP (horizontal boxes): a child that does not fit after the others of its row starts
// a new row (a row's first child stays, however wide).  Each row is as tall as its tallest
// child; the cross-axis alignment (BOX_XCENTER ..., stretched: the row's height) and
// BOX_CENTER / BOX_END work per row with the previous pass's rows.  Natural size: the rows
// and the gaps between them high, as wide as one row with every child (the box is placed at
// most as wide as avail_width allows, so it wraps where the parent has less room).  Fill
// children share what is left only while everything fits on one row, else they keep their
// natural size.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::consts::*;
use super::{Sp, Ui};

// boxes inside each other (fixed: nesting; deeper ones warn and are left out)
pub(crate) const BOX_STACK_MAX: usize = 48;

/// layout frame (box stack)  [BX_*]
#[derive(Clone, Copy, Default)]
pub(crate) struct BoxFrame {
    pub(crate) id: u32,
    pub(crate) flags: u32,
    /// content rectangle (px, absolute): x, y, w, h -- index 0 / 1 by axis
    pub(crate) pos: [i32; 2],
    pub(crate) len: [i32; 2],
    /// next position along the main axis (absolute px)
    pub(crate) cur: i32,
    pub(crate) gap: i32,
    /// accumulated non-fill main size + gaps, fill weights; previous pass values
    pub(crate) fixed: i32,
    pub(crate) wsum: i32,
    pub(crate) fixedp: i32,
    pub(crate) wsump: i32,
    /// children placed
    pub(crate) n: i32,
    /// largest cross-axis child size
    pub(crate) cross: i32,
    pub(crate) pad: i32,
    /// bit 0: natural size along the parent's main axis, bit 1: cross axis (aligned),
    /// bit 2: cross axis stretched or filling (box_end reports the natural cross size)
    pub(crate) auto: u32,
    /// size the parent used along its main axis when placing us
    pub(crate) pmain: i32,
    /// automatic-id child boxes begun so far
    pub(crate) nbox: u32,
    /// a grid (grid.rs): its run (index + 1, 0 = a plain box); as a grid child: the parent
    /// grid's run and the cell (col, row, cols, rows)
    pub(crate) grid: u16,
    pub(crate) gcell: (u16, [u16; 4]),
}

/// a row of a wrapping box (BOX_WRAP): its height (the tallest child) and its children's
/// width with the gaps between them
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Row {
    pub(crate) h: i32,
    pub(crate) w: i32,
}

/// a wrapping box being built: its rows so far (the last one is being filled), that row's
/// top (px below the content start) and children, the previous pass's rows
#[derive(Default)]
pub(crate) struct Wrap {
    rows: Vec<Row>,
    top: i32,
    n: i32,
    prev: Vec<Row>,
}

// a box whose children wrap: horizontal with BOX_WRAP
fn wraps(flags: u32) -> bool {
    flags & (BOX_WRAP | BOX_V) == BOX_WRAP
}

// where a row of a wrapping box starts (px after the content start): BOX_CENTER / BOX_END
// by the row's width in the previous pass
fn row_start(flags: u32, len: i32, prev: Option<&Row>) -> i32 {
    let spare = prev.map_or(0, |r| len.wrapping_sub(r.w));
    if spare <= 0 {
        0
    } else if flags & BOX_CENTER != 0 {
        spare >> 1
    } else if flags & BOX_END != 0 {
        spare
    } else {
        0
    }
}

/// the per-window part of the layout (ui_layout_wb0 .. ui_layout_wb1)
#[derive(Default)]
pub(crate) struct W {
    pub(crate) boxes: Vec<BoxFrame>,
    pub(crate) next_w: i32,
    pub(crate) next_h: i32,
    pub(crate) next_rx: i32,
    pub(crate) next_ry: i32,
    pub(crate) next_rw: i32,
    pub(crate) next_rh: i32,
    /// 0 none, 1 next_rect (DIP), 2 next_rect_px
    pub(crate) next_rect_set: u32,
    pub(crate) next_flags: u32,
    pub(crate) next_icon: u32,
    pub(crate) place_auto: u32,
    pub(crate) place_main: i32,
    /// box_begin is placing a box: its natural cross size counts at its box_end (with this
    /// pass's measurement), not with the previous pass's at placement
    pub(crate) placing_box: bool,
    /// id of the window's root box (1 = main window)
    pub(crate) root_id: u32,
    /// grids (grid.rs): the open ones, their measured sizes, the next child's span / cell
    pub(crate) grid: super::grid::W,
    /// wrapping boxes (BOX_WRAP): the open ones, the rows each one measured (previous pass)
    pub(crate) wraps: Vec<Wrap>,
    pub(crate) wrap_rows: BTreeMap<u32, Vec<Row>>,
}

impl Ui {
    // frame_top: the top box frame (outside a pass, where the asm reads a stale frame, an
    // empty one)
    fn top(&mut self) -> &mut BoxFrame {
        let b = &mut self.wp.lay.boxes;
        if b.is_empty() {
            b.push(BoxFrame::default());
        }
        let n = b.len();
        &mut b[n - 1]
    }

    /// layout_root(x, y, w, h)
    pub(crate) fn layout_root(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let id = self.wp.lay.root_id;
        let s = *self.slot(id);
        let l = &mut self.wp.lay;
        l.boxes.clear();
        l.boxes.push(BoxFrame {
            id,
            flags: BOX_V,
            pos: [x, y],
            len: [w, h],
            cur: y,
            fixedp: s.fixed,
            wsump: s.wsum,
            ..BoxFrame::default()
        });
        l.next_w = 0;
        l.next_h = 0;
        l.next_rect_set = 0;
        l.next_flags = 0;
        l.next_icon = 0;
        l.grid.runs.clear();
        l.grid.span = (1, 1);
        l.grid.cell = None;
        l.grid.align = (0, 0);
        l.wraps.clear();
    }

    /// layout_end_root: close unbalanced boxes and store the root measurements
    pub(crate) fn layout_end_root(&mut self) {
        while self.wp.lay.boxes.len() > 1 {
            self.warn("ui_box_begin without ui_box_end");
            self.box_end();
        }
        let b = self.wp.lay.boxes.first().copied().unwrap_or_default();
        let id = self.wp.lay.root_id;
        let s = self.slot(id);
        let changed = b.fixed != s.fixed || b.wsum != s.wsum;
        s.fixed = b.fixed;
        s.wsum = b.wsum;
        if changed {
            self.repass = true;
        }
        self.wp.lay.boxes.clear();
    }

    /// ui_next_size(w, h): DIP; > 0 fixed, 0 natural, < 0 fill weight
    pub fn next_size(&mut self, w: i32, h: i32) {
        self.wp.lay.next_w = w;
        self.wp.lay.next_h = h;
    }

    /// ui_next_rect(x, y, w, h): DIP relative to the current box's content origin
    pub fn next_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let l = &mut self.wp.lay;
        l.next_rx = x;
        l.next_ry = y;
        l.next_rw = w;
        l.next_rh = h;
        l.next_rect_set = 1;
    }

    /// ui_next_rect_px(x, y, w, h): absolute placement of the next widget / box in window
    /// px (w or h 0 = natural size); nothing is accounted in the parent box
    pub fn next_rect_px(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let l = &mut self.wp.lay;
        l.next_rx = x;
        l.next_ry = y;
        l.next_rw = w;
        l.next_rh = h;
        l.next_rect_set = 2;
    }

    /// ui_box_offset(dx, dy): shift the content of the current box by (-dx, -dy) px
    pub fn box_offset(&mut self, dx: i32, dy: i32) {
        let b = self.top();
        b.pos[0] = b.pos[0].wrapping_sub(dx);
        b.pos[1] = b.pos[1].wrapping_sub(dy);
        if b.flags & BOX_V != 0 {
            b.cur = b.cur.wrapping_sub(dy);
        } else {
            b.cur = b.cur.wrapping_sub(dx);
        }
    }

    /// ui_next_flags(ST_*): style flags for the next widget (OR-ed)
    pub fn next_flags(&mut self, f: u32) {
        self.wp.lay.next_flags |= f;
    }

    /// ui_take_flags: the pending next_flags value (cleared; for widgets)
    pub fn take_flags(&mut self) -> u32 {
        core::mem::take(&mut self.wp.lay.next_flags)
    }

    /// ui_next_icon(cp): icon code point (IC_*) for the next widget
    pub fn next_icon(&mut self, cp: u32) {
        self.wp.lay.next_icon = cp;
    }

    /// ui_take_icon: the pending icon (cleared; for widgets)
    pub fn take_icon(&mut self) -> u32 {
        core::mem::take(&mut self.wp.lay.next_icon)
    }

    /// ui_last_rect: [x, y, w, h] of the last placed widget (px)
    pub fn last_rect(&self) -> [i32; 4] {
        self.wp.ctx.last_rect
    }

    /// ui_box_id: id of the current box (the root box's id at the top level; 0 outside a pass)
    pub fn box_id(&self) -> u32 {
        self.wp.lay.boxes.last().map_or(0, |b| b.id)
    }

    /// ui_box_rect: [x, y, w, h] content rectangle of the current box (px)
    pub fn box_rect(&self) -> [i32; 4] {
        let b = self.wp.lay.boxes.last().copied().unwrap_or_default();
        [b.pos[0], b.pos[1], b.len[0], b.len[1]]
    }

    /// flags of the current box (ui_box_rect - 8 + 4 in the asm)
    pub fn box_flags(&self) -> u32 {
        self.wp.lay.boxes.last().map_or(0, |b| b.flags)
    }

    /// ui_place(iw, ih) -> last_rect: place the next widget (natural size in px)
    pub fn place(&mut self, iw: i32, ih: i32) -> [i32; 4] {
        let dpi = self.wp.rt.dpi;
        let dp = |v: i32| dp_at(v, dpi);
        let mut repass = false;
        self.top();
        // a grid places its children in cells (grid.rs)
        let gk = self.wp.lay.boxes.last().map_or(0, |b| b.grid);
        if gk != 0 && self.wp.lay.next_rect_set == 0 {
            let r = self.grid_place(gk as usize, iw, ih);
            self.wp.ctx.last_rect = r;
            return r;
        }
        let l = &mut self.wp.lay;
        let r = if l.next_rect_set == 0 {
            let n = l.boxes.len();
            let b = &mut l.boxes[n - 1];
            let a = (b.flags & BOX_V) as usize; // main axis index
            let c = a ^ 1; // cross axis index
            // a wrapping box: its rows
            let mut wr = if wraps(b.flags) { l.wraps.last_mut() } else { None };
            // requested sizes along main and cross axis
            let (mut rmain, mut rcross) = (l.next_w, l.next_h);
            let (nat_main, mut nat_cross) = if a == 1 { (ih, iw) } else { (iw, ih) };
            if a == 1 {
                core::mem::swap(&mut rmain, &mut rcross);
            }
            // remember which sizes are natural (for box_end's late correction)
            let aligned = b.flags & (BOX_XCENTER | BOX_XSTART | BOX_XEND) != 0;
            let mut auto = 0;
            if rmain == 0 {
                auto |= 1;
            }
            if rcross == 0 && aligned {
                auto |= 2;
            } else if rcross <= 0 {
                // stretched or filling: the parent's natural cross size still counts ours
                auto |= 4;
            }
            l.place_auto = auto;
            // main size (in a wrapping box that does not fit on one row a fill child keeps its
            // natural size)
            let mut weight = 0;
            let main = if rmain == 0 {
                nat_main
            } else if rmain < 0 {
                weight = rmain.wrapping_neg();
                let spare = (b.len[a].wrapping_sub(b.fixedp)).max(0);
                if b.wsump == 0 {
                    repass = true;
                    if wr.is_some() { nat_main } else { 0 }
                } else if wr.is_some() && spare == 0 {
                    nat_main
                } else {
                    spare.wrapping_mul(weight) / b.wsump
                }
            } else {
                dp(rmain)
            };
            // a wrapping box: a new row when the child does not fit after the row's others (by
            // the row's width, so a row moved by BOX_CENTER / BOX_END breaks the same way)
            if let Some(w) = wr.as_deref_mut() {
                let used = w.rows.last().map_or(0, |r| r.w);
                if w.n != 0 && used.wrapping_add(b.gap).wrapping_add(main) > b.len[a] {
                    let h = w.rows.last().map_or(0, |r| r.h);
                    w.top = w.top.wrapping_add(h).wrapping_add(b.gap);
                    w.rows.push(Row::default());
                    w.n = 0;
                    b.cur = b.pos[a].wrapping_add(row_start(b.flags, b.len[a], w.prev.get(w.rows.len() - 1)));
                }
            }
            // the room across: the box's (a wrapping box: its row's height in the previous
            // pass, 0 while not known)
            let (start, room) = match wr.as_deref() {
                Some(w) => (b.pos[c].wrapping_add(w.top), w.prev.get(w.rows.len() - 1).map_or(0, |p| p.h)),
                None => (b.pos[c], b.len[c]),
            };
            // cross size
            let cross = if rcross > 0 {
                nat_cross = dp(rcross); // a fixed cross size is also natural
                nat_cross
            } else if (rcross == 0 && aligned) || (wr.is_some() && room == 0) {
                nat_cross
            } else {
                room
            };
            // gap before every child but the first (a wrapping box: but a row's first; its
            // natural width counts them all)
            let gap = match wr.as_deref() {
                Some(w) if w.n == 0 => 0,
                None if b.n == 0 => 0,
                _ => b.gap,
            };
            if b.n != 0 {
                b.fixed = b.fixed.wrapping_add(b.gap);
            }
            b.cur = b.cur.wrapping_add(gap);
            b.n += 1;
            l.place_main = main;
            if weight == 0 {
                b.fixed = b.fixed.wrapping_add(main);
            } else {
                b.wsum = b.wsum.wrapping_add(weight);
            }
            let counts = !(l.placing_box && auto & 6 != 0);
            if nat_cross > b.cross && counts {
                b.cross = nat_cross;
            }
            if let Some(w) = wr.as_deref_mut() {
                w.n += 1;
                if let Some(row) = w.rows.last_mut() {
                    row.w = row.w.wrapping_add(gap).wrapping_add(main);
                    if nat_cross > row.h && counts {
                        row.h = nat_cross;
                    }
                }
            }
            // position: main = cursor, cross = start of the room (+ alignment)
            let mut r = [0i32; 4];
            r[a] = b.cur;
            r[2 + a] = main;
            b.cur = b.cur.wrapping_add(main);
            let mut spare = room.wrapping_sub(cross);
            if wr.is_some() {
                spare = spare.max(0); // a row not measured yet: at its top
            }
            let mut cp = start;
            if b.flags & BOX_XCENTER != 0 {
                cp = cp.wrapping_add(spare >> 1);
            } else if b.flags & BOX_XEND != 0 {
                cp = cp.wrapping_add(spare);
            }
            r[c] = cp;
            r[2 + c] = cross;
            r
        } else if l.next_rect_set == 2 {
            // next_rect_px: window px, 0 = natural size
            l.next_rect_set = 0;
            l.place_auto = 0;
            [
                l.next_rx,
                l.next_ry,
                if l.next_rw != 0 { l.next_rw } else { iw },
                if l.next_rh != 0 { l.next_rh } else { ih },
            ]
        } else {
            // absolute placement relative to the box content origin
            l.next_rect_set = 0;
            l.place_auto = 0;
            let n = l.boxes.len();
            let b = l.boxes[n - 1];
            [
                dp(l.next_rx).wrapping_add(b.pos[0]),
                dp(l.next_ry).wrapping_add(b.pos[1]),
                if l.next_rw != 0 { dp(l.next_rw) } else { iw },
                if l.next_rh != 0 { dp(l.next_rh) } else { ih },
            ]
        };
        l.next_w = 0;
        l.next_h = 0;
        self.wp.ctx.last_rect = r;
        if repass {
            self.repass = true;
        }
        r
    }

    /// ui_avail_width(iw) -> (width px a wrapping widget of natural (unwrapped) width iw
    /// gets from the next place, 1 when that width does not depend on iw) -- no side
    /// effects.  0 = unknown (no wrapping).
    pub fn avail_width(&self, iw: i32) -> (i32, i32) {
        let l = &self.wp.lay;
        if l.next_rect_set != 0 {
            if l.next_rw == 0 {
                return (iw, 0);
            }
            return (if l.next_rect_set == 2 { l.next_rw } else { self.dp(l.next_rw) }, 1);
        }
        let b = l.boxes.last().copied().unwrap_or_default();
        // a grid: the width of the cell the next child gets (grid.rs)
        if b.grid != 0 {
            return self.grid_avail_width(b.grid as usize, iw);
        }
        let w = l.next_w;
        if b.flags & BOX_V != 0 {
            // vertical box: the width is its cross axis
            if w > 0 {
                return (self.dp(w), 1);
            }
            if w < 0 || b.flags & (BOX_XCENTER | BOX_XSTART | BOX_XEND) == 0 {
                return (b.len[0], 1);
            }
            // natural width, at most the box's
            if b.len[0] <= 0 {
                return (iw, 0);
            }
            return (b.len[0].min(iw), 0);
        }
        if w > 0 {
            return (self.dp(w), 1);
        }
        let wrap = wraps(b.flags);
        let spare = b.len[0].wrapping_sub(b.fixedp).max(0);
        if w < 0 && !(wrap && (b.wsump == 0 || spare == 0)) {
            // fill weight: its share of what the fixed children left (previous pass)
            if b.wsump == 0 {
                return (0, 1);
            }
            return (spare.wrapping_mul(w.wrapping_neg()) / b.wsump, 1);
        }
        if wrap {
            // a wrapping box (natural, or fill where it wraps): at most the box's width (a
            // child that does not fit after the others of its row starts the next one)
            if b.len[0] <= 0 {
                return (iw, 0);
            }
            return (b.len[0].min(iw), 0);
        }
        // natural: at most the space left on the row
        let mut rest = b.pos[0].wrapping_add(b.len[0]).wrapping_sub(b.cur);
        if b.n != 0 {
            rest = rest.wrapping_sub(b.gap);
        }
        if rest <= 0 {
            return (iw, 0);
        }
        (rest.min(iw), 0)
    }

    /// ui_box_begin(id, flags, gap): open a container (id 0 = automatic)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn box_begin(&mut self, id: u32, flags: u32, gap: i32) {
        let _src = self.src_scope();
        let id = if id != 0 {
            id
        } else {
            // automatic id from the parent id and the index among its automatic-id boxes
            // (the child count would miss absolutely placed boxes)
            let t = self.top();
            let k = t.id.wrapping_mul(31).wrapping_add(t.nbox);
            t.nbox = t.nbox.wrapping_add(1);
            k.wrapping_add(0x2545_F491).wrapping_mul(0x9E37_79B1) | 0x8000_0000
        };
        let s = *self.slot(id);
        let pad = self.dp_space(((flags >> PAD_SHIFT) & 255) as i32); // (theme) spacing
        let gap = self.dp_space(gap);
        // natural size from the previous pass (a wrapping box: one row, but at most as wide as
        // a wrapping text would be there)
        let pgrid = self.wp.lay.boxes.last().map_or(0, |b| b.grid);
        let mw = if wraps(flags) { Some(self.avail_width(s.measw).0).filter(|&w| w > 0).unwrap_or(s.measw) } else { s.measw };
        self.wp.lay.placing_box = true;
        let r = self.place(mw, s.meash);
        self.wp.lay.placing_box = false;
        let gcell = if pgrid != 0 { self.wp.lay.grid.last } else { (0, [0; 4]) };
        let auto = self.wp.lay.place_auto;
        let pmain = self.wp.lay.place_main;
        // decorations
        if flags & (BOX_BG | BOX_CARD | BOX_LAYER) != 0 {
            let st = self.style;
            let mut sp = Sp { flags, ..Sp::default() };
            sp.place(r);
            st.box_deco(self, &sp);
        }
        if flags & BOX_BLOCK != 0 {
            self.hit(id, HB_BLOCK);
        }
        if flags & BOX_DISABLED != 0 {
            self.disabled_push();
        }
        if flags & BOX_CLIP != 0 {
            self.clip_push(r[0], r[1], r[2], r[3]);
        }
        // push the frame
        if self.wp.lay.boxes.len() >= BOX_STACK_MAX {
            self.warn("boxes nested too deeply");
            return;
        }
        let a = (flags & BOX_V) as usize;
        let mut b = BoxFrame {
            id,
            flags,
            pos: [r[0].wrapping_add(pad), r[1].wrapping_add(pad)],
            len: [r[2].wrapping_sub(pad).wrapping_sub(pad).max(0), r[3].wrapping_sub(pad).wrapping_sub(pad).max(0)],
            gap,
            pad,
            auto,
            pmain,
            fixedp: s.fixed,
            wsump: s.wsum,
            gcell,
            ..BoxFrame::default()
        };
        // cursor at the start of the main axis, shifted for centre / end alignment (a wrapping
        // box: its first row, with the previous pass's rows)
        b.cur = b.pos[a];
        if wraps(flags) {
            let prev = self.wp.lay.wrap_rows.get(&id).cloned().unwrap_or_default();
            b.cur = b.cur.wrapping_add(row_start(flags, b.len[a], prev.first()));
            self.wp.lay.wraps.push(Wrap { rows: alloc::vec![Row::default()], prev, ..Wrap::default() });
        } else if flags & (BOX_CENTER | BOX_END) != 0 && b.wsump == 0 {
            let spare = b.len[a].wrapping_sub(b.fixedp);
            if spare > 0 {
                b.cur = b.cur.wrapping_add(if flags & BOX_CENTER != 0 { spare >> 1 } else { spare });
            }
        }
        self.wp.lay.boxes.push(b);
        if self.m.ins.on {
            self.ins_box(id, r); // the inspector's tree and outlines (inspect.rs)
        }
    }

    /// ui_box_end: store the measured natural size, correct the parent's totals
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn box_end(&mut self) {
        let _src = self.src_scope();
        if self.wp.lay.boxes.len() <= 1 {
            return; // never pop the root here
        }
        let mut b = *self.top();
        // a wrapping box: as high as its rows and the gaps between them; changed rows align
        // their children again in another pass
        if wraps(b.flags) {
            if let Some(w) = self.wp.lay.wraps.pop() {
                b.cross = w.top.wrapping_add(w.rows.last().map_or(0, |r| r.h));
                if self.wp.lay.wrap_rows.get(&b.id) != Some(&w.rows) {
                    self.repass = true;
                    self.wp.lay.wrap_rows.insert(b.id, w.rows);
                }
            }
        }
        // measured natural size (content + padding)
        let p2 = b.pad.wrapping_add(b.pad);
        let (mut w, mut h) = (b.fixed.wrapping_add(p2), b.cross.wrapping_add(p2));
        if b.flags & BOX_V != 0 {
            core::mem::swap(&mut w, &mut h);
        }
        let s = *self.slot(b.id);
        let size_changed = b.flags & BOX_FIXED == 0 && (w != s.measw || h != s.meash);
        if size_changed || b.fixed != s.fixed || b.wsum != s.wsum {
            self.repass = true;
            if self.ui_debug {
                let m = alloc::format!(
                    "layout f{}: box {:x} {}x{} -> {}x{} fixed {}->{} wsum {}->{}\n",
                    self.frame as i32, b.id, s.measw, s.meash, w, h, s.fixed, b.fixed, s.wsum, b.wsum
                );
                plat::write_stderr(m.as_bytes());
            }
        }
        {
            let s = self.slot(b.id);
            s.measw = w;
            s.meash = h;
            s.fixed = b.fixed;
            s.wsum = b.wsum;
        }
        // late correction: the parent accounted for our previous natural size (a grid parent
        // takes this pass's size for the cell)
        let n = self.wp.lay.boxes.len();
        if n >= 2 && self.wp.lay.boxes[n - 2].grid != 0 && b.gcell.0 != 0 {
            let k = self.wp.lay.boxes[n - 2].grid as usize;
            self.grid_child_measured(k, b.gcell.1, w, h);
        }
        if n >= 2 {
            let l = &mut self.wp.lay;
            let p = &mut l.boxes[n - 2];
            let (mut along, mut across) = (w, h);
            if p.flags & BOX_V != 0 {
                core::mem::swap(&mut along, &mut across);
            }
            // a wrapping parent: the same for the row we are on
            let mut row = if wraps(p.flags) { l.wraps.last_mut().and_then(|w| w.rows.last_mut()) } else { None };
            if b.auto & 1 != 0 {
                p.fixed = p.fixed.wrapping_add(along.wrapping_sub(b.pmain));
                if let Some(r) = row.as_deref_mut() {
                    r.w = r.w.wrapping_add(along.wrapping_sub(b.pmain));
                }
            }
            // natural cross size: aligned (2) and stretched / filling (4) child boxes count
            // here with this pass's measurement (not at placement with the previous one), so
            // a changed subtree settles in one pass instead of one pass per nesting level
            if b.auto & 6 != 0 {
                if across > p.cross {
                    p.cross = across;
                }
                if let Some(r) = row.filter(|r| across > r.h) {
                    r.h = across;
                }
            }
        }
        self.wp.lay.boxes.pop();
        if self.m.ins.on {
            self.ins_box_end();
        }
        if b.flags & BOX_CLIP != 0 {
            self.clip_pop();
        }
        if b.flags & BOX_DISABLED != 0 {
            self.disabled_pop();
        }
    }

    /// ui_spacer(dip): empty space along the main axis (< 0 = fill weight)
    pub fn spacer(&mut self, dip: i32) {
        if self.top().flags & BOX_V != 0 {
            self.wp.lay.next_h = dip;
            self.wp.lay.next_w = 1;
        } else {
            self.wp.lay.next_w = dip;
            self.wp.lay.next_h = 1;
        }
        self.place(0, 0);
    }

    /// ui_dp(dip): DIP -> physical pixels, round(dip * dpi / 96) (signed)
    pub fn dp(&self, dip: i32) -> i32 {
        dp_at(dip, self.wp.rt.dpi)
    }

    /// ui_px2dip(px): physical pixels -> DIP, (px * 96 + dpi / 2) / dpi
    pub fn px2dip(&self, px: i32) -> i32 {
        let dpi = self.wp.rt.dpi;
        px.wrapping_mul(96).wrapping_add(((dpi as u32) >> 1) as i32) / dpi
    }

    /// ui_dpi: current dpi of the live window (96 = 100 %)
    pub fn dpi(&self) -> i32 {
        self.wp.rt.dpi
    }

    /// ui_set_dpi(dpi): override the dpi (tests)
    pub fn set_dpi(&mut self, dpi: i32) {
        self.wp.rt.dpi = dpi;
    }

    /// dp_fx(dip_fx) -> px in 24.8 (dip_fx * dpi / 96, truncated)
    pub fn dp_fx(&self, dip_fx: i32) -> i32 {
        dip_fx.wrapping_mul(self.wp.rt.dpi) / 96
    }
}

/// dip2px_at / dp_at: round(dip * dpi / 96) (signed)
pub fn dp_at(dip: i32, dpi: i32) -> i32 {
    let v = dip.wrapping_mul(dpi);
    if v >= 0 { (v + 48) / 96 } else { (v - 48) / 96 }
}

/// px2dip_at: round(px * 96 / dpi), dpi 0 = 96 (ui_win.s)
pub fn px2dip_at(px: i32, dpi: i32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    let v = px.wrapping_mul(96);
    let half = ((dpi as u32) >> 1) as i32;
    if v >= 0 { v.wrapping_add(half) / dpi } else { v.wrapping_sub(half) / dpi }
}
