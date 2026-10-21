// edit -- the kde text boxes and number boxes                             [styles/kde/edit.s]
//
//   textbox_metrics(sp) -> (w, h)   30 DIP high (multi-line: 5 lines + 2 x 6 DIP), insets:
//                                   left 8 (border + 7), right 8, top 6 DIP
//   textbox_frame(sp)               white field, grey outline (3 DIP corners), highlight
//                                   outline on hover (half) and while editing
//   textbox_vbar(sp)                2 DIP scroll indicator of a multi-line box
//
// The text, placeholder, selection and caret are drawn by the core in T_TEXT, T_TEXT2,
// T_TEXT_DISABLED, T_ACCENT (selection) and T_ON_ACCENT (selected text).
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::{Sp, StyleEdit, Ui};

use crate::basic::focus_ring;
use crate::Kde;

const TB_H: i32 = 30; // height (DIP)
const TB_PADL: i32 = 8; // text inset: 1 border + 7 padding (DIP)
const TB_PADR: i32 = 8;
const TB_RADIUS: i32 = 3;
const TBM_LINES: i32 = 5; // natural height of a multi-line box in lines
const TBM_PADT: i32 = 6; // vertical text inset (DIP)

impl StyleEdit for Kde {
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
    let hair = ui.hair();
    let rad = ui.dp(TB_RADIUS);
    let fill = ui.color(if disabled { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let mut stroke = ui.color(T_CTRL_STROKE);
    if ev & EV_HOVER != 0 && !disabled {
        // hover: half-transparent highlight outline
        stroke = (ui.color(T_ACCENT) & 0x00FF_FFFF) | 0x9900_0000;
    }
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
    // editing: the focus ring around the field
    if editing {
        focus_ring(ui, r, TB_RADIUS);
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
