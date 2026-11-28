# Changelog of jflib-cidmap

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The predefined CMaps of ISO 32000-1 Table 118 for Adobe-GB1, -CNS1, -Japan1 and -Korea1 (59, from GB-EUC-H to UniKS-UTF16-V): `predefined(name)` gives a `CMap` with its code space (`code_len`), its codes' CIDs (`cid`, with the notdef ranges) and its ranges, with what `usecmap` takes in; `names()`. `to_unicode(collection)`: CID -> Unicode text, Adobe's Adobe-XX-UCS2 tables. `Collection` with `from_ordering`, `ordering` and `lang` (zh-Hans, zh-Hant, ja, ko). The data (261 KB, a zstd frame per collection) is made from Adobe's files (BSD-3-Clause) by `tools/cidmap_gen.py`.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
