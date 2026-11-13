# Changelog of jflib-netpkt

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Security

- `decode::layers` and `decode::info` no longer panic on packets from the network: the TLS hello
  and its server name stop at the end of the segment (a ClientHello split across segments was
  read up to its declared length, past the segment), the text of an FTP or POP3 reply is taken
  from the bytes after byte 4 (a character across byte 4 was cut inside it), and the fields of
  SMTP, POP3, FTP and IMAP lines with bytes that are no UTF-8 stay inside their line.
- `smtp::parse_path` compares the keyword as bytes and no longer cuts an argument inside a
  character.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
