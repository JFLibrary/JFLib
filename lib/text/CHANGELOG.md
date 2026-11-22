# Changelog of jflib-text

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- fontconfig files are read with jflib-formats' forgiving `formats::xml::Lenient` instead of a reader of the crate's own (`fontconf`): a '>' inside a quoted attribute value no longer ends the tag, an unquoted value no longer ends the attributes; the system's 63 fontconfig files give the same rules as before.

### Fixed

- `segment::wrap_end` / `wrap_end_hyph` read the break opportunities in a window that grows while its text fits, and measure the rest of the paragraph only that far; every line scanned and measured the whole rest (quadratic over a long paragraph). A line start inside a character counts from that character's start (it panicked); words longer than about 256 characters are no longer hyphenated (they were found and hyphenated whole for every line).
- `segment::word_bounds` counts a run of regional indicators along instead of walking back over it for every pair (quadratic in the run).
- `Text::hit` measures the line once instead of every prefix up to the hit (quadratic in the index).
- A line with right-to-left characters (Hebrew, Arabic, ... or RLM, RLE, RLO, RLI) is laid out by the Unicode bidi algorithm also while shaping is off (bidi.rs; `Text::set_bidi`, on by default; `has_rtl`): its runs in visual order, a right-to-left run shaped with its TrueType / OpenType font (the fallback font, usually: Arabic joined, marks placed, brackets mirrored), other right-to-left text drawn cluster by cluster from the right with mirrored brackets. Measure, draw, `caret_x`, `hit` and `selection_spans` read the same layout; the last 128 such lines are kept. Lines without right-to-left characters keep their metrics and pixels. Before, such lines were drawn in logical order: Hebrew and Arabic read backwards ("מחיקה" as "הקיחמ") wherever shaping was off, in all of ui's controls

### Security

- CFF: a Private DICT is read to at most 64 KiB. Each of up to 256 font DICTs may name its own, and walking each to the end of the file was 256 times the file.

## [0.2.1] - 2026-11-16

### Fixed

- Line breaking with soft hyphens (`segment::wrap_end_hyph`) stops measuring the later soft hyphens once one does not fit. 30 KB of text with many of them froze the UI.
- The CFF charstring operator `roll` rotates in one pass. A roll of the whole CFF2 stack cost num × r moves while the budget counted one operator; such a font took about 35 s to load.

## [0.2.0] - 2026-11-14

### Added

- macOS and iOS: `FontDb` scans the system's font folders there (`/System/Library/Fonts`,
  `/Library/Fonts`, `~/Library/Fonts`), for the installed fonts and the system fonts that draw
  missing characters.

### Fixed

- `Text::set_system_fallback` also forgets the line it shaped last: a line shaped before the
  switch kept its old glyphs.
- Overflows that only a build with overflow checks saw (a debug build of an app without JFLib's profile aborted): the underline position of variable fonts, shaped glyph offsets near i32::MAX.
- macOS and iOS: the system fallback and `FontConf`'s built-in tables name Apple's fonts (Helvetica Neue, Hiragino, Heiti, Apple SD Gothic Neo, Thonburi, Kohinoor, Geeza Pro, ...); they named Linux families, so Chinese, Japanese, Korean and other scripts found no font there. Emoji still have none: Apple Color Emoji's glyphs are sbix bitmaps, which `text` does not draw

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
