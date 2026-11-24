# jflib-x509

X.509 certificates for [JFLib](https://jflib.org), in plain Rust, without the standard library
and without a line of `unsafe` code. It is the one certificate code of the library: netio's TLS
and https (`netio::x509`) and netpkt's TLS core and simulators (`netpkt::x509`) are this crate.
It has no I/O and no clock; the caller passes the time.

## What's in it

- **Parsing, strictly:** `Cert::parse` reads DER (RFC 5280) with Ed25519, RSA and ECDSA P-256 /
  P-384 keys and signatures. Malformed or ambiguous encodings are refused: non-minimal integers
  and signature values, an empty extended key usage, subject alternative names that are empty,
  of an unknown form or not in the preferred name syntax, name constraints without subtrees,
  critical key identifiers, unknown critical extensions.
- **Path validation:** `verify_chain` builds a path from the leaf through the given
  intermediates (in any order) to a trust anchor: validity, CA flags, key usage, extended key
  usage of the CAs on the path, path length (self-issued intermediates not counted) and name
  constraints for DNS names, IP addresses, e-mail addresses, URIs and directory names, applied to
  the leaf and the intermediates. A constrained name form it does not process refuses the
  certificates below. RSA keys under 2048 bits are not taken.
- **Names:** `Cert::names(host)` checks a DNS name (one wildcard label at the left) or an IP
  address against the subject alternative names; the common name does not count.
- **A private CA:** `CertBuilder` builds and signs certificates, CAs and leaves with names, key
  usages and further extensions: Ed25519 ones with `sign` / `sign_ext`, ones of any key with
  `sign_key` (the subject's key as `spki`, signed with Ed25519, ECDSA or RSA).
- **Private keys:** `PrivateKey` holds an Ed25519, ECDSA (P-256 / P-384) or RSA key, reads PKCS #8,
  SEC 1 and PKCS #1 (the DER inside the PEM files of OpenSSL and the CAs), writes PKCS #8, gives
  the public key and its SubjectPublicKeyInfo and signs; netio's TLS signs its handshakes with it.
- **Dialogs and tools:** fingerprints (`fingerprint`, `fingerprint_bytes`, `fingerprint_text`),
  `name_list`, the subject's and issuer's names, `self_signed`.
- **DER:** the bounded reader and the writer certificates need (`x509::der`).

Checked against x509-limbo and Wycheproof's ECDSA DER cases
([docs/EXTERNAL_SUITES.md](https://github.com/JFLibrary/JFLib/blob/main/docs/EXTERNAL_SUITES.md)).

## What's new in 0.3.0

The crate is new: netio's certificate code on its own, which netpkt now uses as well instead of
a parser of its own. Since x509-limbo it processes name constraints for every name form and
applies them to intermediates too, and refuses short RSA keys, malformed subject alternative
names and non-minimal ECDSA signatures. Everything is in the
[changelog](https://docs.rs/crate/jflib-x509/latest/source/CHANGELOG.md).

## Not here

Revocation (CRL, OCSP), certificate policies, fetching missing intermediates (AIA).

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::x509` (and `jflib::netio::x509`). All of
JFLib's crates share one version and are released together. It runs on every platform JFLib
supports.

## License

MIT or Apache-2.0, at your choice.
