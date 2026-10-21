// edit -- BackupCraft text boxes and number boxes
//
//   textbox_metrics(sp) -> (w, h)   32 DIP high (multi-line: 5 lines + 2 x 7 DIP); insets
//                                   left / right 10, top 7 DIP; icon boxes keep 34 left
//   textbox_frame(sp)               a framed field (KDE): input fill, control outline with a
//                                   darker bottom edge, the outline tinted on hover; while
//                                   editing an accent outline with a 2 DIP accent line along
//                                   the bottom (WinUI)
//   textbox_vbar(sp)                3 DIP scroll indicator of a multi-line box

use ui::consts::*;
use ui::theme::color_mix;
use ui::{Sp, StyleEdit, Ui};

use crate::common::R_CTRL;
use crate::BackupCraft;

const TB_H: i32 = 32;
const TB_PADL: i32 = 10;
const TB_PADR: i32 = 10;
const TBM_LINES: i32 = 5;
const TBM_PADT: i32 = 7;

impl StyleEdit for BackupCraft {
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
        ui.draw_rrect(x, y, w, bh, w >> 1);
    }
}

/// textbox_frame: ev (hover), value 1 = editing, value2 1 = disabled
fn textbox_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, editing, disabled) = (sp.ev, sp.value != 0, sp.value2 != 0);
    let hair = ui.hair();
    let rad = ui.dp(R_CTRL);
    let fill = ui.color(if disabled { T_CTRL_DISABLED } else { T_INPUT_ACTIVE });
    let accent = ui.color(T_ACCENT);
    let (stroke, stroke2) = if disabled {
        let s = ui.color(T_CTRL_STROKE);
        (s, s)
    } else if editing {
        (accent, accent)
    } else if ev & EV_HOVER != 0 {
        let s = color_mix(ui.color(T_CTRL_STROKE), accent, 110);
        (s, s)
    } else {
        (ui.color(T_CTRL_STROKE), ui.color(T_CTRL_STROKE2))
    };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke2);
    if editing {
        // the WinUI focus line along the bottom: the frame's own shape, clipped to its
        // lowest 2 DIP, so the line follows the rounded corners
        let t = ui.dp(2);
        ui.draw_clip(r[0], r[1] + r[3] - t, r[2], t);
        ui.draw_color(accent);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
        ui.draw_unclip();
    }
}
