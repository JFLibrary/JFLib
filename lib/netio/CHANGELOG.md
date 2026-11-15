# Changelog of jflib-netio

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.1] - 2026-11-15

### Added

- `Stream::end_is_clean()` (default true): after a read gave 0, whether the peer ended the stream in order. `TlsStream` answers whether close_notify came.
- `x509::CertBuilder::sign_ext` (further extensions; one with the OID of a built-in one replaces it) and `x509::extension`; `server::MIN_BODY_RATE` and `http::Reader::set_min_rate`

### Fixed

- Server: a request body sent a byte at a time (each within `read_ms`) held a worker as long as the client liked; a body must now bring `MIN_BODY_RATE` (1 KiB a second) on average after a first `read_ms`, else the connection ends
- TLS: records that carry no data (empty application data records) kept `TlsStream::read` going without end, past the read timeout and the HTTP deadlines, which are checked between reads; a read now ends with TimedOut after the stream's read timeout without data
- X.509: a wildcard certificate (`*.example.com`) passed a CA's excluded name constraint for a host it stands for (`secret.example.com`) or for the subdomains of its base (`.example.com`); and the extended key usage of the CAs on the path was not checked, so a CA for client certificates (also a trusted one) could certify a server

### Security

- A response without a length, which runs to the end of the connection, ends only at an orderly end. A reset, or a TLS connection closed without close_notify, is an error instead of the whole body.
- SSH `exec`: the names of `exit-signal` requests count against `max_out` like the output.

## [0.2.0] - 2026-11-14

### Fixed

- `Client::new()` sends the user agent `JFLib-netio` and `ssh::Client` the version `SSH-2.0-JFLib_netio` (both still named CraftLIB); a mail without a HELO name gets its Message-ID at `jflib` instead of `backupcraft`
- `smtp::send` with `Security::None`: the docs (GUIDE section 17, the README) say what the code does, a plain connection to any server (a relay on port 25 inside a network) and no login without TLS; they said "plain on loopback"

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
