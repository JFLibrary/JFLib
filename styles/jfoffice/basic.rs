// jfOffice: Compact buttons, selection controls and progress indicators.
// Uses jfguiLib drawing primitives; local implementations derived from the
// existing native widget contract. Reference geometry is implemented locally.

use ui::consts::*;
use ui::keys::{IC_CHECK_MARK, IC_CHEV_DOWN};
use ui::{Sp, StyleBasic, Ui};

use crate::Office;
use crate::common::{control_font, current_baseline};
use crate::palette;

impl StyleBasic for Office {
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
        let h = ui.dp(1);
        ui.dp(8).wrapping_add(h)
    }
    fn separator(&self, ui: &mut Ui, sp: &Sp) {
        separator(ui, sp)
    }
}

/// A blue focus outline aligned to the control edge at every DPI.
pub(crate) fn focus_ring(ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
    let rad = ui.dp(radius_dip);
    let w = ui.dp(1);
    let c = ui.color(T_ACCENT);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, w, 0, c, c);
}

// A ribbon line is 22 DIP; standard dialog actions are 26 DIP.
fn btn_height(ui: &Ui, flags: u32) -> i32 {
    ui.dp(if flags & ST_COMPACT != 0 { 22 } else { 26 })
}

/// sty_button_size -> (w, h) (keeps the text width in s[0], the icon px in s[1])
fn button_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let (flags, icon) = (sp.flags, sp.icon);
    // measure
    if flags & ST_ON_ACCENT != 0 {
        ui.text.set_font(FONT_REG, ui.dp_fx(14 * 256));
    } else if flags & (ST_LARGE | ST_COMPACT) != 0 {
        control_font(ui);
    } else {
        ui.ts_font(TS_BODY);
    }
    let tw = match sp.label {
        Some(l) if flags & ST_ICON_ONLY == 0 => l
            .split('\n')
            .map(|line| ui.text.width(line))
            .max()
            .unwrap_or(0),
        _ => 0,
    };
    let (mut w, h, ip);
    if flags & ST_ON_ACCENT != 0 {
        sp.s[0] = tw;
        sp.s[1] = ui.dp(17);
        return (tw + ui.dp(if icon == 0 { 44 } else { 68 }), ui.dp(38));
    }
    if flags & ST_LARGE == 0 {
        // Normal: 8-DIP inset, 16-DIP icon, 5-DIP gap; tool buttons stay square.
        ip = ui.dp(if flags & ST_COMPACT != 0 { 17 } else { 16 });
        let mut ww = ui.dp(8).wrapping_mul(2).wrapping_add(tw);
        if icon != 0 {
            ww = ww.wrapping_add(ip);
            if tw == 0 {
                ww = btn_height(ui, flags); // square icon button
            } else {
                ww = ww.wrapping_add(ui.dp(5));
            }
        } else if flags & ST_SUBTLE == 0 && tw != 0 {
            // text-only push buttons are at least 72 DIP wide (dialog buttons)
            let min = ui.dp(72);
            if ww < min {
                ww = min;
            }
        }
        w = if flags & ST_COMPACT != 0 {
            if icon == 0 {
                (tw + ui.dp(12)).max(ui.dp(22))
            } else if tw == 0 {
                ui.dp(24)
            } else {
                tw + ui.dp(32)
            }
        } else {
            ww
        };
        h = btn_height(ui, flags);
    } else {
        // A large ribbon action has a 32-DIP icon above one or two 13-DIP label lines.
        ip = ui.dp(32);
        w = (tw + ui.dp(12)).max(ui.dp(44));
        h = ui.dp(66);
    }
    sp.s[0] = tw;
    sp.s[1] = ip;
    if flags & ST_MENU != 0 {
        if flags & ST_LARGE != 0 {
            let last = sp.label.unwrap_or("").rsplit('\n').next().unwrap_or("");
            w = w.max(ui.text.width(last) + ui.dp(22));
        } else {
            w += ui.dp(10);
        }
    }
    (w, h)
}

