// data -- BackupCraft data views (ui::StyleData): 36 DIP grid header on a tinted strip, 40 DIP
// rows, status badges with a dot, a Petrol-led chart palette; the rest are the stock painters

use ui::consts::*;
use ui::theme::color_mix;
use ui::{kind_colors, Sp, Stock, StyleData, Ui};

use crate::common::{baseline, fill_rrect, text};

pub(crate) struct Data;
pub(crate) static DATA: Data = Data;

impl StyleData for Data {
    fn grid_metrics(&self, _ui: &Ui) -> (i32, i32, i32) {
        (36, 40, 12)
    }
    fn grid_header_bg(&self, ui: &mut Ui, sp: &Sp) {
        let card = ui.color(T_CARD) | 0xFF00_0000;
        let alt = ui.color(T_BG_ALT) | 0xFF00_0000;
        let c = color_mix(card, alt, if ui.theme_dark() { 96 } else { 120 });
        fill_rrect(ui, sp.x, sp.y, sp.w, sp.h, 0, c);
        let h = ui.hair();
        let d = ui.color(T_DIVIDER);
        ui.draw_color(d);
        ui.draw_rect(sp.x, sp.y + sp.h - h, sp.w, h);
    }
    fn grid_header(&self, ui: &mut Ui, sp: &Sp) {
        Stock.grid_header(ui, sp)
    }
    fn grid_row(&self, ui: &mut Ui, sp: &Sp) {
        Stock.grid_row(ui, sp)
    }
    fn badge_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_CAPTION);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        sp.s[0] = tw;
        let lead = if sp.icon != 0 { ui.dp(16) } else { ui.dp(12) };
        (tw + ui.dp(10 + 10) + lead - ui.dp(4), ui.dp_text(22))
    }
    fn badge(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let (fg, bg, solid) = kind_colors(ui, sp.flags & 7);
        let h = r[3].min(ui.dp_text(22));
        let y = r[1] + ((r[3] - h) >> 1);
        fill_rrect(ui, r[0], y, r[2], h, h >> 1, bg);
        let mut x = r[0] + ui.dp(9);
        if sp.icon != 0 {
            let px = ui.dp(12);
            ui.draw_color(solid);
            ui.draw_icon(sp.icon, x, y + ((h - px) >> 1), px << 8);
            x += ui.dp(16);
        } else {
            let d = ui.dp(6);
            ui.draw_color(solid);
            ui.draw_circle((x << 8) + (d << 7), (y << 8) + (h << 7), d << 7, 0);
            x += ui.dp(12);
        }
        let b = baseline(ui, TS_CAPTION, y, h);
        ui.ts_font(TS_CAPTION);
        text(ui, x, b, sp.label.unwrap_or(""), fg, (r[0] + r[2] - x - ui.dp(6)).max(1));
    }
    fn chart_palette(&self, ui: &Ui) -> [u32; 8] {
        let dark = ui.theme_dark();
        let a = ui.color(T_ACCENT);
        if dark {
            [a, 0xFF748FFC, 0xFF51CF66, 0xFFFFA94D, 0xFFFF6B6B, 0xFFB197FC, 0xFF868E96, 0xFF66D9E8]
        } else {
            [a, 0xFF4C6EF5, 0xFF2B9348, 0xFFE07B00, 0xFFD9383C, 0xFF845EF7, 0xFF868E96, 0xFF15AABF]
        }
    }
}
