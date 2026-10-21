// stock -- the default painters of the data views and the app shell parts: drawn from the
// theme tokens only, so every style shows grids, trees, badges, meters, charts, navigation
// panes, command bars, splitters, breadcrumbs, segmented controls, wizard steps, toasts,
// spinners and calendars.  A style that wants its own look returns its StyleData /
// StyleShell from Style::data / Style::shell and overrides the methods it likes; the others
// keep these painters (call them from the override to extend them).
//
//   kind_colors(ui, BADGE_*) -> (text, fill, solid)   the colours of a status kind
//   Stock                                             the stock StyleData + StyleShell

use alloc::string::String;

use crate::consts::*;
use crate::keys::*;
use crate::theme::color_mix;
use crate::{Sp, StyleData, StyleShell, Ui, DOCK_MK_BOTTOM, DOCK_MK_LEFT, DOCK_MK_RIGHT, DOCK_MK_TOP};

/// the stock data-view and shell painters (Style::data / Style::shell default)
pub struct Stock;

impl StyleData for Stock {}
impl StyleShell for Stock {}

// ------------------------------------------------------------------ helpers

fn fill(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32) {
    if argb & 0xFF00_0000 != 0 && w > 0 && h > 0 {
        ui.cmd_rrect(x, y, w, h, r, 0, argb, 0, 0);
    }
}

fn frame(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, argb: u32) {
    if argb & 0xFF00_0000 != 0 && w > 0 && h > 0 {
        ui.cmd_rrect(x, y, w, h, r, t, 0, argb, argb);
    }
}

/// baseline of text style ts centred in y .. y + h (selects the font)
pub(crate) fn baseline(ui: &mut Ui, ts: u32, y: i32, h: i32) -> i32 {
    let line = ui.ts_line(ts);
    let b = ui.ts_base(ts);
    y + ((h - line) >> 1) + b
}

/// text in style ts, vertically centred in r, from x, cut with "…" at maxw (0 = unclipped)
pub(crate) fn text_at(ui: &mut Ui, ts: u32, x: i32, r: [i32; 4], s: &str, argb: u32, maxw: i32) {
    let y = baseline(ui, ts, r[1], r[3]);
    let (font, size) = ui.text.get_font();
    let b = s.as_bytes();
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    if maxw > 0 && ui.text.width_n(b, n) > maxw {
        let (e, _) = ui.ellipsize(&b[..n], maxw);
        ui.cmd_text(x, y, e.as_bytes(), font, size, argb, maxw);
        return;
    }
    ui.cmd_text(x, y, &b[..n], font, size, argb, maxw);
}

/// s cut with "…" to maxw in the current font, and its width
pub(crate) fn fit(ui: &mut Ui, s: &str, maxw: i32) -> (String, i32) {
    let w = ui.text.width(s);
    if w <= maxw {
        return (String::from(s), w);
    }
    ui.ellipsize(s.as_bytes(), maxw)
}

/// an icon of size `px` centred in the rectangle
pub(crate) fn icon_in(ui: &mut Ui, cp: u32, r: [i32; 4], px: i32, argb: u32) {
    ui.draw_color(argb);
    ui.draw_icon(cp, r[0] + ((r[2] - px) >> 1), r[1] + ((r[3] - px) >> 1), px << 8);
}

/// the card colour as it shows on the window: a translucent card token (some styles layer
/// their cards) composited over the window background
pub fn card_opaque(ui: &Ui) -> u32 {
    let card = ui.color(T_CARD);
    let a = (card >> 24) as i32;
    if a >= 255 {
        return card;
    }
    color_mix(ui.color(T_BG) | 0xFF00_0000, card | 0xFF00_0000, a)
}

