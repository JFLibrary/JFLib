# Changelog of jflib-style-netlab

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

- NetLab is an official style of JFLib: `jflib` brings it like the other four, as
  `jflib::style_netlab` (its `STYLE`), as `jflib::style` with `JFLIB_STYLE=netlab`, and
  `cargo jflib package --styles netlab` builds it into an app's packages. With 0.1.0 an app
  named the crate itself; it now comes in the same version as `jflib`.

### Fixed

- Views that scroll only vertically keep the room of netlab's always visible scroll bar (10 DIP)
  free beside their content, overflowing or not (`StyleList::scrollbar_room`): the content no
  longer runs under the bar.
- The manifest names the licences of the fonts and the icon font the crate carries:
  `(MIT OR Apache-2.0) AND OFL-1.1 AND MIT` (it said `MIT OR Apache-2.0` alone).

## [0.1.0] - 2026-11-10

First release on crates.io, published on its own.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/jflib-style-netlab-v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/jflib-style-netlab-v0.1.0
