// misc -- BackupCraft colour wells, colour wheel, box decorations, window background
//
//   box_deco      BOX_CARD: card fill with a hairline, 8 DIP corners; BOX_LAYER: panel fill
//                 with its stroke; BOX_BG: the window background
//   window_bg     the window background colour

use ui::consts::*;
use ui::{cvtd, deg_sincos, Sp, StyleMisc, Ui};

use crate::common::{stroke_rrect, R_CARD};
use crate::BackupCraft;

impl StyleMisc for BackupCraft {
    fn swatch_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        let s = ui.dp(24);
        (s, s)
    }
    fn swatch(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = sp.value as u32;
        let d = ui.dp(3);
        let well = [r[0] + d, r[1] + d, r[2] - d * 2, r[3] - d * 2];
        let fill = if c & 0xFF00_0000 != 0 { c } else { 0 };
        let stroke = ui.color(T_CTRL_STROKE2);
        let t = ui.hair();
        let rad = ui.dp(4);
        ui.cmd_rrect(well[0], well[1], well[2], well[3], rad, t, fill, stroke, stroke);
        let (col, t) = if sp.value2 != 0 {
            (ui.color(T_ACCENT), ui.dp(2))
        } else if sp.ev & EV_HOVER != 0 {
            (ui.color(T_TEXT3), ui.hair())
        } else {
            return;
        };
        let rad = ui.dp(6);
        stroke_rrect(ui, r[0], r[1], r[2], r[3], rad, t, col);
    }
    fn color_preview(&self, ui: &mut Ui, argb: u32, r: [i32; 4]) {
        let stroke = ui.color(T_CTRL_STROKE2);
        let t = ui.hair();
        let rad = ui.dp(5);
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, argb, stroke, stroke);
    }
    fn color_marker(&self, ui: &mut Ui, x: i32, y: i32) {
        ui.draw_color(0xFFFF_FFFF);
        let r = ui.dp_fx(7 * 256);
        let w = ui.dp_fx(2 * 256);
        ui.draw_circle(x, y, r, w);
        ui.draw_color(0xFF00_0000);
        let r = ui.dp_fx(9 * 256);
        ui.draw_circle(x, y, r, 256);
    }
    fn color_wheel(&self, ui: &mut Ui, sp: &Sp) {
        let (x, y, side, rad) = (sp.x, sp.y, sp.w, sp.s[0]);
        let cx = (x << 8) + (side << 7);
        let cy = (y << 8) + (side << 7);
        let c = ui.color(T_CTRL_STROKE2);
        ui.draw_color(c);
        let t = ui.hair() << 8;
        ui.draw_circle(cx, cy, (rad << 8) + 128 - (t >> 1), t);
        let (cs, sn) = deg_sincos(sp.value as f64 * 0.1);
        let d = sp.value2 as f64 * 0.001 * rad as f64 * 256.0;
        let dx = cvtd(cs * d);
        let dy = cvtd(sn * d);
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_circle(cx + dx, cy - dy, ui.dp_fx(6 * 256), ui.dp_fx(2 * 256));
        ui.draw_color(0xFF00_0000);
        ui.draw_circle(cx + dx, cy - dy, ui.dp_fx(8 * 256), 256);
        if sp.s[1] != 0 {
            let c = ui.color(T_FOCUS_OUT);
            ui.draw_color(c);
            let d = ui.dp(3);
            let s = side + d * 2;
            ui.draw_frame(x - d, y - d, s, s, s >> 1, ui.dp(2));
        }
    }
    fn box_deco(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let flags = sp.flags;
        if flags & BOX_BG != 0 {
            let c = ui.color(T_BG);
            ui.draw_color(c);
            ui.draw_rect(r[0], r[1], r[2], r[3]);
        }
        if flags & (BOX_CARD | BOX_LAYER) != 0 {
            let tok = if flags & BOX_LAYER != 0 { T_LAYER } else { T_CARD };
            let fill = ui.color(tok);
            let stroke = ui.color(tok + 4);
            let rad = ui.dp(R_CARD);
            let t = ui.hair();
            ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke);
        }
    }
    fn window_bg(&self, ui: &mut Ui, w: i32, h: i32) {
        let c = ui.color(T_BG);
        ui.draw_color(c);
        ui.draw_rect(0, 0, w, h);
    }
}
