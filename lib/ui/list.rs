// list -- scroll views, scroll bars, lists, tabs and expanders (WinUI 3 style)
//                                                                  [src/ui/ui_list.s]
//
//   scroll_begin(id, flags)                clipped viewport that takes its size like a box
//        (next_size / parent) and lays its children out in a content area that may be
//        larger; flags = SC_V (default) | SC_H | SC_NOBAR | SC_NOWHEEL | BOX_BG | BOX_CARD |
//        PAD_n ...  Mouse wheel (48 DIP per notch, smooth), overlay scroll bars (2 DIP, 6 DIP
//        while pointed at, draggable thumb, click on the track pages), Tab focus is scrolled
//        into view.
//   scroll_end()
//   scroll_to(id, x, y)                    jump to a position in DIP (-1 keeps that axis)
//   scroll_pos(id) -> (x, y)               current position in DIP
//   scroll_extent(id) -> (w, h)            content size in DIP (last measure; 0 before) (U3)
//   scrollbar(id, &mut pos, content, view, vertical) -> EV
//        standalone scroll bar (natural thickness 12 DIP) for custom canvases; pos runs from
//        0 to content - view in the caller's unit; EV_ACT when pos changed
//   list_begin(id, &mut sel, count, item_h | LF_*) -> (first, end)
//        virtualized list (own scroll view, fixed row height in DIP, 0 = 40); emit the rows
//        first .. end-1; row i has the widget id `id + 1 + i`.  Up/Down/Home/End/PgUp/PgDn
//        move the selection (sel, -1 = none) and scroll it into view; Enter: the selected
//        row reports EV_DOUBLE (list_end too), Space: the first check box / switch of the
//        selected row acts; the menu key: the selected row's right click.  The list is one Tab
//        stop; the widgets inside custom rows are Tab stops in the selected row only.  Focus
//        alone selects nothing: a focused list without a selection has the focus visual
//        around it.
//        LF_REORDER (U1): a row dragged more than 4 DIP is moved: an accent insertion line
//        follows the pointer, the view scrolls near its edges, Esc cancels; on release
//        list_end returns EV_DRAG with from | to << 16 (the app moves item `from` so that it
//        ends at index `to`) and sel = to.  Widgets inside custom rows keep their clicks.
//        The keyboard moves the selected row the same way: Ctrl+Up / Ctrl+Down one place,
//        Ctrl+Home / Ctrl+End to the top / bottom.
//   list_item(i, label) -> EV              text row (EV_ACT clicked, EV_DOUBLE, EV_RCLICK)
//   list_item_begin(i) -> EV               custom row: opens a BOX_H inside the row
//   list_item_end()
//   list_end(&mut sel) -> (EV, sel)        EV_ACT if sel changed this pass, EV_DOUBLE;
//                                          EV_DRAG: a row was dropped (from | to << 16)
//        (Rust: list_begin reads the selection into the list, list_end writes it back --
//        the asm keeps a pointer to the app's variable in between; list_sel() reads it in
//        the rows)
//   tabs(id, &mut sel, names) -> EV        text tab header row, tab i has the id
//                                          `id + 1 + i`; EV_ACT when sel changed; placed
//                                          narrower than the tabs, the widest ones give
//                                          way (labels cut with "…")
//   expander_begin(id, header, &mut open) -> open   (emit the content when 1)
//   expander_end()
//
// Widget ids used internally: (id << 3 | k) | 0x80000000 for sub parts (k = 1..7),
// (id + 1 + i) | 0xC0000000 for the content box of custom list rows.

use alloc::vec::Vec;

use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};

// nesting stacks (fixed: scroll views, lists and expanders inside each other; deeper ones
// share the last record, limit_hit warns)
const SCROLL_MAX: usize = 8;
pub(crate) const SCR_F_LIST: u32 = 0x4000_0000; // internal: the list emits the semantic record itself
const LIST_MAX: usize = 4;
const EXP_MAX: usize = 16;
/// tab widths on the stack; more tabs (any number) use the heap
const TABS_INLINE: usize = 32;
const WHEEL_DIP: i32 = 48;

// tab widths: on the stack up to TABS_INLINE tabs (no allocation), on the heap beyond
#[derive(Default)]
struct Tw {
    inline: [i32; TABS_INLINE],
    heap: Vec<i32>,
}

impl Tw {
    // get(n): room for the widths of n tabs
    fn get(&mut self, n: usize) -> &mut [i32] {
        if n <= TABS_INLINE {
            &mut self.inline[..n]
        } else {
            self.heap.resize(n, 0);
            &mut self.heap[..]
        }
    }
}

// tabs_fit(widths, room, min): the widest tabs narrowed to one width so that the row fits
// room (none below min)
fn tabs_fit(ws: &mut [i32], room: i32, min: i32) {
    let mut sorted = ws.to_vec();
    sorted.sort_unstable();
    let (mut rest, mut cap) = (room, i32::MAX);
    for (k, &w) in sorted.iter().enumerate() {
        let left = (sorted.len() - k) as i32;
        if w.saturating_mul(left) > rest {
            cap = rest / left;
            break;
        }
        rest -= w;
    }
    let cap = cap.max(min);
    for w in ws.iter_mut() {
        *w = (*w).min(cap);
    }
}

// scroll view slot fields (the box slot of the view; the animation fields are free):
// SLS_FOCUS = afrom | ato << 16 (focus id seen at the last scroll_end), SLS_TX = at0 / SLS_TY
// = adur (target position px), SLS_T = aux (ms of the last smooth scrolling step).
// Scroll bar slot: SB_MODE = scrollx (0 idle, 1 dragging the thumb, 2 paging), SB_GRAB = aux.
// Tabs slot (selection pill animation): TB_CUR = scrollx, TB_TGT = scrolly, TB_T = aux,
// TB_INIT = at0.

/// a scroll view on the stack  [SCR_*]
#[derive(Clone, Copy, Default)]
pub(crate) struct Scr {
    id: u32,
    flags: u32,
    /// viewport rectangle (px)
    rect: [i32; 4],
    /// index of the viewport's hit record (current list)
    hit0: usize,
    /// first semantic record of the content
    sem0: usize,
    layer: u32,
    /// events of the viewport hit box (lists)
    ev: u32,
}

/// a list on the stack  [LS_*]
#[derive(Clone, Copy, Default)]
pub(crate) struct Ls {
    id: u32,
    count: i32,
    /// the selection (the asm keeps a pointer to the app's variable)
    sel: i32,
    /// row height px
    ih: i32,
    /// next row index expected
    next: i32,
    /// sel at list_begin
    old: i32,
    ev: u32,
    /// events of the viewport (focus)
    vev: u32,
    /// y of row 0 (px, scrolled)  (U1)
    y0: i32,
    /// LF_* flags
    flags: u32,
    /// from | to << 16 of a drop in this pass, -1 = none
    drop: i32,
    /// keyboard: Enter (the selected row reports EV_DOUBLE), Space (its check box acts)
    kdbl: bool,
    ktog: bool,
    /// the row being emitted is no Tab stop host (kbnav.rs: not the selected row)
    notab: bool,
}

