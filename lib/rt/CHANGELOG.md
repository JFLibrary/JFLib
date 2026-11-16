# Changelog of jflib-rt

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- rt::outline_lse(): the way the outline atomics of a static Linux ARM64 program went (LSE or the LL/SC loops; None where a program has none); jfgui_ARM_LLSC=1 in the environment makes them take LL/SC on a CPU with LSE, so the same program checks both ways (docs/ARM64.md, ARM7)

## [0.2.1] - 2026-11-16

### Added

- Intel Macs (`x86_64-apple-darwin`): `cpu` detects the processor by CPUID (AVX2, FMA, BMI2) and `x87` computes on the x87 unit, as on Windows and Linux (before, every Apple target took the portable versions); checked under Rosetta 2, where the decimal conversions give the same bits as on Windows ([INTEL_MAC.md](https://github.com/JFLibrary/JFLib/blob/main/docs/INTEL_MAC.md))

### Changed

- Linux: the build script picks rt's build against glibc (`--cfg jfgui_glibc`) unless
  `-C target-feature=+crt-static` asks for one static file, so a JFLib app builds on Linux with
  plain `cargo run`, no flags and no `--target`.

### Fixed

- `rt::main!` in a library crate with std: `cargo test --lib` builds its unit tests. The panic
  check refused the test build (tests always unwind); `main!`'s own `jfgui_main` links the tests,
  `rt::test_main!` stays for library crates without `main!`.

### Security

- New plat::fs::Dir works below an open folder without ever following a link (Linux openat2 with a step-by-step O_NOFOLLOW fallback, macOS openat walk, Windows folders held open without share-delete), with set_file_times / set_file_mode through a file's own handle; TcpStream::peer_uid returns the user id of the other end of a Unix socket (SO_PEERCRED, getpeereid)

## [0.2.0] - 2026-11-14

### Added

- macOS and iOS (`aarch64-apple-darwin`, `aarch64-apple-ios`, `aarch64-apple-ios-sim`): the
  runtime runs on libSystem.
- Linux on ARM64: system calls through `svc`, `memcpy` and its relatives for ARM64, `getauxval`
  and the constructors the LSE atomics of `compiler-builtins` need; `rt::os::page_size()` reads
  the page size at run time (4, 16 or 64 KB) from `AT_PAGESZ`.
- Windows on ARM64: `__chkstk` for ARM64.
- `rt::plugin_abi!`: the calling convention of plugins and loadable modules as a macro,
  `extern "win64"` on x86-64 as before, `extern "C"` on ARM64 and elsewhere.
- With the feature `unwind` (and std): `rt::abort_on_unwind(f)` ends the program when a panic
  leaves `f`, `rt::panic_take()` gives the text of the calling thread's last panic, and
  `rt::panic_hook()` reports only panics nobody catches.
- `rt::test_main!()`: the `jfgui_main` stub the unit tests of a library crate with std need to
  link (`#[cfg(test)] rt::test_main!();` in its `lib.rs`).
- `rt::os::write_stderr_crash`: stderr for the crash path; on Linux it gives up a non-blocking
  stderr that takes nothing for half a second instead of waiting for it.

### Changed

- `rt::x87` outside x86-64 (`rt::portable::math`): `pow`, `sin` and `cos` are within about half
  an ulp (were up to 118 ulp for `pow`), `exp2`, `log2` and `atan2` too (were up to 4 to 5 ulp);
  any finite angle is reduced exactly. The results are still the same bits on every processor.
- The macros of rt expand without `#[allow(unsafe_code)]`, so a crate with
  `#![forbid(unsafe_code)]` can use them (it was error E0453).

### Fixed

- Programs without std link with Rust 1.99 (LLVM 23) again: LLVM turns a loop that counts wide
  characters up to the NUL into a call to `wcslen`, which rt now has. With 0.1.0 every program on
  Windows failed with "unresolved external symbol wcslen".
- `SpinLock::try_lock` no longer releases a lock another thread holds when it fails.
- The guard of `SpinLock` is `Sync` only when the value is (a guard of a `Cell` could be shared
  between threads).

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