/// the (text, fill, solid) colours of a BADGE_* kind in the current theme
pub fn kind_colors(ui: &Ui, kind: u32) -> (u32, u32, u32) {
    let dark = ui.theme_dark();
    let card = card_opaque(ui);
    let solid = match kind {
        BADGE_INFO => ui.color(T_INFO),
        BADGE_OK => ui.color(T_OK),
        BADGE_WARN => ui.color(T_WARN),
        BADGE_CRIT => ui.color(T_CRIT),
        BADGE_ACCENT => ui.color(T_ACCENT),
        _ => ui.color(T_TEXT3),
    };
    let bg = match kind {
        BADGE_INFO => ui.color(T_INFO_BG),
        BADGE_OK => ui.color(T_OK_BG),
        BADGE_WARN => ui.color(T_WARN_BG),
        BADGE_CRIT => ui.color(T_CRIT_BG),
        BADGE_ACCENT => color_mix(card, ui.color(T_ACCENT), if dark { 72 } else { 40 }),
        _ => color_mix(card, ui.color(T_TEXT), if dark { 36 } else { 22 }),
    };
    // light theme: status colours are darkened for legible text on their light fill
    let text = if kind == BADGE_NEUTRAL {
        ui.color(T_TEXT2)
    } else if dark {
        color_mix(solid, 0xFFFF_FFFF, 48)
    } else {
        color_mix(solid, 0xFF00_0000, 72)
    };
    (text, bg | 0xFF00_0000, solid)
}

// ------------------------------------------------------------------ data views

pub(crate) fn grid_header_bg(ui: &mut Ui, sp: &Sp) {
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    ui.draw_rect(sp.x, sp.y + sp.h - h, sp.w, h);
}

pub(crate) fn grid_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    if sp.ev & (EV_HOVER | EV_HELD) != 0 {
        let c = ui.color(if sp.ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
        fill(ui, r[0], r[1], r[2], r[3] - ui.hair(), 0, c);
    }
    let inset = sp.s[0];
    let chev = ui.dp(12);
    let sorted = sp.flags & (GH_SORT_ASC | GH_SORT_DESC) != 0;
    let avail = r[2] - inset * 2 - if sorted { chev + ui.dp(4) } else { 0 };
    // a narrow column: the label cut with "…"
    ui.ts_font(TS_CAPTION);
    let (label, tw) = fit(ui, sp.label.unwrap_or(""), avail.max(1));
    let tx = if sp.flags & GH_RIGHT != 0 { r[0] + r[2] - inset - tw.min(avail) - if sorted { chev + ui.dp(4) } else { 0 } } else { r[0] + inset };
    let c = ui.color(if sorted { T_TEXT } else { T_TEXT2 });
    text_at(ui, TS_CAPTION, tx, r, &label, c, avail.max(1));
    if sorted {
        let x = tx + tw.min(avail) + ui.dp(4);
        let cp = if sp.flags & GH_SORT_DESC != 0 { IC_CHEV_DOWN_SMALL } else { IC_CHEV_UP_SMALL };
        icon_in(ui, cp, [x, r[1], chev, r[3]], chev, c);
    }
    if sp.flags & GH_LAST == 0 {
        let h = ui.hair();
        let m = r[3] / 4;
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        ui.draw_rect(r[0] + r[2] - h, r[1] + m, h, r[3] - m * 2);
    }
}

pub(crate) fn grid_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let card = card_opaque(ui);
    let sel = sp.value != 0;
    let bg = if sel {
        let a = ui.color(T_ACCENT);
        color_mix(card, a, if ui.theme_dark() { 64 } else { 36 })
    } else if sp.ev & EV_HELD != 0 {
        ui.color(T_SUBTLE_PRESSED)
    } else if sp.ev & EV_HOVER != 0 {
        ui.color(T_SUBTLE_HOVER)
    } else if sp.flags & GR_STRIPE != 0 {
        color_mix(card, ui.color(T_TEXT), 6)
    } else {
        0
    };
    fill(ui, r[0], r[1], r[2], r[3], 0, bg);
    if sel {
        let c = ui.color(T_ACCENT);
        let (pw, ph) = (ui.dp(3), ui.dp(16).min(r[3] - ui.dp(8)));
        fill(ui, r[0], r[1] + ((r[3] - ph) >> 1), pw, ph, pw >> 1, c);
    }
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
    if sp.flags & GR_FOCUS != 0 {
        let c = ui.color(T_FOCUS_OUT);
        frame(ui, r[0] + h, r[1] + h, r[2] - h * 2, r[3] - h * 2, ui.dp(2), h, c);
    }
}

