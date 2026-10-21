// popup -- the winui flyouts, menus, menu bars, drop-downs, split buttons, combo boxes, tooltips  [styles/winui/popup.s]
//
//   flyout_pad / flyout_box / flyout_frame     4 DIP padding, BOX_V | PAD_4; soft shadow,
//                                              flyout fill, 1 px stroke, 8 DIP corners
//   menu_item_h / menu_item_size / menu_item   32 DIP rows: 11 [+ 28 icon / check column (x 2
//                                              with MIF_TWOCOL)] + label [+ 24 + accelerator]
//                                              [+ 36 chevron] + 11, at least 120 DIP
//   menu_sep_size / menu_sep                   9 DIP, a hairline 4 DIP past both sides
//   menubar_item_size / menubar_item           label + 24, 32 high; mnemonic underline
//   dropdown_size / dropdown                   control surface (ST_SUBTLE: flat), icon +
//                                              label, chevron
//   split_size / split                         main part + 36 DIP chevron part, divider
//   combo_size / combo                         12 + widest text + 38, at least 64 DIP
//   tooltip_size / tooltip                     caption text (at most 300 DIP) + 18, 30 high
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN, IC_CHEV_RIGHT};
use ui::{Sp, StylePopup, Ui};

use crate::WinUi;

const MENU_ITEM_H: i32 = 32;
const MENU_MIN_W: i32 = 120;

impl StylePopup for WinUi {
    fn flyout_pad(&self, ui: &mut Ui) -> i32 {
        ui.dp(4)
    }
    fn flyout_box(&self, _ui: &mut Ui) -> u32 {
        BOX_V | PAD_4
    }
    fn flyout_frame(&self, ui: &mut Ui, sp: &Sp) {
        flyout_frame(ui, sp)
    }
    fn menu_item_h(&self, ui: &mut Ui) -> i32 {
        ui.dp(MENU_ITEM_H)
    }
    fn menu_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        menu_item_size(ui, sp)
    }
    fn menu_item(&self, ui: &mut Ui, sp: &Sp) {
        menu_item(ui, sp)
    }
    fn menu_sep_size(&self, ui: &mut Ui) -> i32 {
        ui.dp(9)
    }
    fn menu_sep(&self, ui: &mut Ui, sp: &Sp) {
        menu_sep(ui, sp)
    }
    fn menubar_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let w = ui.text.width(sp.label.unwrap_or("")) + ui.dp(24);
        (w, ui.dp(32))
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
        // 12 + widest text + 38 chevron column, at least 64 DIP, 32 high
        let w = sp.value + ui.dp(50);
        let min = ui.dp(64);
        (if w < min { min } else { w }, ui.dp(32))
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

// ---------------------------------------------------------------- helpers

// s_tbase(y, h, ts) -> baseline of a text style centred in a row (selects the font)
fn tbase(ui: &mut Ui, y: i32, h: i32, ts: u32) -> i32 {
    let line = ui.ts_line(ts);
    let y = y + ((h - line) >> 1);
    ui.ts_base(ts) + y
}

// s_ptext(x, baseline, s, colour, ts)
fn ptext(ui: &mut Ui, x: i32, y: i32, s: &str, c: u32, ts: u32) {
    ui.ts_font(ts);
    ui.emit_text(x, y, s, c);
}

// s_fill_rect_r(token, radius_dip): the rectangle filled with a token, rounded
fn fill_rect_r(ui: &mut Ui, tok: u32, rad: i32, r: [i32; 4]) {
    let c = ui.color(tok);
    ui.draw_color(c);
    let rad = ui.dp(rad);
    ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
}

// s_ctl_draw(state, rect) -> text colour: control fill (state 0..3), 1 px border with a
// darker bottom edge (rest / hover), 4 DIP corners
fn ctl_draw(ui: &mut Ui, state: u32, r: [i32; 4]) -> u32 {
    let fill = ui.color(state * 4 + T_CTRL);
    let stroke = ui.color(T_CTRL_STROKE);
    let mut stroke2 = stroke;
    if state < 2 {
        stroke2 = ui.color(T_CTRL_STROKE2);
    }
    let rad = ui.dp(4);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    text_tok(ui, state)
}

// the text colour of a control state: rest / hover primary, pressed secondary, disabled
fn text_tok(ui: &Ui, state: u32) -> u32 {
    ui.color(match state {
        2 => T_TEXT2,
        3 => T_TEXT_DISABLED,
        _ => T_TEXT,
    })
}

