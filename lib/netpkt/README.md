# jflib-netpkt

Network packets byte for byte, for [JFLib](https://jflib.org): addresses and checksums, the headers
from Ethernet to TCP, the messages of many application protocols, a dissector that names every
field the way capture tools do, and capture files that Wireshark opens. The crate does no I/O,
reads no clock and draws no random numbers: it turns bytes into records and records into bytes,
so everything it does is deterministic and the same on every target. It was written for a network
simulator and serves packet tools and tests just as well. Plain Rust, without the standard
library and without a line of `unsafe` code.

## Views and records

Reading and writing have two shapes. A view (`EthView`, `Ipv4View`, `TcpView`, ...) checks the
lengths once in `parse` and then reads the fields straight from the bytes, without copying. A
record (`EthHdr`, `Ipv4Hdr`, `UdpHdr`, ...) writes its header in front of a payload in a `Pkt`, a
buffer with room at the front, so a packet grows outwards from its payload without being moved,
and the checksums are filled in on the way:

```rust
use jflib::netpkt::{decode, eth::{self, EthHdr}, ipv4::{self, Ipv4Hdr}, pcapng, pkt, udp::UdpHdr, Ipv4, Mac, Pkt};

let (src, dst) = (Ipv4::new(10, 0, 0, 1), Ipv4::new(10, 0, 0, 53));
let mut p = Pkt::new(pkt::HEADROOM, b"payload");
UdpHdr::new(40000, 53).push_v4(&mut p, src, dst);       // with the checksum over the pseudo header
Ipv4Hdr::new(src, dst, ipv4::PROTO_UDP, 64).push_onto(&mut p);
EthHdr { dst: Mac::new(2, 0, 0, 0, 0, 2), src: Mac::new(2, 0, 0, 0, 0, 1), ethertype: eth::ET_IPV4 }.push_onto(&mut p);
let frame = p.into_vec();

for layer in decode::layers(&frame) {
    // layer.title, layer.fields (name, shown value, byte range), layer.problems
}
let mut file = pcapng::Writer::new("my tool");
let eth0 = file.add_interface("eth0", pcapng::LINKTYPE_ETHERNET, 65535);
file.packet(eth0, 0, &frame);                           // the time in nanoseconds, from the caller
let bytes = file.finish();                              // a .pcapng for Wireshark
```

The Internet checksum comes with pseudo headers and incremental updates (`checksum::update`, as
RFC 1624 describes), so a router or a NAT that changes a field does not have to sum the packet
again. `nat::rewrite` translates addresses and ports in place that way, and `nat::rewrite_quoted`
does the same inside the packet an ICMP error quotes.

## What it covers

- **Links and the network layer:** MAC and IP addresses with prefixes, Ethernet II with 802.1Q
  tags and LLC, ARP, IPv4 with fragmentation and reassembly, IPv6 with its extension headers,
  fragments and reassembly, ICMP, ICMPv6 with Neighbor Discovery, the BPDUs of STP and RSTP,
  EAPOL with the key frames of the WPA 4-way handshake, and destination address selection as
  RFC 6724 orders it.
- **Transport and routing:** UDP, TCP with its options, RIP, OSPF version 2 with its LSAs, and
  BGP-4 with its capabilities and path attributes.
- **Services:** DHCP, DNS, NTP, syslog, TFTP, FTP, HTTP/1.1 (a parser fed with bytes as they
  arrive), SMTP, POP3, IMAP, Internet messages with the MIME a plain text needs, Telnet, SNMP
  versions 1, 2c and 3 with BER and a small built-in MIB, RADIUS and EAP.
- **Secure protocols without I/O:** SSH-2 and TLS 1.3 as cores for a client or a server that take
  the bytes that arrived and give the bytes to send, with random bytes from a seed, so the same
  seed gives the same connection; X.509 certificates of a private Ed25519 PKI.
- **For browsers and clients:** URI references with relative resolution (RFC 3986), the form
  encoding of HTML, and cookies with a jar (RFC 6265).

The protocol modules check what they read and say what is wrong instead of guessing: BGP and
OSPF report what a receiver would refuse, DNS names with compression loops are refused, and BER
refuses indefinite lengths and object identifiers that are not in their shortest form.

## The dissector

`decode::layers(frame)` takes a frame apart into its layers, each with a title, every field with
its shown value and its byte range in the frame, and the problems a receiver would notice, such
as a checksum that does not fit. `decode::info(frame)` sums a frame up in one line, the way
capture tools list packets, and `decode::proto_mask(frame)` gives bits (`P_*`) for filters.
Together they are what a packet inspector needs; frames without a link header, as on serial
lines, go through `decode::stack_ip` and `decode::ip_mask`. `pcapng::read` reads capture files
back.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::netpkt`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports, the browser and
Android included. The blocking clients and servers on real sockets (HTTP, TLS, SSH) are in
[`jflib-netio`](https://crates.io/crates/jflib-netio), the EAP methods as state machines in
[`jflib-eapm`](https://crates.io/crates/jflib-eapm).

## What's new in 0.2.0

The dissector no longer panics on packets from the network: a TLS ClientHello split across TCP
segments, an FTP or POP3 reply with a character across its fourth byte, and lines with bytes
that are not UTF-8 are taken apart within their bytes. `smtp::parse_path` no longer cuts an
argument inside a character, and on 32-bit targets pcapng's block and packet lengths are
checked without wrapping. Everything is in the
[changelog](https://docs.rs/crate/jflib-netpkt/latest/source/CHANGELOG.md).

## Documentation

- [Network programs in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  with examples for most protocols
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#netpkt): every public
  item of `netpkt`, also on [docs.rs](https://docs.rs/jflib-netpkt)

## License

MIT or Apache-2.0, at your choice.
