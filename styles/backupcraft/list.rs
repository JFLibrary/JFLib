// list -- BackupCraft focus rectangles, scroll bars, list rows, tabs, expanders
//
//   focus_rect            2 DIP accent frame on the rectangle (4 DIP corners)
//   scrollbar             12 DIP area; a 4 DIP thumb that widens to 8 DIP with a visible
//                         track (KDE) while pointed at
//   list_row              selected: accent tint with a 3 x 16 DIP accent pill (WinUI); hover:
//                         subtle shade; 4 DIP corners, 4 / 1 DIP inset
//   tabs                  label padding 14, 38 DIP strip; the selected tab underlined over its
//                         full width with a 2 DIP accent bar (KDE), semibold label
//   expander              a card header (8 DIP corners) with the chevron at the right end

use ui::consts::*;
use ui::keys::{IC_CHEV_DOWN, IC_CHEV_UP};
use ui::theme::color_mix;
use ui::{Sp, StyleList, Ui};

use crate::common::{baseline, fill_rrect, stroke_rrect, text, R_CARD};
use crate::BackupCraft;

impl StyleList for BackupCraft {
    fn focus_rect(&self, ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
        focus_rect(ui, x, y, w, h)
    }
    fn scrollbar_thickness(&self, ui: &mut Ui) -> i32 {
        ui.dp(12)
    }
    fn scrollbar_margin(&self, ui: &mut Ui) -> i32 {
        ui.dp(2)
    }
    fn scrollbar_min_thumb(&self, ui: &mut Ui) -> i32 {
        ui.dp(32)
    }
    fn scrollbar(&self, ui: &mut Ui, sp: &Sp) {
        scrollbar(ui, sp)
    }
    fn list_drop_line(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        let t = ui.dp(2);
        let inset = ui.dp(4);
        ui.draw_rrect(sp.x + inset, sp.y - (t >> 1), sp.w - inset * 2, t, t >> 1);
    }
    fn list_row(&self, ui: &mut Ui, sp: &Sp) {
        list_row(ui, sp)
    }
    fn tab_metrics(&self, ui: &mut Ui, sp: &mut Sp) {
        sp.s[0] = ui.dp(14);
        sp.s[1] = ui.dp(38);
        sp.s[2] = 0;
    }
    fn tab_font(&self, ui: &mut Ui, selected: bool) {
        ui.ts_font(if selected { TS_BODY_STRONG } else { TS_BODY });
    }
    fn tab(&self, ui: &mut Ui, sp: &Sp) {
        tab(ui, sp)
    }
    fn tab_pill(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn expander_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        (tw + ui.dp(16 + 40), ui.dp(44))
    }
    fn expander_header(&self, ui: &mut Ui, sp: &Sp) {
        expander_header(ui, sp)
    }
    fn expander_body(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_CARD);
        ui.draw_clip(r[0], r[1], r[2], r[3]);
        let fill = ui.color(T_CARD);
        let stroke = ui.color(T_CARD_STROKE);
        let t = ui.hair();
        ui.cmd_rrect(r[0], r[1] - rad, r[2], r[3] + rad, rad, t, fill, stroke, stroke);
        ui.draw_unclip();
        ui.draw_color(stroke);
        ui.draw_rect(r[0], r[1], r[2], t);
    }
}

pub(crate) fn focus_rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let r = ui.dp(4);
    let t = ui.dp(2);
    let c = ui.color(T_FOCUS_OUT);
    stroke_rrect(ui, x, y, w, h, r, t, c);
}

fn scrollbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let exp = sp.value;
    let (start, thumb, off) = (sp.s[0], sp.s[1], sp.value2);
    let t4 = ui.dp(4);
    let t = t4 + ((ui.dp(8) - t4) * exp >> 8);
    if exp != 0 {
        // the visible track (KDE)
        let a = (exp as u32).min(255);
        let c = ui.color(T_TRACK);
        let c = (c & 0xFF_FFFF) | (((c >> 24) * a >> 8) << 24);
        if sp.flags != 0 {
            let x = r[0] + ((r[2] - t) >> 1);
            fill_rrect(ui, x, r[1] + ui.dp(2), t, r[3] - ui.dp(4), t >> 1, c);
        } else {
            let y = r[1] + ((r[3] - t) >> 1);
            fill_rrect(ui, r[0] + ui.dp(2), y, r[2] - ui.dp(4), t, t >> 1, c);
        }
    }
    let c = ui.color(T_SCROLL);
    let c = if exp > 128 { color_mix(c, ui.color(T_TEXT2) | 0xFF00_0000, 96) } else { c };
    if sp.flags != 0 {
        let x = r[0] + ((r[2] - t) >> 1);
        fill_rrect(ui, x, start + off, t, thumb, t >> 1, c);
    } else {
        let y = r[1] + ((r[3] - t) >> 1);
        fill_rrect(ui, start + off, y, thumb, t, t >> 1, c);
    }
}

