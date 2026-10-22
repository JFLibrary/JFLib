// style_classic -- the "classic" style (Windows 2000 / Office 97 look): tokens, type ramp,
// fonts, paint functions                                               [styles/classic]
//
// Grey 3D bevels, square controls, DejaVu Sans Condensed, navy selection, gradient title
// bars.  `STYLE` implements ui::Style; an app is linked with it by `build_rs.py --style
// classic` and passes `&style::STYLE` to ui::run.  The drawing helpers of the style's common.s
// are in common.rs; the text boxes (list.s in the asm) are in edit.rs, the focus rectangle
// (basic.s) in list.rs, like the sub-traits of lib/ui/style.rs.
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

pub struct Classic;

/// the style the app passes to ui::run
pub static STYLE: Classic = Classic;

impl Style for Classic {
    fn name(&self) -> &'static str {
        "classic"
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
        // classic has no semibold: FONT_SEMI is the bold face (fonts.s)
        [FONT_REG_DATA, FONT_BOLD_DATA, FONT_BOLD_DATA, FONT_ICON_DATA]
    }
    fn font_names(&self) -> [[&'static str; 5]; 4] {
        FONT_NAMES
    }
}

// ---------------------------------------------------------------- fonts (styles/classic/fonts.s)
// DejaVu Sans Condensed regular / bold in Tahoma's line box (SGF1; assets/DejaVu-LICENSE.txt,
// made by tools/ttf2bin.py --metrics 2049,-423, see docs/STYLES.md) and the WinUI icons
static FONT_REG_DATA: &[u8] = include_bytes!("assets/font_reg.bin");
static FONT_BOLD_DATA: &[u8] = include_bytes!("assets/font_bold.bin");
static FONT_ICON_DATA: &[u8] = include_bytes!("../winui/assets/icons.bin");
// family, style, full, typographic family, typographic style  [fonts.s sty_font_names]
static FONT_NAMES: [[&str; 5]; 4] = [
    ["DejaVu Sans Condensed", "Book", "DejaVu Sans Condensed", "DejaVu Sans", "Condensed"],
    ["DejaVu Sans Condensed", "Bold", "DejaVu Sans Condensed Bold", "DejaVu Sans", "Condensed Bold"],
    ["DejaVu Sans Condensed", "Bold", "DejaVu Sans Condensed Bold", "DejaVu Sans", "Condensed Bold"],
    ["Fluent System Icons", "Regular", "Fluent System Icons", "Fluent System Icons", "Regular"],
];

// ---------------------------------------------------------------- type ramp (theme.s)
//                        caption body strong subtitle title large display
const TS_SIZE: [u8; 8] = [11, 11, 11, 13, 16, 20, 32, 11];
const TS_LINE: [u8; 8] = [14, 16, 16, 18, 22, 28, 40, 16];
const TS_FONT: [u8; 8] = [
    FONT_REG as u8, FONT_REG as u8, FONT_SEMI as u8, FONT_SEMI as u8,
    FONT_SEMI as u8, FONT_SEMI as u8, FONT_SEMI as u8, FONT_REG as u8,
];

// ---------------------------------------------------------------- tokens (T_* order)
const TOKENS_LIGHT: [u32; T_COUNT] = [
    0xFFD4D0C8, 0xFFC8C4BC, 0xFFD4D0C8, 0xFF808080, // bg, bg alt, layer, layer stroke
    0xFFD4D0C8, 0xFF808080, // card, card stroke
    0xFFD4D0C8, 0xFFD4D0C8, 0xFFD4D0C8, 0xFFD4D0C8, // control rest/hover/pressed/disabled
    0xFF808080, 0xFF404040, 0xFF404040, // ctrl stroke, bottom stroke, strong stroke
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // subtle rest/hover/pressed/disabled
    0xFF000000, 0xFF404040, 0xFF808080, 0xFF808080, // text primary/secondary/tertiary/disabled
    0, 0, 0, 0xFF808080, // accent (accent()), accent disabled
    0xFFFFFFFF, 0xFFD4D0C8, 0, // on accent, on accent disabled, accent text
    0xFF000000, 0xFFFFFFFF, // focus outer, inner
    0xFFD4D0C8, 0xFF404040, 0x30000000, 0xFF808080, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFC00000, 0xFF008000, 0xFF806000, 0xFF000080, // critical, success, caution, info
    0xFFFFE0E0, 0xFFE0FFE0, 0xFFFFFFE1, 0xFFFFFFE1, // their backgrounds
    0xFF0A246A, 0x40000000, 0xFFD4D0C8, 0xFFFFFFFF, // close hover, shadow, scroll thumb, track
    0xFFD4D0C8, 0xFFFFFFFF, 0xFF808080, // knob, alt fill, strong fill
];
const TOKENS_DARK: [u32; T_COUNT] = [
    0xFF404040, 0xFF383838, 0xFF404040, 0xFF202020,
    0xFF404040, 0xFF202020,
    0xFF404040, 0xFF404040, 0xFF404040, 0xFF404040,
    0xFF202020, 0xFF101010, 0xFF101010,
    0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0xFFE8E8E8, 0xFFC0C0C0, 0xFF909090, 0xFF808080,
    0, 0, 0, 0xFF707070,
    0xFFFFFFFF, 0xFF606060, 0,
    0xFFFFFFFF, 0xFF000000,
    0xFF404040, 0xFF101010, 0x50000000, 0xFF202020,
    0xFF2A2A2A,
    0xFFFF8080, 0xFF80E080, 0xFFE0C060, 0xFF80A0FF,
    0xFF503030, 0xFF305030, 0xFF4A4A30, 0xFF4A4A30,
    0xFF2F5DA8, 0x60000000, 0xFF404040, 0xFF2A2A2A,
    0xFF404040, 0xFF2A2A2A, 0xFF707070,
];

/// sty_accent: T_ACCENT* and T_ACCENT_TEXT from the app's accent (0 = the classic navy, a
/// lighter blue in the dark variant); hover a little lighter, accent text lighter in the
/// dark theme
fn accent(theme: &mut [u32; T_COUNT], dark: bool, rgb: u32) {
    let mut a = rgb & 0xFF_FFFF;
    if a == 0 {
        a = if dark { 0x2F5DA8 } else { 0x0A246A };
    }
    let c = a | 0xFF00_0000;
    theme[(T_ACCENT / 4) as usize] = c;
    theme[(T_ACCENT_PRESSED / 4) as usize] = c;
    theme[(T_ACCENT_HOVER / 4) as usize] = color_mix(c, 0xFFFF_FFFF, 32);
    let mut t = a | 0xFF00_0000;
    if dark {
        t = color_mix(t, 0xFFFF_FFFF, 96);
    }
    theme[(T_ACCENT_TEXT / 4) as usize] = t;
}
