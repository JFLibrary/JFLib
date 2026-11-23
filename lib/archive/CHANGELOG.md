# Changelog of jflib-archive

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- DEFLATE when writing: `write_with(entries, limits, Method::Deflate(level))` compresses with
  img_core's DEFLATE and is as deterministic as `write` (the same parts give the same bytes); a
  part that does not get smaller stays stored. `write` itself stays stored, byte for byte as
  before.
- ZIP64: `read` and `ZipFs` take ZIP64 end records, ZIP64 fields in central and local headers and
  64-bit data descriptors, and a version needed up to 4.5 (such archives were `Unsupported`).
  `write` and `write_with` add ZIP64 records only where an archive needs them: 65535 parts or
  more, a size or offset of 4 GiB or more.
- `Limits::ratio`: the highest ratio of a DEFLATE part's size to its compressed size, checked for
  parts over 1 MiB before they are inflated; a denser part is a ZIP bomb (`Limit`). The default,
  1032, is the most DEFLATE can reach and so refuses nothing. A part that claims more than that is
  `Invalid` before its buffer is allocated. `Limits` also derives `Debug`, `PartialEq` and `Eq`.
- `opc`, the Open Packaging Conventions (ECMA-376 Part 2) that DOCX and VSDX are built on, taken
  out of jflib-doc-io: `Package` (`read`, `from_entries`, `part`, `content_type`,
  `relationships`, `resolve`, `main_part`, `set_part`, `replace`, `remove_part`,
  `set_relationships`, `write`; parts nobody touched go back unchanged), `ContentTypes`,
  `Relationship`, `Rules` (macros, exact names, XML limits), `normalize` and `same_part` (part
  names compared ASCII case-insensitive and percent-escape aware), `rels_name`, `rels_source`,
  `parse_relationships` and `relationships_xml`.
- (JD17) archive::cfb reads compound files ([MS-CFB], Microsoft's open specification; the container of Office's binary formats): File::read(bytes, cfb::Limits) checks header, allocation tables and directory (sector numbers inside the file, every chain without loops, the sibling tree without cycles and each entry once, names as the specification allows, file, stream and directory bounded by Limits) and gives entries, children, find(&[path]) and stream(i); version 3 and 4, read only

### Fixed

- Archives with many parts: the checks for duplicate names and overlapping records take
  O(n log n) instead of quadratic time.

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