pub(crate) fn tree_twisty(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    if sp.ev & EV_HOVER != 0 {
        let c = ui.color(T_SUBTLE_HOVER);
        fill(ui, r[0], r[1], r[2], r[3], ui.dp(4), c);
    }
    let cp = if sp.value != 0 { IC_CHEV_DOWN_SMALL } else { IC_CHEV_RIGHT_SMALL };
    let c = ui.color(T_TEXT2);
    let px = ui.dp(12);
    icon_in(ui, cp, r, px, c);
}

pub(crate) fn badge_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_CAPTION);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    sp.s[0] = tw;
    let dot = if sp.icon != 0 { ui.dp(14) } else { 0 };
    (tw + ui.dp(16) + dot, ui.dp_text(20))
}

pub(crate) fn badge(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (text, bg, solid) = kind_colors(ui, sp.flags & 7);
    let h = r[3].min(ui.dp_text(20));
    let y = r[1] + ((r[3] - h) >> 1);
    fill(ui, r[0], y, r[2], h, h >> 1, bg);
    let mut x = r[0] + ui.dp(8);
    if sp.icon != 0 {
        let px = ui.dp(12);
        icon_in(ui, sp.icon, [x, y, px, h], px, solid);
        x += ui.dp(14);
    }
    text_at(ui, TS_CAPTION, x, [r[0], y, r[2], h], sp.label.unwrap_or(""), text, r[0] + r[2] - x);
}

pub(crate) fn status_dot(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (_, bg, solid) = kind_colors(ui, sp.flags & 7);
    let d = r[2].min(r[3]);
    let cx = (r[0] << 8) + (r[2] << 7);
    let cy = (r[1] << 8) + (r[3] << 7);
    ui.draw_color(bg);
    ui.draw_circle(cx, cy, d << 7, 0);
    ui.draw_color(solid);
    ui.draw_circle(cx, cy, (d << 7) * 5 / 8, 0);
}

pub(crate) fn meter(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let th = r[3].min(ui.dp(8));
    let y = r[1] + ((r[3] - th) >> 1);
    let c = ui.color(T_TRACK);
    fill(ui, r[0], y, r[2], th, th >> 1, c);
    if sp.value2 <= 0 || sp.value <= 0 {
        return;
    }
    let w = ((sp.value.min(sp.value2) as i64 * r[2] as i64) / sp.value2 as i64) as i32;
    let (_, _, solid) = kind_colors(ui, sp.flags & 7);
    fill(ui, r[0], y, w.max(th), th, th >> 1, solid);
}

pub(crate) fn chart_palette(ui: &Ui) -> [u32; 8] {
    let a = ui.color(T_ACCENT);
    let t = ui.color(T_TEXT);
    [
        a,
        ui.color(T_INFO),
        ui.color(T_OK),
        ui.color(T_WARN),
        ui.color(T_CRIT),
        color_mix(a, t, 112),
        ui.color(T_TEXT3),
        color_mix(ui.color(T_INFO), ui.color(T_OK), 128),
    ]
}

// ------------------------------------------------------------------ shell

pub(crate) fn nav_pane(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = ui.color(T_LAYER);
    fill(ui, r[0], r[1], r[2], r[3], 0, c);
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]);
}

pub(crate) fn nav_item_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    if sp.flags & NAV_COMPACT != 0 {
        return (ui.dp(40), ui.dp(36));
    }
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    let bw = match sp.label2 {
        Some(b) if !b.is_empty() => {
            ui.ts_font(TS_CAPTION);
            ui.text.width(b) + ui.dp(16)
        }
        _ => 0,
    };
    sp.s[0] = tw;
    sp.s[1] = bw;
    (ui.dp(16 + 16 + 12 + 12) + tw + bw, ui.dp(36))
}

