// basic -- BackupCraft buttons, check boxes, radios, switches, sliders, progress, separators
//
//   focus_ring(r, radius)    2 DIP accent ring 2 DIP outside the control (WinUI)
//   button_size / button     32 DIP (compact 26), padding 12, icon 16 + gap 8, text-only push
//                            buttons at least 80 wide; outlined surface with a darker bottom
//                            edge, accent-tinted outline on hover (KDE), accent-filled primary
//                            (ST_ACCENT); checked: accent tint with an accent outline; subtle:
//                            flat, a shade on hover; ST_LARGE: 24 DIP icon over a caption
//   checkbox / radio         18 DIP, accent-filled when checked with an on-accent mark
//   switch                   40 x 20 pill: outlined off, accent on; the knob grows on hover
//   slider                   4 DIP track, accent up to the value, ringed thumb with an
//                            accent core that grows on hover
//   progress                 4 DIP pill; separator: a divider hairline

use ui::consts::*;
use ui::keys::IC_CHECK_MARK;
use ui::theme::color_mix;
use ui::{Sp, StyleBasic, Ui};

use crate::common::{fill_rrect, stroke_rrect, R_CTRL};
use crate::BackupCraft;

impl StyleBasic for BackupCraft {
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
        let vert = sp.value;
        sp.s[1] = vert;
        sp.s[0] = ui.dp(20);
        let (a, b) = (ui.dp(200), ui.dp(28));
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

/// the focus visual: a 2 DIP accent ring 2 DIP outside the rectangle (radius + 3)
pub(crate) fn focus_ring(ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
    let d = ui.dp(3);
    let t = ui.dp(2);
    let c = ui.color(T_FOCUS_OUT);
    let rad = ui.dp(radius_dip.max(R_CTRL) + 3);
    stroke_rrect(ui, r[0] - d, r[1] - d, r[2] + d * 2, r[3] + d * 2, rad, t, c);
}

fn btn_height(ui: &Ui, flags: u32) -> i32 {
    ui.dp(if flags & ST_COMPACT != 0 { 26 } else { 32 })
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
        let pad = ui.dp(if flags & ST_COMPACT != 0 { 10 } else { 12 });
        let mut ww = pad * 2 + tw;
        if icon != 0 {
            ww += ip;
            if tw == 0 {
                ww = btn_height(ui, flags); // square icon button
            } else {
                ww += ui.dp(8);
            }
        } else if flags & (ST_SUBTLE | ST_COMPACT) == 0 && tw != 0 {
            ww = ww.max(ui.dp(80));
        }
        if flags & ST_MENU != 0 {
            ww += ui.dp(20);
        }
        w = ww;
        h = btn_height(ui, flags);
    } else {
        ip = ui.dp(24);
        w = tw.max(ip) + ui.dp(20);
        h = ui.dp(56);
    }
    sp.s[0] = tw;
    sp.s[1] = ip;
    (w, h)
}

