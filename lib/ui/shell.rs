// shell -- the parts of an application shell: navigation pane, command bar, splitter,
// breadcrumb, segmented control, wizard step list
//
//   nav_begin(id, &mut NavState)        vertical navigation pane (next_size sets its width;
//                                       the style's surface), items scroll when they do not fit
//   nav_toggle(id, label)               the pane's menu button: toggles NavState::compact
//   nav_section(label)                  a group caption (compact: a divider)
//   nav_item(id, icon, label, selected, badge) -> EV   EV_ACT when clicked; badge: a count
//   nav_footer()                        the items after it stay at the bottom of the pane
//   nav_end()
//   command_bar(id, items) -> Option<id>   a row of subtle icon + label buttons; what does not
//                                       fit goes into a "…" menu; CBI_END starts the group
//                                       aligned to the end, CBI_SEP a separator; ST_ACCENT /
//                                       ST_DISABLED / ST_CHECKED per item
//   split_begin(id, pos, vertical)      two panes: the first `pos` DIP wide (vertical: high),
//   split_next()                        a draggable grip, the second fills
//   split_end(&mut pos) -> changed      the dragged position (DIP)
//   breadcrumb(id, parts) -> Option<i>  path of links, the last part is the current page
//   segmented(id, &mut sel, items) -> EV   a segmented control (EV_ACT when sel changed)
//   wizard_steps(id, steps, current, states) -> Option<i>   vertical step list; a click on
//                                       a done step (or one before the current) returns it
//
// Ids: parts (id << 3 | k) | 0x80000000; items of segmented / wizard / breadcrumb id + 1 + i.

use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::{Sp, Ui};

/// the app-owned state of a navigation pane
#[derive(Clone, Copy, Default, Debug)]
pub struct NavState {
    /// icons only
    pub compact: bool,
}

/// an item of a command bar
#[derive(Clone, Copy, Debug)]
pub struct CmdItem<'a> {
    pub id: u32,
    pub icon: u32,
    pub label: &'a str,
    /// ST_ACCENT, ST_DISABLED, ST_CHECKED, CBI_SEP, CBI_END
    pub flags: u32,
}

impl<'a> CmdItem<'a> {
    pub const fn new(id: u32, icon: u32, label: &'a str) -> CmdItem<'a> {
        CmdItem { id, icon, label, flags: 0 }
    }
    /// add flags (ST_ACCENT, CBI_SEP, ...; in any order with enabled())
    pub const fn flags(mut self, f: u32) -> CmdItem<'a> {
        self.flags |= f;
        self
    }
    /// disabled unless `on`
    pub const fn enabled(mut self, on: bool) -> CmdItem<'a> {
        if !on {
            self.flags |= ST_DISABLED;
        }
        self
    }
}

#[derive(Clone, Copy, Default)]
struct NavFrame {
    compact: bool,
    scrolled: bool,
}

#[derive(Clone, Copy, Default)]
struct SplitFrame {
    id: u32,
    pos: i32,
    vertical: bool,
    rect: [i32; 4],
    changed: bool,
}

/// shared state: the open navigation panes and splitters
#[derive(Default)]
pub(crate) struct S {
    nav: Vec<NavFrame>,
    split: Vec<SplitFrame>,
}

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

impl Ui {
    // ------------------------------------------------------------------ navigation pane

    /// ui_nav_begin(id, &mut nav): open the pane (always call nav_end)
    pub fn nav_begin(&mut self, id: u32, nav: &mut NavState) {
        self.box_begin(id, BOX_V | BOX_CLIP | BOX_BLOCK, 0);
        let r = self.box_rect();
        let sh = self.style.shell();
        let mut sp = Sp { flags: nav.compact as u32, ..Sp::default() };
        sp.place(r);
        sh.nav_pane(self, &sp);
        self.next_size(-1, -1);
        self.scroll_begin(part(id, 1), SC_V | pad(4));
        self.m.shell.nav.push(NavFrame { compact: nav.compact, scrolled: true });
    }

    fn nav_top(&self) -> NavFrame {
        self.m.shell.nav.last().copied().unwrap_or_default()
    }

