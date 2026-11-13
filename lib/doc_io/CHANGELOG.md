# Changelog of jflib-doc-io

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- A long formatted document can be saved: `encode_docx` (and so `save_docx`) reads its own output
  back, and the XML reader stopped at 30000 elements and 4 MiB a part, which some dozen pages of
  formatted text reach (`Error::Xml`). The parts that grow with a document's content (the text,
  the notes, the sources, the tables' identities) are now written and read up to 250000 elements
  and 8 MiB, the size a package holds in one part; `decode_docx` reads such documents too.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
