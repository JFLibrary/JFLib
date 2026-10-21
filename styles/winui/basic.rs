// basic -- the winui buttons, check boxes, radios, switches, sliders, progress, separators
//                                                                 [styles/winui/basic.s]
//
//   focus_ring(r, radius_dip)          2 px ring 3 DIP outside the control
//   button_size / button               standard 32 (compact 24) high, padding 11, icon 16 +
//                                      gap 8; ST_LARGE: 24 icon over a caption, 60 high;
//                                      fill + 1 px border (bottom edge darker), icon, label
//   checkbox_size / checkbox, radio_size / radio, switch_size / switch,
//   slider_size / slider, progress_size / progress, separator_size / separator
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::IC_CHECK_MARK;
use ui::{Sp, StyleBasic, Ui};

use crate::WinUi;

impl StyleBasic for WinUi {
    fn focus_ring(&self, ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
        focus_ring(ui, r, radius_dip)
    }
    fn button_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        button_size(ui, sp)
    }
    fn button(&self, ui: &mut Ui, sp: &Sp) {
        button(ui, sp)
    }
    fn checkbox_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        check_size(ui, sp, 20, 8)
    }
    fn checkbox(&self, ui: &mut Ui, sp: &Sp) {
        checkbox(ui, sp)
    }
    fn radio_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        check_size(ui, sp, 20, 8)
    }
    fn radio(&self, ui: &mut Ui, sp: &Sp) {
        radio(ui, sp)
    }
    fn switch_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        switch_size(ui, sp)
    }
    fn switch(&self, ui: &mut Ui, sp: &Sp) {
        switch(ui, sp)
    }
    fn slider_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        slider_size(ui, sp)
    }
    fn slider(&self, ui: &mut Ui, sp: &Sp) {
        slider(ui, sp)
    }
    fn progress_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (ui.dp(200), ui.dp(12))
    }
    fn progress(&self, ui: &mut Ui, sp: &Sp) {
        progress(ui, sp)
    }
    fn separator_size(&self, ui: &mut Ui, _sp: &mut Sp) -> i32 {
        // 4 + hairline + 4
        let h = ui.hair();
        ui.dp(8) + h
    }
    fn separator(&self, ui: &mut Ui, sp: &Sp) {
        separator(ui, sp)
    }
}

/// sty_focus_ring(r, radius_dip): Fluent focus visual: 2 px ring 3 DIP outside the control,
/// corner radius = the control's radius + 3
fn focus_ring(ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
    let off = ui.dp(3);
    let rad = ui.dp(radius_dip) + off;
    let c = ui.color(T_FOCUS_OUT);
    ui.cmd_rrect(r[0] - off, r[1] - off, r[2] + off * 2, r[3] + off * 2, rad, 2, 0, c, c);
}

/// sty_button_size -> (w, h) (keeps the text width in s[0], the icon px in s[1])
fn button_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let (flags, icon) = (sp.flags, sp.icon);
    ui.ts_font(if flags & ST_LARGE != 0 { TS_CAPTION } else { TS_BODY });
    let tw = match sp.label {
        Some(l) if flags & ST_ICON_ONLY == 0 => ui.text.width(l),
        _ => 0,
    };
    let (w, h, ip);
    if flags & ST_LARGE == 0 {
        // normal: padx 11, icon 16, gap 8, height 32 (compact 24)
        ip = ui.dp(16);
        let mut ww = ui.dp(11) * 2 + tw;
        if icon != 0 {
            ww += ip;
            if tw == 0 {
                ww = ui.dp(if flags & ST_COMPACT != 0 { 24 } else { 32 }); // square icon button
            } else {
                ww += ui.dp(8);
            }
        }
        w = ww;
        h = ui.dp(if flags & ST_COMPACT != 0 { 24 } else { 32 });
    } else {
        // large: 24 icon over a caption label, 8 padding
        ip = ui.dp(24);
        w = tw.max(ip) + ui.dp(16);
        h = ui.dp(60);
    }
    sp.s[0] = tw;
    sp.s[1] = ip;
    (w, h)
}

