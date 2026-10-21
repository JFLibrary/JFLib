// basic -- NetLab buttons, check boxes, radios, switches, sliders, progress, separators
//
//   focus_ring(r, radius)    1 DIP accent ring with a 1 DIP gap outside the control
//   button_size / button     28 DIP (compact 24), padding 12 (compact 8), icon 16 + gap 6,
//                            text-only push buttons at least 76 wide; flat control fill with
//                            a 1 DIP frame (a shade darker along the bottom), the frame drawn
//                            toward the accent on hover; primary (ST_ACCENT): accent fill with
//                            a darker accent frame; checked: the selection (16 % accent tint,
//                            accent frame); subtle: flat, on hover a shade with a frame (the
//                            classic tool button); ST_LARGE: 24 DIP icon over a caption
//   checkbox / radio         16 DIP on a 22 DIP row: field fill with a strong frame (accent
//                            on hover); checked: accent fill with an on-accent check / an
//                            accent ring with an accent dot
//   switch                   34 x 18 rounded rectangle with a square knob: framed field off,
//                            accent on
//   slider                   4 DIP track, accent up to the value, a 10 x 18 DIP framed thumb
//   progress                 a framed 8 DIP bar with the accent inside; separator: a divider
//                            hairline

use ui::consts::*;
use ui::keys::IC_CHECK_MARK;
use ui::theme::color_mix;
use ui::{Sp, StyleBasic, Ui};

use crate::common::{card, fill_rrect, framed, select_colors, stroke_rrect, R_CTRL};
use crate::Netlab;

const THUMB_W: i32 = 10; // slider thumb along the track (DIP)
const THUMB_H: i32 = 18; // ... across it

impl StyleBasic for Netlab {
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
        check_size(ui, sp)
    }
    fn checkbox(&self, ui: &mut Ui, sp: &Sp) {
        checkbox(ui, sp)
    }
    fn radio_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
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
        // s[0] = the thumb length along the track (the core keeps the travel inside it)
        let vert = sp.value;
        sp.s[1] = vert;
        sp.s[0] = ui.dp(THUMB_W);
        let (a, b) = (ui.dp(200), ui.dp(24));
        if vert != 0 { (b, a) } else { (a, b) }
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
        ui.dp(9).wrapping_add(ui.hair())
    }
    fn separator(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let h = ui.hair();
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        if sp.value != 0 {
            ui.draw_rect(r[0], (r[3] - h) / 2 + r[1], r[2], h);
        } else {
            ui.draw_rect((r[2] - h) / 2 + r[0], r[1], h, r[3]);
        }
    }
}

/// the focus visual: a 1 DIP accent ring with a 1 DIP gap outside the rectangle
pub(crate) fn focus_ring(ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
    let t = ui.dp(1).max(ui.hair());
    let d = ui.dp(1) + t;
    let c = ui.color(T_FOCUS_OUT);
    let rad = ui.dp(radius_dip.max(R_CTRL) + 2);
    stroke_rrect(ui, r[0] - d, r[1] - d, r[2] + d * 2, r[3] + d * 2, rad, t, c);
}

fn btn_height(ui: &Ui, flags: u32) -> i32 {
    ui.dp(if flags & ST_COMPACT != 0 { 24 } else { 28 })
}

/// button_size -> (w, h); s[0] = text width, s[1] = icon px
fn button_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let (flags, icon) = (sp.flags, sp.icon);
    ui.ts_font(if flags & ST_LARGE != 0 { TS_CAPTION } else { TS_BODY });
    let tw = match sp.label {
        Some(l) if flags & ST_ICON_ONLY == 0 => ui.text.width(l),
        _ => 0,
    };
    let (w, h, ip);
    if flags & ST_LARGE == 0 {
        ip = ui.dp(16);
        let pad = ui.dp(if flags & ST_COMPACT != 0 { 8 } else { 12 });
        let mut ww = pad * 2 + tw;
        if icon != 0 {
            ww += ip;
            if tw == 0 {
                ww = btn_height(ui, flags); // square icon button
            } else {
                ww += ui.dp(6);
            }
        } else if flags & (ST_SUBTLE | ST_COMPACT) == 0 && tw != 0 {
            ww = ww.max(ui.dp(76));
        }
        if flags & ST_MENU != 0 {
            ww += ui.dp(18);
        }
        w = ww;
        h = btn_height(ui, flags);
    } else {
        ip = ui.dp(24);
        w = tw.max(ip) + ui.dp(16);
        h = ui.dp(52);
    }
    sp.s[0] = tw;
    sp.s[1] = ip;
    (w, h)
}

