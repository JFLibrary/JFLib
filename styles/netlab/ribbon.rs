// ribbon -- the NetLab ribbon: the band, the register row, key tips (only ribbons use these;
// JFNetworker has no ribbon, so nothing here changes how it looks)
//
//   METRICS             92 DIP: 3 DIP inset, 66 DIP of controls (three 22 DIP rows or a large
//                       button with a 32 DIP icon over two caption lines), 4 DIP gap, 16 DIP
//                       group captions; groups 8 DIP apart with a divider between them
//   band                the panel colour from edge to edge with a frame line along the bottom
//                       (no frame at the sides or the top: the register row ends in the line
//                       the selected register opens)
//   strip               the register row on the window colour, a frame line along its bottom
//                       that leaves the selected register open toward the band
//   tab                 the selected register in the panel colour, framed on three sides with
//                       a 2 DIP accent edge on top (the classic shape of NetLab's dock tabs);
//                       File on the accent; a register of a context set with a 3 DIP edge of
//                       the set's colour above it; hover a shade
//   keytip              a 16 DIP badge in the text colour with the keys in the window colour,
//                       2 DIP corners

use ui::consts::*;
use ui::{RibbonMetrics, Sp, Ui, RT_CONTEXT, RT_FILE};

use crate::common::{baseline, fill_rrect, text};

pub(crate) const METRICS: RibbonMetrics = RibbonMetrics {
    content_height: 66,
    caption_height: 16,
    group_gap: 8,
    content_gap: 0,
    inset_x: 4,
    inset_y: 3,
    collapse_groups: true,
};

/// the band's height: inset, controls, the 4 DIP gap the core keeps above the captions,
/// captions, inset
pub(crate) const HEIGHT: i32 = 3 + 66 + 4 + 16 + 3;

/// the register row's height
pub(crate) const TABS_H: i32 = 30;

// the room above a register (the context edge)
const TOP: i32 = 3;

pub(crate) fn band(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let fill = ui.color(T_LAYER);
    ui.draw_color(fill | 0xFF00_0000);
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    let h = ui.hair();
    let c = ui.color(T_LAYER_STROKE);
    ui.draw_color(c);
    ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
}

pub(crate) fn strip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let c = ui.color(T_LAYER_STROKE);
    ui.draw_color(c);
    let y = r[1] + r[3] - h;
    let (sx, sw) = (sp.s[0], sp.s[1]);
    if sw > 0 {
        ui.draw_rect(r[0], y, (sx - r[0]).max(0), h);
        ui.draw_rect(sx + sw, y, (r[0] + r[2] - sx - sw).max(0), h);
    } else {
        ui.draw_rect(r[0], y, r[2], h);
    }
}

pub(crate) fn tab(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let h = ui.hair();
    let top = ui.dp(TOP);
    let (x, y, w, bh) = (r[0], r[1] + top, r[2], r[3] - top);
    let file = sp.flags & RT_FILE != 0;
    let ctx = sp.flags & RT_CONTEXT != 0;
    let sel = sp.value != 0;
    let set = sp.value2 as u32;
    if ctx {
        // the set's edge above every register of it (side by side they make one bar)
        ui.draw_color(set | 0xFF00_0000);
        ui.draw_rect(x, r[1], w, top);
    }
    let fg;
    if file {
        let c = ui.color(match sp.state {
            1 => T_ACCENT_HOVER,
            2 => T_ACCENT_PRESSED,
            _ => T_ACCENT,
        });
        ui.draw_color(c);
        ui.draw_rect(x, y, w, bh - h);
        fg = ui.color(T_ON_ACCENT);
    } else if sel {
        // the panel colour over the row's line, framed left, top and right
        let fill = ui.color(T_LAYER) | 0xFF00_0000;
        ui.draw_color(fill);
        ui.draw_rect(x, y, w, bh);
        let c = ui.color(T_LAYER_STROKE);
        ui.draw_color(c);
        ui.draw_rect(x, y, h, bh);
        ui.draw_rect(x + w - h, y, h, bh);
        ui.draw_rect(x, y, w, h);
        let edge = if ctx { set | 0xFF00_0000 } else { ui.color(T_ACCENT) };
        ui.draw_color(edge);
        ui.draw_rect(x, y, w, ui.dp(2));
        fg = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    } else {
        if sp.state == 1 || sp.state == 2 {
            let c = ui.color(if sp.state == 2 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            fill_rrect(ui, x + h, y + ui.dp(2), w - 2 * h, bh - ui.dp(4) - h, 0, c);
        }
        fg = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    }
    let label = sp.label.unwrap_or("");
    let by = baseline(ui, TS_BODY, y, bh - h);
    ui.ts_font(TS_BODY);
    let tw = ui.text.width(label);
    text(ui, x + ((w - tw) >> 1).max(0), by, label, fg, 0);
}

pub(crate) fn keytip(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let fill = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT }) | 0xFF00_0000;
    fill_rrect(ui, r[0], r[1], r[2], r[3], ui.dp(2), fill);
    let label = sp.label.unwrap_or("");
    let by = baseline(ui, TS_CAPTION, r[1], r[3]);
    ui.ts_font(TS_CAPTION);
    let tw = ui.text.width(label);
    let c = ui.color(T_BG) | 0xFF00_0000;
    text(ui, r[0] + ((r[2] - tw) >> 1).max(0), by, label, c, 0);
}
