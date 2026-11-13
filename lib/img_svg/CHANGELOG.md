# Changelog of jflib-img-svg

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

### Security

- A style sheet whose block stands behind many selectors that match every element is no longer
  copied per selector and per element: the selectors of a list share one copy, each declaration
  counts toward the cascade's step budget, and a cascade that would copy more than 128 MB of
  declarations makes `Svg::parse` return `ERR_LIMIT`. A 64 KB block behind 4096 selectors took
  minutes and gigabytes.
- Rendering is bounded by its work, not only by element visits: pixels painted, layers, masks and
  images allocated, path points, children and text walked count against 64 per pixel of the drawn
  area (at least 2^26), and drawing stops there. A `use` graph of a million visits, each covering
  the whole surface, took over a minute at 256 x 256 pixels.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