/// state shared by all windows (none: ui_list.s keeps everything per window)
#[derive(Default)]
pub(crate) struct S {}

/// per-window state (ui_list_wb0 .. ui_list_wb1, swapped by win.rs): scroll view, list and
/// expander stacks, the dragged row
#[derive(Default)]
pub(crate) struct W {
    /// SCROLL_MAX entries + the dummy the asm writes when the stack overflows
    scr: [Scr; SCROLL_MAX + 1],
    list: [Ls; LIST_MAX + 1],
    exp: [u32; EXP_MAX],
    scr_sp: u32,
    list_sp: u32,
    exp_sp: u32,
    /// (U1) list whose row is dragged (0 = none)
    ldrag_list: u32,
    ldrag_from: i32,
    /// insertion slot 0 .. count
    ldrag_to: i32,
    /// autoscroll clock
    ldrag_t: u32,
}

// clamp_mem(v, max) -> (v moved into 0 .. max, it had to be moved)
fn clamp_mem(v: i32, max: i32) -> (i32, bool) {
    let mut x = if v < 0 { 0 } else { v };
    if x > max {
        x = max;
    }
    (x, x != v)
}

// anim_decay(cur, target, steps) -> cur moved towards target (the remaining distance shrinks
// to 150/256 per step; the last pixel snaps)
pub(crate) fn anim_decay(cur: i32, target: i32, steps: u32) -> i32 {
    if steps == 0 {
        return cur;
    }
    let d = target.wrapping_sub(cur);
    let sign = d >> 31;
    let mut rem = ((d ^ sign).wrapping_sub(sign)) as u32 as u64; // |remaining|
    for _ in 0..steps {
        rem = rem * 150 >> 8;
    }
    let mut r = rem as u32;
    if r <= 1 {
        r = 0;
    }
    let r = ((r as i32) ^ sign).wrapping_sub(sign);
    target.wrapping_sub(r)
}

// thumb_pos(pos, free, range) -> free * pos / range (0 when range or free <= 0)
fn thumb_pos(pos: i32, free: i32, range: i32) -> i32 {
    if range <= 0 || free <= 0 {
        return 0;
    }
    ((pos as u32 as u64 * free as u32 as u64) / range as u32 as u64) as u32 as i32
}

// into_view(pos, size, vpos, vsize) -> scroll delta that brings pos .. pos+size into
// vpos .. vpos+vsize (the start wins when it does not fit)
pub(crate) fn into_view(pos: i32, size: i32, vpos: i32, vsize: i32) -> i32 {
    let start = pos.wrapping_sub(vpos);
    if start < 0 {
        return start;
    }
    let end = pos.wrapping_add(size).wrapping_sub(vpos).wrapping_sub(vsize);
    if end <= 0 {
        return 0;
    }
    if end <= start { end } else { start }
}

// scr_axes(flags) -> bit 0 vertical, bit 1 horizontal scrolling
fn scr_axes(flags: u32) -> u32 {
    if flags & SC_H != 0 {
        if flags & SC_V != 0 { 3 } else { 2 }
    } else {
        1
    }
}

// anim_steps(&t0) -> number of 16 ms steps since t0 (0..24); t0 = now if > 0
pub(crate) fn anim_steps(now: u32, t0: &mut i32) -> u32 {
    let d = (now as i32).wrapping_sub(*t0);
    if d <= 0 {
        return 0;
    }
    *t0 = now as i32;
    let mut n = (d as u32).wrapping_add(8) >> 4;
    if n == 0 {
        n = 1;
    }
    n.min(24)
}

impl Ui {
    // ------------------------------------------------------------------ scroll views

    // scr_top: the innermost scroll entry index (the dummy when the stack overflowed)
    pub(crate) fn scr_top(&self) -> usize {
        let i = self.wp.list.scr_sp.wrapping_sub(1) as usize;
        if i < SCROLL_MAX { i } else { SCROLL_MAX }
    }

    // scr_max(entry) -> (max x, max y) px from the last measured content
    fn scr_max(&mut self, e: &Scr) -> (i32, i32) {
        let s = *self.slot(e.id);
        let mut my = 0;
        let mut mx = 0;
        if e.flags & SC_H == 0 || e.flags & SC_V != 0 {
            my = s.meash.wrapping_sub(e.rect[3]).max(0);
        }
        if e.flags & SC_H != 0 {
            mx = s.measw.wrapping_sub(e.rect[2]).max(0);
        }
        (mx, my)
    }

    // scr_open(id, flags, hbflags) -> EV of the viewport hit box: places the viewport (a
    // clipped box), pushes the scroll entry, registers the viewport hit box (behave when
    // hbflags has HB_FOCUS, else a plain hit box)
    pub(crate) fn scr_open(&mut self, id: u32, flags: u32, hbflags: u32) -> u32 {
        let axes = scr_axes(flags);
        let mut bf = (flags & 0x00FF_FFFF & !BOX_V) | BOX_CLIP;
        if axes != 2 {
            bf |= BOX_V; // vertical (or both): children top to bottom
        }
        self.box_begin(id, bf, 0);
        // stack entry
        let i = (self.wp.list.scr_sp as usize).min(SCROLL_MAX);
        if i == SCROLL_MAX {
            self.limit_hit(super::ctx::LIM_SCROLLS);
        }
        self.wp.list.scr_sp = self.wp.list.scr_sp.wrapping_add(1);
        let c = &self.wp.ctx;
        let mut e = Scr {
            id,
            flags,
            rect: c.last_rect,
            layer: c.cmd_layer_ui,
            sem0: self.sems.len(),
            hit0: c.hits[c.hit_cur].len(),
            ev: 0,
        };
        self.slot(id);
        // viewport hit box (the content's hit boxes follow it)
        if hbflags & HB_FOCUS != 0 {
            e.ev = self.behave(id, hbflags);
        } else {
            self.hit(id, hbflags);
        }
        self.wp.list.scr[i] = e;
        e.ev
    }

    // scr_settle(entry) -> the position had to be clamped: clamps target and position to the
    // content and advances smooth scrolling
    fn scr_settle(&mut self, e: &Scr) -> bool {
        let (mx, my) = self.scr_max(e);
        let now = self.now_ms;
        let anim_off = self.anim_off;
        let s = self.slot(e.id);
        // never measured (first pass): keep a position set by scroll_to
        if s.measw | s.meash == 0 {
            return false;
        }
        s.at0 = clamp_mem(s.at0 as i32, mx).0 as u32;
        s.adur = clamp_mem(s.adur as i32, my).0 as u32;
        let (tx, ty) = (s.at0 as i32, s.adur as i32);
        let mut animating = false;
        if s.scrollx != tx || s.scrolly != ty {
            if anim_off {
                s.scrollx = tx;
                s.scrolly = ty;
            } else {
                let steps = anim_steps(now, &mut s.aux);
                s.scrollx = anim_decay(s.scrollx, tx, steps);
                s.scrolly = anim_decay(s.scrolly, ty, steps);
                if s.scrollx != tx || s.scrolly != ty {
                    animating = true;
                }
            }
        }
        let (x, cx) = clamp_mem(s.scrollx, mx);
        let (y, cy) = clamp_mem(s.scrolly, my);
        s.scrollx = x;
        s.scrolly = y;
        if animating {
            self.animating = true;
        }
        cx || cy
    }

