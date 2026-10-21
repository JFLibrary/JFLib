// popup -- the macos flyouts, menus, menu bars, pop-up buttons, split buttons, combo boxes,
// tooltips                                                          [styles/macos/popup.s]
//
//   flyout_pad / flyout_box / flyout_frame     4 DIP padding, BOX_V | PAD_4; soft shadow,
//                                              translucent menu fill, hairline, 7 DIP corners
//   menu_item_h / menu_item_size / menu_item   22 DIP rows: 8 [+ 20 icon / check column (x 2
//                                              with MIF_TWOCOL)] + label [+ 24 + accelerator]
//                                              [+ 24 chevron] + 8, at least 140 DIP; accent
//                                              highlight with white text
//   menu_sep_size / menu_sep                   11 DIP, a hairline 6 DIP inside both ends
//   menubar_item_size / menubar_item           label + 18, 24 high; mnemonic underline
//   dropdown_size / dropdown                   pop-up button: push-button surface with the
//                                              accent square and up / down chevrons
//                                              (ST_SUBTLE: flat tool bar pull-down)
//   split_size / split                         main part + 20 DIP chevron part, divider
//   combo_size / combo                         white field, 6 + widest text + 25 (accent
//                                              square), at least 64 DIP
//   tooltip_size / tooltip                     caption text (at most 300 DIP) + 12, 20 high
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN, IC_CHEV_RIGHT, IC_CHEV_UP_DOWN};
use ui::{Sp, StylePopup, Ui};

use crate::MacOs;

const MENU_ITEM_H: i32 = 22;
const MENU_MIN_W: i32 = 140;

impl StylePopup for MacOs {
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
        ui.dp(11)
    }
    fn menu_sep(&self, ui: &mut Ui, sp: &Sp) {
        menu_sep(ui, sp)
    }
    fn menubar_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // label + 18, 24 high
        ui.ts_font(TS_BODY);
        let w = ui.text.width(sp.label.unwrap_or("")) + ui.dp(18);
        (w, ui.dp(24))
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
        // 6 + widest text + 6 + 16 + 3 (the accent square), at least 64 DIP, 22 high
        let w = sp.value + ui.dp(31);
        let min = ui.dp(64);
        (if w < min { min } else { w }, ui.dp(22))
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

// s_ctl_draw(state, rect) -> text colour: push-button surface (white, hairline, darker
// bottom edge, 5 DIP corners)
fn ctl_draw(ui: &mut Ui, state: u32, r: [i32; 4]) -> u32 {
    let fill = ui.color(state * 4 + T_CTRL);
    let stroke = ui.color(T_CTRL_STROKE);
    let stroke2 = if state == 3 { stroke } else { ui.color(T_CTRL_STROKE2) };
    let rad = ui.dp(5);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT })
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
    ui.color(match state {
        2 => T_TEXT2,
        3 => T_TEXT_DISABLED,
        _ => T_TEXT,
    })
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

// s_pop_square(cp, disabled, rect): 16 DIP accent square (4 DIP corners) 3 DIP inside the
// right end of the rectangle with the white glyph cp (10 DIP)
fn pop_square(ui: &mut Ui, cp: u32, disabled: bool, r: [i32; 4]) {
    let c = ui.color(if disabled { T_ACCENT_DISABLED } else { T_ACCENT });
    ui.draw_color(c);
    let sq = ui.dp(16); // square
    let d = ui.dp(3);
    let x = r[0] + r[2] - d - sq;
    let y = ((r[3] - sq) >> 1) + r[1];
    let rad = ui.dp(4);
    ui.draw_rrect(x, y, sq, sq, rad);
    let c = ui.color(T_ON_ACCENT);
    ui.draw_color(c);
    let g = ui.dp(10);
    let o = (sq - g) >> 1;
    ui.draw_icon(cp, o + x, y + o, g << 8);
}

