# Changelog of jflib-wgsl

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

### Fixed

- A compound assignment of a vector with a scalar (`v += 1.0`, `v -= s`, `v /= 2.0`, `w += 1` of an integer vector, also through a swizzle) splats the scalar as `v + 1.0` does. In SPIR-V `v += 1.0` became a multiplication by 1.0 (`-=` and `/=` alike), and integer vectors gave invalid SPIR-V; a float `v *= s` stays a vector times a scalar.
- A module whose SPIR-V needs an instruction longer than 65535 words (a name of 262,140 characters or more, a constant with that many components) is a compile error instead of a broken module.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
