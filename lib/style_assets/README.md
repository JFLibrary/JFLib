# jflib-style-assets

The fonts and the icon set that several of [JFLib](https://jflib.org)'s styles share, kept in one
crate so that their bytes exist only once. A style is the crate that decides how JFLib's widgets
look; this one holds no drawing code, only four embedded files.

## A building block

`jflib` brings this crate along and makes it reachable as `jflib::style_assets`, but hides it
from its documentation: it is no part of what `jflib` promises (JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md#the-crates-on-cratesio))
and may change in any release. An app does not need it; it picks a style, and the style takes
what it needs from here.

## What's in it

`FONT_REG`, `FONT_SEMI` and `FONT_BOLD` are "JFLib Sans" in regular, semibold and bold: subsets
of Microsoft's Selawik 1.01 without hinting, in JFLib's compact font format. The characters of
JFLib's character set that Selawik lacks are composed from its own glyphs, and three (∞, ●, №)
come from Inter. Because the subsets are modified versions and
"Selawik" is a reserved font name, they carry a name of their own.

`ICONS` is the icon font: glyphs of Microsoft's Fluent UI System Icons at the code points of
JFLib's icon constants (`IC_*` in `ui::keys`), so every widget finds its arrow, check mark or
folder.

The WinUI and macOS styles take all four files, the classic and KDE styles the icons beside
their own DejaVu text fonts.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), which brings it along with
the styles. All of JFLib's crates share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-style-assets/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [Fonts and assets of the styles](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md#fonts-and-assets):
  how the fonts were made and which style uses what
- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  in JFLib's guide
- [API reference](https://docs.rs/jflib-style-assets) on docs.rs

## License

The crate is MIT or Apache-2.0, at your choice. The fonts are under the SIL Open Font License
1.1 (Selawik and Inter) and the icons under the MIT license of Fluent UI System Icons; the
license texts lie in the crate's `assets/` folder.