pub(crate) fn nav_item(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let m = ui.dp(4);
    let (x, y, w, h) = (r[0] + m, r[1] + ui.dp(2), r[2] - m * 2, r[3] - ui.dp(4));
    let sel = sp.value != 0;
    let bg = if sp.ev & EV_HELD != 0 {
        ui.color(T_SUBTLE_PRESSED)
    } else if sel || sp.ev & EV_HOVER != 0 {
        ui.color(T_SUBTLE_HOVER)
    } else {
        0
    };
    fill(ui, x, y, w, h, ui.dp(4), bg);
    if sel {
        let c = ui.color(T_ACCENT);
        let (pw, ph) = (ui.dp(3), ui.dp(16));
        fill(ui, x, y + ((h - ph) >> 1), pw, ph, pw >> 1, c);
    }
    let disabled = sp.state == 3;
    let fg = ui.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
    let px = ui.dp(16);
    let compact = sp.flags & NAV_COMPACT != 0;
    let ix = if compact { x + ((w - px) >> 1) } else { x + ui.dp(12) };
    if sp.icon != 0 {
        icon_in(ui, sp.icon, [ix, y, px, h], px, if sel { ui.color(T_ACCENT_TEXT) } else { fg });
    }
    // badge: a count pill at the end (compact: a dot on the icon)
    let badge = sp.label2.filter(|b| !b.is_empty());
    if compact {
        if badge.is_some() {
            let c = ui.color(T_ACCENT);
            ui.draw_color(c);
            ui.draw_circle((ix + px) << 8, (y + ui.dp(8)) << 8, ui.dp_fx(3 * 256), 0);
        }
        return;
    }
    let tx = ix + px + ui.dp(12);
    let mut end = x + w - ui.dp(8);
    if let Some(b) = badge {
        ui.ts_font(TS_CAPTION);
        // (large text: a taller pill, at least round)
        let bh = ui.dp_text(18).min(h);
        let bw = (ui.text.width(b) + ui.dp(12)).max(bh);
        end -= bw;
        let a = ui.color(T_ACCENT);
        fill(ui, end, y + ((h - bh) >> 1), bw, bh, bh >> 1, a);
        let c = ui.color(T_ON_ACCENT);
        text_at(ui, TS_CAPTION, end + ui.dp(6), [end, y, bw, h], b, c, 0);
        end -= ui.dp(6);
    }
    let ts = if sel { TS_BODY_STRONG } else { TS_BODY };
    text_at(ui, ts, tx, [x, y, w, h], sp.label.unwrap_or(""), fg, (end - tx).max(1));
}

pub(crate) fn nav_section_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    if sp.flags & NAV_COMPACT != 0 {
        return (ui.dp(40), ui.dp(13));
    }
    (ui.dp(40), ui.dp(30))
}

pub(crate) fn nav_section(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    if sp.flags & NAV_COMPACT != 0 {
        let h = ui.hair();
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        ui.draw_rect(r[0] + ui.dp(10), r[1] + ((r[3] - h) >> 1), r[2] - ui.dp(20), h);
        return;
    }
    let c = ui.color(T_TEXT3);
    let y = r[1] + ui.dp(8);
    text_at(ui, TS_CAPTION, r[0] + ui.dp(16), [r[0], y, r[2], r[3] - ui.dp(8)], sp.label.unwrap_or(""), c, r[2] - ui.dp(24));
}

pub(crate) fn command_bar(_ui: &mut Ui, _sp: &Sp) {}

pub(crate) fn split_grip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let active = sp.ev & (EV_HOVER | EV_HELD) != 0;
    let t = if active { ui.dp(2) } else { ui.hair() };
    let c = ui.color(if active { T_ACCENT } else { T_DIVIDER });
    ui.draw_color(c);
    if sp.flags & 1 != 0 {
        ui.draw_rect(r[0], r[1] + ((r[3] - t) >> 1), r[2], t);
    } else {
        ui.draw_rect(r[0] + ((r[2] - t) >> 1), r[1], t, r[3]);
    }
}

