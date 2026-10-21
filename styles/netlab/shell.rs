// shell -- NetLab app shell parts (ui::StyleShell): a framed panel-coloured navigation pane
// and command bar, the selection (16 % accent tint, 1 DIP accent frame) on navigation items,
// segments and calendar days, framed toasts with the soft flyout shadow, a framed
// indeterminate bar; classic dock groups: square panels with a 1 DIP frame, a 26 DIP tab
// strip in the window colour (SM_DOCK_HDR), the shown tab in the panel colour framed on three
// sides (a 2 DIP accent edge on top while the group has the focus), square drop markers
// without a shadow; splitter sashes with grip dots.  Wizard steps, breadcrumbs and the spinner
// are the stock painters.

use ui::consts::*;
use ui::theme::color_mix;
use ui::{kind_colors, Sp, Stock, StyleShell, Ui, DOCK_MK_BOTTOM, DOCK_MK_LEFT, DOCK_MK_RIGHT, DOCK_MK_TOP};

use crate::common::{baseline, card, fill_rrect, framed, select_colors, shadow, text, R_CTRL, R_FLYOUT};

pub(crate) struct Shell;
pub(crate) static SHELL: Shell = Shell;

const DOCK_TAB_H: i32 = 26; // the strip's height (SM_DOCK_HDR)

// text in style ts, vertically centred in r, from x, clipped at maxw (0 = unclipped)
fn text_in(ui: &mut Ui, ts: u32, x: i32, r: [i32; 4], s: &str, argb: u32, maxw: i32) {
    let y = baseline(ui, ts, r[1], r[3]);
    ui.ts_font(ts);
    text(ui, x, y, s, argb, maxw);
}

// s cut to fit maxw px in the current font, with "…" at the end when cut
fn ellipsize(ui: &mut Ui, s: &str, maxw: i32) -> alloc::string::String {
    if ui.text.width(s) <= maxw {
        return alloc::string::String::from(s);
    }
    let mut out = alloc::string::String::new();
    for (i, _) in s.char_indices().rev() {
        out.clear();
        out.push_str(&s[..i]);
        out.push('…');
        if ui.text.width(&out) <= maxw {
            return out;
        }
    }
    alloc::string::String::from("…")
}

// an icon of `px` centred in r
fn icon_in(ui: &mut Ui, cp: u32, r: [i32; 4], px: i32, argb: u32) {
    ui.draw_color(argb);
    ui.draw_icon(cp, r[0] + ((r[2] - px) >> 1), r[1] + ((r[3] - px) >> 1), px << 8);
}

