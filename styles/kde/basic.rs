// basic -- the kde buttons, check boxes, radios, switches, sliders, progress, separators
//                                                                   [styles/kde/basic.s]
//
//   focus_ring(r, radius_dip)          Breeze focus: the control's outline in the highlight
//                                      colour
//   button_size / button               push button 30 (compact 24) high, padding 10, icon 16 +
//                                      gap 6, labels at least 72 wide; tool buttons 30 square;
//                                      ST_LARGE: 24 icon over a caption, 54 high; light button
//                                      with a grey outline (3 DIP corners); hover / press:
//                                      highlight outline; default and checked buttons tinted
//                                      with the highlight
//   checkbox_size / checkbox, radio_size / radio, switch_size / switch,
//   slider_size / slider, progress_size / progress, separator_size / separator
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::IC_CHECK_MARK;
use ui::theme::color_mix;
use ui::{Sp, StyleBasic, Ui};

use crate::Kde;

impl StyleBasic for Kde {
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
        // box 16, gap 6, 24 high
        check_size(ui, sp)
    }
    fn checkbox(&self, ui: &mut Ui, sp: &Sp) {
        checkbox(ui, sp)
    }
    fn radio_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // circle 16, gap 6, 24 high
        check_size(ui, sp)
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
        // 200 x 16
        (ui.dp(200), ui.dp(16))
    }
    fn progress(&self, ui: &mut Ui, sp: &Sp) {
        progress(ui, sp)
    }
    fn separator_size(&self, ui: &mut Ui, _sp: &mut Sp) -> i32 {
        // 4 + hairline + 4
        let h = ui.hair();
        ui.dp(8).wrapping_add(h)
    }
    fn separator(&self, ui: &mut Ui, sp: &Sp) {
        separator(ui, sp)
    }
}

/// sty_focus_ring(r, radius_dip): Breeze focus visual: a 1 DIP outline in the highlight
/// colour on the control's edge (same corner radius)
pub(crate) fn focus_ring(ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
    let rad = ui.dp(radius_dip);
    let w = ui.dp(1);
    let c = ui.color(T_ACCENT);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, w, 0, c, c);
}

// .sbz_height: 30 / 24 DIP (ST_COMPACT)
fn btn_height(ui: &Ui, flags: u32) -> i32 {
    ui.dp(if flags & ST_COMPACT != 0 { 30 - 6 } else { 30 })
}

/// sty_button_size -> (w, h) (keeps the text width in s[0], the icon px in s[1])
fn button_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let (flags, icon) = (sp.flags, sp.icon);
    // measure
    ui.ts_font(if flags & ST_LARGE != 0 { TS_CAPTION } else { TS_BODY });
    let tw = match sp.label {
        Some(l) if flags & ST_ICON_ONLY == 0 => ui.text.width(l),
        _ => 0,
    };
    let (w, h, ip);
    if flags & ST_LARGE == 0 {
        // normal: padx 10, icon 16, gap 6, height 30 (compact 24); tool buttons square
        ip = ui.dp(16);
        let mut ww = ui.dp(10).wrapping_mul(2).wrapping_add(tw); // 2 * padx
        if icon != 0 {
            ww = ww.wrapping_add(ip);
            if tw == 0 {
                ww = btn_height(ui, flags); // square icon button
            } else {
                ww = ww.wrapping_add(ui.dp(6));
            }
        } else if flags & ST_SUBTLE == 0 && tw != 0 {
            // text-only push buttons are at least 72 DIP wide (dialog buttons)
            let min = ui.dp(72);
            if ww < min {
                ww = min;
            }
        }
        w = ww;
        h = btn_height(ui, flags);
    } else {
        // large: 24 icon over a caption label, 8 padding
        ip = ui.dp(24);
        let ww = if tw < ip { ip } else { tw };
        w = ww.wrapping_add(ui.dp(16));
        h = ui.dp(54);
    }
    sp.s[0] = tw;
    sp.s[1] = ip;
    (w, h)
}

