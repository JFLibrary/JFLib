# Changelog of jflib-diagram-view

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- New crate `diagram_view` (`jflib::diagram_view`): draws a page's display list (diagram_core::render) on a render2d scene -- its CPU path without a device; gradient fills and pictures drawn by gfx between the scene's layers -- `PageView` (`draw`, `render`, `fit`), `FontSet` with the app's fonts and installed ones (`FontSource`), missing families substituted by class with a width factor, PNG and JPEG pictures decoded, placeholders for EMF and WMF

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
