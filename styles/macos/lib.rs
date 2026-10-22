// style_macos -- the macOS (Sonoma look) style: tokens, type ramp, fonts, paint functions
//                                                                       [styles/macos]
// `STYLE` implements ui::Style; the app is linked with one style crate (build_rs.py --style
// macos) and passes `&style::STYLE` to ui::run.  Grey window, white push buttons, blue
// default button and selection, pop-up buttons with the blue chevron square, overlay
// scrollers, traffic-light title bars (close at the left, centred titles), dialog buttons
// right-aligned with the default last.  JFLib Sans stands in for SF Pro (the faces and the
// icon set are winui's assets, no second copy).
#![no_std]
#![forbid(unsafe_code)]

mod basic;
mod chrome;
mod edit;
mod list;
mod misc;
mod popup;
mod value;

use ui::consts::*;
use ui::theme::color_mix;
use ui::Style;

pub struct MacOs;

/// the style the app passes to ui::run
pub static STYLE: MacOs = MacOs;

impl Style for MacOs {
    fn name(&self) -> &'static str {
        "macos"
    }
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
        [FONT_REG_BIN, FONT_SEMI_BIN, FONT_BOLD_BIN, FONT_ICON_BIN]
    }
    fn font_names(&self) -> [[&'static str; 5]; 4] {
        FONT_NAMES
    }
}

// ---------------------------------------------------------------- fonts (styles/macos/fonts.s)
// SF Pro is not freely available: winui's embedded faces, JFLib Sans regular / semibold / bold
// (Selawik subsets with ∞ ● № from Inter; the licences are copied to assets/OFL-Selawik.txt and
// assets/OFL-Inter.txt because build_rs.py ships only a style's own licence files), and the
// WinUI icon set (styles/winui/assets, shared)
static FONT_REG_BIN: &[u8] = include_bytes!("../winui/assets/font_reg.bin");
static FONT_SEMI_BIN: &[u8] = include_bytes!("../winui/assets/font_semi.bin");
static FONT_BOLD_BIN: &[u8] = include_bytes!("../winui/assets/font_bold.bin");
static FONT_ICON_BIN: &[u8] = include_bytes!("../winui/assets/icons.bin");
// family, style, full, typographic family, typographic style  [fonts.s sty_font_names]
static FONT_NAMES: [[&str; 5]; 4] = [
    ["JFLib Sans", "Regular", "JFLib Sans", "JFLib Sans", "Regular"],
    ["JFLib Sans Semibold", "Regular", "JFLib Sans Semibold", "JFLib Sans", "Semibold"],
    ["JFLib Sans", "Bold", "JFLib Sans Bold", "JFLib Sans", "Bold"],
    ["Fluent System Icons", "Regular", "Fluent System Icons", "Fluent System Icons", "Regular"],
];

// ---------------------------------------------------------------- type ramp (theme.s)
// 13 px body, 11 px caption, bold titles
//                        caption body strong subtitle title large display
const TS_SIZE: [u8; 8] = [11, 13, 13, 15, 22, 28, 34, 13];
const TS_LINE: [u8; 8] = [14, 16, 16, 20, 28, 34, 41, 16];
const R: u8 = FONT_REG as u8;
const S: u8 = FONT_SEMI as u8;
const B: u8 = FONT_BOLD as u8;
const TS_FONT: [u8; 8] = [R, R, S, S, B, B, B, R];

// ---------------------------------------------------------------- tokens (T_* order)
// grey window, white controls, label colours with 85 / 50 / 25 % opacity
const TOKENS_LIGHT: [u32; T_COUNT] = [
    0xFFECECEC, 0xFFE4E4E4, 0xFFE5E5E5, 0x1A000000, // bg, bg alt, layer, layer stroke
    0xB3FFFFFF, 0x14000000, // card, card stroke
    0xFFFFFFFF, 0xFFFFFFFF, 0xFFE3E3E3, 0x80FFFFFF, // control rest/hover/pressed/disabled
    0x1F000000, 0x40000000, 0x5C000000, // ctrl stroke, bottom stroke, strong stroke
    0x00000000, 0x0F000000, 0x1C000000, 0x00000000, // subtle rest/hover/pressed/disabled
    0xD9000000, 0x80000000, 0x40000000, 0x40000000, // text primary/secondary/tertiary/disabled
    0, 0, 0, 0x26000000, // accent (accent()), accent disabled
    0xFFFFFFFF, 0xB3FFFFFF, 0, // on accent, on accent disabled, accent text
    0x8C007AFF, 0xFFFFFFFF, // focus outer, inner
    0xF5F6F6F6, 0x26000000, 0x26000000, 0x1A000000, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFE0352B, 0xFF1F9D3F, 0xFFC77700, 0xFF007AFF, // critical, success, caution, info
    0xFFFCE9E7, 0xFFE4F5E8, 0xFFFDF1DC, 0xFFE3EEFD, // their backgrounds
    0xFFFF5F57, 0x33000000, 0x80000000, 0x1F000000, // close hover, shadow, scroll thumb, track
    0xFFFFFFFF, 0xFFFFFFFF, 0x40000000, // knob, alt fill, strong fill
];
const TOKENS_DARK: [u32; T_COUNT] = [
    0xFF1E1E1E, 0xFF262626, 0xFF2C2C2C, 0x1AFFFFFF,
    0x0DFFFFFF, 0x14FFFFFF,
    0x40FFFFFF, 0x40FFFFFF, 0x59FFFFFF, 0x1AFFFFFF,
    0x1AFFFFFF, 0x2EFFFFFF, 0x59FFFFFF,
    0x00000000, 0x14FFFFFF, 0x24FFFFFF, 0x00000000,
    0xD9FFFFFF, 0x8CFFFFFF, 0x40FFFFFF, 0x40FFFFFF,
    0, 0, 0, 0x26FFFFFF,
    0xFFFFFFFF, 0x80FFFFFF, 0,
    0x8C0A84FF, 0xFF000000,
    0xF5282828, 0x33FFFFFF, 0x4D000000, 0x1AFFFFFF,
    0x0DFFFFFF,
    0xFFFF6961, 0xFF30D158, 0xFFFFB340, 0xFF409CFF,
    0xFF3A2422, 0xFF1E3224, 0xFF392D1A, 0xFF1C2A3E,
    0xFFFF5F57, 0x66000000, 0x80FFFFFF, 0x26FFFFFF,
    0xFFDCDCDC, 0x33FFFFFF, 0x59FFFFFF,
];

/// sty_accent: T_ACCENT (rest and hover alike: macOS controls do not highlight on hover),
/// T_ACCENT_PRESSED 20 % darker, T_ACCENT_TEXT = the accent (dark theme: 25 % lighter for
/// links on dark grounds); rgb 0 = system blue (#007AFF / dark #0A84FF)
fn accent(theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
    let mut c = rgb & 0xFF_FFFF;
    if c == 0 {
        c = if dark { 0x0A84FF } else { 0x007AFF };
    }
    let c = c | 0xFF00_0000;
    theme[(T_ACCENT / 4) as usize] = c;
    theme[(T_ACCENT_HOVER / 4) as usize] = c;
    theme[(T_ACCENT_PRESSED / 4) as usize] = color_mix(c, 0xFF00_0000, 51);
    let t = if dark { color_mix(c, 0xFFFF_FFFF, 64) } else { c };
    theme[(T_ACCENT_TEXT / 4) as usize] = t;
}
