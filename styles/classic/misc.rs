// misc -- the classic colour wells, colour picker parts, colour wheel rim, box decorations,
// window background                                                [styles/classic/misc.s]
//
//   swatch_size(sp), swatch(sp)         20 DIP square well with a sunken frame
//   color_preview(argb, r), color_marker(x, y), color_wheel(sp)
//   box_deco(sp)                        BOX_BG / BOX_LAYER: the face; BOX_CARD: etched frame
//   window_bg(w, h)                     the grey face over the whole surface
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::{cvtd, deg_sincos, Sp, StyleMisc, Ui};

use crate::common::*;
use crate::Classic;

impl StyleMisc for Classic {
    fn swatch_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        // 20 x 20 DIP
        let s = ui.dp(20);
        (s, s)
    }
    fn swatch(&self, ui: &mut Ui, sp: &Sp) {
        swatch(ui, sp)
    }
    fn color_preview(&self, ui: &mut Ui, argb: u32, r: [i32; 4]) {
        // square well with a sunken frame
        fill(ui, r, argb);
        frame(ui, r, FR_SUNKEN);
    }
    fn color_marker(&self, ui: &mut Ui, x_fx: i32, y_fx: i32) {
        color_marker(ui, x_fx, y_fx)
    }
    fn color_wheel(&self, ui: &mut Ui, sp: &Sp) {
        color_wheel(ui, sp)
    }
    fn box_deco(&self, ui: &mut Ui, sp: &Sp) {
        // BOX_BG: the window face; BOX_CARD: an etched group frame; BOX_LAYER: the face
        if sp.flags & (BOX_BG | BOX_LAYER) != 0 {
            let c = ui.color(T_BG);
            fill(ui, sp.rect(), c);
        }
        if sp.flags & BOX_CARD != 0 {
            frame(ui, sp.rect(), FR_ETCHED);
        }
    }
    fn window_bg(&self, ui: &mut Ui, w: i32, h: i32) {
        // the grey face over the whole surface
        let c = ui.color(T_BG);
        ui.draw_color(c);
        ui.draw_rect(0, 0, w, h);
    }
}

/// sty_swatch(sp): square colour well with a sunken frame; selected: a black frame around
/// it, hover: a thin raised one.  value colour, value2 selected, ev
fn swatch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let d = ui.dp(2);
    let well = inset(r, d);
    let c = sp.value as u32;
    if c & 0xFF00_0000 != 0 {
        fill(ui, well, c);
    }
    frame(ui, well, FR_THIN_SUNKEN);
    if sp.value2 != 0 {
        frame(ui, r, FR_BLACK);
    } else if sp.ev & EV_HOVER != 0 {
        frame(ui, r, FR_THIN_RAISED);
    }
}

/// sty_color_marker(x, y): 9 x 9 DIP square marker (white with a black outline) at (x, y) fx
fn color_marker(ui: &mut Ui, x: i32, y: i32) {
    let (px, py) = (x >> 8, y >> 8);
    let d = ui.dp(4);
    let s = d + d + 1;
    let m = [px.wrapping_sub(d), py.wrapping_sub(d), s, s];
    frame(ui, m, FR_BLACK);
    let l = ui.hair();
    let m = inset(m, l);
    bevel(ui, m, 0xFFFF_FFFF, 0xFFFF_FFFF);
}

/// sty_color_wheel(sp): black rim and a square marker at (hue value tenths of a degree,
/// saturation value2 0..1000); dotted focus rectangle (s[1]); rect = the wheel's square, s[0]
/// its radius (px)
fn color_wheel(ui: &mut Ui, sp: &Sp) {
    let c = pal(ui, CP_DK);
    ui.draw_color(c);
    let rad = sp.s[0]; // radius (px)
    let half = sp.w << 7;
    let mut cx = (sp.x << 8).wrapping_add(half);
    let mut cy = (sp.y << 8).wrapping_add(half);
    let t = ui.hair() << 8;
    let r = (rad << 8).wrapping_add(128).wrapping_sub(((t as u32) >> 1) as i32);
    ui.draw_circle(cx, cy, r, t);
    let (cs, sn) = deg_sincos(sp.value as f64 * 0.1);
    let d = sp.value2 as f64 * 0.001 * rad as f64 * 256.0; // distance (fx)
    cx = cx.wrapping_add(cvtd(cs * d));
    cy = cy.wrapping_sub(cvtd(sn * d));
    color_marker(ui, cx, cy);
    if sp.s[1] != 0 {
        dots(ui, sp.rect());
    }
}
