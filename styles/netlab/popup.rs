// popup -- NetLab flyouts, menus, menu bars, drop-downs, split buttons, combo boxes,
// tooltips
//
//   flyout_frame            4 DIP corners, the one soft shadow, flyout fill, 1 DIP frame
//   menu rows               26 DIP: 10 [+ 24 icon / check column (x 2: MIF_TWOCOL)] + label
//                           [+ 24 + accelerator] [+ 20 chevron] + 10, at least 160 DIP; the
//                           selection (16 % accent tint, 1 DIP accent frame) on hover and on
//                           the selected list row
//   menubar items           label + 16, 26 high; a framed shade on hover, the selection while
//                           open
//   dropdown / split        the push-button surface (framed, the frame toward the accent on
//                           hover) with a chevron; ST_SUBTLE: flat, framed on hover
//   combo                   a framed field like the text box with a chevron
//   tooltip                 3 DIP corners, flyout surface with a frame, no shadow, caption
//                           text (at most 320 DIP)

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN, IC_CHEV_RIGHT};
use ui::{Sp, StylePopup, Ui};

use crate::basic::ctl_strokes;
use crate::common::{framed, select_colors, shadow, R_CTRL, R_FLYOUT};
use crate::Netlab;

const MENU_ITEM_H: i32 = 26;
const MENU_MIN_W: i32 = 160;

impl StylePopup for Netlab {
    fn flyout_pad(&self, ui: &mut Ui) -> i32 {
        ui.dp(3)
    }
    fn flyout_box(&self, _ui: &mut Ui) -> u32 {
        BOX_V | (3 << PAD_SHIFT)
    }
    fn flyout_frame(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_FLYOUT);
        let depth = ui.dp(10);
        shadow(ui, r, rad, depth, 0x22, 0x70);
        let fill = ui.color(T_FLYOUT);
        let stroke = ui.color(T_FLYOUT_STROKE);
        let t = ui.hair();
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke);
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
        ui.dp(7)
    }
    fn menu_sep(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let h = ui.hair();
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        let d = ui.dp(3);
        ui.draw_rect(r[0] - d, r[1] + ((r[3] - h) >> 1), r[2] + d * 2, h);
    }
    fn menubar_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let w = ui.text.width(sp.label.unwrap_or("")) + ui.dp(16);
        (w, ui.dp(26))
    }
    fn menubar_item(&self, ui: &mut Ui, sp: &Sp) {
        menubar_item(ui, sp)
    }
    fn dropdown_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let tw = if sp.flags & ST_ICON_ONLY != 0 { 0 } else { ui.text.width(sp.label.unwrap_or("")) };
        let mut w = ui.dp(10 + 8 + 12 + 8) + tw;
        if sp.icon != 0 {
            w += ui.dp(22);
            if tw == 0 {
                w -= ui.dp(6);
            }
        }
        (w, ui.dp(if sp.flags & ST_COMPACT != 0 { 24 } else { 28 }))
    }
    fn dropdown(&self, ui: &mut Ui, sp: &Sp) {
        dropdown(ui, sp)
    }
    fn split_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        let mut main = ui.dp(20) + tw;
        if sp.icon != 0 {
            main += ui.dp(22);
        }
        let chev = ui.dp(24);
        sp.s[0] = main;
        sp.s[1] = chev;
        (main + chev, ui.dp(28))
    }
    fn split(&self, ui: &mut Ui, sp: &Sp) {
        split(ui, sp)
    }
    fn combo_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        let w = (sp.value + ui.dp(8 + 28)).max(ui.dp(72));
        (w, ui.dp(28))
    }
    fn combo(&self, ui: &mut Ui, sp: &Sp) {
        combo(ui, sp)
    }
    fn tooltip_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_CAPTION);
        let label = sp.label.unwrap_or("");
        // multi-line tooltips: the widest line, a line per '\n'
        let mut tw = 0;
        let mut lines = 0;
        for l in label.split('\n') {
            tw = tw.max(ui.text.width(l));
            lines += 1;
        }
        let tw = tw.min(ui.dp(320));
        sp.s[0] = tw;
        sp.s[1] = ui.dp(16);
        sp.s[2] = ui.dp(4);
        (tw + ui.dp(16), ui.ts_line(TS_CAPTION) * lines.max(1) + ui.dp(8))
    }
    fn tooltip(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_CTRL);
        let fill = ui.color(T_FLYOUT);
        let stroke = ui.color(T_FLYOUT_STROKE);
        framed(ui, r[0], r[1], r[2], r[3], rad, fill, stroke);
        let line = ui.ts_line(TS_CAPTION);
        let base = ui.ts_base(TS_CAPTION);
        ui.ts_font(TS_CAPTION);
        let (font, size) = ui.text.get_font();
        let c = ui.color(T_TEXT);
        let mut y = r[1] + ui.dp(4) + base;
        for l in sp.label.unwrap_or("").split('\n') {
            let b = l.as_bytes();
            let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
            ui.cmd_text(r[0] + ui.dp(8), y, &b[..n], font, size, c, sp.s[0] + 1);
            y += line;
        }
    }
}

