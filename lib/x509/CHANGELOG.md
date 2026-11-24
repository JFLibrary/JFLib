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
- `PrivateKey`: Ed25519, ECDSA (P-256 / P-384) and RSA keys from PKCS #8, SEC 1 and PKCS #1 DER (ECDSA without a branch on the secret, checked to be in [1, n - 1] and against a stored public key), written as PKCS #8; `public_key`, `spki`, `sign(alg, data, random)` (ECDSA as DER with RFC 6979 and `random` mixed in, RSA PKCS #1 v1.5 or PSS), `cert_alg`, `can`; `spki_of(&PubKey)`. `CertBuilder::spki` and `CertBuilder::sign_key`: certificates of any of these keys, signed with any of them (ECDSA with SHA-256 / SHA-384, RSA PKCS #1 v1.5 with SHA-256, Ed25519)

### Changed

- `PubKey::Rsa` holds n and e without leading zero bytes, as `PrivateKey::public_key` gives them (a certificate's INTEGER kept its sign byte)

