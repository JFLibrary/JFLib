# Changelog of jflib-img-jbig2

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The JBIG2 decoder, written from ITU-T T.88: PDF's embedded form with its JBIG2Globals (`decode_embedded`) and JBIG2 files, sequential and random-access, every page (`decode_file`); generic regions (MMR; templates 0 to 3 with adaptive pixels and TPGDON; data of unknown length), refinement regions (templates 0 and 1, TPGRON), symbol dictionaries and text regions (arithmetic and Huffman with the standard and custom tables, refinement and aggregate coding, retained contexts, every reference corner, transposed), pattern dictionaries and halftone regions (skip, grids at any angle), intermediate, immediate and lossless regions, striped pages of unknown height, every combination operator; a damaged segment ends the page with what was decoded before it, a stream's work is bounded (four times `Options::max_pixels`), and a symbol dictionary or text region whose arithmetic data has run out stops there instead of decoding up to that bound from the fill; `img_jbig2::CODEC` for img_all.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
