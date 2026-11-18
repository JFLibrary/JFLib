# jflib-netchan

Message channels for games, part of [JFLib](https://jflib.org): the same channel over UDP and over
a WebSocket, so a game server takes desktop players over UDP and browser players over a WebSocket
in one table. Every packet has a sequence number and acknowledges the newest 33 packets it
received, so the netcode learns which packets arrived over either transport. It is plain Rust,
without the standard library and without a line of `unsafe` code.

- **UDP** (Windows, Linux, macOS): unreliable and unordered. A connection handshake with a
  stateless cookie (nothing is kept for an address before it proved that it receives there, and
  no answer is larger than its request), a random token in every packet, keep-alives and timeouts.
- **WebSocket** (everywhere, and the browser's only way): reliable and ordered, over the browser's
  own WebSocket (`plat::net::WebSocket` through the page's host) or netio's on the desktop.
- **Limits:** the payload of a message (1200 bytes over UDP, 64 KiB over a WebSocket), packets and
  bytes a second per connection, connections per server and per address, handshake answers a
  second.

## How it is used

```rust
use jflib::netchan::{Client, ClientConfig, ClientEvent, Endpoint};

// desktop: Endpoint::Udp(addr); the browser: a WebSocket, "/game" on the page's own server
let mut client = Client::connect(Endpoint::WebSocket("/game".into()), ClientConfig { protocol: 0x47414d45, ..Default::default() })?;
// every frame
while let Some(event) = client.poll() {
    match event {
        ClientEvent::Connected => {}
        ClientEvent::Message { seq, data } => { /* a snapshot of the server */ }
        ClientEvent::Delivered(seq) => { /* the server has our packet seq */ }
        ClientEvent::Lost(seq) => {}
        ClientEvent::Disconnected(reason) => {}
    }
}
let seq = client.send(&input_bytes)?;
```

```rust
use jflib::netchan::{Server, ServerConfig, ServerEvent};

let mut server = Server::new(ServerConfig { protocol: 0x47414d45, ..Default::default() });
server.bind_udp(&udp_addr)?;            // desktop players
server.listen_ws(&http_addr, "/game")?; // browser players (or ws_endpoint() in an own netio handler)
while let Some(event) = server.poll() {
    if let ServerEvent::Message { id, data, .. } = event {
        server.send(id, &data)?;
    }
}
```

`netchan::endpoint` has the protocol of both sides without I/O (`ServerCore`, `ClientCore`, the time
comes from the caller), for tests on a simulated network and for other transports; `netchan::packet`
the wire format, `netchan::conn` the sequence numbers and acks.

## Platforms

Windows, Linux, macOS and iOS; the client in the browser (over a WebSocket only). On Android plat
has no sockets yet: `Error::Unsupported`. The server needs netio's HTTP server: not in the browser.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::netchan`. All of JFLib's crates share one
version and are released together. Its sockets go through the platform layer, so the network rules
of an app's `PERMISSIONS` file apply (`net.connect` for the server's address, `net.listen` for a
server's sockets; in the browser `net.connect` on the WebSocket URL's host and port).

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release (G7d of JFLib's game engine
plan). The [changelog](https://docs.rs/crate/jflib-netchan/latest/source/CHANGELOG.md) will say so.

## Documentation

- [Transport for games](https://github.com/JFLibrary/JFLib/blob/main/docs/TRANSPORT.md) (German):
  the design, the wire format, the limits, the tests and the measurements
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#netchan): every public
  item of `netchan`, also on [docs.rs](https://docs.rs/jflib-netchan)

## License

MIT or Apache-2.0, at your choice.