/// sty_button: fill, border, icon and label in the colours of the state
fn button(ui: &mut Ui, sp: &Sp) {
    let (r, state, flags, icon) = (sp.rect(), sp.state, sp.flags, sp.icon);
    let label = sp.label.unwrap_or("");
    let (tw, ip) = (sp.s[0], sp.s[1]);
    let st = state * 4;
    // colours: fill, stroke, bottom stroke, text
    let (fill, mut stroke, mut stroke2, text);
    let accent = flags & ST_ACCENT != 0 || (flags & ST_SUBTLE == 0 && flags & ST_CHECKED != 0);
    if accent {
        fill = ui.color(T_ACCENT + st);
        stroke = 0x14FF_FFFF;
        stroke2 = if ui.theme_dark() { 0x2400_0000 } else { 0x6600_0000 };
        if state >= 2 {
            stroke = 0;
            stroke2 = 0;
        }
        text = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
    } else if flags & ST_SUBTLE != 0 {
        if flags & ST_CHECKED != 0 {
            // checked tool: control fill with a border
            fill = ui.color(if state == 0 { T_CTRL_HOVER } else { T_CTRL + st });
            stroke = ui.color(T_CTRL_STROKE);
            stroke2 = stroke;
        } else {
            fill = ui.color(T_SUBTLE + st);
            stroke = 0;
            stroke2 = 0;
        }
        text = ui.color(match state {
            3 => T_TEXT_DISABLED,
            2 => T_TEXT2,
            _ => T_TEXT,
        });
    } else {
        // standard
        fill = ui.color(T_CTRL + st);
        stroke = ui.color(T_CTRL_STROKE);
        stroke2 = stroke;
        if state < 2 {
            stroke2 = ui.color(T_CTRL_STROKE2);
        }
        text = ui.color(match state {
            3 => T_TEXT_DISABLED,
            2 => T_TEXT2,
            _ => T_TEXT,
        });
    }
    let rad = ui.dp(4);
    let t = if stroke != 0 { ui.hair() } else { 0 };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    // content
    ui.draw_color(text);
    if flags & ST_LARGE != 0 {
        // icon centred at the top, caption centred below
        if icon != 0 {
            let x = ((r[2] - ip) >> 1) + r[0];
            let y = r[1] + ui.dp(8);
            ui.draw_icon(icon, x, y, ip << 8);
        }
        if tw != 0 {
            let base = ui.ts_base(TS_CAPTION);
            ui.ts_font(TS_CAPTION);
            let x = ((r[2] - tw) >> 1) + r[0];
            let y = ui.dp(36) + base + r[1];
            ui.emit_text(x, y, label, text);
        }
        return;
    }
    // width of icon + gap + text, centred
    let mut cw = tw;
    if icon != 0 {
        cw += ip;
        if tw != 0 {
            cw += ui.dp(8);
        }
    }
    let mut x = r[0] + ((r[2] - cw) >> 1);
    if icon != 0 {
        let y = r[1] + ((r[3] - ip) >> 1);
        ui.draw_icon(icon, x, y, ip << 8);
        x += ip + ui.dp(8);
    }
    if tw != 0 {
        let base = ui.ts_base(TS_BODY);
        let line = ui.ts_line(TS_BODY);
        let y = ((r[3] - line) >> 1) + r[1] + base;
        ui.emit_text(x, y, label, text);
    }
}

/// sty_checkbox_size / sty_radio_size -> (w, h): box / circle `box_dip`, gap, 32 high;
/// s[0] = box px
fn check_size(ui: &mut Ui, sp: &mut Sp, box_dip: i32, gap: i32) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) => ui.text.width(l),
        None => 0,
    };
    let b = ui.dp(box_dip);
    let mut w = b;
    if tw != 0 {
        w += ui.dp(gap) + tw;
    }
    let h = ui.dp(32);
    sp.s[0] = b;
    (w, h)
}

// the label of a check box / radio / switch: x after the control and its gap, body text
fn label_after(ui: &mut Ui, sp: &Sp, x_after: i32, gap: i32) {
    let Some(l) = sp.label else { return };
    let r = sp.rect();
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let base = ui.ts_base(TS_BODY);
    let line = ui.ts_line(TS_BODY);
    let y = ((r[3] - line) >> 1) + r[1] + base;
    let x = r[0] + ui.dp(gap) + x_after;
    ui.emit_text(x, y, l, c);
}

