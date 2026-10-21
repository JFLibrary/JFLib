// vlist -- lists whose rows have different heights, virtualized: only the rows in view are
// built, laid out and drawn, for any number of rows
//
//   ui.vlist_begin(id, &mut VListState, count) -> (first, end)   the rows to emit, first ..
//        end - 1 (the view takes its place like a box: natural 320 x 240 DIP, next_size(-1, -1)
//        fills)
//   ui.vlist_item_begin(i) -> EV       row i: the style's list row (hover, selection, focus)
//        and a BOX_V of the list's width at the row's place; the box's natural height is the
//        row's height (measured every pass it is built, unless st.set_height gave one).
//        EV_ACT clicked, EV_DOUBLE, EV_RCLICK
//   ui.vlist_item_end()
//   ui.vlist_end(&mut VListState) -> EV   EV_ACT the selection changed (a click, the keys),
//        EV_DOUBLE Enter or a double click, EV_FOCUS, EV_HOVER
//   ui.vlist_sel() -> i32               the selection while the rows are emitted
//   VListState::new() / sel (-1: none) / estimate (DIP of a row not measured yet; 0: 40, then
//        the mean of the measured rows) / set_height(i, dip) (0: measure again) /
//        invalidate(i) / invalidate_all() / insert(at, n) / remove(at, n) (rows above the view:
//        it keeps showing the same rows) / reveal(i) / scroll_to(i) / first_visible() ->
//        (row, px of it above the view) / row_height(i) (px, measured or estimated) /
//        row_known(i) / set_count(n) (vlist_begin does it) / scroll_pos() / set_scroll_pos(px)
//        / total_height() (px; rows not measured count with the estimate)
//
// Scrolling is anchored at the first visible row (its index and how far it is scrolled out):
// rows above it may change their height, come or go without moving what is shown.  The
// heights sit in Fenwick trees (the heights known and how many are known; the others count
// with the estimate), so a position and the row at a position cost O(log n); the scroll bar
// shows the anchor's position in the total.  Keys (focused): Up / Down, PgUp / PgDn (a view of
// rows), Home / End move the selection and scroll it into view (exactly, once the rows on the
// way are measured), Enter: EV_DOUBLE.  The wheel scrolls (smoothly unless animations are
// off), the scroll bar drags.  A change of dpi, width or text scale measures every row again.
// Ids: id the view (focus; K_LIST: value sel, max count), id + 1 + i the rows (K_LISTITEM,
// value i), (id + 1 + i) | 0xC000_0000 their content boxes, part_id(id, 1) the scroll bar.
//
// Rows (crate): the height cache and anchor on their own -- the Markdown viewer (md_view.rs)
// scrolls its blocks with it.

use alloc::vec;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::list::{anim_decay, anim_steps};
use crate::value::part_id;
use crate::{Sp, Ui};

const NAT_W: i32 = 320;
const NAT_H: i32 = 240;
const EST_DIP: i32 = 40;
const WHEEL_DIP: i32 = 48;
const REVEAL_TRIES: u8 = 8;

// ------------------------------------------------------------------ the height cache

/// heights of rows and the scroll anchor (see the module header)
#[derive(Clone, Default)]
pub(crate) struct Rows {
    // per row: > 0 a known height (px), < 0 an estimate of its own (-px), 0 the common estimate
    h: Vec<i32>,
    // Fenwick trees, 1-based: the heights given (|h|), how many are given
    sum: Vec<i64>,
    cnt: Vec<i32>,
    /// px of a row without a height of its own
    pub(crate) est: i32,
    /// the anchor: the first visible row and how many px of it are above the view
    pub(crate) top: usize,
    pub(crate) off: i32,
}

fn val(v: i32) -> (i64, i32) {
    if v == 0 { (0, 0) } else { (v.unsigned_abs() as i64, 1) }
}

impl Rows {
    pub(crate) fn len(&self) -> usize {
        self.h.len()
    }

    fn est(&self) -> i64 {
        self.est.max(1) as i64
    }