    // scr_offset: settle the innermost view and shift its content by the position
    pub(crate) fn scr_offset(&mut self) {
        let e = self.wp.list.scr[self.scr_top()];
        self.scr_settle(&e);
        let s = *self.slot(e.id);
        self.box_offset(s.scrollx, s.scrolly);
    }

    // scr_follow(entry): keyboard focus moved onto a widget of this view: scroll it into view
    fn scr_follow(&mut self, e: &Scr) {
        let f = self.wp.ctx.focus;
        let s = self.slot(e.id);
        if f == (s.afrom as u32 | (s.ato as u32) << 16) {
            return;
        }
        s.afrom = f as u16;
        s.ato = (f >> 16) as u16;
        if f == 0 || !self.wp.ctx.focus_visible || f == e.id {
            return;
        }
        // the focused widget's semantic record among ours (last match)
        let Some(r) = self.sems.get(e.sem0..).and_then(|v| v.iter().rev().find(|r| r.id == f)).map(|r| self.phys_rect(r.rect)) else {
            return;
        };
        let axes = scr_axes(e.flags);
        let m = self.dp(8); // margin: keep the focus visual inside
        if axes & 1 != 0 {
            let d = into_view(r[1].wrapping_sub(m), r[3].wrapping_add(m * 2), e.rect[1], e.rect[3]);
            if d != 0 {
                let s = self.slot(e.id);
                s.scrolly = s.scrolly.wrapping_add(d);
                s.adur = s.scrolly as u32;
                self.repass();
            }
        }
        if axes & 2 != 0 {
            let d = into_view(r[0].wrapping_sub(m), r[2].wrapping_add(m * 2), e.rect[0], e.rect[2]);
            if d != 0 {
                let s = self.slot(e.id);
                s.scrollx = s.scrollx.wrapping_add(d);
                s.at0 = s.scrollx as u32;
                self.repass();
            }
        }
    }

    // scr_bars(entry): overlay scroll bars for the axes whose content overflows
    fn scr_bars(&mut self, e: &Scr) {
        let st = self.style;
        let t = st.scrollbar_thickness(self); // bar thickness (hover area)
        let axes = scr_axes(e.flags);
        let s = *self.slot(e.id);
        let vneed = axes & 1 != 0 && s.meash > e.rect[3];
        let hneed = axes & 2 != 0 && s.measw > e.rect[2];
        let r = e.rect;
        if vneed {
            let h = if hneed { r[3].wrapping_sub(t) } else { r[3] };
            self.wp.ctx.last_rect = [r[0].wrapping_add(r[2]).wrapping_sub(t), r[1], t, h];
            let id = e.id.wrapping_mul(8).wrapping_add(1) | 0x8000_0000;
            let (p, _) = self.sbar_core(id, s.scrolly, s.meash, r[3], true);
            if p != s.scrolly {
                let sl = self.slot(e.id);
                sl.scrolly = p;
                sl.adur = p as u32;
                self.repass();
            }
        }
        if hneed {
            let s = *self.slot(e.id);
            let w = if vneed { r[2].wrapping_sub(t) } else { r[2] };
            self.wp.ctx.last_rect = [r[0], r[1].wrapping_add(r[3]).wrapping_sub(t), w, t];
            let id = e.id.wrapping_mul(8).wrapping_add(2) | 0x8000_0000;
            let (p, _) = self.sbar_core(id, s.scrollx, s.measw, r[2], false);
            if p != s.scrollx {
                let sl = self.slot(e.id);
                sl.scrollx = p;
                sl.at0 = p as u32;
                self.repass();
            }
        }
    }

    // scr_wheel(entry): the wheel scrolls the view when the pointer is over its content
    fn scr_wheel(&mut self, e: &Scr) {
        if e.flags & SC_NOWHEEL != 0 {
            return;
        }
        let c = &self.wp.ctx;
        if c.wheel_y | c.wheel_x == 0 {
            return;
        }
        // hovered: the hot widget is one of our hit boxes on our layer
        let hot = c.hot;
        if hot == 0 {
            return;
        }
        let list = &c.hits[c.hit_cur];
        let found = list.get(e.hit0..).is_some_and(|v| v.iter().any(|h| h.id == hot && h.layer as u32 == e.layer));
        if !found {
            return;
        }
        let step = self.dp(WHEEL_DIP);
        let c = &self.wp.ctx;
        let mut dy = c.wheel_y.wrapping_neg().wrapping_mul(step) / 120; // wheel up = towards the start
        let mut dx = c.wheel_x.wrapping_mul(step) / 120;
        let axes = scr_axes(e.flags);
        // the vertical wheel scrolls sideways in horizontal views or with Shift
        if axes & 1 == 0 || (axes & 2 != 0 && self.mods & MOD_SHIFT != 0) {
            dx = dx.wrapping_add(dy);
            dy = 0;
        }
        if axes & 1 == 0 {
            dy = 0;
        }
        if axes & 2 == 0 {
            dx = 0;
        }
        let (mx, my) = self.scr_max(e);
        let now = self.now_ms;
        let anim_off = self.anim_off;
        let s = self.slot(e.id);
        let ny = clamp_mem((s.adur as i32).wrapping_add(dy), my).0;
        let nx = clamp_mem((s.at0 as i32).wrapping_add(dx), mx).0;
        if ny == s.adur as i32 && nx == s.at0 as i32 {
            return; // at the limit: leave the wheel to outer views
        }
        // starting from rest: the smooth scrolling clock starts now
        if s.scrolly == s.adur as i32 && s.scrollx == s.at0 as i32 {
            s.aux = now as i32;
        }
        s.adur = ny as u32;
        s.at0 = nx as u32;
        if anim_off {
            s.scrolly = ny;
            s.scrollx = nx;
        }
        self.wp.ctx.wheel_y = 0;
        self.wp.ctx.wheel_x = 0;
        if anim_off {
            self.repass();
        } else {
            self.animating = true;
        }
    }

    // scr_close: end the innermost scroll view: measure, clamp, bars, wheel
    pub(crate) fn scr_close(&mut self) {
        if self.wp.list.scr_sp == 0 {
            return;
        }
        let e = self.wp.list.scr[self.scr_top()];
        self.box_end();
        self.scr_follow(&e);
        if self.scr_settle(&e) {
            self.repass(); // the content shrank under the position
        }
        if e.flags & SC_NOBAR == 0 {
            self.scr_bars(&e);
        }
        self.scr_wheel(&e);
        self.wp.ctx.last_rect = e.rect;
        if e.flags & SCR_F_LIST == 0 {
            // semantic record: value = position in DIP (vertical, or horizontal for SC_H views)
            self.wp.ctx.last_state = 0;
            let s = *self.slot(e.id);
            let p = if scr_axes(e.flags) & 1 != 0 { s.scrolly } else { s.scrollx };
            let v = self.px2dip(p);
            self.sem(K_BOX, e.id, None, v);
        }
        self.wp.list.scr_sp -= 1;
    }