/// button: surface, icon and label in the colours of the state (0 rest, 1 hover, 2 pressed,
/// 3 disabled)
fn button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, flags, icon) = (sp.state, sp.flags, sp.icon);
    let label = sp.label.unwrap_or("");
    let (tw, ip) = (sp.s[0], sp.s[1]);
    let rad = ui.dp(if flags & ST_ROUND != 0 { 16 } else { R_CTRL });
    let hair = ui.hair();
    let accent = ui.color(T_ACCENT);
    let text;
    let mut icon_c = 0;
    if flags & ST_ACCENT != 0 && flags & ST_SUBTLE == 0 {
        // primary: accent fill, a darker bottom edge
        let fill = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        let edge = if state == 3 { fill } else { color_mix(fill, 0xFF00_0000, 40) };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, fill, edge);
        text = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
    } else if flags & ST_SUBTLE != 0 {
        if flags & ST_CHECKED != 0 {
            // checked tool: accent tint, accent icon
            let card = ui.color(T_CARD) | 0xFF00_0000;
            let fill = color_mix(card, accent, if ui.theme_dark() { 64 } else { 40 });
            let fill = if state == 1 { color_mix(fill, accent, 24) } else { fill };
            fill_rrect(ui, r[0], r[1], r[2], r[3], rad, fill);
            text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
            icon_c = if state == 3 { text } else { ui.color(T_ACCENT_TEXT) };
        } else {
            let c = ui.color(T_SUBTLE + state * 4);
            fill_rrect(ui, r[0], r[1], r[2], r[3], rad, c);
            text = ui.color(match state {
                3 => T_TEXT_DISABLED,
                2 => T_TEXT2,
                _ => T_TEXT,
            });
        }
    } else if flags & ST_CHECKED != 0 {
        // checked push button: accent tint with an accent outline
        let base = ui.color(T_CTRL) | 0xFF00_0000;
        let fill = color_mix(base, accent, match state {
            1 => 56,
            2 => 72,
            _ => 40,
        });
        let stroke = if state == 3 { ui.color(T_CTRL_STROKE) } else { accent };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke);
        text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
        icon_c = if state == 3 { text } else { ui.color(T_ACCENT_TEXT) };
    } else {
        // standard: outlined control, darker bottom edge; accent-tinted outline on hover
        let fill = ui.color(T_CTRL + state * 4);
        let (stroke, stroke2) = match state {
            1 => {
                let s = color_mix(ui.color(T_CTRL_STROKE), accent, 150);
                (s, s)
            }
            2 => (accent, accent),
            3 => {
                let s = ui.color(T_CTRL_STROKE);
                (s, s)
            }
            _ => (ui.color(T_CTRL_STROKE), ui.color(T_CTRL_STROKE2)),
        };
        let fill = if flags & ST_ON_ACCENT != 0 { 0 } else { fill };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, hair, fill, stroke, stroke2);
        text = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    }
    let icon_c = if icon_c == 0 { text } else { icon_c };
    if flags & ST_LARGE != 0 {
        if icon != 0 {
            let x = r[0] + ((r[2] - ip) >> 1);
            let y = r[1] + ui.dp(8);
            ui.draw_color(icon_c);
            ui.draw_icon(icon, x, y, ip << 8);
        }
        if tw != 0 {
            let base = ui.ts_base(TS_CAPTION);
            ui.ts_font(TS_CAPTION);
            let x = r[0] + ((r[2] - tw) >> 1);
            let y = r[1] + ui.dp(36) + base;
            ui.emit_text(x, y, label, text);
        }
        return;
    }
    let mut cw = tw;
    if icon != 0 {
        cw += ip;
        if tw != 0 {
            cw += ui.dp(8);
        }
    }
    let menu = flags & ST_MENU != 0;
    let avail = r[2] - if menu { ui.dp(20) } else { 0 };
    let mut x = if flags & ST_ALIGN_START != 0 { r[0] + ui.dp(12) } else { r[0] + ((avail - cw) >> 1).max(ui.dp(6)) };
    if icon != 0 {
        let y = r[1] + ((r[3] - ip) >> 1);
        ui.draw_color(icon_c);
        ui.draw_icon(icon, x, y, ip << 8);
        x += ip + ui.dp(8);
    }
    if tw != 0 {
        let base = ui.ts_base(TS_BODY);
        let line = ui.ts_line(TS_BODY);
        let y = r[1] + ((r[3] - line) >> 1) + base;
        let maxw = (r[0] + avail - x - ui.dp(6)).max(1);
        let (font, size) = ui.text.get_font();
        let b = label.as_bytes();
        let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        ui.cmd_text(x, y, &b[..n], font, size, text, if tw > maxw { maxw } else { 0 });
    }
    if menu {
        let s = ui.dp(10);
        ui.draw_color(text);
        ui.draw_icon(ui::keys::IC_CHEV_DOWN, r[0] + r[2] - ui.dp(14) - (s >> 1), r[1] + ((r[3] - s) >> 1), s << 8);
    }
}

