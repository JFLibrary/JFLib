// data -- NetLab data views (ui::StyleData): a 26 DIP grid header in the panel colour with
// column lines, 22 DIP rows with grid lines, the selection as a 16 % accent tint with a 1 DIP
// accent frame, square corners; triangles as tree twisties, framed rectangular badges and
// meters, a muted chart palette

use ui::consts::*;
use ui::keys::{IC_CARET_DOWN, IC_CARET_RIGHT, IC_CHEV_DOWN_SMALL, IC_CHEV_UP_SMALL};
use ui::theme::color_mix;
use ui::{kind_colors, Sp, StyleData, Ui};

use crate::common::{baseline, card, fill_rrect, framed, select_colors, text};

pub(crate) struct Data;
pub(crate) static DATA: Data = Data;

/// the chart series (light theme); the dark theme lifts them a quarter toward white
const CHART: [u32; 8] = [
    0xFF3E6E9E, 0xFFC9792B, 0xFF3D8C84, 0xFFB0453F, 0xFF7A5C99, 0xFF7F8C3A, 0xFF8C6A4F, 0xFF6C757D,
];

impl StyleData for Data {
    fn grid_metrics(&self, _ui: &Ui) -> (i32, i32, i32) {
        (26, 22, 8)
    }
    fn grid_header_bg(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_LAYER) | 0xFF00_0000;
        fill_rrect(ui, sp.x, sp.y, sp.w, sp.h, 0, c);
        let h = ui.hair();
        let d = ui.color(T_CTRL_STROKE);
        ui.draw_color(d);
        ui.draw_rect(sp.x, sp.y + sp.h - h, sp.w, h);
    }
    fn grid_header(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let h = ui.hair();
        if sp.ev & (EV_HOVER | EV_HELD) != 0 {
            let c = ui.color(if sp.ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            fill_rrect(ui, r[0], r[1], r[2], r[3] - h, 0, c);
        }
        let inset = sp.s[0];
        let label = sp.label.unwrap_or("");
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(label);
        let chev = ui.dp(10);
        let sorted = sp.flags & (GH_SORT_ASC | GH_SORT_DESC) != 0;
        let extra = if sorted { chev + ui.dp(4) } else { 0 };
        let avail = r[2] - inset * 2 - extra;
        let tx = if sp.flags & GH_RIGHT != 0 { r[0] + r[2] - inset - tw.min(avail) - extra } else { r[0] + inset };
        let c = ui.color(if sorted { T_TEXT } else { T_TEXT2 });
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        ui.ts_font(TS_BODY);
        text(ui, tx, y, label, c, avail.max(1));
        if sorted {
            let x = tx + tw.min(avail) + ui.dp(4);
            let cp = if sp.flags & GH_SORT_DESC != 0 { IC_CHEV_DOWN_SMALL } else { IC_CHEV_UP_SMALL };
            ui.draw_color(c);
            ui.draw_icon(cp, x, r[1] + ((r[3] - chev) >> 1), chev << 8);
        }
        if sp.flags & GH_LAST == 0 {
            // a column line over the full height of the strip
            let c = ui.color(T_DIVIDER);
            ui.draw_color(c);
            ui.draw_rect(r[0] + r[2] - h, r[1] + ui.dp(3), h, r[3] - ui.dp(3) - h);
        }
    }
    fn grid_row(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let base = card(ui);
        let h = ui.hair();
        let sel = sp.value != 0;
        let bg = if sel {
            0
        } else if sp.ev & EV_HELD != 0 {
            ui.color(T_SUBTLE_PRESSED)
        } else if sp.ev & EV_HOVER != 0 {
            ui.color(T_SUBTLE_HOVER)
        } else if sp.flags & GR_STRIPE != 0 {
            color_mix(base, ui.color(T_TEXT) | 0xFF00_0000, 7)
        } else {
            0
        };
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, bg);
        // the grid line under the row
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        ui.draw_rect(r[0], r[1] + r[3] - h, r[2], h);
        if sel {
            let (fill, stroke) = select_colors(ui, base, sp.ev & EV_HOVER != 0);
            framed(ui, r[0], r[1], r[2], r[3], 0, fill, stroke);
        }
        if sp.flags & GR_FOCUS != 0 {
            // the focus: a 1 DIP accent frame (inside the selection frame)
            let c = ui.color(T_FOCUS_OUT);
            let i = if sel { h } else { 0 };
            framed(ui, r[0] + i, r[1] + i, r[2] - i * 2, r[3] - i * 2, 0, 0, c);
        }
    }
    fn tree_twisty(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        if sp.ev & EV_HOVER != 0 {
            let c = ui.color(T_SUBTLE_HOVER);
            fill_rrect(ui, r[0], r[1], r[2], r[3], ui.dp(2), c);
        }
        let cp = if sp.value != 0 { IC_CARET_DOWN } else { IC_CARET_RIGHT };
        let c = ui.color(if sp.ev & EV_HOVER != 0 { T_TEXT } else { T_TEXT2 });
        let px = ui.dp(14);
        ui.draw_color(c);
        ui.draw_icon(cp, r[0] + ((r[2] - px) >> 1), r[1] + ((r[3] - px) >> 1), px << 8);
    }
    fn badge_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_CAPTION);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        sp.s[0] = tw;
        let lead = if sp.icon != 0 { ui.dp(14) } else { 0 };
        (tw + ui.dp(12) + lead, ui.dp_text(18))
    }
    fn badge(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let (fg, bg, solid) = kind_colors(ui, sp.flags & 7);
        let h = r[3].min(ui.dp_text(18));
        let y = r[1] + ((r[3] - h) >> 1);
        let edge = color_mix(bg, solid | 0xFF00_0000, 96);
        framed(ui, r[0], y, r[2], h, ui.dp(2), bg, edge);
        let mut x = r[0] + ui.dp(6);
        if sp.icon != 0 {
            let px = ui.dp(12);
            ui.draw_color(solid);
            ui.draw_icon(sp.icon, x, y + ((h - px) >> 1), px << 8);
            x += ui.dp(14);
        }
        let b = baseline(ui, TS_CAPTION, y, h);
        ui.ts_font(TS_CAPTION);
        text(ui, x, b, sp.label.unwrap_or(""), fg, (r[0] + r[2] - x - ui.dp(4)).max(1));
    }
    fn status_dot(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let (_, _, solid) = kind_colors(ui, sp.flags & 7);
        let d = r[2].min(r[3]).min(ui.dp(10));
        let cx = (r[0] << 8) + (r[2] << 7);
        let cy = (r[1] << 8) + (r[3] << 7);
        ui.draw_color(solid);
        ui.draw_circle(cx, cy, d << 7, 0);
        let ring = color_mix(solid | 0xFF00_0000, 0xFF00_0000, 64);
        ui.draw_color(ring);
        let t = ui.hair() << 8;
        ui.draw_circle(cx, cy, (d << 7) - (t >> 1), t);
    }
    fn meter(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let th = r[3].min(ui.dp(8));
        let y = r[1] + ((r[3] - th) >> 1);
        let field = card(ui);
        let stroke = ui.color(T_CTRL_STROKE);
        framed(ui, r[0], y, r[2], th, ui.dp(2), field, stroke);
        if sp.value2 <= 0 || sp.value <= 0 {
            return;
        }
        let h = ui.hair();
        let iw = r[2] - h * 2;
        let w = ((sp.value.min(sp.value2) as i64 * iw as i64) / sp.value2 as i64) as i32;
        let (_, _, solid) = kind_colors(ui, sp.flags & 7);
        fill_rrect(ui, r[0] + h, y + h, w.max(h), th - h * 2, ui.dp(1), solid);
    }
    fn chart_palette(&self, ui: &Ui) -> [u32; 8] {
        if ui.theme_dark() {
            CHART.map(|c| color_mix(c, 0xFFFF_FFFF, 64))
        } else {
            CHART
        }
    }
}