/// sty_checkbox: box (alt fill + strong border, or accent with a check mark / dash), label
fn checkbox(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, b) = (sp.state, sp.value, sp.s[0]);
    let x = r[0];
    let y = ((r[3] - b) >> 1) + r[1];
    let (fill, t, stroke);
    if v == 0 {
        // unchecked: alt fill + strong border
        fill = ui.color(T_ALT_FILL);
        stroke = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_STRONG_STROKE });
        t = ui.hair();
    } else {
        fill = ui.color(T_ACCENT + state * 4);
        t = 0;
        stroke = 0;
    }
    let rad = ui.dp(4);
    ui.cmd_rrect(x, y, b, b, rad, t, fill, stroke, stroke);
    if v != 0 {
        // check mark (or dash for indeterminate) in the on-accent colour
        let c = ui.color(T_ON_ACCENT);
        ui.draw_color(c);
        if v == 2 {
            let dw = ui.dp(8);
            let h = ui.hair();
            ui.draw_rect(((b - dw) >> 1) + x, ((b - h) >> 1) + y, dw, h);
        } else {
            let g = ui.dp(14); // glyph size
            ui.draw_icon(IC_CHECK_MARK, ((b - g) >> 1) + x, ((b - g) >> 1) + y, g << 8);
        }
    }
    label_after(ui, sp, b, 8);
}

/// sty_radio: circle (ring, or accent with a dot growing on hover); value 1 = selected
fn radio(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, b) = (sp.state, sp.s[0]);
    // centre in 24.8
    let rad = b << 7; // radius fx = px * 256 / 2
    let cx = (r[0] << 8) + rad;
    let cy = (r[3] << 7) + (r[1] << 8);
    if sp.value != 1 {
        let c = ui.color(T_ALT_FILL);
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
        let c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_STRONG_STROKE });
        ui.draw_color(c);
        let t = ui.hair() << 8;
        ui.draw_circle(cx, cy, rad - ((t as u32) >> 1) as i32, t); // ring centred inside the circle
    } else {
        let c = ui.color(T_ACCENT + state * 4);
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
        let c = ui.color(T_ON_ACCENT);
        ui.draw_color(c);
        // dot radius: 4, hover 5, pressed 3
        let d = match state {
            1 => 5u32,
            2 => 3,
            _ => 4,
        };
        let dr = ((d << 8).wrapping_mul(ui.dpi() as u32) / 96) as i32;
        ui.draw_circle(cx, cy, dr, 0);
    }
    label_after(ui, sp, b, 8);
}

/// sty_switch_size -> (w, h): track 40 x 20, gap 12, 32 high; s[0] / s[1] = track w / h
fn switch_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) => ui.text.width(l),
        None => 0,
    };
    let (tkw, tkh) = (ui.dp(40), ui.dp(20));
    let mut w = tkw;
    if tw != 0 {
        w += ui.dp(12) + tw;
    }
    let h = ui.dp(32);
    sp.s[0] = tkw;
    sp.s[1] = tkh;
    (w, h)
}

/// sty_switch: pill track, knob sliding with value2 (0 off .. 256 on), label
fn switch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, tkw, tkh, pos) = (sp.state, sp.value, sp.s[0], sp.s[1], sp.value2);
    // track
    let x = r[0];
    let y = ((r[3] - tkh) >> 1) + r[1];
    let rad = tkh >> 1; // pill radius
    let (fill, t, stroke);
    if v == 0 {
        fill = ui.color(T_ALT_FILL);
        stroke = ui.color(T_STRONG_STROKE);
        t = ui.hair();
    } else {
        fill = ui.color(T_ACCENT + state * 4);
        t = 0;
        stroke = 0;
    }
    ui.cmd_rrect(x, y, tkw, tkh, rad, t, fill, stroke, stroke);
    // knob: radius 6 DIP (7 hover), travels from 10 to 30 DIP
    let c = ui.color(if v == 0 { T_STRONG_FILL } else { T_ON_ACCENT });
    ui.draw_color(c);
    let off = ui.dp_fx(10 * 256) + ((ui.dp_fx(20 * 256).wrapping_mul(pos)) >> 8);
    let cx = (x << 8) + off;
    let cy = (y << 8) + (tkh << 7);
    let kr = ui.dp_fx(if state == 1 { 7 * 256 } else { 6 * 256 });
    ui.draw_circle(cx, cy, kr, 0);
    label_after(ui, sp, tkw, 12);
}