fn list_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let sel = sp.value != 0;
    let dis = sp.value2 != 0;
    let flags = sp.flags;
    let ix = ui.dp(4);
    let iy = ui.dp(1);
    let (bx, by, bw, bh) = (r[0] + ix, r[1] + iy, r[2] - ix * 2, r[3] - iy * 2);
    let rad = ui.dp(4);
    let accent = ui.color(T_ACCENT);
    if sel {
        let card = ui.color(T_CARD) | 0xFF00_0000;
        let mut fill = color_mix(card, accent, if ui.theme_dark() { 64 } else { 38 });
        if ev & EV_HOVER != 0 || flags & 1 != 0 {
            fill = color_mix(fill, accent, 20);
        }
        fill_rrect(ui, bx, by, bw, bh, rad, fill);
        if !dis {
            let (pw, ph) = (ui.dp(3), ui.dp(16).min(bh - ui.dp(8)));
            fill_rrect(ui, bx, by + ((bh - ph) >> 1), pw, ph, pw >> 1, accent);
        }
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
        let tw = (r[2] - ui.dp(28)).max(1);
        let y = baseline(ui, if sel { TS_BODY_STRONG } else { TS_BODY }, r[1], r[3]);
        text(ui, r[0] + ui.dp(16), y, l, c, tw);
    }
}

fn tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let pad = ui.dp(14);
    let sel = sp.value != 0;
    if ev & EV_HOVER != 0 && !sel {
        let c = ui.color(T_SUBTLE_HOVER);
        fill_rrect(ui, r[0] + ui.dp(2), r[1] + ui.dp(4), r[2] - ui.dp(4), r[3] - ui.dp(8), ui.dp(4), c);
    }
    let h = ui.dp(2);
    if sel {
        let c = ui.color(if sp.value2 != 0 { T_ACCENT_DISABLED } else { T_ACCENT });
        fill_rrect(ui, r[0] + ui.dp(4), r[1] + r[3] - h, r[2] - ui.dp(8), h, h >> 1, c);
    }
    let tok = if sp.value2 != 0 {
        T_TEXT_DISABLED
    } else if sel || ev & EV_HOVER != 0 {
        T_TEXT
    } else {
        T_TEXT2
    };
    let c = ui.color(tok);
    let ts = if sel { TS_BODY_STRONG } else { TS_BODY };
    let y = baseline(ui, ts, r[1], r[3]);
    text(ui, r[0] + pad, y, sp.label.unwrap_or(""), c, 0);
}

fn expander_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let open = sp.value != 0;
    let dis = sp.value2 != 0;
    let rad = ui.dp(R_CARD);
    let fill = ui.color(if ev & EV_HELD != 0 {
        T_CTRL_PRESSED
    } else if ev & EV_HOVER != 0 {
        T_CTRL_HOVER
    } else {
        T_CARD
    });
    let stroke = ui.color(T_CARD_STROKE);
    if open {
        ui.draw_clip(r[0], r[1], r[2], r[3]);
    }
    let t = ui.hair();
    let h = if open { r[3] + rad } else { r[3] };
    ui.cmd_rrect(r[0], r[1], r[2], h, rad, t, fill | 0xFF00_0000, stroke, stroke);
    if open {
        ui.draw_unclip();
    }
    let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    text(ui, r[0] + ui.dp(16), y, sp.label.unwrap_or(""), c, (r[2] - ui.dp(16 + 40)).max(1));
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
    ui.draw_color(c);
    let s = ui.dp(12);
    ui.draw_icon(if open { IC_CHEV_UP } else { IC_CHEV_DOWN }, r[0] + r[2] - ui.dp(16) - s, r[1] + ((r[3] - s) >> 1), s << 8);
}
