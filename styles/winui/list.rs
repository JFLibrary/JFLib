// list -- the winui focus rectangles, scroll bars, list rows, tabs, expanders
//                                                                  [styles/winui/list.s]
//
//   focus_rect(x, y, w, h)              focus visual drawn on a rectangle (rows, tabs, headers)
//   scrollbar_thickness / margin / min_thumb -> px, scrollbar(sp)
//   list_drop_line(sp), list_row(sp)
//   tab_metrics(sp), tab(sp), tab_pill(sp)
//   expander_size(sp), expander_header(sp), expander_body(sp)
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHEV_DOWN, IC_CHEV_UP};
use ui::{Sp, StyleList, Ui};

use crate::WinUi;

const BAR_DIP: i32 = 12; // scroll bar hover area
const THUMB_MIN_DIP: i32 = 24; // shortest scroll thumb

impl StyleList for WinUi {
    fn focus_rect(&self, ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
        focus_rect(ui, x, y, w, h)
    }
    fn scrollbar_thickness(&self, ui: &mut Ui) -> i32 {
        ui.dp(BAR_DIP)
    }
    fn scrollbar_margin(&self, ui: &mut Ui) -> i32 {
        ui.dp(2)
    }
    fn scrollbar_min_thumb(&self, ui: &mut Ui) -> i32 {
        ui.dp(THUMB_MIN_DIP)
    }
    fn scrollbar(&self, ui: &mut Ui, sp: &Sp) {
        scrollbar(ui, sp)
    }
    fn list_drop_line(&self, ui: &mut Ui, sp: &Sp) {
        list_drop_line(ui, sp)
    }
    fn list_row(&self, ui: &mut Ui, sp: &Sp) {
        list_row(ui, sp)
    }
    fn tab_metrics(&self, ui: &mut Ui, sp: &mut Sp) {
        sp.s[0] = ui.dp(12);
        sp.s[1] = ui.dp(40);
        sp.s[2] = ui.dp(16);
    }
    fn tab(&self, ui: &mut Ui, sp: &Sp) {
        tab(ui, sp)
    }
    fn tab_pill(&self, ui: &mut Ui, sp: &Sp) {
        tab_pill(ui, sp)
    }
    fn expander_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 48 DIP header: 16 + label + 48 (chevron area)
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        (tw + ui.dp(16 + 48), ui.dp(48))
    }
    fn expander_header(&self, ui: &mut Ui, sp: &Sp) {
        expander_header(ui, sp)
    }
    fn expander_body(&self, ui: &mut Ui, sp: &Sp) {
        expander_body(ui, sp)
    }
}

/// w_fill_rrect(x, y, w, h, r, argb): filled rounded rectangle (skipped when transparent)
pub(crate) fn fill_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32) {
    if argb & 0xFF00_0000 == 0 {
        return;
    }
    ui.cmd_rrect(x, y, w, h, r, 0, argb, 0, 0);
}

/// w_stroke_rrect(x, y, w, h, r, t, argb): rounded border of thickness t
pub(crate) fn stroke_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, argb: u32) {
    ui.cmd_rrect(x, y, w, h, r, t, 0, argb, argb);
}

/// w_baseline(ts, y, h) -> baseline of style ts centred in y .. y+h (selects the font)
pub(crate) fn baseline(ui: &mut Ui, ts: u32, y: i32, h: i32) -> i32 {
    let line = ui.ts_line(ts);
    let y = y + ((h - line) >> 1);
    ui.ts_base(ts) + y
}

/// w_text(x, baseline, s, argb, maxw): text in the current font (maxw 0 = unclipped)
pub(crate) fn text(ui: &mut Ui, x: i32, y: i32, s: &str, argb: u32, maxw: i32) {
    let (font, size) = ui.text.get_font();
    let b = s.as_bytes();
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    ui.cmd_text(x, y, &b[..n], font, size, argb, maxw);
}

/// sty_focus_rect(x, y, w, h): focus visual on a rectangle (list rows, tabs, expander
/// headers): 2 DIP border, 4 DIP corners
fn focus_rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let r = ui.dp(4);
    let t = ui.dp(2);
    let c = ui.color(T_FOCUS_OUT);
    stroke_rrect(ui, x, y, w, h, r, t, c);
}