pub(crate) fn breadcrumb_sep(ui: &mut Ui, sp: &Sp) {
    let px = ui.dp(12);
    let c = ui.color(T_TEXT3);
    icon_in(ui, IC_CHEV_RIGHT_SMALL, sp.rect(), px, c);
}

pub(crate) fn segment(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(4);
    if sp.value2 == 1 {
        // the track behind every item
        let c = ui.color(T_SUBTLE_HOVER);
        let c = if c & 0xFF00_0000 == 0 { ui.color(T_CTRL_DISABLED) } else { c };
        fill(ui, r[0], r[1], r[2], r[3], rad + ui.dp(2), c);
        return;
    }
    let m = ui.dp(2);
    let (x, y, w, h) = (r[0] + m, r[1] + m, r[2] - m * 2, r[3] - m * 2);
    if sp.value != 0 {
        // light themes show the control fill solid; a dark theme's translucent white is
        // layered over the track (made opaque it turns white under the white text)
        let f = ui.color(T_CTRL);
        let f = if ui.theme_dark() { f } else { f | 0xFF00_0000 };
        let s = ui.color(T_CTRL_STROKE);
        ui.cmd_rrect(x, y, w, h, rad, ui.hair(), f, s, ui.color(T_CTRL_STROKE2));
    } else if sp.ev & EV_HOVER != 0 {
        let c = ui.color(T_SUBTLE_HOVER);
        fill(ui, x, y, w, h, rad, c);
    }
    let ts = if sp.value != 0 { TS_BODY_STRONG } else { TS_BODY };
    ui.ts_font(ts);
    // a squeezed item: its label cut with "…", centred
    let (label, tw) = fit(ui, sp.label.unwrap_or(""), (w - ui.dp(8)).max(1));
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else if sp.value != 0 { T_TEXT } else { T_TEXT2 });
    text_at(ui, ts, x + ((w - tw) >> 1).max(0), r, &label, c, w);
}

pub(crate) fn wizard_step(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let d = ui.dp(24);
    let cx = r[0] + ui.dp(12) + (d >> 1);
    let cy = r[1] + (r[3] >> 1);
    // connector to the next step
    if sp.flags & 1 != 0 {
        let t = ui.dp(2);
        let c = ui.color(if sp.value2 == WS_DONE { T_ACCENT } else { T_DIVIDER });
        ui.draw_color(c);
        ui.draw_rect(cx - (t >> 1), cy + (d >> 1) + ui.dp(2), t, r[1] + r[3] - cy - (d >> 1) + (r[3] >> 1) - ui.dp(4));
    }
    let hover = sp.ev & EV_HOVER != 0;
    if hover && sp.value2 != WS_CURRENT {
        let c = ui.color(T_SUBTLE_HOVER);
        fill(ui, r[0] + ui.dp(4), r[1] + ui.dp(2), r[2] - ui.dp(8), r[3] - ui.dp(4), ui.dp(4), c);
    }
    let (ring, inner, num) = match sp.value2 {
        WS_CURRENT => (ui.color(T_ACCENT), ui.color(T_ACCENT), ui.color(T_ON_ACCENT)),
        WS_DONE => (ui.color(T_ACCENT), color_mix(card_opaque(ui), ui.color(T_ACCENT), 40), ui.color(T_ACCENT_TEXT)),
        WS_ERROR => (ui.color(T_CRIT), ui.color(T_CRIT_BG), ui.color(T_CRIT)),
        _ => (ui.color(T_STRONG_STROKE), card_opaque(ui), ui.color(T_TEXT2)),
    };
    ui.draw_color(ring);
    ui.draw_circle(cx << 8, cy << 8, d << 7, 0);
    ui.draw_color(inner);
    ui.draw_circle(cx << 8, cy << 8, (d << 7) - (ui.hair() << 8) - 128, 0);
    let cr = [cx - (d >> 1), cy - (d >> 1), d, d];
    if sp.value2 == WS_DONE {
        let px = ui.dp(12);
        icon_in(ui, IC_CHECK_MARK, cr, px, num);
    } else if sp.value2 == WS_ERROR {
        let px = ui.dp(12);
        icon_in(ui, IC_CANCEL, cr, px, num);
    } else {
        let s = alloc::format!("{}", sp.value);
        ui.ts_font(TS_CAPTION);
        let tw = ui.text.width(&s);
        text_at(ui, TS_CAPTION, cx - (tw >> 1), cr, &s, num, 0);
    }
    let ts = if sp.value2 == WS_CURRENT { TS_BODY_STRONG } else { TS_BODY };
    let c = ui.color(match sp.value2 {
        WS_CURRENT => T_TEXT,
        WS_TODO => T_TEXT2,
        _ => T_TEXT,
    });
    let tx = cx + (d >> 1) + ui.dp(12);
    text_at(ui, ts, tx, r, sp.label.unwrap_or(""), c, r[0] + r[2] - tx - ui.dp(8));
}

