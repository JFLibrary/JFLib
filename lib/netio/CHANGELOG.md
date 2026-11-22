# Changelog of jflib-netio

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- WebSocket (RFC 6455, G7d): `ws::Acceptor` answers upgrade requests in a `Server`'s handler (`accept(&req, on_open)`: 101 and the connection on a thread of its own, or 400 / 403 for an origin not on `origins` / 426 / 503 past `max_connections`; subprotocols by `protocols`), `ws::connect(url, &ClientConfig)` for clients (ws://, wss:// through a connector; the server's Sec-WebSocket-Accept checked, extensions refused), `ws::WebSocket` with `recv` / `recv_timeout`, `send_text`, `send_binary`, `send_fragmented`, `ping`, `close`; pings answered, pongs taken, the close handshake both ways, a ping after `ping_ms` of quiet, the end after `idle_ms`. `ws::Decoder` reads frames of untrusted bytes without I/O within `ws::Limits` (frame, message, fragments) and names the close code of anything malformed (1002, 1007 for bad UTF-8, 1008, 1009); `ws::encode_frame`, `ws::apply_mask`, `ws::accept_key`, `ws::close_code_valid`, `ws::parse_url`. Messages and bytes a second per connection as token buckets (1008 beyond). docs/TRANSPORT.md
- `http::Body::Upgrade(http::Upgrade)`: with status 101 the server sends the head as the handler built it and hands the connection (`http::Upgraded`: the stream, the bytes read after the request, the peer, the server's stop flag) to the upgrade on a thread of its own; `Server::stop` waits for those threads, `Server::upgraded()` counts them. `http::write_switch`, `http::Reader::take_buffered`; `http::reason(101)`.

- `ssh::Client::set_time_limit(ms)`: each following `auth_password`, `auth_key` and `exec` ends with `TimedOut` after that time in all.
- `x509::NameConstraints`: `permitted_email` / `excluded_email`, `permitted_uri` / `excluded_uri`, `permitted_dn` / `excluded_dn`, `other_forms`; `x509::Cert`: `emails` (with the subject's emailAddress), `uris`, `dirs`, `other_names`
- TLS server: takes TLS_AES_128_GCM_SHA256 and P-256 (secp256r1) key shares besides ChaCha20-Poly1305 and X25519, as RFC 8446 9.1 requires of every TLS 1.3 implementation; the group is the first of the client's list that the server has (with a HelloRetryRequest for it when the client sent no share), a key share for a group the client does not list or a second share for a group is refused (illegal_parameter), and to a client that sent a legacy session id the server sends the dummy change_cipher_spec of middlebox compatibility (RFC 8446 D.4). Its certificate and signatures stay Ed25519
- TLS: `Trust::Custom(Verifier)`, a check of the peer's certificates the program brings (a PKI of its own, SPKI pinning, Certificate Transparency, the platform's verifier); a client offers with it what it offers with `Trust::System`

### Changed

- TLS: a Certificate message may hold 20 certificates (10 before), and an intermediate in it that cannot be read is left out instead of failing the handshake, as browsers do; the peer's own certificate must still be readable
- TLS 1.2: a server without the extended master secret is taken again (since the review of 2026-10-23 it was refused, and with it the TLS 1.2 servers of badssl.com and other old servers). The Triple Handshake attack the extension closes needs session resumption or renegotiation, which netio's client does neither of; resumption, once it comes, will require it
- `x509::Cert::ips` and `CertBuilder::ips` are `x509::IpAddr` (the same shape as `plat::net::IpAddr`; `IpAddr::from([u8; 4])` / `from([u8; 16])`, `from_octets`, `octets` convert); `netio::x509` and `netio::der` are the crate jflib-x509

### Fixed

- `dns::resolve` (Linux, macOS): a numeric address or `localhost` no longer reads /etc/hosts and /etc/resolv.conf, which an app's rules do not grant on macOS (PERMDENY lines for every WebSocket and HTTP connection to an address).
- SSH: a peer that sends only messages to skip (ignore, debug, banners, global requests) while an answer is due ends the session after 1024 of them (login and opening a channel never returned); data past the channel window the client granted is a protocol error.
- TLS: a plaintext alert during the handshake ends the connection like an encrypted one (it was answered with unexpected_message); a Finished of the wrong length is answered with decode_error instead of decrypt_error (RFC 8446 4.4.4)
- TLS server, as tlsfuzzer checks RFC 8446: an empty alert record is answered with unexpected_message, a record that holds only padding (no content type) with unexpected_message instead of bad_record_mac, a protected record over 2^14 + 1 bytes of inner plaintext (padding included) or a plaintext record over 2^14 with record_overflow; a ClientHello with supported_groups but no key_share with missing_extension, one with a legacy_version below 0x0303 with protocol_version; a second dummy change_cipher_spec, or one before the first ClientHello, with unexpected_message; a second ClientHello that differs from the first in more than key_share, cookie, early_data and padding with illegal_parameter (as does early_data in it); and 0-RTT data, which the server never takes, is skipped as RFC 8446 4.2.10 asks (up to 64 KiB) instead of ending the handshake; application data or a change_cipher_spec between the parts of a handshake message split over records with unexpected_message (RFC 8446 5.1; application data inside a KeyUpdate was delivered); an empty application_data record counts as 0-RTT data only when the client offered early data (one between the parts of a ClientHello was skipped and the handshake waited), and after a HelloRetryRequest nothing is skipped once the second ClientHello came (a record that does not open is bad_record_mac); a pre_shared_key that is not the last extension with illegal_parameter (RFC 8446 4.2.11)
- TLS server: a second ClientHello whose pre_shared_key has new binders or ticket ages, as RFC 8446 4.1.2 allows, is taken; it was refused with illegal_parameter, which ended the handshake of a client that holds a ticket (from an earlier server under the same name) after a HelloRetryRequest
- TLS client: an extension in the ServerHello or EncryptedExtensions that the ClientHello did not offer (an unknown type, a GREASE value) is refused with unsupported_extension, one it offered in a message that does not answer it with illegal_parameter (RFC 8446 4.2, RFC 5246 7.4.1.4, RFC 8701 4; `tls::conn::AL_UNSUPPORTED_EXTENSION`). They were ignored; TLS-Anvil found it
- TLS, as TLS-Anvil checks the client: a TLS 1.2 peer's close_notify is answered with one (RFC 5246 7.2.1); an empty handshake record, protected or not, is unexpected_message (RFC 8446 5.1; a protected one was taken, a plaintext one was decode_error); the client ignores a TLS 1.3 ServerHello's legacy_version (RFC 8446 4.2.1; anything but 0x0303 was protocol_version) and refuses one older than TLS 1.2 only without supported_versions
- `dns::resolve` on macOS and iOS asks the system's resolver (`getaddrinfo`, as `plat::net::resolve` does there) instead of reading /etc/hosts and /etc/resolv.conf: names behind a VPN with name servers of its own (split DNS), `.local` over mDNS and the rules of iOS resolve, and a sandboxed Mac app, which may not read those files, resolves names at all; the app's net.connect rules apply as before

### Security

- `ssh`: `private_key` overwrites the key file's decoded bytes and text, `auth_password` the message with the password, once used.
- `x509::Cert::verify` reads an ECDSA signature (certificates, TLS CertificateVerify) strictly as DER: nothing after the SEQUENCE or in it after s, r and s in the shortest form of a non-negative INTEGER. Bytes appended to the signature and zeros put before r or s were accepted, so a CA-signed certificate could be re-encoded into copies with other fingerprints that still validated (no forgery; Wycheproof's InvalidEncoding cases, tools/wycheproof_check.py now without a failure)
- TLS refuses handshake data that crosses any change of the read key (RFC 8446 5.1): the application keys after Finished and those of a KeyUpdate count too; before, only the first keys did
- X.509 closes what x509-limbo found: name constraints are processed for e-mail addresses, URIs (their host) and directory names too, and a constrained form netio does not process refuses the certificates below it (such constraints were dropped); they apply to the intermediates below a CA as well, not only to the leaf. Refused now: RSA keys under 2048 bits or not in whole bytes, an unknown critical extension in a trust anchor, an empty extended key usage, a subject alternative name that is empty, of an unknown form, an iPAddress of another length or a dNSName outside the preferred name syntax (an underscore, a leading dot), NameConstraints without subtrees, critical AKI / SKI, name constraints or keyCertSign in a leaf without cA, a CA with an empty subject. Self-issued intermediates no longer count against pathLenConstraint
- DNS: an answer counts only with addresses of the asked name, or of the name its CNAME chain in the answer leads to (at most 8 steps); records for other names in an answer were taken as well
- PEM (`pem::decode_all`) is read line by line, each line compared with the markers, instead of with a substring search across the text, whose steps depended on which Base64 characters a private key has (tools/ct_check.py's memcheck found it); a marker must stand on a line of its own (RFC 7468)

## [0.2.1] - 2026-11-16

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
