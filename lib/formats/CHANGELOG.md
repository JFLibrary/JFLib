# Changelog of jflib-formats

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- `formats::json` (strict bounded JSON: exact numbers, member order kept, duplicate names and lone surrogates refused, `Limits` for bytes, nesting, values, strings and members, `stringify_pretty`), `formats::jmap` (typed records and string enums: `Field`, `Json`, `record!`, `wire_enum!`), `formats::xml` (a bounded, namespace-aware XML subset: `parse`, `parse_with(Limits)`, `escape`) and `formats::encoding` (UTF-8 with and without BOM, UTF-16 LE and BE). The code was `agent_core::json` and `agent_core::jmap`, `base::xml` and `base::encoding` until now and moved unchanged, with the same limits, errors and output; those paths remain as re-exports. The crate depends on nothing but `alloc`.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
