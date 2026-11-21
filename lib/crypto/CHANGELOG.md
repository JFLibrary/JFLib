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
- The masks of secret-dependent selects pass an optimization barrier (`ct_mask`). On x86-64 with Rust 1.98 LLVM had compiled the masked final subtractions of the P-256 field arithmetic and of the Ed25519 scalar reduction into branches, the select over all 16 entries of P-256's window table into a jump table indexed by the secret nibble, and at opt-level 2 with LTO (JFLib's release) the window select of `bn::Mont::pow_ct` into a branch: the time and the memory accesses of `p256::public_key`, `shared` and `mul_point` (EAP-PWD too), of Ed25519 signing and of `pow_ct` (RSA signing, the Diffie-Hellman of plat's Secret Service client on Linux) depended on the secret. Results are unchanged; P-256 is about 24 % slower
- EAP-PWD's hunting and pecking no longer branches on whether a round's x has a point: `p256::y_for_x` (variable time before) has no branch on x up to its answer, and `eappwd::pwe` uses its branch-free core, so which rounds find a point no longer shows in the way the program takes (the partitioning of Dragonblood, CVE-2019-9494, and the cache attacks after it). `p256::scalar_add` (the Scalar rand + mask of a commit), `p256::secret_from` and the subkeys of AES-CMAC select without branches as well; `tools/ct_check.py` found them
- Base64 and hex without a branch on a character or a value and without table lookups: `b64::decode` and `decode_url` look at every character before they answer, `b64::encode` and `hex` compute the characters instead of reading an alphabet at the value, `unhex` likewise. Private keys come this way from PEM files, OpenSSH key files and PHC strings (the attack of "Util::Lookup", 2021, on the key decoding of other libraries); the results are unchanged
- EAP-PWD's commit and shared secret no longer check the password element and the points made from it as if they came from outside: `point_of` branched on their secret coordinates (`p256::mul_point_known`, `add_points_known`; the peer's element is still checked) (tools/ct_check.py's memcheck found it)

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
