// edit -- the winui text boxes and number boxes                        [styles/winui/edit.s]
//
//   textbox_metrics(sp) -> (w, h)   32 DIP high (multi-line: 5 lines + 2 x 6 DIP), insets:
//                                   left 11 (border + 10), right 8, top 6 DIP
//   textbox_frame(sp)               control fill by state (input fill while editing),
//                                   hairline border, bottom edge: strong hairline or 2 DIP
//                                   accent while editing
//   textbox_vbar(sp)                2 DIP scroll indicator of a multi-line box
//
// The text, placeholder, selection and caret are drawn by the core in T_TEXT, T_TEXT2,
// T_TEXT_DISABLED, T_ACCENT (selection) and T_ON_ACCENT (selected text).
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::{Sp, StyleEdit, Ui};

use crate::WinUi;

const TB_H: i32 = 32; // height (DIP)
const TB_PADL: i32 = 11; // text inset: 1 border + 10 padding (DIP)
const TB_PADR: i32 = 8;
const TB_RADIUS: i32 = 4;
const TBM_LINES: i32 = 5; // natural height of a multi-line box in lines
const TBM_PADT: i32 = 6; // vertical text inset (DIP)

impl StyleEdit for WinUi {
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
    let mut t = T_CTRL;
    if ev & EV_HOVER != 0 {
        t = T_CTRL_HOVER;
    }
    if editing {
        t = T_INPUT_ACTIVE;
    }
    if disabled {
        t = T_CTRL_DISABLED;
    }
    let fill = ui.color(t);
    let stroke = ui.color(T_CTRL_STROKE);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
    // bottom edge: strong hairline, or 2 DIP accent while focused
    if disabled {
        return;
    }
    let (bh, tok) = if editing { (ui.dp(2), T_ACCENT) } else { (hair, T_STRONG_STROKE) };
    let c = ui.color(tok);
    ui.draw_clip(r[0], r[1].wrapping_add(r[3]).wrapping_sub(bh), r[2], bh);
    ui.draw_color(c);
    ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
    ui.draw_unclip();
}

/// sty_textbox_vbar(sp): value view height, value2 content height (> view), s[0] scroll y,
/// s[1] text top (px): a 2 DIP pill 3 DIP from the right edge, at least 16 DIP long
fn textbox_vbar(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (vh, ch, sy, top) = (sp.value, sp.value2, sp.s[0], sp.s[1]);
    let c = ui.color(T_TEXT3);
    ui.draw_color(c);
    let bh = (vh.wrapping_mul(vh) / ch).max(ui.dp(16)); // bar height
    let y = (vh.wrapping_sub(bh).wrapping_mul(sy) / ch.wrapping_sub(vh)).wrapping_add(top);
    let w = ui.dp(2);
    let m = ui.dp(3);
    let x = r[0].wrapping_add(r[2]).wrapping_sub(m).wrapping_sub(w);
    ui.draw_rrect(x, y, w, bh, ((w as u32) >> 1) as i32);
}
