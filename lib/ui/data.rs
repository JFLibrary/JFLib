// data -- data views: grid (table with columns), tree, badges, status dots, meters
//
//   grid_begin(id, &mut st, cols, keys) -> (first, end)
//        a virtualized table (fills its parent unless next_size says otherwise): a fixed
//        header (click: sort by the column, again: descending;
//        drag a column's right edge: its width, double click there: the default width) and
//        the rows below it in a vertical scroll view (row height and header height from
//        StyleData::grid_metrics).  keys: one stable key per row in display order (the app
//        sorts and filters; the selection is kept by key, so it survives refreshes).
//        Emit rows first .. end-1: grid_row(&mut st, i), then per column grid_cell(&st, c)
//        and any widget (it is placed into the cell), or grid_text(&st, c, s).
//        Mouse: click selects, Ctrl+click toggles, Shift+click selects a range, double click
//        activates, right click selects the row and reports it.  Keys (grid focused): Up /
//        Down / PgUp / PgDn / Home / End move the current row (Shift: extend), Ctrl+A all,
//        Space toggles the current row, Enter activates.
//   grid_row(&mut st, i) -> EV        row i (widget id id + 1 + i)
//   grid_cell(&st, c)                 the next widget goes into column c of the current row
//   grid_text(&st, c, s)              text in column c (ellipsis, the column's alignment)
//   grid_end(&mut st) -> GridEv       ev: EV_ACT selection changed, EV_DOUBLE a row was
//                                     activated, EV_RCLICK context menu requested; row: that
//                                     row's index; sort_changed: re-sort the keys
//   tree(id, &mut st, rows, row_h) -> TreeEv
//        a virtualized tree from the app's visible rows (TreeRow: key, depth, children,
//        check state, icon, label, detail); the chevron / Right / Left expand and collapse
//        (st.expanded; TreeEv::expanded names a newly opened node for lazy loading), the
//        check box / Space report TreeEv::check (the app applies it, e.g. with cascade),
//        double click / Enter TreeEv::activated
//   badge(kind, label)                status pill (BADGE_NEUTRAL .. BADGE_ACCENT; next_icon); in
//                                     a narrower cell the label is cut with "…"
//   status_dot(kind)                  a small status dot (10 DIP)
//   meter(id, v, max, kind)           a capacity bar (next_size sets its size)
//
// Ids: the header box (id << 3) | 0x80000000, the header cells ((id ^ 0xA5A5) << 3 | k) |
// 0x80000000 with k = 1 + column (sort), the grips ((id ^ 0x5A5A) << 3 | k) | 0x80000000; the
// rows' scroll view id (its bars (id << 3 | 1 / 2) | 0x80000000); rows id + 1 + i; tree chevrons / check boxes
// (id + 1 + i) | 0x40000000 / | 0x20000000.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::list::{into_view, SCR_F_LIST};
use crate::{Sp, Ui};

/// a grid column: title, width and minimum width in DIP, GC_* flags
#[derive(Clone, Copy, Debug)]
pub struct GridCol<'a> {
    pub title: &'a str,
    pub width: i32,
    pub min: i32,
    pub flags: u32,
}

impl<'a> GridCol<'a> {
    /// a column `width` DIP wide (at least 48)
    pub const fn new(title: &'a str, width: i32) -> GridCol<'a> {
        GridCol { title, width, min: 48, flags: 0 }
    }
    /// the same column with GC_* flags
    pub const fn flags(mut self, flags: u32) -> GridCol<'a> {
        self.flags = flags;
        self
    }
    /// the same column with a minimum width (DIP)
    pub const fn min(mut self, min: i32) -> GridCol<'a> {
        self.min = min;
        self
    }
}

/// the per-pass data of a grid between grid_begin and grid_end
#[derive(Clone, Default, Debug)]
struct GridFrame {
    id: u32,
    keys: Vec<u64>,
    /// column x (relative to the grid), width, GC_* flags (px)
    cols: Vec<(i32, i32, u32)>,
    x0: i32,
    row_h: i32,
    pad: i32,
    next: i32,
    vev: u32,
    ev: u32,
    row: i32,
    row_rect: [i32; 4],
    cur_row: i32,
    focus_visible: bool,
}

