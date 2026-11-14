# Changelog of jflib-base

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- `base::Ini::load_checked` gives None when it cannot tell that the name is free (refused, an I/O error, a folder there) or cannot learn the file's size; only a missing name gives an empty Ini. `file_exists_checked` on a folder mount answers for its OS file (it gave the mount's kind: 0 for an error too)

### Fixed

- `Ini::load_checked` returned an empty Ini for a file it could not open when asking whether it exists failed (refused, an I/O error), and for a file whose size was unknown: a later save replaced the settings it never read

## [0.2.0] - 2026-11-13

### Added

- `xml::Limits` (`bytes`, `nodes`, `depth`) and `xml::parse_with(text, limits)`: the XML reader
  within limits the caller names. `Limits::default()` is what `xml::parse` keeps to (4 MiB,
  30000 elements, 48 levels of nesting).

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
