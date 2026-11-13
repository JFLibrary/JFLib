# Changelog of jflib-shader

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

### Added

- `shader::msl`: SPIR-V to the Metal Shading Language. `msl::translate(words, &msl::Options)`
  gives an `Msl` with the source text and, per entry point (`msl::Entry`), the MSL function's
  name, the workgroup size with the constants substituted, the slot of each resource, whether it
  reads push constants (`PUSH_SLOT`), buffer sizes (`SIZES_SLOT`) or `DrawIndex`
  (`DRAW_INDEX_AT`), and the locations of its vertex inputs. jflib-gpu's Metal backend translates
  every module with it, so the SPIR-V of jflib-wgsl and of this crate's builder runs on macOS and
  iOS unchanged.
- The translation substitutes specialization constants (`Options::spec`), turns arrays of images,
  samplers and storage buffers into Metal 3 argument buffers (MSL 3.0), wraps signed integer
  arithmetic as SPIR-V does, clamps image reads into the image and drops stores outside it, as
  WGSL allows.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
