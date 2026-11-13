# Changelog of jflib-update

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

### Added

- macOS app bundles: a program that runs from `NAME.app/Contents/MacOS/NAME` takes the channel's
  new kind `app` (`Kind::App`) and nothing else. The bundle is replaced as a whole: unpacked
  beside the installed one, its signature checked against the installed bundle's designated
  requirement (the same team and identifier; an installed bundle signed ad hoc asks for a valid
  signature and the same identifier), a copy of the old bundle kept as the backup, and the two
  folders exchanged in one step, so the app is never missing. Trial, `healthy()` and the rollback
  work as for an `exe`. A bundle in a folder the user cannot write, or one macOS moved away (App
  Translocation), is `Error::ReadOnly`; a bundle whose signature does not follow is
  `Startup::Failed(Error::Rejected(..))` and the staged update is dropped.

### Fixed

- `update::target()` names macOS and iOS (`macos-aarch64`, `ios-aarch64`); a Mac reported
  `linux-aarch64`.
- macOS: an `exe` update kept the old file's permissions only on Linux, so an updated program on
  a Mac lost its executable bit.
- macOS and iOS: an update's new file is created with `create_new`, as on Windows and Linux, and
  never written through an existing name or link.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