    /// the innermost scroll view: (id, viewport rectangle px) -- data views page by it
    pub(crate) fn scr_view(&self) -> (u32, [i32; 4]) {
        let e = self.wp.list.scr[self.scr_top()];
        (e.id, e.rect)
    }

    /// ui_scroll_begin(id, flags)
    pub fn scroll_begin(&mut self, id: u32, flags: u32) {
        self.scr_open(id, flags & !SCR_F_LIST, 0);
        self.scr_offset();
    }

    /// ui_scroll_end
    pub fn scroll_end(&mut self) {
        self.scr_close();
    }

    /// ui_scroll_to(id, x, y): DIP, -1 keeps that axis; jumps without animation
    pub fn scroll_to(&mut self, id: u32, x: i32, y: i32) {
        let (px, py) = (self.dp(x), self.dp(y));
        let s = self.slot(id);
        if x >= 0 {
            s.scrollx = px;
            s.at0 = px as u32;
        }
        if y >= 0 {
            s.scrolly = py;
            s.adur = py as u32;
        }
        self.repass();
    }

    /// ui_scroll_pos(id) -> (x, y) DIP
    pub fn scroll_pos(&mut self, id: u32) -> (i32, i32) {
        let s = *self.slot(id);
        (self.px2dip(s.scrollx), self.px2dip(s.scrolly))
    }

    /// ui_scroll_extent(id) -> (w, h): measured content size of scroll view `id` (DIP; 0
    /// before its first pass).  With the view size (last_rect after scroll_end) the app can
    /// tell whether the content overflows.  (U3)
    pub fn scroll_extent(&mut self, id: u32) -> (i32, i32) {
        let s = *self.slot(id);
        (self.px2dip(s.measw), self.px2dip(s.meash))
    }

    // ------------------------------------------------------------------ scroll bars

    // sbar_core(id, pos, content, view, vertical) -> (new pos, EV): the bar occupies
    // last_rect; nothing is drawn when the content fits
    fn sbar_core(&mut self, id: u32, pos: i32, content: i32, view: i32, vertical: bool) -> (i32, u32) {
        let rect = self.wp.ctx.last_rect;
        let mut pos = pos;
        let mut ev = 0;
        let range = content.wrapping_sub(view);
        if range <= 0 {
            pos = 0;
        } else if view <= 0 {
            pos = pos.max(0).min(range);
        } else {
            ev = self.behave(id, 0);
            self.slot(id);
            let exp = self.anim(id, if ev & (EV_HOVER | EV_HELD) != 0 { 256 } else { 0 }, 120);
            // track along the axis, margins at both ends
            let (mut start, mut len) = if vertical { (rect[1], rect[3]) } else { (rect[0], rect[2]) };
            let st = self.style;
            let m = st.scrollbar_margin(self);
            start = start.wrapping_add(m);
            len = len.wrapping_sub(m).wrapping_sub(m).max(1);
            // thumb length = track * view / content, at least the style's minimum
            let mut thumb = ((len as u32 as u64 * view as u32 as u64) / content as u32 as u64) as u32 as i32;
            let mn = st.scrollbar_min_thumb(self);
            if thumb < mn {
                thumb = mn;
            }
            if thumb > len {
                thumb = len;
            }
            let free = len - thumb;
            // press: on the thumb starts a drag, on the track pages
            let c = &self.wp.ctx;
            let (active, pressed, held) = (c.active == id, c.pressed & (1 << BTN_LEFT) != 0, ev & EV_HELD != 0);
            let (press, cur) = if vertical { (c.press_y, c.my) } else { (c.press_x, c.mx) };
            if active && pressed {
                let tp = thumb_pos(pos, free, range);
                let p = press.wrapping_sub(start);
                let s = self.slot(id);
                if p < tp {
                    pos = pos.wrapping_sub(view);
                    s.scrollx = 2;
                } else if p >= tp.wrapping_add(thumb) {
                    pos = pos.wrapping_add(view);
                    s.scrollx = 2;
                } else {
                    s.aux = p - tp;
                    s.scrollx = 1;
                }
            } else if held {
                let s = *self.slot(id);
                if s.scrollx == 1 {
                    let t = cur.wrapping_sub(start).wrapping_sub(s.aux).max(0); // new thumb start
                    if free > 0 {
                        let t = t.min(free);
                        // pos = thumb * range / free (rounded)
                        let f = free as u32 as u64;
                        pos = ((t as u32 as u64 * range as u32 as u64 + (f >> 1)) / f) as u32 as i32;
                    }
                }
            } else {
                self.slot(id).scrollx = 0;
            }
            if pos < 0 {
                pos = 0;
            }
            if pos > range {
                pos = range;
            }
            // paint (style)
            let mut sp = Sp { value: exp as i32, flags: vertical as u32, ..Sp::default() };
            sp.place(rect);
            sp.s[0] = start; // track start (px, along the axis)
            sp.s[1] = thumb; // thumb length
            sp.value2 = thumb_pos(pos, free, range); // thumb offset from the track start
            st.scrollbar(self, &sp);
        }
        self.wp.ctx.last_rect = rect;
        (pos, ev)
    }

    /// ui_scrollbar(id, &mut pos, content, view, vertical) -> EV_*
    pub fn scrollbar(&mut self, id: u32, pos: &mut i32, content: i32, view: i32, vertical: bool) -> u32 {
        let st = self.style;
        let t = st.scrollbar_thickness(self);
        if vertical {
            self.place(t, 0);
        } else {
            self.place(0, t);
        }
        let (p, mut ev) = self.sbar_core(id, *pos, content, view, vertical);
        if p != *pos {
            *pos = p;
            ev |= EV_ACT;
            self.repass();
        }
        // semantic record (a range: value = position, max = content - view)
        let mut s = 0;
        if ev & EV_HOVER != 0 {
            s |= SS_HOVER as u32;
        }
        if ev & EV_HELD != 0 {
            s |= SS_PRESSED as u32;
        }
        self.wp.ctx.last_state = s;
        self.sem(K_SLIDER, id, None, *pos).max = content.wrapping_sub(view).max(0);
        ev
    }

    // ------------------------------------------------------------------ lists

    // list_top: the innermost list entry index
    fn list_top(&self) -> usize {
        let i = self.wp.list.list_sp.wrapping_sub(1) as usize;
        if i < LIST_MAX { i } else { LIST_MAX }
    }