/// sty_slider_size -> (w, h): 200 x 32, swapped when value = vertical; s[0] = thumb px
/// along the axis, s[1] = vertical
fn slider_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let vert = sp.value;
    sp.s[1] = vert;
    let thumb = ui.dp(20);
    let (a, b) = (ui.dp(200), ui.dp(32));
    sp.s[0] = thumb;
    if vert != 0 { (b, a) } else { (a, b) }
}

/// sty_slider: 4 px track (strong fill, accent up to the value), knob with an accent dot
/// (value thumb offset, value2 track length)
fn slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, vert, len, t, thumb) = (sp.state, sp.s[1], sp.value2, sp.value, sp.s[0]);
    let th = ui.dp(4); // track thickness
    let half = thumb >> 1; // half thumb
    let (cx, cy);
    if vert == 0 {
        // horizontal: full track (strong fill), then the accent part
        let c = ui.color(T_STRONG_FILL);
        ui.draw_color(c);
        let ty = ((r[3] - th) >> 1) + r[1];
        ui.draw_rrect(r[0] + half, ty, len, th, th >> 1);
        let c = ui.color(T_ACCENT + state * 4);
        ui.draw_color(c);
        ui.draw_rrect(r[0] + half, ty, t, th, th >> 1);
        // thumb centre
        cx = (r[0] + half + t) << 8;
        cy = (r[3] << 7) + (r[1] << 8);
    } else {
        let c = ui.color(T_STRONG_FILL);
        ui.draw_color(c);
        let tx = ((r[2] - th) >> 1) + r[0];
        ui.draw_rrect(tx, r[1] + half, th, len, th >> 1);
        let c = ui.color(T_ACCENT + state * 4);
        ui.draw_color(c);
        // accent from the value point down to the bottom
        let ay = r[1] + r[3] - half - t;
        ui.draw_rrect(tx, ay, th, t, th >> 1);
        cx = (r[2] << 7) + (r[0] << 8);
        cy = (r[1] + r[3] - half - t) << 8;
    }
    // outer knob with a hairline border, inner accent dot (12 / hover 14 / pressed 10 DIP)
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let k = ui.dp_fx(10 * 256);
    ui.draw_circle(cx, cy, k, 0);
    let c = ui.color(T_KNOB);
    ui.draw_color(c);
    let k = ui.dp_fx(10 * 256 - 256);
    ui.draw_circle(cx, cy, k, 0);
    let c = ui.color(T_ACCENT + state * 4);
    ui.draw_color(c);
    let d = match state {
        1 => 7 * 256,
        2 => 5 * 256,
        _ => 6 * 256,
    };
    let k = ui.dp_fx(d);
    ui.draw_circle(cx, cy, k, 0);
}

/// sty_progress: hairline track, 3 DIP accent pill up to value / value2
fn progress(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (v, max) = (sp.value, sp.value2);
    // track: hairline in the strong stroke colour
    let c = ui.color(T_STRONG_STROKE);
    ui.draw_color(c);
    let h = ui.hair();
    ui.draw_rect(r[0], ((r[3] - h) >> 1) + r[1], r[2], h);
    // value bar: 3 DIP accent pill
    if max <= 0 || v <= 0 {
        return;
    }
    let w = v.min(max).wrapping_mul(r[2]) / max;
    let c = ui.color(T_ACCENT);
    ui.draw_color(c);
    let ph = ui.dp(3);
    ui.draw_rrect(r[0], ((r[3] - ph) >> 1) + r[1], w, ph, ph >> 1);
}

/// sty_separator: hairline in the divider colour, centred across the rectangle (value 1 =
/// horizontal line, 0 = vertical)
fn separator(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    if sp.value != 0 {
        ui.draw_rect(r[0], ((r[3] - h) >> 1) + r[1], r[2], h);
    } else {
        ui.draw_rect(((r[2] - h) >> 1) + r[0], r[1], h, r[3]);
    }
}