/// the app-owned state of a grid: selection (by key), current row, sort, column widths
#[derive(Clone, Default, Debug)]
pub struct GridState {
    /// the selected rows' keys
    pub sel: BTreeSet<u64>,
    /// the current row (keyboard focus / last clicked)
    pub cur: Option<u64>,
    anchor: Option<u64>,
    /// the sort column and direction (true = descending); None = the app's order
    pub sort: Option<(usize, bool)>,
    /// column widths set by the user (DIP, 0 = the column's own)
    pub widths: Vec<i32>,
    /// alternate row shading
    pub striped: bool,
    f: GridFrame,
}

impl GridState {
    /// the keys selected, in display order of the last pass
    pub fn selected_in_order(&self) -> Vec<u64> {
        self.f.keys.iter().copied().filter(|k| self.sel.contains(k)).collect()
    }
    /// select exactly this key (None: nothing) and make it current
    pub fn select_only(&mut self, key: Option<u64>) {
        self.sel.clear();
        if let Some(k) = key {
            self.sel.insert(k);
        }
        self.cur = key;
        self.anchor = key;
    }
    /// drop selected keys that are no longer shown (after a refresh)
    pub fn retain(&mut self, keys: &[u64]) {
        let set: BTreeSet<u64> = keys.iter().copied().collect();
        self.sel.retain(|k| set.contains(k));
        if self.cur.is_some_and(|k| !set.contains(&k)) {
            self.cur = None;
        }
        if self.anchor.is_some_and(|k| !set.contains(&k)) {
            self.anchor = None;
        }
    }
}

/// what happened in a grid during the pass
#[derive(Clone, Copy, Default, Debug)]
pub struct GridEv {
    /// EV_ACT selection changed, EV_DOUBLE a row was activated, EV_RCLICK context requested
    pub ev: u32,
    /// the activated / context row (-1 = none)
    pub row: i32,
    /// the sort column or direction changed
    pub sort_changed: bool,
}

/// one visible row of a tree (the app flattens its model through TreeState::expanded)
#[derive(Clone, Copy, Debug)]
pub struct TreeRow<'a> {
    pub key: u64,
    pub depth: u16,
    pub has_children: bool,
    /// check box state 0 / 1 / 2 (indeterminate); None = no check box
    pub check: Option<u8>,
    pub icon: u32,
    pub label: &'a str,
    /// secondary text after the label (size, date)
    pub detail: Option<&'a str>,
}

/// the app-owned state of a tree: expanded nodes, current node
#[derive(Clone, Default, Debug)]
pub struct TreeState {
    pub expanded: BTreeSet<u64>,
    pub cur: Option<u64>,
    /// app data per node (free use, e.g. a loading flag)
    pub marks: BTreeMap<u64, u32>,
}

/// what happened in a tree during the pass
#[derive(Clone, Copy, Default, Debug)]
pub struct TreeEv {
    /// a node opened in this pass (load its children)
    pub expanded: Option<u64>,
    /// a node closed in this pass
    pub collapsed: Option<u64>,
    /// a check box was toggled: (key, new state 0 / 1)
    pub check: Option<(u64, u8)>,
    /// double click / Enter on a node
    pub activated: Option<u64>,
    /// the current node changed
    pub sel_changed: bool,
}

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

/// a header cell: not part(id, 1 + column) -- the rows' scroll view (id) has its scroll bars
/// there (part 1 vertical, 2 horizontal), a click on the bar sorted by the first column
fn head(id: u32, column: u32) -> u32 {
    part(id ^ 0xA5A5, 1 + column)
}

impl Ui {
    // ------------------------------------------------------------------ grid

