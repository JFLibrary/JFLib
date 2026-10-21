// kbnav -- keyboard navigation between widgets: the Tab order with focus groups, panes and
// windows (F6), splitters (F8), the menu key on every widget, the focus after its widget went
// away, the first control of a new dialog window
//
// App-facing:
//   focus_group_begin(id) / focus_group_end()   the focusable widgets in between are one Tab
//        stop (a tool bar, a row of segments, a grid of buttons or swatches): the arrows move
//        inside (one row or column: Left / Up the previous, Right / Down the next, wrapping;
//        a grid: the nearest member in the arrow's direction), Home / End the first / last.
//        Tab enters a group at its current member (HB_CURRENT: the checked tool button, the
//        selected swatch), else at the member focused last, else at its first.  Radios with
//        one variable, tool buttons with one variable and the swatches of one box are groups
//        by themselves (radios: the selection follows the arrows).  Text boxes never join.
//   splitter_keys(ev, along_x) -> DIP            the arrows on a focused splitter of the app
//   group_arrived(id) -> bool                    the arrows just focused this member (once):
//        selection-follows-focus widgets (tabs, radios) select it
//   focus_pane_begin(id) / focus_pane_end()     a pane of the window: F6 / Shift+F6 go to the
//        next / previous pane (to the widget focused there last, else its first Tab stop),
//        past the last one on to the next window -- tool windows too, which then get the keys
//        until Esc, F6 or a click elsewhere; Ctrl+F6 / Ctrl+Shift+F6 switch the windows
//        directly.  The document area and every group of a dock area and both sides of a
//        split view are panes by themselves.
// Built in: Tab / Shift+Tab (Ctrl+Tab stays the app's); F8 / Shift+F8 the window's splitters
// (HB_SPLITTER: the arrows move the focused one by 8 DIP, Shift+arrows by 1, Home / End to the
// ends; Enter, Esc or F8 past the last return to the widget focused before); the menu key /
// Shift+F10 that no widget took: a right click of the focused widget (EV_RCLICK, a focused
// list passes it to its selected row; menu_popup then opens at the widget with its first row
// focused); Tab after the focused widget went away goes on from its place; a dialog window
// shown starts with the focus on its first control (the ring appears with the first key).
// An app accelerator on F6 / F8 wins over these keys.
// Tool windows that borrow their owner's keys (win.rs, U3) keep the navigation keys and hand
// the others on (kb_take_rest).

use alloc::vec::Vec;

use super::consts::*;
use super::ctx::Hit;
use super::keys::*;
use super::Ui;

const MEM_MAX: usize = 64; // remembered groups / panes per window
const GRIP_STEP: i32 = 8; // DIP per arrow on a splitter
const MOD3: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;

/// per-window state (ctx::W): the groups and panes open in this pass, the memory of the
/// member / widget focused last in a group / pane, where the focus was among the Tab stops
#[derive(Default)]
pub(crate) struct W {
    groups: Vec<u32>,
    panes: Vec<u32>,
    /// the automatic group of the next focusable box (radio, tool button, swatch)
    next: u32,
    /// > 0: focusable boxes are no Tab stops (the rows of a list that are not selected)
    notab: u32,
    auto_addr: usize,
    auto_serial: u32,
    auto_seen: u64,
    gmem: Vec<(u32, u32)>,
    pmem: Vec<(u32, u32)>,
    /// the focus, the Tab stop after it and the one before it (last pass that had it)
    anchor: [u32; 3],
    /// a dialog window was shown: its first control gets the focus; that control while the
    /// focus stays there (Esc in a text box closes the dialog at once, edit.rs)
    pub(crate) autofocus: bool,
    pub(crate) soft: u32,
    /// F8: the widget focused before the splitters
    split_ret: u32,
}

impl W {
    /// the member focused last in each group (the inspector's Tab order: inspect.rs)
    pub(crate) fn gmem(&self) -> &[(u32, u32)] {
        &self.gmem
    }

    /// a pass of the window begins
    pub(crate) fn pass_begin(&mut self) {
        self.groups.clear();
        self.panes.clear();
        self.next = 0;
        self.notab = 0;
        self.auto_addr = 0;
        self.auto_serial = 0;
        self.auto_seen = 0;
    }