    fn rebuild(&mut self) {
        let n = self.h.len();
        self.sum = vec![0; n + 1];
        self.cnt = vec![0; n + 1];
        for i in 1..=n {
            let (s, c) = val(self.h[i - 1]);
            self.sum[i] += s;
            self.cnt[i] += c;
            let j = i + (i & i.wrapping_neg());
            if j <= n {
                self.sum[j] += self.sum[i];
                self.cnt[j] += self.cnt[i];
            }
        }
    }

    fn add(&mut self, i: usize, ds: i64, dc: i32) {
        let mut k = i + 1;
        while k < self.sum.len() {
            self.sum[k] += ds;
            self.cnt[k] += dc;
            k += k & k.wrapping_neg();
        }
    }

    fn put(&mut self, i: usize, v: i32) -> bool {
        let Some(&old) = self.h.get(i) else { return false };
        if old == v {
            return false;
        }
        let (s0, c0) = val(old);
        let (s1, c1) = val(v);
        self.h[i] = v;
        self.add(i, s1 - s0, c1 - c0);
        true
    }

    /// the number of rows (new ones at the end, dropped ones from the end)
    pub(crate) fn resize(&mut self, n: usize) {
        if n == self.h.len() && self.sum.len() == n + 1 {
            return;
        }
        self.h.resize(n, 0);
        self.rebuild();
        if self.top >= n {
            self.top = n.saturating_sub(1);
            self.off = 0;
        }
    }

    /// the height of row i (px): known, estimated or the common estimate
    pub(crate) fn height(&self, i: usize) -> i32 {
        match self.h.get(i) {
            Some(&v) if v != 0 => v.abs(),
            _ => self.est.max(1),
        }
    }

    /// row i's height is known (measured or given)
    pub(crate) fn known(&self, i: usize) -> bool {
        self.h.get(i).is_some_and(|&v| v > 0)
    }

    /// a known height -> it changed
    pub(crate) fn set(&mut self, i: usize, h: i32) -> bool {
        self.put(i, h.max(1))
    }

    /// an estimate of row i's own (kept until its height is known)
    pub(crate) fn guess(&mut self, i: usize, h: i32) {
        if self.h.get(i).is_some_and(|&v| v <= 0) {
            self.put(i, -h.max(1));
        }
    }

    /// row i is measured again
    pub(crate) fn forget(&mut self, i: usize) {
        self.put(i, 0);
    }

    /// every row is measured again
    pub(crate) fn forget_all(&mut self) {
        self.h.iter_mut().for_each(|v| *v = 0);
        self.rebuild();
    }

    /// the mean of the known heights (None: none known)
    pub(crate) fn mean(&self) -> Option<i32> {
        let n = self.h.len();
        let (mut s, mut c, mut k) = (0i64, 0i64, n);
        while k > 0 {
            s += self.sum[k];
            c += self.cnt[k] as i64;
            k &= k - 1;
        }
        (c > 0).then(|| (s / c).clamp(1, i32::MAX as i64) as i32)
    }

    /// the height of rows 0 .. i
    pub(crate) fn prefix(&self, i: usize) -> i64 {
        let i = i.min(self.h.len());
        let (mut s, mut c, mut k) = (0i64, 0i64, i);
        while k > 0 {
            s += self.sum[k];
            c += self.cnt[k] as i64;
            k &= k - 1;
        }
        s + (i as i64 - c) * self.est()
    }

    /// the height of every row
    pub(crate) fn total(&self) -> i64 {
        self.prefix(self.h.len())
    }

    /// the row at y (px from the first row's top, clamped) and y's offset into it
    pub(crate) fn find(&self, y: i64) -> (usize, i32) {
        let n = self.h.len();
        if n == 0 {
            return (0, 0);
        }
        let est = self.est();
        let (mut pos, mut rem) = (0usize, y.max(0));
        let mut step = 1usize << (usize::BITS - 1 - n.leading_zeros());
        while step > 0 {
            let k = pos + step;
            if k <= n {
                let v = self.sum[k] + (step as i64 - self.cnt[k] as i64) * est;
                if v <= rem {
                    pos = k;
                    rem -= v;
                }
            }
            step >>= 1;
        }
        if pos >= n { (n - 1, self.height(n - 1)) } else { (pos, rem.min(i32::MAX as i64) as i32) }
    }

    /// the scroll position: px from the first row's top to the view's top
    pub(crate) fn pos(&self) -> i64 {
        self.prefix(self.top) + self.off as i64
    }

