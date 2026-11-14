# Changelog of jflib-doc-io

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- HTML clipboard round trips keep the size of raised and lowered text: the import took the shown script size for the nominal one, so every copy and paste shrank such text by a third. The nominal size travels in `data-jf-size`.
- HTML clipboard round trips keep the line spacing: the import cut the decimals of `line-height` and rounds them now.
- A source list that `encode_sources` / `save_sources` writes loads back: `decode_sources` / `load_sources` read with the same limits, 8 MiB and 250000 elements (they stopped at 4 MiB and 30000, so a long list could be saved but not opened); `decode_bibliography` / `load_bibliography` take Word XML up to 8 MiB too

## [0.2.0] - 2026-11-13

### Fixed

- A long formatted document can be saved: `encode_docx` (and so `save_docx`) reads its own output
  back, and the XML reader stopped at 30000 elements and 4 MiB a part, which some dozen pages of
  formatted text reach (`Error::Xml`). The parts that grow with a document's content (the text,
  the notes, the sources, the tables' identities) are now written and read up to 250000 elements
  and 8 MiB, the size a package holds in one part; `decode_docx` reads such documents too.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