/// sty_button: fill, border, icon and label in the colours of the state
fn button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, flags, icon) = (sp.state, sp.flags, sp.icon);
    let label = sp.label.unwrap_or("");
    let (tw, ip) = (sp.s[0], sp.s[1]);
    if flags & ST_ON_ACCENT != 0 {
        on_accent_button(ui, sp);
        return;
    }
    let st = state.wrapping_mul(4);
    // colours: fill, stroke, bottom stroke, text
    let (fill, stroke, stroke2);
    let accent = flags & ST_ACCENT != 0;
    if accent {
        // Primary actions have a solid accent face, not the selection tint.
        fill = ui.color(T_ACCENT.wrapping_add(st));
        stroke = fill;
        stroke2 = stroke;
    } else if flags & ST_CHECKED != 0 {
        fill = if state == 2 {
            ui.color(T_SUBTLE_PRESSED)
        } else {
            palette::checked(ui.theme_dark())
        };
        stroke = if state == 3 {
            ui.color(T_CTRL_STROKE)
        } else {
            (ui.color(T_ACCENT) & 0xffffff) | 0x80000000
        };
        stroke2 = stroke;
    } else if flags & ST_SUBTLE != 0 {
        fill = ui.color(T_SUBTLE.wrapping_add(st));
        stroke = 0;
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
    let text = ui.color(if accent {
        if state == 3 {
            T_ON_ACCENT_DIS
        } else {
            T_ON_ACCENT
        }
    } else if state == 3 {
        T_TEXT_DISABLED
    } else {
        T_TEXT
    });
    let rad = if flags & ST_ROUND != 0 { r[2].min(r[3]) / 2 } else { ui.dp(4) };
    let t = if stroke != 0 { ui.dp(1) } else { 0 };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, fill, stroke, stroke2);
    // content
    let icon_color = if state == 3 || accent {
        text
    } else {
        palette::icon(ui.theme_dark())
    };
    ui.draw_color(icon_color);
    if flags & ST_LARGE != 0 {
        // icon centred at the top, caption centred below
        if icon != 0 {
            let x = (r[2].wrapping_sub(ip) >> 1).wrapping_add(r[0]);
            let y = r[1].wrapping_add(ui.dp(4));
            ui.draw_icon(icon, x, y, ip << 8);
        }
        if tw != 0 {
            control_font(ui);
            let line = ui.dp(13);
            let last = label.bytes().filter(|byte| *byte == b'\n').count();
            let mut top = r[1]
                + ui.dp(43)
                    .min((r[3] - line * (last + 1) as i32).max(ui.dp(36)));
            for (index, label_line) in label.split('\n').enumerate() {
                let width = ui.text.width(label_line);
                let menu = flags & ST_MENU != 0 && index == last;
                let x = r[0] + (r[2] - width - if menu { ui.dp(10) } else { 0 }) / 2;
                let y = current_baseline(ui, top, line);
                ui.emit_text(x, y, label_line, text);
                if menu {
                    menu_mark(
                        ui,
                        x + width + ui.dp(4),
                        top + (line - ui.dp(5)) / 2,
                        icon_color,
                    );
                }
                top += line;
            }
        }
        return;
    }
    // width of icon + gap + text, centred
    let mut cw = tw;
    if flags & ST_MENU != 0 {
        cw += ui.dp(10);
    }
    if icon != 0 {
        cw = cw.wrapping_add(ip);
        if tw != 0 {
            cw = cw.wrapping_add(ui.dp(5));
        }
    }
    let mut x = if flags & ST_COMPACT != 0 && icon != 0 && tw != 0 {
        r[0] + ui.dp(4)
    } else if flags & ST_ALIGN_START != 0 {
        r[0].wrapping_add(ui.dp(8))
    } else {
        r[0].wrapping_add(r[2].wrapping_sub(cw) >> 1)
    };
    if icon != 0 {
        let y = r[1].wrapping_add(r[3].wrapping_sub(ip) >> 1);
        ui.draw_icon(icon, x, y, ip << 8);
        x = if flags & ST_COMPACT != 0 {
            r[0] + ui.dp(24)
        } else {
            x + ip + ui.dp(5)
        };
    }
    if tw != 0 {
        if flags & ST_COMPACT != 0 {
            control_font(ui);
        } else {
            ui.ts_font(TS_BODY);
        }
        let y = current_baseline(ui, r[1], r[3]);
        if flags & ST_ALIGN_START != 0 {
            let tail = if flags & ST_MENU != 0 { 18 } else { 8 };
            let available = (r[0] + r[2] - ui.dp(tail) - x).max(0);
            start_label(ui, x, y, label, text, available);
        } else {
            ui.emit_text(x, y, label, text);
        }
    }
    if flags & ST_MENU != 0 {
        let menu_x = if icon == 0 && tw == 0 {
            r[0] + (r[2] - ui.dp(5)) / 2
        } else {
            (x + tw + ui.dp(4)).min(r[0] + r[2] - ui.dp(9))
        };
        menu_mark(ui, menu_x, r[1] + (r[3] - ui.dp(5)) / 2, icon_color);
    }
}

