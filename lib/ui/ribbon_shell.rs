// ribbon_shell -- the window parts around a ribbon: the register row with its context sets,
// the ribbon's access keys (key tips) and the backstage frame of the File register
//
//   ribbon_tabs(id, &mut sel, tabs) -> EV_*    the register row: an optional File register
//                                     (RibbonTab::file: activating it returns EV_OPEN and
//                                     leaves `sel`), the registers, then the context sets
//                                     (RibbonTab::context: a coloured edge, the set's name in
//                                     the tooltip); EV_ACT when `sel` changed
//   keytip(level, label)              an access key for the last widget, made from its label
//   keytip_keys(level, keys)          ... with fixed keys ("1" for a quick access button)
//   keytips_level() -> 0 / 1 / 2      the key tip mode: off, the registers, the commands
//   backstage_begin(id, back, items, &mut sel) -> Option<i32>   the File register's page:
//                                     a navigation pane (a back item, the items, BS_FOOTER
//                                     items at its bottom) and a scrolling content area the
//                                     app fills; Some(-1): back (or Esc), Some(i): item i was
//                                     activated (a page item also selects itself)
//   backstage_end()
//
// Key tips (Office's access keys): Alt pressed and released alone, or F10, shows a badge with
// keys on every register (and on what the app marks with keytip at level 1, its quick access
// buttons); typing a register's keys shows its ribbon with a badge on every command (level 2:
// the ribbon's buttons register themselves between ribbon_begin and ribbon_end, other controls
// through keytip(2, label)); typing a command's keys activates it in the next pass as a click
// would.  Esc steps back a level, a click or Alt ends the mode.  Keys made from labels take the
// first free letter of the label (accents dropped), then "0A".."0Z" (never a single "0", so no
// key is the start of another); they follow the labels of the current language.
//
// Ids: a register is id + 1 + i (its record is a tab, K_TAB, so scripts click tab:"Start");
// the backstage's back item id + 1, its items id + 2 + i, its content view id + 0x40.

use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::{NavState, Sp, Ui};

/// RibbonTab::flags: the File register (opens the backstage; never selected)
pub const RT_FILE: u32 = 1 << 20;
/// RibbonTab::flags: a register of a context set (shown while the app's selection asks for it)
pub const RT_CONTEXT: u32 = 1 << 21;
/// BackstageItem::flags: an action (Schließen): activating it does not select it
pub const BS_ACTION: u32 = 1 << 20;
/// BackstageItem::flags: placed at the bottom of the pane (Optionen, Info)
pub const BS_FOOTER: u32 = 1 << 21;
/// BackstageItem::flags: a divider before the item
pub const BS_SEP: u32 = 1 << 22;

const MODS: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;
const BTN_MASK: u32 = (1 << BTN_LEFT) | (1 << BTN_RIGHT) | (1 << BTN_MIDDLE);

/// a register of `ribbon_tabs`
#[derive(Clone, Copy, Debug)]
pub struct RibbonTab<'a> {
    /// the visible name (also its record's label)
    pub label: &'a str,
    /// RT_FILE, RT_CONTEXT, ST_DISABLED
    pub flags: u32,
    /// RT_CONTEXT: the name of its context set ("Container-Tools"); tooltip and screen readers
    pub set: &'a str,
    /// RT_CONTEXT: the set's colour (ARGB; 0 = the accent)
    pub color: u32,
}

impl<'a> RibbonTab<'a> {
    /// a fixed register
    pub const fn new(label: &'a str) -> RibbonTab<'a> {
        RibbonTab { label, flags: 0, set: "", color: 0 }
    }
    /// the File register (activating it returns EV_OPEN)
    pub const fn file(label: &'a str) -> RibbonTab<'a> {
        RibbonTab { label, flags: RT_FILE, set: "", color: 0 }
    }
    /// a register of the context set `set` in `color` (ARGB, 0 = the accent)
    pub const fn context(label: &'a str, set: &'a str, color: u32) -> RibbonTab<'a> {
        RibbonTab { label, flags: RT_CONTEXT, set, color }
    }
    /// add flags (ST_DISABLED)
    pub const fn flags(mut self, f: u32) -> RibbonTab<'a> {
        self.flags |= f;
        self
    }
}