/// the frame colours of a standard control in a state (0 rest, 1 hover, 2 pressed,
/// 3 disabled) -> (frame, bottom edge)
pub(crate) fn ctl_strokes(ui: &Ui, state: u32) -> (u32, u32) {
    let s = ui.color(T_CTRL_STROKE);
    let accent = ui.color(T_ACCENT);
    match state {
        1 => {
            let c = color_mix(s, accent, 112);
            (c, c)
        }
        2 => {
            let c = color_mix(s, accent, 160);
            (c, c)
        }
        3 => (s, s),
        _ => (s, ui.color(T_CTRL_STROKE2)),
    }
}

/// button: surface, icon and label in the colours of the state (0 rest, 1 hover, 2 pressed,
/// 3 disabled)
fn button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, flags, icon) = (sp.state, sp.flags, sp.icon);
    let label = sp.label.unwrap_or("");
    let (tw, ip) = (sp.s[0], sp.s[1]);
    let rad = ui.dp(if flags & ST_ROUND != 0 { 14 } else { R_CTRL });
    let hair = ui.hair();
    let text;
    let mut icon_c = 0;
    if flags & ST_ACCENT != 0 && flags & ST_SUBTLE == 0 {
        // primary: accent fill, a darker accent frame
        let fill = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        let edge = if state == 3 { fill } else { color_mix(fill, 0xFF00_0000, 48) };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, edge, edge);
        text = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
    } else if flags & ST_CHECKED != 0 {
        // checked tool / toggle button: the selection
        let base = if flags & ST_SUBTLE != 0 { ui.color(T_LAYER) } else { ui.color(T_CTRL) };
        let (fill, stroke) = select_colors(ui, base, state == 1 || state == 2);
        let stroke = if state == 3 { ui.color(T_CTRL_STROKE) } else { stroke };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
        text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
        icon_c = if state == 3 { text } else { ui.color(T_ACCENT_TEXT) };
    } else if flags & ST_SUBTLE != 0 {
        // tool button: flat; a shade and a frame while pointed at
        if state == 1 || state == 2 {
            let fill = ui.color(T_SUBTLE + state * 4);
            let stroke = ui.color(if state == 2 { T_CTRL_STROKE2 } else { T_CTRL_STROKE });
            ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
        }
        text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    } else {
        // standard: control fill, 1 DIP frame
        let fill = if flags & ST_ON_ACCENT != 0 { 0 } else { ui.color(T_CTRL + state * 4) };
        let (stroke, stroke2) = ctl_strokes(ui, state);
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke2);
        text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    }
    let icon_c = if icon_c == 0 { text } else { icon_c };
    if flags & ST_LARGE != 0 {
        if icon != 0 {
            let x = r[0] + ((r[2] - ip) >> 1);
            let y = r[1] + ui.dp(6);
            ui.draw_color(icon_c);
            ui.draw_icon(icon, x, y, ip << 8);
        }
        if tw != 0 {
            let base = ui.ts_base(TS_CAPTION);
            ui.ts_font(TS_CAPTION);
            let x = r[0] + ((r[2] - tw) >> 1);
            let y = r[1] + ui.dp(33) + base;
            ui.emit_text(x, y, label, text);
        }
        return;
    }
    let mut cw = tw;
    if icon != 0 {
        cw += ip;
        if tw != 0 {
            cw += ui.dp(6);
        }
    }
    let menu = flags & ST_MENU != 0;
    let avail = r[2] - if menu { ui.dp(18) } else { 0 };
    let mut x = if flags & ST_ALIGN_START != 0 { r[0] + ui.dp(10) } else { r[0] + ((avail - cw) >> 1).max(ui.dp(4)) };
    if icon != 0 {
        let y = r[1] + ((r[3] - ip) >> 1);
        ui.draw_color(icon_c);
        ui.draw_icon(icon, x, y, ip << 8);
        x += ip + ui.dp(6);
    }
    if tw != 0 {
        let base = ui.ts_base(TS_BODY);
        let line = ui.ts_line(TS_BODY);
        let y = r[1] + ((r[3] - line) >> 1) + base;
        let maxw = (r[0] + avail - x - ui.dp(4)).max(1);
        ui.ts_font(TS_BODY);
        let (font, size) = ui.text.get_font();
        let b = label.as_bytes();
        let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        ui.cmd_text(x, y, &b[..n], font, size, text, if tw > maxw { maxw } else { 0 });
    }
    if menu {
        let s = ui.dp(10);
        ui.draw_color(text);
        ui.draw_icon(ui::keys::IC_CHEV_DOWN, r[0] + r[2] - ui.dp(13) - (s >> 1), r[1] + ((r[3] - s) >> 1), s << 8);
    }
}