    pub(crate) fn set_pos(&mut self, y: i64) {
        let (t, o) = self.find(y);
        self.top = t;
        self.off = o;
    }

    /// the anchor inside the rows, the view not beyond the end (view: its height px)
    pub(crate) fn clamp(&mut self, view: i32) {
        if self.h.is_empty() {
            self.top = 0;
            self.off = 0;
            return;
        }
        let n = self.h.len();
        if self.top >= n {
            self.top = n - 1;
            self.off = 0;
        }
        // the anchor walks into the row it is in now (rows measured since it was set must not
        // move it: it is not taken from the absolute position again)
        while self.off >= self.height(self.top) && self.top + 1 < n {
            self.off -= self.height(self.top);
            self.top += 1;
        }
        while self.off < 0 && self.top > 0 {
            self.top -= 1;
            self.off += self.height(self.top);
        }
        self.off = self.off.clamp(0, self.height(self.top) - 1);
        let max = (self.total() - view as i64).max(0);
        if self.pos() > max {
            self.set_pos(max);
        }
    }

    /// n rows inserted before row `at` (the anchor stays on its row)
    pub(crate) fn insert(&mut self, at: usize, n: usize) {
        let at = at.min(self.h.len());
        self.h.splice(at..at, core::iter::repeat_n(0, n));
        self.rebuild();
        if at <= self.top && self.h.len() > n {
            self.top += n;
        }
    }

    /// rows at .. at + n removed (the anchor stays on its row; on the first row after them
    /// when it was one of them)
    pub(crate) fn remove(&mut self, at: usize, n: usize) {
        let at = at.min(self.h.len());
        let end = at.saturating_add(n).min(self.h.len());
        self.h.drain(at..end);
        self.rebuild();
        if self.top >= end {
            self.top -= end - at;
        } else if self.top >= at {
            self.top = at;
            self.off = 0;
        }
        if self.top >= self.h.len() {
            self.top = self.h.len().saturating_sub(1);
            self.off = 0;
        }
    }

    /// row i fully in a view of `view` px (at the top when it is above or taller than the view)
    pub(crate) fn reveal(&mut self, i: usize, view: i32) {
        if i >= self.h.len() {
            return;
        }
        let y = self.prefix(i) - self.pos();
        let h = self.height(i) as i64;
        if y < 0 || h >= view as i64 {
            self.top = i;
            self.off = 0;
        } else if y + h > view as i64 {
            let p = self.prefix(i) + h - view as i64;
            self.set_pos(p);
        }
    }
}

// ------------------------------------------------------------------ the app's state

/// the state of a list with rows of different heights (see the module header)
#[derive(Clone)]
pub struct VListState {
    /// the selected row, -1 none
    pub sel: i32,
    /// the height of a row that was not measured yet (DIP; 0: 40 at first, then the mean of
    /// the measured rows)
    pub estimate: i32,
    rows: Rows,
    // heights the app gave (DIP; 0: measured)
    given: Vec<i32>,
    // dpi, width px, text scale of the measurements
    key: (i32, i32, u32),
    // smooth scrolling: px still to go, its clock
    pending: i32,
    t0: i32,
    reveal: Option<(usize, u8)>,
    // the pointer's y and the bar's position when the scroll bar last moved the view
    sb_y: i32,
    sb_pos: i32,
}

impl Default for VListState {
    fn default() -> VListState {
        VListState::new()
    }
}

impl VListState {
    /// no rows yet, nothing selected
    pub fn new() -> VListState {
        let rows = Rows { est: EST_DIP, ..Rows::default() };
        VListState { sel: -1, estimate: 0, rows, given: Vec::new(), key: (0, 0, 0), pending: 0, t0: 0, reveal: None, sb_y: i32::MIN, sb_pos: 0 }
    }

    fn dpi(&self) -> i32 {
        if self.key.0 > 0 { self.key.0 } else { 96 }
    }