/// an item of the backstage's navigation pane
#[derive(Clone, Copy, Debug)]
pub struct BackstageItem<'a> {
    pub label: &'a str,
    pub icon: u32,
    /// BS_ACTION, BS_FOOTER, BS_SEP, ST_DISABLED
    pub flags: u32,
}

impl<'a> BackstageItem<'a> {
    pub const fn new(label: &'a str, icon: u32) -> BackstageItem<'a> {
        BackstageItem { label, icon, flags: 0 }
    }
    /// add flags (BS_ACTION, BS_FOOTER, BS_SEP, ST_DISABLED)
    pub const fn flags(mut self, f: u32) -> BackstageItem<'a> {
        self.flags |= f;
        self
    }
    /// disabled unless `on`
    pub const fn enabled(mut self, on: bool) -> BackstageItem<'a> {
        if !on {
            self.flags |= ST_DISABLED;
        }
        self
    }
}

/// the key tip state of a window (WinParts::rshell)
#[derive(Default)]
pub(crate) struct W {
    /// 0 off, 1 the registers, 2 the commands of the shown ribbon
    level: u8,
    /// Alt went down alone (a tap enters the mode when it comes up)
    armed: bool,
    /// the keys typed at this level (upper case)
    typed: String,
    /// the keys handed out in this pass at the current level
    taken: Vec<String>,
    /// those of the last pass (whether typed keys can still lead somewhere)
    last: Vec<String>,
    /// the widget a key tip activates: behave gives it EV_ACT (ctx.rs) in the next pass only
    pub(crate) fire: u32,
    fire_age: u8,
    /// between ribbon_begin and ribbon_end: the ribbon's buttons register at level 2
    pub(crate) in_ribbon: bool,
}

impl W {
    /// a new pass of the window (ctx.rs uiw_pass_begin)
    pub(crate) fn pass_begin(&mut self) {
        if self.fire != 0 {
            if self.fire_age >= 1 {
                self.fire = 0;
            }
            self.fire_age += 1;
        }
        if self.level != 0 {
            self.last = core::mem::take(&mut self.taken);
        }
        self.in_ribbon = false;
    }
}

// the letter of c for a key tip: ASCII letters and digits upper case, accented Latin letters
// as their base letter, anything else none
fn tip_char(c: char) -> Option<char> {
    let b = match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => c,
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'ç' | 'Ç' => 'C',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ñ' | 'Ñ' => 'N',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'ß' => 'S',
        _ => return None,
    };
    Some(b.to_ascii_uppercase())
}

// the first free key for `label`: a letter of it (the initials of its words first), else
// "0A".."0Z", "0AA".. (never a single "0")
fn tip_for(label: &str, taken: &[String]) -> String {
    let free = |s: &str| !taken.iter().any(|t| t == s);
    let mut cands: Vec<char> = Vec::new();
    for w in label.split([' ', '-', '\n', '/']) {
        if let Some(c) = w.chars().find_map(tip_char) {
            cands.push(c);
        }
    }
    cands.extend(label.chars().filter_map(tip_char));
    for c in cands {
        if c == '0' {
            continue;
        }
        let mut b = [0u8; 4];
        let s = c.encode_utf8(&mut b);
        if free(s) {
            return String::from(&*s);
        }
    }
    for c in b'A'..=b'Z' {
        let s = alloc::format!("0{}", c as char);
        if free(&s) {
            return s;
        }
    }
    let mut n = taken.len();
    loop {
        let s = alloc::format!("0{}{}", (b'A' + (n / 26 % 26) as u8) as char, (b'A' + (n % 26) as u8) as char);
        if free(&s) {
            return s;
        }
        n += 1;
    }
}

impl Ui {
    // ------------------------------------------------------------------ key tips

