# Changelog of jflib-netio

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Fixed

- `Client::new()` sends the user agent `JFLib-netio` and `ssh::Client` the version `SSH-2.0-JFLib_netio` (both still named CraftLIB); a mail without a HELO name gets its Message-ID at `jflib` instead of `backupcraft`
- `smtp::send` with `Security::None`: the docs (GUIDE section 17, the README) say what the code does, a plain connection to any server (a relay on port 25 inside a network) and no login without TLS; they said "plain on loopback"

## [0.2.0] - 2026-11-13

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