    /// row i is `dip` high (the app knows it: its box is not measured); 0 measures it again
    pub fn set_height(&mut self, i: usize, dip: i32) {
        if self.given.len() <= i {
            if dip <= 0 {
                return;
            }
            self.given.resize(i + 1, 0);
        }
        self.given[i] = dip.max(0);
        if i < self.rows.len() {
            if dip > 0 {
                let px = crate::layout::dp_at(dip, self.dpi()).max(1);
                self.rows.set(i, px);
            } else {
                self.rows.forget(i);
            }
        }
    }

    /// row i is measured again the next time it is built
    pub fn invalidate(&mut self, i: usize) {
        if self.given.get(i).is_none_or(|&g| g == 0) {
            self.rows.forget(i);
        }
    }

    /// every row is measured again
    pub fn invalidate_all(&mut self) {
        self.rows.forget_all();
        self.key.1 = -1;
    }

    /// n rows were inserted before row `at`: the view keeps showing the rows it showed (also
    /// the selection stays on its row)
    pub fn insert(&mut self, at: usize, n: usize) {
        self.rows.insert(at, n);
        if self.given.len() > at {
            self.given.splice(at..at, core::iter::repeat_n(0, n));
        }
        if self.sel >= 0 && self.sel as usize >= at {
            self.sel += n as i32;
        }
    }

    /// rows at .. at + n were removed (the selection on one of them: none)
    pub fn remove(&mut self, at: usize, n: usize) {
        self.rows.remove(at, n);
        if self.given.len() > at {
            let end = at.saturating_add(n).min(self.given.len());
            self.given.drain(at..end);
        }
        if self.sel >= 0 {
            let s = self.sel as usize;
            if s >= at.saturating_add(n) {
                self.sel -= n as i32;
            } else if s >= at {
                self.sel = -1;
            }
        }
    }

    /// scroll row i fully into view (the next pass)
    pub fn reveal(&mut self, i: usize) {
        self.reveal = Some((i, 0));
    }

    /// show row i at the top of the view
    pub fn scroll_to(&mut self, i: usize) {
        self.rows.top = i;
        self.rows.off = 0;
        self.pending = 0;
        self.reveal = None;
    }

    /// the first visible row and how many px of it are scrolled out above the view
    pub fn first_visible(&self) -> (usize, i32) {
        (self.rows.top, self.rows.off)
    }

    /// row i's height in px (measured or given; else the estimate)
    pub fn row_height(&self, i: usize) -> i32 {
        self.rows.height(i)
    }

    /// row i's height is measured or given
    pub fn row_known(&self, i: usize) -> bool {
        self.rows.known(i)
    }

    /// the number of rows (vlist_begin sets it from its count; rows come and go at the end)
    pub fn set_count(&mut self, n: usize) {
        self.rows.resize(n);
        for (i, &g) in self.given.iter().enumerate().take(n) {
            if g > 0 {
                let px = crate::layout::dp_at(g, self.dpi()).max(1);
                self.rows.set(i, px);
            }
        }
    }

    /// the scroll position: px from the first row's top to the view's top (rows not measured
    /// count with the estimate)
    pub fn scroll_pos(&self) -> i64 {
        self.rows.pos()
    }

    /// scroll to a position (px; the next pass keeps the view inside the rows)
    pub fn set_scroll_pos(&mut self, px: i64) {
        self.rows.set_pos(px);
        self.pending = 0;
        self.reveal = None;
    }

    /// the height of all rows (px; rows not measured count with the estimate)
    pub fn total_height(&self) -> i64 {
        self.rows.total()
    }
}

// ------------------------------------------------------------------ the widget

// an open list (between vlist_begin and vlist_end)
struct Open {
    id: u32,
    st: VListState,
    count: usize,
    rect: [i32; 4],
    // the rows' column (px): x, width
    rx: i32,
    rw: i32,
    // the next row's top, the next row expected, the row being built
    y: i32,
    next: usize,
    cur: usize,
    cur_y: i32,
    vev: u32,
    ev: u32,
    old: i32,
    kdbl: bool,
    changed: bool,
    hit0: usize,
    layer: u32,
    // the row to reveal was built: its final top and height
    shown: Option<(i32, i32)>,
}

/// shared state (Ui::m.vlist): the open lists
#[derive(Default)]
pub(crate) struct S {
    open: Vec<Open>,
}

