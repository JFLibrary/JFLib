# Changelog of jflib-img-jxl

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- `modular::SqWork` / `SQ_WORK` and `frm::Jf::sq_work`: the squeeze budget of a decode, handed on from frame to frame (`modular::jm_decode` called alone has one of its own).

### Fixed

- Modular images: a squeeze of a channel whose shift is already above 30 is refused (`ERR_HDR`, as libjxl does). Squeeze steps on a 1 x 1 channel added an empty channel each, up to 65536, each inserted into the middle of the list (74 KB took 232 s).
- Modular images: the squeeze steps of a decode may move at most 2^24 channel entries in all, over its frames and groups (each step's residuals go in as one block), beyond that `ERR_LIMIT`. Steps on a palette's meta channel, which has no shift to count, inserted a channel each near the front of the list (74 KB took 56 s; now refused in about 0.3 s).

## [0.2.1] - 2026-11-16

### Fixed

- The bit reader's position saturates instead of wrapping. Past u64::MAX it read the stream again from its start while `check()` saw no error.

## [0.2.0] - 2026-11-14

### Fixed

- The edge-preserving filter of a frame with chroma subsampling reads each colour plane with its own stride. When the width is no multiple of 16 the chroma planes' stride differs from luma's after upsampling, and valid files decoded with wrong colours.
- Plane sizes are counted without wrapping a 32-bit usize: on wasm32 a huge frame is `ERR_NOMEM` instead of a plane that is too small.
- The spline area estimate saturates instead of wrapping. A huge quantizer adjustment or colour factor wrapped it small and let the area limit pass.

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