    // list_keys(li): selection keys of a focused list (the list's scroll view is on top);
    // without such a key the selection stays as it is, -1 too (the focus alone selects no row)
    fn list_keys(&mut self, li: usize) {
        let e = self.wp.list.list[li];
        let count = e.count;
        if count <= 0 {
            return;
        }
        let cur = e.sel;
        let mut new = None;
        let top = self.wp.list.scr[self.scr_top()];
        // page = rows per view (at least 1)
        let page = (top.rect[3] / e.ih).max(1);
        // Ctrl+arrows on a list with LF_REORDER: the selected row moves (like a drop)
        if e.flags & LF_REORDER != 0 && cur >= 0 && cur < count {
            let mut to = cur;
            if self.kb_chord(VK_UP, MOD_CTRL) {
                to = cur - 1;
            } else if self.kb_chord(VK_DOWN, MOD_CTRL) {
                to = cur + 1;
            } else if self.kb_chord(VK_HOME, MOD_CTRL) {
                to = 0;
            } else if self.kb_chord(VK_END, MOD_CTRL) {
                to = count - 1;
            }
            let to = to.clamp(0, count - 1);
            if to != cur {
                let l = &mut self.wp.list.list[li];
                l.drop = (to << 16) | cur;
                l.sel = to;
                self.wp.ctx.focus_visible = true;
                self.repass();
                let s = self.slot(top.id);
                let d = into_view(to.wrapping_mul(e.ih), e.ih, s.scrolly, top.rect[3]);
                s.scrolly = s.scrolly.wrapping_add(d);
                s.adur = s.scrolly as u32;
                return;
            }
        }
        if cur >= 0 && self.key(VK_RETURN) {
            self.wp.list.list[li].kdbl = true;
            self.repass();
        }
        if cur >= 0 && self.key(VK_SPACE) {
            self.wp.list.list[li].ktog = true;
            self.wp.ctx.focus_visible = true;
            self.repass();
        }
        if self.key(VK_UP) {
            new = Some(if cur > 0 { cur - 1 } else { 0 });
        }
        if self.key(VK_DOWN) {
            new = Some(if cur < 0 { 0 } else { cur.wrapping_add(1) });
        }
        if self.key(VK_HOME) {
            new = Some(0);
        }
        if self.key(VK_END) {
            new = Some(count - 1);
        }
        if self.key(VK_PRIOR) {
            new = Some(cur.wrapping_sub(page));
        }
        if self.key(VK_NEXT) {
            new = Some(cur.wrapping_add(page));
        }
        let Some(mut new) = new else {
            return;
        };
        if new < 0 {
            new = 0;
        }
        if new >= count {
            new = count - 1;
        }
        if new == cur {
            return;
        }
        self.wp.list.list[li].sel = new;
        self.wp.ctx.focus_visible = true;
        self.repass();
        // scroll the row into view (immediately)
        let s = self.slot(top.id);
        let d = into_view(new.wrapping_mul(e.ih), e.ih, s.scrolly, top.rect[3]);
        s.scrolly = s.scrolly.wrapping_add(d);
        s.adur = s.scrolly as u32;
    }

    // list_drag(li): the list whose row is being dragged, before its rows: release -> drop
    // (drop, sel), Esc / lost pointer -> cancel, pointer near the edges -> scroll
    fn list_drag(&mut self, li: usize) {
        let e = self.wp.list.list[li];
        if e.vev & EV_HELD != 0 {
            if self.key(VK_ESCAPE) {
                self.wp.ctx.active = 0;
                self.wp.list.ldrag_list = 0;
                self.repass();
                return;
            }
            // autoscroll: within 24 DIP of the top / bottom edge (or beyond it)
            let top = self.wp.list.scr[self.scr_top()];
            let zone = self.dp(24);
            let my = self.wp.ctx.my;
            let mut depth = zone.wrapping_sub(my.wrapping_sub(top.rect[1])); // > 0: inside the top zone
            if depth <= 0 {
                let b = top.rect[1].wrapping_add(top.rect[3]).wrapping_sub(my);
                depth = zone.wrapping_sub(b);
                if depth <= 0 {
                    self.wp.list.ldrag_t = self.now_ms;
                    return;
                }
                depth = depth.wrapping_neg(); // < 0: bottom zone, scroll down
            }
            // px per 16 ms step: 2 + depth / 4, at most one row
            self.animating = true;
            let now = self.now_ms;
            let mut t0 = self.wp.list.ldrag_t as i32;
            let steps = anim_steps(now, &mut t0);
            self.wp.list.ldrag_t = t0 as u32;
            if steps == 0 {
                return;
            }
            let sign = depth >> 31;
            let mut sp = ((((depth ^ sign).wrapping_sub(sign)) as u32 >> 2) + 2) as i32;
            if sp > e.ih {
                sp = e.ih;
            }
            sp = sp.wrapping_mul(steps as i32);
            if depth <= 0 {
                sp = sp.wrapping_neg();
            }
            let s = self.slot(top.id);
            let mut y = s.scrolly.wrapping_sub(sp);
            if y < 0 {
                y = 0;
            }
            s.scrolly = y;
            s.adur = y as u32;
            self.repass();
        } else if e.vev & EV_COMMIT != 0 {
            // released: from -> to
            let from = self.wp.list.ldrag_from;
            let mut to = self.wp.list.ldrag_to;
            if to > from {
                to -= 1; // below the item: it leaves a gap above
            }
            self.wp.list.ldrag_list = 0;
            if to == from || from as u32 >= e.count as u32 || to as u32 >= e.count as u32 {
                return;
            }
            let l = &mut self.wp.list.list[li];
            l.drop = (to << 16) | from;
            l.sel = to; // the moved item stays selected
            self.repass();
        } else {
            self.wp.list.ldrag_list = 0;
            self.repass();
        }
    }

    // list_drag_slot(li): insertion slot 0 .. count from the pointer (y0 known)
    fn list_drag_slot(&mut self, li: usize) {
        let e = self.wp.list.list[li];
        let half = ((e.ih as u32) >> 1) as i32;
        let p = self.wp.ctx.my.wrapping_sub(e.y0).wrapping_add(half).max(0);
        let mut to = p / e.ih;
        if to > e.count {
            to = e.count;
        }
        self.wp.list.ldrag_to = to;
    }

    // list_drag_line(li): the style's insertion marker at the drop slot (inside the view)
    fn list_drag_line(&mut self, li: usize) {
        let e = self.wp.list.list[li];
        let r = self.box_rect();
        let mut sp = Sp { x: r[0], w: r[2], ..Sp::default() };
        sp.y = self.wp.list.ldrag_to.wrapping_mul(e.ih).wrapping_add(e.y0); // the line's centre
        sp.h = 0;
        let st = self.style;
        st.list_drop_line(self, &sp);
    }

