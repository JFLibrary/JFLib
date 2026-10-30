// popup -- popups, flyouts, menus, menu bar, command table and accelerators, drop-down / split buttons, combo boxes, tooltips
//                                                                  [src/ui/ui_popup.s]
//
// Popups (a stack of up to 6 levels per window, drawn on L_POPUP):
//   popup_open(id)                      open popup `id` (inside a flyout: as its child)
//   popup_close()                       close every popup; the focus returns to the opener
//   popup_is_open(id) -> bool   popup_count() -> n
//   flyout_begin(id, FL_*) -> bool      draw popup `id` when it is open: a flyout surface
//                                       anchored to the last widget, content box BOX_V|PAD_4
//   flyout_end()                        always paired with flyout_begin
//   A press outside every popup closes them all and is consumed; Esc closes the top one;
//   Up / Down move the focus inside the top one.
//   popup_ext(i) / popup_rect(i) -> rect (U3, §6: popups that leave their window; the area
//   they may use is the monitor's work area -- they flip above / to the left where it ends)
// Commands (the table AppDesc::cmds, Command records):
//   cmd_find(id) -> &Command   cmd_state(id, CF_*)   cmd_flags(id) -> CF_*
//   cmd_label(id) -> label without '&'   accel_text(id) -> "Strg+Umschalt+S"
//   accel_order_set(1): "Strg+Alt+Umschalt+S" (ui_accel_order)
//   cmd_button(id, ST_*) -> EV          button from the table (icon, label, tooltip with
//                                       the accelerator, disabled / checked); queues it
// Menus (Mi records; Mi::List rows come from menu_list(base, rows), row k = id base + k; a
// checked row with an icon gives the flyout a check column and an icon column):
//   menubar(None | Some(bar)) -> EV_OPEN   WinUI MenuBar (None = AppDesc::menubar): items,
//                                       flyouts, submenus, keys (Alt / F10, Alt+mnemonic,
//                                       arrows, Enter, Esc)
//   menu_popup(menu)                    open a menu at the pointer (drawn by the runtime)
//   context_menu(menu) -> EV_OPEN       post-modifier: right click / menu key / Shift+F10 on
//                                       the last widget opens `menu`
//   context_target() -> id              the widget whose context menu is open
//   menu_item(id, label) -> EV          row inside a flyout (next_icon, next_flags
//                                       ST_DISABLED / ST_CHECKED); activation closes popups
//   menu_cmd(cmd) -> EV   menu_sep()    a row for a command (queues it), a separator
//   menu_list(base, rows)               the rows of the Mi::List with that base (the asm's
//                                       app-owned STRLIST buffer)
// Drop-down controls:
//   dropdown_button(id, label) -> EV    button with a chevron; opens popup `id` (EV_OPEN)
//   split_button(id, label) -> EV       EV_ACT from the main part; the chevron part
//                                       (id | 0x80000000) opens popup `id`
//   combobox(id, &mut sel, list) -> EV  EV_ACT when sel changed, EV_OPEN while open
//   (drop-downs open with Alt+Down, and with F4 unless the command table has an F4 accelerator)
// Tooltips:
//   tooltip(s)                          post-modifier: tooltip for the last widget after
//                                       500 ms of hover (timer UIT_TOOLTIP wakes the loop)
//   tooltip_part(s, part)               the same for one part of the last widget (an item of
//                                       a canvas, part != 0): the delay starts again when the
//                                       part changes, a press hides it until another part
// Runtime: accel_dispatch() at the start of uiw_pass_end: runtime-drawn menus, stale
// popups, command accelerators.
//
// Ids: a menu's popup id is a hash of the menu's address (menu_pid), the menu bar's item
// ids a hash of the bar's address -- like the asm, whose dumps therefore show other
// numbers for them.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::{Sp, Ui};
use plat::{PA_ABOVE, PA_BELOW, PA_FREE, PA_POINT, PA_SIDE};

// popup stack: fixed (menus and submenus inside each other; the platform shows a few popup
// windows, plat::PPW_MAX); a popup opened deeper replaces the deepest one (limit_hit)
const POPUP_MAX: usize = 6;
pub(crate) const PPF_FOCUS: u32 = 1; // focus the first focusable row once drawn
const PPF_AUTO: u32 = 2; // drawn by the runtime (menu_popup)
const PPF_POINT: u32 = 4; // anchored to a fixed point
const PPF_QUIET: u32 = 8; // focusid without the focus visual
const PPF_TWOCOL: u32 = 16; // a checked row with an icon: check column + icon column (U1)
const PPF_SCROLLED: u32 = 32; // (U3) a scrolling combo list was scrolled to its selection

const FLN_MAX: usize = 8; // flyouts inside each other (fixed: nesting; deeper: limit_hit)
const CST_LIMIT: usize = 1536; // at most this many commands with a state
// rows of a menu (fixed: a row's record index is a byte; more: limit_hit), all of them
// reachable by keys and mnemonics
const MENU_MAX: usize = 250;
const MB_MAX: usize = 16; // menus per menu bar (fixed: wider than any window; more: limit_hit)
const LAB_MAX: usize = 256;
const CATCH_ID: u32 = 0x7FFE_0C0C; // hit box under the popups (light dismiss)
const CTX_SALT: u32 = 0x00C0_FFEE;
const BOXID_XOR: u32 = 0x6C00_0000; // flyout content box id = popup id ^ BOXID_XOR
const FLS_XOR: u32 = 0x0100_0000; // (U3) scroll view of a tall flyout = content box id ^ FLS_XOR
const TIP_DELAY: u32 = 500;
const TIP_QUICK: u32 = 300;
const BTN_MASK: u32 = (1 << BTN_LEFT) | (1 << BTN_RIGHT) | (1 << BTN_MIDDLE);
const MOD3: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;

/// CMD record: one row of the app's command table (AppDesc::cmds)
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// command id (!= 0); also the widget id of cmd_button
    pub id: u32,
    /// IC_* code point or 0
    pub icon: u32,
    /// UTF-8 label, '&' marks the mnemonic ("&&" = '&')
    pub label: &'static str,
    /// accelerator key VK_* (0 = none)
    pub key: u16,
    /// MOD_SHIFT | MOD_CTRL | MOD_ALT
    pub mods: u8,
    /// initial CF_* state
    pub flags: u8,
}

impl Command {
    /// CMD id, "&Label", icon, mods, key, flags (the order of the asm macro)
    pub const fn new(id: u32, label: &'static str, icon: u32, mods: u32, key: u32, flags: u32) -> Command {
        Command { id, icon, label, key: key as u16, mods: mods as u8, flags: flags as u8 }
    }
}

/// menu record (MENUBAR / MENU / MI_* of ui.inc); menus are `static` slices of them
#[derive(Clone, Copy, Debug)]
pub enum Mi {
    /// MI_CMD cmd: a row from the command table
    Cmd(u32),
    /// MI_SEP
    Sep,
    /// MI_SUB "Label", submenu
    Sub(&'static str, &'static [Mi]),
    /// MENU "Label", menu: a top-level menu of a menu bar
    Menu(&'static str, &'static [Mi]),
    /// MI_ITEM id, "Label", icon: a plain row, runs App::command(id, 0)
    Item(u32, &'static str, u32),
    /// MI_LIST base, max, "placeholder": one row per string of the rows set with
    /// Ui::menu_list(base, ..) (at most max); row k runs App::command(base + k, 0) and has
    /// the widget id base + k; no rows: the placeholder as a disabled row ("" = none)  (U1)
    List(u32, u16, &'static str),
}

impl Mi {
    // MI_LABEL: the label of MENU / MI_SUB / MI_ITEM, the placeholder of MI_LIST
    fn label(&self) -> Option<&'static str> {
        match *self {
            Mi::Sub(l, _) | Mi::Menu(l, _) | Mi::Item(_, l, _) | Mi::List(_, _, l) => Some(l),
            Mi::Cmd(_) | Mi::Sep => None,
        }
    }
    // MI_ARG: the command of MI_CMD, the id of MI_ITEM, the base of MI_LIST
    fn arg(&self) -> u32 {
        match *self {
            Mi::Cmd(a) | Mi::Item(a, _, _) | Mi::List(a, _, _) => a,
            _ => 0,
        }
    }
}

// popup stack entry  [PP_*]
#[derive(Clone, Copy, Default)]
struct Pp {
    id: u32,
    /// PPF_*
    flags: u32,
    /// anchor x, y, w, h (px)
    anchor: [i32; 4],
    /// flyout rectangle when last drawn (px)
    rect: [i32; 4],
    /// menu of a runtime-drawn popup
    menu: Option<&'static [Mi]>,
    /// focus to restore when it closes
    ret: u32,
    /// (§6) how it hangs at the anchor (PA_*); its natural place before the platform's
    /// placement moved it (px, logical): as computed (the key of the placement) and where its
    /// rectangle lay (plat::popup_anchor)
    kind: u32,
    key: [i32; 2],
    nat: [i32; 2],
    /// pass that drew it, pass that opened it
    seen: u32,
    born: u32,
    /// hit list range of its content (this pass)
    hit: u32,
    hitend: u32,
    /// widget to focus once drawn (0 = none)
    focusid: u32,
}

// flyout nesting record (flyout_begin .. flyout_end)  [FLN_*]
#[derive(Clone, Copy, Default)]
struct Fln {
    open: bool,
    idx: u32,
    layer: u32,
    dis: u32,
    level: u32,
    /// (U3) the content scrolls (taller than the area)
    scroll: bool,
    clip: [i32; 4],
}

/// state shared by all windows
#[derive(Default)]
pub(crate) struct S {
    /// command states (ui_cmd_state; the asm's hash table of up to CST_LIMIT entries)
    cst: BTreeMap<u32, u32>,
    cst_warned: bool,
    /// ui_accel_order: 0 = Ctrl, Shift, Alt (default), 1 = Ctrl, Alt, Shift
    accel_order: u32,
    /// rows of the Mi::List menus by base id (the app's STRLIST buffers)
    pub(crate) lists: Vec<(u32, Vec<String>)>,
}

/// per-window state (ui_popup_wb0 .. ui_popup_wb1: popup stack, flyouts, menu bar, tooltip)
#[derive(Default)]
pub(crate) struct W {
    pp: [Pp; POPUP_MAX],
    pp_n: u32,
    fln: [Fln; FLN_MAX],
    fl_sp: u32,
    fl_over: u32,
    fl_level: u32,
    fl_offy: i32,
    catch_frame: u32,
    mb_mode: bool,
    mb_prev: u32,
    alt_armed: bool,
    ctx_target: u32,
    tip_id: u32,
    tip_t0: u32,
    tip_block: u32,
    tip_shown: u32,
    tip_hid_t: u32,
    tip_hid_ok: bool,
    tip_suppress: bool,
    /// the part of tip_id the tooltip is for (tooltip_part; 0: the whole widget)
    tip_part: u64,
    hot_prev: u32,
    /// (U3) tooltip rectangle of its last pass (px), the pass that drew it
    tip_rect: [i32; 4],
    tip_seen: u32,
    /// (§6) the pointer it hangs below, its natural place (px, logical)
    tip_anchor: [i32; 4],
    tip_nat: [i32; 2],
}

// ------------------------------------------------------------------ small helpers

/// strip_amp(src) -> (src without the '&' markers ("&&" = '&'; at most LAB_MAX - 1 bytes),
/// mnemonic (upper-case ASCII, 0 = none), byte offset of the mnemonic in the result or -1)
fn strip_amp(src: &str) -> (String, u32, i32) {
    let s = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(s.len().min(LAB_MAX));
    let mut m = 0u32;
    let mut pos = -1i32;
    let mut i = 0;
    while out.len() < LAB_MAX - 1 && i < s.len() {
        let mut c = s[i];
        if c == 0 {
            break;
        }
        i += 1;
        if c == b'&' {
            if i >= s.len() || s[i] == 0 {
                break;
            }
            c = s[i];
            i += 1;
            if c != b'&' && m == 0 {
                pos = out.len() as i32;
                m = c as u32;
                if c.is_ascii_lowercase() {
                    m -= 32;
                }
            }
        }
        out.push(c);
    }
    (utf8_string(out), m, pos)
}

// bytes -> String, cut at the last complete UTF-8 sequence (a label cut at LAB_MAX)
fn utf8_string(mut b: Vec<u8>) -> String {
    match core::str::from_utf8(&b) {
        Ok(_) => {}
        Err(e) => b.truncate(e.valid_up_to()),
    }
    String::from_utf8(b).unwrap_or_default()
}

// str_cat(dst, src, cap): append src while the total stays below cap (never inside a
// UTF-8 sequence)
fn cat(dst: &mut String, src: &str, cap: usize) {
    if dst.len() >= cap {
        return;
    }
    let mut n = (cap - dst.len() - 1).min(src.len());
    while !src.is_char_boundary(n) {
        n -= 1;
    }
    dst.push_str(&src[..n]);
}

/// menu_pid(menu, salt) -> popup id of a menu (low byte 0: rows are id + 1 + index)
fn menu_pid(menu: &'static [Mi], salt: u32) -> u32 {
    let a = (menu.as_ptr() as usize as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let e = ((a >> 32) as u32) ^ salt.wrapping_mul(0x045D_9F3B);
    (e & 0x0FFF_FF00) | 0xB000_0000
}

// mnem_find(mnemonics, vk) -> index or -1
fn mnem_find(mn: &[u8], vk: u32) -> i32 {
    mn.iter().position(|&m| m != 0 && m as u32 == vk).map_or(-1, |i| i as i32)
}

// is_alnum_vk(vk): VK '0'..'9' / 'A'..'Z'
fn is_alnum_vk(vk: u32) -> bool {
    vk.wrapping_sub(0x30) <= 9 || vk.wrapping_sub(0x41) <= 25
}

/// the strings of a STRLIST (the list ends at the first empty string)
fn strlist_n(list: &[&str]) -> usize {
    list.iter().position(|s| s.is_empty()).unwrap_or(list.len())
}

// strlist_at(list, i) -> string i of a STRLIST or None
fn strlist_at<'a>(list: &[&'a str], i: i32) -> Option<&'a str> {
    if i < 0 || i as usize >= strlist_n(list) { None } else { Some(list[i as usize]) }
}

