// popup -- the kde flyouts, menus, menu bars, drop-downs, split buttons, combo boxes, tooltips
//                                                                    [styles/kde/popup.s]
//
//   flyout_pad / flyout_box / flyout_frame     4 DIP padding, BOX_V | PAD_4; soft shadow, menu
//                                              fill, outline, 4 DIP corners
//   menu_item_h / menu_item_size / menu_item   28 DIP rows: 8 [+ 20 icon / check column (x 2
//                                              with MIF_TWOCOL)] + label [+ 24 + accelerator]
//                                              [+ 24 chevron] + 8, at least 160 DIP; Breeze
//                                              highlight: tint with a highlight outline
//   menu_sep_size / menu_sep                   11 DIP, a hairline 6 DIP inside both ends
//   menubar_item_size / menubar_item           label + 20, 28 high; mnemonic underline
//   dropdown_size / dropdown                   button surface (ST_SUBTLE: flat), icon + label,
//                                              chevron
//   split_size / split                         main part + 24 DIP chevron part, divider
//   combo_size / combo                         white field, 8 + widest text + 23, at least
//                                              64 DIP
//   tooltip_size / tooltip                     caption text (at most 300 DIP) + 16, 24 high
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN, IC_CHEV_RIGHT};
use ui::{Sp, StylePopup, Ui};

use crate::Kde;

const MENU_ITEM_H: i32 = 28;
const MENU_MIN_W: i32 = 160;

impl StylePopup for Kde {
    fn flyout_pad(&self, ui: &mut Ui) -> i32 {
        // padding between the flyout edge and its rows (4 DIP)
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
        // height of a menu separator row (11 DIP)
        ui.dp(11)
    }
    fn menu_sep(&self, ui: &mut Ui, sp: &Sp) {
        menu_sep(ui, sp)
    }
    fn menubar_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // label + 20, 28 high
        ui.ts_font(TS_BODY);
        let w = ui.text.width(sp.label.unwrap_or("")).wrapping_add(ui.dp(20));
        (w, ui.dp(28))
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
        // 8 + widest text (value px) + 23 (the chevron), at least 64 DIP, 30 high
        let mut w = sp.value.wrapping_add(ui.dp(31));
        let min = ui.dp(64);
        if w < min {
            w = min;
        }
        (w, ui.dp(30))
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
    let y = y.wrapping_add(h.wrapping_sub(line) >> 1);
    ui.ts_base(ts).wrapping_add(y)
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

// s_ctl_draw(state, rect) -> text colour: button surface (light, grey outline, darker bottom
// edge, 3 DIP corners; highlight outline while hovered / pressed)
fn ctl_draw(ui: &mut Ui, state: u32, r: [i32; 4]) -> u32 {
    let fill = ui.color(state.wrapping_mul(4).wrapping_add(T_CTRL));
    let mut stroke = ui.color(T_CTRL_STROKE);
    let mut stroke2 = stroke;
    if state != 3 {
        if state != 0 {
            stroke = ui.color(T_ACCENT);
            stroke2 = stroke;
        } else {
            stroke2 = ui.color(T_CTRL_STROKE2);
        }
    }
    let rad = ui.dp(3);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT })
}

// s_subtle_draw(state, open, rect) -> text colour: the flat (tool bar) look; hover shade
// while open
fn subtle_draw(ui: &mut Ui, state: u32, open: bool, r: [i32; 4]) -> u32 {
    let mut tok = state.wrapping_mul(4).wrapping_add(T_SUBTLE);
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
    let x = r[0].wrapping_add(r[2]).wrapping_sub(ui.dp(right)).wrapping_sub(s);
    let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
    ui.draw_icon(cp, x, y, s << 8);
}

// s_pop_square(disabled, rect): (Breeze has no accent square) a down chevron 8 DIP inside the
// right end of the rectangle in the text colour
fn pop_square(ui: &mut Ui, disabled: bool, r: [i32; 4]) {
    let c = ui.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
    chev_right(ui, IC_CHEV_DOWN, c, 8, r);
}

// s_hl_rect(rect): Breeze highlight of a menu row / menu bar item: highlight tint with a
// highlight outline (3 DIP corners)
fn hl_rect(ui: &mut Ui, r: [i32; 4]) {
    let fill = ui.color(T_SUBTLE_HOVER);
    let stroke = ui.color(T_ACCENT);
    let t = ui.hair();
    let rad = ui.dp(3);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke);
}