    /// ui_grid_begin(id, &mut st, cols, keys) -> (first, end): the rows to emit
    pub fn grid_begin(&mut self, id: u32, st: &mut GridState, cols: &[GridCol], keys: &[u64]) -> (i32, i32) {
        let sd = self.style.data();
        let (hh, rh, pad) = sd.grid_metrics(self);
        let (hh, rh, pad) = (self.dp(hh), self.dp(rh).max(1), self.dp(pad));
        // outer column: header + rows; without next_size the grid fills its parent
        if self.wp.lay.next_w == 0 && self.wp.lay.next_h == 0 && self.wp.lay.next_rect_set == 0 {
            self.next_size(-1, -1);
        }
        self.box_begin(part(id, 0), BOX_V, 0);
        let outer = self.box_rect();
        // column geometry: user widths, the fill column takes what is left
        if st.widths.len() != cols.len() {
            st.widths.resize(cols.len(), 0);
        }
        let mut ws: Vec<i32> = cols
            .iter()
            .zip(&st.widths)
            .map(|(c, &u)| self.dp(if u > 0 { u.max(c.min) } else { c.width.max(c.min) }))
            .collect();
        let fill = cols.iter().position(|c| c.flags & GC_FILL != 0).unwrap_or(cols.len().saturating_sub(1));
        let total: i32 = ws.iter().sum();
        if !ws.is_empty() && total < outer[2] {
            ws[fill] += outer[2] - total;
        } else if !ws.is_empty() && total > outer[2] && st.widths[fill] == 0 {
            // too narrow: the fill column gives way down to its minimum (then the grid scrolls)
            let floor = self.dp(cols[fill].min.max(48));
            ws[fill] = (ws[fill] - (total - outer[2])).max(floor.min(ws[fill]));
        }
        let mut colv = Vec::with_capacity(cols.len());
        let mut x = 0;
        for (c, &w) in cols.iter().zip(&ws) {
            colv.push((x, w, c.flags));
            x += w;
        }
        // header strip
        let hr = self.place(0, hh);
        let mut sp = Sp::default();
        sp.place(hr);
        sd.grid_header_bg(self, &sp);
        self.clip_push(hr[0], hr[1], hr[2], hr[3]);
        let mut sort_changed = false;
        for (i, c) in cols.iter().enumerate() {
            let (cx, cw, cf) = colv[i];
            let r = [hr[0] + cx, hr[1], cw, hh];
            self.wp.ctx.last_rect = r;
            let sortable = cf & GC_NOSORT == 0;
            let ev = self.behave(head(id, i as u32), if sortable { 0 } else { HB_DISABLED });
            if ev & EV_ACT != 0 && sortable {
                st.sort = match st.sort {
                    Some((s, desc)) if s == i => Some((i, !desc)),
                    _ => Some((i, false)),
                };
                sort_changed = true;
                self.repass();
            }
            let mut f = 0;
            match st.sort {
                Some((s, false)) if s == i => f |= GH_SORT_ASC,
                Some((s, true)) if s == i => f |= GH_SORT_DESC,
                _ => {}
            }
            if i + 1 == cols.len() {
                f |= GH_LAST;
            }
            if cf & GC_RIGHT != 0 {
                f |= GH_RIGHT;
            }
            let mut sp = Sp { ev, flags: f, label: Some(c.title), ..Sp::default() };
            sp.place(r);
            sp.s[0] = pad;
            sp.state = if self.wp.ctx.last_state == SS_DISABLED as u32 { 0 } else { sp.state };
            sd.grid_header(self, &sp);
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = 0;
            self.sem(K_BUTTON, head(id, i as u32), Some(c.title), f as i32);
        }
        // column resize grips (on top of the header cells)
        for (i, c) in cols.iter().enumerate() {
            if i + 1 == cols.len() {
                break;
            }
            let (cx, cw, _) = colv[i];
            let gw = self.dp(8);
            let gid = (id ^ 0x5A5A).wrapping_mul(8).wrapping_add(1 + i as u32) | 0x8000_0000;
            let ev = self.hit_rect(gid, hr[0] + cx + cw - (gw >> 1), hr[1], gw, hh, CUR_SIZE_WE << HB_CURSOR_SHIFT);
            if ev & EV_DOUBLE != 0 {
                st.widths[i] = 0;
                self.repass();
            } else if ev & EV_HELD != 0 {
                let w = self.px2dip(self.wp.ctx.mx - (hr[0] + cx)).max(c.min);
                if w != st.widths[i] {
                    st.widths[i] = w;
                    self.repass();
                }
            }
        }
        self.clip_pop();
        // the rows: a vertical scroll view that takes the keyboard focus (the menu key on it
        // is the current row's right click, kbnav.rs)
        self.next_size(-1, -1);
        if let Some(c) = st.cur.and_then(|k| keys.iter().position(|&x| x == k)) {
            self.kb_ctx_redirect(id, id.wrapping_add(1).wrapping_add(c as u32));
        }
        let vev = self.scr_open(id, SC_V | SCR_F_LIST, HB_FOCUS | HB_OWNKEYS);
        let count = keys.len() as i32;
        st.f = GridFrame {
            id,
            keys: keys.to_vec(),
            cols: colv,
            x0: hr[0],
            row_h: rh,
            pad,
            next: 0,
            vev,
            ev: if sort_changed { 0x8000_0000 } else { 0 },
            row: -1,
            row_rect: [0; 4],
            cur_row: st.cur.and_then(|k| keys.iter().position(|&x| x == k)).map_or(-1, |p| p as i32),
            focus_visible: false,
        };
        if vev & EV_FOCUS != 0 {
            self.grid_keys(st);
        }
        self.scr_offset();
        let y0 = self.box_rect()[1];
        let k = self.wp.ctx.uiclip;
        let first = (k[1].wrapping_sub(y0).max(0) / rh).min(count);
        let end = ((k[3].wrapping_sub(y0).max(0) + rh - 1) / rh).min(count).max(first);
        st.f.next = first;
        if first != 0 {
            self.place(0, first * rh);
        }
        st.f.focus_visible = vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible;
        (first, end)
    }

