# Changelog of jflib-img-core

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- JDF_RAW_CMYK: a 4-component JPEG's samples as stored (YCCK after its YCC step, as libjpeg gives them) in the bytes R, G, B and A, for a caller with its own CMYK conversion
- `IMGF_JP2`: the slot of JPEG 2000, the one before `IMGF_SVG`, outside the static table (img_all fills it with img_jp2's codec); `register_slot` takes it as it takes `IMGF_SVG`, and `register_external` stops before it, so the external codecs keep their numbers

### Changed

- `inflate`, `deflate`, `gzip` and `checksum` call jflib-compress, where the code lives now (`compress::deflate`, `zlib`, `gzip`, `crc32`, `adler32`); their functions keep their names, signatures, bytes and `ERR_*` codes. The internal code tables `inflate::{LBASE, LEXT, DBASE, DEXT, CLORDER, rev16}` are gone. The crate's licence is MIT or Apache-2.0 again; the zlib, miniz and stb_image notices moved with the code.

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

### Changed

- The format table gives Paint.NET files (`IMGF_PDN`) the MIME type `image/x-paint.net`; it was
  `image/x-PaintCraft`, a leftover of a renaming.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
