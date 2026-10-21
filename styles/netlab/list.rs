// list -- NetLab focus rectangles, scroll bars, list rows, tabs, expanders
//
//   focus_rect            1 DIP accent frame on the rectangle (2 DIP corners)
//   scrollbar             10 DIP area: an always visible 6 DIP track with a 6 DIP thumb that
//                         widens to 8 DIP and darkens while pointed at; 2 DIP corners
//   list_row              selected: the selection (16 % accent tint, 1 DIP accent frame);
//                         hover: a subtle shade; 2 DIP corners, 2 / 1 DIP inset
//   tabs                  label padding 12, 30 DIP strip; a divider along the bottom of the
//                         tabs, the selected tab underlined with a 2 DIP accent bar
//   expander              a framed header in the panel colour (4 DIP corners) with the
//                         chevron at the right end over a framed card body

use ui::consts::*;
use ui::keys::{IC_CHEV_DOWN, IC_CHEV_UP};
use ui::theme::color_mix;
use ui::{Sp, StyleList, Ui};

use crate::common::{baseline, card, fill_rrect, framed, select_colors, stroke_rrect, text, R_CARD};
use crate::Netlab;

impl StyleList for Netlab {
    fn focus_rect(&self, ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
        focus_rect(ui, x, y, w, h)
    }
    fn scrollbar_thickness(&self, ui: &mut Ui) -> i32 {
        ui.dp(10)
    }
    fn scrollbar_margin(&self, ui: &mut Ui) -> i32 {
        ui.dp(1)
    }
    fn scrollbar_min_thumb(&self, ui: &mut Ui) -> i32 {
        ui.dp(24)
    }
    fn scrollbar(&self, ui: &mut Ui, sp: &Sp) {
        scrollbar(ui, sp)
    }
    fn list_drop_line(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        let t = ui.dp(2);
        let inset = ui.dp(2);
        ui.draw_rect(sp.x + inset, sp.y - (t >> 1), sp.w - inset * 2, t);
    }
    fn list_row(&self, ui: &mut Ui, sp: &Sp) {
        list_row(ui, sp)
    }
    fn tab_metrics(&self, ui: &mut Ui, sp: &mut Sp) {
        sp.s[0] = ui.dp(12);
        sp.s[1] = ui.dp(30);
        sp.s[2] = 0;
    }
    fn tab_font(&self, ui: &mut Ui, _selected: bool) {
        ui.ts_font(TS_BODY);
    }
    fn tab(&self, ui: &mut Ui, sp: &Sp) {
        tab(ui, sp)
    }
    fn tab_pill(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn expander_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        (tw + ui.dp(12 + 36), ui.dp(32))
    }
    fn expander_header(&self, ui: &mut Ui, sp: &Sp) {
        expander_header(ui, sp)
    }
    fn expander_body(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_CARD);
        ui.draw_clip(r[0], r[1], r[2], r[3]);
        let fill = card(ui);
        let stroke = ui.color(T_CARD_STROKE);
        framed(ui, r[0], r[1] - rad, r[2], r[3] + rad, rad, fill, stroke);
        ui.draw_unclip();
        let t = ui.hair();
        ui.draw_color(stroke);
        ui.draw_rect(r[0], r[1], r[2], t);
    }
}

pub(crate) fn focus_rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let r = ui.dp(2);
    let t = ui.dp(1).max(ui.hair());
    let c = ui.color(T_FOCUS_OUT);
    stroke_rrect(ui, x, y, w, h, r, t, c);
}

fn scrollbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let exp = sp.value.clamp(0, 256);
    let (start, thumb, off) = (sp.s[0], sp.s[1], sp.value2);
    let t6 = ui.dp(6);
    let t = t6 + ((ui.dp(8) - t6) * exp >> 8);
    let rad = ui.dp(2);
    let m = ui.dp(1);
    // the track: always visible (thin), full width while pointed at
    let track = ui.color(T_TRACK);
    let thumb_c = ui.color(T_SCROLL);
    let thumb_c = if exp > 0 { color_mix(thumb_c, ui.color(T_TEXT2) | 0xFF00_0000, exp >> 1) } else { thumb_c };
    if sp.flags != 0 {
        let x = r[0] + ((r[2] - t) >> 1);
        fill_rrect(ui, x, r[1] + m, t, r[3] - m * 2, rad, track);
        fill_rrect(ui, x, start + off, t, thumb, rad, thumb_c);
    } else {
        let y = r[1] + ((r[3] - t) >> 1);
        fill_rrect(ui, r[0] + m, y, r[2] - m * 2, t, rad, track);
        fill_rrect(ui, start + off, y, thumb, t, rad, thumb_c);
    }
}