    // grid_keys: the selection keys of a focused grid
    fn grid_keys(&mut self, st: &mut GridState) {
        let n = st.f.keys.len() as i32;
        if n == 0 {
            return;
        }
        let (_, view) = self.scr_view();
        let page = (view[3] / st.f.row_h).max(1);
        let cur = st.f.cur_row;
        let shift = self.mods & MOD_SHIFT != 0;
        let ctrl = self.mods & MOD_CTRL != 0;
        let mut new = cur;
        if self.key(VK_UP) {
            new = if cur > 0 { cur - 1 } else { 0 };
        }
        if self.key(VK_DOWN) {
            new = if cur < 0 { 0 } else { cur + 1 };
        }
        if self.key(VK_HOME) {
            new = 0;
        }
        if self.key(VK_END) {
            new = n - 1;
        }
        if self.key(VK_PRIOR) {
            new = cur - page;
        }
        if self.key(VK_NEXT) {
            new = if cur < 0 { page } else { cur + page };
        }
        new = new.clamp(0, n - 1);
        if ctrl && self.key(b'A' as u32) {
            st.sel = st.f.keys.iter().copied().collect();
            st.f.ev |= EV_ACT;
            self.repass();
        }
        if self.key(VK_SPACE) && cur >= 0 {
            let k = st.f.keys[cur as usize];
            if !st.sel.remove(&k) {
                st.sel.insert(k);
            }
            st.anchor = Some(k);
            st.f.ev |= EV_ACT;
            self.repass();
        }
        if self.key(VK_RETURN) && cur >= 0 {
            st.f.ev |= EV_DOUBLE;
            st.f.row = cur;
        }
        if new == cur {
            return;
        }
        let k = st.f.keys[new as usize];
        st.cur = Some(k);
        st.f.cur_row = new;
        if shift {
            let a = st.anchor.and_then(|a| st.f.keys.iter().position(|&x| x == a)).map_or(new, |p| p as i32);
            let (lo, hi) = (a.min(new) as usize, a.max(new) as usize);
            st.sel = st.f.keys[lo..=hi].iter().copied().collect();
        } else {
            st.sel.clear();
            st.sel.insert(k);
            st.anchor = Some(k);
        }
        st.f.ev |= EV_ACT;
        self.wp.ctx.focus_visible = true;
        self.repass();
        // scroll the row into view at once
        let (vid, view) = self.scr_view();
        let rh = st.f.row_h;
        let s = self.slot(vid);
        let d = into_view(new * rh, rh, s.scrolly, view[3]);
        s.scrolly += d;
        s.adur = s.scrolly as u32;
    }