impl StyleShell for Shell {
    fn nav_pane(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = ui.color(T_LAYER);
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, c);
        let h = ui.hair();
        let d = ui.color(T_LAYER_STROKE);
        ui.draw_color(d);
        ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]);
    }
    fn nav_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        if sp.flags & NAV_COMPACT != 0 {
            return (ui.dp(36), ui.dp(30));
        }
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        let bw = match sp.label2 {
            Some(b) if !b.is_empty() => {
                ui.ts_font(TS_CAPTION);
                ui.text.width(b) + ui.dp(14)
            }
            _ => 0,
        };
        sp.s[0] = tw;
        sp.s[1] = bw;
        (ui.dp(12 + 16 + 8 + 12) + tw + bw, ui.dp(30))
    }
    fn nav_item(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let m = ui.dp(4);
        let (x, y, w, h) = (r[0] + m, r[1] + ui.dp(1), r[2] - m * 2, r[3] - ui.dp(2));
        let sel = sp.value != 0;
        let rad = ui.dp(2);
        if sel {
            let base = ui.color(T_LAYER);
            let (fill, stroke) = select_colors(ui, base, sp.ev & EV_HOVER != 0);
            framed(ui, x, y, w, h, rad, fill, stroke);
        } else if sp.ev & (EV_HOVER | EV_HELD) != 0 {
            let c = ui.color(if sp.ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            fill_rrect(ui, x, y, w, h, rad, c);
        }
        let disabled = sp.state == 3;
        let fg = ui.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
        let px = ui.dp(16);
        let compact = sp.flags & NAV_COMPACT != 0;
        let ix = if compact { x + ((w - px) >> 1) } else { x + ui.dp(8) };
        if sp.icon != 0 {
            let ic = if sel && !disabled { ui.color(T_ACCENT_TEXT) } else { fg };
            icon_in(ui, sp.icon, [ix, y, px, h], px, ic);
        }
        // badge: a count box at the end (compact: a square mark on the icon)
        let badge = sp.label2.filter(|b| !b.is_empty());
        if compact {
            if badge.is_some() {
                let c = ui.color(T_ACCENT);
                let d = ui.dp(6);
                fill_rrect(ui, ix + px - (d >> 1), y + ui.dp(5), d, d, ui.dp(1), c);
            }
            return;
        }
        let tx = ix + px + ui.dp(8);
        let mut end = x + w - ui.dp(6);
        if let Some(b) = badge {
            ui.ts_font(TS_CAPTION);
            let bw = ui.text.width(b) + ui.dp(10);
            let bh = ui.dp(16);
            end -= bw;
            let a = ui.color(T_ACCENT);
            fill_rrect(ui, end, y + ((h - bh) >> 1), bw, bh, ui.dp(2), a);
            let c = ui.color(T_ON_ACCENT);
            text_in(ui, TS_CAPTION, end + ui.dp(5), [end, y, bw, h], b, c, 0);
            end -= ui.dp(6);
        }
        text_in(ui, TS_BODY, tx, [x, y, w, h], sp.label.unwrap_or(""), fg, (end - tx).max(1));
    }
    fn nav_section_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        if sp.flags & NAV_COMPACT != 0 {
            return (ui.dp(36), ui.dp(9));
        }
        (ui.dp(36), ui.dp(26))
    }
    fn nav_section(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let h = ui.hair();
        if sp.flags & NAV_COMPACT != 0 {
            let c = ui.color(T_DIVIDER);
            ui.draw_color(c);
            ui.draw_rect(r[0] + ui.dp(8), r[1] + ((r[3] - h) >> 1), r[2] - ui.dp(16), h);
            return;
        }
        let c = ui.color(T_TEXT3);
        let y = r[1] + ui.dp(6);
        text_in(ui, TS_CAPTION, r[0] + ui.dp(12), [r[0], y, r[2], r[3] - ui.dp(6)], sp.label.unwrap_or(""), c, r[2] - ui.dp(20));
    }
    fn command_bar(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = ui.color(T_LAYER);
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, c);
        let h = ui.hair();
        let d = ui.color(T_LAYER_STROKE);
        ui.draw_color(d);
        ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
    }
    fn split_grip(&self, ui: &mut Ui, sp: &Sp) {
        // a sash: at rest three grip dots in its middle (the panels beside it carry their own
        // frames), a 2 DIP accent line while pointed at or dragged
        let r = sp.rect();
        let vert = sp.flags & 1 != 0;
        if sp.ev & (EV_HOVER | EV_HELD) != 0 {
            let t = ui.dp(2);
            let c = ui.color(T_ACCENT);
            ui.draw_color(c);
            if vert {
                ui.draw_rect(r[0], r[1] + ((r[3] - t) >> 1), r[2], t);
            } else {
                ui.draw_rect(r[0] + ((r[2] - t) >> 1), r[1], t, r[3]);
            }
            return;
        }
        let d = ui.dp(2);
        let step = ui.dp(5);
        let c = ui.color(T_TEXT3);
        ui.draw_color(c);
        let (cx, cy) = (r[0] + ((r[2] - d) >> 1), r[1] + ((r[3] - d) >> 1));
        for k in -1..=1 {
            if vert {
                ui.draw_rect(cx + k * step, cy, d, d);
            } else {
                ui.draw_rect(cx, cy + k * step, d, d);
            }
        }
    }
    fn breadcrumb_sep(&self, ui: &mut Ui, sp: &Sp) {
        Stock.breadcrumb_sep(ui, sp)
    }
    fn segment(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_CTRL);
        if sp.value2 == 1 {
            // the frame behind every item
            let f = ui.color(T_CTRL) | 0xFF00_0000;
            let s = ui.color(T_CTRL_STROKE);
            ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, ui.hair(), f, s, ui.color(T_CTRL_STROKE2));
            return;
        }
        let m = ui.dp(2);
        let (x, y, w, h) = (r[0] + m, r[1] + m, r[2] - m * 2, r[3] - m * 2);
        if sp.value != 0 {
            let base = ui.color(T_CTRL);
            let (fill, stroke) = select_colors(ui, base, sp.ev & EV_HOVER != 0);
            framed(ui, x, y, w, h, ui.dp(2), fill, stroke);
        } else if sp.ev & EV_HOVER != 0 {
            let c = ui.color(T_SUBTLE_HOVER);
            fill_rrect(ui, x, y, w, h, ui.dp(2), c);
        }
        let label = sp.label.unwrap_or("");
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(label);
        let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else if sp.value != 0 { T_TEXT } else { T_TEXT2 });
        text_in(ui, TS_BODY, x + ((w - tw) >> 1).max(0), r, label, c, w);
    }
    fn wizard_step(&self, ui: &mut Ui, sp: &Sp) {
        Stock.wizard_step(ui, sp)
    }
    fn toast(&self, ui: &mut Ui, sp: &mut Sp) {
        let r = sp.rect();
        let rad = ui.dp(R_FLYOUT);
        shadow(ui, r, rad, ui.dp(10), 0x22, 0x70);
        let f = ui.color(T_FLYOUT) | 0xFF00_0000;
        let s = ui.color(T_FLYOUT_STROKE);
        framed(ui, r[0], r[1], r[2], r[3], rad, f, s);
        let kind = match sp.flags & 3 {
            IB_SUCCESS => BADGE_OK,
            IB_WARNING => BADGE_WARN,
            IB_ERROR => BADGE_CRIT,
            _ => BADGE_INFO,
        };
        let (_, _, solid) = kind_colors(ui, kind);
        let icon = match sp.flags & 3 {
            IB_SUCCESS => ui::keys::IC_CHECK_CIRCLE_FILL,
            IB_WARNING => ui::keys::IC_WARNING,
            IB_ERROR => ui::keys::IC_ERROR_FILL,
            _ => ui::keys::IC_INFO,
        };
        let px = ui.dp(16);
        icon_in(ui, icon, [r[0] + ui.dp(12), r[1] + ui.dp(12), px, px], px, solid);
        let tx = r[0] + ui.dp(38);
        let cs = ui.dp(24);
        let cx = r[0] + r[2] - cs - ui.dp(6);
        let maxw = cx - tx - ui.dp(4);
        let t = ui.color(T_TEXT);
        text_in(ui, TS_BODY_STRONG, tx, [r[0], r[1] + ui.dp(8), r[2], ui.dp(24)], sp.label.unwrap_or(""), t, maxw);
        if let Some(m) = sp.label2 {
            let c = ui.color(T_TEXT2);
            text_in(ui, TS_CAPTION, tx, [r[0], r[1] + ui.dp(30), r[2], ui.dp(20)], m, c, maxw);
        }
        sp.s[0] = cx;
        sp.s[1] = cs;
    }
    fn spinner(&self, ui: &mut Ui, sp: &Sp) {
        Stock.spinner(ui, sp)
    }
    fn progress_indeterminate(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let th = r[3].min(ui.dp(8));
        let y = r[1] + ((r[3] - th) >> 1);
        let field = card(ui);
        let stroke = ui.color(T_CTRL_STROKE);
        framed(ui, r[0], y, r[2], th, ui.dp(2), field, stroke);
        let h = ui.hair();
        let (ix, iw) = (r[0] + h, r[2] - h * 2);
        let seg = iw * 3 / 10;
        let pos = ((sp.value & 1023) as i64 * (iw + seg) as i64 / 1024) as i32 - seg;
        let x0 = (ix + pos).max(ix);
        let x1 = (ix + pos + seg).min(ix + iw);
        if x1 > x0 {
            let c = ui.color(T_ACCENT);
            fill_rrect(ui, x0, y + h, x1 - x0, th - h * 2, ui.dp(1), c);
        }
    }
    fn calendar_day(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let m = ui.dp(2);
        let (x, y, w, h) = (r[0] + m, r[1] + m, r[2] - m * 2, r[3] - m * 2);
        let sel = sp.value != 0;
        let rad = ui.dp(2);
        if sel {
            let c = ui.color(T_ACCENT);
            fill_rrect(ui, x, y, w, h, rad, c);
        } else if sp.ev & EV_HOVER != 0 {
            let c = ui.color(T_SUBTLE_HOVER);
            fill_rrect(ui, x, y, w, h, rad, c);
        }
        if sp.value2 != 0 && !sel {
            let c = ui.color(T_ACCENT);
            framed(ui, x, y, w, h, rad, 0, c);
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
        text_in(ui, TS_BODY, r[0] + ((r[2] - tw) >> 1), r, label, c, 0);
    }
    fn dock_group(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let f = ui.color(T_LAYER) | 0xFF00_0000;
        let s = ui.color(T_LAYER_STROKE);
        framed(ui, r[0], r[1], r[2], r[3], 0, f, s);
    }
    fn dock_header(&self, ui: &mut Ui, sp: &Sp) {
        // the strip in the window colour inside the group's frame, a frame line along its
        // bottom (the shown tab covers it)
        let r = sp.rect();
        let h = ui.hair();
        let c = ui.color(T_BG) | 0xFF00_0000;
        fill_rrect(ui, r[0] + h, r[1] + h, r[2] - h * 2, r[3] - h * 2, 0, c);
        let s = ui.color(T_LAYER_STROKE);
        ui.draw_color(s);
        ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
    }
    fn dock_tab_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        let icon = if sp.icon != 0 { 16 + 6 } else { 0 };
        (ui.dp(10 + 10 + icon) + tw, ui.dp(DOCK_TAB_H))
    }
    fn dock_tab(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let h = ui.hair();
        let sel = sp.value != 0;
        if sel {
            // the shown tab: the panel colour over the strip's bottom line, framed on three
            // sides; while the group has the focus its top edge is a 2 DIP accent bar
            let f = ui.color(T_LAYER) | 0xFF00_0000;
            fill_rrect(ui, r[0], r[1], r[2], r[3], 0, f);
            let s = ui.color(T_LAYER_STROKE);
            ui.draw_color(s);
            ui.draw_rect(r[0], r[1], h, r[3]);
            ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]);
            if sp.value2 != 0 {
                let a = ui.color(T_ACCENT);
                ui.draw_color(a);
                ui.draw_rect(r[0], r[1], r[2], ui.dp(2));
            } else {
                ui.draw_rect(r[0], r[1], r[2], h);
            }
        } else if sp.ev & (EV_HOVER | EV_HELD) != 0 {
            let c = ui.color(if sp.ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            fill_rrect(ui, r[0] + h, r[1] + ui.dp(2), r[2] - h * 2, r[3] - ui.dp(2) - h, 0, c);
        }
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
        let s = ellipsize(ui, label, avail);
        // too narrow for a part of the title: the icon alone
        if sp.icon == 0 || s != "…" {
            text_in(ui, TS_BODY, tx, r, &s, fg, avail);
        }
    }
    fn dock_rail(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = ui.color(T_LAYER);
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, c);
        let h = ui.hair();
        let c = ui.color(T_LAYER_STROKE);
        ui.draw_color(c);
        match sp.flags {
            0 => ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]),
            1 => ui.draw_rect(r[0], r[1], h, r[3]),
            _ => ui.draw_rect(r[0], r[1], r[2], h),
        }
    }
    fn dock_marker(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let hover = sp.ev & EV_HOVER != 0;
        let rad = ui.dp(R_CTRL);
        let a = ui.color(T_ACCENT);
        let bg = card(ui);
        let stroke = if hover { a } else { ui.color(T_STRONG_STROKE) };
        let t = if hover { ui.dp(2) } else { ui.hair() };
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, bg, stroke, stroke);
        // the place in small: a frame and the part the panel takes
        let i = ui.dp(7);
        let f = [r[0] + i, r[1] + i, r[2] - i * 2, r[3] - i * 2];
        let c = ui.color(T_TEXT2);
        framed(ui, f[0], f[1], f[2], f[3], 0, 0, c);
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
        let fc = if hover { a } else { color_mix(bg, a | 0xFF00_0000, 150) };
        fill_rrect(ui, p[0], p[1], p[2], p[3], 0, fc);
    }
    fn dock_preview(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = ui.color(T_ACCENT);
        let tint = (c & 0x00FF_FFFF) | if sp.value != 0 { 0x1800_0000 } else { 0x2A00_0000 };
        let t = ui.dp(2);
        ui.cmd_rrect(r[0], r[1], r[2], r[3], 0, t, tint, c, c);
    }
}