    /// ui_list_begin(id, &mut sel, count, item_h | LF_*) -> (first, end): the rows to emit
    pub fn list_begin(&mut self, id: u32, sel: &mut i32, count: i32, item_h: u32) -> (i32, i32) {
        let flags = item_h & 0xFFFF_0000; // (U1) LF_* flags
        let count = count.max(0);
        let ihd = (item_h & 0xFFFF) as i32;
        let ih = self.dp(if ihd > 0 { ihd } else { 40 }).max(1);
        // list entry
        let li = (self.wp.list.list_sp as usize).min(LIST_MAX);
        if li == LIST_MAX {
            self.limit_hit(super::ctx::LIM_LISTS);
        }
        self.wp.list.list_sp = self.wp.list.list_sp.wrapping_add(1);
        self.wp.list.list[li] = Ls { id, count, sel: *sel, ih, next: 0, old: *sel, ev: 0, vev: 0, y0: 0, flags, drop: -1, kdbl: false, ktog: false, notab: false };
        // viewport: a vertical scroll view that takes the keyboard focus (Space / Enter are the
        // list's; the menu key goes to the selected row)
        if *sel >= 0 && *sel < count {
            self.kb_ctx_redirect(id, id.wrapping_add(1).wrapping_add(*sel as u32));
        }
        let vev = self.scr_open(id, SC_V | SCR_F_LIST, HB_FOCUS | HB_OWNKEYS);
        self.wp.list.list[li].vev = vev;
        if vev & EV_FOCUS != 0 {
            self.list_keys(li);
        }
        if self.wp.list.ldrag_list == id {
            self.list_drag(li); // (U1) a row of this list is dragged
        }
        self.scr_offset();
        // visible rows from the clip rectangle
        let y0 = self.box_rect()[1]; // y of row 0
        self.wp.list.list[li].y0 = y0;
        if self.wp.list.ldrag_list == id {
            self.list_drag_slot(li);
        }
        let k = self.wp.ctx.uiclip;
        let first = (k[1].wrapping_sub(y0).max(0) / ih).min(count);
        let end = ((k[3].wrapping_sub(y0).max(0).wrapping_add(ih).wrapping_sub(1)) / ih).min(count).max(first);
        self.wp.list.list[li].next = first;
        if first != 0 {
            self.place(0, first.wrapping_mul(ih)); // rows above the view
        }
        *sel = self.wp.list.list[li].sel;
        (first, end)
    }