/// sty_scrollbar(sp): overlay bar: 2 DIP thumb growing to 6 DIP with the expansion (value
/// 0..256), flyout-coloured track while expanded; flags 1 = vertical, s[0] track start, s[1]
/// thumb length, value2 thumb offset (px)
fn scrollbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let exp = sp.value;
    let (start, thumb, off) = (sp.s[0], sp.s[1], sp.value2);
    // thickness 2 -> 6 DIP with the expansion
    let t2 = ui.dp(2);
    let t = t2 + ((ui.dp(6) - t2).wrapping_mul(exp) >> 8);
    // track background while expanded
    if exp != 0 {
        let a = (exp as u32).min(255);
        let c = ui.color(T_FLYOUT);
        let c = (c & 0xFF_FFFF) | ((c >> 24) * a >> 8) << 24;
        let rad = r[2].min(r[3]) >> 1;
        fill_rrect(ui, r[0], r[1], r[2], r[3], rad, c);
    }
    let c = ui.color(T_SCROLL);
    let rad = t >> 1;
    if sp.flags != 0 {
        let x = ((r[2] - t) >> 1) + r[0];
        fill_rrect(ui, x, start + off, t, thumb, rad, c);
    } else {
        let y = ((r[3] - t) >> 1) + r[1];
        fill_rrect(ui, start + off, y, thumb, t, rad, c);
    }
}

/// sty_list_drop_line(sp): reorder insertion marker: 2 DIP accent line, 4 DIP inset, centred
/// on y across x .. x + w
fn list_drop_line(ui: &mut Ui, sp: &Sp) {
    let c = ui.color(T_ACCENT);
    ui.draw_color(c);
    let t = ui.dp(2); // thickness
    let inset = ui.dp(4);
    let half = ((t as u32) >> 1) as i32;
    ui.draw_rrect(sp.x + inset, sp.y - half, sp.w - inset - inset, t, half);
}

/// sty_list_row(sp): row background (subtle shades; selected rest = hover shade), 3 x 16 DIP
/// accent pill when selected, focus visual, label 16 DIP from the left.  ev, value 1 =
/// selected, value2 1 = disabled, flags 1 = dragged (pressed look), 2 = focus visual
fn list_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let sel = sp.value != 0;
    let dis = sp.value2 != 0;
    // background inset 4 / 2 DIP
    let ix = ui.dp(4);
    let iy = ui.dp(2);
    let (bx, by, bw, bh) = (r[0] + ix, r[1] + iy, r[2] - ix - ix, r[3] - iy - iy);
    // fill: rest none / hover / pressed; selected rest = hover shade, selected hover lighter
    let mut tok = 0;
    if ev & EV_HOVER != 0 {
        tok = if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER };
        if sel {
            tok ^= T_SUBTLE_HOVER ^ T_SUBTLE_PRESSED; // swap the two shades
        }
    } else if sel {
        tok = T_SUBTLE_HOVER;
    }
    if sp.flags & 1 != 0 {
        tok = T_SUBTLE_PRESSED; // the dragged row looks pressed
    }
    if tok != 0 {
        let c = ui.color(tok);
        fill_rrect(ui, bx, by, bw, bh, ix, c);
    }
    if sel {
        // selection pill 3 x 16 DIP at the left of the background
        let c = ui.color(if dis { T_ACCENT_DISABLED } else { T_ACCENT });
        let pw = ui.dp(3);
        let ph = ui.dp(16);
        let y = ((bh - ph) >> 1) + by;
        fill_rrect(ui, bx, y, pw, ph, pw >> 1, c);
        if sp.flags & 2 != 0 {
            // focus visual around the background, inside the row (rows touch each other)
            let d = ui.dp(2);
            focus_rect(ui, r[0] + d, r[1], r[2] - d - d, r[3]);
        }
    }
    if let Some(l) = sp.label {
        let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
        let mut tw = r[2] - ui.dp(28);
        if tw <= 0 {
            tw = 1;
        }
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let x = ui.dp(16) + r[0];
        text(ui, x, y, l, c, tw);
    }
}