    /// The key tip mode of the window: 0 off, 1 the registers' keys are shown, 2 the commands'.
    pub fn keytips_level(&self) -> u32 {
        self.wp.rshell.level as u32
    }

    /// End the key tip mode (the app opened something the keys should not reach).
    pub fn keytips_off(&mut self) {
        let k = &mut self.wp.rshell;
        if k.level != 0 {
            k.level = 0;
            k.typed.clear();
            self.repass();
        }
    }

    /// An access key for the last widget at `level` (1: beside the registers, such as a quick
    /// access button; 2: a command of the shown ribbon), made from its label: shown as a
    /// badge while the mode is at that level; typing it activates the widget in the next
    /// pass. The ribbon's own buttons register themselves.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn keytip(&mut self, level: u32, label: &str) {
        let _src = self.src_scope();
        if self.wp.rshell.level as u32 != level || level == 0 {
            return;
        }
        let (id, r) = self.last_widget();
        let off = self.wp.ctx.last_state & SS_DISABLED as u32 != 0;
        let tip = tip_for(label, &self.wp.rshell.taken);
        self.keytip_at(id, r, tip, false, off);
    }

    /// keytip with fixed keys (upper-case letters or digits, at most three).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn keytip_keys(&mut self, level: u32, keys: &str) {
        let _src = self.src_scope();
        if self.wp.rshell.level as u32 != level || level == 0 || keys.is_empty() {
            return;
        }
        let (id, r) = self.last_widget();
        let off = self.wp.ctx.last_state & SS_DISABLED as u32 != 0;
        self.keytip_at(id, r, keys.to_ascii_uppercase(), false, off);
    }

    // (ribbon.rs) a ribbon button's own key tip at level 2
    pub(crate) fn keytip_ribbon(&mut self, id: u32, r: [i32; 4], label: &str, disabled: bool) {
        let k = &self.wp.rshell;
        if k.level != 2 || !k.in_ribbon || self.wp.ctx.cmd_layer_ui != L_BASE as _ || id == 0 {
            return;
        }
        let tip = tip_for(label, &k.taken);
        self.keytip_at(id, r, tip, false, disabled);
    }

    // the badge for `tip` at widget id (rect r) -> the typed keys chose it (the widget fires in
    // the next pass unless `own`: the caller acts itself; a disabled one shows its keys dimmed
    // and does nothing)
    fn keytip_at(&mut self, id: u32, r: [i32; 4], tip: String, own: bool, disabled: bool) -> bool {
        let hit = !disabled && !self.wp.rshell.typed.is_empty() && self.wp.rshell.typed == tip;
        self.keytip_paint(r, &tip, disabled);
        self.wp.rshell.taken.push(tip);
        if !hit {
            return false;
        }
        let k = &mut self.wp.rshell;
        k.typed.clear();
        if !own {
            k.fire = id;
            k.fire_age = 0;
            k.level = 0;
        }
        self.repass();
        true
    }

    // the badge of a key tip below the middle of r (large widgets) or at its left (small ones)
    fn keytip_paint(&mut self, r: [i32; 4], tip: &str, disabled: bool) {
        self.ts_font(TS_CAPTION);
        let h = self.dp(16);
        let w = (self.text.width(tip) + self.dp(8)).max(h);
        let (x, y) = if r[3] > self.dp(36) {
            (r[0] + (r[2] - w) / 2, r[1] + r[3] - h + self.dp(4))
        } else {
            (r[0] + self.dp(4), r[1] + (r[3] - h) / 2 + self.dp(6))
        };
        let prev = self.cmd_layer(L_TIP);
        let mut sp = Sp { label: Some(tip), state: if disabled { 3 } else { 0 }, ..Sp::default() };
        sp.place([x, y, w, h]);
        let st = self.style;
        st.keytip(self, &sp);
        self.cmd_layer(prev);
    }

    // the keys of the mode, once per pass at the register row -> the mode changed
    fn keytips_keys(&mut self) {
        if self.modal_active() != 0 || self.popup_count() != 0 {
            let no_popup = self.popup_count() == 0;
            let k = &mut self.wp.rshell;
            k.armed = false;
            if k.level != 0 && no_popup {
                k.level = 0;
                k.typed.clear();
            }
            return;
        }
        // keys typed that no key of the last pass starts with lead nowhere
        {
            let k = &mut self.wp.rshell;
            if !k.typed.is_empty() && !k.last.iter().any(|t| t.starts_with(k.typed.as_str())) {
                k.typed.clear();
            }
        }
        let first = self.key_first();
        let vk = first & 0xFFFF;
        let allm = first >> 16;
        let mut toggle = false;
        // Alt tap: Alt down and up with nothing in between (as the menu bar's)
        if vk == VK_MENU {
            self.wp.rshell.armed = allm & (MOD_CTRL | MOD_SHIFT | MOD_ALTGR) == 0;
        } else if vk != 0 || self.wp.ctx.pressed & BTN_MASK != 0 {
            self.wp.rshell.armed = false;
        } else if self.wp.rshell.armed && !self.key_down(VK_MENU) {
            self.wp.rshell.armed = false;
            if self.headless || plat::window_state() & plat::WST_ACTIVE != 0 {
                toggle = true;
            }
        }
        if vk == VK_F10 && allm & MODS == 0 {
            self.key(VK_F10);
            toggle = true;
        }
        let k = &mut self.wp.rshell;
        if toggle {
            k.level = if k.level == 0 { 1 } else { 0 };
            k.typed.clear();
            self.repass();
            return;
        }
        if k.level == 0 {
            return;
        }
        // a click anywhere ends the mode
        if self.wp.ctx.pressed & BTN_MASK != 0 {
            k.level = 0;
            k.typed.clear();
            self.repass();
            return;
        }
        if vk == VK_ESCAPE && allm & MODS == 0 {
            self.key(VK_ESCAPE);
            let k = &mut self.wp.rshell;
            k.level -= 1;
            k.typed.clear();
            self.repass();
            return;
        }
        if vk == VK_BACK && allm & MODS == 0 {
            self.key(VK_BACK);
            self.wp.rshell.typed.pop();
            self.repass();
            return;
        }
        let ch = match vk {
            0x30..=0x39 | 0x41..=0x5A if allm & (MOD_CTRL | MOD_ALT) == 0 => vk as u8 as char,
            0x60..=0x69 if allm & (MOD_CTRL | MOD_ALT) == 0 => (b'0' + (vk - 0x60) as u8) as char, // the number pad
            _ => return,
        };
        self.key(vk);
        let k = &mut self.wp.rshell;
        if k.typed.len() < 3 {
            k.typed.push(ch);
        }
        self.repass();
    }

    // ------------------------------------------------------------------ the register row

    /// The register row above a ribbon (see the module header): returns EV_ACT when `sel`
    /// changed, EV_OPEN when the File register was activated (click, Enter, its key tip).
    /// `sel` indexes `tabs` and never names the File register. The row is one focus stop:
    /// Left / Right / Home / End choose among the enabled registers. It also runs the key
    /// tip mode (Alt tap or F10): call it once per pass, before the ribbon. The row is as wide
    /// as its box and as high as the style says (Style::ribbon_tabs_height).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_tabs(&mut self, id: u32, sel: &mut i32, tabs: &[RibbonTab<'_>]) -> u32 {
        let _src = self.src_scope();
        self.keytips_keys();
        let st = self.style;
        let flags = self.take_flags();
        let hd = st.ribbon_tabs_height();
        let h = self.dp(hd);
        // widths: the label and 12 DIP each side; a gap before the first context register
        self.ts_font(TS_BODY);
        let pad = self.dp(12);
        let gap = self.dp(8);
        let mut widths: Vec<i32> = tabs.iter().map(|t| self.text.width(t.label) + pad * 2).collect();
        let full = widths.clone();
        let first_ctx = tabs.iter().position(|t| t.flags & RT_CONTEXT != 0);
        let inset = self.dp(2);
        let natural = widths.iter().sum::<i32>() + inset + if first_ctx.is_some() { gap } else { 0 };
        // the row takes the box's width at the style's height
        self.next_size(-1, hd);
        let row = self.place(self.px2dip(natural), hd);
        let room = row[2] - inset - if first_ctx.is_some() { gap } else { 0 };
        if natural > row[2] && !tabs.is_empty() {
            let min = pad * 2 + self.text.width("…");
            crate::list::tabs_fit(&mut widths, room, min);
        }
        let disabled = if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev0 = self.behave(id, HB_FOCUS | disabled);
        self.wp.ctx.last_state = 0;
        let mut ev = ev0 & (EV_FOCUS | EV_HOVER);
        let usable = |t: &RibbonTab| t.flags & (RT_FILE | ST_DISABLED) == 0;
        // keyboard: the row is one stop
        if ev0 & EV_FOCUS != 0 {
            let order: Vec<usize> = (0..tabs.len()).filter(|&i| usable(&tabs[i])).collect();
            let at = order.iter().position(|&i| i as i32 == *sel);
            let mut to = at;
            if self.key(VK_LEFT) {
                to = Some(at.map_or(0, |a| a.saturating_sub(1)));
            }
            if self.key(VK_RIGHT) {
                to = Some(at.map_or(0, |a| (a + 1).min(order.len().saturating_sub(1))));
            }
            if self.key(VK_HOME) {
                to = Some(0);
            }
            if self.key(VK_END) {
                to = Some(order.len().saturating_sub(1));
            }
            if let Some(&i) = to.and_then(|a| order.get(a)) {
                if i as i32 != *sel {
                    *sel = i as i32;
                    ev |= EV_ACT;
                    self.wp.ctx.focus_visible = true;
                    self.repass();
                }
            }
        }
        // where the registers go
        let mut xs = Vec::with_capacity(tabs.len());
        let mut x = row[0] + inset;
        for (i, w) in widths.iter().enumerate() {
            if Some(i) == first_ctx {
                x += gap;
            }
            xs.push(x);
            x += *w;
        }
        // input first, so the strip knows the register selected in this pass
        let mut tevs = Vec::with_capacity(tabs.len());
        for (i, t) in tabs.iter().enumerate() {
            self.wp.ctx.last_rect = [xs[i], row[1], widths[i], h];
            let off = if t.flags & ST_DISABLED != 0 { HB_DISABLED } else { disabled };
            let tid = id.wrapping_add(1 + i as u32);
            let tev = self.behave(tid, off);
            if tev & EV_ACT != 0 {
                self.wp.ctx.focus = id;
                self.wp.ctx.focus_visible = false;
                if t.flags & RT_FILE != 0 {
                    ev |= EV_OPEN;
                    self.keytips_off();
                } else if *sel != i as i32 {
                    *sel = i as i32;
                    ev |= EV_ACT;
                    self.repass();
                }
            }
            tevs.push((tev, self.wp.ctx.last_state));
        }
        // the register shown (never File); the strip leaves it open toward the ribbon
        let shown = (*sel >= 0 && (*sel as usize) < tabs.len() && usable(&tabs[*sel as usize])).then_some(*sel as usize);
        let mut sp = Sp::default();
        sp.place(row);
        if let Some(i) = shown {
            sp.s = [xs[i], widths[i], 0, 0];
        }
        st.ribbon_tabs_strip(self, &sp);
        // the registers
        for (i, t) in tabs.iter().enumerate() {
            let r = [xs[i], row[1], widths[i], h];
            let (tev, state) = tevs[i];
            let short;
            let label = if widths[i] < full[i] {
                self.ts_font(TS_BODY);
                short = self.ellipsize(t.label.as_bytes(), widths[i] - pad * 2).0;
                short.as_str()
            } else {
                t.label
            };
            let mut tsp = Sp {
                flags: t.flags & (RT_FILE | RT_CONTEXT),
                label: Some(label),
                value: (shown == Some(i)) as i32,
                value2: if t.flags & RT_CONTEXT != 0 { if t.color != 0 { t.color as i32 } else { self.color(T_ACCENT) as i32 } } else { 0 },
                ..Sp::default()
            };
            tsp.place(r);
            tsp.ev = tev;
            tsp.state = if state & SS_DISABLED as u32 != 0 {
                3
            } else if tev & EV_HELD != 0 {
                2
            } else if tev & EV_HOVER != 0 {
                1
            } else {
                0
            };
            st.ribbon_tab(self, &tsp);
            let tid = id.wrapping_add(1 + i as u32);
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = state | if shown == Some(i) { SS_SELECTED as u32 } else { 0 };
            self.sem(K_TAB, tid, Some(t.label), i as i32);
            if t.flags & RT_CONTEXT != 0 && !t.set.is_empty() {
                let tip = alloc::format!("{}: {}", t.set, t.label);
                self.tooltip(&tip);
            } else if widths[i] < full[i] {
                self.tooltip(t.label);
            }
            // its key tip (level 1): a register shows its ribbon's tips, File opens
            if self.wp.rshell.level == 1 && t.flags & ST_DISABLED == 0 && disabled == 0 {
                let tip = tip_for(t.label, &self.wp.rshell.taken);
                if self.keytip_at(tid, r, tip, true, false) {
                    if t.flags & RT_FILE != 0 {
                        ev |= EV_OPEN;
                        self.wp.rshell.level = 0;
                    } else {
                        if *sel != i as i32 {
                            *sel = i as i32;
                            ev |= EV_ACT;
                        }
                        self.wp.rshell.level = 2;
                        self.wp.rshell.taken.clear();
                    }
                }
            }
        }
        // the focus visual of the row: around the selected register
        if ev0 & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
            if let Some(i) = shown {
                st.focus_rect(self, xs[i], row[1], widths[i], h);
            }
        }
        self.wp.ctx.last_rect = row;
        ev
    }

    // ------------------------------------------------------------------ backstage

    /// The File register's page (see the module header): a navigation pane `nav_w` DIP wide
    /// with a back item labelled `back`, the items, then the area for the page; the app fills
    /// it and calls backstage_end. Returns Some(-1) for back (also Esc), Some(i) when item i
    /// was activated (a page item, not BS_ACTION, becomes `*sel`).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn backstage_begin(&mut self, id: u32, back: &str, items: &[BackstageItem<'_>], sel: &mut i32, nav_w: i32) -> Option<i32> {
        let _src = self.src_scope();
        let mut out = None;
        if self.modal_active() == 0 && self.popup_count() == 0 && self.wp.rshell.level == 0 && self.key(VK_ESCAPE) {
            out = Some(-1);
        }
        self.next_size(-1, -1);
        self.box_begin(id, BOX_H, 0);
        let mut nav = NavState::default();
        self.next_size(nav_w, -1);
        self.nav_begin(id.wrapping_add(0x41), &mut nav);
        self.next_flags(0);
        if self.nav_item(id.wrapping_add(1), IC_BACK, back, false, None) & EV_ACT != 0 {
            out = Some(-1);
        }
        self.spacer(6);
        let mut footer = false;
        for (i, it) in items.iter().enumerate() {
            if it.flags & BS_FOOTER != 0 && !footer {
                footer = true;
                self.nav_footer();
            }
            if it.flags & BS_SEP != 0 {
                self.separator();
            }
            self.next_flags(it.flags & ST_DISABLED);
            let page = it.flags & BS_ACTION == 0;
            if self.nav_item(id.wrapping_add(2 + i as u32), it.icon, it.label, page && *sel == i as i32, None) & EV_ACT != 0 {
                if page && *sel != i as i32 {
                    *sel = i as i32;
                    self.repass();
                }
                out = Some(i as i32);
            }
        }
        self.nav_end();
        self.next_size(-1, -1);
        self.scroll_begin(id.wrapping_add(0x40), SC_V | BOX_V | pad(24));
        out
    }

    /// Finish the page begun with backstage_begin.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn backstage_end(&mut self) {
        let _src = self.src_scope();
        self.scroll_end();
        self.box_end();
    }
}