// key names of the accelerator texts (German; i18n::tr translates them)  [key_table]
static KEY_TABLE: [(u32, &str); 26] = [
    (VK_DELETE, "Entf"),
    (VK_INSERT, "Einfg"),
    (VK_RETURN, "Eingabe"),
    (VK_ESCAPE, "Esc"),
    (VK_BACK, "Rücktaste"),
    (VK_TAB, "Tab"),
    (VK_SPACE, "Leertaste"),
    (VK_HOME, "Pos1"),
    (VK_END, "Ende"),
    (VK_PRIOR, "Bild auf"),
    (VK_NEXT, "Bild ab"),
    (VK_LEFT, "Nach-links"),
    (VK_RIGHT, "Nach-rechts"),
    (VK_UP, "Nach-oben"),
    (VK_DOWN, "Nach-unten"),
    (VK_OEM_PLUS, "Plus"),
    (VK_OEM_MINUS, "Minus"),
    (VK_ADD, "Num +"),
    (VK_SUBTRACT, "Num -"),
    (VK_OEM_COMMA, ","),
    (VK_OEM_PERIOD, "."),
    (VK_OEM_4, "["), // (U1)
    (VK_OEM_6, "]"),
    (VK_MULTIPLY, "Num *"),
    (VK_DIVIDE, "Num /"),
    (VK_DECIMAL, "Num ,"),
];

// key_name(vk): letters / digits, "F1".."F24", "Num 0".."Num 9", the table, else "?"
fn key_name(vk: u32) -> String {
    if is_alnum_vk(vk) {
        let mut s = String::new();
        s.push(vk as u8 as char);
        return s;
    }
    if vk.wrapping_sub(VK_F1) <= 23 {
        return alloc::format!("F{}", vk - VK_F1 + 1);
    }
    if vk.wrapping_sub(VK_NUMPAD0) <= 9 {
        return alloc::format!("Num {}", vk - VK_NUMPAD0); // (U1)
    }
    let name = KEY_TABLE.iter().find(|k| k.0 == vk).map_or("?", |k| crate::i18n::tr(k.1));
    String::from(name)
}

impl Ui {
    // ------------------------------------------------------------------ popup stack helpers

    // pp_find(id) -> index in the popup stack or -1
    fn pp_find(&self, id: u32) -> i32 {
        let w = &self.wp.popup;
        (0..w.pp_n as usize).rev().find(|&i| w.pp[i].id == id).map_or(-1, |i| i as i32)
    }

    // pp_flag(id, flags): or PPF_* flags into the entry of popup id (if open)
    pub(crate) fn pp_flag(&mut self, id: u32, f: u32) {
        let i = self.pp_find(id);
        if i >= 0 {
            self.wp.popup.pp[i as usize].flags |= f;
        }
    }

    // pp_push(id, level) -> index: close popups >= level, open `id` at level
    fn pp_push(&mut self, id: u32, level: u32) -> usize {
        let mut l = level.min(self.wp.popup.pp_n);
        if l > POPUP_MAX as u32 - 1 {
            l = POPUP_MAX as u32 - 1;
            self.limit_hit(super::ctx::LIM_POPUPS);
        }
        let w = &mut self.wp.popup;
        w.pp_n = l + 1;
        w.pp[l as usize] = Pp {
            id,
            ret: self.wp.ctx.focus,
            born: self.frame,
            anchor: self.wp.ctx.last_rect,
            ..Pp::default()
        };
        self.repass = true;
        l as usize
    }

    // pp_close_to(level): close the popups >= level; the focus returns to what it was when
    // `level` opened
    fn pp_close_to(&mut self, level: u32) {
        let w = &mut self.wp.popup;
        if level >= w.pp_n {
            return;
        }
        w.pp_n = level;
        self.wp.ctx.focus = w.pp[level as usize].ret;
        self.repass = true;
    }

    // pp_dismiss_all: close every popup and leave menu bar mode
    fn pp_dismiss_all(&mut self) {
        self.pp_close_to(0);
        if self.wp.popup.mb_mode {
            self.wp.popup.mb_mode = false;
            self.wp.ctx.focus = self.wp.popup.mb_prev;
            self.repass = true;
        }
    }

    // open_key -> Alt+Down or F4 was pressed (consumed): opens drop-downs.  F4 is left
    // alone when the command table has an F4 accelerator (paint.net uses it).
    pub(crate) fn open_key(&mut self) -> bool {
        let k = self.key_first();
        let vk = k & 0xFFFF;
        let m = (k >> 16) & MOD3;
        let yes = if vk == VK_F4 && m == 0 {
            self.accel_find(VK_F4, 0).is_none() // (U1) an F4 accelerator wins
        } else {
            vk == VK_DOWN && m == MOD_ALT
        };
        if yes {
            self.key(vk);
        }
        yes
    }

