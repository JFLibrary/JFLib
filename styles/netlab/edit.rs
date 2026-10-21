// edit -- NetLab text boxes and number boxes
//
//   textbox_metrics(sp) -> (w, h)   28 DIP high (multi-line: 5 lines + 2 x 5 DIP); insets
//                                   left / right 8, top 5 DIP
//   textbox_frame(sp)               a white field with a 1 DIP frame (3 DIP corners) in the
//                                   control stroke, a shade darker along the bottom; the frame
//                                   darkens on hover and turns accent while editing
//   textbox_vbar(sp)                3 DIP scroll indicator of a multi-line box

use ui::consts::*;
use ui::theme::color_mix;
use ui::{Sp, StyleEdit, Ui};

use crate::common::R_CTRL;
use crate::Netlab;

const TB_H: i32 = 28;
const TB_PADL: i32 = 8;
const TB_PADR: i32 = 8;
const TBM_LINES: i32 = 5;
const TBM_PADT: i32 = 5;

impl StyleEdit for Netlab {
    fn textbox_metrics(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        sp.s[0] = ui.dp(TB_PADL);
        sp.s[1] = ui.dp(TB_PADR);
        sp.s[2] = ui.dp(TBM_PADT);
        let w = ui.dp(sp.value);
        if sp.flags == 0 {
            return (w, ui.dp(TB_H));
        }
        (w, sp.value2 * TBM_LINES + ui.dp(TBM_PADT * 2))
    }
    fn textbox_frame(&self, ui: &mut Ui, sp: &Sp) {
        textbox_frame(ui, sp)
    }
    fn textbox_vbar(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let (vh, ch, sy, top) = (sp.value, sp.value2, sp.s[0], sp.s[1]);
        if ch <= vh {
            return;
        }
        let c = ui.color(T_SCROLL);
        ui.draw_color(c);
        let bh = (vh * vh / ch).max(ui.dp(16));
        let y = (vh - bh) * sy / (ch - vh) + top;
        let w = ui.dp(3);
        let x = r[0] + r[2] - ui.dp(3) - w;
        ui.draw_rect(x, y, w, bh);
    }
}

/// textbox_frame: ev (hover), value 1 = editing, value2 1 = disabled
fn textbox_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, editing, disabled) = (sp.ev, sp.value != 0, sp.value2 != 0);
    let hair = ui.hair();
    let rad = ui.dp(R_CTRL);
    let fill = ui.color(if disabled { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let (stroke, stroke2) = if disabled {
        let s = ui.color(T_CTRL_STROKE);
        (s, s)
    } else if editing {
        let a = ui.color(T_ACCENT);
        (a, a)
    } else if ev & EV_HOVER != 0 {
        let s = color_mix(ui.color(T_CTRL_STROKE), ui.color(T_STRONG_STROKE), 128);
        (s, s)
    } else {
        (ui.color(T_CTRL_STROKE), ui.color(T_CTRL_STROKE2))
    };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke2);
}