// s_subtle_draw(state, open, rect) -> text colour: the flat (tool bar) look; hover shade
// while open
fn subtle_draw(ui: &mut Ui, state: u32, open: bool, r: [i32; 4]) -> u32 {
    let mut tok = state * 4 + T_SUBTLE;
    if open && state != 3 {
        tok = T_SUBTLE_HOVER;
    }
    let c = ui.color(tok);
    if c & 0xFF00_0000 != 0 {
        ui.draw_color(c);
        let rad = ui.dp(4);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
    }
    text_tok(ui, state)
}

// s_chev_right(cp, colour, right_dip, rect): a 12 DIP glyph right_dip from the right edge,
// vertically centred
fn chev_right(ui: &mut Ui, cp: u32, c: u32, right: i32, r: [i32; 4]) {
    ui.draw_color(c);
    let s = ui.dp(12);
    let x = r[0] + r[2] - ui.dp(right) - s;
    let y = ((r[3] - s) >> 1) + r[1];
    ui.draw_icon(cp, x, y, s << 8);
}

// s_icon_label(x, icon, label, colour, rect): [16 DIP icon + 8 gap] label (body)
fn icon_label(ui: &mut Ui, x: i32, icon: u32, label: &str, c: u32, r: [i32; 4]) {
    let mut x = x;
    if icon != 0 {
        ui.draw_color(c);
        let s = ui.dp(16);
        let y = ((r[3] - s) >> 1) + r[1];
        ui.draw_icon(icon, x, y, s << 8);
        x += ui.dp(24);
    }
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, c, TS_BODY);
}

// s_pstate(ev, disabled) -> 0 rest, 1 hover, 2 pressed, 3 disabled
fn pstate(ev: u32, disabled: bool) -> u32 {
    if disabled {
        3
    } else if ev & EV_HOVER == 0 {
        0
    } else if ev & EV_HELD == 0 {
        1
    } else {
        2
    }
}

// ---------------------------------------------------------------- flyouts

/// sty_flyout_frame: soft shadow, flyout fill, 1 px stroke, 8 DIP corners
fn flyout_frame(ui: &mut Ui, sp: &Sp) {
    let rad = ui.dp(8);
    let sh = if ui.theme_dark() { 0x3C00_0000 } else { 0x1A00_0000 };
    ui.draw_color(sh);
    let depth = ui.dp(7);
    ui.cmd_shadow(sp.x, sp.y, sp.w, sp.h, rad, depth);
    let fill = ui.color(T_FLYOUT);
    let stroke = ui.color(T_FLYOUT_STROKE);
    let t = ui.hair();
    ui.cmd_rrect(sp.x, sp.y, sp.w, sp.h, rad, t, fill, stroke, stroke);
}

// ---------------------------------------------------------------- menu rows

/// sty_menu_item_size -> (w, h); s[0] label width, s[1] accelerator width
fn menu_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mf = sp.flags;
    ui.ts_font(TS_BODY);
    let lw = ui.text.width(sp.label.unwrap_or(""));
    let mut aw = 0;
    if let Some(a) = sp.label2.filter(|a| !a.is_empty()) {
        ui.ts_font(TS_CAPTION);
        aw = ui.text.width(a);
    }
    // natural width: 11 [+ 16 + 12] + label [+ 24 + accel] [+ 24 + 12] + 11
    let mut w = ui.dp(22);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(28);
        w += c;
        if mf & MIF_TWOCOL != 0 {
            w += c;
        }
    }
    w += lw;
    if aw != 0 {
        w += ui.dp(24) + aw;
    }
    if mf & MIF_SUB != 0 {
        w += ui.dp(36);
    }
    if mf & MIF_LIST == 0 {
        let min = ui.dp(MENU_MIN_W);
        if w < min {
            w = min;
        }
    }
    let h = ui.dp(MENU_ITEM_H);
    sp.s[0] = lw;
    sp.s[1] = aw;
    (w, h)
}

