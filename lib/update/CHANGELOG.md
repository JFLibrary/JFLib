# Changelog of jflib-update

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- An apply that ended in the middle (a crash between two files, or after the app bundle's exchange) keeps its backup of the old version at the next start and finishes there, also when the new program is the one starting. The second attempt backed up files already replaced as the old ones, and a rollback then kept the new version.
- `startup`: a program on trial whose version is neither the trial's nor the one it replaced (the package's own number differs from its manifest's) ends the trial and records the manifest's version as bad. healthy() could never end it, and an older number had the same update offered and applied at every start.

### Security

- The backup of a replaced file carries its SHA-256 (backup/list "H sha256 path"); the way back checks every copy before it puts one back and again while it copies, so a changed copy leaves the new version whole ("R path" of earlier backups still works).

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