pub(crate) fn toast(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let rad = ui.dp(8);
    ui.draw_color(ui.color(T_SHADOW));
    ui.cmd_shadow(r[0], r[1], r[2], r[3], rad, ui.dp(12));
    let f = ui.color(T_FLYOUT) | 0xFF00_0000;
    let s = ui.color(T_FLYOUT_STROKE);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, ui.hair(), f, s, s);
    let kind = match sp.flags & 3 {
        IB_SUCCESS => BADGE_OK,
        IB_WARNING => BADGE_WARN,
        IB_ERROR => BADGE_CRIT,
        _ => BADGE_INFO,
    };
    let (_, _, solid) = kind_colors(ui, kind);
    // severity bar and icon
    fill(ui, r[0] + ui.dp(6), r[1] + ui.dp(10), ui.dp(3), r[3] - ui.dp(20), ui.dp(1), solid);
    let icon = match sp.flags & 3 {
        IB_SUCCESS => IC_CHECK_CIRCLE_FILL,
        IB_WARNING => IC_WARNING,
        IB_ERROR => IC_ERROR_FILL,
        _ => IC_INFO,
    };
    let px = ui.dp(16);
    icon_in(ui, icon, [r[0] + ui.dp(18), r[1] + ui.dp(14), px, px], px, solid);
    let tx = r[0] + ui.dp(44);
    let cs = ui.dp(28);
    let cx = r[0] + r[2] - cs - ui.dp(8);
    let maxw = cx - tx - ui.dp(4);
    let t = ui.color(T_TEXT);
    text_at(ui, TS_BODY_STRONG, tx, [r[0], r[1] + ui.dp(10), r[2], ui.dp(24)], sp.label.unwrap_or(""), t, maxw);
    if let Some(m) = sp.label2 {
        let c = ui.color(T_TEXT2);
        text_at(ui, TS_CAPTION, tx, [r[0], r[1] + ui.dp(32), r[2], ui.dp(20)], m, c, maxw);
    }
    sp.s[0] = cx;
    sp.s[1] = cs;
}

