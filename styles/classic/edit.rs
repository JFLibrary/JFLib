// edit -- the classic text boxes                     [styles/classic/list.s, text boxes]
//
//   textbox_metrics(sp) -> (w, h)   21 DIP high (multi-line: 5 lines + 2 x 3 DIP); insets
//                                   4 / 4 / 3 DIP
//   textbox_frame(sp)               white field (face when disabled), sunken 3D frame
//   textbox_vbar(sp)                a slim raised thumb at the right edge of a multi-line box
//
// The text, placeholder, selection and caret are drawn by the core.
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::{Sp, StyleEdit, Ui};

use crate::common::*;
use crate::Classic;

const TB_H: i32 = 21; // text box height (DIP)
const TB_PAD: i32 = 4; // text inset left / right
const TBM_PADT: i32 = 3; // top inset of a multi-line box
const TBM_LINES: i32 = 5;

impl StyleEdit for Classic {
    fn textbox_metrics(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        textbox_metrics(ui, sp)
    }
    fn textbox_frame(&self, ui: &mut Ui, sp: &Sp) {
        let c = pal(ui, if sp.value2 != 0 { CP_FACE } else { CP_FIELD });
        fill(ui, sp.rect(), c);
        frame(ui, sp.rect(), FR_SUNKEN);
    }
    fn textbox_vbar(&self, ui: &mut Ui, sp: &Sp) {
        textbox_vbar(ui, sp)
    }
}

/// sty_textbox_metrics(sp) -> (w, h): value natural width (DIP), flags 1 = multi-line (value2
/// = line height px); sets s[0] / s[1] / s[2] = left / right / top text inset (px)
fn textbox_metrics(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let p = ui.dp(TB_PAD);
    sp.s[0] = p;
    sp.s[1] = p;
    sp.s[2] = ui.dp(TBM_PADT);
    let w = ui.dp(sp.value);
    if sp.flags == 0 {
        return (w, ui.dp(TB_H));
    }
    let pad = ui.dp(TBM_PADT * 2);
    (w, sp.value2.wrapping_mul(TBM_LINES).wrapping_add(pad))
}

/// sty_textbox_vbar(sp): value view h, value2 content h, s[0] scroll y, s[1] text top (px)
fn textbox_vbar(ui: &mut Ui, sp: &Sp) {
    let (vh, ch) = (sp.value, sp.value2);
    let mut th = vh.wrapping_mul(vh) / ch; // thumb height
    let m = ui.dp(12);
    if th < m {
        th = m;
    }
    let mut den = ch.wrapping_sub(vh);
    if den < 1 {
        den = 1;
    }
    let y = (vh.wrapping_sub(th).wrapping_mul(sp.s[0]) / den).wrapping_add(sp.s[1]);
    let w = ui.dp(6);
    let t = ui.hair();
    let x = sp.x.wrapping_add(sp.w).wrapping_sub(w).wrapping_sub(t + t);
    let r = [x, y, w, th];
    let c = pal(ui, CP_FACE);
    fill(ui, r, c);
    frame(ui, r, FR_THIN_RAISED);
}
