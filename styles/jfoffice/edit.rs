// jfOffice: Compact text inputs with a clear editing outline.
// Uses jfguiLib drawing primitives; local implementations derived from the
// existing native widget contract. Reference geometry is implemented locally.

use ui::consts::*;
use ui::{Sp, StyleEdit, Ui};

use crate::Office;
use crate::basic::focus_ring;

const TB_H: i32 = 26; // aligned with standard controls (DIP)
const TB_PADL: i32 = 7; // text inset includes the 1-DIP border
const TB_PADR: i32 = 7;
const TB_RADIUS: i32 = 4;
const TBM_LINES: i32 = 5; // natural height of a multi-line box in lines
const TBM_PADT: i32 = 4; // vertical text inset (DIP)

impl StyleEdit for Office {
    fn textbox_font(&self, ui: &mut Ui, sp: &Sp) -> (i32, i32) {
        if sp.icon == 0 { return (ui.ts_line(TS_BODY), ui.ts_base(TS_BODY)); }
        crate::common::control_font(ui);
        let line = ui.dp(16);
        (line, crate::common::current_baseline(ui, 0, line))
    }
    fn textbox_metrics(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        textbox_metrics(ui, sp)
    }
    fn textbox_frame(&self, ui: &mut Ui, sp: &Sp) {
        textbox_frame(ui, sp)
    }
    fn textbox_vbar(&self, ui: &mut Ui, sp: &Sp) {
        textbox_vbar(ui, sp)
    }
}

/// sty_textbox_metrics(sp) -> (w, h): value natural width (DIP), flags 1 = multi-line (value2
/// = line height px); sets s[0] / s[1] / s[2] = left / right / top text inset (px)
fn textbox_metrics(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    sp.s[0] = ui.dp(TB_PADL);
    sp.s[1] = ui.dp(TB_PADR);
    sp.s[2] = ui.dp(TBM_PADT);
    let w = ui.dp(sp.value);
    if sp.flags == 0 {
        return (w, ui.dp(TB_H));
    }
    let pad = ui.dp(TBM_PADT * 2);
    (w, sp.value2.wrapping_mul(TBM_LINES).wrapping_add(pad))
}

/// sty_textbox_frame(sp): ev (hover), value 1 = editing, value2 1 = disabled
fn textbox_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, editing, disabled) = (sp.ev, sp.value != 0, sp.value2 != 0);
    let hair = ui.dp(1);
    let radius = if sp.icon == 0 { TB_RADIUS } else { 6 };
    let rad = ui.dp(radius);
    let fill = ui.color(if disabled {
        T_CTRL_DISABLED
    } else if sp.icon != 0 && !editing {
        T_LAYER
    } else {
        T_INPUT_ACTIVE
    });
    let mut stroke = ui.color(if sp.icon == 0 { T_CTRL_STROKE } else { T_LAYER_STROKE });
    if ev & EV_HOVER != 0 && !disabled {
        stroke = ui.color(T_STRONG_STROKE);
    }
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
    // editing: the focus ring around the field
    if editing {
        focus_ring(ui, r, radius);
    }
}

/// sty_textbox_vbar(sp): value view height, value2 content height (> view), s[0] scroll y,
/// s[1] text top (px): a 2 DIP pill 3 DIP from the right edge, at least 16 DIP long
fn textbox_vbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (vh, ch, sy, top) = (sp.value, sp.value2, sp.s[0], sp.s[1]);
    let c = ui.color(T_TEXT3);
    ui.draw_color(c);
    let bh = (vh.wrapping_mul(vh) / ch).max(ui.dp(16)); // bar height
    let y = (vh.wrapping_sub(bh).wrapping_mul(sy) / ch.wrapping_sub(vh)).wrapping_add(top); // bar y
    let w = ui.dp(2);
    let m = ui.dp(3);
    let x = r[0].wrapping_add(r[2]).wrapping_sub(m).wrapping_sub(w);
    ui.draw_rrect(x, y, w, bh, ((w as u32) >> 1) as i32);
}