    /// ui_nav_toggle(id, label): the menu button of the pane (toggles compact)
    pub fn nav_toggle(&mut self, id: u32, label: &str, nav: &mut NavState) -> u32 {
        self.next_icon(IC_MENU);
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
        self.next_size(40, 36);
        let ev = self.button(id, label);
        if ev & EV_ACT != 0 {
            nav.compact = !nav.compact;
            self.repass();
        }
        ev
    }

    /// ui_nav_section(label): a group caption (a divider when the pane is compact)
    pub fn nav_section(&mut self, label: &str) {
        let f = self.nav_top();
        let sh = self.style.shell();
        let mut sp = Sp { flags: if f.compact { NAV_COMPACT } else { 0 }, label: Some(label), ..Sp::default() };
        let (w, h) = sh.nav_section_size(self, &mut sp);
        self.next_size(-1, 0);
        let r = self.place(w, h);
        sp.place(r);
        sh.nav_section(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(label), 0);
    }

    /// ui_nav_item(id, icon, label, selected, badge) -> EV_* (EV_ACT clicked)
    pub fn nav_item(&mut self, id: u32, icon: u32, label: &str, selected: bool, badge: Option<&str>) -> u32 {
        let f = self.nav_top();
        let flags = self.take_flags();
        let sh = self.style.shell();
        let mut sp = Sp {
            flags: if f.compact { NAV_COMPACT } else { 0 },
            icon,
            label: Some(label),
            label2: badge,
            value: selected as i32,
            ..Sp::default()
        };
        let (w, h) = sh.nav_item_size(self, &mut sp);
        self.next_size(-1, 0);
        self.place(w, h);
        let hb = HB_FOCUS | if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev = self.behave(id, hb);
        self.sp_placed(&mut sp, ev);
        sh.nav_item(self, &sp);
        self.focus_ring_draw(ev);
        if selected {
            self.wp.ctx.last_state |= SS_SELECTED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), selected as i32);
        if f.compact {
            self.tooltip(label);
        }
        ev
    }

    /// ui_nav_footer: end the scrolling item list; the items after it stay at the bottom
    pub fn nav_footer(&mut self) {
        let Some(f) = self.m.shell.nav.last_mut() else { return };
        if f.scrolled {
            f.scrolled = false;
            self.scroll_end();
            self.spacer(4);
        }
    }

    /// ui_nav_end
    pub fn nav_end(&mut self) {
        let Some(f) = self.m.shell.nav.pop() else { return };
        if f.scrolled {
            self.scroll_end();
        } else {
            self.spacer(4);
        }
        self.box_end();
    }

    // ------------------------------------------------------------------ command bar

    /// ui_command_bar(id, items) -> the id of the item clicked in this pass
    pub fn command_bar(&mut self, id: u32, items: &[CmdItem]) -> Option<u32> {
        self.box_begin(id, BOX_H | BOX_XCENTER, 4);
        let r = self.box_rect();
        let sh = self.style.shell();
        let mut sp = Sp::default();
        sp.place(r);
        sh.command_bar(self, &sp);
        let st = self.style;
        // widths of the items as buttons, the "…" button
        let mut widths = Vec::with_capacity(items.len());
        for it in items {
            let mut f = ST_SUBTLE;
            if it.flags & ST_ACCENT != 0 {
                f = ST_ACCENT;
            }
            let mut sp = Sp { flags: f, icon: it.icon, label: Some(it.label), ..Sp::default() };
            let (w, _) = st.button_size(self, &mut sp);
            widths.push(w);
        }
        let mut more_sp = Sp { flags: ST_SUBTLE | ST_ICON_ONLY, icon: IC_MORE, ..Sp::default() };
        let (more_w, _) = st.button_size(self, &mut more_sp);
        let gap = self.dp(4);
        let sep = self.dp(9);
        let avail = r[2];
        // which items fit (in order; the end group competes the same way)
        let total: i32 = widths.iter().sum::<i32>()
            + gap * items.len() as i32
            + items.iter().filter(|i| i.flags & CBI_SEP != 0).count() as i32 * sep;
        let mut shown = alloc::vec![true; items.len()];
        if total > avail {
            let mut used = more_w + gap;
            for (i, it) in items.iter().enumerate() {
                let w = widths[i] + gap + if it.flags & CBI_SEP != 0 { sep } else { 0 };
                if used + w <= avail {
                    used += w;
                } else {
                    shown[i] = false;
                }
            }
        }
        let mut clicked = None;
        let mut ended = false;
        for (i, it) in items.iter().enumerate() {
            if !shown[i] {
                continue;
            }
            if it.flags & CBI_END != 0 && !ended {
                ended = true;
                self.spacer(-1);
            } else if it.flags & CBI_SEP != 0 {
                self.separator();
            }
            self.next_icon(it.icon);
            let mut f = it.flags & (ST_DISABLED | ST_CHECKED);
            f |= if it.flags & ST_ACCENT != 0 { ST_ACCENT } else { ST_SUBTLE };
            self.next_flags(f);
            if self.button(it.id, it.label) & EV_ACT != 0 {
                clicked = Some(it.id);
            }
        }
        if shown.iter().any(|s| !s) {
            if !ended {
                self.spacer(-1);
            }
            self.next_icon(IC_MORE);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
            let mid = part(id, 1);
            self.dropdown_button(mid, crate::i18n::tr("Weitere Befehle"));
            if self.flyout_begin(mid, FL_ALIGN_END) {
                for (i, it) in items.iter().enumerate() {
                    if shown[i] {
                        continue;
                    }
                    self.next_icon(it.icon);
                    self.next_flags(it.flags & (ST_DISABLED | ST_CHECKED));
                    if self.menu_item(it.id, it.label) & EV_ACT != 0 {
                        clicked = Some(it.id);
                    }
                }
            }
            self.flyout_end();
        }
        self.box_end();
        clicked
    }

    // ------------------------------------------------------------------ splitter

    /// ui_split_begin(id, pos, vertical): two panes, the first `pos` DIP wide (vertical:
    /// high); emit the first pane, split_next, the second pane, split_end
    pub fn split_begin(&mut self, id: u32, pos: i32, vertical: bool) {
        // the split and its first pane get their size from outside, never from the content
        self.box_begin(part(id, 0), BOX_FIXED | if vertical { BOX_V } else { BOX_H }, 0);
        let rect = self.box_rect();
        let total = self.px2dip(if vertical { rect[3] } else { rect[2] });
        let grip = 6;
        let mut p = pos.max(80);
        if total > 0 {
            p = p.min((total - grip - 80).max(80));
        }
        if vertical {
            self.next_size(-1, p);
        } else {
            self.next_size(p, -1);
        }
        self.box_begin(part(id, 1), BOX_V | BOX_CLIP | BOX_FIXED, 0);
        self.focus_pane_begin(part(id, 1)); // F6 visits both sides (kbnav.rs)
        self.m.shell.split.push(SplitFrame { id, pos: p, vertical, rect, changed: false });
    }

    /// ui_split_next: the grip, then the second pane
    pub fn split_next(&mut self) {
        let Some(mut f) = self.m.shell.split.pop() else { return };
        self.focus_pane_end();
        self.box_end();
        let g = 6;
        if f.vertical {
            self.next_size(-1, g);
        } else {
            self.next_size(g, -1);
        }
        let r = self.place(self.dp(g), self.dp(g));
        let cur = if f.vertical { CUR_SIZE_NS } else { CUR_SIZE_WE };
        let ev = self.behave(part(f.id, 2), HB_SPLITTER | cur << HB_CURSOR_SHIFT);
        // the keyboard (F8 reaches the grip): arrows move it
        let d = self.kb_grip(ev, !f.vertical);
        if d != 0 {
            let total = self.px2dip(if f.vertical { f.rect[3] } else { f.rect[2] });
            let p = f.pos.saturating_add(d).clamp(80, (total - g - 80).max(80));
            if p != f.pos {
                f.pos = p;
                f.changed = true;
                self.repass();
            }
        }
        if ev & EV_HELD != 0 {
            let (mx, my) = self.mouse();
            let p = if f.vertical { my - f.rect[1] } else { mx - f.rect[0] } - (self.dp(g) >> 1);
            let total = self.px2dip(if f.vertical { f.rect[3] } else { f.rect[2] });
            let p = self.px2dip(p).clamp(80, (total - g - 80).max(80));
            if p != f.pos {
                f.pos = p;
                f.changed = true;
                self.repass();
            }
        }
        let sh = self.style.shell();
        let mut sp = Sp { ev, flags: f.vertical as u32, ..Sp::default() };
        sp.place(r);
        sh.split_grip(self, &sp);
        if self.focus_ring(ev) {
            let st = self.style;
            st.focus_ring(self, r, 0);
        }
        // the record: a range (the first pane's size in DIP of the whole)
        let total = self.px2dip(if f.vertical { f.rect[3] } else { f.rect[2] });
        let rec = self.sem(K_SLIDER, part(f.id, 2), Some(crate::i18n::tr("Teiler")), f.pos);
        rec.max = total;
        self.next_size(-1, -1);
        self.box_begin(part(f.id, 3), BOX_V | BOX_CLIP | BOX_FIXED, 0);
        self.focus_pane_begin(part(f.id, 3));
        self.m.shell.split.push(f);
    }

    /// ui_split_end(&mut pos) -> the grip moved (pos updated, DIP; the mouse or the keyboard:
    /// F8 focuses the grip, arrows move it)
    pub fn split_end(&mut self, pos: &mut i32) -> bool {
        let Some(f) = self.m.shell.split.pop() else { return false };
        self.focus_pane_end();
        self.box_end();
        self.box_end();
        if f.changed {
            *pos = f.pos;
        }
        f.changed
    }

    // ------------------------------------------------------------------ breadcrumb

    /// ui_breadcrumb(id, parts) -> the index of a clicked part (the last one is the current
    /// page and not a link)
    pub fn breadcrumb(&mut self, id: u32, parts: &[&str]) -> Option<usize> {
        self.box_begin(0, BOX_H | BOX_XCENTER, 2);
        let mut out = None;
        let n = parts.len();
        for (i, p) in parts.iter().enumerate() {
            if i + 1 < n {
                self.next_flags(ST_SUBTLE | ST_COMPACT);
                if self.label_button(id.wrapping_add(1 + i as u32), p, TS_BODY | TX_SECONDARY, 6) & EV_ACT != 0 {
                    out = Some(i);
                }
                let s = self.dp(16);
                let r = self.place(s, s);
                let sh = self.style.shell();
                let mut sp = Sp::default();
                sp.place(r);
                sh.breadcrumb_sep(self, &sp);
            } else {
                self.text(TS_BODY_STRONG, p);
            }
        }
        self.box_end();
        out
    }

    // ------------------------------------------------------------------ segmented control

    /// ui_segmented(id, &mut sel, items) -> EV_* (EV_ACT when sel changed)
    pub fn segmented(&mut self, id: u32, sel: &mut i32, items: &[&str]) -> u32 {
        let flags = self.take_flags();
        let old = *sel;
        let pad = self.dp(14);
        self.ts_font(TS_BODY_STRONG);
        let ws: Vec<i32> = items.iter().map(|s| self.text.width(s) + pad * 2).collect();
        let total: i32 = ws.iter().sum();
        let h = self.dp(if flags & ST_COMPACT != 0 { 28 } else { 32 });
        let r = self.place(total, h);
        let disabled = if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev = self.behave(id, HB_FOCUS | disabled);
        let state = self.wp.ctx.last_state;
        let n = items.len() as i32;
        if ev & EV_FOCUS != 0 && n > 0 {
            let mut s = *sel;
            if self.key(VK_LEFT) {
                s -= 1;
            }
            if self.key(VK_RIGHT) {
                s += 1;
            }
            s = s.clamp(0, n - 1);
            if s != *sel {
                *sel = s;
                self.wp.ctx.focus_visible = true;
                self.repass();
            }
        }
        let sh = self.style.shell();
        let mut tsp = Sp { value2: 1, ..Sp::default() };
        tsp.place(r);
        sh.segment(self, &tsp);
        // item widths scaled to the placed width
        let scale = |w: i32| if total > 0 { (w as i64 * r[2] as i64 / total as i64) as i32 } else { w };
        let mut x = r[0];
        for (i, label) in items.iter().enumerate() {
            let w = if i + 1 == items.len() { r[0] + r[2] - x } else { scale(ws[i]) };
            let ir = [x, r[1], w, r[3]];
            let iid = id.wrapping_add(1 + i as u32);
            let iev = self.hit_rect(iid, ir[0], ir[1], ir[2], ir[3], disabled);
            if iev & EV_ACT != 0 && *sel != i as i32 {
                *sel = i as i32;
                self.wp.ctx.focus = id;
                self.wp.ctx.focus_visible = false;
                self.repass();
            }
            let mut f = 0;
            if i == 0 {
                f |= SEG_FIRST;
            }
            if i + 1 == items.len() {
                f |= SEG_LAST;
            }
            let mut sp = Sp { ev: iev, value: (*sel == i as i32) as i32, flags: f, label: Some(label), ..Sp::default() };
            sp.place(ir);
            sp.state = if disabled != 0 { 3 } else { 0 };
            sh.segment(self, &sp);
            if *sel == i as i32 {
                self.wp.ctx.last_state |= SS_SELECTED as u32;
            }
            self.sem(K_TAB, iid, Some(label), i as i32);
            x += w;
        }
        if ev & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
            let st = self.style;
            st.focus_ring(self, r, 4);
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state;
        self.sem(K_BOX, id, None, *sel).max = n;
        if *sel != old { (ev & !EV_ACT) | EV_ACT } else { ev & !EV_ACT }
    }

    // ------------------------------------------------------------------ wizard steps

    /// ui_wizard_steps(id, steps, current, states) -> a step to go back to (clicked);
    /// states: WS_* per step (missing: done before the current, todo after it)
    pub fn wizard_steps(&mut self, id: u32, steps: &[&str], current: usize, states: &[i32]) -> Option<usize> {
        let mut out = None;
        let sh = self.style.shell();
        let h = self.dp(48);
        for (i, label) in steps.iter().enumerate() {
            let s = states.get(i).copied().unwrap_or(if i < current {
                WS_DONE
            } else if i == current {
                WS_CURRENT
            } else {
                WS_TODO
            });
            let s = if i == current { WS_CURRENT } else { s };
            self.next_size(-1, 0);
            let r = self.place(self.dp(200), h);
            let can = i < current || (s == WS_DONE && i != current);
            let iid = id.wrapping_add(1 + i as u32);
            let ev = self.behave(iid, if can { HB_FOCUS } else { HB_DISABLED });
            if ev & EV_ACT != 0 && can {
                out = Some(i);
            }
            let mut sp = Sp {
                ev,
                value: i as i32 + 1,
                value2: s,
                label: Some(label),
                flags: (i + 1 < steps.len()) as u32,
                ..Sp::default()
            };
            sp.place(r);
            sh.wizard_step(self, &sp);
            self.focus_ring_draw(ev);
            if i == current {
                self.wp.ctx.last_state |= SS_SELECTED as u32;
            }
            self.sem(K_TAB, iid, Some(label), s);
        }
        out
    }

    /// ui_page_header(title, subtitle): a page title (TS_TITLE) with an optional secondary
    /// line under it
    pub fn page_header(&mut self, title: &str, subtitle: &str) {
        self.box_begin(0, BOX_V, 2);
        self.text(TS_TITLE, title);
        if !subtitle.is_empty() {
            self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, subtitle);
        }
        self.box_end();
    }

    /// ui_key_value(key, value, key_width): a "label: value" row (TS_BODY, the key in the
    /// secondary colour, key_width DIP)
    pub fn key_value(&mut self, key: &str, value: &str, key_width: i32) {
        self.box_begin(0, BOX_H | BOX_XCENTER, 12);
        self.next_size(key_width, 0);
        self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, key);
        self.next_size(-1, 0);
        self.text(TS_BODY | TX_ELLIPSIS, value);
        self.box_end();
    }

    /// ui_empty_state(icon, title, text): a centred hint for an empty view (icon 0 = none);
    /// the text wraps at 360 DIP, or at the enclosing box's width when that is narrower
    pub fn empty_state(&mut self, icon: u32, title: &str, text: &str) {
        // the text wraps at 360 DIP, narrower in a narrow panel (the enclosing box's width)
        let avail = self.px2dip(self.box_rect()[2]);
        self.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 8);
        if icon != 0 {
            let s = self.dp(40);
            let r = self.place(s, s);
            let c = self.color(T_TEXT3);
            crate::stock::icon_in(self, icon, r, s, c);
        }
        self.text(TS_SUBTITLE | TX_CENTER, title);
        if !text.is_empty() {
            self.next_size(if avail > 80 { avail.min(360) } else { 360 }, 0);
            self.text(TS_BODY | TX_SECONDARY | TX_WRAP | TX_CENTER, text);
        }
        self.box_end();
    }
}