fn menu_mark(ui: &mut Ui, x: i32, y: i32, color: u32) {
    ui.draw_color(color);
    ui.draw_icon(IC_CHEV_DOWN, x, y, ui.dp(5) << 8);
}

// Accent-surface navigation remains a normal focusable semantic button.
fn on_accent_button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let alpha = if sp.state == 2 {
        56
    } else if sp.flags & ST_CHECKED != 0 {
        46
    } else if sp.state == 1 {
        26
    } else {
        0
    };
    if alpha != 0 {
        ui.draw_color((alpha << 24) | 0xffffff);
        ui.draw_rect(r[0], r[1], r[2], r[3]);
    }
    let color = if sp.state == 3 {
        0x80ffffff
    } else {
        0xffffffff
    };
    ui.text.set_font(if sp.flags & ST_CHECKED != 0 { FONT_BOLD } else { FONT_SEMI }, ui.dp_fx(14 * 256));
    let mut x = r[0] + ui.dp(if sp.flags & ST_ICON_ONLY != 0 { 17 } else { 22 });
    if sp.icon != 0 {
        ui.draw_color(color);
        let size = ui.dp(17);
        ui.draw_icon(sp.icon, x, r[1] + (r[3] - size) / 2, size << 8);
        x += ui.dp(24);
    }
    if sp.flags & ST_ICON_ONLY == 0 {
        let y = current_baseline(ui, r[1], r[3]);
        start_label(
            ui,
            x,
            y,
            sp.label.unwrap_or(""),
            color,
            (r[0] + r[2] - ui.dp(12) - x).max(0),
        );
    }
    if sp.ev & EV_FOCUS != 0 {
        let inset = ui.dp(3);
        ui.cmd_rrect(
            r[0] + inset,
            r[1] + inset,
            (r[2] - 2 * inset).max(0),
            (r[3] - 2 * inset).max(0),
            ui.dp(2),
            ui.dp(1),
            0,
            0x80ffffff,
            0x80ffffff,
        );
    }
}

