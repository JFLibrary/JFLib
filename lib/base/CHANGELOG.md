# Changelog of jflib-base

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- `Settings::set`, `set_at` and `reset` report whether the value or the file changed: setting the default removes every line of the key, also one that already read as the default (an explicit default, an invalid value), and that counts as a change (`dirty`, `revision`). A text is kept as the file holds it -- CR, LF and NUL as blanks, no blanks around it -- and a value whose line would be longer than 64 KiB is not set (false)
- `Ini::set` refuses a line longer than 64 KiB (`key=value`, a new `[section]`) and drops the blanks around a value, as reading does: `get` after `set` gives what a reload gives

### Fixed

- `Settings`: a reset left a line of the key in the file when the value already read as the default, and only the first line when the key was there twice -- after a restart the old value came back. A text with a line break or blanks around it read back otherwise after a restart, and `text()` of a key outside the schema ignored its maximum
- `Ini::set` accepted a line longer than 64 KiB that the file then read as a malformed line (the key gone, the keys after a long header in the section before), and `Ini::sections` compared every header with every name before it (a file of 50 000 headers: seconds)

## [0.2.0] - 2026-11-14

### Added

- `xml::Limits` (`bytes`, `nodes`, `depth`) and `xml::parse_with(text, limits)`: the XML reader
  within limits the caller names. `Limits::default()` is what `xml::parse` keeps to (4 MiB,
  30000 elements, 48 levels of nesting).

### Changed

- `base::Ini::load_checked` gives None when it cannot tell that the name is free (refused, an I/O error, a folder there) or cannot learn the file's size; only a missing name gives an empty Ini. `file_exists_checked` on a folder mount answers for its OS file (it gave the mount's kind: 0 for an error too)

### Fixed

- `Ini::load_checked` returned an empty Ini for a file it could not open when asking whether it exists failed (refused, an I/O error), and for a file whose size was unknown: a later save replaced the settings it never read

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
