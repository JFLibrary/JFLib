// style_netlab -- the NetLab style: sober grey surfaces, clear frames, compact and timeless
//                                                                        [styles/netlab]
//
// The look of JFNetworker.  Three flat grey layers (window, panel, field) without gradients
// or acrylic; every control, card and dock group carries a 1 DIP solid frame.  Corners are
// small: 3 DIP on controls, 4 DIP on flyouts and dialogs, none on dock panels and grids.
// Only flyouts and dialogs cast one soft shadow.  Selection is a 16 % accent tint with a
// 1 DIP accent frame, the keyboard focus a 1 DIP accent ring 1 DIP outside the control, tabs
// carry a 2 DIP accent underline and dock tabs the classic shape (the shown tab in the panel
// colour, framed on three sides).  "Signalblau" accent (#1F5FA8 light, #6FA3E0 dark), Inter
// type at 13 DIP, compact 28 DIP controls and 22 DIP grid rows.
// Icons: own vector artwork (icons.rs, made by mkicons.py) through ui::paint_svg_icon; the
// remaining code points come from the jfOffice symbol font.  Fonts and icons may be
// redistributed (OFL / own work).
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

/// the NetLab style
pub struct Netlab;

/// the style the app passes to ui::run
pub static STYLE: Netlab = Netlab;

impl Style for Netlab {
    fn name(&self) -> &'static str {
        "netlab"
    }
    fn tokens(&self, dark: bool) -> [u32; T_COUNT] {
        if dark { palette::DARK } else { palette::LIGHT }
    }
    fn accent(&self, theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
        accent(theme, dark, rgb)
    }
    // caption body strong subtitle title large display spare
    fn ts_size(&self) -> [u8; 8] {
        [11, 13, 13, 15, 20, 26, 36, 12]
    }
    fn ts_line(&self) -> [u8; 8] {
        [15, 18, 18, 20, 26, 32, 44, 16]
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
            ["NetLab Symbols", "Regular", "NetLab Symbols", "NetLab Symbols", "Regular"],
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

/// the default accent of the light / dark theme ("Signalblau")
const ACCENT_LIGHT: u32 = 0x1F5FA8;
const ACCENT_DARK: u32 = 0x6FA3E0;

// relative luminance (0..1000) of an sRGB colour, rough (for the text on an accent fill)
fn luma(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    (r * 213 + g * 715 + b * 72) / 255
}

/// the accent tokens from 0xRRGGBB (0 = Signalblau): hover a little lighter, pressed darker,
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
        set(theme, T_ACCENT_HOVER, color_mix(c, 0xFFFF_FFFF, 28));
        set(theme, T_ACCENT_PRESSED, color_mix(c, 0xFF00_0000, 40));
        set(theme, T_ACCENT_TEXT, color_mix(c, 0xFFFF_FFFF, 40));
    } else {
        set(theme, T_ACCENT_HOVER, color_mix(c, 0xFFFF_FFFF, 24));
        set(theme, T_ACCENT_PRESSED, color_mix(c, 0xFF00_0000, 40));
        set(theme, T_ACCENT_TEXT, color_mix(c, 0xFF00_0000, 24));
    }
    let on = if luma(a) > 560 { 0xFF10_161D } else { 0xFFFF_FFFF };
    set(theme, T_ON_ACCENT, on);
    set(theme, T_FOCUS_OUT, c);
}
