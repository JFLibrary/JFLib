# jflib-netchan

Message channels for games, part of [JFLib](https://jflib.org): the same channel over UDP and over
a WebSocket, so a game server takes desktop players over UDP and browser players over a WebSocket
in one table. Every packet has a sequence number and acknowledges the newest 33 packets it
received, so the netcode learns which packets arrived over either transport. It is plain Rust,
without the standard library and without a line of `unsafe` code.

- **UDP** (Windows, Linux, macOS): unreliable and unordered, and encrypted. A connection
  handshake with a stateless cookie (nothing is kept for an address before it proved that it
  receives there, and no answer is larger than its request) agrees on keys of this connection
  alone (X25519); every packet after it is sealed with ChaCha20-Poly1305. Keep-alives and
  timeouts.
- **WebSocket** (everywhere, and the browser's only way): reliable and ordered, over the browser's
  own WebSocket (`plat::net::WebSocket` through the page's host) or netio's on the desktop.
- **Reliable messages** on both, beside the packets: any size up to 1 MiB, in order and exactly
  once, over UDP in chunks that are sent again until they arrive.
- **Limits:** the payload of a message (1200 bytes over UDP, 64 KiB over a WebSocket), packets and
  bytes a second per connection, connections per server and per address, handshake answers a
  second, the size, queue and pace of reliable messages.

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
        ClientEvent::Reliable(data) => { /* a message of the reliable channel */ }
        ClientEvent::Disconnected(reason) => {}
        _ => {}
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

## Reliable messages

`send` puts one message into one packet: it may arrive late, twice or not at all, and the
`Delivered` and `Lost` of its sequence number tell the netcode which. For what must arrive (the
map, the full state when a player joins, chat, events), `send_reliable(message)` on the client
and `send_reliable(id, message)` on the server take messages of any size up to `max_message`
(1 MiB). The other side gets each one exactly once, whole and in the order it was sent, as
`ClientEvent::Reliable(data)` or `ServerEvent::Reliable { id, data }`.

```rust
server.send_reliable(id, &map_bytes)?;              // 300 KB: about 250 packets over UDP
while let Some(event) = client.poll() {
    if let ClientEvent::Reliable(map) = event { /* the whole map */ }
}
```

Over UDP a message is cut into chunks of a packet each, in packets of their own kind that share
the sequence numbers and acks of the others. A chunk goes again when its packet was lost or no
ack came within a timeout from the measured round-trip time. The receiver acknowledges these
packets within 10 ms, and keeps chunks that come early until their turn. The chunks go out at
`reliable_bytes_per_s` (128 KiB a second), below what a server takes from one client by
default, so 1 MiB takes about six seconds. `send_reliable` refuses a message over `max_message`
(`Error::Size`) and one that would make more than `max_reliable_queued` (4 MiB) wait
(`Error::Backlog`). A peer that breaks the protocol of this channel is disconnected. Over a
WebSocket the same messages go in the same chunks and are never sent twice.

## Security over UDP

Someone who listens reads nothing, and a packet that was changed, forged or sent again is dropped
before the game sees it: the payload is encrypted, the header with the sequence number and the
acks is authenticated with it, and the nonce is the packet's full 64-bit number, so a replay
fails its tag or is found by the receive window. The keys are new for every connection (ephemeral
X25519 on both sides), so a key that leaks later does not open what was recorded before.

Ephemeral keys alone do not tell the client who answered: someone on the path could take the
server's place. Against that the server can have a static key, whose public half the client
knows (shipped with the game, or from its server list):

```rust
// once, kept by the server's operator: 32 random bytes
server.set_static_key(secret);
let public = server.public_key();       // give this to the clients

// the client: only the holder of the secret can finish the handshake
let cfg = ClientConfig { protocol: 0x47414d45, server_key: Some(public_key), ..Default::default() };
```

A client with the wrong key, or a server without one, ends with `Reason::ServerKey`. What stays
open: clients are not authenticated (that is the game's login), and someone on the path can always
drop packets. Over a WebSocket the channel has no encryption of its own; `wss://` is it.

`netchan::endpoint` has the protocol of both sides without I/O (`ServerCore`, `ClientCore`, the time
comes from the caller), for tests on a simulated network and for other transports; `netchan::packet`
the wire format, `netchan::conn` the sequence numbers and acks, `netchan::seal` the keys and the
seal.

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