/// sty_menu_item: highlight (subtle hover / pressed shades, 4 DIP corners), accent pill of a
/// selected list row, check / icon glyphs, label, right-aligned accelerator (caption,
/// secondary colour), submenu chevron
fn menu_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (icon, mf, aw, ev) = (sp.icon, sp.flags, sp.s[1], sp.ev);
    let label = sp.label.unwrap_or("");
    // background: hover / focus / open / selected, pressed
    let mut tok = T_TEXT_DISABLED;
    if mf & MIF_DISABLED == 0 {
        if ev & (EV_HOVER | EV_FOCUS) != 0 || mf & (MIF_OPEN | MIF_SELECTED) != 0 {
            let mut t = T_SUBTLE_HOVER;
            if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD || (mf & MIF_SELECTED != 0 && ev & EV_HOVER != 0) {
                t = T_SUBTLE_PRESSED;
            }
            fill_rect_r(ui, t, 4, r);
        }
        tok = T_TEXT;
    }
    let tc = ui.color(tok);
    // accent pill of the selected list row
    if mf & MIF_SELECTED != 0 {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        let ph = ui.dp(16);
        let pw = ui.dp(3);
        ui.draw_rrect(r[0], ((r[3] - ph) >> 1) + r[1], pw, ph, pw >> 1);
    }
    if mf & MIF_LIST == 0 {
        let mut cp = icon;
        let mut xo = 0; // x offset of the icon (two columns)
        if mf & MIF_TWOCOL != 0 {
            xo = ui.dp(28);
            if mf & MIF_CHECKED != 0 {
                glyph(ui, r, tc, 0, IC_CHECK_MARK);
            }
        } else if mf & MIF_CHECKED != 0 {
            cp = IC_CHECK_MARK;
        }
        if cp != 0 {
            glyph(ui, r, tc, xo, cp);
        }
    }
    // label
    let mut x = ui.dp(11);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(28);
        x += c;
        if mf & MIF_TWOCOL != 0 {
            x += c;
        }
    }
    x += r[0];
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, tc, TS_BODY);
    // accelerator, right-aligned in the secondary text colour
    if aw != 0 {
        let c = ui.color(if mf & MIF_DISABLED != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
        let x = r[0] + r[2] - ui.dp(11) - aw;
        let y = tbase(ui, r[1], r[3], TS_CAPTION);
        ptext(ui, x, y, sp.label2.unwrap_or(""), c, TS_CAPTION);
    }
    if mf & MIF_SUB != 0 {
        let c = ui.color(T_TEXT2);
        chev_right(ui, IC_CHEV_RIGHT, c, 11, r);
    }
}

// .smi_glyph(x offset, cp): a 16 DIP glyph in the text colour, 11 DIP + offset from the
// row's left edge, vertically centred
fn glyph(ui: &mut Ui, r: [i32; 4], c: u32, xo: i32, cp: u32) {
    ui.draw_color(c);
    let s = ui.dp(16);
    let x = r[0] + ui.dp(11) + xo;
    let y = ((r[3] - s) >> 1) + r[1];
    ui.draw_icon(cp, x, y, s << 8);
}

/// sty_menu_sep: hairline in the divider colour across the row, 4 DIP past both sides
fn menu_sep(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let d = ui.dp(4);
    ui.draw_rect(r[0] - d, ((r[3] - h) >> 1) + r[1], r[2] + d * 2, h);
}

// ---------------------------------------------------------------- menu bar

/// sty_menubar_item: subtle highlight (open / hover / pressed / current item), label 12 DIP
/// from the left, mnemonic underline (flags 1 open, 2 current item, value byte index of the
/// mnemonic or -1)
fn menubar_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, f, mnem) = (sp.ev, sp.flags, sp.value);
    let label = sp.label.unwrap_or("");
    let mut tok = 0;
    if f & 1 != 0 {
        tok = T_SUBTLE_HOVER;
    } else if ev & EV_HOVER != 0 || f & 2 != 0 {
        tok = T_SUBTLE_HOVER;
        if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD {
            tok = T_SUBTLE_PRESSED;
        }
    }
    if tok != 0 {
        fill_rect_r(ui, tok, 4, r);
    }
    let c = ui.color(T_TEXT);
    let mut x = ui.dp(12) + r[0];
    let y = tbase(ui, r[1], r[3], TS_BODY); // baseline
    ptext(ui, x, y, label, c, TS_BODY);
    // mnemonic underline in menu bar mode
    if mnem >= 0 {
        let m = mnem as usize;
        ui.ts_font(TS_BODY);
        let b = label.as_bytes();
        x += ui.text.width_n(b, m);
        let uw = ui.text.width_n(b.get(m..).unwrap_or(&[]), 1);
        ui.draw_color(c);
        let y = y + ui.dp(2);
        let h = ui.hair();
        ui.draw_rect(x, y, uw, h);
    }
}

// ---------------------------------------------------------------- drop-down, split button, combo box

/// sty_dropdown_size -> (w, h): 11 [+ icon 16 + 8] + label + 8 + 12 + 11, 32 high
/// (ST_ICON_ONLY: icon + chevron)
fn dropdown_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = if sp.flags & ST_ICON_ONLY != 0 { 0 } else { ui.text.width(sp.label.unwrap_or("")) };
    let mut w = ui.dp(42) + tw;
    if sp.icon != 0 {
        w += ui.dp(24);
        if tw == 0 {
            w -= ui.dp(8); // icon only: no gap after the icon
        }
    }
    (w, ui.dp(32))
}

