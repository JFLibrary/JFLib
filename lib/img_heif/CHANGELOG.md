# Changelog of jflib-img-heif

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Security

- Grid images are no longer decoded without a bound: every tile's picture is checked before it
  is decoded (none larger than the first, all together within about four times the image), and
  the data of a tile referenced many times is not fed to the decoder again and again. Such files
  fail with `ERR_LIMIT` instead of decoding up to 65535 large tiles from a small file.
- The pixel budget (`ImgDec::maxpx`) also applies to the picture an HEVC or AV1 item declares in
  its stream (the SPS, the AV1 frame header), not only to the image's size: a small file whose
  stream declares a huge picture fails with `ERR_LIMIT` before anything is allocated.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
