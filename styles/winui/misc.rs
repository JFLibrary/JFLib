// misc -- the winui swatches, colour picker parts, box decorations, window background  [styles/winui/misc.s]

use ui::consts::*;
use ui::{cvtd, deg_sincos, Sp, StyleMisc, Ui};

use crate::WinUi;

impl StyleMisc for WinUi {
    fn window_bg(&self, ui: &mut Ui, w: i32, h: i32) {
        window_bg(ui, w, h)
    }
    fn box_deco(&self, ui: &mut Ui, sp: &Sp) {
        box_deco(ui, sp.rect(), sp.flags)
    }
    // ui-list: the colour parts (swatch, colour picker, colour wheel)
    fn swatch_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        let s = ui.dp(22);
        (s, s)
    }
    fn swatch(&self, ui: &mut Ui, sp: &Sp) {
        swatch(ui, sp)
    }
    fn color_preview(&self, ui: &mut Ui, argb: u32, r: [i32; 4]) {
        // the colour picker's preview well (4 DIP corners)
        ui.draw_color(argb);
        let rad = ui.dp(4);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
    }
    fn color_marker(&self, ui: &mut Ui, x_fx: i32, y_fx: i32) {
        color_marker(ui, x_fx, y_fx)
    }
    fn color_wheel(&self, ui: &mut Ui, sp: &Sp) {
        color_wheel(ui, sp)
    }
}


/// sty_window_bg(w, h): the background of the whole surface
fn window_bg(ui: &mut Ui, w: i32, h: i32) {
    let c = ui.color(T_BG);
    ui.draw_color(c);
    ui.draw_rect(0, 0, w, h);
}

/// sty_box: BOX_BG window background, BOX_CARD / BOX_LAYER fill + stroke (8 DIP corners)
fn box_deco(ui: &mut Ui, r: [i32; 4], flags: u32) {
    if flags & BOX_BG != 0 {
        let c = ui.color(T_BG);
        ui.draw_color(c);
        ui.draw_rect(r[0], r[1], r[2], r[3]);
    }
    if flags & (BOX_CARD | BOX_LAYER) != 0 {
        let tok = if flags & BOX_LAYER != 0 { T_LAYER } else { T_CARD };
        let fill = ui.color(tok);
        let stroke = ui.color(tok + 4);
        let rad = ui.dp(8);
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, 1, fill, stroke, stroke);
    }
}

// ------------------------------------------------------------------ colour parts (ui-list)

/// sty_swatch(sp): round colour well (hairline border; transparent: an empty ring), ring in
/// the text colour when selected (value2), tertiary ring on hover.  value colour, ev
fn swatch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = sp.value as u32;
    // geometry (fx): radius = half the smaller side minus 3 DIP for the rings
    let rad = (r[2].min(r[3]) << 7) - ui.dp_fx(3 * 256);
    let cx = (r[0] << 8) + (r[2] << 7);
    let cy = (r[1] << 8) + (r[3] << 7);
    let ring = ui.dp_fx(2 * 256);
    if c & 0xFF00_0000 != 0 {
        // filled circle + hairline border so light colours stay visible
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
        let b = ui.color(T_CTRL_STROKE2);
        ui.draw_color(b);
    } else {
        let b = ui.color(T_STRONG_STROKE);
        ui.draw_color((b & 0xFF_FFFF) | 0x6000_0000);
    }
    ui.draw_circle(cx, cy, rad - 128, 256);
    // selected: ring in the text colour around the well; hover: tertiary colour
    let tok = if sp.value2 != 0 {
        T_TEXT
    } else if sp.ev & EV_HOVER != 0 {
        T_TEXT3
    } else {
        return;
    };
    let c = ui.color(tok);
    ui.draw_color(c);
    ui.draw_circle(cx, cy, rad + ring, ring); // radius + gap + half the ring
}

/// sty_color_marker(x, y): ring marker on a colour field at (x, y) 24.8 px: white ring,
/// black outline
fn color_marker(ui: &mut Ui, x: i32, y: i32) {
    ui.draw_color(0xFFFF_FFFF);
    let r = ui.dp_fx(7 * 256);
    let w = ui.dp_fx(2 * 256);
    ui.draw_circle(x, y, r, w);
    ui.draw_color(0xFF00_0000);
    let r = ui.dp_fx(9 * 256);
    ui.draw_circle(x, y, r, 256);
}

/// sty_color_wheel(sp): hairline rim, ring marker at (hue value tenths of a degree,
/// saturation value2 0..1000), round focus visual (s[1] = 1); rect = the wheel's square, s[0]
/// its radius (px)
fn color_wheel(ui: &mut Ui, sp: &Sp) {
    let (x, y, side, rad) = (sp.x, sp.y, sp.w, sp.s[0]);
    let cx = (x << 8) + (side << 7);
    let cy = (y << 8) + (side << 7);
    // hairline rim
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let t = ui.hair() << 8;
    ui.draw_circle(cx, cy, (rad << 8) + 128 - ((t as u32 >> 1) as i32), t);
    // marker: white ring, black outline, at (hue, saturation)
    let (cs, sn) = deg_sincos(sp.value as f64 * 0.1);
    let d = sp.value2 as f64 * 0.001 * rad as f64 * 256.0; // distance (fx)
    let dx = cvtd(cs * d);
    let dy = cvtd(sn * d); // up
    ui.draw_color(0xFFFF_FFFF);
    let r = ui.dp_fx(6 * 256);
    let w = ui.dp_fx(2 * 256);
    ui.draw_circle(cx + dx, cy - dy, r, w);
    ui.draw_color(0xFF00_0000);
    let r = ui.dp_fx(8 * 256);
    ui.draw_circle(cx + dx, cy - dy, r, 256);
    // focus ring around the wheel
    if sp.s[1] != 0 {
        let c = ui.color(T_FOCUS_OUT);
        ui.draw_color(c);
        let d = ui.dp(3);
        let s = side + d * 2;
        ui.draw_frame(x - d, y - d, s, s, ((s as u32) >> 1) as i32, 2);
    }
}
