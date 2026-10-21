// popup -- the classic flyouts, menus, menu bar, drop-downs, split buttons, combo boxes,
// tooltips                                                         [styles/classic/popup.s]
//
//   flyout_pad / flyout_box / flyout_frame     2 DIP, BOX_V | PAD_2; grey face, raised
//                                              frame (no shadow, square corners)
//   menu_item_h / menu_item_size / menu_item   22 DIP rows: [column 20 (x 2 with
//                                              MIF_TWOCOL)] + label [+ 24 + accelerator]
//                                              + 28, at least 100 DIP; navy highlight
//   menu_sep_size / menu_sep                   8 DIP, an etched line
//   menubar_item_size / menubar_item           label + 14, 20 high; thin frames
//   dropdown_size / dropdown, split_size / split   raised button faces, small down arrow
//   combo_size / combo                         white sunken field, raised arrow button
//   tooltip_size / tooltip                     pale yellow box with a black line
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN_SMALL, IC_CHEV_RIGHT_SMALL};
use ui::{Sp, StylePopup, Ui};

use crate::basic::separator;
use crate::common::*;
use crate::Classic;

const ITEM_H: i32 = 22; // menu row height (DIP)
const COL_W: i32 = 20; // check / icon column (DIP)

impl StylePopup for Classic {
    fn flyout_pad(&self, ui: &mut Ui) -> i32 {
        ui.dp(2)
    }
    fn flyout_box(&self, _ui: &mut Ui) -> u32 {
        BOX_V | PAD_2
    }
    fn flyout_frame(&self, ui: &mut Ui, sp: &Sp) {
        // grey face with a raised 3D frame (no shadow, square corners)
        let c = ui.color(T_FLYOUT);
        fill(ui, sp.rect(), c);
        frame(ui, sp.rect(), FR_RAISED);
    }
    fn menu_item_h(&self, ui: &mut Ui) -> i32 {
        ui.dp(ITEM_H)
    }
    fn menu_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        menu_item_size(ui, sp)
    }
    fn menu_item(&self, ui: &mut Ui, sp: &Sp) {
        menu_item(ui, sp)
    }
    fn menu_sep_size(&self, ui: &mut Ui) -> i32 {
        ui.dp(8)
    }
    fn menu_sep(&self, ui: &mut Ui, sp: &Sp) {
        // etched line across the row: a horizontal separator
        separator(ui, sp.rect(), 1);
    }
    fn menubar_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // label + 14, 20 DIP high
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        let w = tw + ui.dp(14);
        (w, ui.dp(20))
    }
    fn menubar_item(&self, ui: &mut Ui, sp: &Sp) {
        menubar_item(ui, sp)
    }
    fn dropdown_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        dropdown_size(ui, sp)
    }
    fn dropdown(&self, ui: &mut Ui, sp: &Sp) {
        dropdown(ui, sp)
    }
    fn split_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        split_size(ui, sp)
    }
    fn split(&self, ui: &mut Ui, sp: &Sp) {
        split(ui, sp)
    }
    fn combo_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 4 + widest text + 4 + 16 arrow button + 4, at least 64 DIP, 21 high
        let mut w = sp.value.wrapping_add(ui.dp(28));
        let m = ui.dp(64);
        if w < m {
            w = m;
        }
        (w, ui.dp(21))
    }
    fn combo(&self, ui: &mut Ui, sp: &Sp) {
        combo(ui, sp)
    }
    fn tooltip_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        tooltip_size(ui, sp)
    }
    fn tooltip(&self, ui: &mut Ui, sp: &Sp) {
        tooltip(ui, sp)
    }
}

// ------------------------------------------------------------------ menu rows

/// sty_menu_item_size -> (w, h); keeps s[0] label width, s[1] accelerator width
fn menu_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let f = sp.flags;
    ui.ts_font(TS_BODY);
    let lw = ui.text.width(sp.label.unwrap_or(""));
    sp.s[0] = lw;
    let mut w = lw;
    let aw = match sp.label2 {
        Some(a) if !a.is_empty() && a.as_bytes()[0] != 0 => ui.text.width(a),
        _ => 0,
    };
    sp.s[1] = aw;
    if f & MIF_LIST != 0 {
        // list rows: 4 + label + 4
        w += ui.dp(8);
    } else {
        let c = ui.dp(COL_W);
        w += c;
        if f & MIF_TWOCOL != 0 {
            w += c;
        }
        if aw != 0 {
            w += ui.dp(24);
            w += aw;
        }
        w += ui.dp(28); // arrow column + right padding
        let m = ui.dp(100);
        if w < m {
            w = m;
        }
    }
    (w, ui.dp(ITEM_H))
}