/// check box / radio size: 16 DIP mark, gap 6, 22 high; s[0] = mark px
fn check_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = sp.label.map_or(0, |l| ui.text.width(l));
    let b = ui.dp(16);
    let mut w = b;
    if tw != 0 {
        w += ui.dp(6) + tw;
    }
    sp.s[0] = b;
    (w, ui.dp(22))
}

fn label_after(ui: &mut Ui, sp: &Sp, x_after: i32, gap: i32) {
    let Some(l) = sp.label else { return };
    if l.is_empty() {
        return;
    }
    let r = sp.rect();
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let base = ui.ts_base(TS_BODY);
    let line = ui.ts_line(TS_BODY);
    let y = r[1] + ((r[3] - line) >> 1) + base;
    ui.ts_font(TS_BODY);
    ui.emit_text(r[0] + x_after + ui.dp(gap), y, l, c);
}

// the frame of an unchecked mark in a state
fn mark_stroke(ui: &Ui, state: u32) -> u32 {
    match state {
        1 | 2 => ui.color(T_ACCENT),
        3 => ui.color(T_TEXT_DISABLED),
        _ => ui.color(T_STRONG_STROKE),
    }
}

/// check box: 2 DIP corners; off: field fill with a strong frame (accent on hover); on /
/// indeterminate: accent fill with an on-accent check mark / dash
fn checkbox(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, b) = (sp.state, sp.value, sp.s[0]);
    let x = r[0];
    let y = r[1] + ((r[3] - b) >> 1);
    let rad = ui.dp(2);
    if v == 0 {
        let fill = ui.color(match state {
            2 => T_CTRL_PRESSED,
            3 => T_CTRL_DISABLED,
            _ => T_ALT_FILL,
        });
        let stroke = mark_stroke(ui, state);
        framed(ui, x, y, b, b, rad, fill, stroke);
    } else {
        let fill = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        let edge = if state == 3 { fill } else { color_mix(fill, 0xFF00_0000, 40) };
        framed(ui, x, y, b, b, rad, fill, edge);
        let c = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
        ui.draw_color(c);
        if v == 2 {
            let (w, h) = (ui.dp(8), ui.dp(2));
            ui.draw_rect(x + ((b - w) >> 1), y + ((b - h) >> 1), w, h);
        } else {
            let g = ui.dp(12);
            let o = (b - g) >> 1;
            ui.draw_icon(IC_CHECK_MARK, x + o, y + o, g << 8);
        }
    }
    label_after(ui, sp, b, 6);
}

/// radio: field fill with a strong ring (accent on hover); on: an accent ring with an accent
/// dot
fn radio(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, b) = (sp.state, sp.s[0]);
    let rad = b << 7;
    let cx = (r[0] << 8) + rad;
    let cy = (r[1] << 8) + (r[3] << 7);
    let on = sp.value == 1;
    let fill = ui.color(match state {
        2 => T_CTRL_PRESSED,
        3 => T_CTRL_DISABLED,
        _ => T_ALT_FILL,
    });
    ui.draw_color(fill);
    ui.draw_circle(cx, cy, rad, 0);
    let ring = if on {
        ui.color(match state {
            1 => T_ACCENT_HOVER,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        })
    } else {
        mark_stroke(ui, state)
    };
    ui.draw_color(ring);
    let t = ui.hair() << 8;
    ui.draw_circle(cx, cy, rad - (t >> 1), t);
    if on {
        let c = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        ui.draw_color(c);
        ui.draw_circle(cx, cy, ui.dp_fx(4 * 256), 0);
    }
    label_after(ui, sp, b, 6);
}