pub(crate) fn spinner(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let d = r[2].min(r[3]);
    let cx = ((r[0] << 8) + (r[2] << 7)) as f32;
    let cy = ((r[1] << 8) + (r[3] << 7)) as f32;
    let t = (d / 8).max(ui.dp(2));
    let rad = ((d - t) << 7) as f32;
    // a quarter-to-three-quarter arc that turns with the phase
    let phase = (sp.value & 1023) as f32 / 1024.0;
    let a0 = phase * core::f32::consts::TAU;
    let track = ui.color(T_TRACK);
    let accent = ui.color(T_ACCENT);
    let n = 32;
    for k in 0..n {
        let b0 = k as f32 / n as f32 * core::f32::consts::TAU;
        let b1 = (k + 1) as f32 / n as f32 * core::f32::consts::TAU;
        let (s0, c0) = sincos(b0);
        let (s1, c1) = sincos(b1);
        ui.draw_color(track);
        ui.draw_line(rt::fx::cvt(cx + c0 * rad), rt::fx::cvt(cy + s0 * rad), rt::fx::cvt(cx + c1 * rad), rt::fx::cvt(cy + s1 * rad), t << 8);
    }
    let span = 0.30 * core::f32::consts::TAU;
    let m = 12;
    for k in 0..m {
        let b0 = a0 + span * k as f32 / m as f32;
        let b1 = a0 + span * (k + 1) as f32 / m as f32;
        let (s0, c0) = sincos(b0);
        let (s1, c1) = sincos(b1);
        ui.draw_color(accent);
        ui.draw_line(rt::fx::cvt(cx + c0 * rad), rt::fx::cvt(cy + s0 * rad), rt::fx::cvt(cx + c1 * rad), rt::fx::cvt(cy + s1 * rad), t << 8);
    }
}

pub(crate) fn progress_indeterminate(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let th = r[3].min(ui.dp(4));
    let y = r[1] + ((r[3] - th) >> 1);
    let c = ui.color(T_TRACK);
    fill(ui, r[0], y, r[2], th, th >> 1, c);
    let seg = r[2] * 3 / 10;
    let pos = ((sp.value & 1023) as i64 * (r[2] + seg) as i64 / 1024) as i32 - seg;
    let x0 = (r[0] + pos).max(r[0]);
    let x1 = (r[0] + pos + seg).min(r[0] + r[2]);
    if x1 > x0 {
        let c = ui.color(T_ACCENT);
        fill(ui, x0, y, x1 - x0, th, th >> 1, c);
    }
}

pub(crate) fn calendar_day(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let d = r[2].min(r[3]) - ui.dp(4);
    let cx = r[0] + (r[2] >> 1);
    let cy = r[1] + (r[3] >> 1);
    let sel = sp.value != 0;
    if sel {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        ui.draw_circle(cx << 8, cy << 8, d << 7, 0);
    } else if sp.ev & EV_HOVER != 0 {
        let c = ui.color(T_SUBTLE_HOVER);
        ui.draw_color(c);
        ui.draw_circle(cx << 8, cy << 8, d << 7, 0);
    }
    if sp.value2 != 0 && !sel {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        ui.draw_circle(cx << 8, cy << 8, (d << 7) - 128, ui.hair() << 8);
    }
    let label = sp.label.unwrap_or("");
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(label);
    let c = ui.color(if sel {
        T_ON_ACCENT
    } else if sp.state == 3 {
        T_TEXT_DISABLED
    } else if sp.flags & 1 != 0 {
        T_TEXT3
    } else {
        T_TEXT
    });
    text_at(ui, TS_BODY, cx - (tw >> 1), r, label, c, 0);
}

// ------------------------------------------------------------------ docking (dock.rs)

pub(crate) fn dock_group(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = ui.color(T_LAYER);
    fill(ui, r[0], r[1], r[2], r[3], 0, c);
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    frame(ui, r[0], r[1], r[2], r[3], 0, h, c);
}

pub(crate) fn dock_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
}

pub(crate) fn dock_tab_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(sp.label.unwrap_or(""));
    let icon = if sp.icon != 0 { 16 + 6 } else { 0 };
    (ui.dp(10 + 10 + icon) + tw, ui.dp(32))
}

