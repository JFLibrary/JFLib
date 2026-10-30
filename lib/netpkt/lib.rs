// netpkt -- network packets byte for byte: addresses, checksums, header codecs, a dissector
// and capture files.  No I/O, no clock, no randomness: everything is deterministic and works
// on every target (simulators, packet tools, tests).
//
//   addr      Mac, Ipv4, Ipv6, IpAddr, Prefix4, Prefix6, wildcard_match
//   checksum  Internet checksum (sum / finish / verify / pseudo_v4 / pseudo_v6 / update16),
//             crc32 (Ethernet FCS)
//   pkt       Pkt: a buffer with headroom, headers are prepended in place
//   eth       Ethernet II (EthView, EthHdr), 802.1Q tags (VlanTag, tag_of, tagged, untagged,
//             L2), 802.2 LLC
//   arp       ArpView, Arp (request / reply / probe / announce)          (RFC 826, 5227)
//   ipv4      Ipv4View, Ipv4Hdr, dec_ttl, fragment, Reassembly           (RFC 791, 1624)
//   ipv6      Ipv6View, Ipv6Hdr, upper_layer / ext_headers (extension header walk),
//             FragHdr, fragment, Reassembly6                             (RFC 8200, 5722)
//   icmp      IcmpView, Icmp, error_quote, name                          (RFC 792, 1191)
//   udp       UdpView, UdpHdr (push_v4 / push_v6 with checksum)          (RFC 768)
//   tcp       TcpView, TcpHdr, TcpOpts (MSS, WS, SACK, TS), seq_lt ...   (RFC 9293, 7323, 2018)
//   dhcp      DhcpView, Dhcp with typed options (Opt), MsgType           (RFC 2131, 2132, 3046)
//   dns       Message, Header, Question, Rr, RData, names with compression   (RFC 1035, 3596)
//   ntp       NtpView, Ntp, Ts (32.32 timestamps), offset_delay           (RFC 5905)
//   syslog    pri / bsd / ios formatting, parse                           (RFC 3164, IOS)
//   decode    stack / proto_mask / layers / info: layers with fields and byte ranges for
//             packet inspectors, P_* bits for filters, one-line summaries
//   pcapng    Writer (section, interfaces, enhanced packets), read
//   http      HTTP/1.1 without I/O: Parser (fed as bytes arrive, says when it needs more),
//             Head, Headers, framing, encode_request / encode_response / chunks (RFC 9112)
//   stp       BPDUs of STP and RSTP: Configuration, TCN and RST BPDUs with their flags,
//             BridgeId, PortId, path_cost, the 802.3 / LLC frame (802.1D-1998, 802.1D-2004)
//   rip       RipView, Rip, Rte, Auth: RIP requests and responses, version 2 and 1, the
//             authentication entry recognised                       (RFC 2453, 1058)
//   nat       address and port translation in place: Flow (what translation and filters look
//             at), rewrite / rewrite_quoted with incremental checksums (RFC 3022, 1624, 5508)
//   icmpv6    Icmp6View, Icmp6 (echo and the errors with their quote), Nd: router and neighbor
//             solicitations and advertisements, redirects, their options (link-layer
//             addresses, prefix information, MTU); error_quote, name   (RFC 4443, 4861, 4862)
//   ospf      OspfView, Packet (Hello, Database Description, Link State Request, Update,
//             Acknowledgment), LsaHeader, Lsa (router, network, summary, AS-external) with the
//             Fletcher checksum, compare (which instance is newer)      (RFC 2328, RFC 905)
//   addrsel   destination address selection: scope, the default policy table (precedence,
//             label), Dest and sort, the order getaddrinfo tries addresses in   (RFC 6724)
//   eapol     EapolView, Key: EAPOL-Key frames of the 4-way handshake (Key Information bits,
//             replay counter, nonces, MIC field, key data), the RSN element, KDEs (GTK,
//             IGTK); mic_input / set_mic for the caller's MIC (802.1X-2004, 802.11-2020 12.7)
//   bgp       BGP-4 messages: frame (a message out of a TCP stream), BgpView, Message (OPEN with
//             capabilities, UPDATE with its path attributes, NOTIFICATION, KEEPALIVE,
//             ROUTE-REFRESH), BgpErr (the NOTIFICATION a receiver sends)   (RFC 4271, 5492,
//             6793, 2918, 1997, 4486)
//   tftp      TftpView, Packet (RRQ, WRQ, DATA, ACK, ERROR, OACK) with their options, error
//             codes, netascii, looks_like (a transfer away from port 69)  (RFC 1350, 2347-2349)
//   ftp       Command, Reply (multi-line), Lines, ReplyReader, host_port (PORT, 227), eprt,
//             epsv_port (229), the reply texts as the server's own words  (RFC 959, 2428, 2389)
//   ber       ASN.1 BER as SNMP uses it: Oid (dotted, lexicographic order), Value (the SMI's
//             types and the exceptions of SNMPv2), Reader / Writer, strict lengths  (X.690, RFC 3417)
//   snmp      SNMPv1 and v2c messages: community, the PDUs Get, GetNext, GetBulk, Set, Response,
//             the v1 Trap, SNMPv2-Trap, Inform and Report; error statuses and their v1 mapping,
//             the objects of notifications                       (RFC 1157, 1901, 3416, 3584)
//   mib       the objects of SNMPv2-MIB, IF-MIB, IP-MIB and IP-FORWARD-MIB an agent of the
//             simulator serves: names, OIDs, syntax, access, lookup by name, values as net-snmp's
//             tools print them                                   (RFC 3418, 2863, 4293, 2096)
//
// Views (XxxView::parse) check lengths once and then read fields without copying; records
// (XxxHdr, Arp, Icmp) write headers, usually with push_onto / push_v4 into a Pkt.  Multi-byte
// fields are big endian (network order); be16 / be32 / put16 / put32 below read and write them.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod addr;
pub mod arp;
pub mod checksum;
pub mod decode;
pub mod dhcp;
pub mod dns;
pub mod eth;
pub mod icmp;
pub mod ipv4;
pub mod ipv6;
pub mod ntp;
pub mod pcapng;
pub mod pkt;
pub mod syslog;
pub mod tcp;
pub mod udp;
pub mod http;
pub mod stp;
pub mod rip;
pub mod nat;
pub mod icmpv6;
pub mod ospf;
pub mod addrsel;
pub mod eapol;
pub mod bgp;
pub mod tftp;
pub mod ftp;
pub mod ber;
pub mod snmp;
pub mod mib;

pub use addr::{wildcard_match, IpAddr, Ipv4, Ipv6, Mac, Prefix4, Prefix6};
pub use pkt::Pkt;

/// the big-endian u16 at `off` (the caller has checked the length)
pub fn be16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}

/// the big-endian u32 at `off`
pub fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// writes a big-endian u16 at `off`
pub fn put16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_be_bytes());
}

/// writes a big-endian u32 at `off`
pub fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}