/// sty_dropdown: control surface (ST_SUBTLE: flat tool bar look, hover shade while open),
/// icon + label, chevron (value 1 = open, value2 1 = disabled)
fn dropdown(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let st = pstate(sp.ev, sp.value2 != 0);
    let tc = if sp.flags & ST_SUBTLE != 0 { subtle_draw(ui, st, sp.value != 0, r) } else { ctl_draw(ui, st, r) };
    let x = ui.dp(11) + r[0];
    let label = if sp.flags & ST_ICON_ONLY != 0 { "" } else { sp.label.unwrap_or("") };
    icon_label(ui, x, sp.icon, label, tc, r);
    let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 11, r);
}

/// sty_split_size -> (w, h): main part 11 [+ 24 icon] + label + 11, chevron part 36 DIP,
/// 32 high; s[0] / s[1] = main / chevron part width
fn split_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    let mut main = ui.dp(22) + tw;
    if sp.icon != 0 {
        main += ui.dp(24);
    }
    let chev = ui.dp(36);
    let w = chev + main;
    let h = ui.dp(32);
    sp.s[0] = main;
    sp.s[1] = chev;
    (w, h)
}

/// sty_split: two control surfaces (main / chevron part, each clipped), divider, icon +
/// label, chevron (ev main part, s[2] chevron events, value open, value2 1 main / 2 chevron
/// disabled)
fn split(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (mw, cw, dis) = (sp.s[0], sp.s[1], sp.value2);
    // states
    let sm = pstate(sp.ev, dis & 1 != 0);
    let mut sc = pstate(sp.s[2] as u32, dis >> 1 != 0);
    if sp.value != 0 && sc != 3 {
        sc = 2;
    }
    // main part: the whole control clipped to the part
    ui.draw_clip(r[0], r[1], mw, r[3]);
    let tc = ctl_draw(ui, sm, r);
    ui.draw_unclip();
    // chevron part
    ui.draw_clip(r[0] + mw, r[1], cw, r[3]);
    ctl_draw(ui, sc, r);
    ui.draw_unclip();
    // divider
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let h = ui.hair();
    ui.draw_rect(r[0] + mw, r[1] + h, h, r[3] - h - h);
    // icon and label in the main part
    let x = ui.dp(11) + r[0];
    icon_label(ui, x, sp.icon, sp.label.unwrap_or(""), tc, [r[0], r[1], mw, r[3]]);
    // chevron, centred in its part
    let c = ui.color(if dis & 2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 12, [r[0] + mw, r[1], cw, r[3]]);
}

/// sty_combo: control surface, the selected text (label or None) 12 DIP from the left,
/// chevron (value2 1 = disabled)
fn combo(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let st = pstate(sp.ev, sp.value2 != 0);
    let tc = ctl_draw(ui, st, r);
    if let Some(l) = sp.label {
        let x = ui.dp(12) + r[0];
        let y = tbase(ui, r[1], r[3], TS_BODY);
        ptext(ui, x, y, l, tc, TS_BODY);
    }
    let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 14, r);
}

// ---------------------------------------------------------------- tooltips

/// sty_tooltip_size -> (w, h): caption text (at most 300 DIP wide) + 18, 30 high; s[0] =
/// text width, s[1] / s[2] = gap below / above the pointer (20 / 8 DIP)
fn tooltip_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_CAPTION);
    let mut tw = ui.text.width(sp.label.unwrap_or(""));
    let max = ui.dp(300);
    if tw > max {
        tw = max;
    }
    sp.s[0] = tw;
    let w = tw + ui.dp(18);
    let h = ui.dp(30);
    sp.s[1] = ui.dp(20);
    sp.s[2] = ui.dp(8);
    (w, h)
}

/// sty_tooltip: shadow, flyout surface with a 1 px stroke and 4 DIP corners, caption text
/// 9 / 6 DIP inside (s[0] text width)
fn tooltip(ui: &mut Ui, sp: &Sp) {
    let (x, y, w, h) = (sp.x, sp.y, sp.w, sp.h);
    let label = sp.label.unwrap_or("");
    let rad = ui.dp(4);
    let sh = if ui.theme_dark() { 0x3000_0000 } else { 0x1400_0000 };
    ui.draw_color(sh);
    let depth = ui.dp(3);
    ui.cmd_shadow(x, y, w, h, rad, depth);
    let fill = ui.color(T_FLYOUT);
    let stroke = ui.color(T_FLYOUT_STROKE);
    let t = ui.hair();
    ui.cmd_rrect(x, y, w, h, rad, t, fill, stroke, stroke);
    // text
    let tx = x + ui.dp(9);
    let ty = ui.dp(6) + y + ui.ts_base(TS_CAPTION);
    ui.ts_font(TS_CAPTION);
    let (font, size) = ui.text.get_font();
    let c = ui.color(T_TEXT);
    let b = label.as_bytes();
    let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    ui.cmd_text(tx, ty, &b[..n], font, size, c, sp.s[0] + 1);
}