pub(crate) fn dock_tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let m = ui.dp(2);
    let (x, y, w, h) = (r[0] + m, r[1] + ui.dp(4), r[2] - m * 2, r[3] - ui.dp(8));
    let bg = if sp.ev & EV_HELD != 0 {
        ui.color(T_SUBTLE_PRESSED)
    } else if sp.ev & EV_HOVER != 0 {
        ui.color(T_SUBTLE_HOVER)
    } else {
        0
    };
    fill(ui, x, y, w, h, ui.dp(4), bg);
    let sel = sp.value != 0;
    let fg = ui.color(if sel { T_TEXT } else { T_TEXT2 });
    let mut tx = r[0] + ui.dp(10);
    if sp.icon != 0 {
        let px = ui.dp(16);
        icon_in(ui, sp.icon, [tx, r[1], px, r[3]], px, fg);
        tx += px + ui.dp(6);
    }
    let avail = (r[0] + r[2] - ui.dp(8) - tx).max(1);
    let label = sp.label.unwrap_or("");
    ui.ts_font(TS_BODY);
    let s = if ui.text.width(label) > avail { ui.ellipsize(label.as_bytes(), avail).0 } else { alloc::string::String::from(label) };
    // too narrow for a part of the title: the icon alone
    if sp.icon == 0 || s != "…" {
        text_at(ui, TS_BODY, tx, r, &s, fg, avail);
    }
    if sel {
        // the shown tab: a bar along the bottom, accent while the group has the focus
        let t = ui.dp(2);
        let c = ui.color(if sp.value2 != 0 { T_ACCENT } else { T_TEXT3 });
        let i = ui.dp(8);
        fill(ui, r[0] + i, r[1] + r[3] - t, r[2] - i * 2, t, t >> 1, c);
    }
}

pub(crate) fn dock_rail(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = ui.color(T_LAYER);
    fill(ui, r[0], r[1], r[2], r[3], 0, c);
    let h = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    match sp.flags {
        0 => ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]),
        1 => ui.draw_rect(r[0], r[1], h, r[3]),
        _ => ui.draw_rect(r[0], r[1], r[2], h),
    }
}

pub(crate) fn dock_marker(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let hover = sp.ev & EV_HOVER != 0;
    let rad = ui.dp(6);
    let a = ui.color(T_ACCENT);
    ui.draw_color(ui.color(T_SHADOW));
    ui.cmd_shadow(r[0], r[1], r[2], r[3], rad, ui.dp(8));
    let bg = card_opaque(ui) | 0xFF00_0000;
    let stroke = if hover { a } else { ui.color(T_CTRL_STROKE) };
    let t = if hover { ui.dp(2) } else { ui.hair() };
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, bg, stroke, stroke);
    // the place in small: a frame and the part the panel takes
    let i = ui.dp(8);
    let f = [r[0] + i, r[1] + i, r[2] - i * 2, r[3] - i * 2];
    let c = ui.color(T_TEXT2);
    frame(ui, f[0], f[1], f[2], f[3], ui.dp(2), ui.hair(), c);
    let (pw, ph) = (f[2] * 2 / 5, f[3] * 2 / 5);
    let p = match sp.value {
        DOCK_MK_LEFT => [f[0], f[1], pw, f[3]],
        DOCK_MK_RIGHT => [f[0] + f[2] - pw, f[1], pw, f[3]],
        DOCK_MK_TOP => [f[0], f[1], f[2], ph],
        DOCK_MK_BOTTOM => [f[0], f[1] + f[3] - ph, f[2], ph],
        _ => {
            let m = ui.dp(3);
            [f[0] + m, f[1] + m, f[2] - m * 2, f[3] - m * 2]
        }
    };
    let fc = if hover { a } else { (a & 0x00FF_FFFF) | 0xA000_0000 };
    fill(ui, p[0], p[1], p[2], p[3], ui.dp(1), fc);
}

pub(crate) fn dock_preview(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = ui.color(T_ACCENT);
    let tint = (c & 0x00FF_FFFF) | if sp.value != 0 { 0x1800_0000 } else { 0x3000_0000 };
    let (t, rad) = (ui.dp(2), ui.dp(4));
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, tint, c, c);
}

/// (sin, cos) of an angle in radians (single precision, enough for drawing)
pub(crate) fn sincos(a: f32) -> (f32, f32) {
    let (s, c) = gfx::xform::sincos(a as f64);
    (s as f32, c as f32)
}
