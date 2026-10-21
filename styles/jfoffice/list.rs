// jfOffice: Navigation rows, ribbon tabs, scrollbars and expanders.
// Uses jfguiLib drawing primitives; local implementations derived from the
// existing native widget contract. Reference geometry is implemented locally.

use ui::consts::*;
use ui::keys::{IC_CHEV_DOWN, IC_CHEV_RIGHT};
use ui::{Sp, StyleList, Ui};

use crate::Office;
use crate::common::{baseline, current_baseline, fill_rrect, stroke_rrect, text};

const BAR_DIP: i32 = 14; // scroll bar hover area
const THUMB_MIN_DIP: i32 = 24; // shortest scroll thumb

impl StyleList for Office {
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
        // Compact command strip with an underline drawn by the active tab.
        sp.s[0] = ui.dp(10);
        sp.s[1] = ui.dp(if sp.flags & ST_COMPACT != 0 { 22 } else { 30 });
        sp.s[2] = 0;
    }
    fn tab_font(&self, ui: &mut Ui, selected: bool) {
        ui.text.set_font(
            if selected { FONT_BOLD } else { FONT_SEMI },
            ui.dp_fx(12 * 256 + 128),
        );
    }
    fn tab(&self, ui: &mut Ui, sp: &Sp) {
        tab(ui, sp)
    }
    // nothing: the selected segment is drawn by tab
    fn tab_pill(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn expander_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 36 DIP header: 34 (disclosure chevron) + label + 16
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        (tw.wrapping_add(ui.dp(34 + 16)), ui.dp(36))
    }
    fn expander_header(&self, ui: &mut Ui, sp: &Sp) {
        expander_header(ui, sp)
    }
    fn expander_body(&self, ui: &mut Ui, sp: &Sp) {
        expander_body(ui, sp)
    }
}

/// sty_focus_rect(x, y, w, h): focus visual on a rectangle (list rows, tabs, expander
/// headers): a 1 DIP highlight outline, 3 DIP corners
fn focus_rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let r = ui.dp(3);
    let t = ui.dp(1);
    let c = ui.color(T_ACCENT);
    stroke_rrect(ui, x, y, w, h, r, t, c);
}

/// sty_scrollbar(sp): overlay scroller: 5 DIP thumb growing to 8 DIP with the expansion
/// (value 0..256), flyout-coloured track while expanded; flags 1 = vertical, s[0] track
/// start, s[1] thumb length, value2 thumb offset (px)
fn scrollbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let exp = sp.value;
    let (start, thumb, off) = (sp.s[0], sp.s[1], sp.value2);
    // thickness 5 -> 8 DIP with the expansion
    let t5 = ui.dp(5);
    let t = t5.wrapping_add(ui.dp(8).wrapping_sub(t5).wrapping_mul(exp) >> 8);
    // track background while expanded
    if exp != 0 {
        let a = (exp as u32).min(255);
        let c = ui.color(T_FLYOUT);
        let c = (c & 0xFF_FFFF) | (((c >> 24).wrapping_mul(a) >> 8) << 24);
        let rad = r[2].min(r[3]) >> 1;
        fill_rrect(ui, r[0], r[1], r[2], r[3], rad, c);
    }
    let c = ui.color(T_SCROLL);
    let rad = t >> 1;
    if sp.flags != 0 {
        let x = (r[2].wrapping_sub(t) >> 1).wrapping_add(r[0]);
        fill_rrect(ui, x, start.wrapping_add(off), t, thumb, rad, c);
    } else {
        let y = (r[3].wrapping_sub(t) >> 1).wrapping_add(r[1]);
        fill_rrect(ui, start.wrapping_add(off), y, thumb, t, rad, c);
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
    ui.draw_rrect(
        sp.x.wrapping_add(inset),
        sp.y.wrapping_sub(half),
        sp.w.wrapping_sub(inset).wrapping_sub(inset),
        t,
        half,
    );
}

/// sty_list_row(sp): selected: highlight fill (3 DIP corners, 3 / 1 DIP inset) with white
/// text (disabled: grey); hover: subtle shade; focus visual; label 12 DIP from the left.  ev,
/// value 1 = selected, value2 1 = disabled, flags 1 = dragged (pressed look), 2 = focus visual
fn list_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let sel = sp.value != 0;
    let dis = sp.value2 != 0;
    let flags = sp.flags;
    // background inset 3 / 1 DIP
    let ix = ui.dp(3);
    let iy = ui.dp(1);
    let (bx, by) = (r[0].wrapping_add(ix), r[1].wrapping_add(iy));
    let bw = r[2].wrapping_sub(ix).wrapping_sub(ix);
    let bh = r[3].wrapping_sub(iy).wrapping_sub(iy);
    let mut ttok = T_TEXT;
    // fill: selected accent (disabled: the pressed shade); else hover / pressed shades
    let tok;
    if sel {
        let mut t = T_ACCENT;
        ttok = T_ON_ACCENT;
        if flags & 1 != 0 {
            t = T_ACCENT_PRESSED;
        }
        if dis {
            t = T_SUBTLE_PRESSED;
            ttok = T_TEXT;
        }
        tok = t;
    } else {
        let mut t = 0; // 0 none, else token
        if ev & EV_HOVER != 0 {
            t = if ev & EV_HELD != 0 {
                T_SUBTLE_PRESSED
            } else {
                T_SUBTLE_HOVER
            };
        }
        if flags & 1 != 0 {
            t = T_SUBTLE_PRESSED; // the dragged row looks pressed
        }
        tok = t;
    }
    if tok != 0 {
        let c = ui.color(tok);
        fill_rrect(ui, bx, by, bw, bh, ix, c);
    }
    // focus visual (the accent fill already marks a selected row)
    if flags & 2 != 0 && !sel {
        focus_rect(ui, bx, by, bw, bh);
    }
    if let Some(l) = sp.label {
        let c = ui.color(if dis { T_TEXT_DISABLED } else { ttok });
        let mut tw = r[2].wrapping_sub(ui.dp(24));
        if tw <= 0 {
            tw = 1;
        }
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let x = ui.dp(12).wrapping_add(r[0]);
        text(ui, x, y, l, c, tw);
    }
}