/// sty_tab(sp): tab background (subtle hover / pressed) and label (secondary text unless
/// selected or hovered).  ev, value 1 = selected, value2 1 = disabled, label
fn tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let pad = ui.dp(12);
    // hover / pressed background
    if ev & EV_HOVER != 0 {
        let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
        let d4 = ui.dp(4);
        let d3 = ui.dp(3);
        fill_rrect(ui, r[0] + d3, r[1] + d4, r[2] - d3 - d3, r[3] - d4 - d4, d4, c);
    }
    let mut tok = if sp.value != 0 { T_TEXT } else { T_TEXT2 };
    if ev & EV_HOVER != 0 {
        tok = if ev & EV_HELD != 0 { T_TEXT2 } else { T_TEXT };
    }
    if sp.value2 != 0 {
        tok = T_TEXT_DISABLED;
    }
    let c = ui.color(tok);
    let y = baseline(ui, TS_BODY, r[1], r[3]);
    text(ui, r[0] + pad, y, sp.label.unwrap_or(""), c, 0);
}

/// sty_tab_pill(sp): 3 DIP accent pill (width from tab_metrics) 2 DIP above the bottom of the
/// strip, at value px from its left; value2 1 = disabled
fn tab_pill(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let pw = ui.dp(16); // pill width
    let c = ui.color(if sp.value2 != 0 { T_ACCENT_DISABLED } else { T_ACCENT });
    let ph = ui.dp(3); // pill height
    let d2 = ui.dp(2);
    let y = r[1] + r[3] - d2 - ph;
    fill_rrect(ui, r[0] + sp.value, y, pw, ph, ph >> 1, c);
}

/// sty_expander_header(sp): card-coloured header (control shades on hover / press; only the
/// top corners round while open), chevron in a 32 DIP square, label.  ev, value 1 = open,
/// value2 1 = disabled, label
fn expander_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let open = sp.value != 0;
    let dis = sp.value2 != 0;
    let rad = ui.dp(4);
    // header fill: card, hover / pressed control shades
    let tok = if ev & EV_HOVER == 0 {
        T_CARD
    } else if ev & EV_HELD == 0 {
        T_CTRL_HOVER
    } else {
        T_CTRL_PRESSED
    };
    let fill = ui.color(tok);
    let stroke = ui.color(T_CARD_STROKE);
    // open: only the top corners are round (clip away the bottom ones)
    if open {
        ui.draw_clip(r[0], r[1], r[2], r[3]);
    }
    let t = ui.hair();
    let h = if open { r[3] + rad } else { r[3] };
    ui.cmd_rrect(r[0], r[1], r[2], h, rad, t, fill, stroke, stroke);
    if open {
        ui.draw_unclip();
    }
    // chevron in a 32 DIP square, 8 DIP from the right edge
    let sq = ui.dp(32);
    let sx = r[0] + r[2] - ui.dp(8) - sq; // square x
    let sy = ((r[3] - sq) >> 1) + r[1]; // square y
    if ev & EV_HOVER != 0 {
        let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
        fill_rrect(ui, sx, sy, sq, sq, rad, c);
    }
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    ui.draw_color(c);
    let is = ui.dp(12);
    let d = (sq - is) >> 1;
    ui.draw_icon(if open { IC_CHEV_UP } else { IC_CHEV_DOWN }, sx + d, sy + d, is << 8);
    // header text
    let y = baseline(ui, TS_BODY, r[1], r[3]);
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    let mut tw = r[2] - ui.dp(16 + 48);
    if tw <= 0 {
        tw = 1;
    }
    let x = ui.dp(16) + r[0];
    text(ui, x, y, sp.label.unwrap_or(""), c, tw);
}

/// sty_expander_body(sp): content card below an open header: square top corners, a shared
/// hairline edge with the header
fn expander_body(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(4);
    ui.draw_clip(r[0], r[1], r[2], r[3]);
    let fill = if ui.theme_dark() { 0x08FF_FFFF } else { 0x80F6_F6F6 };
    let stroke = ui.color(T_CARD_STROKE);
    let t = ui.hair();
    ui.cmd_rrect(r[0], r[1] - rad, r[2], r[3] + rad, rad, t, fill, stroke, stroke);
    ui.draw_unclip();
    // the shared edge
    ui.draw_color(stroke);
    let t = ui.hair();
    ui.draw_rect(r[0], r[1], r[2], t);
}
