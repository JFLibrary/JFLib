# Changelog of jflib-x509

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The crate: netio's X.509 and DER code (`netio::x509`, `netio::der`, which stay as paths), with
  its own `IpAddr` (the octets of an address, `parse`, the text form of RFC 5952, `from_octets`,
  `octets`), and what netpkt's certificates had: the subject's organization and the issuer's
  names, `fingerprint_bytes`, `fingerprint_text`, `self_signed`, `ed25519_key`, `name_list`,
  `bytes`. netpkt's own certificate parser is gone; `netpkt::x509` is this crate.

