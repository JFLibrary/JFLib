# Changelog of jflib-img-jxl

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Security

- A container box with a 64-bit size that reaches the end of the number range no longer crashes
  `probe` and `decode` (an out-of-bounds access, from a file of 28 bytes); such files are refused
  with `ERR_HDR`.
- A frame with a size of its own is held to the pixel budget (`ImgDec::maxpx`, by default 2^28)
  like the image: a larger frame fails with `ERR_LIMIT` instead of being decoded at up to
  2^30 x 2^30 pixels.
- Modular images: the channels that palette transforms add may together hold at most as many
  samples as the image plus 2^20. Small files with large palettes no longer make the decoder
  allocate and decode gigabytes; they fail with `ERR_LIMIT`.
- The splines' draw cache covers only the rows the splines reach and is allocated fallibly
  (`ERR_NOMEM`). It took one entry per row of the upsampled frame, up to 4 GB, and crashed when
  that failed.
- Patches: the rectangles of all patch positions together may cover at most four times the
  frame's pixels plus 2^20, else `ERR_LIMIT`; only their number was limited, and blending them
  could take minutes.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
