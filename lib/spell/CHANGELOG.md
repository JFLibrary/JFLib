# Changelog of jflib-spell

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- A dictionary's SET is decoded with jflib-formats' character sets (`formats::encoding::Charset`) instead of tables of its own: Hunspell's spellings as before (ISO8859-1 stays each byte its code point), and every label of the WHATWG Encoding Standard besides. Changed bytes: in microsoft-cp1251 and -cp1252 an unassigned byte of 0x80..0x9F is the C1 control of the same number instead of U+FFFD; ISO8859-9 / latin5 is windows-1254 and TIS-620 windows-874 (their punctuation in 0x80..0x9F); KOI8-U 0xAE and 0xBE are ў and Ў.

### Fixed

- `token_errors` tries runs of at most 8 parts of a hyphenated word: a word of 1000 parts took 500 000 checks in the UI thread.
- COMPOUNDRULE matches with a table over rule elements and parts; trying every way the `*` and `?` elements could share the parts was exponential in their number.

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

### Added

- macOS and iOS: `SRC_SYSTEM` finds the system's spell checker, `NSSpellChecker` on macOS and
  `UITextChecker` on iOS, with the languages it knows and the words the user taught it; a
  `Speller` on it reports the backend `"system"`. On macOS `SRC_SYSTEM` also takes Hunspell
  dictionaries from `~/Library/Spelling` and `/Library/Spelling`.

### Fixed

- COMPOUNDMIN is clamped to 1..=65535. A value of 2^63 wrapped to 0 when doubled, and checking a word then read past its end.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