// s_icon_label(x, icon, label, colour, rect): [16 DIP icon + 6 gap] label (body)
fn icon_label(ui: &mut Ui, x: i32, icon: u32, label: &str, c: u32, r: [i32; 4]) {
    let mut x = x;
    if icon != 0 {
        ui.draw_color(c);
        let s = ui.dp(16);
        let y = ((r[3] - s) >> 1) + r[1];
        ui.draw_icon(icon, x, y, s << 8);
        x += ui.dp(22);
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

/// sty_flyout_frame: soft shadow, translucent menu fill, hairline, 7 DIP corners
fn flyout_frame(ui: &mut Ui, sp: &Sp) {
    let rad = ui.dp(7); // corner radius
    let sh = if ui.theme_dark() { 0x4000_0000 } else { 0x1400_0000 };
    ui.draw_color(sh);
    let depth = ui.dp(10); // shadow depth
    ui.cmd_shadow(sp.x, sp.y, sp.w, sp.h, rad, depth);
    let fill = ui.color(T_FLYOUT);
    let stroke = ui.color(T_FLYOUT_STROKE);
    let t = ui.hair();
    ui.cmd_rrect(sp.x, sp.y, sp.w, sp.h, rad, t, fill, stroke, stroke);
}

// ---------------------------------------------------------------- menu rows

/// sty_menu_item_size -> (w, h): 8 [+ 20 icon / check column (x 2 with MIF_TWOCOL)] + label
/// [+ 24 + accelerator] [+ 24 chevron] + 8, at least 140 DIP (not for list rows), 22 high;
/// s[0] label width, s[1] accelerator width
fn menu_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mf = sp.flags;
    ui.ts_font(TS_BODY);
    let lw = ui.text.width(sp.label.unwrap_or(""));
    let mut aw = 0;
    if let Some(a) = sp.label2.filter(|a| !a.is_empty()) {
        ui.ts_font(TS_CAPTION);
        aw = ui.text.width(a);
    }
    // natural width: 8 [+ 20] + label [+ 24 + accel] [+ 24] + 8
    let mut w = ui.dp(16);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(20);
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
        w += ui.dp(24);
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

/// sty_menu_item: highlight: accent fill (4 DIP corners) with white text, glyphs and
/// accelerator while hovered / focused / open; the selected row of a list in the shade of
/// the pressed subtle fill; check / icon glyphs, label, right-aligned accelerator (caption,
/// secondary colour), submenu chevron
fn menu_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (icon, mf, aw, ev) = (sp.icon, sp.flags, sp.s[1], sp.ev);
    let label = sp.label.unwrap_or("");
    // background: accent while hovered / focused / open; a selected list row grey
    let mut hl = false;
    let tok;
    if mf & MIF_DISABLED != 0 {
        tok = T_TEXT_DISABLED;
    } else if ev & (EV_HOVER | EV_FOCUS) != 0 || mf & MIF_OPEN != 0 {
        hl = true;
        let t = if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD { T_ACCENT_PRESSED } else { T_ACCENT };
        fill_rect_r(ui, t, 4, r);
        tok = T_ON_ACCENT;
    } else {
        if mf & MIF_SELECTED != 0 {
            fill_rect_r(ui, T_SUBTLE_PRESSED, 4, r);
        }
        tok = T_TEXT;
    }
    let tc = ui.color(tok);
    if mf & MIF_LIST == 0 {
        let mut cp = icon;
        let mut xo = 0; // x offset of the icon (two columns)
        if mf & MIF_TWOCOL != 0 {
            xo = ui.dp(20);
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
    let mut x = ui.dp(8);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(20);
        x += c;
        if mf & MIF_TWOCOL != 0 {
            x += c;
        }
    }
    x += r[0];
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, tc, TS_BODY);
    // accelerator, right-aligned in the secondary text colour (white when highlighted)
    if aw != 0 {
        let mut t = if mf & MIF_DISABLED != 0 { T_TEXT_DISABLED } else { T_TEXT2 };
        if hl {
            t = T_ON_ACCENT;
        }
        let c = ui.color(t);
        let x = r[0] + r[2] - ui.dp(8) - aw;
        let y = tbase(ui, r[1], r[3], TS_CAPTION);
        ptext(ui, x, y, sp.label2.unwrap_or(""), c, TS_CAPTION);
    }
    if mf & MIF_SUB != 0 {
        chev_right(ui, IC_CHEV_RIGHT, tc, 6, r);
    }
}

// .smi_glyph(x offset, cp): a 12 DIP glyph in the text colour, 10 DIP + offset from the
// row's left edge (centred in the 20 DIP column), vertically centred
fn glyph(ui: &mut Ui, r: [i32; 4], c: u32, xo: i32, cp: u32) {
    ui.draw_color(c);
    let s = ui.dp(12);
    let x = r[0] + ui.dp(10) + xo;
    let y = ((r[3] - s) >> 1) + r[1];
    ui.draw_icon(cp, x, y, s << 8);
}

/// sty_menu_sep: hairline in the divider colour, 6 DIP inside both ends of the row
fn menu_sep(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let d = ui.dp(6);
    ui.draw_rect(r[0] + d, ((r[3] - h) >> 1) + r[1], r[2] - d - d, h);
}

// ---------------------------------------------------------------- menu bar

/// sty_menubar_item: subtle highlight (open / hover / pressed / current item), label 9 DIP
/// from the left, mnemonic underline (flags 1 open, 2 current item, value byte index of the
/// mnemonic or -1)
fn menubar_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, f, mnem) = (sp.ev, sp.flags, sp.value);
    let label = sp.label.unwrap_or("");
    let mut tok = 0;
    if f & 1 != 0 {
        tok = T_SUBTLE_PRESSED;
    } else if ev & EV_HOVER != 0 || f & 2 != 0 {
        tok = T_SUBTLE_HOVER;
        if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD {
            tok = T_SUBTLE_PRESSED;
        }
    }
    if tok != 0 {
        fill_rect_r(ui, tok, 5, r);
    }
    let c = ui.color(T_TEXT);
    let mut x = ui.dp(9) + r[0];
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