    /// hit_keys(hbflags) -> (group, pane, HB_NOTAB or 0) of a box hit() registers
    pub(crate) fn hit_keys(&mut self, hb: u32) -> (u32, u32, u32) {
        let pane = self.panes.last().copied().unwrap_or(0);
        if hb & HB_FOCUS == 0 {
            return (0, pane, 0);
        }
        let g = if self.next != 0 {
            core::mem::take(&mut self.next)
        } else if hb & HB_TEXT != 0 {
            0
        } else {
            self.groups.last().copied().unwrap_or(0)
        };
        (g, pane, if self.notab != 0 { HB_NOTAB } else { 0 })
    }
}

/// shared state (Mods)
#[derive(Default)]
pub(crate) struct S {
    /// the widget that gets the menu key as a right click in the next pass
    pub(crate) ctx: u32,
    /// the pass that asked for it (a widget that never calls behave lets it expire)
    ctx_set: u32,
    ctx_frame: u32,
    ctx_rect: [i32; 4],
    /// a group member the arrows just focused (a radio selects itself)
    moved: u32,
    /// the keys went to a tool window by F6 / Ctrl+F6 (Esc gives them back)
    pub(crate) tool: bool,
    /// Space on a focused list: the first check box / switch of the selected row acts
    pub(crate) toggle: bool,
}

fn mem_get(m: &[(u32, u32)], k: u32) -> u32 {
    m.iter().find(|e| e.0 == k).map_or(0, |e| e.1)
}

fn mem_put(m: &mut Vec<(u32, u32)>, k: u32, v: u32) {
    if let Some(i) = m.iter().position(|e| e.0 == k) {
        if m[i].1 == v {
            return;
        }
        m.remove(i);
    }
    if m.len() >= MEM_MAX {
        m.remove(0);
    }
    m.push((k, v));
}

// focusable(h): a box the keyboard may focus
fn focusable(h: &Hit) -> bool {
    h.flags as u32 & (HB_FOCUS | HB_DISABLED) == HB_FOCUS
}

/// stops(list, focus, gmem) -> (the indexes of the Tab stops in order, their layer): the
/// focusable boxes of the top layer that has any, without HB_NOTAB, one per focus group (the
/// focused member, else the current one, else the one focused last, else the first)
pub(crate) fn stops(list: &[Hit], focus: u32, gmem: &[(u32, u32)]) -> (Vec<usize>, i32) {
    let tab = |h: &Hit| focusable(h) && h.flags as u32 & HB_NOTAB == 0 && !h.continuation;
    let top = list.iter().filter(|h| tab(h)).map(|h| h.layer as i32).max().unwrap_or(-1);
    let mut out = Vec::new();
    if top < 0 {
        return (out, top);
    }
    let cand = |h: &Hit| tab(h) && h.layer as i32 == top;
    // (group, index, rank): rank 0 focused, 1 current, 2 remembered, 3 first
    let mut gs: Vec<(u32, usize, u8)> = Vec::new();
    for (i, h) in list.iter().enumerate() {
        if !cand(h) || h.group == 0 {
            continue;
        }
        let rank = if focus != 0 && h.id == focus {
            0
        } else if h.flags as u32 & HB_CURRENT != 0 {
            1
        } else if mem_get(gmem, h.group) == h.id {
            2
        } else {
            3
        };
        match gs.iter_mut().find(|e| e.0 == h.group) {
            Some(e) => {
                if rank < e.2 {
                    e.1 = i;
                    e.2 = rank;
                }
            }
            None => gs.push((h.group, i, rank)),
        }
    }
    for (i, h) in list.iter().enumerate() {
        if cand(h) && (h.group == 0 || gs.iter().any(|e| e.0 == h.group && e.1 == i)) {
            out.push(i);
        }
    }
    // the stops of a pane stay together, at the place of its first one (a dock area emits
    // every tab strip before the panels' contents); the window's root keeps the emission order
    let mut first: Vec<(u32, usize)> = Vec::new();
    for (k, &i) in out.iter().enumerate() {
        let p = list[i].pane;
        if p != 0 && !first.iter().any(|e| e.0 == p) {
            first.push((p, k));
        }
    }
    if !first.is_empty() {
        let mut keyed: Vec<(usize, usize, usize)> = out
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let p = list[i].pane;
                (if p == 0 { k } else { first.iter().find(|e| e.0 == p).map_or(k, |e| e.1) }, k, i)
            })
            .collect();
        keyed.sort_unstable();
        out = keyed.into_iter().map(|e| e.2).collect();
    }
    (out, top)
}

