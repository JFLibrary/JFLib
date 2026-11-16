# Changelog of jflib-appimagert

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.1] - 2026-11-16

### Security

- Without $XDG_CACHE_HOME and $HOME the runtime unpacks into $TMPDIR/appimage-run-<uid>, and only when that folder belongs to this user alone. It used /tmp/appimage-run, which every user can make, and would have started another user's AppRun from it.
- Extracting an AppImage drops setuid, setgid, sticky and world-write bits of the image and respects the umask; --appimage-extract refuses a squashfs-root that is not the user's own folder or a current folder where other users can rename entries, and keeps the extracted folders private until it finishes

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