impl Ui {
    /// ui_vlist_begin(id, &mut st, count) -> (first, end): the rows to emit (see the module
    /// header)
    pub fn vlist_begin(&mut self, id: u32, st: &mut VListState, count: usize) -> (usize, usize) {
        let mut s = core::mem::take(st);
        let r = self.place(self.dp(NAT_W), self.dp(NAT_H));
        let sbw = self.style.scrollbar_thickness(self);
        let view = r[3].max(1);
        // the measurements belong to a dpi, a width and a text scale
        let key = (self.wp.rt.dpi, r[2], self.text_scale);
        if key != s.key {
            let dpi_changed = key.0 != s.key.0;
            s.key = key;
            s.rows.forget_all();
            if dpi_changed || s.rows.len() != count {
                s.rows.resize(count);
            }
            for (i, &g) in s.given.iter().enumerate() {
                if g > 0 && i < count {
                    s.rows.set(i, self.dp(g).max(1));
                }
            }
        }
        if s.rows.len() != count {
            s.rows.resize(count);
            for (i, &g) in s.given.iter().enumerate().take(count) {
                if g > 0 {
                    s.rows.set(i, self.dp(g).max(1));
                }
            }
        }
        if s.sel >= count as i32 {
            s.sel = count as i32 - 1;
        }
        s.rows.est = if s.estimate > 0 { self.dp(s.estimate) } else { s.rows.mean().unwrap_or(self.dp(EST_DIP)) }.max(1);
        // the view: focus and keys
        self.wp.ctx.last_rect = r;
        let vev = self.behave(id, HB_FOCUS | HB_OWNKEYS);
        let old = s.sel;
        let mut kdbl = false;
        if vev & EV_FOCUS != 0 && count > 0 {
            kdbl = self.vl_keys(&mut s, count, view);
        }
        // smooth scrolling
        if s.pending != 0 {
            let p0 = s.pending;
            if self.anim_off {
                s.pending = 0;
            } else {
                let steps = anim_steps(self.now_ms, &mut s.t0);
                s.pending = anim_decay(p0, 0, steps);
            }
            let moved = p0 - s.pending;
            let p = s.rows.pos() + moved as i64;
            s.rows.set_pos(p);
            if s.pending != 0 {
                self.animating = true;
            }
        }
        if let Some((i, _)) = s.reveal {
            s.rows.reveal(i, view);
        }
        s.rows.clamp(view);
        let first = s.rows.top.min(count);
        let mut end = first;
        let mut y = r[1] - s.rows.off;
        while end < count && y < r[1] + view {
            y += s.rows.height(end);
            end += 1;
        }
        self.clip_push(r[0], r[1], r[2], r[3]);
        let c = &self.wp.ctx;
        let (hit0, layer) = (c.hits[c.hit_cur].len(), c.cmd_layer_ui);
        let rw = (r[2] - sbw).max(1);
        let y0 = r[1] - s.rows.off;
        self.m.vlist.open.push(Open {
            id,
            st: s,
            count,
            rect: r,
            rx: r[0],
            rw,
            y: y0,
            next: first,
            cur: usize::MAX,
            cur_y: 0,
            vev,
            ev: 0,
            old,
            kdbl,
            changed: false,
            hit0,
            layer,
            shown: None,
        });
        (first, end)
    }

    // the keys of a focused list -> Enter was pressed
    fn vl_keys(&mut self, s: &mut VListState, count: usize, view: i32) -> bool {
        let n = count as i64;
        let cur = s.sel as i64;
        let start = if cur < 0 { s.rows.top as i64 } else { cur };
        // the row a page away (by the heights)
        let page = |s: &VListState, from: i64, down: bool| -> i64 {
            let (mut i, mut acc) = (from, 0i64);
            loop {
                let next = if down { i + 1 } else { i - 1 };
                if next < 0 || next >= n {
                    return i;
                }
                acc += s.rows.height(next as usize) as i64;
                if acc > view as i64 {
                    return if i == from { next } else { i };
                }
                i = next;
            }
        };
        let mut new = None;
        if self.key(VK_UP) {
            new = Some(if cur < 0 { start } else { cur - 1 });
        }
        if self.key(VK_DOWN) {
            new = Some(if cur < 0 { start } else { cur + 1 });
        }
        if self.key(VK_HOME) {
            new = Some(0);
        }
        if self.key(VK_END) {
            new = Some(n - 1);
        }
        if self.key(VK_PRIOR) {
            new = Some(page(s, start, false));
        }
        if self.key(VK_NEXT) {
            new = Some(page(s, start, true));
        }
        let enter = cur >= 0 && self.key(VK_RETURN);
        if let Some(v) = new {
            let v = v.clamp(0, n - 1);
            if v != cur {
                s.sel = v as i32;
                self.repass();
            }
            s.reveal = Some((v as usize, 0));
            self.wp.ctx.focus_visible = true;
        }
        if enter {
            s.reveal = Some((cur as usize, 0));
            self.repass();
        }
        enter
    }