    // li_row(i, label) -> EV: place row i, behaviour, background, pill, text, record
    fn li_row(&mut self, i: i32, label: Option<&str>) -> u32 {
        let li = self.list_top();
        let e = self.wp.list.list[li];
        // rows skipped by the caller keep their space
        let d = i.wrapping_sub(e.next);
        if d > 0 {
            self.place(0, d.wrapping_mul(e.ih));
        }
        self.wp.list.list[li].next = i.wrapping_add(1);
        self.place(0, e.ih);
        let iid = e.id.wrapping_add(i as u32).wrapping_add(1);
        let ev0 = self.behave(iid, 0);
        let mut ev = ev0;
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        // a press on a row gives the list the keyboard focus
        let c = &mut self.wp.ctx;
        if c.active == iid && c.pressed & (1 << BTN_LEFT) != 0 {
            c.focus = e.id;
            c.focus_visible = false;
        }
        // (U1) LF_REORDER: dragging a row starts moving it; the list takes the pointer
        if e.flags & LF_REORDER != 0 && ev & EV_DRAG != 0 && self.wp.list.ldrag_list == 0 && self.wp.ctx.active == iid {
            let l = &mut self.wp.list;
            l.ldrag_list = e.id;
            l.ldrag_from = i;
            l.ldrag_to = i;
            l.ldrag_t = self.now_ms;
            self.wp.ctx.active = e.id;
            self.wp.ctx.focus = e.id;
            ev &= !(EV_ACT | EV_DRAG);
            self.repass();
        }
        if ev & EV_ACT != 0 && self.wp.list.list[li].sel != i {
            self.wp.list.list[li].sel = i;
            self.wp.list.list[li].ev |= EV_ACT;
            self.repass();
        }
        let selected = self.wp.list.list[li].sel == i;
        // Enter / Space on the focused list (list_keys) act on the selected row
        if selected && core::mem::take(&mut self.wp.list.list[li].kdbl) {
            ev |= EV_DOUBLE;
        }
        self.m.kbnav.toggle = selected && core::mem::take(&mut self.wp.list.list[li].ktog);
        if ev & EV_DOUBLE != 0 {
            self.wp.list.list[li].ev |= EV_DOUBLE;
        }
        // paint (style): background, selection pill, focus visual, label
        let mut fl = 0;
        if self.wp.list.ldrag_list == e.id && self.wp.list.ldrag_from == i {
            fl |= 1; // (U1) the dragged row looks pressed
        }
        if e.vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
            fl |= 2;
        }
        let mut sp = Sp { ev, value: selected as i32, value2: disabled as i32, label, flags: fl, ..Sp::default() };
        sp.place(self.wp.ctx.last_rect);
        let st = self.style;
        st.list_row(self, &sp);
        if selected {
            self.wp.ctx.last_state |= SS_SELECTED as u32;
        }
        self.sem(K_LISTITEM, iid, label, i);
        ev0 | (ev & EV_DOUBLE)
    }

    /// ui_list_item(i, label) -> EV_*
    pub fn list_item(&mut self, i: i32, label: &str) -> u32 {
        let ev = self.li_row(i, Some(super::basic::cstr(label)));
        self.m.kbnav.toggle = false; // (no check box in a text row)
        ev
    }

    /// ui_list_item_begin(i) -> EV_*: row i with a BOX_H (gap 8 DIP) for custom content
    pub fn list_item_begin(&mut self, i: i32) -> u32 {
        let ev = self.li_row(i, None);
        let li = self.list_top();
        let id = self.wp.list.list[li].id;
        // the widgets of rows that are not selected are no Tab stops (kbnav.rs)
        let notab = self.wp.list.list[li].sel != i;
        self.wp.list.list[li].notab = notab;
        if notab {
            self.kb_notab(true);
        }
        let cid = id.wrapping_add(i as u32).wrapping_add(1) | 0xC000_0000; // the row's content box
        let x0 = self.dp(16);
        let wm = self.dp(28);
        let r = self.wp.ctx.last_rect;
        let mut w = r[2].wrapping_sub(wm);
        if w <= 0 {
            w = 1;
        }
        self.next_rect_px(r[0].wrapping_add(x0), r[1], w, r[3]);
        // BOX_FIXED: the row has a fixed height, nobody depends on the content box's natural
        // size -- a new row must not cost an extra layout pass
        self.box_begin(cid, BOX_H | BOX_XCENTER | BOX_FIXED, 8);
        ev
    }

    /// the innermost list's selection while its rows are emitted (the asm's rows see the
    /// app's variable change at once; here the list holds it until list_end), -1 outside a
    /// list
    pub fn list_sel(&self) -> i32 {
        if self.wp.list.list_sp == 0 {
            return -1;
        }
        self.wp.list.list[self.list_top()].sel
    }

    /// list_row_focused(id): id is the selected row of the innermost list and that list has
    /// the keyboard focus (the row acts for the menu key)
    pub(crate) fn list_row_focused(&self, id: u32) -> bool {
        if self.wp.list.list_sp == 0 {
            return false;
        }
        let e = &self.wp.list.list[self.list_top()];
        e.sel >= 0 && self.wp.ctx.focus == e.id && id == e.id.wrapping_add(1).wrapping_add(e.sel as u32)
    }

    /// ui_list_item_end
    pub fn list_item_end(&mut self) {
        self.box_end();
        let li = self.list_top();
        if core::mem::take(&mut self.wp.list.list[li].notab) {
            self.kb_notab(false);
        }
        self.m.kbnav.toggle = false;
    }

    /// ui_list_end(&mut sel) -> (EV_* (EV_ACT if sel changed, EV_DOUBLE), sel); EV_DRAG: a
    /// row was dropped, the second value is from | to << 16
    pub fn list_end(&mut self, sel: &mut i32) -> (u32, i32) {
        if self.wp.list.list_sp == 0 {
            return (0, -1);
        }
        let li = self.list_top();
        let e = self.wp.list.list[li];
        // rows below the view
        let d = e.count.wrapping_sub(e.next);
        if d > 0 {
            self.place(0, d.wrapping_mul(e.ih));
        }
        if self.wp.list.ldrag_list == e.id {
            self.list_drag_line(li); // (U1) insertion line while a row is dragged
        }
        self.scr_close();
        let e = self.wp.list.list[li];
        let s = e.sel;
        *sel = s;
        // focus visual on the whole list when nothing is selected
        if e.vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible && s < 0 {
            let r = self.wp.ctx.last_rect;
            let st = self.style;
            st.focus_rect(self, r[0], r[1], r[2], r[3]);
        }
        let mut stt = 0;
        if e.vev & EV_HOVER != 0 {
            stt |= SS_HOVER as u32;
        }
        if e.vev & EV_FOCUS != 0 {
            stt |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_state = stt;
        self.sem(K_LIST, e.id, None, s).max = e.count;
        let mut ev = e.ev;
        if s != e.old {
            ev |= EV_ACT;
        }
        if e.kdbl {
            ev |= EV_DOUBLE; // Enter while the selected row was not emitted
        }
        self.m.kbnav.toggle = false;
        self.wp.list.list_sp -= 1;
        if e.drop != -1 {
            return (ev | EV_DRAG, e.drop); // (U1) a dropped row
        }
        (ev, s)
    }

    // ------------------------------------------------------------------ tabs

    // tab_sizes(flags, names, widths) -> (paint record, tabs, total width): the tabs up to
    // the first empty name, their widths into `widths` (Tw: any number of tabs)
    fn tab_sizes<'a>(&mut self, flags: u32, names: &[&'a str], widths: &mut Tw) -> (Sp<'a>, usize, i32) {
        let st = self.style;
        let mut sp = Sp { flags, ..Sp::default() };
        st.tab_metrics(self, &mut sp); // padding, height, pill width
        let pad = sp.s[0];
        // The style selects its tab face; existing styles retain the body face.
        st.tab_font(self, false);
        let n = names.iter().position(|s| super::basic::cstr(s).is_empty()).unwrap_or(names.len());
        let w = widths.get(n);
        let mut total = 0i32;
        for (k, name) in names[..n].iter().enumerate() {
            w[k] = self.text.width(super::basic::cstr(name)).wrapping_add(pad).wrapping_add(pad);
            total = total.wrapping_add(w[k]);
        }
        (sp, n, total)
    }

    /// Tab strip with horizontal overflow arrows. Mouse scrolling and Left/Right,
    /// Home/End on the strip retain ordinary tab semantics; changed selections
    /// and resized viewports reveal the selected tab. Internal IDs follow the
    /// usual `(id * 8 + part) | 0x80000000` convention (parts 1, 2, 3 and 7).
    pub fn tabs_scrolling(&mut self, id: u32, sel: &mut i32, names: &[&str]) -> u32 {
        let flags = self.take_flags();
        let mut tw = Tw::default();
        let (sp, count, total) = self.tab_sizes(flags, names, &mut tw);
        let widths = tw.get(count);
        let rect = self.place(total, sp.s[1]);
        let base = id.wrapping_mul(8) | 0x8000_0000;
        let state = self.slot(base + 7);
        let changed = state.measw != rect[2] || state.meash != total || state.aux != *sel;
        state.measw = rect[2];
        state.meash = total;
        state.aux = *sel;
        if total <= rect[2] {
            self.next_rect_px(rect[0], rect[1], rect[2], rect[3]);
            self.next_flags(flags);
            return self.tabs(id, sel, names);
        }
        let arrow = self.dp(22).min((rect[2] / 3).max(0));
        let view = (rect[2] - arrow * 2).max(0);
        let sid = base + 1;
        let old = *sel;
        self.next_rect_px(rect[0] + arrow, rect[1], view, rect[3]);
        self.scroll_begin(sid, SC_H | SC_NOBAR | BOX_H);
        let width_dip = ((total as i64 * 96 + self.dpi() as i64 - 1) / self.dpi() as i64) as i32;
        self.next_size(width_dip, self.px2dip(rect[3]));
        self.next_flags(flags);
        let ev = self.tabs(id, sel, names);
        self.scroll_end();
        let (offset, _) = self.scroll_pos(sid);
        let viewport = self.px2dip(view);
        if (changed || old != *sel) && *sel >= 0 && (*sel as usize) < count {
            let left: i32 = widths[..*sel as usize].iter().sum();
            let right = self.px2dip(left + widths[*sel as usize]);
            let left = self.px2dip(left);
            let next = if left < offset { left }
                else if right > offset + viewport { (right - viewport).min(left) }
                else { offset };
            if next != offset { self.scroll_to(sid, next, 0); }
        }
        self.slot(base + 7).aux = *sel;
        let maximum = (width_dip - viewport).max(0);
        for (part, x, icon, label, next, enabled) in [
            (2, rect[0], IC_CHEV_LEFT, crate::i18n::tr("Vorherige Register"), (offset - viewport / 2).max(0), offset > 0),
            (3, rect[0] + rect[2] - arrow, IC_CHEV_RIGHT, crate::i18n::tr("Weitere Register"), (offset + viewport / 2).min(maximum), offset < maximum),
        ] {
            self.next_rect_px(x, rect[1], arrow, rect[3]);
            self.next_icon(icon);
            self.next_flags(flags | ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY | if enabled { 0 } else { ST_DISABLED });
            if self.button(base + part, label) & EV_ACT != 0 { self.scroll_to(sid, next, 0); }
            self.tooltip(label);
        }
        self.wp.ctx.last_rect = rect;
        ev
    }

    /// ui_tabs(id, &mut sel, names) -> EV_* (EV_ACT when sel changed)
    pub fn tabs(&mut self, id: u32, sel: &mut i32, names: &[&str]) -> u32 {
        let old = *sel;
        let st = self.style;
        let flags = self.take_flags();
        let (mut tw, mut tfull) = (Tw::default(), Tw::default());
        let (mut sp, n, total) = self.tab_sizes(flags, names, &mut tw);
        let widths = tw.get(n);
        let (h, pillw) = (sp.s[1], sp.s[2]);
        let count = n as i32;
        self.place(total, h);
        let strip = self.wp.ctx.last_rect;
        // a strip narrower than the tabs: the widest ones give way, their labels cut with "…"
        let full = tfull.get(n);
        full.copy_from_slice(widths);
        if total > strip[2] && n > 0 {
            let pad = sp.s[0];
            st.tab_font(self, false);
            let min = pad * 2 + self.text.width("…");
            tabs_fit(widths, strip[2], min);
        }
        let disabled = if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev = self.behave(id, HB_FOCUS | disabled);
        let state = self.wp.ctx.last_state;
        // keyboard
        if ev & EV_FOCUS != 0 && count != 0 {
            let mut s = *sel;
            if self.key(VK_LEFT) {
                s = s.wrapping_sub(1);
            }
            if self.key(VK_RIGHT) {
                s = s.wrapping_add(1);
            }
            if self.key(VK_HOME) {
                s = 0;
            }
            if self.key(VK_END) {
                s = count - 1;
            }
            if s < 0 {
                s = 0;
            }
            if s >= count {
                s = count - 1;
            }
            if s != *sel {
                *sel = s;
                self.wp.ctx.focus_visible = true;
                self.repass();
            }
        }
        // the tabs
        let mut x = strip[0];
        for i in 0..n {
            let label = super::basic::cstr(names[i]);
            self.wp.ctx.last_rect = [x, strip[1], widths[i], h];
            let tid = id.wrapping_add(i as u32).wrapping_add(1);
            let tev = self.behave(tid, disabled);
            if tev & EV_ACT != 0 {
                self.wp.ctx.focus = id;
                self.wp.ctx.focus_visible = false;
                if *sel != i as i32 {
                    *sel = i as i32;
                    self.repass();
                }
            }
            // paint (style): background and label of tab i
            sp.place(self.wp.ctx.last_rect);
            sp.ev = tev;
            sp.value = (*sel == i as i32) as i32;
            sp.value2 = (self.wp.ctx.last_state == SS_DISABLED as u32) as i32;
            sp.label = Some(label);
            if widths[i] < full[i] {
                st.tab_font(self, *sel == i as i32);
                let short = self.ellipsize(label.as_bytes(), widths[i] - sp.s[0] * 2).0;
                st.tab(self, &Sp { label: Some(&short), ..sp });
            } else {
                st.tab(self, &sp);
            }
            if *sel == i as i32 {
                self.wp.ctx.last_state |= SS_SELECTED as u32;
            }
            self.sem(K_TAB, tid, Some(label), i as i32);
            x = x.wrapping_add(widths[i]);
        }
        // selection pill sliding to the selected tab
        let s = *sel;
        if s >= 0 && (s as u32) < count as u32 {
            let s = s as usize;
            let tx: i32 = widths[..s].iter().fold(0i32, |a, &w| a.wrapping_add(w)); // x of the selected tab (relative)
            let target = tx.wrapping_add(widths[s].wrapping_sub(pillw) >> 1);
            let now = self.now_ms;
            let anim_off = self.anim_off;
            let sl = self.slot(id);
            if sl.at0 == 0 {
                sl.at0 = 1;
                sl.scrollx = target;
                sl.scrolly = target;
            }
            if target != sl.scrolly {
                if sl.scrollx == sl.scrolly {
                    sl.aux = now as i32;
                }
                sl.scrolly = target;
            }
            let mut animating = false;
            if sl.scrollx != target {
                if anim_off {
                    sl.scrollx = target;
                } else {
                    let steps = anim_steps(now, &mut sl.aux);
                    sl.scrollx = anim_decay(sl.scrollx, target, steps);
                    if sl.scrollx != target {
                        animating = true;
                    }
                }
            }
            let px = sl.scrollx;
            if animating {
                self.animating = true;
            }
            // paint (style): the selection pill
            let mut pp = sp;
            pp.place(strip);
            pp.value = px; // pill x (relative to the strip)
            pp.value2 = (state == SS_DISABLED as u32) as i32;
            st.tab_pill(self, &pp);
            // focus visual around the selected tab
            if ev & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
                let fx = strip[0].wrapping_add(tx);
                st.focus_rect(self, fx, strip[1], widths[s], h);
            }
        }
        // record of the whole strip: value = selection
        self.wp.ctx.last_rect = strip;
        self.wp.ctx.last_state = state;
        self.sem(K_BOX, id, None, *sel).max = count;
        let mut r = ev & !EV_ACT;
        if *sel != old {
            r |= EV_ACT;
        }
        r
    }

    // ------------------------------------------------------------------ expander

    /// ui_expander_begin(id, header, &mut open) -> open: a card-like header (48 DIP) with a
    /// chevron; when open the content follows in a padded card below it (BOX_V, PAD_16, gap
    /// 8 DIP).  Always call expander_end.
    pub fn expander_begin(&mut self, id: u32, header: &str, open: &mut i32) -> i32 {
        let header = super::basic::cstr(header);
        // outer column
        self.box_begin(id.wrapping_mul(8).wrapping_add(1) | 0x8000_0000, BOX_V, 0);
        // header
        let st = self.style;
        let mut sp = Sp { label: Some(header), ..Sp::default() };
        let (w, h) = st.expander_size(self, &mut sp);
        self.place(w, h);
        let hdr = self.wp.ctx.last_rect;
        let ev = self.behave(id, HB_FOCUS);
        let state = self.wp.ctx.last_state;
        if ev & EV_ACT != 0 {
            *open = (*open ^ 1) & 1;
            self.repass();
        }
        let o = *open;
        // paint (style): header
        sp.place(hdr);
        sp.ev = ev;
        sp.value = o;
        sp.value2 = (state == SS_DISABLED as u32) as i32;
        st.expander_header(self, &sp);
        // focus visual, record
        self.wp.ctx.last_rect = hdr;
        if ev & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
            st.focus_rect(self, hdr[0], hdr[1], hdr[2], hdr[3]);
        }
        let mut s = state;
        if o != 0 {
            s |= SS_OPEN as u32;
        }
        self.wp.ctx.last_state = s;
        self.sem(K_BUTTON, id, Some(header), o);
        // remember whether a content box was opened
        if self.wp.list.exp_sp as usize >= EXP_MAX {
            self.limit_hit(super::ctx::LIM_EXPANDERS);
        }
        let l = &mut self.wp.list;
        if (l.exp_sp as usize) < EXP_MAX {
            l.exp[l.exp_sp as usize] = o as u32;
        }
        l.exp_sp = l.exp_sp.wrapping_add(1);
        if o != 0 {
            // content card: square top corners, shared edge with the header
            self.box_begin(id.wrapping_mul(8).wrapping_add(2) | 0x8000_0000, BOX_V | PAD_16, 8);
            sp.place(self.wp.ctx.last_rect);
            st.expander_body(self, &sp);
        }
        o
    }

    /// ui_expander_end
    pub fn expander_end(&mut self) {
        let l = &mut self.wp.list;
        if l.exp_sp == 0 {
            return;
        }
        l.exp_sp -= 1;
        let i = l.exp_sp as usize;
        if i < EXP_MAX && l.exp[i] != 0 {
            self.box_end(); // content card
        }
        self.box_end(); // column
    }
}
