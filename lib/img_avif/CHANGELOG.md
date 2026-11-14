# Changelog of jflib-img-avif

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- The AVIF writer takes quality 0 as 95, like `io::opt_init(IMGF_AVIF)` and the other writers (it was 85)

## [0.2.0] - 2026-11-13

### Security

- Grid images are no longer decoded without a bound by `decode` (img_heif's reader): every
  tile's picture is checked before it is decoded (none larger than the first, all together
  within about four times the image), and a tile referenced many times is not decoded again and
  again. Such files fail with `ERR_LIMIT` instead of decoding up to 65535 large tiles from a
  small file.
- The pixel budget (`ImgDec::maxpx`) also applies to the picture an AV1 item declares in its
  frame header, not only to the image's size: a small file whose stream declares a huge picture
  fails with `ERR_LIMIT` before anything is allocated.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