/// sty_button: fill, border, icon and label in the colours of the state
fn button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, flags, icon) = (sp.state, sp.flags, sp.icon);
    let label = sp.label.unwrap_or("");
    let (tw, ip) = (sp.s[0], sp.s[1]);
    let st = state.wrapping_mul(4);
    // colours: fill, stroke, bottom stroke, text
    let (fill, stroke, stroke2);
    let accent = flags & ST_ACCENT != 0 || (flags & ST_SUBTLE == 0 && flags & ST_CHECKED != 0);
    if accent {
        // default / checked button: fill tinted with the highlight, highlight outline
        let f = ui.color(T_CTRL);
        let a = ui.color(T_ACCENT);
        fill = color_mix(f, a, if state == 2 { 112 } else { 56 });
        if state == 3 {
            stroke = ui.color(T_CTRL_STROKE);
        } else {
            stroke = a;
        }
        stroke2 = stroke;
    } else if flags & ST_SUBTLE != 0 {
        if flags & ST_CHECKED != 0 {
            // checked tool: highlight tint with a highlight outline
            fill = ui.color(T_SUBTLE_HOVER);
            stroke = ui.color(T_ACCENT);
        } else {
            fill = ui.color(T_SUBTLE.wrapping_add(st));
            // hover / press: highlight outline
            stroke = if state == 1 || state == 2 { ui.color(T_ACCENT) } else { 0 };
        }
        stroke2 = stroke;
    } else {
        // standard: light fill, grey outline (darker at the bottom); highlight outline while
        // hovered / pressed
        fill = ui.color(T_CTRL.wrapping_add(st));
        let s = ui.color(T_CTRL_STROKE);
        if state == 3 {
            stroke = s;
            stroke2 = s;
        } else if state != 0 {
            stroke = ui.color(T_ACCENT);
            stroke2 = stroke;
        } else {
            stroke = s;
            stroke2 = ui.color(T_CTRL_STROKE2);
        }
    }
    let text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let rad = ui.dp(3);
    let t = if stroke != 0 { ui.hair() } else { 0 };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    // content
    ui.draw_color(text);
    if flags & ST_LARGE != 0 {
        // icon centred at the top, caption centred below
        if icon != 0 {
            let x = (r[2].wrapping_sub(ip) >> 1).wrapping_add(r[0]);
            let y = r[1].wrapping_add(ui.dp(8));
            ui.draw_icon(icon, x, y, ip << 8);
        }
        if tw != 0 {
            let base = ui.ts_base(TS_CAPTION);
            ui.ts_font(TS_CAPTION);
            let x = (r[2].wrapping_sub(tw) >> 1).wrapping_add(r[0]);
            let y = ui.dp(34).wrapping_add(base).wrapping_add(r[1]);
            ui.emit_text(x, y, label, text);
        }
        return;
    }
    // width of icon + gap + text, centred
    let mut cw = tw;
    if icon != 0 {
        cw = cw.wrapping_add(ip);
        if tw != 0 {
            cw = cw.wrapping_add(ui.dp(6));
        }
    }
    let mut x = r[0].wrapping_add(r[2].wrapping_sub(cw) >> 1); // content x
    if icon != 0 {
        let y = r[1].wrapping_add(r[3].wrapping_sub(ip) >> 1);
        ui.draw_icon(icon, x, y, ip << 8);
        x = x.wrapping_add(ip).wrapping_add(ui.dp(6));
    }
    if tw != 0 {
        let base = ui.ts_base(TS_BODY);
        let line = ui.ts_line(TS_BODY);
        let y = (r[3].wrapping_sub(line) >> 1).wrapping_add(r[1]).wrapping_add(base);
        ui.emit_text(x, y, label, text);
    }
}

/// sty_checkbox_size / sty_radio_size -> (w, h): box / circle 16, gap 6, 24 high; s[0] =
/// box px
fn check_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) => ui.text.width(l),
        None => 0,
    };
    let b = ui.dp(16);
    let mut w = b;
    if tw != 0 {
        w = w.wrapping_add(ui.dp(6)).wrapping_add(tw);
    }
    let h = ui.dp(24);
    sp.s[0] = b;
    (w, h)
}

// the outline token of a check box / radio: strong stroke, highlight while hovered / pressed,
// disabled text colour
fn mark_stroke(state: u32) -> u32 {
    match state {
        1 | 2 => T_ACCENT,
        3 => T_TEXT_DISABLED,
        _ => T_STRONG_STROKE,
    }
}

// the label of a check box / radio / switch: x after the control and its gap, body text
fn label_after(ui: &mut Ui, sp: &Sp, x_after: i32, gap: i32) {
    let Some(l) = sp.label else { return };
    let r = sp.rect();
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let base = ui.ts_base(TS_BODY);
    let line = ui.ts_line(TS_BODY);
    let y = (r[3].wrapping_sub(line) >> 1).wrapping_add(r[1]).wrapping_add(base);
    let x = r[0].wrapping_add(ui.dp(gap)).wrapping_add(x_after);
    ui.emit_text(x, y, l, c);
}

