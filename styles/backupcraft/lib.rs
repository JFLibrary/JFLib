// style_backupcraft -- the BackupCraft style: WinUI layering with KDE Breeze framing
//                                                                   [styles/backupcraft]
//
// Slate surfaces in three layers (window, panel, card) with soft shadows on flyouts and
// dialogs (WinUI), fully framed inputs and outlined push buttons that take an accent outline
// on hover, full-width tab underlines and visible scroll tracks (KDE Breeze); accent-filled
// primary buttons, check boxes and switches, a pill marking the selected row and navigation
// item, a 2 DIP focus ring outside the control (WinUI).  Petrol accent (#0B7285 light,
// #3BC9DB dark), Inter type, compact 32 DIP controls with 5 DIP corners, 8 DIP cards.
// Icons: own vector artwork (icons.rs) through ui::paint_svg_icon; the remaining code points
// come from the jfOffice symbol font.  Fonts and icons may be redistributed (OFL / own work).
//
// Files: lib.rs (tokens, accent, type ramp, fonts), palette.rs (the colour tables),
// common.rs (drawing helpers), basic.rs, value.rs, edit.rs, list.rs, popup.rs, chrome.rs,
// misc.rs (the widget families of ui::Style), data.rs (StyleData), shell.rs (StyleShell),
// icons.rs (the vector icons).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod basic;
mod chrome;
mod common;
mod data;
mod edit;
mod icons;
mod list;
mod misc;
mod palette;
mod popup;
mod shell;
mod value;

use ui::consts::*;
use ui::theme::color_mix;
use ui::{Style, StyleData, StyleShell};

/// the BackupCraft style
pub struct BackupCraft;

/// the style the app passes to ui::run
pub static STYLE: BackupCraft = BackupCraft;

impl Style for BackupCraft {
    fn tokens(&self, dark: bool) -> [u32; T_COUNT] {
        if dark { palette::DARK } else { palette::LIGHT }
    }
    fn accent(&self, theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
        accent(theme, dark, rgb)
    }
    // caption body strong subtitle title large display spare
    fn ts_size(&self) -> [u8; 8] {
        [12, 14, 14, 17, 24, 30, 40, 13]
    }
    fn ts_size_fx(&self) -> [u16; 8] {
        // 12, 13.5, 13.5, 17, 24, 30, 40, 13 DIP (24.8)
        [3072, 3456, 3456, 4352, 6144, 7680, 10240, 3328]
    }
    fn ts_line(&self) -> [u8; 8] {
        [16, 20, 20, 24, 32, 38, 50, 18]
    }
    fn ts_font(&self) -> [u8; 8] {
        [
            FONT_REG as u8, FONT_REG as u8, FONT_BOLD as u8, FONT_BOLD as u8,
            FONT_BOLD as u8, FONT_BOLD as u8, FONT_BOLD as u8, FONT_SEMI as u8,
        ]
    }
    fn fonts(&self) -> [&'static [u8]; 4] {
        // FONT_SEMI is Inter Medium, FONT_BOLD Inter SemiBold (strong text and titles)
        [
            include_bytes!("../jfoffice/assets/Inter-Regular.ttf"),
            include_bytes!("../jfoffice/assets/Inter-Medium.ttf"),
            include_bytes!("../jfoffice/assets/Inter-SemiBold.ttf"),
            include_bytes!("../jfoffice/assets/icons.bin"),
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
            ["BackupCraft Symbols", "Regular", "BackupCraft Symbols", "BackupCraft Symbols", "Regular"],
        ]
    }
    fn paint_icon(&self, g: &mut ui::Gfx, cp: u32, x: i32, y: i32, size_fx: i32, color: u32, accent: u32,
                  _dark: bool, _disabled: bool, _dpi: u32) -> bool {
        icons::paint(g, cp, x, y, size_fx, color, accent)
    }
    fn data(&self) -> &'static dyn StyleData {
        &data::DATA
    }
    fn shell(&self) -> &'static dyn StyleShell {
        &shell::SHELL
    }
}

/// the default accent of the light / dark theme
const ACCENT_LIGHT: u32 = 0x0B7285;
const ACCENT_DARK: u32 = 0x3BC9DB;

// relative luminance (0..1000) of an sRGB colour, rough (for the text on an accent fill)
fn luma(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    (r * 213 + g * 715 + b * 72) / 255
}

/// the accent tokens from 0xRRGGBB (0 = Petrol): hover a little lighter, pressed darker,
/// link text darker in the light theme / lighter in the dark one, text on the accent black
/// or white by luminance, the focus ring in the accent
fn accent(theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
    let a = match rgb & 0xFF_FFFF {
        0 => if dark { ACCENT_DARK } else { ACCENT_LIGHT },
        x => x,
    };
    let c = a | 0xFF00_0000;
    let set = |t: &mut [u32; T_COUNT], tok: u32, v: u32| t[(tok / 4) as usize] = v;
    set(theme, T_ACCENT, c);
    if dark {
        set(theme, T_ACCENT_HOVER, color_mix(c, 0xFFFF_FFFF, 30));
        set(theme, T_ACCENT_PRESSED, color_mix(c, 0xFF00_0000, 40));
        set(theme, T_ACCENT_TEXT, color_mix(c, 0xFFFF_FFFF, 56));
    } else {
        set(theme, T_ACCENT_HOVER, color_mix(c, 0xFFFF_FFFF, 26));
        set(theme, T_ACCENT_PRESSED, color_mix(c, 0xFF00_0000, 44));
        set(theme, T_ACCENT_TEXT, color_mix(c, 0xFF00_0000, 36));
    }
    let on = if luma(a) > 560 { 0xFF06_1F24 } else { 0xFFFF_FFFF };
    set(theme, T_ON_ACCENT, on);
    set(theme, T_FOCUS_OUT, c);
}