// ---------------------------------------------------------------- pop-up button, split button, combo box

/// sty_dropdown_size -> (w, h): 8 [+ icon 16 + 6] + label + 8 + 16 + 3, 22 high (ST_SUBTLE:
/// 28, the tool bar pull-down; ST_ICON_ONLY: icon + chevron)
fn dropdown_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = if sp.flags & ST_ICON_ONLY != 0 { 0 } else { ui.text.width(sp.label.unwrap_or("")) };
    let mut w = ui.dp(35) + tw;
    if sp.icon != 0 {
        w += ui.dp(22);
        if tw == 0 {
            w -= ui.dp(6); // icon only: no gap after the icon
        }
    }
    let h = ui.dp(if sp.flags & ST_SUBTLE != 0 { 28 } else { 22 });
    (w, h)
}

/// sty_dropdown: pop-up button: push-button surface with the accent square and up / down
/// chevrons (ST_SUBTLE: flat tool bar pull-down with a grey chevron), icon + label (value 1
/// = open, value2 1 = disabled)
fn dropdown(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let st = pstate(sp.ev, sp.value2 != 0);
    let flat = sp.flags & ST_SUBTLE != 0;
    let tc = if flat { subtle_draw(ui, st, sp.value != 0, r) } else { ctl_draw(ui, st, r) };
    let x = ui.dp(8) + r[0];
    let label = if sp.flags & ST_ICON_ONLY != 0 { "" } else { sp.label.unwrap_or("") };
    icon_label(ui, x, sp.icon, label, tc, r);
    if !flat {
        pop_square(ui, IC_CHEV_UP_DOWN, sp.value2 != 0, r);
    } else {
        let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
        chev_right(ui, IC_CHEV_DOWN, c, 6, r);
    }
}

/// sty_split_size -> (w, h): main part 10 [+ 22 icon] + label + 10, chevron part 20 DIP,
/// 22 high; s[0] / s[1] = main / chevron part width
fn split_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    let mut main = ui.dp(20) + tw;
    if sp.icon != 0 {
        main += ui.dp(22);
    }
    let chev = ui.dp(20);
    let w = chev + main;
    let h = ui.dp(22);
    sp.s[0] = main;
    sp.s[1] = chev;
    (w, h)
}

/// sty_split: two control surfaces (main / chevron part, each clipped), divider, icon +
/// label, chevron (ev main part, s[2] chevron events, value open, value2 1 main / 2 chevron
/// disabled)
fn split(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (mw, cw, dis) = (sp.s[0], sp.s[1], sp.value2 as u32);
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
    let x = ui.dp(10) + r[0];
    icon_label(ui, x, sp.icon, sp.label.unwrap_or(""), tc, [r[0], r[1], mw, r[3]]);
    // chevron, centred in its part
    let c = ui.color(if dis & 2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 4, [r[0] + mw, r[1], cw, r[3]]);
}

/// sty_combo: white field with a hairline (4 DIP corners), the selected text (label or
/// None) 6 DIP from the left, accent square with a chevron (value2 1 = disabled)
fn combo(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 != 0;
    let fill = ui.color(if dis { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let stroke = ui.color(T_CTRL_STROKE2);
    let stroke2 = ui.color(T_CTRL_STROKE);
    let rad = ui.dp(4);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    let tc = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    if let Some(l) = sp.label {
        let x = ui.dp(6) + r[0];
        let y = tbase(ui, r[1], r[3], TS_BODY);
        ptext(ui, x, y, l, tc, TS_BODY);
    }
    pop_square(ui, IC_CHEV_DOWN, dis, r);
}

// ---------------------------------------------------------------- tooltips

/// sty_tooltip_size -> (w, h): caption text (at most 300 DIP wide) + 12, 20 high; s[0] =
/// text width, s[1] / s[2] = gap below / above the pointer (20 / 8 DIP)
fn tooltip_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_CAPTION);
    let mut tw = ui.text.width(sp.label.unwrap_or(""));
    let max = ui.dp(300);
    if tw > max {
        tw = max;
    }
    sp.s[0] = tw;
    let w = tw + ui.dp(12);
    let h = ui.dp(20);
    sp.s[1] = ui.dp(20);
    sp.s[2] = ui.dp(8);
    (w, h)
}

/// sty_tooltip: shadow, flyout surface with a hairline and 4 DIP corners, caption text
/// 6 / 3 DIP inside (s[0] text width)
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
    let tx = x + ui.dp(6);
    let ty = ui.dp(3) + y + ui.ts_base(TS_CAPTION);
    ui.ts_font(TS_CAPTION);
    let (font, size) = ui.text.get_font();
    let c = ui.color(T_TEXT);
    let b = label.as_bytes();
    let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    ui.cmd_text(tx, ty, &b[..n], font, size, c, sp.s[0] + 1);
}
