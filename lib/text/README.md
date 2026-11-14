# jflib-text

The text engine of [JFLib](https://jflib.org): it loads fonts, turns text into glyphs, smooths
them and draws them into JFLib's images and windows. Everything happens in this crate, in plain
Rust without the standard library and without `unsafe` code; no FreeType or HarfBuzz has to be
installed. Every label and every text box of JFLib's UI is drawn by it.

## What it does

**Fonts.** The four fonts a style embeds (regular, semibold, bold and the icon font) are always
there. TrueType and OpenType files and collections (`.ttf`, `.otf`, `.ttc`, `.otc`) load at run
time, with TrueType, CFF or CFF2 outlines. Variable fonts give instances at any axis values
(`Text::vary`), and their outlines, advances and coordinates match HarfBuzz's. A family without
a bold or italic face gets a synthetic one. Colour fonts draw their COLR layers and CBDT bitmaps.

**Drawing.** Glyphs are placed on quarter pixels, and their coverage masks wait in a glyph cache
for the next time. Text is smoothed in grey, in black and white when the system's font smoothing
is off, or with subpixel (ClearType-like) smoothing in the display's order on opaque targets,
using FreeType's 5-tap filter against colour fringes. Glyphs can also be recorded now and blended
later from any thread, which is how the UI renders large windows in parallel.

**Complex scripts.** With shaping switched on (`Text::set_shaping`), measuring, drawing, hit
testing and carets follow OpenType's rules: ligatures, combining marks, Arabic joining, Indic
reordering, and right-to-left runs in their visual order. The shaping itself comes from
[`jflib-text-shape`](https://crates.io/crates/jflib-text-shape).

**Missing characters.** A character the font lacks, an emoji or a Chinese character in a Latin
font, is drawn with another font: first the app's own chain (`set_fallback_fonts`), then the
system's fonts (`set_system_fallback`), found by scanning the font folders once and keeping the
result in a cache. Among the system's fonts those of the UI language come first, so Han
characters take the forms of Japanese or of Chinese as the language wants.

**System fonts.** `fontdb::FontDb` lists the installed fonts as families with their faces and
finds the closest weight and slant. `fontconf::FontConf` resolves a family the way fontconfig
does: generic names such as `sans-serif` or `monospace`, substitutes (Arial and Liberation Sans
for each other), the fonts of a language.

**Segmentation.** `text::segment` finds grapheme clusters and words (UAX #29) and line break
opportunities (UAX #14) after Unicode 15.0, and wraps lines, with soft hyphens and a hook for
hyphenation (`wrap_end_hyph`, which `jflib-spell`'s `Hyphenator` can serve).

## How it is used

Most apps meet this crate as `ui.text`, the engine the UI draws with. A font the app ships loads
into it once, and every text of the UI can use it from then on:

```rust
use jflib::ui::*;

struct Poster { font: i32 }

impl App for Poster {
    fn init(&mut self, ui: &mut Ui) -> bool {
        // a negative id is an error code
        self.font = ui.text.load_mem(include_bytes!("../assets/Display-VF.ttf"), 0);
        true
    }
    fn frame(&mut self, ui: &mut Ui) {
        if self.font >= 0 {
            // the variable font at weight 800: the same values give the same id every frame
            let heavy = ui.text.vary(self.font as u32, &[(*b"wght", 800.0)]);
            if heavy >= 0 {
                ui.text_in_font(heavy as u32, 24.0, "Variabel");
            }
        }
    }
}
```

The UI also chooses installed fonts by family, weight and size (`ui.font_load` with a `FontSel`)
and offers a font picker and a font dialog built on `FontDb`. Code that draws without the UI
creates a `Text` with the four embedded fonts of a style (`Text::new`), selects a font and a
size (`set_font`) and draws into a `gfx` surface (`draw`).

## Platforms

What differs between the systems is where the installed fonts are. The font database scans the
font folders of Windows, of Linux (with the folders of the fontconfig configuration, whose rules
`FontConf` reads there as well; elsewhere it has built-in tables), and of macOS and iOS.
The built-in tables and the fallback list name each system's own families: Segoe UI, YaHei
and Yu Gothic on Windows, Noto and DejaVu on Linux, Helvetica Neue, Hiragino, Heiti and Apple
SD Gothic Neo on macOS and iOS. Apple's colour emoji are sbix bitmaps, which `text` does not
draw, so emoji stay without a fallback there. `jfgui_FONTDIRS` replaces the folders. JFLib's UI turns the system fonts for missing characters on in
its windows and leaves them off in headless runs, so screenshots match on every machine.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), as `jflib::text`. All of
JFLib's crates share one version and are released together.

## What's new in 0.2.0

On macOS and iOS the font database now scans the system's font folders. Switching the system
fonts for missing characters on or off no longer leaves the line shaped last with its old
glyphs. Everything is in the
[changelog](https://docs.rs/crate/jflib-text/latest/source/CHANGELOG.md).

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  in JFLib's guide: fonts of the style, system fonts, fallback, subpixel smoothing, variable fonts
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#text): every public
  item of `text`, also on [docs.rs](https://docs.rs/jflib-text)

## License

MIT or Apache-2.0, at your choice. The synthetic bold (`Outline::embolden`) is a translation of
FreeType's `FT_Outline_EmboldenXY` under the FreeType License, and the segmentation tables are
generated from the Unicode Character Database under the Unicode License v3; both texts are in
the crate's `licenses/` folder.
