# Changelog of jflib-modrt

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

### Added

- ARM64, on Linux and Windows: `cpu`, `fx` and `x87` take rt's versions for ARM64 (the software
  math gives the same bits as rt's on every processor), `memcpy`, `memmove`, `memset`, `memcmp`
  and `bcmp` for ARM64, `trap()` as `udf`; for Windows on ARM64 also `__chkstk`,
  `__CxxFrameHandler3` and `_fltused`, so a module still imports nothing.
- `plugin_abi!`: the calling convention of plugins and modules as a macro, `extern "win64"` on
  x86-64 as before, `extern "C"` on ARM64. `module_entry!` and the records of `modabi`
  (`HostTable`, `ModTable`, `ItemFn`) use it.

### Fixed

- `sync::SpinLock::try_lock` no longer releases a lock another thread holds when it fails.
- The guard of `sync::SpinLock` is `Sync` only when the value is (a guard of a `Cell` could be
  shared between threads).

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
