# Changelog of jflib-compress

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- DEFLATE, zlib and gzip in both directions, the Brotli decoder, CRC-32 and Adler-32, moved here from img_core and img_jxl: `compress::deflate`, `compress::zlib` and `compress::gzip` with `compress(data, level)` and `decompress(data, max_out)` (strict: data after the stream is refused; `gzip::decompress` reads every member, as gunzip does), reusable states (`deflate::DeflateState`, `deflate::InflateState`) and `*_into` variants in the caller's buffer that report the input a stream took, `deflate::Error`, `deflate::bound`; `compress::brotli::{decompress, decompress_prefix, Error}` (RFC 7932, no encoder); `compress::{crc32, adler32}`. The encoder gives the same bytes as before at every level, the decoder the same errors; the output buffer of `decompress` grows with the data, up to max_out. compress still depends on nothing, so the build tools with std use it too.

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