    /// ui_vlist_item_begin(i) -> EV_*: row i (see the module header)
    pub fn vlist_item_begin(&mut self, i: usize) -> u32 {
        let Some(mut o) = self.m.vlist.open.pop() else {
            self.warn("ui_vlist_item_begin outside a vlist");
            return 0;
        };
        // rows the caller skipped keep their space
        if o.next < i {
            for k in o.next..i.min(o.count) {
                o.y += o.st.rows.height(k);
            }
        }
        o.next = i + 1;
        let h = o.st.rows.height(i);
        let rid = o.id.wrapping_add(1).wrapping_add(i as u32);
        let rect = [o.rx, o.y, o.rw, h];
        self.wp.ctx.last_rect = rect;
        let ev0 = self.behave(rid, 0);
        let mut ev = ev0;
        let c = &mut self.wp.ctx;
        if c.active == rid && c.pressed & (1 << BTN_LEFT) != 0 {
            c.focus = o.id;
            c.focus_visible = false;
        }
        if ev & EV_ACT != 0 && o.st.sel != i as i32 {
            o.st.sel = i as i32;
            o.ev |= EV_ACT;
            self.repass();
        }
        let selected = o.st.sel == i as i32;
        if selected && core::mem::take(&mut o.kdbl) {
            ev |= EV_DOUBLE;
        }
        if ev & EV_DOUBLE != 0 {
            o.ev |= EV_DOUBLE;
        }
        let fl = if o.vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible { 2 } else { 0 };
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let mut sp = Sp { ev, value: selected as i32, value2: disabled as i32, flags: fl, ..Sp::default() };
        sp.place(rect);
        let st = self.style;
        st.list_row(self, &sp);
        if selected {
            self.wp.ctx.last_state |= SS_SELECTED as u32;
        }
        self.sem(K_LISTITEM, rid, None, i as i32);
        // the content: natural height (or the height the app gave)
        let given = o.st.given.get(i).copied().unwrap_or(0) > 0;
        let px = self.dp(8);
        self.next_rect_px(o.rx + px, o.y, (o.rw - px * 2).max(1), if given { h } else { 0 });
        o.cur = i;
        o.cur_y = o.y;
        self.m.vlist.open.push(o);
        self.box_begin(rid | 0xC000_0000, BOX_V | if given { BOX_FIXED } else { 0 } | pad(4), 4);
        ev0 | (ev & EV_DOUBLE)
    }

    /// ui_vlist_item_end: the row's height is measured
    pub fn vlist_item_end(&mut self) {
        self.box_end();
        let Some(mut o) = self.m.vlist.open.pop() else {
            return;
        };
        let i = o.cur;
        if i < o.count {
            let rid = o.id.wrapping_add(1).wrapping_add(i as u32);
            let given = o.st.given.get(i).copied().unwrap_or(0) > 0;
            if !given {
                let mh = self.slot(rid | 0xC000_0000).meash.max(1);
                if o.st.rows.set(i, mh) {
                    o.changed = true;
                }
            }
            let h = o.st.rows.height(i);
            if o.st.reveal.is_some_and(|r| r.0 == i) {
                o.shown = Some((o.cur_y - o.rect[1], h));
            }
            o.y = o.cur_y + h;
        }
        o.cur = usize::MAX;
        self.m.vlist.open.push(o);
    }

    /// the selection of the innermost vlist while its rows are emitted (-1 outside one)
    pub fn vlist_sel(&self) -> i32 {
        self.m.vlist.open.last().map_or(-1, |o| o.st.sel)
    }