/// switch size: track 34 x 18 (compact 28 x 14), gap 8, 22 high; s[0] / s[1] = track w / h
fn switch_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = sp.label.map_or(0, |l| ui.text.width(l));
    let compact = sp.flags & ST_COMPACT != 0;
    let (tkw, tkh) = if compact { (ui.dp(28), ui.dp(14)) } else { (ui.dp(34), ui.dp(18)) };
    let mut w = tkw;
    if tw != 0 {
        w += ui.dp(8) + tw;
    }
    sp.s[0] = tkw;
    sp.s[1] = tkh;
    (w, ui.dp(22))
}

/// switch: off: framed field with a strong-fill knob; on: accent with an on-accent knob;
/// value2 = knob position 0 .. 256
fn switch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, tkw, tkh, pos) = (sp.state, sp.value, sp.s[0], sp.s[1], sp.value2);
    let x = r[0];
    let y = r[1] + ((r[3] - tkh) >> 1);
    let rad = ui.dp(R_CTRL);
    let knob_c;
    if v != 0 {
        let c = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        let edge = if state == 3 { c } else { color_mix(c, 0xFF00_0000, 40) };
        framed(ui, x, y, tkw, tkh, rad, c, edge);
        knob_c = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
    } else {
        let fill = ui.color(match state {
            2 => T_CTRL_PRESSED,
            3 => T_CTRL_DISABLED,
            _ => T_ALT_FILL,
        });
        let stroke = mark_stroke(ui, state);
        framed(ui, x, y, tkw, tkh, rad, fill, stroke);
        knob_c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_STRONG_FILL });
    }
    // knob: a square 3 DIP inside the track, travelling between its ends
    let m = ui.dp(3);
    let k = tkh - m * 2;
    let travel = tkw - m * 2 - k;
    let kx = x + m + (travel * pos.clamp(0, 256)) / 256;
    fill_rrect(ui, kx, y + m, k, k, ui.dp(2), knob_c);
    label_after(ui, sp, tkw, 8);
}

/// slider: 4 DIP track, the accent part, a framed rectangular thumb (accent frame on hover)
fn slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, vert, len, t, thumb) = (sp.state, sp.s[1], sp.value2, sp.value, sp.s[0]);
    let th = ui.dp(4);
    let half = thumb >> 1;
    let acc = ui.color(if state == 3 { T_ACCENT_DISABLED } else { T_ACCENT });
    let track = ui.color(T_TRACK);
    let rad = ui.dp(1);
    let across = ui.dp(THUMB_H);
    let tr;
    if vert == 0 {
        let ty = r[1] + ((r[3] - th) >> 1);
        fill_rrect(ui, r[0] + half, ty, len, th, rad, track);
        fill_rrect(ui, r[0] + half, ty, t, th, rad, acc);
        tr = [r[0] + t, r[1] + ((r[3] - across) >> 1), thumb, across];
    } else {
        let tx = r[0] + ((r[2] - th) >> 1);
        fill_rrect(ui, tx, r[1] + half, th, len, rad, track);
        let ay = r[1] + r[3] - half - t;
        fill_rrect(ui, tx, ay, th, t, rad, acc);
        tr = [r[0] + ((r[2] - across) >> 1), ay - half, across, thumb];
    }
    let fill = ui.color(match state {
        2 => T_CTRL_PRESSED,
        3 => T_CTRL_DISABLED,
        _ => T_CTRL,
    }) | 0xFF00_0000;
    let stroke = match state {
        1 | 2 => ui.color(T_ACCENT),
        3 => ui.color(T_CTRL_STROKE),
        _ => ui.color(T_STRONG_STROKE),
    };
    framed(ui, tr[0], tr[1], tr[2], tr[3], ui.dp(2), fill, stroke);
}

/// progress: a framed 8 DIP bar on the field colour, the accent inside up to value / value2
fn progress(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (v, max) = (sp.value, sp.value2);
    let th = ui.dp(8).min(r[3]);
    let y = r[1] + ((r[3] - th) >> 1);
    let rad = ui.dp(2);
    let field = card(ui);
    let stroke = ui.color(T_CTRL_STROKE);
    framed(ui, r[0], y, r[2], th, rad, field, stroke);
    if max <= 0 || v <= 0 {
        return;
    }
    let h = ui.hair();
    let iw = r[2] - h * 2;
    let w = ((v.min(max) as i64 * iw as i64) / max as i64) as i32;
    let c = ui.color(T_ACCENT);
    fill_rrect(ui, r[0] + h, y + h, w.max(h), th - h * 2, ui.dp(1), c);
}