/// sty_checkbox: white box with a grey outline (2 DIP corners; highlight outline on hover),
/// highlight-coloured check mark / dash, label
fn checkbox(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, b) = (sp.state, sp.value, sp.s[0]);
    // box rectangle
    let x = r[0];
    let y = (r[3].wrapping_sub(b) >> 1).wrapping_add(r[1]);
    // box: alt fill, strong outline (highlight while hovered / pressed)
    let fill = ui.color(T_ALT_FILL);
    let stroke = ui.color(mark_stroke(state));
    let t = ui.hair();
    let rad = ui.dp(2);
    ui.cmd_rrect(x, y, b, b, rad, t, fill, stroke, stroke);
    if v != 0 {
        // check mark (or dash for indeterminate) in the highlight colour
        let c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_ACCENT });
        ui.draw_color(c);
        if v == 2 {
            let d = ui.dp(8); // a square
            let o = b.wrapping_sub(d) >> 1;
            ui.draw_rect(o.wrapping_add(x), o.wrapping_add(y), d, d);
        } else {
            let g = ui.dp(12); // glyph size
            let o = b.wrapping_sub(g) >> 1;
            ui.draw_icon(IC_CHECK_MARK, o.wrapping_add(x), o.wrapping_add(y), g << 8);
        }
    }
    label_after(ui, sp, b, 6);
}

/// sty_radio: white circle with a grey outline (highlight on hover), a highlight dot when
/// selected (value 1)
fn radio(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, b) = (sp.state, sp.s[0]);
    // centre in 24.8
    let rad = b << 7; // radius fx = px * 256 / 2
    let cx = (r[0] << 8).wrapping_add(rad);
    let cy = (r[3] << 7).wrapping_add(r[1] << 8);
    let c = ui.color(T_ALT_FILL);
    ui.draw_color(c);
    ui.draw_circle(cx, cy, rad, 0);
    let c = ui.color(mark_stroke(state));
    ui.draw_color(c);
    let t = ui.hair() << 8;
    ui.draw_circle(cx, cy, rad.wrapping_sub(((t as u32) >> 1) as i32), t); // ring centred inside the circle
    if sp.value == 1 {
        let c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_ACCENT });
        ui.draw_color(c);
        // dot radius 4 DIP
        let dr = ((4u32 << 8).wrapping_mul(ui.dpi() as u32) / 96) as i32;
        ui.draw_circle(cx, cy, dr, 0);
    }
    label_after(ui, sp, b, 6);
}

/// sty_switch_size -> (w, h): track 36 x 18, gap 8, 24 high; s[0] / s[1] = track w / h
fn switch_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) => ui.text.width(l),
        None => 0,
    };
    let (tkw, tkh) = (ui.dp(36), ui.dp(18));
    let mut w = tkw;
    if tw != 0 {
        w = w.wrapping_add(ui.dp(8)).wrapping_add(tw);
    }
    let h = ui.dp(24);
    sp.s[0] = tkw;
    sp.s[1] = tkh;
    (w, h)
}

/// sty_switch: grey / accent pill, white knob with an outline sliding with value2 (0 off ..
/// 256 on), label
fn switch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, tkw, tkh, pos) = (sp.state, sp.value, sp.s[0], sp.s[1], sp.value2);
    // track
    let x = r[0];
    let y = (r[3].wrapping_sub(tkh) >> 1).wrapping_add(r[1]);
    let rad = tkh >> 1; // pill radius
    let fill = if v == 0 { ui.color(T_TRACK) } else { ui.color((state << 2).wrapping_add(T_ACCENT)) };
    ui.cmd_rrect(x, y, tkw, tkh, rad, 0, fill, 0, 0);
    // knob: radius 7 DIP with an outline (highlight on hover), travels from 9 to 27 DIP
    let c = ui.color(if state == 1 { T_ACCENT } else { T_CTRL_STROKE2 });
    ui.draw_color(c);
    let off = ui.dp_fx(9 * 256).wrapping_add(ui.dp_fx(18 * 256).wrapping_mul(pos) >> 8); // start offset (fx)
    let cx = (x << 8).wrapping_add(off);
    let cy = (y << 8).wrapping_add(tkh << 7);
    let k = ui.dp_fx(8 * 256);
    ui.draw_circle(cx, cy, k, 0);
    let c = ui.color(T_KNOB);
    ui.draw_color(c);
    let k = ui.dp_fx(7 * 256);
    ui.draw_circle(cx, cy, k, 0);
    // label
    label_after(ui, sp, tkw, 8);
}