/// sty_menu_item: navy highlight with white text (hover, keyboard focus, open submenu,
/// selected list row), check mark / icon column, label, accelerator, submenu arrow
fn menu_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let f = sp.flags;
    let mut hl = false;
    if f & MIF_DISABLED == 0 && (sp.ev & (EV_HOVER | EV_FOCUS) != 0 || f & (MIF_OPEN | MIF_SELECTED) != 0) {
        hl = true;
        let c = ui.color(T_ACCENT);
        fill(ui, r, c);
    }
    let mut tok = if hl { T_ON_ACCENT } else { T_TEXT };
    if f & MIF_DISABLED != 0 {
        tok = T_TEXT_DISABLED;
    }
    let tc = ui.color(tok);
    // left column: check mark, else the icon (MIF_TWOCOL: check, then icon)
    let mut x = sp.x;
    if f & MIF_LIST == 0 {
        let cw = ui.dp(COL_W);
        let mut col = [x, sp.y, cw, sp.h];
        if f & MIF_CHECKED != 0 {
            icon_mid(ui, col, IC_CHECK_MARK, 10, tc);
        }
        if sp.icon != 0 {
            if f & MIF_TWOCOL != 0 {
                col[0] = col[0].wrapping_add(cw); // two columns: the icon in the second
                icon_mid(ui, col, sp.icon, 16, tc);
            } else if f & MIF_CHECKED == 0 {
                icon_mid(ui, col, sp.icon, 16, tc); // one column: the check mark wins
            }
        }
        x = x.wrapping_add(cw);
        if f & MIF_TWOCOL != 0 {
            x = x.wrapping_add(cw);
        }
    } else {
        x = x.wrapping_add(ui.dp(4));
    }
    ui.ts_font(TS_BODY);
    text_at(ui, x, r, sp.label, tc, TS_BODY);
    // accelerator, right-aligned before the arrow column
    if sp.label2.is_some() && sp.s[1] != 0 {
        let d = ui.dp(28);
        let ax = sp.x.wrapping_add(sp.w).wrapping_sub(d).wrapping_sub(sp.s[1]);
        text_at(ui, ax, r, sp.label2, tc, TS_BODY);
    }
    if f & MIF_SUB != 0 {
        let a = ui.dp(20);
        let ar = [sp.x.wrapping_add(sp.w).wrapping_sub(a), sp.y, a, sp.h];
        icon_mid(ui, ar, IC_CHEV_RIGHT_SMALL, 8, tc);
    }
}

// ------------------------------------------------------------------ menu bar

/// sty_menubar_item: thin sunken while its menu is open, thin raised on hover / as the
/// current item, label 7 DIP from the left, mnemonic underline (value >= 0)
fn menubar_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    if sp.flags & 1 != 0 {
        frame(ui, r, FR_THIN_SUNKEN);
    } else if sp.ev & EV_HOVER != 0 || sp.flags & 2 != 0 {
        frame(ui, r, FR_THIN_RAISED);
    }
    let c = ui.color(T_TEXT);
    ui.ts_font(TS_BODY);
    let mut x = ui.dp(7).wrapping_add(sp.x);
    text_at(ui, x, r, sp.label, c, TS_BODY);
    let m = sp.value;
    if m < 0 {
        return;
    }
    let m = m as usize;
    ui.ts_font(TS_BODY);
    let b = sp.label.unwrap_or("").as_bytes();
    x = x.wrapping_add(ui.text.width_n(b, m));
    let uw = ui.text.width_n(b.get(m..).unwrap_or(&[]), 1);
    let y = baseline(ui, TS_BODY, sp.y, sp.h);
    let y = y.wrapping_add(ui.dp(2));
    let t = ui.hair();
    rect(ui, x, y, uw, t, c);
}

// ------------------------------------------------------------------ drop-down, split button

// button_face(r, state, flags): raised (pressed / open: dark) grey face, or the flat tool
// bar look with ST_SUBTLE
fn button_face(ui: &mut Ui, r: [i32; 4], state: u32, flags: u32) {
    if flags & ST_SUBTLE == 0 {
        let c = pal(ui, CP_FACE);
        fill(ui, r, c);
        frame(ui, r, if state == 2 { FR_PRESSED } else { FR_RAISED });
    } else if state == 1 {
        frame(ui, r, FR_THIN_RAISED);
    } else if state == 2 {
        frame(ui, r, FR_THIN_SUNKEN);
    }
}

// state_of(ev, disabled) -> 0 rest, 1 hover, 2 pressed, 3 disabled
fn state_of(ev: u32, disabled: u32) -> u32 {
    if disabled != 0 {
        3
    } else if ev & EV_HOVER == 0 {
        0
    } else if ev & EV_HELD == 0 {
        1
    } else {
        2
    }
}

/// sty_dropdown_size -> (w, h): 8 [+ icon 16 + 4] + label + 4 + 12 arrow + 8, 23 DIP high;
/// keeps s[0] label width
fn dropdown_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) if sp.flags & ST_ICON_ONLY == 0 => ui.text.width(l),
        _ => 0,
    };
    sp.s[0] = tw;
    let mut w = tw + ui.dp(32);
    if sp.icon != 0 {
        w += ui.dp(20);
    }
    (w, ui.dp(23))
}

