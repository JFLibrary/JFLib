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

### Fixed

- Tile-part and PPM lengths saturate: on 32-bit targets a tile-part length near 2^32 wrapped in front of the part (the parts went round forever) and a PPM length could slice backwards
- The layer-resolution progressions (LRCP, RLCP) pass only over the components that have precincts at a resolution, and each pass over a layer and a resolution counts against the budget of the packets looked at: 8192 empty components with 65535 layers and 33 resolutions (25 KB) took 50 s, 7281 POC volumes over them ran for days (now 0.1 s, with packets the budget ends them after about 1.5 s).
- A tile's structures (resolutions, precincts with their code-blocks and tag trees, the coded data, the progression lists) are counted in bytes, at most the size of the samples (at least 64 MB); a band without code-blocks in a precinct makes no tag trees: 2 million empty packets of 1 x 1 precincts (2 MB of zeros) committed 2.4 GB, now Error::Limit at 106 MB.
- Memory: Options::max_mem bounds what a decode holds at once (the samples, a tile's planes, its structures; 0, the default, adds no limit) and img_all's codec takes it from ImgDec::maxmem less the ARGB surface (it was ignored); the planes go into the image one component after another (the first three wait for each other under a multiple-component transform), a component's code-blocks are freed after its plane and bands without data are not allocated: a 4-component 8192 x 8192 file without data committed 2.3 GB, now 1.35 GB.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