// s_icon_label(x, icon, label, colour, rect): [16 DIP icon + 6 gap] label (body)
fn icon_label(ui: &mut Ui, x: i32, icon: u32, label: &str, c: u32, r: [i32; 4]) {
    let mut x = x;
    if icon != 0 {
        ui.draw_color(c);
        let s = ui.dp(16);
        let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
        ui.draw_icon(icon, x, y, s << 8);
        x = x.wrapping_add(ui.dp(22));
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

/// sty_flyout_frame: soft shadow, menu fill, outline, 4 DIP corners
fn flyout_frame(ui: &mut Ui, sp: &Sp) {
    let rad = ui.dp(4); // corner radius
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
/// [+ 24 + accelerator] [+ 24 chevron] + 8, at least 160 DIP (not for list rows), 28 high;
/// s[0] label width, s[1] accelerator width
fn menu_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mf = sp.flags;
    ui.ts_font(TS_BODY);
    let lw = ui.text.width(sp.label.unwrap_or(""));
    let mut aw = 0;
    if let Some(a) = sp.label2.filter(|a| !a.is_empty() && a.as_bytes()[0] != 0) {
        ui.ts_font(TS_CAPTION);
        aw = ui.text.width(a);
    }
    // natural width: 8 [+ 20] + label [+ 24 + accel] [+ 24] + 8
    let mut w = ui.dp(16);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(20);
        w = w.wrapping_add(c);
        if mf & MIF_TWOCOL != 0 {
            w = w.wrapping_add(c);
        }
    }
    w = w.wrapping_add(lw);
    if aw != 0 {
        w = w.wrapping_add(ui.dp(24)).wrapping_add(aw);
    }
    if mf & MIF_SUB != 0 {
        w = w.wrapping_add(ui.dp(24));
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

/// sty_menu_item: Breeze highlight (tint with a highlight outline) while hovered / focused /
/// open, the selected row of a list in the pressed subtle shade; check / icon glyphs, label,
/// right-aligned accelerator (caption, secondary colour), submenu chevron.  ev, flags MIF_*,
/// icon, label, label2 accelerator, s[1] accelerator width (from the size call)
fn menu_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (icon, mf, aw, ev) = (sp.icon, sp.flags, sp.s[1], sp.ev);
    let label = sp.label.unwrap_or("");
    // background: highlight while hovered / focused / open; a selected list row grey
    let mut tok = T_TEXT_DISABLED;
    if mf & MIF_DISABLED == 0 {
        if ev & (EV_HOVER | EV_FOCUS) != 0 || mf & MIF_OPEN != 0 {
            hl_rect(ui, r); // tint + highlight outline, normal text
        } else if mf & MIF_SELECTED != 0 {
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
        x = x.wrapping_add(c);
        if mf & MIF_TWOCOL != 0 {
            x = x.wrapping_add(c);
        }
    }
    x = x.wrapping_add(r[0]);
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, tc, TS_BODY);
    // accelerator, right-aligned in the secondary text colour
    if aw != 0 {
        let c = ui.color(if mf & MIF_DISABLED != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
        let x = r[0].wrapping_add(r[2]).wrapping_sub(ui.dp(8)).wrapping_sub(aw);
        let y = tbase(ui, r[1], r[3], TS_CAPTION);
        ptext(ui, x, y, sp.label2.unwrap_or(""), c, TS_CAPTION);
    }
    if mf & MIF_SUB != 0 {
        chev_right(ui, IC_CHEV_RIGHT, tc, 6, r);
    }
}

// .smi_glyph(x offset, cp): a 12 DIP glyph in the text colour, 10 DIP + offset from the row's
// left edge (centred in the 20 DIP column... of the asm's comment: 8), vertically centred
fn glyph(ui: &mut Ui, r: [i32; 4], c: u32, xo: i32, cp: u32) {
    ui.draw_color(c);
    let s = ui.dp(12);
    let x = r[0].wrapping_add(ui.dp(10)).wrapping_add(xo);
    let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
    ui.draw_icon(cp, x, y, s << 8);
}

/// sty_menu_sep: hairline in the divider colour, 6 DIP inside both ends of the row
fn menu_sep(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let d = ui.dp(6);
    ui.draw_rect(r[0].wrapping_add(d), (r[3].wrapping_sub(h) >> 1).wrapping_add(r[1]), r[2].wrapping_sub(d).wrapping_sub(d), h);
}

// ---------------------------------------------------------------- menu bar

/// sty_menubar_item: Breeze highlight (open / hover / pressed / current item), label 9 DIP
/// from the left, mnemonic underline (flags 1 open, 2 current item, value byte index of the
/// mnemonic or -1)
fn menubar_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, f, mnem) = (sp.ev, sp.flags, sp.value);
    let label = sp.label.unwrap_or("");
    // (the asm picks a shade token, but s_hl_rect draws the one highlight look for all)
    if f & 1 != 0 || ev & EV_HOVER != 0 || f & 2 != 0 {
        hl_rect(ui, r);
    }
    let c = ui.color(T_TEXT);
    let mut x = ui.dp(9).wrapping_add(r[0]);
    let y = tbase(ui, r[1], r[3], TS_BODY); // baseline
    ptext(ui, x, y, label, c, TS_BODY);
    // mnemonic underline in menu bar mode
    if mnem >= 0 {
        let m = mnem as usize;
        ui.ts_font(TS_BODY);
        let b = label.as_bytes();
        x = x.wrapping_add(ui.text.width_n(b, m));
        let uw = ui.text.width_n(b.get(m..).unwrap_or(&[]), 1);
        ui.draw_color(c);
        let y = y.wrapping_add(ui.dp(2));
        let h = ui.hair();
        ui.draw_rect(x, y, uw, h);
    }
}

// ---------------------------------------------------------------- drop-down, split button, combo box

/// sty_dropdown_size -> (w, h): 8 [+ icon 16 + 6] + label + 8 + 16 + 3, 30 high
/// (ST_ICON_ONLY: icon + chevron)
fn dropdown_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = if sp.flags & ST_ICON_ONLY != 0 { 0 } else { ui.text.width(sp.label.unwrap_or("")) };
    let mut w = ui.dp(35).wrapping_add(tw);
    if sp.icon != 0 {
        w = w.wrapping_add(ui.dp(22));
        if tw == 0 {
            w = w.wrapping_sub(ui.dp(6)); // icon only: no gap after the icon
        }
    }
    (w, ui.dp(30))
}

