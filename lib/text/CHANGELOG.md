# Changelog of jflib-text

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
