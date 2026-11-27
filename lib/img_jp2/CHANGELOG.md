# Changelog of jflib-img-jp2

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The JPEG 2000 decoder, written from ITU-T T.800: JP2 and JPX files and raw codestreams (J2K); every progression order and POC, tiles and tile-parts in any order, precincts, layers, SOP / EPH, PPM / PPT, every code-block style (bypass, reset, terminate-all, vertically causal, predictable termination, segmentation symbols), ROI max-shift, the 5/3 and 9/7 wavelets with any image and tile offsets, RCT and ICT, sub-sampled components, 1 to 24 bits, signed and unsigned; from the JP2 header the colour space (sRGB, grey, sYCC converted to RGB, CMYK, CIELab, an ICC profile), the palette and the channel definitions (opacity, premultiplied). `decode(data, &Options) -> Result<Image, Error>` with `Options::reduce` (lower resolutions), `max_pixels` and `convert_sycc` (false: sYCC stays as coded); damaged files give what was decoded up to the damage, limits on pixels, code-blocks and precincts refuse hostile ones; the same bits on every target; `img_jp2::CODEC` for img_all (8-bit ARGB; CIELab files are `ERR_UNSUPPORTED` there).

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