// ---------------------------------------------------------------- helpers

fn tbase(ui: &mut Ui, y: i32, h: i32, ts: u32) -> i32 {
    let line = ui.ts_line(ts);
    ui.ts_base(ts) + y + ((h - line) >> 1)
}

fn ptext(ui: &mut Ui, x: i32, y: i32, s: &str, c: u32, ts: u32) {
    ui.ts_font(ts);
    ui.emit_text(x, y, s, c);
}

// the push-button surface -> text colour (state 0 rest, 1 hover, 2 pressed, 3 disabled)
fn ctl_draw(ui: &mut Ui, state: u32, r: [i32; 4]) -> u32 {
    let fill = ui.color(T_CTRL + state * 4);
    let (stroke, stroke2) = ctl_strokes(ui, state);
    let rad = ui.dp(R_CTRL);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT })
}

// the tool-button surface: flat, a shade and a frame on hover / pressed / open
fn subtle_draw(ui: &mut Ui, state: u32, open: bool, r: [i32; 4]) -> u32 {
    let mut tok = T_SUBTLE + state * 4;
    if open && state != 3 {
        tok = T_SUBTLE_PRESSED;
    }
    if tok != T_SUBTLE && tok != T_SUBTLE_DISABLED {
        let c = ui.color(tok);
        let s = ui.color(if tok == T_SUBTLE_PRESSED { T_CTRL_STROKE2 } else { T_CTRL_STROKE });
        let rad = ui.dp(R_CTRL);
        framed(ui, r[0], r[1], r[2], r[3], rad, c, s);
    }
    ui.color(match state {
        2 => T_TEXT2,
        3 => T_TEXT_DISABLED,
        _ => T_TEXT,
    })
}

// a 12 DIP glyph `right` DIP from the right edge, vertically centred
fn chev_right(ui: &mut Ui, cp: u32, c: u32, right: i32, r: [i32; 4]) {
    ui.draw_color(c);
    let s = ui.dp(12);
    let x = r[0] + r[2] - ui.dp(right) - s;
    let y = r[1] + ((r[3] - s) >> 1);
    ui.draw_icon(cp, x, y, s << 8);
}

fn icon_label(ui: &mut Ui, x: i32, icon: u32, label: &str, c: u32, r: [i32; 4]) {
    let mut x = x;
    if icon != 0 {
        ui.draw_color(c);
        let s = ui.dp(16);
        let y = r[1] + ((r[3] - s) >> 1);
        ui.draw_icon(icon, x, y, s << 8);
        x += ui.dp(22);
    }
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, c, TS_BODY);
}

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

// ---------------------------------------------------------------- menu rows

fn menu_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mf = sp.flags;
    ui.ts_font(TS_BODY);
    let lw = ui.text.width(sp.label.unwrap_or(""));
    let mut aw = 0;
    if let Some(a) = sp.label2.filter(|a| !a.is_empty() && a.as_bytes()[0] != 0) {
        ui.ts_font(TS_CAPTION);
        aw = ui.text.width(a);
    }
    let mut w = ui.dp(20);
    if mf & MIF_LIST == 0 {
        let c = ui.dp(24);
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
        w += ui.dp(20);
    }
    if mf & MIF_LIST == 0 {
        w = w.max(ui.dp(MENU_MIN_W));
    }
    sp.s[0] = lw;
    sp.s[1] = aw;
    (w, ui.dp(MENU_ITEM_H))
}

fn menu_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (icon, mf, aw, ev) = (sp.icon, sp.flags, sp.s[1], sp.ev);
    let label = sp.label.unwrap_or("");
    let rad = ui.dp(2);
    let mut tok = T_TEXT_DISABLED;
    if mf & MIF_DISABLED == 0 {
        let hot = ev & (EV_HOVER | EV_FOCUS) != 0 || mf & MIF_OPEN != 0;
        if hot || mf & MIF_SELECTED != 0 {
            let base = ui.color(T_FLYOUT);
            let (fill, stroke) = select_colors(ui, base, hot && (ev & EV_HELD != 0 || mf & MIF_SELECTED != 0));
            framed(ui, r[0], r[1], r[2], r[3], rad, fill, stroke);
        }
        tok = T_TEXT;
    }
    let tc = ui.color(tok);
    if mf & MIF_LIST == 0 {
        let mut cp = icon;
        let mut xo = 0;
        if mf & MIF_TWOCOL != 0 {
            xo = ui.dp(24);
            if mf & MIF_CHECKED != 0 {
                glyph(ui, r, tc, 0, IC_CHECK_MARK, 12);
            }
        } else if mf & MIF_CHECKED != 0 {
            cp = IC_CHECK_MARK;
        }
        if cp != 0 {
            glyph(ui, r, tc, xo, cp, if cp == IC_CHECK_MARK { 12 } else { 16 });
        }
    }
    let mut x = ui.dp(10);
    if mf & MIF_LIST == 0 {
        x += ui.dp(24);
        if mf & MIF_TWOCOL != 0 {
            x += ui.dp(24);
        }
    }
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, r[0] + x, y, label, tc, TS_BODY);
    if aw != 0 {
        let c = ui.color(if mf & MIF_DISABLED != 0 { T_TEXT_DISABLED } else { T_TEXT3 });
        let y = tbase(ui, r[1], r[3], TS_CAPTION);
        ptext(ui, r[0] + r[2] - ui.dp(10) - aw, y, sp.label2.unwrap_or(""), c, TS_CAPTION);
    }
    if mf & MIF_SUB != 0 {
        chev_right(ui, IC_CHEV_RIGHT, tc, 8, r);
    }
}

