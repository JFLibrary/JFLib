# Changelog of jflib-netchan

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- Message channels for games (G7d part 1): the same channel over UDP (unreliable, unordered) and over a WebSocket (reliable, ordered), one sequence number per packet and acks of the newest 33 received (`ClientEvent::Delivered` / `Lost`, `ServerEvent::Delivered` / `Lost`), the smoothed round-trip time. `Client::connect(Endpoint::Udp | Endpoint::WebSocket, ClientConfig)` with `poll`, `send`, `state`, `reliable`, `max_payload`, `stats`, `disconnect`; `Server::new(ServerConfig)` with `bind_udp`, `listen_ws`, `ws_endpoint` (`WsEndpoint::accept` in an own netio handler), `poll`, `wait` (sleep until something arrives), `send` (`Error::Backlog` when a WebSocket client has `server::LINK_UNSENT` bytes waiting), `disconnect`, `stats`, `peer`, `server_stats`. An unreliable connection that received `conn::ACK_EVERY` (16) packets without sending one owes an empty DATA packet at once, so its acks reach every packet. The UDP handshake: CONNECT padded to 64 bytes, CHALLENGE with a stateless HMAC-SHA256 cookie over address, nonce and time (10 s), RESPONSE, ACCEPT with a random token in every DATA and DISCONNECT; no answer before ACCEPT is larger than its request. Limits: `max_payload` (1200 bytes over UDP, at most `packet::MAX_UDP_PAYLOAD` 1400), `max_stream_payload` (64 KiB), `packets_per_s` and `bytes_per_s` per connection (UDP drops the excess, a WebSocket client is disconnected), `max_connections`, `max_per_ip`, `challenges_per_s`, `timeout_ms`, `keepalive_ms`, `handshake_ms`. `WebSocket`: one client type that never blocks, the browser's own (`plat::net::WebSocket`) or netio's on threads, waking the UI (`plat::wake`). Without I/O: `endpoint::ServerCore` and `ClientCore` (the caller's clock), `conn::Conn`, `packet::parse` and `Packet::write`. docs/TRANSPORT.md

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