/// check box / radio size: 18 DIP mark, gap 8, 24 high; s[0] = mark px
fn check_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = sp.label.map_or(0, |l| ui.text.width(l));
    let b = ui.dp(18);
    let mut w = b;
    if tw != 0 {
        w += ui.dp(8) + tw;
    }
    sp.s[0] = b;
    (w, ui.dp(24))
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
    ui.emit_text(r[0] + x_after + ui.dp(gap), y, l, c);
}

/// check box: 4 DIP corners; off: alt fill with a strong outline (accent on hover); on /
/// indeterminate: accent fill with an on-accent check mark / dash
fn checkbox(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, b) = (sp.state, sp.value, sp.s[0]);
    let x = r[0];
    let y = r[1] + ((r[3] - b) >> 1);
    let rad = ui.dp(4);
    let hair = ui.hair();
    if v == 0 {
        let fill = ui.color(if state == 3 { T_CTRL_DISABLED } else { T_ALT_FILL });
        let stroke = ui.color(match state {
            1 | 2 => T_ACCENT,
            3 => T_TEXT_DISABLED,
            _ => T_STRONG_STROKE,
        });
        let fill = if state == 2 { ui.color(T_CTRL_PRESSED) } else { fill };
        ui.cmd_rrect(x, y, b, b, rad, hair, fill, stroke, stroke);
    } else {
        let fill = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        fill_rrect(ui, x, y, b, b, rad, fill);
        let c = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
        ui.draw_color(c);
        if v == 2 {
            let (w, h) = (ui.dp(8), ui.dp(2));
            ui.draw_rrect(x + ((b - w) >> 1), y + ((b - h) >> 1), w, h, h >> 1);
        } else {
            let g = ui.dp(12);
            let o = (b - g) >> 1;
            ui.draw_icon(IC_CHECK_MARK, x + o, y + o, g << 8);
        }
    }
    label_after(ui, sp, b, 8);
}

/// radio: off: alt fill with a strong ring; on: accent disc with an on-accent dot (larger on
/// hover)
fn radio(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, b) = (sp.state, sp.s[0]);
    let rad = b << 7;
    let cx = (r[0] << 8) + rad;
    let cy = (r[1] << 8) + (r[3] << 7);
    if sp.value == 1 {
        let c = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
        let c = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
        ui.draw_color(c);
        let d = ui.dp_fx(if state == 1 { 5 * 256 } else if state == 2 { 3 * 256 } else { 4 * 256 });
        ui.draw_circle(cx, cy, d, 0);
    } else {
        let c = ui.color(if state == 2 { T_CTRL_PRESSED } else { T_ALT_FILL });
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
        let c = ui.color(match state {
            1 | 2 => T_ACCENT,
            3 => T_TEXT_DISABLED,
            _ => T_STRONG_STROKE,
        });
        ui.draw_color(c);
        let t = ui.hair() << 8;
        ui.draw_circle(cx, cy, rad - (t >> 1), t);
    }
    label_after(ui, sp, b, 8);
}

/// switch size: track 40 x 20, gap 10, 24 high; s[0] / s[1] = track w / h
fn switch_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = sp.label.map_or(0, |l| ui.text.width(l));
    let compact = sp.flags & ST_COMPACT != 0;
    let (tkw, tkh) = if compact { (ui.dp(32), ui.dp(16)) } else { (ui.dp(40), ui.dp(20)) };
    let mut w = tkw;
    if tw != 0 {
        w += ui.dp(10) + tw;
    }
    sp.s[0] = tkw;
    sp.s[1] = tkh;
    (w, ui.dp(24))
}