fn list_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let sel = sp.value != 0;
    let dis = sp.value2 != 0;
    let flags = sp.flags;
    let ix = ui.dp(2);
    let iy = ui.dp(1);
    let (bx, by, bw, bh) = (r[0] + ix, r[1] + iy, r[2] - ix * 2, r[3] - iy * 2);
    let rad = ui.dp(2);
    if sel {
        let base = card(ui);
        let (fill, stroke) = select_colors(ui, base, ev & EV_HOVER != 0 || flags & 1 != 0);
        let stroke = if dis { ui.color(T_CTRL_STROKE) } else { stroke };
        framed(ui, bx, by, bw, bh, rad, fill, stroke);
    } else {
        let mut tok = 0;
        if ev & EV_HOVER != 0 {
            tok = if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER };
        }
        if flags & 1 != 0 {
            tok = T_SUBTLE_PRESSED;
        }
        if tok != 0 {
            let c = ui.color(tok);
            fill_rrect(ui, bx, by, bw, bh, rad, c);
        }
    }
    // the focus visual marks the focused list's selected row (the list itself: list_end)
    if flags & 2 != 0 && sel {
        focus_rect(ui, bx, by, bw, bh);
    }
    if let Some(l) = sp.label {
        let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
        let tw = (r[2] - ui.dp(24)).max(1);
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        ui.ts_font(TS_BODY);
        text(ui, r[0] + ui.dp(12), y, l, c, tw);
    }
}

fn tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let pad = ui.dp(12);
    let sel = sp.value != 0;
    let hair = ui.hair();
    if ev & EV_HOVER != 0 && !sel {
        let c = ui.color(T_SUBTLE_HOVER);
        fill_rrect(ui, r[0], r[1] + ui.dp(3), r[2], r[3] - ui.dp(3) - hair, ui.dp(2), c);
    }
    // the strip's divider under every tab, the selected one underlined in the accent
    let d = ui.color(T_CTRL_STROKE);
    ui.draw_color(d);
    ui.draw_rect(r[0], r[1] + r[3] - hair, r[2], hair);
    if sel {
        let h = ui.dp(2);
        let c = ui.color(if sp.value2 != 0 { T_ACCENT_DISABLED } else { T_ACCENT });
        ui.draw_color(c);
        ui.draw_rect(r[0] + ui.dp(2), r[1] + r[3] - h, r[2] - ui.dp(4), h);
    }
    let tok = if sp.value2 != 0 {
        T_TEXT_DISABLED
    } else if sel || ev & EV_HOVER != 0 {
        T_TEXT
    } else {
        T_TEXT2
    };
    let c = ui.color(tok);
    let y = baseline(ui, TS_BODY, r[1], r[3]);
    ui.ts_font(TS_BODY);
    text(ui, r[0] + pad, y, sp.label.unwrap_or(""), c, 0);
}

fn expander_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let open = sp.value != 0;
    let dis = sp.value2 != 0;
    let rad = ui.dp(R_CARD);
    let base = ui.color(T_LAYER) | 0xFF00_0000;
    let fill = if ev & EV_HELD != 0 {
        color_mix(base, ui.color(T_TEXT) | 0xFF00_0000, 24)
    } else if ev & EV_HOVER != 0 {
        color_mix(base, ui.color(T_TEXT) | 0xFF00_0000, 12)
    } else {
        base
    };
    let stroke = ui.color(T_CARD_STROKE);
    if open {
        ui.draw_clip(r[0], r[1], r[2], r[3]);
    }
    let h = if open { r[3] + rad } else { r[3] };
    framed(ui, r[0], r[1], r[2], h, rad, fill, stroke);
    if open {
        ui.draw_unclip();
    }
    let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    ui.ts_font(TS_BODY_STRONG);
    text(ui, r[0] + ui.dp(12), y, sp.label.unwrap_or(""), c, (r[2] - ui.dp(12 + 36)).max(1));
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
    ui.draw_color(c);
    let s = ui.dp(12);
    ui.draw_icon(if open { IC_CHEV_UP } else { IC_CHEV_DOWN }, r[0] + r[2] - ui.dp(12) - s, r[1] + ((r[3] - s) >> 1), s << 8);
}