    /// ui_grid_row(&mut st, i) -> EV_* of row i (EV_ACT clicked)
    pub fn grid_row(&mut self, st: &mut GridState, i: i32) -> u32 {
        let f = &st.f;
        let (id, rh) = (f.id, f.row_h);
        let d = i - f.next;
        if d > 0 {
            self.place(0, d * rh);
        }
        st.f.next = i + 1;
        let r = self.place(0, rh);
        let iid = id.wrapping_add(1).wrapping_add(i as u32);
        let ev = self.behave(iid, 0);
        let key = st.f.keys.get(i as usize).copied().unwrap_or(0);
        // a press gives the grid the keyboard focus
        let c = &mut self.wp.ctx;
        if c.active == iid && c.pressed & (1 << BTN_LEFT) != 0 {
            c.focus = id;
            c.focus_visible = false;
        }
        if ev & EV_ACT != 0 {
            let m = self.mods;
            if m & MOD_SHIFT != 0 {
                let a = st.anchor.and_then(|a| st.f.keys.iter().position(|&x| x == a)).map_or(i, |p| p as i32);
                let (lo, hi) = (a.min(i) as usize, a.max(i) as usize);
                st.sel = st.f.keys[lo..=hi.min(st.f.keys.len() - 1)].iter().copied().collect();
            } else if m & MOD_CTRL != 0 {
                if !st.sel.remove(&key) {
                    st.sel.insert(key);
                }
                st.anchor = Some(key);
            } else {
                st.sel.clear();
                st.sel.insert(key);
                st.anchor = Some(key);
            }
            st.cur = Some(key);
            st.f.cur_row = i;
            st.f.ev |= EV_ACT;
            self.repass();
        }
        if ev & EV_RCLICK != 0 {
            if !st.sel.contains(&key) {
                st.sel.clear();
                st.sel.insert(key);
                st.anchor = Some(key);
                st.f.ev |= EV_ACT;
            }
            st.cur = Some(key);
            st.f.cur_row = i;
            st.f.ev |= EV_RCLICK;
            st.f.row = i;
            self.repass();
        }
        if ev & EV_DOUBLE != 0 {
            st.f.ev |= EV_DOUBLE;
            st.f.row = i;
        }
        let selected = st.sel.contains(&key);
        let mut fl = 0;
        if st.f.cur_row == i {
            fl |= if st.f.focus_visible { GR_FOCUS } else { GR_CURRENT };
        }
        if st.striped && i & 1 != 0 {
            fl |= GR_STRIPE;
        }
        let mut sp = Sp { ev, value: selected as i32, value2: i, flags: fl, ..Sp::default() };
        sp.place(r);
        let sd = self.style.data();
        sd.grid_row(self, &sp);
        self.wp.ctx.last_rect = r;
        if selected {
            self.wp.ctx.last_state |= SS_SELECTED as u32;
        }
        self.sem(K_LISTITEM, iid, None, i);
        st.f.row_rect = r;
        ev
    }

    /// ui_grid_cell(&st, c): the next widget is placed into column c of the current row
    /// (inset by the cell padding, full row height)
    pub fn grid_cell(&mut self, st: &GridState, c: usize) {
        let r = st.f.row_rect;
        let Some(&(cx, cw, _)) = st.f.cols.get(c) else { return };
        let pad = st.f.pad;
        let x = st.f.x0 + cx + pad;
        let w = (cw - pad * 2).max(1);
        self.next_rect_px(x, r[1], w, r[3]);
    }

    /// ui_grid_text(&st, c, s): body text in column c (cut with "…", the column's alignment)
    pub fn grid_text(&mut self, st: &GridState, c: usize, s: &str) {
        self.grid_text_ts(st, c, TS_BODY, s);
    }

    /// ui_grid_text_ts(&st, c, ts, s): text in column c with a text style (TS_* | TX_*)
    pub fn grid_text_ts(&mut self, st: &GridState, c: usize, ts: u32, s: &str) {
        let f = st.f.cols.get(c).map_or(0, |c| c.2);
        let mut style = ts | TX_ELLIPSIS;
        if f & GC_RIGHT != 0 {
            style |= TX_RIGHT;
        } else if f & GC_CENTER != 0 {
            style |= TX_CENTER;
        }
        self.grid_cell(st, c);
        self.text(style, s);
    }

