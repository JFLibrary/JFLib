# Changelog of jflib-text

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- macOS and iOS: `FontDb` scans the system's font folders there (`/System/Library/Fonts`,
  `/Library/Fonts`, `~/Library/Fonts`), for the installed fonts and the system fonts that draw
  missing characters.

### Fixed

- `Text::set_system_fallback` also forgets the line it shaped last: a line shaped before the
  switch kept its old glyphs.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