// Keep semantic labels intact; only the visible paint is shortened at a UTF-8 boundary.
fn start_label(ui: &mut Ui, x: i32, y: i32, label: &str, color: u32, available: i32) {
    if available <= 0 {
        return;
    }
    let (font, size) = ui.text.get_font();
    let width = ui.text.width(label);
    if width <= available {
        ui.cmd_text(x, y, label.as_bytes(), font, size, color, available);
        return;
    }
    let dots = ui.text.width("…");
    if dots > available {
        return;
    }
    let room = available - dots;
    let mut end = ui.text.hit(label.as_bytes(), None, room);
    let mut width = ui.text.width(&label[..end]);
    while width > room && end > 0 {
        end = label[..end]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at);
        width = ui.text.width(&label[..end]);
    }
    ui.cmd_text(x, y, &label.as_bytes()[..end], font, size, color, room);
    ui.cmd_text(
        x + width,
        y,
        "…".as_bytes(),
        font,
        size,
        color,
        available - width,
    );
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
    let c = ui.color(if sp.state == 3 {
        T_TEXT_DISABLED
    } else {
        T_TEXT
    });
    let base = ui.ts_base(TS_BODY);
    let line = ui.ts_line(TS_BODY);
    let y = (r[3].wrapping_sub(line) >> 1)
        .wrapping_add(r[1])
        .wrapping_add(base);
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
    let t = ui.dp(1);
    let rad = ui.dp(2);
    ui.cmd_rrect(x, y, b, b, rad, t, fill, stroke, stroke);
    if v != 0 {
        // check mark (or dash for indeterminate) in the highlight colour
        let c = ui.color(if state == 3 {
            T_TEXT_DISABLED
        } else {
            T_ACCENT
        });
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
    let t = ui.dp(1) << 8;
    ui.draw_circle(cx, cy, rad.wrapping_sub(((t as u32) >> 1) as i32), t); // ring centred inside the circle
    if sp.value == 1 {
        let c = ui.color(if state == 3 {
            T_TEXT_DISABLED
        } else {
            T_ACCENT
        });
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
    let compact = sp.flags & ST_COMPACT != 0;
    let (tkw, tkh) = if compact {
        (ui.dp(30), ui.dp(16))
    } else {
        (ui.dp(36), ui.dp(18))
    };
    let mut w = tkw;
    if tw != 0 {
        w = w.wrapping_add(ui.dp(8)).wrapping_add(tw);
    }
    let h = ui.dp(if compact { 16 } else { 24 });
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
    if sp.flags & ST_COMPACT != 0 {
        let fill = ui.color(if v == 0 { T_INPUT_ACTIVE } else { T_ACCENT });
        let stroke = ui.color(if v == 0 { T_STRONG_STROKE } else { T_ACCENT });
        ui.cmd_rrect(x, y, tkw, tkh, rad, ui.dp(1), fill, stroke, stroke);
        let c = ui.color(if state == 3 {
            T_TEXT_DISABLED
        } else if v == 0 {
            T_TEXT2
        } else {
            T_ON_ACCENT
        });
        ui.draw_color(c);
        let cx = (x << 8) + ui.dp_fx(8 * 256) + (ui.dp_fx(14 * 256) * pos >> 8);
        ui.draw_circle(cx, (y << 8) + (tkh << 7), ui.dp_fx(5 * 256), 0);
        label_after(ui, sp, tkw, 8);
        return;
    }
    let fill = if v == 0 {
        ui.color(T_TRACK)
    } else {
        ui.color((state << 2).wrapping_add(T_ACCENT))
    };
    ui.cmd_rrect(x, y, tkw, tkh, rad, 0, fill, 0, 0);
    // knob: radius 7 DIP with an outline (highlight on hover), travels from 9 to 27 DIP
    let c = ui.color(if state == 1 { T_ACCENT } else { T_CTRL_STROKE2 });
    ui.draw_color(c);
    let off = ui
        .dp_fx(9 * 256)
        .wrapping_add(ui.dp_fx(18 * 256).wrapping_mul(pos) >> 8); // start offset (fx)
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
    let thumb = ui.dp(12);
    let (a, b) = (ui.dp(110), ui.dp(22));
    sp.s[0] = thumb;
    if vert != 0 { (b, a) } else { (a, b) }
}

/// A neutral slider track and a narrow outlined thumb, matching compact Office chrome.
fn slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (state, vert, len, t, thumb) = (sp.state, sp.s[1], sp.value2, sp.value, sp.s[0]);
    let th = ui.dp(4); // track thickness
    let half = thumb >> 1; // half thumb
    let (cx, cy);
    if vert == 0 {
        // horizontal: full track, then the accent part
        let c = ui.color(T_TRACK);
        ui.draw_color(c);
        let ty = (r[3].wrapping_sub(th) >> 1).wrapping_add(r[1]);
        ui.draw_rrect(r[0].wrapping_add(half), ty, len, th, th >> 1);
        // thumb centre
        cx = r[0].wrapping_add(half).wrapping_add(t) << 8;
        cy = (r[3] << 7).wrapping_add(r[1] << 8);
    } else {
        let c = ui.color(T_TRACK);
        ui.draw_color(c);
        let tx = (r[2].wrapping_sub(th) >> 1).wrapping_add(r[0]);
        ui.draw_rrect(tx, r[1].wrapping_add(half), th, len, th >> 1);
        // accent from the value point down to the bottom
        let ay = r[1].wrapping_add(r[3]).wrapping_sub(half).wrapping_sub(t);
        cx = (r[2] << 7).wrapping_add(r[0] << 8);
        cy = ay << 8;
    }
    let stroke = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let fill = ui.color(if state == 2 {
        T_CTRL_PRESSED
    } else if state == 1 {
        T_CTRL_HOVER
    } else {
        T_KNOB
    });
    let (width, height) = if vert == 0 {
        (thumb, ui.dp(18))
    } else {
        (ui.dp(18), thumb)
    };
    ui.cmd_rrect(
        (cx >> 8) - width / 2,
        (cy >> 8) - height / 2,
        width,
        height,
        ui.dp(4),
        ui.dp(1),
        fill,
        stroke,
        stroke,
    );
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
    let h = ui.dp(2);
    let c = ui::theme::color_mix(ui.color(T_CARD), ui.color(T_DIVIDER), 128);
    ui.draw_color(c);
    if sp.value != 0 {
        ui.draw_rect(
            r[0],
            (r[3].wrapping_sub(h) >> 1).wrapping_add(r[1]),
            r[2],
            h,
        );
    } else {
        ui.draw_rect(
            (r[2].wrapping_sub(h) >> 1).wrapping_add(r[0]),
            r[1],
            h,
            r[3],
        );
    }
}
