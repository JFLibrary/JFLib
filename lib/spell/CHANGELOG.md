# Changelog of jflib-spell

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- COMPOUNDMIN is clamped to 1..=65535. A value of 2^63 wrapped to 0 when doubled, and checking a word then read past its end.

## [0.2.0] - 2026-11-13

### Added

- macOS and iOS: `SRC_SYSTEM` finds the system's spell checker, `NSSpellChecker` on macOS and
  `UITextChecker` on iOS, with the languages it knows and the words the user taught it; a
  `Speller` on it reports the backend `"system"`. On macOS `SRC_SYSTEM` also takes Hunspell
  dictionaries from `~/Library/Spelling` and `/Library/Spelling`.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
