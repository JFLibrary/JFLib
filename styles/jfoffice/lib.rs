// jfOffice: original jfguiLib painters using the requested WordCraft visual reference.
// All paint and metrics live in this style. Widget behavior remains in ui. Embedded
// fonts and independently drawn reference icon motifs carry their license notices.
#![no_std]
#![forbid(unsafe_code)]
mod basic;
mod chrome;
mod common;
mod edit;
mod icons;
mod list;
mod misc;
mod palette;
mod popup;
mod value;
use ui::{Style, consts::*};
/// Office-oriented style; all widget state and accessibility behavior remains in ui.
pub struct Office;
/// Linkable style instance.
pub static STYLE: Office = Office;
impl Style for Office {
    fn paint_icon(
        &self,
        g: &mut ui::Gfx,
        cp: u32,
        x: i32,
        y: i32,
        size_fx: i32,
        color: u32,
        accent: u32,
        dark: bool,
        disabled: bool,
        dpi: u32,
    ) -> bool {
        icons::paint(g, cp, x, y, size_fx, color, accent, dark, disabled, dpi)
    }
    fn ribbon_height(&self) -> i32 {
        92
    }
    fn ribbon_padding(&self) -> u32 {
        2
    }
    fn preview_card_offset(&self) -> Option<[i32; 2]> { Some([0, -128]) }
    fn ribbon_preview(&self, typography: u32) -> Option<(u32, i32, u32)> {
        let (size, color) = match typography {
            TS_SUBTITLE => (20, 0xff156082),
            TS_TITLE | TS_TITLE_LARGE | TS_DISPLAY => (26, 0xff000000),
            _ => (14, 0xff000000),
        };
        Some((FONT_REG, size * 256, color))
    }
    fn ribbon_metrics(&self) -> ui::RibbonMetrics {
        ui::RibbonMetrics {
            content_height: 66,
            caption_height: 16,
            group_gap: 14,
            content_gap: 0,
            inset_x: 8,
            inset_y: 4,
            collapse_groups: false,
        }
    }
    fn tokens(&self, dark: bool) -> [u32; T_COUNT] {
        if dark { palette::DARK } else { palette::LIGHT }
    }
    fn accent(&self, t: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
        let c = 0xff000000
            | if rgb == 0 {
                if dark { 0x7a93f0 } else { 0x3b5bdb }
            } else {
                rgb & 0xffffff
            };
        for (token, color) in [
            (T_ACCENT, c),
            (T_ACCENT_HOVER, ui::theme::color_mix(c, 0xffffffff, 24)),
            (T_ACCENT_PRESSED, ui::theme::color_mix(c, 0xff000000, 32)),
            (
                T_ACCENT_TEXT,
                if rgb == 0 {
                    if dark { 0xff9cb0f5 } else { 0xff2b47b5 }
                } else {
                    c
                },
            ),
            (T_ON_ACCENT, 0xffffffff),
            (T_FOCUS_OUT, c),
        ] {
            t[(token / 4) as usize] = color;
        }
    }
    fn ts_size(&self) -> [u8; 8] {
        [11, 12, 12, 15, 26, 28, 36, 12]
    }
    fn ts_size_fx(&self) -> [u16; 8] {
        [2816, 3200, 3200, 3840, 6656, 7168, 9216, 3200]
    }
    fn ts_line(&self) -> [u8; 8] {
        [17, 18, 18, 24, 36, 38, 46, 18]
    }
    fn ts_font(&self) -> [u8; 8] {
        [0, 0, 2, 2, 2, 2, 2, 0]
    }
    fn fonts(&self) -> [&'static [u8]; 4] {
        [
            include_bytes!("assets/Inter-Regular.ttf"),
            include_bytes!("assets/Inter-Medium.ttf"),
            include_bytes!("assets/Inter-SemiBold.ttf"),
            include_bytes!("assets/icons.bin"),
        ]
    }
    fn font_raster(&self) -> [text::FontRaster; 4] {
        use text::FontRaster::{Legacy, Natural};
        [Natural, Natural, Natural, Legacy]
    }
    fn font_names(&self) -> [[&'static str; 5]; 4] {
        [
            ["Inter", "Regular", "Inter Regular", "Inter", "Regular"],
            ["Inter", "Medium", "Inter Medium", "Inter", "Medium"],
            ["Inter", "SemiBold", "Inter SemiBold", "Inter", "SemiBold"],
            [
                "jfOffice Symbols",
                "Regular",
                "jfOffice Symbols",
                "jfOffice Symbols",
                "Regular",
            ],
        ]
    }
}