// span_gap(a0, a1, b0, b1): the distance between two intervals (0 when they overlap)
fn span_gap(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    if b1 <= a0 {
        a0 - b1
    } else if b0 >= a1 {
        b0 - a1
    } else {
        0
    }
}

// nearest(members, cur, vk): the member nearest to members[cur] in the arrow's direction
fn nearest(ms: &[(u32, [i32; 4])], cur: usize, vk: u32) -> Option<usize> {
    let r = ms[cur].1;
    let (cx, cy) = ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2);
    let mut best: Option<(i64, usize)> = None;
    for (i, m) in ms.iter().enumerate() {
        if i == cur {
            continue;
        }
        let q = m.1;
        let (qx, qy) = ((q[0] + q[2]) / 2, (q[1] + q[3]) / 2);
        let (along, gap) = match vk {
            VK_LEFT => (cx - qx, span_gap(r[1], r[3], q[1], q[3])),
            VK_RIGHT => (qx - cx, span_gap(r[1], r[3], q[1], q[3])),
            VK_UP => (cy - qy, span_gap(r[0], r[2], q[0], q[2])),
            _ => (qy - cy, span_gap(r[0], r[2], q[0], q[2])),
        };
        if along <= 0 {
            continue;
        }
        let score = along as i64 + gap as i64 * 4;
        if best.is_none_or(|b| score < b.0) {
            best = Some((score, i));
        }
    }
    best.map(|b| b.1)
}

// what F6 does in a window
enum PaneMove {
    None,
    Focus(u32),
    Window(bool),
}

impl Ui {
    // ------------------------------------------------------------------ app API

    /// focus_group_begin(id): the focusable widgets up to focus_group_end are one Tab stop,
    /// the arrows move inside (id 0: the current box's id)
    pub fn focus_group_begin(&mut self, id: u32) {
        let k = if id != 0 { id } else { self.box_id().wrapping_mul(0x9E37_79B1) | 1 };
        self.wp.ctx.kb.groups.push(k);
    }

    /// focus_group_end
    pub fn focus_group_end(&mut self) {
        self.wp.ctx.kb.groups.pop();
    }

    /// focus_pane_begin(id): the widgets up to focus_pane_end are a pane (F6 / Shift+F6)
    pub fn focus_pane_begin(&mut self, id: u32) {
        self.wp.ctx.kb.panes.push(id);
    }

    /// focus_pane_end
    pub fn focus_pane_end(&mut self) {
        self.wp.ctx.kb.panes.pop();
    }

    // ------------------------------------------------------------------ for the widgets

