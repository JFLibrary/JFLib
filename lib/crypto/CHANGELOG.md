# Changelog of jflib-crypto

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- `argon2::VERIFY_WORK`: the most work `verify` does for a PHC string.

### Security

- `eappwd::shared` refuses a peer scalar of 1 (RFC 8146 2.1; hostapd since CVE-2019-9497). `argon2::verify` refuses a PHC string that asks for more than the new `VERIFY_WORK` (2^24 KiB times passes) instead of running it; 4 GiB with 2^32 - 1 passes passed `valid()`. `aes::Aes::new` overwrites the expanded key words with a store the compiler keeps.

## [0.2.1] - 2026-11-16

### Security

- eappwd::pwe always runs at least 40 hunting-and-pecking rounds and selects the first point without branching, so its time no longer shows how many rounds a password needs (Dragonblood, CVE-2019-9494); the element is unchanged

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
