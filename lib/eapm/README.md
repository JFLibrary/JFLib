# jflib-eapm

Two EAP methods for [JFLib](https://jflib.org), EAP-MD5 and EAP-PWD, as state machines without
I/O. EAP is the login protocol behind 802.1X: a device that wants onto a wired port or a
WPA-Enterprise network authenticates with it, and the switch or access point passes the packets
on to a RADIUS server. This crate holds both ends of the method, the peer (the device) and the
server, for simulators, test tools and devices of one's own. It is plain Rust, without the
standard library and without a line of `unsafe` code.

## How it works

A `Peer` and a `Server` take the EAP packets they receive and give back the packet to send. They
never touch a socket and never look at a clock: whoever holds them carries the bytes (in EAPOL
frames, in RADIUS attributes), keeps the timers and repeats a request that got no answer.
Randomness comes from the caller too, so a run with the same random bytes is the same byte for
byte, which makes them easy to test.

```rust
use jflib::eapm::{Method, Peer, PeerOut, Server, ServerOut};
use jflib::netpkt::eap;

let mut peer = Peer::new("anna", "secret", Method::Pwd);
let mut server = Server::new("radius", &[Method::Pwd, Method::Md5]); // offered in this order
let lookup = |user: &str| (user == "anna").then(|| String::from("secret"));
let mut rnd = |b: &mut [u8]| jflib::plat::fill_random(b);        // or a seeded generator in tests

let mut request = eap::Packet::identity_request(1, "").to_bytes();
loop {
    let PeerOut::Send(response) = peer.on_packet(&request, &mut rnd) else { break };
    match server.on_packet(&response, &lookup, &mut rnd) {
        ServerOut::Send(next) => request = next,
        ServerOut::Success { eap, msk, .. } => {
            peer.on_packet(&eap, &mut rnd); // now peer.msk() is the same key as msk
            break;
        }
        _ => break, // Failure or nothing to send
    }
}
```

The server starts with the first identity response, offers its methods in order and moves on to
the one a peer names in a Nak. The peer answers a repeated request with the same bytes, as
RFC 3748 asks, and ignores a Success that comes before its method is over. EAP-PWD (RFC 5931)
proves the password without sending it and derives the MSK and EMSK that WPA-Enterprise needs;
when the two sides do not share the password, the peer ends the method without an answer, as
the RFC says. EAP-MD5 is the simple challenge and response of wired ports and derives no keys
(`Method::makes_keys`).

The packets themselves are `netpkt::eap` of [`jflib-netpkt`](https://crates.io/crates/jflib-netpkt),
which also has the EAPOL and RADIUS framing around them, and the elliptic-curve sums of EAP-PWD
are `crypto::eappwd` of [`jflib-crypto`](https://crates.io/crates/jflib-crypto).

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::eapm`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-eapm/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Network programs in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  where port access control shows EAP, EAPOL and RADIUS together
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#eapm): every public
  item of `eapm`, also on [docs.rs](https://docs.rs/jflib-eapm)

## License

MIT or Apache-2.0, at your choice.