// ------------------------------------------------------------------ stock painters

/// the default register row: the window background, a divider along the bottom that leaves
/// the selected register (s[0], s[1]) open
pub(crate) fn stock_strip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let y = r[1] + r[3] - h;
    let (sx, sw) = (sp.s[0], sp.s[1]);
    if sw > 0 {
        ui.draw_rect(r[0], y, (sx - r[0]).max(0), h);
        ui.draw_rect(sx + sw, y, (r[0] + r[2] - sx - sw).max(0), h);
    } else {
        ui.draw_rect(r[0], y, r[2], h);
    }
}

/// the default register: the label, the selected one with a 2 DIP accent underline, File
/// on the accent, a context register with a 3 DIP edge of its set's colour on top
pub(crate) fn stock_tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let label = sp.label.unwrap_or("");
    let file = sp.flags & RT_FILE != 0;
    if file {
        let c = ui.color(if sp.state == 1 { T_ACCENT_HOVER } else if sp.state == 2 { T_ACCENT_PRESSED } else { T_ACCENT });
        ui.draw_color(c);
        ui.draw_rect(r[0], r[1] + ui.dp(3), r[2], r[3] - ui.dp(6));
    } else if sp.state == 1 || sp.state == 2 {
        let c = ui.color(T_SUBTLE_HOVER);
        ui.draw_color(c);
        ui.draw_rect(r[0], r[1] + ui.dp(3), r[2], r[3] - ui.dp(6));
    }
    if sp.value2 != 0 {
        ui.draw_color(sp.value2 as u32);
        ui.draw_rect(r[0], r[1], r[2], ui.dp(3));
    }
    let ts = if sp.value != 0 { TS_BODY_STRONG } else { TS_BODY };
    ui.ts_font(ts);
    let tw = ui.text.width(label);
    let line = ui.ts_line(ts);
    let base = ui.ts_base(ts);
    let col = ui.color(if file { T_ON_ACCENT } else if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    ui.emit_text(r[0] + (r[2] - tw) / 2, r[1] + (r[3] - line) / 2 + base, label, col);
    if sp.value != 0 {
        let c = if sp.value2 != 0 { sp.value2 as u32 } else { ui.color(T_ACCENT) };
        ui.draw_color(c);
        let t = ui.dp(2);
        ui.draw_rect(r[0] + ui.dp(8), r[1] + r[3] - t, (r[2] - ui.dp(16)).max(0), t);
    }
}