/// A selected Office tab has blue type and a short underline; hovering only tints its face.
fn tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    if sp.flags & ST_COMPACT != 0 {
        if sp.value != 0 || (ev & EV_HOVER != 0 && sp.value2 == 0) {
            let fill = if sp.value != 0 {
                crate::palette::checked(ui.theme_dark())
            } else {
                ui.color(T_SUBTLE_HOVER)
            };
            let margin = ui.dp(2);
            fill_rrect(ui, r[0], r[1], (r[2] - margin).max(0), r[3], ui.dp(4), fill);
        }
        ui.text.set_font(FONT_REG, ui.dp_fx(12 * 256 + 128));
        let color = ui.color(if sp.value2 != 0 {
            T_TEXT_DISABLED
        } else if sp.value != 0 {
            T_ACCENT
        } else {
            T_TEXT
        });
        let label = sp.label.unwrap_or("");
        let width = ui.text.width(label);
        let base = current_baseline(ui, r[1], r[3]);
        text(
            ui,
            r[0] + (r[2] - ui.dp(2) - width) / 2,
            base,
            label,
            color,
            0,
        );
        return;
    }
    if ev & EV_HOVER != 0 && sp.value2 == 0 && sp.value == 0 {
        let c = ui.color(if ev & EV_HELD != 0 {
            T_SUBTLE_PRESSED
        } else {
            T_SUBTLE_HOVER
        });
        let rad = ui.dp(4);
        let margin = ui.dp(4);
        fill_rrect(ui, r[0], r[1] + margin, r[2], r[3] - 2 * margin, rad, c);
    }
    if sp.value != 0 {
        let h = ui.dp(3);
        let c = ui.color(if sp.value2 != 0 {
            T_TEXT_DISABLED
        } else {
            T_ACCENT
        });
        fill_rrect(
            ui,
            r[0] + ui.dp(8),
            r[1] + r[3] - ui.dp(2) - h / 2,
            (r[2] - ui.dp(16)).max(h),
            h,
            ui.dp(2),
            c,
        );
    }
    let mut tok = if sp.value != 0 { T_ACCENT_TEXT } else { T_TEXT };
    if sp.value2 != 0 {
        tok = T_TEXT_DISABLED;
    }
    let c = ui.color(tok);
    Office.tab_font(ui, sp.value != 0);
    let y = current_baseline(ui, r[1], r[3]);
    let label = sp.label.unwrap_or("");
    let width = ui.text.width(label);
    text(ui, r[0] + (r[2] - width) / 2, y, label, c, 0);
}

/// sty_expander_header(sp): grouped-box header (card fill, 4 DIP corners, only the top ones
/// round while open; pressed shade), disclosure chevron (right / down) 12 DIP from the left,
/// label.  ev, value 1 = open, value2 1 = disabled, label
fn expander_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    let open = sp.value != 0;
    let dis = sp.value2 != 0;
    let rad = ui.dp(4);
    // header fill: card (pressed: the control's pressed shade)
    let tok = if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD {
        T_CTRL_PRESSED
    } else {
        T_CARD
    };
    let fill = ui.color(tok);
    let stroke = ui.color(T_CARD_STROKE);
    // open: only the top corners are round (clip away the bottom ones)
    if open {
        ui.draw_clip(r[0], r[1], r[2], r[3]);
    }
    let t = ui.dp(1);
    let h = if open { r[3].wrapping_add(rad) } else { r[3] };
    ui.cmd_rrect(r[0], r[1], r[2], h, rad, t, fill, stroke, stroke);
    if open {
        ui.draw_unclip();
    }
    // disclosure chevron (12 DIP) 12 DIP from the left edge
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT2 });
    ui.draw_color(c);
    let s = ui.dp(12);
    let x = r[0].wrapping_add(s);
    let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
    ui.draw_icon(
        if open { IC_CHEV_DOWN } else { IC_CHEV_RIGHT },
        x,
        y,
        s << 8,
    );
    // header text
    let y = baseline(ui, TS_BODY, r[1], r[3]);
    let c = ui.color(if dis { T_TEXT_DISABLED } else { T_TEXT });
    let mut tw = r[2].wrapping_sub(ui.dp(34 + 16));
    if tw <= 0 {
        tw = 1;
    }
    let x = ui.dp(34).wrapping_add(r[0]);
    text(ui, x, y, sp.label.unwrap_or(""), c, tw);
}

/// sty_expander_body(sp): content card below an open header: square top corners, a shared
/// hairline edge with the header
fn expander_body(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(4);
    ui.draw_clip(r[0], r[1], r[2], r[3]);
    let fill = ui.color(T_CARD);
    let stroke = ui.color(T_CARD_STROKE);
    let t = ui.dp(1);
    ui.cmd_rrect(
        r[0],
        r[1].wrapping_sub(rad),
        r[2],
        r[3].wrapping_add(rad),
        rad,
        t,
        fill,
        stroke,
        stroke,
    );
    ui.draw_unclip();
    // the shared edge
    ui.draw_color(stroke);
    let t = ui.dp(1);
    ui.draw_rect(r[0], r[1], r[2], t);
}
