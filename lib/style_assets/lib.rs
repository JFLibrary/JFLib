// style_assets -- the fonts and the icon set the library's styles share, one place for the bytes
// (docs/STYLES.md): winui and macos take all four, classic and kde their own text fonts and ICONS.
// The licences lie next to them in assets/.
#![no_std]
#![forbid(unsafe_code)]

/// "JFLib Sans" regular: SGF1, a subset of Selawik 1.01 (OFL-Selawik.txt, OFL-Inter.txt)
pub static FONT_REG: &[u8] = include_bytes!("assets/font_reg.bin");
/// "JFLib Sans" semibold
pub static FONT_SEMI: &[u8] = include_bytes!("assets/font_semi.bin");
/// "JFLib Sans" bold
pub static FONT_BOLD: &[u8] = include_bytes!("assets/font_bold.bin");
/// the icon font, code points IC_* of ui::keys: Fluent UI System Icons (LICENSE-FluentSystemIcons.txt)
pub static ICONS: &[u8] = include_bytes!("assets/icons.bin");