/// switch: off: outlined pill with a strong-fill knob; on: accent pill with an on-accent knob;
/// value2 = knob position 0 .. 256
fn switch(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, v, tkw, tkh, pos) = (sp.state, sp.value, sp.s[0], sp.s[1], sp.value2);
    let x = r[0];
    let y = r[1] + ((r[3] - tkh) >> 1);
    let rad = tkh >> 1;
    let hair = ui.hair();
    let knob_c;
    if v != 0 {
        let c = ui.color(match state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            3 => T_ACCENT_DISABLED,
            _ => T_ACCENT,
        });
        fill_rrect(ui, x, y, tkw, tkh, rad, c);
        knob_c = ui.color(if state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT });
    } else {
        let fill = ui.color(if state == 1 { T_CTRL_HOVER } else { T_ALT_FILL });
        let stroke = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_STRONG_STROKE });
        ui.cmd_rrect(x, y, tkw, tkh, rad, hair, fill, stroke, stroke);
        knob_c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_STRONG_FILL });
    }
    // knob: 12 DIP (14 on hover, 16 wide while pressed), travelling between the pill ends
    let k = ui.dp_fx(match state {
        1 => 7 * 256,
        2 => 7 * 256,
        _ => 6 * 256,
    }) * tkh / ui.dp(20).max(1);
    let edge = (tkh << 7) as i64;
    let travel = ((tkw << 8) as i64) - edge * 2;
    let cx = (x << 8) as i64 + edge + travel * pos as i64 / 256;
    let cy = (y << 8) + (tkh << 7);
    ui.draw_color(knob_c);
    ui.draw_circle(cx as i32, cy, k, 0);
    label_after(ui, sp, tkw, 10);
}

/// slider: 4 DIP track, accent part, thumb: control-coloured disc with a hairline and an
/// accent core (bigger on hover, smaller while pressed)
fn slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, vert, len, t, thumb) = (sp.state, sp.s[1], sp.value2, sp.value, sp.s[0]);
    let th = ui.dp(4);
    let half = thumb >> 1;
    let acc = ui.color(if state == 3 { T_ACCENT_DISABLED } else { T_ACCENT });
    let track = ui.color(T_TRACK);
    let (cx, cy);
    if vert == 0 {
        let ty = r[1] + ((r[3] - th) >> 1);
        ui.draw_color(track);
        ui.draw_rrect(r[0] + half, ty, len, th, th >> 1);
        ui.draw_color(acc);
        ui.draw_rrect(r[0] + half, ty, t, th, th >> 1);
        cx = (r[0] + half + t) << 8;
        cy = (r[3] << 7) + (r[1] << 8);
    } else {
        let tx = r[0] + ((r[2] - th) >> 1);
        ui.draw_color(track);
        ui.draw_rrect(tx, r[1] + half, th, len, th >> 1);
        let ay = r[1] + r[3] - half - t;
        ui.draw_color(acc);
        ui.draw_rrect(tx, ay, th, t, th >> 1);
        cx = (r[2] << 7) + (r[0] << 8);
        cy = ay << 8;
    }
    let ring = ui.color(T_CTRL_STROKE);
    ui.draw_color(ring);
    ui.draw_circle(cx, cy, ui.dp_fx(10 * 256), 0);
    let knob = ui.color(T_CTRL) | 0xFF00_0000;
    ui.draw_color(knob);
    ui.draw_circle(cx, cy, ui.dp_fx(10 * 256) - (ui.hair() << 8), 0);
    ui.draw_color(acc);
    let core = match state {
        1 => 7 * 256,
        2 => 5 * 256,
        _ => 6 * 256,
    };
    ui.draw_circle(cx, cy, ui.dp_fx(core), 0);
}

/// progress: 4 DIP track, accent pill up to value / value2
fn progress(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (v, max) = (sp.value, sp.value2);
    let th = ui.dp(4);
    let y = r[1] + ((r[3] - th) >> 1);
    let c = ui.color(T_TRACK);
    fill_rrect(ui, r[0], y, r[2], th, th >> 1, c);
    if max <= 0 || v <= 0 {
        return;
    }
    let w = ((v.min(max) as i64 * r[2] as i64) / max as i64) as i32;
    let c = ui.color(T_ACCENT);
    fill_rrect(ui, r[0], y, w.max(th), th, th >> 1, c);
}
