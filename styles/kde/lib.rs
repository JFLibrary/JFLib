// style_kde -- the KDE Plasma (Breeze) style: tokens, type ramp, fonts, paint functions
//                                                                         [styles/kde]
//
// Light buttons with an outline, highlight-blue hover and focus frames, Breeze window
// decoration (chevron buttons, round red close on hover), dialog buttons right-aligned with
// the default first; DejaVu Sans.  `STYLE` implements ui::Style; an app is linked with it by
// `build_rs.py --style kde` and passes `&style::STYLE` to ui::run.  The w_* helpers of the
// core (ui_list.s) the widget files share are in common.rs.
#![no_std]
#![forbid(unsafe_code)]

mod basic;
mod chrome;
mod common;
mod edit;
mod list;
mod misc;
mod popup;
mod value;

use ui::consts::*;
use ui::theme::color_mix;
use ui::Style;

pub struct Kde;

/// the style the app passes to ui::run
pub static STYLE: Kde = Kde;

impl Style for Kde {
    fn tokens(&self, dark: bool) -> [u32; T_COUNT] {
        if dark { TOKENS_DARK } else { TOKENS_LIGHT }
    }
    fn accent(&self, theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
        accent(theme, dark, rgb)
    }
    fn ts_size(&self) -> [u8; 8] {
        TS_SIZE
    }
    fn ts_line(&self) -> [u8; 8] {
        TS_LINE
    }
    fn ts_font(&self) -> [u8; 8] {
        TS_FONT
    }
    fn fonts(&self) -> [&'static [u8]; 4] {
        // no semibold face: FONT_SEMI is the bold one (fonts.s)
        [FONT_REG_DATA, FONT_BOLD_DATA, FONT_BOLD_DATA, FONT_ICON_DATA]
    }
    fn font_names(&self) -> [[&'static str; 5]; 4] {
        FONT_NAMES
    }
}

// ---------------------------------------------------------------- fonts (styles/kde/fonts.s)
// DejaVu Sans regular / bold (the classic KDE / Linux desktop face, ASMLIB/docs/FONTS.md) and
// the WinUI icon set
static FONT_REG_DATA: &[u8] = include_bytes!("assets/font_reg.bin");
static FONT_BOLD_DATA: &[u8] = include_bytes!("assets/font_bold.bin");
static FONT_ICON_DATA: &[u8] = include_bytes!("../winui/assets/icons.bin");
// family, style, full, typographic family, typographic style  [fonts.s sty_font_names]
static FONT_NAMES: [[&str; 5]; 4] = [
    ["DejaVu Sans", "Book", "DejaVu Sans", "DejaVu Sans", "Book"],
    ["DejaVu Sans", "Bold", "DejaVu Sans Bold", "DejaVu Sans", "Bold"],
    ["DejaVu Sans", "Bold", "DejaVu Sans Bold", "DejaVu Sans", "Bold"],
    ["Fluent System Icons", "Regular", "Fluent System Icons", "Fluent System Icons", "Regular"],
];

// ---------------------------------------------------------------- type ramp (theme.s)
// 13 px body, bold titles
//                        caption body strong subtitle title large display
const TS_SIZE: [u8; 8] = [11, 13, 13, 16, 20, 26, 34, 13];
const TS_LINE: [u8; 8] = [15, 18, 18, 22, 28, 34, 44, 18];
const TS_FONT: [u8; 8] = [
    FONT_REG as u8, FONT_REG as u8, FONT_SEMI as u8, FONT_SEMI as u8,
    FONT_SEMI as u8, FONT_SEMI as u8, FONT_SEMI as u8, FONT_REG as u8,
];

// ---------------------------------------------------------------- tokens (T_* order)
// Breeze / Breeze Dark: grey window, light buttons with an outline, blue hover and focus frames
const TOKENS_LIGHT: [u32; T_COUNT] = [
    0xFFEFF0F1, 0xFFE3E5E7, 0xFFDEE0E2, 0x26232629, // bg, bg alt, layer, layer stroke
    0x0D232629, 0x26232629, // card, card stroke
    0xFFFCFCFC, 0xFFFCFCFC, 0xFFD3E9F7, 0xFFF2F3F4, // control rest/hover/pressed/disabled
    0xFFB9BBBD, 0xFFAAACAE, 0xFFA2A5A8, // ctrl stroke, bottom stroke, strong stroke
    0x00000000, 0x403DAEE9, 0x803DAEE9, 0x00000000, // subtle rest/hover/pressed/disabled
    0xFF232629, 0xFF707A84, 0xFF9DA2A6, 0xFFA0A2A4, // text primary/secondary/tertiary/disabled
    0, 0, 0, 0xFFB0C4D0, // accent (accent()), accent disabled
    0xFFFFFFFF, 0xFFE0E0E0, 0, // on accent, on accent disabled, accent text
    0xFF3DAEE9, 0xFFFFFFFF, // focus outer, inner
    0xFFFFFFFF, 0x40232629, 0x40000000, 0x26232629, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFDA4453, 0xFF27AE60, 0xFFF67400, 0xFF3DAEE9, // critical, success, caution, info
    0xFFF9DDE0, 0xFFD6F2E1, 0xFFFDE7D2, 0xFFD8EEFA, // their backgrounds
    0xFFDA4453, 0x40000000, 0x66232629, 0x33232629, // close hover, shadow, scroll thumb, track
    0xFFFCFCFC, 0xFFFFFFFF, 0xFF9DA2A6, // knob, alt fill, strong fill
];
const TOKENS_DARK: [u32; T_COUNT] = [
    0xFF31363B, 0xFF2A2E32, 0xFF2A2E32, 0x40000000,
    0x0DFFFFFF, 0x26EFF0F1,
    0xFF3B4045, 0xFF3B4045, 0xFF2B5A73, 0xFF33383D,
    0xFF5E6266, 0xFF55595D, 0xFF6E7276,
    0x00000000, 0x403DAEE9, 0x803DAEE9, 0x00000000,
    0xFFEFF0F1, 0xFFA3A8AD, 0xFF7F8C8D, 0xFF6E7378,
    0, 0, 0, 0xFF3A5566,
    0xFFFFFFFF, 0xFFA0A0A0, 0,
    0xFF3DAEE9, 0xFF000000,
    0xFF232629, 0xFF4D5357, 0x66000000, 0x33EFF0F1,
    0xFF232629,
    0xFFDA4453, 0xFF27AE60, 0xFFF67400, 0xFF3DAEE9,
    0xFF4D2A30, 0xFF1F3D2C, 0xFF4A3420, 0xFF1F3A4A,
    0xFFDA4453, 0x80000000, 0x80EFF0F1, 0x33EFF0F1,
    0xFF41464B, 0xFF232629, 0xFF7F8C8D,
];

/// sty_accent: T_ACCENT (Breeze highlight blue #3DAEE9 or the app's accent), hover 15 %
/// lighter, pressed 15 % darker, T_ACCENT_TEXT (links: light theme 25 % darker)
fn accent(theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
    let mut a = rgb & 0xFF_FFFF;
    if a == 0 {
        a = 0x3DAEE9;
    }
    let c = a | 0xFF00_0000;
    theme[(T_ACCENT / 4) as usize] = c;
    theme[(T_ACCENT_HOVER / 4) as usize] = color_mix(c, 0xFFFF_FFFF, 38);
    theme[(T_ACCENT_PRESSED / 4) as usize] = color_mix(c, 0xFF00_0000, 38);
    let mut t = c;
    if !dark {
        t = color_mix(c, 0xFF00_0000, 64);
    }
    theme[(T_ACCENT_TEXT / 4) as usize] = t;
}