// the icon (16 DIP, vertically centred at x) of a drop-down / split button -> x after it
fn lead_icon(ui: &mut Ui, sp: &Sp, x: i32, c: u32) -> i32 {
    if sp.icon == 0 {
        return x;
    }
    ui.draw_color(c);
    let s = ui.dp(16);
    let y = (sp.h.wrapping_sub(s) >> 1).wrapping_add(sp.y);
    ui.draw_icon(sp.icon, x, y, s << 8);
    x.wrapping_add(ui.dp(20))
}

/// sty_dropdown: button face (sunken while open), icon + label, small down arrow (value 1 =
/// open, value2 disabled)
fn dropdown(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let mut st = state_of(sp.ev, sp.value2 as u32);
    if sp.value != 0 && st != 3 {
        st = 2;
    }
    button_face(ui, r, st, sp.flags);
    let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    let x = ui.dp(8).wrapping_add(sp.x);
    let x = lead_icon(ui, sp, x, c);
    if sp.flags & ST_ICON_ONLY == 0 {
        ui.ts_font(TS_BODY);
        text_at(ui, x, r, sp.label, c, TS_BODY);
    }
    let a = ui.dp(20);
    let ar = [sp.x.wrapping_add(sp.w).wrapping_sub(a), sp.y, a, sp.h];
    icon_mid(ui, ar, IC_CHEV_DOWN_SMALL, 8, c);
}

/// sty_split_size -> (w, h): main part 8 [+ 20 icon] + label + 8, arrow part 16 DIP, 23
/// high; s[0] / s[1] = main / arrow part width
fn split_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let mut m = ui.text.width(sp.label.unwrap_or(""));
    m += ui.dp(16);
    if sp.icon != 0 {
        m += ui.dp(20);
    }
    sp.s[0] = m;
    let a = ui.dp(16);
    sp.s[1] = a;
    (m + a, ui.dp(23))
}

/// sty_split: two button faces side by side (main / arrow part), icon + label, arrow (ev
/// main part, s[2] arrow events, value open, value2 1 main / 2 arrow disabled)
fn split(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 as u32;
    let mut part = [r[0], r[1], sp.s[0], r[3]];
    let st = state_of(sp.ev, dis & 1);
    button_face(ui, part, st, 0);
    part[0] = sp.x.wrapping_add(sp.s[0]);
    part[2] = sp.s[1];
    let mut st = state_of(sp.s[2] as u32, dis >> 1);
    if sp.value != 0 && st != 3 {
        st = 2;
    }
    button_face(ui, part, st, 0);
    let c = ui.color(if dis & 2 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    icon_mid(ui, part, IC_CHEV_DOWN_SMALL, 8, c);
    // icon + label in the main part
    let c = ui.color(if dis & 1 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    let x = ui.dp(8).wrapping_add(sp.x);
    let x = lead_icon(ui, sp, x, c);
    ui.ts_font(TS_BODY);
    text_at(ui, x, r, sp.label, c, TS_BODY);
}

// ------------------------------------------------------------------ combo box

/// sty_combo: white sunken field with the selected text and a raised arrow button (label
/// the selected text, ev, value2 disabled)
fn combo(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = pal(ui, if sp.value2 != 0 { CP_FACE } else { CP_FIELD });
    fill(ui, r, c);
    frame(ui, r, FR_SUNKEN);
    let tc = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    if sp.label.is_some() {
        ui.ts_font(TS_BODY);
        let x = ui.dp(4).wrapping_add(sp.x);
        text_at(ui, x, r, sp.label, tc, TS_BODY);
    }
    // button inside the 2-line frame
    let t = ui.hair();
    let t2 = t + t;
    let bw = ui.dp(16);
    let b = [
        sp.x.wrapping_add(sp.w).wrapping_sub(t2).wrapping_sub(bw),
        sp.y.wrapping_add(t2),
        bw,
        sp.h.wrapping_sub(t2).wrapping_sub(t2),
    ];
    let st = state_of(sp.ev, sp.value2 as u32);
    button_face(ui, b, st, 0);
    icon_mid(ui, b, IC_CHEV_DOWN_SMALL, 8, tc);
}

// ------------------------------------------------------------------ tooltips

/// sty_tooltip_size -> (w, h): body text (at most 300 DIP) + 8, 19 DIP high; s[0] text
/// width, s[1] / s[2] = 20 / 4 DIP gap below / above the pointer
fn tooltip_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let mut tw = ui.text.width(sp.label.unwrap_or(""));
    let m = ui.dp(300);
    if tw > m {
        tw = m;
    }
    sp.s[0] = tw;
    let w = tw + ui.dp(8);
    sp.s[1] = ui.dp(20);
    sp.s[2] = ui.dp(4);
    (w, ui.dp(19))
}

/// sty_tooltip: pale yellow box with a black line, black text (s[0] text width)
fn tooltip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = ui.color(T_INFO_BG);
    fill(ui, r, c);
    frame(ui, r, FR_BLACK);
    ui.ts_font(TS_BODY);
    let c = ui.color(T_TEXT);
    let y = baseline(ui, TS_BODY, sp.y, sp.h);
    let x = sp.x.wrapping_add(ui.dp(4));
    text(ui, x, y, sp.label.unwrap_or(""), c, sp.s[0].wrapping_add(1));
}
