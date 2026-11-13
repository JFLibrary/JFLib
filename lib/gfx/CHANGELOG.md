# Changelog of jflib-gfx

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- `ellipse_fx` and `rrect_fx` compute their control offsets in 64 bits. From a radius of about 59,500 px (rounded rectangles: 73,000 px) they wrapped and the shape came out wrong.

## [0.2.0] - 2026-11-13

### Added

- `gfx::set_parallel(f)`, `gfx::parallel()` and the type `ParallelFor`: the loop large copies run
  on. JFLib's UI runtime passes its worker pool; without a loop the copies run on the calling
  thread, with the same bytes.
- `COPY_PAR_MIN` (1 MB) and `COPY_STREAM_MIN` (12 MB): the sizes from which `Dc::blit` copies in
  row bands on that loop, and with streaming stores.
- `blend::copy_stream_rows`: `copy_stream` for a block of rows, with one fence at the end.

### Changed

- `Dc::blit` copies an XRGB source of `COPY_PAR_MIN` bytes or more in row bands on the loop of
  `set_parallel`, from `COPY_STREAM_MIN` on with streaming stores; the bytes are the same. The
  blit of a 4K canvas frame (3840 x 2160) took 1.9 to 2.0 ms on the UI thread instead of 4.9 to
  5.9 ms (Windows; Linux 2.0 instead of 4.5 ms). Blits through dirty rectangles stay cut to them.

### Fixed

- `blend::copy_stream` ends with the fence its non-temporal stores need before anything reads or
  writes the copy again; callers no longer call `stream_fence` after it.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