    // accel_find(vk, mods) -> the command whose accelerator is vk + mods
    pub(crate) fn accel_find(&self, vk: u32, mods: u32) -> Option<&'static Command> {
        let t: &'static [Command] = self.desc.cmds;
        t.iter().take_while(|c| c.id != 0).find(|c| c.key as u32 == vk && c.mods as u32 & MOD3 == mods)
    }

    // last_widget -> (id, rect) of the last semantic record with an id (0 = none); the rect
    // logical again (sems are physical right-to-left; the mirror is its own inverse)
    fn last_widget(&self) -> (u32, [i32; 4]) {
        self.sems.iter().rev().find(|r| r.id != 0).map_or((0, [0; 4]), |r| (r.id, self.phys_rect(r.rect)))
    }

    // focus_is_text: the focused widget is a text input (this pass)
    pub(crate) fn focus_is_text(&self) -> bool {
        let c = &self.wp.ctx;
        if c.focus == 0 {
            return false;
        }
        c.hits[c.hit_cur].iter().rev().find(|h| h.id == c.focus).is_some_and(|h| h.flags as u32 & HB_TEXT != 0)
    }

    // focus_was_text: the focused widget was a text input in the last pass (for questions
    // asked before this pass drew it, as ui.undo_keys at the start of a frame)
    pub(crate) fn focus_was_text(&self) -> bool {
        let c = &self.wp.ctx;
        c.focus != 0 && c.hits[c.hit_cur ^ 1].iter().rev().find(|h| h.id == c.focus).is_some_and(|h| h.flags as u32 & HB_TEXT != 0)
    }

    // uiw_popup_area (ui_win.s): the area the live window's popups may use when they may
    // leave it (the screen / virtual desktop; known false: Wayland)  (ui-win)
    fn pp_popup_area(&mut self) -> Option<([i32; 4], bool)> {
        self.uiw_popup_area()
    }

    // g_uiw_noaccel (ui_win.s): the live window is a dialog window, no accelerators  (ui-win)
    fn pp_noaccel(&self) -> bool {
        self.m.win.noaccel
    }

    // pp_bounds(w, h) -> the area a popup of w x h px of the live window may use: x0, y0, x1, y1
    // (px, window coordinates): the screen / virtual desktop around the window (headless
    // without a virtual screen: the main window keeps its popups); where the screen is not
    // known (Wayland) the window while the popup fits into it, else a wide area that the
    // compositor keeps on screen
    fn pp_bounds(&mut self, w: i32, h: i32) -> [i32; 4] {
        let win = [0, 0, self.wp.rt.fb.w, self.wp.rt.fb.h];
        match self.pp_popup_area() {
            Some((r, true)) => r,
            Some((r, false)) if w > win[2] || h > win[3] => r,
            _ => win,
        }
    }

    // pp_nudge(item, rect) -> (dx, dy): how far the platform moved popup item `item` from its
    // natural place, computed as rect (px, logical) -- Wayland's compositor flips / slides popups
    // that leave the window (uiw_popup_nudge)
    fn pp_nudge(&self, item: u32, r: [i32; 4]) -> (i32, i32) {
        let p = self.phys_rect(r);
        let (dx, dy) = self.uiw_popup_nudge(item, [p[0], p[1]]);
        (if self.rtl { -dx } else { dx }, dy)
    }

    // ------------------------------------------------------------------ popup stack API

    /// ui_popup_is_open(id): popup `id` is open
    pub fn popup_is_open(&self, id: u32) -> bool {
        self.pp_find(id) >= 0
    }

    /// ui_popup_count -> open popups of the live window (U3)
    pub fn popup_count(&self) -> u32 {
        self.wp.popup.pp_n
    }

    /// ui_popup_open(id): open popup `id` (inside a flyout: as a child of that flyout,
    /// otherwise it replaces every open popup); an open popup only loses its children
    pub fn popup_open(&mut self, id: u32) {
        let i = self.pp_find(id);
        if i >= 0 {
            let n = i as u32 + 1;
            if n < self.wp.popup.pp_n {
                self.wp.popup.pp_n = n;
                self.repass = true;
            }
            return;
        }
        let l = self.wp.popup.fl_level;
        self.pp_push(id, l);
    }

    /// ui_popup_close: close every popup (the focus returns to where it was)
    pub fn popup_close(&mut self) {
        self.pp_dismiss_all();
    }

    // popup_catcher -> a press outside every popup dismissed them.  Once per pass (while
    // popups are open) a window-sized hit box goes under the popups, so the base layer gets
    // no pointer input and a press there closes them.
    fn popup_catcher(&mut self) -> bool {
        if self.wp.popup.pp_n == 0 || self.wp.popup.catch_frame == self.frame {
            return false;
        }
        self.wp.popup.catch_frame = self.frame;
        let c = &mut self.wp.ctx;
        if c.hot == CATCH_ID && c.pressed & BTN_MASK != 0 {
            c.pressed &= !BTN_MASK;
            self.pp_dismiss_all();
            return true;
        }
        let (lr, clip) = (c.last_rect, c.uiclip);
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        self.wp.ctx.last_rect = [0, 0, w, h];
        self.wp.ctx.uiclip = [0, 0, w, h];
        let prev = self.layer_set(L_POPUP);
        self.hit(CATCH_ID, 0);
        self.layer_set(prev);
        self.wp.ctx.last_rect = lr;
        self.wp.ctx.uiclip = clip;
        false
    }

    // ------------------------------------------------------------------ flyouts

    /// ui_flyout_begin(id, FL_*) -> popup `id` is open: then its content follows (a vertical
    /// box with 4 DIP padding) until flyout_end, which is called in any case.  The flyout is
    /// anchored to the last placed widget (FL_POINT: to the pointer position where it
    /// opened) and kept inside the window.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn flyout_begin(&mut self, id: u32, flags: u32) -> bool {
        let _src = self.src_scope();
        let sp_i = self.wp.popup.fl_sp as usize;
        if sp_i >= FLN_MAX {
            self.wp.popup.fl_over += 1;
            self.limit_hit(super::ctx::LIM_FLYOUTS);
            return false;
        }
        self.wp.popup.fl_sp += 1;
        self.wp.popup.fln[sp_i].open = false;
        let i = self.pp_find(id);
        if i < 0 {
            return false;
        }
        let idx = i as usize;
        // anchor: the last widget, or a fixed point
        {
            let (mx, my, lr) = (self.wp.ctx.mx, self.wp.ctx.my, self.wp.ctx.last_rect);
            let e = &mut self.wp.popup.pp[idx];
            if e.flags & PPF_POINT == 0 {
                if flags & FL_POINT != 0 {
                    e.flags |= PPF_POINT;
                    e.anchor = [mx, my, 0, 0];
                } else {
                    e.anchor = lr;
                }
            }
        }
        if self.popup_catcher() {
            return false;
        }
        // Esc closes the top popup
        if self.wp.popup.pp_n.wrapping_sub(1) == idx as u32 && self.key(VK_ESCAPE) {
            self.pp_close_to(idx as u32);
            self.wp.ctx.focus_visible = true;
            return false;
        }
        // size from the previous pass
        let boxid = id ^ BOXID_XOR;
        let (mut fw, mut fh) = {
            let s = self.slot(boxid);
            (s.measw, s.meash)
        };
        let anchor = self.wp.popup.pp[idx].anchor;
        let mut forced = 0; // forced width px (0 = natural)
        if flags & FL_MINW != 0 && fw < anchor[2] {
            fw = anchor[2];
            forced = fw;
        }
        let st = self.style;
        let gap = st.flyout_pad(self);
        // the area the flyout may use (px, window coordinates): the window, or the screen for
        // popups that become their own OS windows (pp_bounds, U3, §6)
        let area = self.pp_bounds(fw, fh);
        // taller than the area: it scrolls (U3).  Where the screen is not known (Wayland) the
        // area is wide, but no popup gets taller than the screen: a long list scrolls instead
        // of hanging off it
        let mut room = area[3] - area[1];
        if fh > self.wp.rt.fb.h {
            if let Some(h) = self.uiw_popup_screen_h() {
                room = room.min(h);
            }
        }
        let mut scroll = false;
        if fh > room {
            fh = room;
            scroll = true;
        }
        // position (x, y) in px
        let mut x = anchor[0];
        let mut y = anchor[1];
        let point = self.wp.popup.pp[idx].flags & PPF_POINT != 0;
        if point {
            if x + fw > area[2] {
                x -= fw;
            }
            if y + fh > area[3] {
                y -= fh;
            }
        } else if flags & FL_RIGHT != 0 {
            // submenu: right of the row, first row level with it; flips to the left
            x += anchor[2];
            y -= gap;
            if x + fw > area[2] {
                x = anchor[0] - fw;
            }
        } else if flags & FL_OVER != 0 {
            if !scroll {
                y -= self.wp.popup.fl_offy; // scrolling list: from the area top
            }
        } else {
            if flags & FL_ALIGN_END != 0 {
                x += anchor[2] - fw;
            }
            if flags & FL_ABOVE != 0 {
                y -= gap + fh;
                if y < area[1] {
                    y = anchor[1] + anchor[3] + gap;
                }
            } else {
                // below; above when it does not fit
                y += anchor[3] + gap;
                if y + fh > area[3] {
                    let a = anchor[1] - gap - fh;
                    if a >= area[1] {
                        y = a;
                    }
                }
            }
        }
        // kept inside the area
        if x > area[2] - fw {
            x = area[2] - fw;
        }
        if x < area[0] {
            x = area[0];
        }
        if y > area[3] - fh {
            y = area[3] - fh;
        }
        if y < area[1] {
            y = area[1];
        }
        // (§6) how it hangs at its anchor; where the platform put it (Wayland)
        let kind = if point {
            PA_POINT
        } else if flags & FL_RIGHT != 0 {
            PA_SIDE
        } else if flags & FL_OVER != 0 {
            PA_FREE
        } else if y < anchor[1] {
            PA_ABOVE
        } else {
            PA_BELOW
        };
        let key = [x, y];
        let (dx, dy) = self.pp_nudge(idx as u32, [x, y, fw, fh]);
        x += dx;
        y += dy;
        let e = &mut self.wp.popup.pp[idx];
        e.kind = kind;
        e.key = key;
        // absolute placement of the content box (DIP relative to the current box)
        let br = self.box_rect();
        let xd = self.px2dip(x - br[0]);
        let yd = self.px2dip(y - br[1]);
        let wd = if forced != 0 { self.px2dip(forced) } else { 0 };
        let hd = if scroll { self.px2dip(fh) } else { 0 };
        self.next_rect(xd, yd, wd, hd);
        // popup layer, clipping to the area, not disabled
        let layer = self.layer_set(L_POPUP);
        let w = &mut self.wp.popup;
        let f = &mut w.fln[sp_i];
        f.layer = layer;
        f.clip = self.wp.ctx.uiclip;
        f.dis = self.wp.ctx.disabled_depth;
        self.wp.ctx.disabled_depth = 0;
        self.wp.ctx.uiclip = area;
        f.level = w.fl_level;
        f.idx = idx as u32;
        w.fl_level = idx as u32 + 1;
        f.open = true;
        f.scroll = scroll;
        let frame = self.frame;
        if !scroll {
            // content box; its rectangle is the flyout
            let bf = st.flyout_box(self);
            self.box_begin(boxid, bf, 0);
            let e = &mut self.wp.popup.pp[idx];
            e.rect = self.wp.ctx.last_rect;
            e.nat = [e.rect[0] - dx, e.rect[1] - dy];
            e.seen = frame;
            self.fl_surface(idx, boxid);
            return true;
        }
        // (U3) too tall: the surface at the height of the area, a scroll view with the
        // content box inside (the shadow stays outside the clip of the view)
        let e = &mut self.wp.popup.pp[idx];
        e.rect = [x, y, fw, fh];
        e.nat = key;
        e.seen = frame;
        self.fl_surface(idx, boxid);
        self.fl_scroll_begin(boxid ^ FLS_XOR);
        // a combo box list opens with its selected row in view
        let e = &mut self.wp.popup.pp[idx];
        if e.flags & PPF_SCROLLED == 0 {
            e.flags |= PPF_SCROLLED;
            if flags & FL_OVER != 0 {
                let oy = self.px2dip(self.wp.popup.fl_offy);
                self.fl_scroll_to(boxid ^ FLS_XOR, oy);
            }
        }
        let bf = st.flyout_box(self);
        self.box_begin(boxid, bf, 0);
        true
    }

    // (U3) flyouts taller than the area scroll (ui-list's scroll view)
    fn fl_scroll_begin(&mut self, id: u32) {
        self.scroll_begin(id, SC_V);
    }
    fn fl_scroll_to(&mut self, id: u32, y_dip: i32) {
        self.scroll_to(id, -1, y_dip);
    }
    fn fl_scroll_end(&mut self) {
        self.scroll_end();
    }

    /// ui_popup_ext(i) -> rect x, y, w, h (px, window coordinates) of item i of the live
    /// window when it was drawn in the last pass partly outside its framebuffer.  Items
    /// 0 .. 5 are the open popups, 6 the tooltip.  (U3, OS popup windows)
    pub fn popup_ext(&self, i: u32) -> Option<[i32; 4]> {
        self.popup_item(i, false)
    }

    /// ui_popup_rect(i) -> rect (px) of item i of the live window when it was drawn in the
    /// last pass (anywhere).  The main window gives its popups own OS windows with it while
    /// tool windows are shown, so menus stay above them.
    pub fn popup_rect(&self, i: u32) -> Option<[i32; 4]> {
        self.popup_item(i, true)
    }

    /// (§6) how item i of the live window hangs at its anchor (physical px): the anchor
    /// rectangle, the PA_* kind, its natural place x, y (plat::popup_anchor) and the key of the
    /// placement (the natural place as computed: uiw_popup_nudge)
    pub(crate) fn popup_place(&self, i: u32) -> Option<([i32; 4], u32, [i32; 2], [i32; 2])> {
        let w = &self.wp.popup;
        let (anchor, kind, nat, key, r) = if i == POPUP_MAX as u32 {
            (w.tip_anchor, PA_BELOW, w.tip_nat, w.tip_nat, w.tip_rect)
        } else {
            let e = w.pp.get(i as usize).filter(|_| i < w.pp_n)?;
            (e.anchor, e.kind, e.nat, e.key, e.rect)
        };
        let n = self.phys_rect([nat[0], nat[1], r[2], r[3]]);
        let k = self.phys_rect([key[0], key[1], r[2], r[3]]);
        Some((self.phys_rect(anchor), kind, [n[0], n[1]], [k[0], k[1]]))
    }

    fn popup_item(&self, i: u32, anywhere: bool) -> Option<[i32; 4]> {
        let w = &self.wp.popup;
        let r = if i == POPUP_MAX as u32 {
            if w.tip_shown == 0 || w.tip_seen != self.frame {
                return None;
            }
            w.tip_rect
        } else {
            if i > POPUP_MAX as u32 || i >= w.pp_n {
                return None;
            }
            let e = &w.pp[i as usize];
            if e.seen != self.frame {
                return None;
            }
            e.rect
        };
        if r[2] <= 0 || r[3] <= 0 {
            return None;
        }
        let r = self.phys_rect(r); // window px (right-to-left: mirrored)
        if anywhere {
            return Some(r);
        }
        // inside the framebuffer: drawn there, no own window
        let fb = &self.wp.rt.fb;
        if r[0] < 0 || r[1] < 0 || r[2] + r[0] > fb.w || r[1] + r[3] > fb.h { Some(r) } else { None }
    }

    // fl_surface(entry, box id): shadow, fill, 1 px stroke, 8 DIP corners of the flyout at
    // entry.rect; its body blocks the pointer (hit box before the content)
    fn fl_surface(&mut self, idx: usize, boxid: u32) {
        let n = self.wp.ctx.hits[self.wp.ctx.hit_cur].len() as u32;
        let e = &mut self.wp.popup.pp[idx];
        e.hit = n;
        e.hitend = n;
        let r = e.rect;
        if r[2] <= 0 || r[3] <= 0 {
            return;
        }
        let st = self.style;
        let mut sp = Sp::default();
        sp.place(r);
        st.flyout_frame(self, &sp);
        self.hit(boxid, HB_BLOCK);
    }

    /// ui_flyout_end: close the content of flyout_begin (also when it returned false)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn flyout_end(&mut self) {
        let _src = self.src_scope();
        let w = &mut self.wp.popup;
        if w.fl_over != 0 {
            w.fl_over -= 1;
            return;
        }
        if w.fl_sp == 0 {
            return;
        }
        w.fl_sp -= 1;
        let f = w.fln[w.fl_sp as usize];
        if !f.open {
            return;
        }
        self.box_end();
        if f.scroll {
            self.fl_scroll_end(); // (U3) tall flyout
        }
        self.layer_set(f.layer);
        self.wp.ctx.uiclip = f.clip;
        self.wp.ctx.disabled_depth = f.dis;
        self.wp.popup.fl_level = f.level;
        // the content may have closed it
        let i = f.idx;
        if i >= self.wp.popup.pp_n {
            return;
        }
        let idx = i as usize;
        self.wp.popup.pp[idx].hitend = self.wp.ctx.hits[self.wp.ctx.hit_cur].len() as u32;
        // focus requests
        let fid = self.wp.popup.pp[idx].focusid;
        if fid != 0 {
            let e = &mut self.wp.popup.pp[idx];
            e.focusid = 0;
            if e.flags & PPF_QUIET != 0 {
                e.flags &= !PPF_QUIET;
                self.wp.ctx.focus = fid;
                self.repass = true;
            } else {
                self.focus(fid);
            }
        } else if self.wp.popup.pp[idx].flags & PPF_FOCUS != 0 {
            self.wp.popup.pp[idx].flags &= !PPF_FOCUS;
            let t = self.pp_nav(idx, 0);
            if t != 0 {
                self.focus(t);
            }
        }
        // Up / Down move the focus through the rows of the top popup
        if self.wp.popup.pp_n.wrapping_sub(1) == i {
            let nav = if self.key_peek(VK_DOWN) {
                Some((1, VK_DOWN))
            } else if self.key_peek(VK_UP) {
                Some((-1, VK_UP))
            } else {
                None
            };
            if let Some((dir, vk)) = nav {
                let t = self.pp_nav(idx, dir);
                if t != 0 {
                    self.key(vk);
                    self.focus(t);
                }
            }
        }
        self.wp.ctx.last_rect = self.wp.popup.pp[idx].rect;
    }

    // pp_nav(entry, dir) -> id of the focusable row to focus (0 = none): dir 0 first, +1
    // after the focused one, -1 before it (wrapping)
    fn pp_nav(&self, idx: usize, dir: i32) -> u32 {
        let e = &self.wp.popup.pp[idx];
        let c = &self.wp.ctx;
        let list = &c.hits[c.hit_cur];
        let (a, b) = ((e.hit as usize).min(list.len()), (e.hitend as usize).min(list.len()));
        let (mut first, mut last, mut before, mut after, mut found, mut prev) = (0, 0, 0, 0, false, 0);
        for h in &list[a..b.max(a)] {
            if h.flags as u32 & (HB_FOCUS | HB_DISABLED) != HB_FOCUS || h.layer as u32 != L_POPUP {
                continue;
            }
            let id = h.id;
            if first == 0 {
                first = id;
            }
            last = id;
            if found {
                if after == 0 {
                    after = id;
                }
            } else if id == c.focus {
                found = true;
                before = prev;
            }
            prev = id;
        }
        if dir == 0 {
            return first;
        }
        if !found {
            return if dir > 0 { first } else { last };
        }
        if dir > 0 {
            if after != 0 { after } else { first }
        } else if before != 0 {
            before
        } else {
            last
        }
    }

    // ------------------------------------------------------------------ commands

    /// ui_cmd_find(id) -> the command record of the app's command table
    pub fn cmd_find(&self, id: u32) -> Option<&'static Command> {
        let t: &'static [Command] = self.desc.cmds;
        t.iter().take_while(|c| c.id != 0).find(|c| c.id == id)
    }

    /// ui_cmd_state(id, flags): set the CF_* state of a command; it stays until changed
    /// (calling it every frame with the same flags is fine) and overrides the table flags.
    /// Up to 1536 commands have a state; more print one UIWARN and are ignored.
    pub fn cmd_state(&mut self, id: u32, flags: u32) {
        if id == 0 {
            return;
        }
        let s = &mut self.m.popup;
        let cur = match s.cst.get(&id) {
            Some(&f) => f,
            None if s.cst.len() >= CST_LIMIT => {
                if !s.cst_warned {
                    s.cst_warned = true;
                    let m = alloc::format!("ui_cmd_state: more than {} command states (id {} ignored)", CST_LIMIT, id);
                    self.warn(&m);
                }
                return;
            }
            None => u32::MAX, // a new entry starts at -1
        };
        self.m.popup.cst.insert(id, flags);
        if cur != flags {
            self.repass = true;
        }
    }

    /// ui_cmd_flags(id) -> CF_* state (cmd_state, else the table flags)
    pub fn cmd_flags(&self, id: u32) -> u32 {
        if id != 0 {
            if let Some(&f) = self.m.popup.cst.get(&id) {
                return f;
            }
        }
        self.cmd_find(id).map_or(0, |c| c.flags as u32)
    }

    /// ui_cmd_label(id) -> label of a command without '&' ("" if unknown)
    pub fn cmd_label(&self, id: u32) -> String {
        strip_amp(self.cmd_find(id).map_or("", |c| c.label)).0
    }

    /// ui_accel_text(id) -> accelerator of a command as text, e.g. "Strg+Umschalt+S"
    pub fn accel_text(&self, id: u32) -> String {
        match self.cmd_find(id) {
            Some(c) => self.accel_fmt(c, 64),
            None => String::new(),
        }
    }

    /// ui_accel_order: 0 = "Strg+Umschalt+Alt+X" (default), 1 = "Strg+Alt+Umschalt+X"  (U1)
    pub fn accel_order_set(&mut self, order: u32) {
        self.m.popup.accel_order = order;
    }

    // accel_fmt(rec, cap) -> "Strg+Umschalt+Alt+Key" (key names of the UI language, i18n::tr;
    // accel_order 1: "Strg+Alt+Umschalt+Key")
    fn accel_fmt(&self, c: &Command, cap: usize) -> String {
        let mut s = String::new();
        if c.key == 0 {
            return s;
        }
        let modifier = |s: &mut String, name: &'static str| {
            cat(s, crate::i18n::tr(name), cap);
            cat(s, "+", cap);
        };
        let mut m = c.mods as u32;
        if m & MOD_CTRL != 0 {
            modifier(&mut s, "Strg");
        }
        if self.m.popup.accel_order != 0 && m & MOD_ALT != 0 {
            modifier(&mut s, "Alt"); // (U1) Ctrl, Alt, Shift
            m &= !MOD_ALT;
        }
        if m & MOD_SHIFT != 0 {
            modifier(&mut s, "Umschalt");
        }
        if m & MOD_ALT != 0 {
            modifier(&mut s, "Alt");
        }
        cat(&mut s, &key_name(c.key as u32), cap);
        s
    }

    /// ui_cmd_button(id, ST_*) -> EV_*: button for a command: icon and label from the table
    /// (ST_ICON_ONLY: icon only), tooltip "Label (Strg+S)", ST_DISABLED / ST_CHECKED from the
    /// command state; a click queues the command.  Hidden commands draw nothing.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn cmd_button(&mut self, id: u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let Some(c) = self.cmd_find(id) else { return 0 };
        let cf = self.cmd_flags(id);
        if cf & CF_HIDDEN != 0 {
            return 0;
        }
        let mut f = flags;
        if cf & CF_DISABLED != 0 {
            f |= ST_DISABLED;
        }
        if cf & CF_CHECKED != 0 {
            f |= ST_CHECKED;
        }
        self.next_flags(f);
        if c.icon != 0 {
            self.next_icon(c.icon);
        }
        let lab = strip_amp(c.label).0;
        self.wp.popup.tip_suppress = true;
        let ev = self.button(id, &lab);
        self.wp.popup.tip_suppress = false;
        if ev & EV_ACT != 0 {
            self.cmd_run(id, 0);
        }
        // tooltip: label (accelerator)
        let mut tip = String::new();
        cat(&mut tip, &lab, LAB_MAX);
        let acc = self.accel_fmt(c, 64);
        if !acc.is_empty() {
            cat(&mut tip, " (", LAB_MAX);
            cat(&mut tip, &acc, LAB_MAX);
            cat(&mut tip, ")", LAB_MAX);
        }
        self.tip_core(&tip);
        ev
    }

    // ------------------------------------------------------------------ menu rows

    // mitem(id, label, icon, accel, mflags) -> EV: one menu / list row (32 DIP): icon or
    // check column, label, accelerator right-aligned, submenu chevron
    fn mitem(&mut self, id: u32, label: &str, icon: u32, accel: Option<&str>, mflags: u32) -> u32 {
        let mut mf = mflags;
        // (U1) WinUI: a flyout with a checked row that has an icon shows a check column and
        // an icon column (MIF_TWOCOL); the flag lives in the flyout's popup entry
        let lv = self.wp.popup.fl_level.wrapping_sub(1);
        if (lv as i32) >= 0 && mf & MIF_LIST == 0 && lv < self.wp.popup.pp_n {
            let e = &mut self.wp.popup.pp[lv as usize];
            if mf & MIF_CHECKED != 0 && icon != 0 && e.flags & PPF_TWOCOL == 0 {
                e.flags |= PPF_TWOCOL;
                self.repass = true;
            }
            if e.flags & PPF_TWOCOL != 0 {
                mf |= MIF_TWOCOL;
            }
        }
        let lab = strip_amp(label).0;
        let st = self.style;
        let mut sp = Sp { label: Some(&lab), label2: accel, flags: mf, ..Sp::default() };
        let (w, h) = st.menu_item_size(self, &mut sp);
        let r = self.place(w, h);
        let mut hb = HB_FOCUS;
        if mf & MIF_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        sp.place(r);
        sp.ev = ev;
        sp.flags = mf;
        sp.icon = icon;
        st.menu_item(self, &sp);
        let mut s = self.wp.ctx.last_state;
        if mf & MIF_CHECKED != 0 {
            s |= SS_CHECKED as u32;
        }
        if mf & MIF_OPEN != 0 {
            s |= SS_OPEN as u32;
        }
        if mf & MIF_SELECTED != 0 {
            s |= SS_SELECTED as u32;
        }
        self.wp.ctx.last_state = s;
        let kind = if mf & MIF_LIST != 0 { K_LISTITEM } else { K_MENUITEM };
        self.sem(kind, id, Some(&lab), 0);
        ev
    }

    // mcmd(cmd, id) -> EV: menu row `id` for a command; activation queues it and closes
    // every popup.  Hidden commands draw nothing (0).
    fn mcmd(&mut self, cmd: u32, id: u32) -> u32 {
        let c = self.cmd_find(cmd);
        let cf = self.cmd_flags(cmd);
        if cf & CF_HIDDEN != 0 {
            return 0;
        }
        let acc = match c {
            Some(c) => self.accel_fmt(c, 64),
            None => String::new(),
        };
        let mut mf = 0;
        if cf & CF_DISABLED != 0 {
            mf |= MIF_DISABLED;
        }
        if cf & CF_CHECKED != 0 {
            mf |= MIF_CHECKED;
        }
        let (label, icon) = match c {
            Some(c) => (c.label, c.icon),
            None => ("?", 0),
        };
        let ev = self.mitem(id, label, icon, Some(&acc), mf);
        if ev & EV_ACT != 0 {
            self.cmd_run(cmd, 0);
            self.pp_dismiss_all();
        }
        ev
    }

    /// ui_menu_item(id, label) -> EV_*: menu row inside a flyout (next_icon; next_flags
    /// ST_DISABLED / ST_CHECKED); activation closes every popup
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menu_item(&mut self, id: u32, label: &str) -> u32 {
        let _src = self.src_scope();
        let f = self.take_flags();
        let mut mf = 0;
        if f & ST_DISABLED != 0 {
            mf |= MIF_DISABLED;
        }
        if f & ST_CHECKED != 0 {
            mf |= MIF_CHECKED;
        }
        let icon = self.take_icon();
        let ev = self.mitem(id, label, icon, None, mf);
        if ev & EV_ACT != 0 {
            self.pp_dismiss_all();
        }
        ev
    }

    /// ui_menu_cmd(cmd) -> EV_*: menu row for a command (widget id = command id)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menu_cmd(&mut self, cmd: u32) -> u32 {
        let _src = self.src_scope();
        self.mcmd(cmd, cmd)
    }

    /// ui_menu_sep -> 0: separator line across the flyout
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menu_sep(&mut self) -> u32 {
        let _src = self.src_scope();
        let st = self.style;
        let h = st.menu_sep_size(self);
        let r = self.place(0, h);
        let mut sp = Sp::default();
        sp.place(r);
        st.menu_sep(self, &sp);
        0
    }

    /// the rows of the Mi::List menus with this base id (row k: id base + k; the asm app's
    /// STRLIST buffer); the list ends at the first empty string
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menu_list<T: AsRef<str>>(&mut self, base: u32, rows: &[T]) {
        let _src = self.src_scope();
        let v: Vec<String> = rows.iter().map(|s| String::from(s.as_ref())).collect();
        let l = &mut self.m.popup.lists;
        match l.iter_mut().find(|e| e.0 == base) {
            Some(e) => e.1 = v,
            None => l.push((base, v)),
        }
    }

    // row_mnemonic(rec) -> mnemonic of a menu record's label (0 = none)
    fn row_mnemonic(&self, rec: &Mi) -> u32 {
        let label = match rec {
            Mi::Cmd(c) => self.cmd_find(*c).map(|c| c.label),
            r => r.label(),
        };
        strip_amp(label.unwrap_or("")).1
    }

    // menu_flyout(menu, pid, flags) -> open: the flyout of a menu with its submenus (drawn
    // next to their rows) and keyboard handling
    fn menu_flyout(&mut self, menu: &'static [Mi], pid: u32, flags: u32) -> bool {
        let mut rows = 0usize;
        let mut ids = [0u32; MENU_MAX];
        let mut recs = [0u8; MENU_MAX];
        let mut activated = false;
        let open = self.flyout_begin(pid, flags);
        if !open {
            self.flyout_end();
            return false;
        }
        let level = self.pp_find(pid) as u32;
        // .mf_remember: add a row to the keyboard list
        let remember = |rows: &mut usize, ids: &mut [u32; MENU_MAX], recs: &mut [u8; MENU_MAX], id: u32, ri: usize| {
            if *rows < MENU_MAX {
                ids[*rows] = id;
                recs[*rows] = ri as u8;
                *rows += 1;
            }
        };
        for (ri, rec) in menu.iter().enumerate() {
            if ri >= MENU_MAX {
                self.limit_hit(super::ctx::LIM_MENU_ROWS);
                break;
            }
            let rid = pid.wrapping_add(ri as u32 + 1); // row id
            let ev = match *rec {
                Mi::Sep => {
                    self.menu_sep();
                    continue;
                }
                Mi::Cmd(c) => {
                    if self.cmd_flags(c) & CF_HIDDEN != 0 {
                        continue;
                    }
                    self.mcmd(c, rid)
                }
                Mi::Item(id, label, icon) => {
                    let ev = self.mitem(rid, label, icon, None, 0);
                    if ev & EV_ACT != 0 {
                        self.cmd_run(id, 0);
                        self.pp_dismiss_all();
                    }
                    ev
                }
                Mi::Sub(label, sub) => {
                    let spid = menu_pid(sub, rid); // submenu popup id
                    let sopen = self.pp_find(spid) >= 0;
                    let mf = if sopen { MIF_SUB | MIF_OPEN } else { MIF_SUB };
                    let ev = self.mitem(rid, label, 0, None, mf);
                    remember(&mut rows, &mut ids, &mut recs, rid, ri);
                    if !sopen {
                        // entering the row, a click or Enter opens it (Enter: focus on its
                        // first row)
                        let entered = ev & EV_HOVER != 0 && self.wp.ctx.hot != self.wp.popup.hot_prev;
                        if ev & EV_ACT == 0 && !entered {
                            continue;
                        }
                        self.popup_open(spid);
                        if ev & EV_ACT != 0 && self.wp.ctx.focus_visible {
                            self.pp_flag(spid, PPF_FOCUS);
                        }
                    }
                    self.menu_flyout(sub, spid, FL_RIGHT);
                    continue;
                }
                Mi::List(base, max, empty) => {
                    // (U1) one row per string of the list (widget id = command id base + k);
                    // an empty list shows its placeholder, disabled
                    let items: Vec<String> =
                        self.m.popup.lists.iter().find(|e| e.0 == base).map_or(Vec::new(), |e| e.1.clone());
                    let mut k = 0u32;
                    for s in items.iter() {
                        if s.is_empty() || k >= max as u32 {
                            break;
                        }
                        let id = base.wrapping_add(k);
                        let ev = self.mitem(id, s, 0, None, 0);
                        if ev & EV_ACT != 0 {
                            activated = true;
                            self.cmd_run(id, 0);
                            self.pp_dismiss_all();
                        } else if ev & EV_HOVER != 0
                            && self.wp.ctx.hot != self.wp.popup.hot_prev
                            && level + 1 < self.wp.popup.pp_n
                        {
                            // entering a row closes the open submenu of this menu
                            self.pp_close_to(level + 1);
                        }
                        k += 1;
                    }
                    if k == 0 && !empty.is_empty() {
                        self.mitem(rid, empty, 0, None, MIF_DISABLED); // (P1) "": no row
                    }
                    continue;
                }
                Mi::Menu(..) => continue,
            };
            // .mf_row: Cmd / Item rows
            if ev & EV_ACT != 0 {
                activated = true;
            }
            // entering a row closes the open submenu of this menu
            if ev & EV_HOVER != 0 && self.wp.ctx.hot != self.wp.popup.hot_prev && level + 1 < self.wp.popup.pp_n {
                self.pp_close_to(level + 1);
            }
            if self.wp.ctx.last_state & SS_DISABLED as u32 == 0 {
                remember(&mut rows, &mut ids, &mut recs, rid, ri);
            }
        }
        // keys go to the top popup only
        'keys: {
            if activated || self.wp.popup.pp_n.wrapping_sub(1) != level {
                break 'keys;
            }
            // the focused row
            let focus = self.wp.ctx.focus;
            let sel = (0..rows).rev().find(|&j| ids[j] == focus).map(|j| (ids[j], recs[j] as usize));
            // Right on a submenu row opens it with the focus inside
            if let Some((rid, ri)) = sel {
                if let Mi::Sub(_, sub) = menu[ri] {
                    if self.key(VK_RIGHT) {
                        self.mf_open_sub(sub, rid);
                        break 'keys;
                    }
                }
            }
            // Left closes a submenu (the focus returns to its row)
            if level != 0 && self.key(VK_LEFT) {
                self.pp_close_to(level);
                self.wp.ctx.focus_visible = true;
                break 'keys;
            }
            // a letter chooses the row with that mnemonic
            let k = self.key_first();
            if k == 0 || k & (MOD_CTRL << 16) != 0 {
                break 'keys;
            }
            let vk = k & 0xFFFF;
            if !is_alnum_vk(vk) {
                break 'keys;
            }
            for j in 0..rows {
                let rec = menu[recs[j] as usize];
                if self.row_mnemonic(&rec) != vk {
                    continue;
                }
                self.key(vk);
                let rid = ids[j];
                if let Mi::Sub(_, sub) = rec {
                    self.focus(rid);
                    self.mf_open_sub(sub, rid);
                } else {
                    self.cmd_run(rec.arg(), 0);
                    self.pp_dismiss_all();
                }
                break;
            }
        }
        self.flyout_end();
        true
    }

    // .mf_open_sub(submenu, row id): open it as a child with the focus on its first row
    fn mf_open_sub(&mut self, sub: &'static [Mi], rid: u32) {
        let pid = menu_pid(sub, rid);
        self.popup_open(pid);
        self.pp_flag(pid, PPF_FOCUS);
    }

    // ------------------------------------------------------------------ menu bar

    // the menus of a menu bar (its MENU records)
    fn mb_menu(mb: &'static [Mi], i: usize) -> &'static [Mi] {
        match mb[i] {
            Mi::Menu(_, m) | Mi::Sub(_, m) => m,
            _ => &[],
        }
    }

    // mb_open_index(menubar, base, n) -> index of the menu of the bar that is open (-1)
    fn mb_open_index(&self, mb: &'static [Mi], base: u32, n: usize) -> i32 {
        if self.wp.popup.pp_n == 0 {
            return -1;
        }
        let top = self.wp.popup.pp[0].id;
        (0..n).find(|&i| menu_pid(Self::mb_menu(mb, i), base.wrapping_add(i as u32)) == top).map_or(-1, |i| i as i32)
    }

    // mb_open(menubar, base, i, focus): open menu i of the bar (replacing every popup)
    fn mb_open(&mut self, mb: &'static [Mi], base: u32, i: usize, focus: bool) {
        let pid = menu_pid(Self::mb_menu(mb, i), base.wrapping_add(i as u32));
        // the focus to return to: the bar item in menu bar mode, else the previous one
        let w = &mut self.wp.popup;
        let mut ret = self.wp.ctx.focus;
        if w.pp_n != 0 {
            ret = w.pp[0].ret;
        }
        if w.mb_mode {
            ret = base.wrapping_add(i as u32);
        }
        w.pp_n = 0;
        let e = self.pp_push(pid, 0);
        let e = &mut self.wp.popup.pp[e];
        e.ret = ret;
        if focus {
            e.flags |= PPF_FOCUS;
        }
    }

    /// ui_menubar(menubar) -> EV_OPEN while one of its menus is open (None = the one of
    /// AppDesc::menubar).  WinUI MenuBar: subtle 32 DIP items, a flyout per menu.
    /// Keyboard: Alt (tap) or F10 enter / leave menu bar mode, Alt+mnemonic opens a menu,
    /// Left / Right move between the menus, Down / Enter open one, Esc steps back.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menubar(&mut self, menubar: Option<&'static [Mi]>) -> u32 {
        let _src = self.src_scope();
        let mb: &'static [Mi] = match menubar {
            Some(m) => m,
            None => self.desc.menubar,
        };
        if mb.is_empty() {
            return 0;
        }
        let a = (mb.as_ptr() as usize as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let base = ((a >> 36) as u32 & 0x0FFF_FF00) | 0xC000_0000;
        // count the menus, collect the mnemonics
        let mut mn = [0u8; MB_MAX];
        let mut mpos = [0u8; MB_MAX];
        let mut n = 0usize;
        while n < mb.len() && n < MB_MAX {
            let Mi::Menu(label, _) = mb[n] else { break };
            let (_, m, pos) = strip_amp(label);
            mn[n] = m as u8;
            mpos[n] = pos as u8;
            n += 1;
        }
        if n == MB_MAX && matches!(mb.get(n), Some(Mi::Menu(..))) {
            self.limit_hit(super::ctx::LIM_MENUBAR);
        }
        if n == 0 {
            return 0;
        }
        let mut open = self.mb_open_index(mb, base, n);
        let fi0 = self.wp.ctx.focus.wrapping_sub(base);
        let mut fi: i32 = if (fi0 as usize) < n { fi0 as i32 } else { -1 };
        // menu bar mode ends when the focus went elsewhere
        if self.wp.popup.mb_mode && fi < 0 && self.wp.popup.pp_n == 0 {
            self.wp.popup.mb_mode = false;
        }
        'keys: {
            if self.modal_active() != 0 {
                break 'keys; // no menu keys while a dialog is open
            }
            let mut toggle = false;
            let k = self.key_first();
            let vk = k & 0xFFFF;
            let allm = k >> 16; // all modifiers (AltGr too)
            let m = allm & MOD3;
            // ---- Alt tap: Alt pressed and released without anything in between
            if vk == VK_MENU {
                self.wp.popup.alt_armed = allm & (MOD_CTRL | MOD_SHIFT | MOD_ALTGR) == 0;
            } else if vk != 0 || self.wp.ctx.pressed & BTN_MASK != 0 {
                self.wp.popup.alt_armed = false;
            } else if self.wp.popup.alt_armed && !self.key_down(VK_MENU) {
                self.wp.popup.alt_armed = false;
                if self.headless || plat::window_state() & plat::WST_ACTIVE != 0 {
                    toggle = true;
                }
            }
            if vk == VK_F10 && m == 0 {
                self.key(VK_F10);
                toggle = true;
            }
            if toggle {
                if !self.wp.popup.mb_mode && open < 0 {
                    // enter menu bar mode: the focus goes to the first menu
                    let w = &mut self.wp.popup;
                    let mut f = self.wp.ctx.focus;
                    if w.pp_n != 0 {
                        f = w.pp[0].ret;
                        w.pp_n = 0;
                    }
                    w.mb_prev = f;
                    w.mb_mode = true;
                    self.focus(base);
                    fi = 0;
                    open = -1;
                } else {
                    self.pp_dismiss_all();
                    open = -1;
                    fi = -1;
                }
                break 'keys;
            }
            // ---- Alt+mnemonic opens a menu
            if m & MOD_ALT != 0 {
                if m & MOD_CTRL != 0 {
                    break 'keys;
                }
                let i = mnem_find(&mn[..n], vk);
                if i < 0 {
                    break 'keys;
                }
                self.key(vk);
                if !self.wp.popup.mb_mode {
                    let w = &mut self.wp.popup;
                    let mut f = self.wp.ctx.focus;
                    if w.pp_n != 0 {
                        f = w.pp[0].ret;
                    }
                    w.mb_prev = f;
                    w.mb_mode = true;
                }
                self.mb_open(mb, base, i as usize, true);
                open = i;
                break 'keys;
            }
            // ---- menu bar mode without an open menu
            if fi < 0 || open >= 0 || vk == 0 || m & (MOD_CTRL | MOD_ALT) != 0 {
                break 'keys;
            }
            let mut i = fi;
            match vk {
                VK_LEFT | VK_RIGHT => {
                    self.key(vk);
                    if vk == VK_LEFT {
                        i = if fi - 1 < 0 { n as i32 - 1 } else { fi - 1 };
                    } else {
                        i = if fi + 1 < n as i32 { fi + 1 } else { 0 };
                    }
                    fi = i;
                    self.focus(base.wrapping_add(i as u32));
                    break 'keys;
                }
                VK_ESCAPE => {
                    self.key(vk);
                    self.wp.popup.mb_mode = false;
                    self.wp.ctx.focus = self.wp.popup.mb_prev;
                    self.repass();
                    fi = -1;
                    break 'keys;
                }
                VK_DOWN | VK_UP | VK_RETURN | VK_SPACE => {}
                _ => {
                    i = mnem_find(&mn[..n], vk);
                    if i < 0 {
                        break 'keys;
                    }
                }
            }
            self.key(vk);
            self.mb_open(mb, base, i as usize, true);
            open = i;
        }
        // a press outside the open menu closes it
        if open >= 0 && self.popup_catcher() {
            open = -1;
            fi = -1;
        }
        self.box_begin(base | 0xFF, BOX_H | BOX_XCENTER | PAD_4, 4);
        let st = self.style;
        let mut rects = [[0i32; 4]; MB_MAX];
        for i in 0..n {
            let Mi::Menu(label, _) = mb[i] else { break };
            let lab = strip_amp(label).0;
            let mut sp = Sp { label: Some(&lab), ..Sp::default() };
            let (w, h) = st.menubar_item_size(self, &mut sp);
            let r = self.place(w, h);
            rects[i] = r;
            let id = base.wrapping_add(i as u32); // item id
            let ev = self.behave(id, 0);
            let state = self.wp.ctx.last_state;
            // while a menu is open the items stay reachable above the popup catcher
            if open >= 0 {
                let prev = self.layer_set(L_POPUP);
                self.hit(id, 0);
                self.layer_set(prev);
            }
            // press: open / close; hover: switch menus while one is open
            let c = &self.wp.ctx;
            if ev & EV_HELD != 0 && c.pressed & (1 << BTN_LEFT) != 0 && c.hot == id {
                if open == i as i32 {
                    self.pp_dismiss_all();
                    open = -1;
                } else {
                    self.mb_open(mb, base, i, false);
                    open = i as i32;
                }
            } else if ev & EV_HOVER != 0 && open >= 0 && open != i as i32 && c.hot != self.wp.popup.hot_prev {
                // the pointer just entered it
                self.mb_open(mb, base, i, false);
                open = i as i32;
            }
            // paint (style)
            sp.place(self.wp.ctx.last_rect);
            sp.ev = ev;
            let mut f = 0;
            if open == i as i32 {
                f |= 1; // its menu is open
            }
            if fi == i as i32 {
                f |= 2; // the menu bar's current item
            }
            sp.flags = f;
            sp.value = -1; // mnemonic underline: byte index or -1
            if self.wp.popup.mb_mode && mpos[i] != 0xFF {
                sp.value = mpos[i] as i32;
            }
            st.menubar_item(self, &sp);
            if fi == i as i32 && open < 0 {
                self.focus_ring_draw(ev);
            }
            let mut s = state;
            if open == i as i32 {
                s |= SS_OPEN as u32;
            }
            self.wp.ctx.last_state = s;
            self.sem(K_MENU, id, Some(&lab), 0);
        }
        self.box_end();
        // the open menu's flyout
        if open >= 0 {
            let oi = open as usize;
            self.wp.ctx.last_rect = rects[oi];
            let menu = Self::mb_menu(mb, oi);
            let pid = menu_pid(menu, base.wrapping_add(oi as u32));
            self.menu_flyout(menu, pid, FL_BELOW);
            // Left / Right not used by the menu switch to the neighbour menu
            if self.wp.popup.pp_n == 1 && self.wp.popup.pp[0].id == pid {
                let d = if self.key(VK_LEFT) {
                    -1
                } else if self.key(VK_RIGHT) {
                    1
                } else {
                    0
                };
                if d != 0 {
                    let mut j = open + d;
                    if j < 0 {
                        j = n as i32 - 1;
                    }
                    if j >= n as i32 {
                        j = 0;
                    }
                    open = j;
                    self.mb_open(mb, base, j as usize, true);
                }
            }
        }
        if open >= 0 { EV_OPEN } else { 0 }
    }

    // ------------------------------------------------------------------ context menus

    // ctx_open(menu, x, y, focus): open `menu` as a runtime-drawn popup at (x, y) px
    fn ctx_open(&mut self, menu: &'static [Mi], x: i32, y: i32, focus: bool) {
        let pid = menu_pid(menu, CTX_SALT);
        let w = &mut self.wp.popup;
        let mut ret = self.wp.ctx.focus;
        if w.pp_n != 0 {
            ret = w.pp[0].ret;
            w.pp_n = 0;
        }
        let i = self.pp_push(pid, 0);
        let e = &mut self.wp.popup.pp[i];
        e.ret = ret;
        e.menu = Some(menu);
        e.flags |= PPF_AUTO | PPF_POINT | if focus { PPF_FOCUS } else { 0 };
        e.anchor = [x, y, 0, 0];
    }

    /// ui_menu_popup(menu): open a menu at the pointer (the runtime draws it); for the
    /// menu key / Shift+F10 (an EV_RCLICK kbnav.rs gave the focused widget in this pass) at
    /// the widget's lower left corner with its first row focused
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn menu_popup(&mut self, menu: &'static [Mi]) {
        let _src = self.src_scope();
        self.wp.popup.ctx_target = 0;
        if let Some(r) = self.kb_ctx_rect() {
            self.ctx_open(menu, r[0], r[1] + r[3], true);
            return;
        }
        let (x, y) = (self.wp.ctx.mx, self.wp.ctx.my);
        self.ctx_open(menu, x, y, false);
    }

    /// ui_context_menu(menu) -> EV_OPEN while `menu` is open for the last widget.  Opens on a
    /// right click (released over the widget), or with the menu key / Shift+F10 while the
    /// widget has the focus -- a list row: while the list has it and the row is selected --
    /// (then at its lower left corner, first row focused).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn context_menu(&mut self, menu: &'static [Mi]) -> u32 {
        let _src = self.src_scope();
        let (id, r) = self.last_widget();
        if id == 0 {
            return 0;
        }
        let c = &self.wp.ctx;
        if c.hot == id && c.released & (1 << BTN_RIGHT) != 0 {
            let (x, y) = (c.mx, c.my);
            self.ctx_open(menu, x, y, false);
            self.wp.popup.ctx_target = id;
        } else if self.wp.ctx.focus == id || self.list_row_focused(id) {
            let mut open = self.key(VK_APPS);
            if !open {
                let k = self.key_first();
                if k & 0xFFFF == VK_F10 && (k >> 16) & MOD3 == MOD_SHIFT {
                    self.key(VK_F10);
                    open = true;
                }
            }
            if open {
                self.ctx_open(menu, r[0], r[1] + r[3], true);
                self.wp.popup.ctx_target = id;
            }
        }
        if self.pp_find(menu_pid(menu, CTX_SALT)) < 0 || self.wp.popup.ctx_target != id {
            return 0;
        }
        EV_OPEN
    }

    /// ui_context_target -> id of the widget whose context menu is (or was last) open
    pub fn context_target(&self) -> u32 {
        self.wp.popup.ctx_target
    }

    // ------------------------------------------------------------------ drop-down controls

    /// ui_dropdown_button(id, label) -> EV_* | EV_OPEN while popup `id` is open (a click,
    /// Enter / Space, Alt+Down or F4 opens it; the app draws the flyout with
    /// flyout_begin(id, ..) right after).  next_flags ST_SUBTLE: flat tool bar look,
    /// ST_ICON_ONLY: icon + chevron, the label as tooltip (U1)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dropdown_button(&mut self, id: u32, label: &str) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags();
        let icon = self.take_icon();
        let st = self.style;
        let mut sp = Sp { flags, icon, label: Some(label), ..Sp::default() };
        let (w, h) = st.dropdown_size(self, &mut sp);
        let (w, h) = self.ctrl_size(w, h, flags, false); // (theme) control height
        self.place(w, h);
        let mut hb = HB_FOCUS;
        if flags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        let state = self.wp.ctx.last_state;
        let mut open = self.popup_is_open(id);
        if !open {
            let mut go = ev & EV_ACT != 0;
            if !go && ev & EV_FOCUS != 0 && self.open_key() {
                self.wp.ctx.focus_visible = true;
                go = true;
            }
            if go {
                self.popup_open(id);
                open = true;
                if self.wp.ctx.focus_visible {
                    self.pp_flag(id, PPF_FOCUS);
                }
            }
        }
        sp.place(self.wp.ctx.last_rect);
        sp.ev = ev;
        sp.value = open as i32;
        sp.value2 = (state == SS_DISABLED as u32) as i32;
        st.dropdown(self, &sp);
        self.focus_ring_draw(ev);
        self.wp.ctx.last_state = state | if open { SS_OPEN as u32 } else { 0 };
        self.sem(K_BUTTON, id, Some(label), 0);
        if flags & ST_ICON_ONLY != 0 && !open {
            self.tooltip(label);
        }
        if open { ev | EV_OPEN } else { ev }
    }

    /// ui_split_button(id, label) -> EV_* of the main part | EV_OPEN while popup `id` is
    /// open.  The chevron part (widget id | 0x80000000; Alt+Down / F4 on the main part)
    /// opens it.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn split_button(&mut self, id: u32, label: &str) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags();
        let icon = self.take_icon();
        let st = self.style;
        let mut sp = Sp { icon, label: Some(label), ..Sp::default() };
        let (w, h) = st.split_size(self, &mut sp);
        let (w, h) = self.ctrl_size(w, h, flags, false); // (theme) control height
        let (mw, cw) = (sp.s[0], sp.s[1]);
        let whole = self.place(w, h);
        let dis = if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        // chevron part
        self.wp.ctx.last_rect[0] += mw;
        self.wp.ctx.last_rect[2] = cw;
        let evc = self.behave(id | 0x8000_0000, dis);
        let stc = self.wp.ctx.last_state;
        // main part
        self.wp.ctx.last_rect[0] = whole[0];
        self.wp.ctx.last_rect[2] = mw;
        let evm = self.behave(id, dis | HB_FOCUS);
        let stm = self.wp.ctx.last_state;
        let mut open = self.popup_is_open(id);
        if !open {
            if evc & EV_ACT != 0 {
                self.popup_open(id);
                open = true;
            } else if evm & EV_FOCUS != 0 && self.open_key() {
                self.wp.ctx.focus_visible = true;
                self.popup_open(id);
                self.pp_flag(id, PPF_FOCUS);
                open = true;
            }
        }
        sp.place(whole);
        sp.ev = evm; // main part
        sp.s[2] = evc as i32; // chevron part
        sp.value = open as i32;
        sp.value2 = (stm == SS_DISABLED as u32) as i32 | ((stc == SS_DISABLED as u32) as i32) << 1;
        sp.s[0] = mw;
        sp.s[1] = cw;
        st.split(self, &sp);
        let os = if open { SS_OPEN as u32 } else { 0 };
        // semantic records: chevron first, so the main part is the "last widget"
        self.wp.ctx.last_rect[0] = whole[0] + mw;
        self.wp.ctx.last_rect[2] = cw;
        self.wp.ctx.last_state = stc | os;
        self.sem(K_BUTTON, id | 0x8000_0000, None, 0);
        // focus ring and record of the main part
        self.wp.ctx.last_rect = whole;
        self.focus_ring_draw(evm);
        self.wp.ctx.last_rect[2] = mw;
        self.wp.ctx.last_state = stm | os;
        self.sem(K_BUTTON, id, Some(label), 0);
        self.wp.ctx.last_rect = whole;
        if open { evm | EV_OPEN } else { evm }
    }

    /// ui_combobox(id, &mut sel, list) -> EV_*: EV_ACT when sel changed, EV_OPEN while the
    /// list is open.  WinUI ComboBox: 32 DIP, control fill, chevron; the list opens over the
    /// box with the selected row (accent pill) on it.  Keyboard: Up / Down change the
    /// selection, Enter / Space / Alt+Down / F4 open the list.  `list` is a STRLIST (it ends
    /// at the first empty string).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn combobox(&mut self, id: u32, sel: &mut i32, list: &[&str]) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags();
        let mut changed = false;
        // rows and the widest text
        self.ts_font(TS_BODY);
        let n = strlist_n(list);
        let mut widest = 0;
        for s in &list[..n] {
            let w = self.text.width(s);
            if w > widest {
                widest = w;
            }
        }
        let st = self.style;
        let mut sp = Sp { flags, value: widest, ..Sp::default() };
        let (w, h) = st.combo_size(self, &mut sp);
        let (w, h) = self.ctrl_size(w, h, flags, false); // (theme) control height
        let rect = self.place(w, h);
        let mut hb = HB_FOCUS;
        if flags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        let state = self.wp.ctx.last_state;
        let mut open = self.popup_is_open(id);
        let rowbase = (id.wrapping_mul(0x9E37_79B1) & 0x0FFF_FF00) | 0xD000_0000;
        if !open {
            let mut go = ev & EV_ACT != 0;
            if !go && ev & EV_FOCUS != 0 {
                if self.open_key() {
                    self.wp.ctx.focus_visible = true;
                    go = true;
                } else if self.key(VK_DOWN) {
                    let v = sel.wrapping_add(1);
                    if v < n as i32 {
                        *sel = v;
                        changed = true;
                        self.wp.ctx.focus_visible = true;
                    }
                } else if self.key(VK_UP) {
                    let v = sel.wrapping_sub(1);
                    if v >= 0 {
                        *sel = v;
                        changed = true;
                        self.wp.ctx.focus_visible = true;
                    }
                }
            }
            if go {
                self.popup_open(id);
                open = true;
                // focus the selected row once the list is drawn
                let i = self.pp_find(id);
                if i >= 0 {
                    let c = *sel;
                    if c >= 0 && c < n as i32 {
                        let fv = self.wp.ctx.focus_visible;
                        let e = &mut self.wp.popup.pp[i as usize];
                        e.focusid = rowbase.wrapping_add(c as u32 + 1);
                        if !fv {
                            e.flags |= PPF_QUIET;
                        }
                    }
                }
            }
        }
        // paint (style)
        let mut sp = Sp { flags, label: strlist_at(list, *sel), ev, value2: (state == SS_DISABLED as u32) as i32, ..Sp::default() };
        sp.place(rect);
        st.combo(self, &sp);
        self.focus_ring_draw(ev);
        // the list
        if open {
            let r = if *sel < 0 || *sel >= n as i32 { 0 } else { *sel };
            let ih = st.menu_item_h(self);
            let pad = st.flyout_pad(self);
            self.wp.popup.fl_offy = r.wrapping_mul(ih) + pad;
            // anchor: the box widened by the flyout padding so the texts line up
            self.wp.ctx.last_rect = [rect[0] - pad, rect[1], rect[2] + pad * 2, rect[3]];
            if self.flyout_begin(id, FL_OVER | FL_MINW) {
                for (k, s) in list[..n].iter().enumerate() {
                    let k = k as i32;
                    let mut mf = MIF_LIST;
                    if k == *sel {
                        mf |= MIF_SELECTED;
                    }
                    let e = self.mitem(rowbase.wrapping_add(k as u32 + 1), s, 0, None, mf);
                    if e & EV_ACT != 0 {
                        if k != *sel {
                            *sel = k;
                            changed = true;
                        }
                        let i = self.pp_find(id);
                        if i >= 0 {
                            self.pp_close_to(i as u32);
                        }
                    }
                }
            }
            self.flyout_end();
            self.wp.ctx.last_rect = rect;
        }
        let lab = strlist_at(list, *sel);
        let open = self.popup_is_open(id);
        self.wp.ctx.last_state = state | if open { SS_OPEN as u32 } else { 0 };
        let v = *sel;
        self.sem(K_COMBO, id, lab, v).max = n as i32;
        let mut r = ev & !EV_ACT;
        if changed {
            r |= EV_ACT;
            self.repass();
        }
        if open {
            r |= EV_OPEN;
        }
        r
    }

    // ------------------------------------------------------------------ tooltips

    /// ui_tooltip(s): post-modifier: show s as the tooltip of the last widget after 500 ms of
    /// hover (at once when another tooltip hid less than 300 ms ago); hidden on press, key or
    /// leave.  Icon-only buttons call it with their label.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn tooltip(&mut self, s: &str) {
        let _src = self.src_scope();
        if self.wp.popup.tip_suppress {
            self.wp.popup.tip_suppress = false;
            return;
        }
        self.tip_core(s);
    }

    /// ui_tooltip_part(s, part): post-modifier like tooltip(s) for one part of the last widget
    /// (an item on a canvas: a cable, a port).  The delay starts again when `part` changes (at
    /// once when a tooltip hid less than 300 ms ago, as between toolbar buttons); a press hides
    /// it until the pointer is over another part; part 0 shows nothing.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn tooltip_part(&mut self, s: &str, part: u64) {
        let _src = self.src_scope();
        let (id, _) = self.last_widget();
        if id == 0 {
            return;
        }
        if self.wp.ctx.hot != id || part == 0 {
            if self.wp.popup.tip_id == id {
                self.tip_reset();
            }
            self.wp.popup.tip_part = 0;
            return;
        }
        if self.wp.popup.tip_part != part {
            // another part: its tooltip starts over, a press on the last one no longer counts
            let w = &mut self.wp.popup;
            w.tip_part = part;
            if w.tip_block == id {
                w.tip_block = 0;
            }
            if w.tip_id == id {
                self.tip_reset();
            }
        }
        self.tip_core(s);
    }

    // tip_core(s)
    fn tip_core(&mut self, s: &str) {
        let (id, _) = self.last_widget();
        if id == 0 {
            return;
        }
        let w = &mut self.wp.popup;
        if self.wp.ctx.hot != id {
            if w.tip_block == id {
                w.tip_block = 0;
            }
            if w.tip_id == id {
                self.tip_reset();
            }
            return;
        }
        if w.tip_block == id {
            return;
        }
        if self.wp.ctx.pressed & BTN_MASK != 0 || self.key_first() != 0 {
            let w = &mut self.wp.popup;
            w.tip_block = id;
            w.tip_id = 0;
            w.tip_shown = 0;
            return;
        }
        let now = self.now_ms;
        let w = &mut self.wp.popup;
        if w.tip_id != id {
            w.tip_id = id;
            w.tip_t0 = now;
            if w.tip_shown != 0 || (w.tip_hid_ok && now.wrapping_sub(w.tip_hid_t) < TIP_QUICK) {
                w.tip_t0 = now.wrapping_sub(TIP_DELAY); // another tooltip is (just) up: at once
            }
        }
        let el = now.wrapping_sub(w.tip_t0);
        if el < TIP_DELAY {
            self.timer(UIT_TOOLTIP, TIP_DELAY - el);
            return;
        }
        w.tip_shown = id;
        w.tip_hid_ok = false;
        self.tip_draw(s);
    }

    // tip_reset: stop tracking the hovered widget (remember when a shown tooltip hid)
    fn tip_reset(&mut self) {
        let now = self.now_ms;
        let w = &mut self.wp.popup;
        w.tip_id = 0;
        if w.tip_shown != 0 {
            w.tip_shown = 0;
            w.tip_hid_t = now;
            w.tip_hid_ok = true;
        }
    }

    // tip_draw(s): the tooltip on L_TIP: below the pointer (above when there is no room),
    // flyout colours, 4 DIP corners, caption text (max 300 DIP)
    fn tip_draw(&mut self, s: &str) {
        let (lr, ls) = (self.wp.ctx.last_rect, self.wp.ctx.last_state);
        let prev = self.layer_set(L_TIP);
        let st = self.style;
        let mut sp = Sp { label: Some(s), ..Sp::default() };
        let (w, h) = st.tooltip_size(self, &mut sp);
        // x centred on the pointer, y below it; inside the popup area (U3, §6: beyond the
        // edges of the window, pp_bounds)
        let area = self.pp_bounds(w, h);
        let (mx, my) = (self.wp.ctx.mx, self.wp.ctx.my);
        let mut x = mx.wrapping_sub(((w as u32) >> 1) as i32);
        let mut y = my + sp.s[1]; // gap below the pointer
        if y + h > area[3] {
            y = my - sp.s[2] - h; // gap above it
        }
        if x > area[2] - w {
            x = area[2] - w;
        }
        if x < area[0] {
            x = area[0];
        }
        if y > area[3] - h {
            y = area[3] - h;
        }
        if y < area[1] {
            y = area[1];
        }
        // (§6) where the platform put it (Wayland)
        self.wp.popup.tip_anchor = [mx, my, 1, 1];
        self.wp.popup.tip_nat = [x, y];
        let (dx, dy) = self.pp_nudge(POPUP_MAX as u32, [x, y, w, h]);
        let (x, y) = (x + dx, y + dy);
        self.wp.popup.tip_rect = [x, y, w, h]; // (U3) for an OS popup window
        self.wp.popup.tip_seen = self.frame;
        sp.place([x, y, w, h]);
        st.tooltip(self, &sp);
        // semantic record (id 0) for the dump and scripts
        self.wp.ctx.last_rect = [x, y, w, h];
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(s), 0);
        self.layer_set(prev);
        self.wp.ctx.last_rect = lr;
        self.wp.ctx.last_state = ls;
    }

    // ------------------------------------------------------------------ runtime hook

    /// ui_accel_dispatch: (start of uiw_pass_end) draws runtime-owned menus (menu_popup /
    /// context_menu), closes popups nobody drew in this pass, ends the tooltip tracking when
    /// the pointer left, and turns an unconsumed key chord that matches a command
    /// accelerator into cmd_run(id, 0)
    pub(crate) fn accel_dispatch(&mut self) {
        // runtime-drawn context menu (level 0)
        let frame = self.frame;
        let w = &self.wp.popup;
        if w.pp_n != 0 && w.pp[0].flags & PPF_AUTO != 0 && w.pp[0].seen != frame {
            let (menu, id) = (w.pp[0].menu.unwrap_or(&[]), w.pp[0].id);
            self.menu_flyout(menu, id, FL_POINT);
        }
        // popups not drawn in this pass (and not opened in it) are gone
        for i in 0..self.wp.popup.pp_n {
            let e = &self.wp.popup.pp[i as usize];
            if e.seen != frame && e.born != frame {
                self.pp_close_to(i);
                break;
            }
        }
        self.wp.popup.hot_prev = self.wp.ctx.hot;
        let t = self.wp.popup.tip_id;
        if t != 0 && t != self.wp.ctx.hot {
            self.tip_reset();
        }
        // accelerators (not in dialog windows)
        if self.pp_noaccel() {
            return;
        }
        let k = self.key_first();
        if k == 0 {
            return;
        }
        let vk = k & 0xFFFF;
        let mods = (k >> 16) & MOD3;
        if vk == VK_SHIFT || vk == VK_CONTROL || vk == VK_MENU {
            return;
        }
        // plain keys belong to a focused text box
        if mods & (MOD_CTRL | MOD_ALT) == 0 && vk.wrapping_sub(VK_F1) > 23 && self.focus_is_text() {
            return;
        }
        let Some(c) = self.accel_find(vk, mods) else { return };
        self.key(vk);
        if self.cmd_flags(c.id) & (CF_DISABLED | CF_HIDDEN) != 0 {
            return;
        }
        self.pp_dismiss_all();
        self.cmd_run(c.id, 0);
    }
}