// a glyph in the icon column (12 DIP from the left + offset), vertically centred
fn glyph(ui: &mut Ui, r: [i32; 4], c: u32, xo: i32, cp: u32, size: i32) {
    ui.draw_color(c);
    let s = ui.dp(size);
    let col = ui.dp(24);
    let x = r[0] + ui.dp(8) + xo + ((col - s) >> 1) - ui.dp(4);
    let y = r[1] + ((r[3] - s) >> 1);
    ui.draw_icon(cp, x, y, s << 8);
}

fn menubar_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, f, mnem) = (sp.ev, sp.flags, sp.value);
    let label = sp.label.unwrap_or("");
    let (y0, h0) = (r[1] + ui.dp(2), r[3] - ui.dp(4));
    let rad = ui.dp(2);
    if f & 1 != 0 {
        // open: the selection
        let base = ui.color(T_LAYER);
        let (fill, stroke) = select_colors(ui, base, false);
        framed(ui, r[0], y0, r[2], h0, rad, fill, stroke);
    } else if ev & EV_HOVER != 0 || f & 2 != 0 {
        let c = ui.color(T_SUBTLE_HOVER);
        let s = ui.color(T_CTRL_STROKE);
        framed(ui, r[0], y0, r[2], h0, rad, c, s);
    }
    let c = ui.color(T_TEXT);
    let mut x = r[0] + ui.dp(8);
    let y = tbase(ui, r[1], r[3], TS_BODY);
    ptext(ui, x, y, label, c, TS_BODY);
    if mnem >= 0 {
        let m = mnem as usize;
        ui.ts_font(TS_BODY);
        let b = label.as_bytes();
        x += ui.text.width_n(b, m);
        let uw = ui.text.width_n(b.get(m..).unwrap_or(&[]), 1);
        ui.draw_color(c);
        let h = ui.hair();
        ui.draw_rect(x, y + ui.dp(2), uw, h);
    }
}

// ---------------------------------------------------------------- drop-down, split, combo

fn dropdown(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 != 0;
    let st = pstate(sp.ev, dis);
    let subtle = sp.flags & ST_SUBTLE != 0;
    let tc = if subtle { subtle_draw(ui, st, sp.value != 0, r) } else { ctl_draw(ui, st, r) };
    let label = if sp.flags & ST_ICON_ONLY != 0 { "" } else { sp.label.unwrap_or("") };
    icon_label(ui, r[0] + ui.dp(10), sp.icon, label, tc, r);
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 8, r);
}

fn split(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (mw, cw, dis) = (sp.s[0], sp.s[1], sp.value2);
    let sm = pstate(sp.ev, dis & 1 != 0);
    let mut sc = pstate(sp.s[2] as u32, (dis >> 1) != 0);
    if sp.value != 0 && sc != 3 {
        sc = 2;
    }
    ui.draw_clip(r[0], r[1], mw, r[3]);
    let tc = ctl_draw(ui, sm, r);
    ui.draw_unclip();
    ui.draw_clip(r[0] + mw, r[1], cw, r[3]);
    ctl_draw(ui, sc, r);
    ui.draw_unclip();
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let h = ui.hair();
    ui.draw_rect(r[0] + mw, r[1], h, r[3]);
    icon_label(ui, r[0] + ui.dp(10), sp.icon, sp.label.unwrap_or(""), tc, [r[0], r[1], mw, r[3]]);
    let c = ui.color(if dis & 2 != 0 { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 6, [r[0] + mw, r[1], cw, r[3]]);
}

fn combo(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let dis = sp.value2 != 0;
    let fill = ui.color(if dis { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let (stroke, stroke2) = if dis {
        let s = ui.color(T_CTRL_STROKE);
        (s, s)
    } else if sp.ev & EV_HOVER != 0 {
        ctl_strokes(ui, 1)
    } else {
        (ui.color(T_CTRL_STROKE), ui.color(T_CTRL_STROKE2))
    };
    let rad = ui.dp(R_CTRL);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    let tc = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    if let Some(l) = sp.label {
        let y = tbase(ui, r[1], r[3], TS_BODY);
        ui.ts_font(TS_BODY);
        let (font, size) = ui.text.get_font();
        let b = l.as_bytes();
        let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
        ui.cmd_text(r[0] + ui.dp(8), y, &b[..n], font, size, tc, (r[2] - ui.dp(8 + 26)).max(1));
    }
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
    chev_right(ui, IC_CHEV_DOWN, c, 8, r);
}
