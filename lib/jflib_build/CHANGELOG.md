# Changelog of jflib-build

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

- macOS and iOS (`aarch64-apple-darwin`, `aarch64-apple-ios`, `aarch64-apple-ios-sim`): the build
  script accepts them and gives the linker nothing extra; rustc links through Apple's linker and
  libSystem.
- `PACKAGE`: `browser=1` says the app is a web browser and reaches the program, whose `--register`
  then registers it as one. It needs `http` and `https` in `schemes=`; a value other than `1` or
  `0` is an error.
- Linux on ARM64 against glibc (`--cfg jfgui_glibc`): the stub `libc.so.6` made for linking also
  carries `getauxval`, which the runtime uses to read the page size.

### Changed

- The error for a Linux build without `-C relocation-model=static` also says to build with
  `--target`, on Linux too, so that the flags do not reach the build scripts.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