    /// ui_vlist_end(&mut st) -> EV_* (see the module header)
    pub fn vlist_end(&mut self, st: &mut VListState) -> u32 {
        let Some(mut o) = self.m.vlist.open.pop() else {
            return 0;
        };
        self.clip_pop();
        let r = o.rect;
        let view = r[3].max(1);
        let s = &mut o.st;
        // the row to reveal: done when it was built and shows whole (its rows measured)
        if let Some((i, tries)) = s.reveal {
            let done = match o.shown {
                Some((y, h)) => (y >= 0 && y + h <= view) || (y == 0 && h >= view) || !o.changed,
                None => false,
            };
            if done || tries >= REVEAL_TRIES || i >= o.count {
                s.reveal = None;
            } else {
                s.reveal = Some((i, tries + 1));
                self.repass();
            }
        }
        // rows shorter than thought leave room below: another pass fills it
        if o.changed {
            self.repass();
        }
        // the scroll bar
        let total = s.rows.total().min(i32::MAX as i64) as i32;
        if total > view {
            let sbw = self.style.scrollbar_thickness(self);
            // a dragged thumb moves the view when the pointer moves, not when rows measured in
            // this pass change the total (that would chase the thumb pass after pass): while
            // the pointer stays, the bar compares with the position it reported last
            let sid = part_id(o.id, 1);
            let c = &self.wp.ctx;
            let (my, press) = (c.my, c.pressed & (1 << BTN_LEFT) != 0);
            let held = c.active == sid && c.mbtn & (1 << BTN_LEFT) != 0;
            let mut pos = if held && !press && my == s.sb_y { s.sb_pos } else { s.rows.pos().min(i32::MAX as i64) as i32 };
            self.next_rect_px(r[0] + r[2] - sbw, r[1], sbw, r[3]);
            let ev = self.scrollbar(sid, &mut pos, total, view, true);
            if ev & EV_ACT != 0 && (press || my != o.st.sb_y) {
                o.st.rows.set_pos(pos as i64);
                o.st.pending = 0;
                o.st.reveal = None;
                self.repass();
            }
            o.st.sb_y = my;
            o.st.sb_pos = pos;
        }
        let s = &mut o.st;
        // the wheel over the list (one of its hit boxes is hot)
        let c = &self.wp.ctx;
        let wy = c.wheel_y;
        let hot = c.hot;
        let over = hot != 0 && c.hits[c.hit_cur].get(o.hit0..).is_some_and(|v| v.iter().any(|h| h.id == hot && h.layer as u32 == o.layer));
        if (over || o.vev & EV_HOVER != 0) && wy != 0 {
            let step = self.dp(WHEEL_DIP);
            let d = wy.wrapping_neg().wrapping_mul(step) / 120;
            let p = s.rows.pos() + (s.pending + d) as i64;
            let max = (s.rows.total() - view as i64).max(0);
            if (d < 0 && s.rows.pos() > 0) || (d > 0 && s.rows.pos() < max) || p.clamp(0, max) != s.rows.pos() {
                if s.pending == 0 {
                    s.t0 = self.now_ms as i32;
                }
                s.pending += d;
                self.wp.ctx.wheel_y = 0;
                s.reveal = None;
                if self.anim_off {
                    self.repass();
                } else {
                    self.animating = true;
                }
            }
        }
        // the focus visual on the whole list when no row is selected
        if o.vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible && s.sel < 0 {
            let st = self.style;
            st.focus_rect(self, r[0], r[1], r[2], r[3]);
        }
        let mut stt = 0;
        if o.vev & EV_HOVER != 0 {
            stt |= SS_HOVER as u32;
        }
        if o.vev & EV_FOCUS != 0 {
            stt |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = stt;
        let sel = s.sel;
        self.sem(K_LIST, o.id, None, sel).max = o.count as i32;
        let mut ev = o.ev | (o.vev & (EV_FOCUS | EV_HOVER));
        if sel != o.old {
            ev |= EV_ACT;
        }
        if o.kdbl {
            ev |= EV_DOUBLE; // Enter while the selected row was not built
        }
        *st = o.st;
        self.wp.ctx.last_rect = r;
        ev
    }
}
