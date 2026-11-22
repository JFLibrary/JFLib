# Changelog of jflib-spell

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- A dictionary's SET is decoded with jflib-formats' character sets (`formats::encoding::Charset`) instead of tables of its own: Hunspell's spellings as before (ISO8859-1 stays each byte its code point), and every label of the WHATWG Encoding Standard besides. Changed bytes: in microsoft-cp1251 and -cp1252 an unassigned byte of 0x80..0x9F is the C1 control of the same number instead of U+FFFD; ISO8859-9 / latin5 is windows-1254 and TIS-620 windows-874 (their punctuation in 0x80..0x9F); KOI8-U 0xAE and 0xBE are ў and Ў.
- The n-gram step builds an affixed form only when its length can be within reach of the word (and finds the edit distance only within its limit, and drops duplicates through a set): real suggestions are faster, hu_HU 35 -> 9 s for 400 (the slowest 953 -> 275 ms), ko_KR 43 -> 20 s, the same answers

### Fixed

- `token_errors` tries runs of at most 8 parts of a hyphenated word: a word of 1000 parts took 500 000 checks in the UI thread.
- COMPOUNDRULE matches with a table over rule elements and parts; trying every way the `*` and `?` elements could share the parts was exponential in their number.

### Security

- A crafted dictionary (config://hunspell is the user's to fill) could make checking and suggestions hang: 20 000 empty suffixes that may follow each other took 2 to 9 s for a check and 540 s for a suggestion, cross products of 3000 x 3000 empty affixes 7 to 63 s for a suggestion, the forms of the n-gram step more than 120 s, 10 000 BREAK patterns 0.95 s for a check; words whose hashes collide loaded with the square of their number, 23 000 affix rules in 0.56 s. The work is now bounded by counts, not a clock, so the answers still never depend on the machine: every affix rule tried (also as the inner of two suffixes and in a cross product), COMPOUNDRULE, CHECKCOMPOUNDPATTERN, BREAK pattern and REP rule of CHECKCOMPOUNDREP takes a unit, 6 million for a check, 64 million for a search for suggestions, 32 million for the n-gram step's forms; the generators (TRY, KEY, REP, MAP) stop once they are spent. The word list looks for a word at most 128 slots on and keeps the rest in a sorted map beside the table; the rule groups and the outer suffixes are found in one pass. Now at most about 0.15 s for a check and 1.6 s for a suggestion. 20 real dictionaries give exactly the answers they gave before and use a sixth of the bounds or less (the most: 638 000 for a check, da_DK; 10.9 million for a search, hu_HU)

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