/// the default key tip: a small flyout-coloured badge with a frame and the keys (state 3: the
/// command is disabled)
pub(crate) fn stock_keytip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let fill = ui.color(T_FLYOUT) | 0xFF00_0000;
    let stroke = ui.color(T_FLYOUT_STROKE);
    let h = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], ui.dp(2), h, fill, stroke, stroke);
    let label = sp.label.unwrap_or("");
    ui.ts_font(TS_CAPTION);
    let tw = ui.text.width(label);
    let line = ui.ts_line(TS_CAPTION);
    let base = ui.ts_base(TS_CAPTION);
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    ui.emit_text(r[0] + (r[2] - tw) / 2, r[1] + (r[3] - line) / 2 + base, label, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tips_take_free_letters_then_zero_pairs() {
        let mut taken: Vec<String> = Vec::new();
        for l in ["Datei", "Start", "Einfügen", "Entwurf", "Ansicht"] {
            let t = tip_for(l, &taken);
            taken.push(t);
        }
        assert_eq!(taken, ["D", "S", "E", "N", "A"]);
        let all: Vec<String> = (b'A'..=b'Z').map(|c| String::from(c as char)).collect();
        assert_eq!(tip_for("Über", &all), "0A");
        assert_eq!(tip_for("Ölfarbe", &[]), "O");
    }
}