/// sty_dropdown: pop-up button: push-button surface with a down chevron (ST_SUBTLE: flat
/// tool bar pull-down with a grey chevron), icon + label (value 1 = open, value2 1 = disabled)
fn dropdown(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 != 0;
    let st = pstate(sp.ev, dis);
    let subtle = sp.flags & ST_SUBTLE != 0;
    let tc = if subtle { subtle_draw(ui, st, sp.value != 0, r) } else { ctl_draw(ui, st, r) };
    let x = ui.dp(8).wrapping_add(r[0]);
    let label = if sp.flags & ST_ICON_ONLY != 0 { "" } else { sp.label.unwrap_or("") };
    icon_label(ui, x, sp.icon, label, tc, r);
    if !subtle {
        pop_square(ui, dis, r);
    } else {
        let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
        chev_right(ui, IC_CHEV_DOWN, c, 6, r);
    }
}

/// sty_split_size -> (w, h): main part 10 [+ 22 icon] + label + 10, chevron part 24 DIP, 30
/// high; s[0] / s[1] = main / chevron part width
fn split_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    let mut main = ui.dp(20).wrapping_add(tw);
    if sp.icon != 0 {
        main = main.wrapping_add(ui.dp(22));
    }
    let chev = ui.dp(24);
    let w = chev.wrapping_add(main);
    let h = ui.dp(30);
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
    let mut sc = pstate(sp.s[2] as u32, ((dis as u32) >> 1) != 0);
    if sp.value != 0 && sc != 3 {
        sc = 2;
    }
    // main part: the whole control clipped to the part
    ui.draw_clip(r[0], r[1], mw, r[3]);
    let tc = ctl_draw(ui, sm, r);
    ui.draw_unclip();
    // chevron part
    ui.draw_clip(r[0].wrapping_add(mw), r[1], cw, r[3]);
    ctl_draw(ui, sc, r);
    ui.draw_unclip();
    // divider
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let h = ui.hair();
    ui.draw_rect(r[0].wrapping_add(mw), r[1].wrapping_add(h), h, r[3].wrapping_sub(h).wrapping_sub(h));
    // icon and label in the main part
    let x = ui.dp(10).wrapping_add(r[0]);
    icon_label(ui, x, sp.icon, sp.label.unwrap_or(""), tc, [r[0], r[1], mw, r[3]]);
    // chevron, centred in its part
    let c = ui.color(if dis & 2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 6, [r[0].wrapping_add(mw), r[1], cw, r[3]]);
}

/// sty_combo: white field with a hairline (3 DIP corners), the selected text (label or None)
/// 8 DIP from the left, down chevron (value2 1 = disabled)
fn combo(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 != 0;
    let fill = ui.color(if dis { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let stroke = ui.color(T_CTRL_STROKE2);
    let stroke2 = ui.color(T_CTRL_STROKE);
    let rad = ui.dp(3);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    let tc = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    if let Some(l) = sp.label {
        let x = ui.dp(8).wrapping_add(r[0]);
        let y = tbase(ui, r[1], r[3], TS_BODY);
        ptext(ui, x, y, l, tc, TS_BODY);
    }
    pop_square(ui, dis, r);
}

// ---------------------------------------------------------------- tooltips

/// sty_tooltip_size -> (w, h): caption text (at most 300 DIP wide) + 16, 24 high; s[0] =
/// text width, s[1] / s[2] = gap below / above the pointer (20 / 8 DIP)
fn tooltip_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_CAPTION);
    let mut tw = ui.text.width(sp.label.unwrap_or(""));
    let max = ui.dp(300);
    if tw > max {
        tw = max;
    }
    sp.s[0] = tw;
    let w = tw.wrapping_add(ui.dp(16));
    let h = ui.dp(24);
    sp.s[1] = ui.dp(20);
    sp.s[2] = ui.dp(8);
    (w, h)
}

/// sty_tooltip: shadow, flyout surface with a hairline and 4 DIP corners, caption text 8 / 5
/// DIP inside (s[0] text width)
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
    let tx = x.wrapping_add(ui.dp(8));
    let ty = ui.dp(5).wrapping_add(y).wrapping_add(ui.ts_base(TS_CAPTION));
    ui.ts_font(TS_CAPTION);
    let (font, size) = ui.text.get_font();
    let c = ui.color(T_TEXT);
    let b = label.as_bytes();
    let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    ui.cmd_text(tx, ty, &b[..n], font, size, c, sp.s[0].wrapping_add(1));
}