    /// kb_auto_group(addr, mine): the next focusable box joins the group of the variable at
    /// addr (radios, tool buttons); a value seen before starts a new group (a variable the
    /// next group reuses).  Inside an explicit group nothing changes.
    pub(crate) fn kb_auto_group(&mut self, addr: usize, mine: i32) {
        let k = &mut self.wp.ctx.kb;
        if !k.groups.is_empty() {
            return;
        }
        let bit = 1u64 << (mine as u32 & 63);
        if addr != k.auto_addr || k.auto_seen & bit != 0 {
            k.auto_addr = addr;
            k.auto_serial = k.auto_serial.wrapping_add(1);
            k.auto_seen = 0;
        }
        k.auto_seen |= bit;
        let h = (addr as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (k.auto_serial as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        k.next = ((h >> 32) as u32) | 0x8000_0001;
    }

    /// kb_box_group(salt): the next focusable box joins the group of the current box
    /// (swatches)
    pub(crate) fn kb_box_group(&mut self, salt: u32) {
        if !self.wp.ctx.kb.groups.is_empty() {
            return;
        }
        let b = self.box_id();
        self.wp.ctx.kb.next = (b.wrapping_mul(0x9E37_79B1) ^ salt) | 0x8000_0001;
    }

    /// kb_chord(vk, mods): vk with exactly these Shift / Ctrl / Alt was pressed in this pass
    /// and not taken yet (consumes it; Caps / Num Lock do not count)
    pub(crate) fn kb_chord(&mut self, vk: u32, mods: u32) -> bool {
        let q = &mut self.wp.ctx.keyq;
        match q.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & MOD3 == mods) {
            Some(i) => {
                q[i] |= 0x8000_0000;
                true
            }
            None => false,
        }
    }

    /// group_arrived(id): the arrow keys of its focus group just moved the focus to widget id
    /// (true once; a widget whose selection follows the focus -- tabs, radios -- selects
    /// itself then)
    pub fn group_arrived(&mut self, id: u32) -> bool {
        self.kb_moved(id)
    }

    /// kb_moved(id): the arrows of its group just focused the widget (once)
    pub(crate) fn kb_moved(&mut self, id: u32) -> bool {
        if id != 0 && self.m.kbnav.moved == id {
            self.m.kbnav.moved = 0;
            return true;
        }
        false
    }

    /// kb_notab(on): the focusable boxes up to kb_notab(false) are no Tab stops
    pub(crate) fn kb_notab(&mut self, on: bool) {
        let k = &mut self.wp.ctx.kb;
        if on {
            k.notab += 1;
        } else if k.notab > 0 {
            k.notab -= 1;
        }
    }

    /// kb_ctx_take -> EV_RCLICK: behave met the widget the menu key was meant for
    pub(crate) fn kb_ctx_take(&mut self) -> u32 {
        let k = &mut self.m.kbnav;
        k.ctx = 0;
        k.ctx_frame = self.frame;
        k.ctx_rect = self.wp.ctx.last_rect;
        EV_RCLICK
    }

    /// kb_ctx_rect -> the rectangle (px) of the widget that got the menu key as a right click
    /// in this pass (menu_popup opens there)
    pub(crate) fn kb_ctx_rect(&self) -> Option<[i32; 4]> {
        let k = &self.m.kbnav;
        if k.ctx_frame != 0 && k.ctx_frame == self.frame { Some(k.ctx_rect) } else { None }
    }

    /// kb_ctx_redirect(from, to): the menu key meant for `from` goes to `to` (a focused list:
    /// its selected row)
    pub(crate) fn kb_ctx_redirect(&mut self, from: u32, to: u32) {
        if self.m.kbnav.ctx == from && from != 0 {
            self.m.kbnav.ctx = to;
        }
    }

    /// splitter_keys(ev, along_x) -> how far (DIP, + right / down) the keys of this pass move a
    /// focused splitter of the app (a box with HB_SPLITTER, its events ev; along_x: it moves
    /// left / right); Home / End: i32::MIN / 2, i32::MAX / 2 (clamp it); Enter / Esc give the
    /// focus back to the widget focused before F8
    pub fn splitter_keys(&mut self, ev: u32, along_x: bool) -> i32 {
        self.kb_grip(ev, along_x)
    }

    // kb_grip(ev, along_x): splitter_keys for the library's splitters
    pub(crate) fn kb_grip(&mut self, ev: u32, along_x: bool) -> i32 {
        if ev & EV_FOCUS == 0 {
            return 0;
        }
        let (dec, inc) = if along_x { (VK_LEFT, VK_RIGHT) } else { (VK_UP, VK_DOWN) };
        let step = if self.mods & MOD_SHIFT != 0 { 1 } else { GRIP_STEP };
        let mut d = 0;
        if self.key(dec) {
            d -= step;
        }
        if self.key(inc) {
            d += step;
        }
        if self.key(VK_HOME) {
            d = i32::MIN / 2;
        }
        if self.key(VK_END) {
            d = i32::MAX / 2;
        }
        if self.key(VK_RETURN) || self.key(VK_ESCAPE) {
            self.kb_split_return();
        } else if d != 0 {
            self.wp.ctx.focus_visible = true;
        }
        d
    }

    /// kb_keeps(k): the live window's own navigation uses this key (a tool window that borrows
    /// its owner's keys hands the others on)
    pub(crate) fn kb_keeps(&self, k: u32) -> bool {
        let vk = k & 0xFFFF;
        let m = (k >> 16) & MOD3;
        match vk {
            VK_TAB => m & (MOD_CTRL | MOD_ALT) == 0,
            VK_F6 => m & MOD_ALT == 0 && self.accel_find(VK_F6, m).is_none(),
            VK_F8 => m & (MOD_CTRL | MOD_ALT) == 0 && self.accel_find(VK_F8, m).is_none() && !self.kb_splitters().is_empty(),
            VK_APPS => self.wp.ctx.focus != 0,
            VK_F10 => m == MOD_SHIFT && self.wp.ctx.focus != 0,
            VK_ESCAPE => m == 0 && self.kb_tool_here(),
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END => m == 0 && self.kb_focus_group() != 0,
            _ => false,
        }
    }

    /// kb_take_rest(cap): the unconsumed keys of this pass except the navigation keys of the
    /// window, consumed (uiw_fwd_take)
    pub(crate) fn kb_take_rest(&mut self, cap: usize) -> Vec<u32> {
        let keep: Vec<bool> = self.wp.ctx.keyq.iter().map(|&k| (k as i32) < 0 || self.kb_keeps(k)).collect();
        let mut out = Vec::new();
        for (i, k) in self.wp.ctx.keyq.iter_mut().enumerate() {
            if keep[i] {
                continue;
            }
            if out.len() >= cap {
                break;
            }
            out.push(*k);
            *k |= 0x8000_0000;
        }
        out
    }

    // ------------------------------------------------------------------ the pass end

    /// kb_nav: at the end of a window's pass (uiw_pass_end): the first navigation key no widget
    /// took, then the memory of groups, panes and the focus' place
    pub(crate) fn kb_nav(&mut self) {
        if self.m.kbnav.ctx != 0 && self.frame.wrapping_sub(self.m.kbnav.ctx_set) > 2 {
            self.m.kbnav.ctx = 0;
        }
        self.kb_autofocus();
        let n = self.wp.ctx.keyq.len();
        for qi in 0..n {
            let k = self.wp.ctx.keyq[qi];
            if (k as i32) < 0 {
                continue;
            }
            if self.kb_key(k) {
                if let Some(e) = self.wp.ctx.keyq.get_mut(qi) {
                    *e |= 0x8000_0000;
                }
                break;
            }
        }
        self.kb_remember();
    }

    // kb_key(k) -> the key was a navigation key and is used
    fn kb_key(&mut self, k: u32) -> bool {
        let vk = k & 0xFFFF;
        let m = (k >> 16) & MOD3;
        let shift = m & MOD_SHIFT != 0;
        match vk {
            VK_TAB if m & (MOD_CTRL | MOD_ALT) == 0 => {
                self.focus_step(shift);
                true
            }
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END if m == 0 => self.kb_group_move(vk),
            VK_F6 if m & MOD_ALT == 0 && self.accel_find(VK_F6, m).is_none() => {
                if m & MOD_CTRL != 0 {
                    self.kb_window_switch(shift);
                } else {
                    self.kb_pane_switch(shift);
                }
                true
            }
            VK_F8 if m & (MOD_CTRL | MOD_ALT) == 0 && self.accel_find(VK_F8, m).is_none() => self.kb_splitter_step(shift),
            VK_APPS => self.kb_ctx_request(),
            VK_F10 if m == MOD_SHIFT => self.kb_ctx_request(),
            VK_ESCAPE if m == 0 => self.kb_escape(),
            _ => false,
        }
    }

    // kb_autofocus: a dialog window's first pass: its first Tab stop gets the focus (no ring)
    fn kb_autofocus(&mut self) {
        if !self.wp.ctx.kb.autofocus {
            return;
        }
        self.wp.ctx.kb.autofocus = false;
        if self.wp.ctx.focus != 0 {
            return;
        }
        let c = &self.wp.ctx;
        let list = &c.hits[c.hit_cur];
        let (st, top) = stops(list, 0, &c.kb.gmem);
        if top != L_BASE as i32 {
            return;
        }
        if let Some(&i) = st.first() {
            let id = list[i].id;
            self.wp.ctx.focus = id;
            self.wp.ctx.focus_visible = false;
            self.wp.ctx.kb.soft = id;
            self.repass = true;
        }
    }

    // kb_remember: the group and pane of the focused widget remember it; its place among the
    // Tab stops (for Tab when it goes away)
    fn kb_remember(&mut self) {
        let c = &mut self.wp.ctx;
        let f = c.focus;
        if f != c.kb.soft {
            c.kb.soft = 0;
        }
        if f == 0 {
            return;
        }
        let list = &c.hits[c.hit_cur];
        let Some(h) = list.iter().find(|h| h.id == f && focusable(h)) else { return };
        if h.group != 0 {
            mem_put(&mut c.kb.gmem, h.group, f);
        }
        mem_put(&mut c.kb.pmem, h.pane, f);
        let (st, _) = stops(list, f, &c.kb.gmem);
        if let Some(pos) = st.iter().position(|&i| list[i].id == f) {
            let n = st.len();
            c.kb.anchor = [f, list[st[(pos + 1) % n]].id, list[st[(pos + n - 1) % n]].id];
        }
    }

    // focus_step(back): Tab / Shift+Tab: the next / previous Tab stop of this pass
    fn focus_step(&mut self, back: bool) {
        let c = &self.wp.ctx;
        let list = &c.hits[c.hit_cur];
        let f = c.focus;
        let (st, _) = stops(list, f, &c.kb.gmem);
        let n = st.len();
        if n == 0 {
            return;
        }
        let t = match st.iter().position(|&i| list[i].id == f) {
            Some(p) => {
                if back {
                    (p + n - 1) % n
                } else {
                    (p + 1) % n
                }
            }
            None => {
                // a focused box that is no Tab stop (a list row's check box, a splitter): from
                // its place
                let at = if f != 0 { list.iter().position(|h| h.id == f && h.flags as u32 & (HB_FOCUS | HB_SPLITTER) != 0) } else { None };
                match at {
                    Some(a) => {
                        if back {
                            st.iter().rposition(|&i| i < a).unwrap_or(n - 1)
                        } else {
                            st.iter().position(|&i| i > a).unwrap_or(0)
                        }
                    }
                    None => {
                        // the focused widget went away: on from the place it had
                        let an = c.kb.anchor;
                        let want = if f != 0 && an[0] == f { if back { an[2] } else { an[1] } } else { 0 };
                        match st.iter().position(|&i| want != 0 && list[i].id == want) {
                            Some(p) => p,
                            None => {
                                if back {
                                    n - 1
                                } else {
                                    0
                                }
                            }
                        }
                    }
                }
            }
        };
        let id = list[st[t]].id;
        self.wp.ctx.focus = id;
        self.wp.ctx.focus_visible = true;
        self.repass = true;
    }

    // kb_focus_group -> the focus group of the focused widget with two members or more (0 none)
    fn kb_focus_group(&self) -> u32 {
        let c = &self.wp.ctx;
        let f = c.focus;
        if f == 0 {
            return 0;
        }
        let list = &c.hits[c.hit_cur];
        let Some(h) = list.iter().find(|h| h.id == f && focusable(h)) else { return 0 };
        if h.group == 0 || h.flags as u32 & HB_TEXT != 0 {
            return 0;
        }
        let g = h.group;
        if list.iter().any(|m| m.group == g && m.id != f && focusable(m)) { g } else { 0 }
    }

    // kb_group_move(vk) -> used: an arrow / Home / End inside the focused widget's group
    fn kb_group_move(&mut self, vk: u32) -> bool {
        let g = self.kb_focus_group();
        if g == 0 {
            return false;
        }
        let c = &self.wp.ctx;
        let f = c.focus;
        let list = &c.hits[c.hit_cur];
        let layer = list.iter().find(|h| h.id == f && focusable(h)).map_or(0, |h| h.layer);
        // the members (the first box of every id)
        let mut ms: Vec<(u32, [i32; 4])> = Vec::new();
        for h in list.iter() {
            if h.group == g && h.layer == layer && focusable(h) && !ms.iter().any(|m| m.0 == h.id) {
                ms.push((h.id, [h.x0, h.y0, h.x1, h.y1]));
            }
        }
        let n = ms.len();
        let Some(cur) = ms.iter().position(|m| m.0 == f) else { return false };
        let t = match vk {
            VK_HOME => 0,
            VK_END => n - 1,
            _ => {
                let r0 = ms[0].1;
                let row = ms.iter().all(|m| span_gap(r0[1], r0[3], m.1[1], m.1[3]) == 0 && m.1[3] > r0[1]);
                let col = ms.iter().all(|m| span_gap(r0[0], r0[2], m.1[0], m.1[2]) == 0 && m.1[2] > r0[0]);
                if row || col {
                    if matches!(vk, VK_RIGHT | VK_DOWN) {
                        (cur + 1) % n
                    } else {
                        (cur + n - 1) % n
                    }
                } else {
                    match nearest(&ms, cur, vk) {
                        Some(t) => t,
                        None => return true, // the edge of a grid: the arrow stays in it
                    }
                }
            }
        };
        let id = ms[t].0;
        if id != f {
            self.wp.ctx.focus = id;
            self.m.kbnav.moved = id;
            self.repass = true;
        }
        self.wp.ctx.focus_visible = true;
        true
    }

    // kb_ctx_request -> used: the menu key / Shift+F10 no widget took: the focused widget gets
    // a right click in the next pass
    fn kb_ctx_request(&mut self) -> bool {
        let f = self.wp.ctx.focus;
        if f == 0 || self.popup_count() != 0 {
            return false;
        }
        let c = &self.wp.ctx;
        if !c.hits[c.hit_cur].iter().any(|h| h.id == f) {
            return false;
        }
        self.m.kbnav.ctx = f;
        self.m.kbnav.ctx_set = self.frame;
        self.repass = true;
        true
    }

    // kb_tool_here: the live window is a tool window that has the keys by F6 / Ctrl+F6
    fn kb_tool_here(&self) -> bool {
        let cur = self.uiw_cur;
        cur != 0 && self.m.kbnav.tool && self.m.win.kfocus == cur + 1
    }

    // kb_escape -> used: Esc no widget took: a splitter gives the focus back; a tool window
    // that got the keys by F6 gives them back to its owner
    fn kb_escape(&mut self) -> bool {
        if self.kb_on_splitter() {
            self.kb_split_return();
            return true;
        }
        if !self.kb_tool_here() {
            return false;
        }
        let cur = self.uiw_cur;
        self.m.win.kfocus = 0;
        self.m.kbnav.tool = false;
        self.wp.ctx.focus_visible = false;
        let o = self.m.win.recs[(cur as usize) & (WIN_MAX - 1)].owner;
        if let Some(p) = self.uiw_parts_mut(o) {
            p.ctx.focus_visible = p.ctx.focus != 0;
        }
        self.repass = true;
        self.invalidate();
        true
    }

    // ------------------------------------------------------------------ panes and windows

    // kb_pane_switch(back): F6 / Shift+F6
    fn kb_pane_switch(&mut self, back: bool) {
        if self.wp.ctx.modal_cur != 0 {
            return; // a content dialog keeps the keyboard
        }
        let others = self.kb_ring().len() > 1;
        let mv = {
            let c = &self.wp.ctx;
            let list = &c.hits[c.hit_cur];
            let f = c.focus;
            let (st, top) = stops(list, f, &c.kb.gmem);
            if top > L_BASE as i32 {
                return;
            }
            // the panes in the order of their first Tab stop
            let mut panes: Vec<u32> = Vec::new();
            for &i in &st {
                if !panes.contains(&list[i].pane) {
                    panes.push(list[i].pane);
                }
            }
            let n = panes.len();
            let cur = list
                .iter()
                .find(|h| f != 0 && h.id == f && h.flags as u32 & (HB_FOCUS | HB_SPLITTER) != 0)
                .and_then(|h| panes.iter().position(|&p| p == h.pane));
            let t = match cur {
                None if n == 0 => None,
                None => Some(if back { n - 1 } else { 0 }),
                Some(ci) if !back && ci + 1 < n => Some(ci + 1),
                Some(ci) if back && ci > 0 => Some(ci - 1),
                Some(_) if others => None,
                Some(ci) => Some(if n == 1 { ci } else if back { n - 1 } else { 0 }),
            };
            match t {
                None if others => PaneMove::Window(back),
                None => PaneMove::None,
                Some(t) => {
                    let p = panes[t];
                    let m = mem_get(&c.kb.pmem, p);
                    let id = if m != 0 && list.iter().any(|h| h.id == m && h.pane == p && focusable(h)) {
                        m
                    } else {
                        st.iter().map(|&i| &list[i]).find(|h| h.pane == p).map_or(0, |h| h.id)
                    };
                    PaneMove::Focus(id)
                }
            }
        };
        match mv {
            PaneMove::Focus(id) if id != 0 => {
                self.wp.ctx.focus = id;
                self.wp.ctx.focus_visible = true;
                self.repass = true;
            }
            PaneMove::Window(back) => self.kb_window_switch(back),
            _ => {}
        }
    }

    // kb_ring -> the windows F6 / Ctrl+F6 visit: the main window and every shown window that
    // is not inert and has a focusable widget, by index
    fn kb_ring(&self) -> Vec<u32> {
        let mut v = Vec::new();
        for i in 0..WIN_MAX as u32 {
            if i != 0 && !self.uiw_drawable(i) || (self.m.win.inert >> i) & 1 != 0 {
                continue;
            }
            let Some(p) = self.uiw_parts(i) else { continue };
            let c = &p.ctx;
            let list = if i == self.uiw_cur { &c.hits[c.hit_cur] } else { &c.hits[c.hit_cur ^ 1] };
            if list.iter().any(focusable) {
                v.push(i);
            }
        }
        v
    }

    // kb_window_switch(back): Ctrl+F6 / Ctrl+Shift+F6 (and F6 past the last pane): the keys go
    // to the next / previous window, to the widget focused there before or its first Tab stop
    fn kb_window_switch(&mut self, back: bool) {
        let ring = self.kb_ring();
        let n = ring.len();
        if n < 2 {
            return;
        }
        let live = self.uiw_cur;
        let ci = ring.iter().position(|&w| w == live).unwrap_or(0);
        let j = if back { ring[(ci + n - 1) % n] } else { ring[(ci + 1) % n] };
        self.wp.ctx.focus_visible = false; // the window without the keys shows no ring
        if let Some(p) = self.uiw_parts_mut(j) {
            let c = &mut p.ctx;
            let nf = {
                let list = if j == live { &c.hits[c.hit_cur] } else { &c.hits[c.hit_cur ^ 1] };
                let f = c.focus;
                if f != 0 && list.iter().any(|h| h.id == f && focusable(h)) {
                    f
                } else {
                    let (st, _) = stops(list, 0, &c.kb.gmem);
                    st.first().map_or(0, |&i| list[i].id)
                }
            };
            c.focus = nf;
            c.focus_visible = nf != 0;
        }
        let r = &self.m.win.recs[(j as usize) & (WIN_MAX - 1)];
        if j != 0 && r.flags & (WF_TOOL | WF_KEYS) == WF_TOOL {
            // a tool window borrows its owner's keys (win.rs uiw_key_target)
            let o = r.owner;
            self.m.win.kfocus = j + 1;
            self.m.kbnav.tool = true;
            self.uiw_activate(o);
        } else {
            self.m.win.kfocus = 0;
            self.m.kbnav.tool = false;
            self.uiw_activate(j);
        }
        self.repass = true;
        self.invalidate();
    }

    // ------------------------------------------------------------------ splitters

    // kb_splitters -> the splitters of this pass (ids, emission order)
    fn kb_splitters(&self) -> Vec<u32> {
        let c = &self.wp.ctx;
        let mut v: Vec<u32> = Vec::new();
        for h in &c.hits[c.hit_cur] {
            if h.flags as u32 & (HB_SPLITTER | HB_DISABLED) == HB_SPLITTER && h.layer as u32 == L_BASE && !v.contains(&h.id) {
                v.push(h.id);
            }
        }
        v
    }

    // kb_on_splitter: the focus is on a splitter
    fn kb_on_splitter(&self) -> bool {
        let c = &self.wp.ctx;
        c.focus != 0 && c.hits[c.hit_cur].iter().any(|h| h.id == c.focus && h.flags as u32 & HB_SPLITTER != 0)
    }

    // kb_splitter_step(back) -> used: F8 / Shift+F8
    fn kb_splitter_step(&mut self, back: bool) -> bool {
        let sp = self.kb_splitters();
        let n = sp.len();
        if n == 0 {
            return false;
        }
        let f = self.wp.ctx.focus;
        let t = match sp.iter().position(|&s| s == f) {
            None => {
                self.wp.ctx.kb.split_ret = f;
                if back { n - 1 } else { 0 }
            }
            Some(p) => {
                let q = if back { p as i32 - 1 } else { p as i32 + 1 };
                if q < 0 || q >= n as i32 {
                    self.kb_split_return();
                    return true;
                }
                q as usize
            }
        };
        self.wp.ctx.focus = sp[t];
        self.wp.ctx.focus_visible = true;
        self.repass = true;
        true
    }

    // kb_split_return: back to the widget focused before F8
    fn kb_split_return(&mut self) {
        let r = core::mem::take(&mut self.wp.ctx.kb.split_ret);
        self.wp.ctx.focus = r;
        self.wp.ctx.focus_visible = r != 0;
        self.repass = true;
    }
}