    /// ui_grid_end(&mut st) -> GridEv
    pub fn grid_end(&mut self, st: &mut GridState) -> GridEv {
        let n = st.f.keys.len() as i32;
        let d = n - st.f.next;
        if d > 0 {
            self.place(0, d * st.f.row_h);
        }
        self.scr_close();
        // focus visual on the whole view when nothing is current
        if st.f.vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible && st.f.cur_row < 0 {
            let r = self.wp.ctx.last_rect;
            let s = self.style;
            s.focus_rect(self, r[0], r[1], r[2], r[3]);
        }
        let mut stt = 0;
        if st.f.vev & EV_HOVER != 0 {
            stt |= SS_HOVER as u32;
        }
        if st.f.vev & EV_FOCUS != 0 {
            stt |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_state = stt;
        self.sem(K_LIST, st.f.id, None, st.f.cur_row).max = n;
        self.box_end(); // outer column
        let sort_changed = st.f.ev & 0x8000_0000 != 0;
        GridEv { ev: st.f.ev & 0x7FFF_FFFF, row: st.f.row, sort_changed }
    }

    // ------------------------------------------------------------------ tree

    /// ui_tree(id, &mut st, rows, row_h) -> TreeEv: the visible rows of a tree (row_h DIP,
    /// 0 = the grid row height)
    pub fn tree(&mut self, id: u32, st: &mut TreeState, rows: &[TreeRow], row_h: i32) -> TreeEv {
        let sd = self.style.data();
        let rh = self.dp(if row_h > 0 { row_h } else { sd.grid_metrics(self).1 }).max(1);
        let mut out = TreeEv::default();
        let n = rows.len() as i32;
        if self.wp.lay.next_w == 0 && self.wp.lay.next_h == 0 && self.wp.lay.next_rect_set == 0 {
            self.next_size(-1, -1);
        }
        let mut cur = st.cur.and_then(|k| rows.iter().position(|r| r.key == k)).map_or(-1, |p| p as i32);
        if cur >= 0 {
            self.kb_ctx_redirect(id, id.wrapping_add(1).wrapping_add(cur as u32)); // the menu key: the current row
        }
        let vev = self.scr_open(id, SC_V | SCR_F_LIST, HB_FOCUS | HB_OWNKEYS);
        if vev & EV_FOCUS != 0 && n > 0 {
            let mut new = cur;
            if self.key(VK_UP) {
                new = if cur > 0 { cur - 1 } else { 0 };
            }
            if self.key(VK_DOWN) {
                new = if cur < 0 { 0 } else { (cur + 1).min(n - 1) };
            }
            if self.key(VK_HOME) {
                new = 0;
            }
            if self.key(VK_END) {
                new = n - 1;
            }
            if cur >= 0 {
                let r = rows[cur as usize];
                if self.key(VK_RIGHT) {
                    if r.has_children && !st.expanded.contains(&r.key) {
                        st.expanded.insert(r.key);
                        out.expanded = Some(r.key);
                    } else if r.has_children && cur + 1 < n {
                        new = cur + 1;
                    }
                }
                if self.key(VK_LEFT) {
                    if st.expanded.contains(&r.key) {
                        st.expanded.remove(&r.key);
                        out.collapsed = Some(r.key);
                    } else if r.depth > 0 {
                        // to the parent: the nearest row above with a smaller depth
                        if let Some(p) = rows[..cur as usize].iter().rposition(|x| x.depth < r.depth) {
                            new = p as i32;
                        }
                    }
                }
                if self.key(VK_SPACE) {
                    if let Some(c) = r.check {
                        out.check = Some((r.key, if c == 1 { 0 } else { 1 }));
                    }
                }
                if self.key(VK_RETURN) {
                    out.activated = Some(r.key);
                }
            }
            if new != cur && new >= 0 {
                cur = new;
                st.cur = Some(rows[new as usize].key);
                out.sel_changed = true;
                self.wp.ctx.focus_visible = true;
                let (vid, view) = self.scr_view();
                let s = self.slot(vid);
                let d = into_view(new * rh, rh, s.scrolly, view[3]);
                s.scrolly += d;
                s.adur = s.scrolly as u32;
            }
            if out.expanded.is_some() || out.collapsed.is_some() || out.check.is_some() || out.sel_changed {
                self.repass();
            }
        }
        self.scr_offset();
        let y0 = self.box_rect()[1];
        let k = self.wp.ctx.uiclip;
        let first = (k[1].wrapping_sub(y0).max(0) / rh).min(n);
        let end = ((k[3].wrapping_sub(y0).max(0) + rh - 1) / rh).min(n).max(first);
        if first != 0 {
            self.place(0, first * rh);
        }
        let focus_visible = vev & EV_FOCUS != 0 && self.wp.ctx.focus_visible;
        let indent = self.dp(18);
        let tw = self.dp(20);
        for i in first..end {
            let row = rows[i as usize];
            let r = self.place(0, rh);
            let iid = id.wrapping_add(1).wrapping_add(i as u32);
            let ev = self.behave(iid, 0);
            let c = &mut self.wp.ctx;
            if c.active == iid && c.pressed & (1 << BTN_LEFT) != 0 {
                c.focus = id;
                c.focus_visible = false;
            }
            if ev & (EV_ACT | EV_RCLICK) != 0 && cur != i {
                cur = i;
                st.cur = Some(row.key);
                out.sel_changed = true;
                self.repass();
            }
            if ev & EV_DOUBLE != 0 {
                if row.has_children {
                    if !st.expanded.remove(&row.key) {
                        st.expanded.insert(row.key);
                        out.expanded = Some(row.key);
                    } else {
                        out.collapsed = Some(row.key);
                    }
                    self.repass();
                }
                out.activated = Some(row.key);
            }
            let sel = cur == i;
            let mut sp = Sp { ev, value: sel as i32, value2: i, flags: if sel && focus_visible { GR_FOCUS } else { 0 }, ..Sp::default() };
            sp.place(r);
            sd.grid_row(self, &sp);
            if sel {
                self.wp.ctx.last_state |= SS_SELECTED as u32;
            }
            self.sem(K_LISTITEM, iid, Some(row.label), i);
            // content: indentation, chevron, check box, icon, label, detail
            let mut x = r[0] + self.dp(8) + indent * row.depth as i32;
            if row.has_children {
                let cr = [x, r[1] + ((r[3] - tw) >> 1), tw, tw];
                let tid = iid | 0x4000_0000;
                let tev = self.hit_rect(tid, cr[0], cr[1], cr[2], cr[3], 0);
                let open = st.expanded.contains(&row.key);
                if tev & EV_ACT != 0 {
                    if open {
                        st.expanded.remove(&row.key);
                        out.collapsed = Some(row.key);
                    } else {
                        st.expanded.insert(row.key);
                        out.expanded = Some(row.key);
                    }
                    self.repass();
                }
                let mut tsp = Sp { ev: tev, value: open as i32, ..Sp::default() };
                tsp.place(cr);
                sd.tree_twisty(self, &tsp);
                self.wp.ctx.last_rect = cr;
                self.sem(K_BUTTON, tid, Some(crate::i18n::tr(if open { "Zuklappen" } else { "Aufklappen" })), open as i32);
            }
            x += tw + self.dp(4);
            if let Some(cv) = row.check {
                let st_ = self.style;
                let mut csp = Sp::default();
                let (cw, ch) = st_.checkbox_size(self, &mut csp);
                let cw = cw.min(self.dp(20)).max(1);
                let cr = [x, r[1] + ((r[3] - ch) >> 1), cw, ch];
                let cid = iid | 0x2000_0000;
                let cev = self.hit_rect(cid, cr[0], cr[1], cr[2], cr[3], 0);
                if cev & EV_ACT != 0 {
                    out.check = Some((row.key, if cv == 1 { 0 } else { 1 }));
                    self.repass();
                }
                self.sp_placed(&mut csp, cev);
                csp.value = cv as i32;
                csp.label = None;
                st_.checkbox(self, &csp);
                self.wp.ctx.last_rect = cr;
                if cv != 0 {
                    self.wp.ctx.last_state |= SS_CHECKED as u32;
                }
                self.sem(K_CHECKBOX, cid, Some(row.label), cv as i32);
                x += cw + self.dp(8);
            }
            if row.icon != 0 {
                let px = self.dp(16);
                let c = self.color(T_TEXT2);
                crate::stock::icon_in(self, row.icon, [x, r[1], px, r[3]], px, c);
                x += px + self.dp(8);
            }
            let end_x = r[0] + r[2] - self.dp(12);
            let mut label_end = end_x;
            if let Some(dt) = row.detail {
                self.ts_font(TS_CAPTION);
                let dw = self.text.width(dt);
                let dx = end_x - dw;
                let c = self.color(T_TEXT2);
                crate::stock::text_at(self, TS_CAPTION, dx, r, dt, c, 0);
                label_end = dx - self.dp(12);
            }
            let c = self.color(T_TEXT);
            self.ts_font(TS_BODY);
            let full = self.text.width(row.label);
            let maxw = (label_end - x).max(1);
            if full > maxw {
                let (e, _) = self.ellipsize(row.label.as_bytes(), maxw);
                crate::stock::text_at(self, TS_BODY, x, r, &e, c, 0);
            } else {
                crate::stock::text_at(self, TS_BODY, x, r, row.label, c, 0);
            }
        }
        if n - end > 0 {
            self.place(0, (n - end) * rh);
        }
        self.scr_close();
        let mut stt = 0;
        if vev & EV_HOVER != 0 {
            stt |= SS_HOVER as u32;
        }
        if vev & EV_FOCUS != 0 {
            stt |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_state = stt;
        self.sem(K_LIST, id, None, cur).max = n;
        out
    }

    // ------------------------------------------------------------------ badges, dots, meters

    /// ui_badge(kind, label): a status pill (BADGE_*; next_icon puts an icon before the text);
    /// placed narrower than the pill (a grid cell), the label is cut with "…"
    pub fn badge(&mut self, kind: u32, label: &str) {
        let label = crate::basic::cstr(label);
        let icon = self.take_icon();
        self.take_flags();
        let sd = self.style.data();
        let mut short = String::new();
        let mut sp = Sp { flags: kind & 7, icon, label: Some(label), ..Sp::default() };
        let (mut w, h) = sd.badge_size(self, &mut sp);
        let r = self.place(w, h);
        // too narrow: the label cut to the room the pill's padding leaves (measured again by
        // the style, whose font may differ a little)
        if w > r[2] {
            self.ts_font(TS_CAPTION);
            let mut room = r[2] - (w - self.text.width(label));
            for _ in 0..4 {
                self.ts_font(TS_CAPTION);
                short = self.ellipsize(label.as_bytes(), room).0;
                let mut t = Sp { label: Some(&short), ..sp };
                let (sw, _) = sd.badge_size(self, &mut t);
                w = sw;
                if sw <= r[2] || short == "…" {
                    break;
                }
                room -= sw - r[2];
            }
            sp.label = Some(&short);
            sd.badge_size(self, &mut sp);
        }
        // keep the pill's own size inside a stretched cell
        let (bw, bh) = (w.min(r[2]), h.min(r[3]));
        sp.place([r[0], r[1] + ((r[3] - bh) >> 1), bw, bh]);
        sd.badge(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(label), kind as i32);
    }

    /// ui_status_dot(kind): a 10 DIP status dot (BADGE_*)
    pub fn status_dot(&mut self, kind: u32) {
        let s = self.dp(10);
        let r = self.place(s, s);
        let sd = self.style.data();
        let mut sp = Sp { flags: kind & 7, ..Sp::default() };
        sp.place([r[0] + ((r[2] - s) >> 1), r[1] + ((r[3] - s) >> 1), s, s]);
        sd.status_dot(self, &sp);
    }

    /// ui_meter(id, v, max, kind): a capacity bar (natural 120 x 8 DIP; next_size)
    pub fn meter(&mut self, id: u32, v: i64, max: i64, kind: u32) {
        let r = self.place(self.dp(120), self.dp(8));
        // scale 64-bit values (bytes) into the i32 of the record
        let (mut a, mut b) = (v.max(0), max.max(0));
        while b > i32::MAX as i64 {
            a >>= 4;
            b >>= 4;
        }
        let sd = self.style.data();
        let mut sp = Sp { flags: kind & 7, value: a as i32, value2: b as i32, ..Sp::default() };
        sp.place(r);
        sd.meter(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_PROGRESS, id, None, a as i32).max = b as i32;
    }
}
