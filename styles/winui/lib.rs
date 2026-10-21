// style_winui -- the WinUI 3 (Fluent) style: tokens, type ramp, fonts, paint functions
//                                                                       [styles/winui]
// `STYLE` implements ui::Style; the app is linked with one style crate (build_rs.py
// --style winui, the default) and passes `&style::STYLE` to ui::run.  Measuring returns
// sizes, painting emits draw commands.
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

pub struct WinUi;

/// the style the app passes to ui::run
pub static STYLE: WinUi = WinUi;

impl Style for WinUi {
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
        [FONT_REG, FONT_SEMI, FONT_BOLD, FONT_ICON]
    }
    fn font_names(&self) -> [[&'static str; 5]; 4] {
        FONT_NAMES
    }
}

// ---------------------------------------------------------------- fonts (styles/winui/fonts.s)
// text: Selawik 1.01 regular / semibold / bold (SIL OFL 1.1, assets/OFL-Selawik.txt), the
// open replacement for Segoe UI, as SGF1 subsets with Segoe UI's line box (ttf2bin.py
// --metrics win).  "Selawik" is a Reserved Font Name, so the subsets are called JFLib Sans.
// What Selawik lacks of the text charset is built from its own glyphs (ttf2bin.py --compose);
// ∞ ● № come from Inter (SIL OFL 1.1, assets/OFL-Inter.txt; --donor).
// Icons: Segoe Fluent Icons (Microsoft, private builds only -- styles/NOTICE.md)
static FONT_REG: &[u8] = include_bytes!("assets/font_reg.bin");
static FONT_SEMI: &[u8] = include_bytes!("assets/font_semi.bin");
static FONT_BOLD: &[u8] = include_bytes!("assets/font_bold.bin");
static FONT_ICON: &[u8] = include_bytes!("assets/icons.bin");
// family, style, full, typographic family, typographic style  [fonts.s sty_font_names]
static FONT_NAMES: [[&str; 5]; 4] = [
    ["JFLib Sans", "Regular", "JFLib Sans", "JFLib Sans", "Regular"],
    ["JFLib Sans Semibold", "Regular", "JFLib Sans Semibold", "JFLib Sans", "Semibold"],
    ["JFLib Sans", "Bold", "JFLib Sans Bold", "JFLib Sans", "Bold"],
    ["Fluent System Icons", "Regular", "Fluent System Icons", "Fluent System Icons", "Regular"],
];

// ---------------------------------------------------------------- type ramp (theme.s)
//                        caption body strong subtitle title large display
const TS_SIZE: [u8; 8] = [12, 14, 14, 20, 28, 40, 68, 14];
const TS_LINE: [u8; 8] = [16, 20, 20, 28, 36, 52, 92, 20];
const TS_FONT: [u8; 8] = [0, 0, 1, 1, 1, 1, 1, 0];

// ---------------------------------------------------------------- tokens (T_* order)
const TOKENS_LIGHT: [u32; T_COUNT] = [
    0xFFF3F3F3, 0xFFEEEEEE, 0x80FFFFFF, 0x12000000, // bg, bg alt, layer, layer stroke
    0xB3FFFFFF, 0x0F000000, // card, card stroke
    0xB3FFFFFF, 0x80F9F9F9, 0x4DF9F9F9, 0x4DF9F9F9, // control rest/hover/pressed/disabled
    0x0F000000, 0x29000000, 0x72000000, // ctrl stroke, bottom stroke, strong stroke
    0x00000000, 0x09000000, 0x06000000, 0x00000000, // subtle rest/hover/pressed/disabled
    0xE4000000, 0x9E000000, 0x72000000, 0x5C000000, // text primary/secondary/tertiary/disabled
    0, 0, 0, 0x37000000, // accent (runtime), accent disabled
    0xFFFFFFFF, 0xFFFFFFFF, 0, // on accent, on accent disabled, accent text
    0xE4000000, 0xB3FFFFFF, // focus outer, inner
    0xFFF9F9F9, 0x0F000000, 0x4D000000, 0x0F000000, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFC42B1C, 0xFF0F7B0F, 0xFF9D5D00, 0xFF005FB7, // critical, success, caution, info
    0xFFFDE7E9, 0xFFDFF6DD, 0xFFFFF4CE, 0xFFF6F6F6, // their backgrounds
    0xFFC42B1C, 0x24000000, 0x72000000, 0x72000000, // close hover, shadow, scroll thumb, track
    0xFFFFFFFF, 0x06000000, 0x72000000, // knob, alt fill, strong fill
];
const TOKENS_DARK: [u32; T_COUNT] = [
    0xFF202020, 0xFF1C1C1C, 0x4D3A3A3A, 0x19000000,
    0x0DFFFFFF, 0x1A000000,
    0x0FFFFFFF, 0x15FFFFFF, 0x08FFFFFF, 0x0BFFFFFF,
    0x12FFFFFF, 0x18FFFFFF, 0x8BFFFFFF,
    0x00000000, 0x0FFFFFFF, 0x0AFFFFFF, 0x00000000,
    0xFFFFFFFF, 0xC5FFFFFF, 0x87FFFFFF, 0x5DFFFFFF,
    0, 0, 0, 0x28FFFFFF,
    0xFF000000, 0x87FFFFFF, 0,
    0xFFFFFFFF, 0xB3000000,
    0xFF2C2C2C, 0x33000000, 0x4D000000, 0x15FFFFFF,
    0xB31E1E1E,
    0xFFFF99A4, 0xFF6CCB5F, 0xFFFCE100, 0xFF60CDFF,
    0xFF442726, 0xFF393D1B, 0xFF433519, 0xFF2B2B2B,
    0xFFC42B1C, 0x42000000, 0x8BFFFFFF, 0x8BFFFFFF,
    0xFF454545, 0x19000000, 0x8BFFFFFF,
];

/// sty_accent: Windows 11 accent ramp (default dark: #4CC2FF / #99EBFF, light: #005FB8 /
/// #003E92); a custom accent is mixed towards white (dark) or black (light)
fn accent(theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
    let rgb = rgb & 0xFF_FFFF;
    let (b, s) = if rgb == 0 {
        if dark { (0x4CC2FF, 0x99EBFF) } else { (0x005FB8, 0x003E92) }
    } else {
        let c = rgb | 0xFF00_0000;
        if dark {
            (color_mix(c, 0xFFFF_FFFF, 102), color_mix(c, 0xFFFF_FFFF, 166))
        } else {
            (color_mix(c, 0xFF00_0000, 38), color_mix(c, 0xFF00_0000, 90))
        }
    };
    let (b, s) = (b & 0xFF_FFFF, s & 0xFF_FFFF);
    theme[(T_ACCENT / 4) as usize] = b | 0xFF00_0000;
    theme[(T_ACCENT_HOVER / 4) as usize] = b | 0xE600_0000;
    theme[(T_ACCENT_PRESSED / 4) as usize] = b | 0xCC00_0000;
    theme[(T_ACCENT_TEXT / 4) as usize] = s | 0xFF00_0000;
}