/// sty_slider_size -> (w, h): 200 x 24, swapped when value = vertical; s[0] = thumb px
/// along the axis (the core maps pointer positions with it), s[1] = vertical
fn slider_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let vert = sp.value;
    sp.s[1] = vert;
    let thumb = ui.dp(20);
    let (a, b) = (ui.dp(200), ui.dp(24));
    sp.s[0] = thumb;
    if vert != 0 { (b, a) } else { (a, b) }
}

/// sty_slider: 6 DIP grey track, accent up to the value, round knob with an outline (value
/// thumb offset, value2 track length)
fn slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, vert, len, t, thumb) = (sp.state, sp.s[1], sp.value2, sp.value, sp.s[0]);
    let th = ui.dp(6); // track thickness
    let half = thumb >> 1; // half thumb
    let accent = (state << 2).wrapping_add(T_ACCENT);
    let (cx, cy);
    if vert == 0 {
        // horizontal: full track, then the accent part
        let c = ui.color(T_TRACK);
        ui.draw_color(c);
        let ty = (r[3].wrapping_sub(th) >> 1).wrapping_add(r[1]);
        ui.draw_rrect(r[0].wrapping_add(half), ty, len, th, th >> 1);
        let c = ui.color(accent);
        ui.draw_color(c);
        ui.draw_rrect(r[0].wrapping_add(half), ty, t, th, th >> 1);
        // thumb centre
        cx = r[0].wrapping_add(half).wrapping_add(t) << 8;
        cy = (r[3] << 7).wrapping_add(r[1] << 8);
    } else {
        let c = ui.color(T_TRACK);
        ui.draw_color(c);
        let tx = (r[2].wrapping_sub(th) >> 1).wrapping_add(r[0]);
        ui.draw_rrect(tx, r[1].wrapping_add(half), th, len, th >> 1);
        let c = ui.color(accent);
        ui.draw_color(c);
        // accent from the value point down to the bottom
        let ay = r[1].wrapping_add(r[3]).wrapping_sub(half).wrapping_sub(t);
        ui.draw_rrect(tx, ay, th, t, th >> 1);
        cx = (r[2] << 7).wrapping_add(r[0] << 8);
        cy = ay << 8;
    }
    // knob: outline (10 DIP; highlight while hovered / held) and the knob fill (9 DIP)
    let c = ui.color(if state == 1 || state == 2 { T_ACCENT } else { T_STRONG_STROKE });
    ui.draw_color(c);
    let k = ui.dp_fx(10 * 256);
    ui.draw_circle(cx, cy, k, 0);
    let c = ui.color(if state == 2 { T_CTRL_PRESSED } else { T_KNOB }) | 0xFF00_0000;
    ui.draw_color(c);
    let k = ui.dp_fx(9 * 256);
    ui.draw_circle(cx, cy, k, 0);
}

/// sty_progress: 6 DIP grey pill, accent pill up to value / value2 (at least a round dot)
fn progress(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (v, max) = (sp.value, sp.value2);
    let th = ui.dp(6); // thickness
    let y = (r[3].wrapping_sub(th) >> 1).wrapping_add(r[1]);
    let c = ui.color(T_TRACK);
    ui.draw_color(c);
    ui.draw_rrect(r[0], y, r[2], th, th >> 1);
    if max <= 0 || v <= 0 {
        return;
    }
    let mut w = v.min(max).wrapping_mul(r[2]) / max;
    if w < th {
        w = th; // at least a round dot
    }
    let c = ui.color(T_ACCENT);
    ui.draw_color(c);
    ui.draw_rrect(r[0], y, w, th, th >> 1);
}

/// sty_separator: hairline in the divider colour, centred across the rectangle (value 1 =
/// horizontal line, 0 = vertical)
fn separator(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    if sp.value != 0 {
        ui.draw_rect(r[0], (r[3].wrapping_sub(h) >> 1).wrapping_add(r[1]), r[2], h);
    } else {
        ui.draw_rect((r[2].wrapping_sub(h) >> 1).wrapping_add(r[0]), r[1], h, r[3]);
    }
}
