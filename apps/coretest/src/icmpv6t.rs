// icmpv6t -- checks of netpkt::icmpv6 and what IPv6 adds around it: echo and the four errors
// with their quote (RFC 4443), the messages of Neighbor Discovery with their options byte for
// byte (RFC 4861: RS, RA with SLLA, MTU and prefix information, NS for address resolution and
// for duplicate address detection, NA with its flags), the validation a receiver does, the
// solicited-node groups and EUI-64 identifiers of SLAAC (RFC 4291, 4862), the fragment header
// with fragmentation and reassembly (RFC 8200, overlaps after RFC 5722), and the dissector
// (fields, byte ranges, checksums, one-line summaries).  The expected messages were computed
// independently (Python: struct, ipaddress and the RFC 1071 sum).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV6};
use netpkt::icmpv6::{self, Icmp6, Icmp6View, Nd, NdErr, NdMsg, NdOpt, PrefixInfo, Ra};
use netpkt::ipv6::{self, Ipv6Hdr, Ipv6View, ReasmErr, Reassembly6};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::{Ipv6, Mac};

use crate::T;

/// bytes from hex digits separated by spaces
fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace().filter_map(|x| u8::from_str_radix(x, 16).ok()).collect()
}

fn a6(s: &str) -> Ipv6 {
    Ipv6::parse(s).unwrap_or_default()
}

const MAC1: Mac = Mac([0x02, 0x4A, 0, 0, 1, 1]);
const MAC2: Mac = Mac([0x02, 0x4A, 0, 0, 2, 1]);

/// an ICMPv6 message in a frame (IPv6 with the given hop limit, Ethernet to the group's MAC or
/// to `dmac`)
fn frame(src: Ipv6, dst: Ipv6, hop: u8, msg: &[u8], dmac: Mac, smac: Mac) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, msg);
    Ipv6Hdr::new(src, dst, ipv6::NH_ICMPV6, hop).push_onto(&mut p);
    EthHdr { dst: dmac, src: smac, ethertype: ET_IPV6 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

pub fn run(t: &mut T) {
    addresses(t);
    echo_and_errors(t);
    neighbor_discovery(t);
    validation(t);
    fragments(t);
    dissector(t);
}

// solicited-node groups, their MACs, EUI-64 link-local and SLAAC addresses
fn addresses(t: &mut T) {
    let ll1 = Ipv6::link_local_of(MAC1);
    t.eq("link-local of 02:4a:00:00:01:01", format!("{}", ll1), String::from("fe80::4a:ff:fe00:101"));
    t.eq("eui-64 identifier", MAC1.eui64(), [0x00, 0x4A, 0x00, 0xFF, 0xFE, 0x00, 0x01, 0x01]);
    t.eq("slaac address", format!("{}", Ipv6::eui64_in(a6("2001:db8:1::"), MAC2)), String::from("2001:db8:1:0:4a:ff:fe00:201"));
    let sn = ll1.solicited_node();
    t.eq("solicited-node group", format!("{}", sn), String::from("ff02::1:ff00:101"));
    t.check("is solicited-node", sn.is_solicited_node() && !Ipv6::ALL_NODES.is_solicited_node() && !ll1.is_solicited_node());
    t.eq("group mac", Mac::of_ipv6_multicast(sn), Mac([0x33, 0x33, 0xFF, 0x00, 0x01, 0x01]));
    t.eq("scopes", (Ipv6::ALL_NODES.mc_scope(), a6("ff05::2").mc_scope(), a6("ff0e::1").mc_scope()), (2, 5, 14));
    t.check("global unicast", a6("2001:db8::1").is_global_unicast() && a6("fd00::1").is_global_unicast() && !ll1.is_global_unicast() && !Ipv6::ALL_NODES.is_global_unicast() && !Ipv6::UNSPECIFIED.is_global_unicast() && !Ipv6::LOCALHOST.is_global_unicast());
    t.eq("common prefix", (a6("2001:db8:1::1").common_prefix(&a6("2001:db8:1::2")), a6("2001:db8::").common_prefix(&a6("3001:db8::")), ll1.common_prefix(&ll1)), (126, 3, 128));
    let p = netpkt::Prefix6::parse("2001:db8:1::/48").unwrap_or_default();
    t.check("prefixes overlap", p.overlaps(&netpkt::Prefix6::parse("2001:db8:1:5::/64").unwrap_or_default()) && !p.overlaps(&netpkt::Prefix6::parse("2001:db8:2::/64").unwrap_or_default()));
}

// echo request and reply, the four errors with their quote
fn echo_and_errors(t: &mut T) {
    let (s, d) = (a6("2001:db8::1"), a6("2001:db8::2"));
    let e = Icmp6::EchoRequest { id: 0x1234, seq: 1 }.encode(s, d, b"abcd");
    t.eq("echo request bytes", &e, &unhex("80 00 4d 48 12 34 00 01 61 62 63 64"));
    let mut p = Pkt::new(HEADROOM, b"abcd");
    Icmp6::EchoRequest { id: 0x1234, seq: 1 }.push_v6(&mut p, s, d);
    t.eq("push_v6 = encode", p.bytes(), &e[..]);
    match Icmp6View::parse(&e) {
        Some(v) => {
            t.eq("echo fields", (v.typ(), v.code(), v.id(), v.seq(), v.body()), (128, 0, 0x1234, 1, &b"abcd"[..]));
            t.check("echo checksum", v.checksum_ok(s, d) && !v.checksum_ok(d, a6("2001:db8::3")) && !v.is_error() && !v.is_nd());
            t.eq("echo read", Icmp6::read(&v), Icmp6::EchoRequest { id: 0x1234, seq: 1 });
        }
        None => t.check("echo parses", false),
    }
    let reply = Icmp6::EchoReply { id: 0x1234, seq: 1 }.encode(d, s, b"abcd");
    t.eq("echo reply type and checksum", (reply[0], Icmp6View::parse(&reply).is_some_and(|v| v.checksum_ok(d, s))), (129, true));
    // the offending packet: the echo request with hop limit 1
    let req = Ipv6Hdr::new(s, d, ipv6::NH_ICMPV6, 1).packet(&e);
    let quote = icmpv6::error_quote(&req);
    let r = a6("2001:db8:1::1");
    let te = Icmp6::TimeExceeded { code: icmpv6::EXCEEDED_HOP_LIMIT }.encode(r, s, quote);
    let want_q = "60 00 00 00 00 0c 3a 01 20 01 0d b8 00 00 00 00 00 00 00 00 00 00 00 01 20 01 0d b8 00 00 00 00 00 00 00 00 00 00 00 02 80 00 4d 48 12 34 00 01 61 62 63 64";
    t.eq("time exceeded", &te, &unhex(&format!("03 00 07 4d 00 00 00 00 {}", want_q)));
    let ptb = Icmp6::PacketTooBig { mtu: 1280 }.encode(r, s, quote);
    t.eq("packet too big", &ptb, &unhex(&format!("02 00 03 4d 00 00 05 00 {}", want_q)));
    let pu = Icmp6::DestUnreach { code: icmpv6::UNREACH_PORT }.encode(d, s, quote);
    t.eq("port unreachable", &pu, &unhex(&format!("01 04 09 49 00 00 00 00 {}", want_q)));
    let pp = Icmp6::ParamProblem { code: icmpv6::PARAM_NEXT_HEADER, pointer: 6 }.encode(d, s, quote);
    t.eq("parameter problem", &pp, &unhex(&format!("04 01 06 46 00 00 00 06 {}", want_q)));
    match Icmp6View::parse(&ptb) {
        Some(v) => {
            t.check("an error", v.is_error() && v.checksum_ok(r, s));
            t.eq("mtu", (v.value(), Icmp6::read(&v)), (1280, Icmp6::PacketTooBig { mtu: 1280 }));
            let q = v.quoted();
            t.eq("quoted packet", q.map(|q| (q.src(), q.dst(), q.hop_limit(), q.payload().len())), Some((s, d, 1, 12)));
        }
        None => t.check("ptb parses", false),
    }
    t.eq("read param problem", Icmp6View::parse(&pp).map(|v| Icmp6::read(&v)), Some(Icmp6::ParamProblem { code: 1, pointer: 6 }));
    // the quote stops at 1232 bytes: the error fits into the minimum MTU
    let big = Ipv6Hdr::new(s, d, ipv6::NH_UDP, 64).packet(&[0u8; 1400]);
    t.eq("long quote", icmpv6::error_quote(&big).len(), 1232);
    let cut = Icmp6::DestUnreach { code: 0 }.encode(r, s, icmpv6::error_quote(&big));
    t.check("a cut quote still reads", Icmp6View::parse(&cut).and_then(|v| v.quoted()).is_some_and(|q| q.is_truncated() && q.payload().len() == 1192 && q.payload_len() == 1400));
    t.eq("names", (icmpv6::name(1, 4), icmpv6::name(3, 0), icmpv6::name(2, 0), icmpv6::name(135, 0)), ("Destination Unreachable (port unreachable)", "Time Exceeded (hop limit exceeded in transit)", "Packet Too Big", "Neighbor Solicitation"));
}

// RS, RA, NS (resolution and DAD), NA byte for byte, parsed back
fn neighbor_discovery(t: &mut T) {
    let (ll1, ll2) = (Ipv6::link_local_of(MAC1), Ipv6::link_local_of(MAC2));
    let sol2 = ll2.solicited_node();
    let ns = Nd::ns(ll2, Some(MAC1)).encode(ll1, sol2);
    t.eq("ns bytes", &ns, &unhex("87 00 75 bb 00 00 00 00 fe 80 00 00 00 00 00 00 00 4a 00 ff fe 00 02 01 01 01 02 4a 00 00 01 01"));
    let f = frame(ll1, sol2, 255, &ns, Mac::of_ipv6_multicast(sol2), MAC1);
    t.eq("ns frame", &f, &unhex("33 33 ff 00 02 01 02 4a 00 00 01 01 86 dd 60 00 00 00 00 20 3a ff fe 80 00 00 00 00 00 00 00 4a 00 ff fe 00 01 01 ff 02 00 00 00 00 00 00 00 00 00 01 ff 00 02 01 87 00 75 bb 00 00 00 00 fe 80 00 00 00 00 00 00 00 4a 00 ff fe 00 02 01 01 01 02 4a 00 00 01 01"));
    let dad = Nd::ns(ll1, None).encode(Ipv6::UNSPECIFIED, ll1.solicited_node());
    t.eq("dad probe", &dad, &unhex("87 00 7a db 00 00 00 00 fe 80 00 00 00 00 00 00 00 4a 00 ff fe 00 01 01"));
    let na = Nd::na(ll2, false, true, true, Some(MAC2)).encode(ll2, ll1);
    t.eq("na bytes", &na, &unhex("88 00 12 f5 60 00 00 00 fe 80 00 00 00 00 00 00 00 4a 00 ff fe 00 02 01 02 01 02 4a 00 00 02 01"));
    let rs = Nd::rs(Some(MAC1)).encode(ll1, Ipv6::ALL_ROUTERS);
    t.eq("rs bytes", &rs, &unhex("85 00 78 98 00 00 00 00 01 01 02 4a 00 00 01 01"));
    let pi = PrefixInfo { len: 64, on_link: true, auto: true, valid: 2_592_000, preferred: 604_800, prefix: a6("2001:db8:1::") };
    let ra_msg = Nd::new(NdMsg::RouterAdvert(Ra { hop_limit: 64, lifetime: 1800, ..Ra::default() }), alloc::vec![NdOpt::SrcLla(MAC2), NdOpt::Mtu(1500), NdOpt::Prefix(pi)]);
    let ra = ra_msg.encode(ll2, Ipv6::ALL_NODES);
    t.eq("ra bytes", &ra, &unhex("86 00 ea 54 40 00 07 08 00 00 00 00 00 00 00 00 01 01 02 4a 00 00 02 01 05 01 00 00 00 00 05 dc 03 04 40 c0 00 27 8d 00 00 09 3a 80 00 00 00 00 20 01 0d b8 00 01 00 00 00 00 00 00 00 00 00 00"));
    let rf = frame(ll2, Ipv6::ALL_NODES, 255, &ra, Mac::of_ipv6_multicast(Ipv6::ALL_NODES), MAC2);
    t.eq("ra frame", rf.len(), 118);
    t.eq("ra frame head", &rf[..14], &unhex("33 33 00 00 00 01 02 4a 00 00 02 01 86 dd")[..]);
    // read back
    let back = |b: &[u8]| Icmp6View::parse(b).map(|v| Nd::parse(&v));
    t.eq("ns parsed", back(&ns), Some(Ok(Nd::ns(ll2, Some(MAC1)))));
    t.eq("dad parsed", back(&dad), Some(Ok(Nd::ns(ll1, None))));
    t.eq("na parsed", back(&na), Some(Ok(Nd::na(ll2, false, true, true, Some(MAC2)))));
    t.eq("rs parsed", back(&rs), Some(Ok(Nd::rs(Some(MAC1)))));
    t.eq("ra parsed", back(&ra), Some(Ok(ra_msg.clone())));
    t.eq("ra options", (ra_msg.src_lla(), ra_msg.mtu(), ra_msg.prefixes().count(), ra_msg.tgt_lla()), (Some(MAC2), Some(1500), 1, None));
    t.check("all checksums", [(&ns, ll1, sol2), (&dad, Ipv6::UNSPECIFIED, ll1.solicited_node()), (&na, ll2, ll1), (&rs, ll1, Ipv6::ALL_ROUTERS), (&ra, ll2, Ipv6::ALL_NODES)].iter().all(|(m, s, d)| Icmp6View::parse(m).is_some_and(|v| v.checksum_ok(*s, *d) && v.is_nd())));
    // flags, preference, a redirect, an unknown option kept
    let ra2 = Nd::new(NdMsg::RouterAdvert(Ra { hop_limit: 0, managed: true, other: true, prf: 1, lifetime: 0, reachable: 30_000, retrans: 1000 }), alloc::vec![NdOpt::Other(14, alloc::vec![1, 2, 3, 4, 5, 6])]);
    let rb = ra2.encode(ll2, Ipv6::ALL_NODES);
    t.eq("ra flags", (rb[5], &rb[16..24]), (0xC8, &[14u8, 1, 1, 2, 3, 4, 5, 6][..]));
    t.eq("ra2 parsed", back(&rb), Some(Ok(ra2)));
    let red = Nd::new(NdMsg::Redirect { target: ll2, dst: a6("2001:db8:5::9") }, alloc::vec![NdOpt::TgtLla(MAC2), NdOpt::Redirected(alloc::vec![0x60, 0, 0, 0, 0, 0, 0, 0])]);
    let redb = red.encode(Ipv6::link_local_of(Mac([2, 0x4A, 0, 0, 3, 1])), ll1);
    t.eq("redirect length", redb.len(), 40 + 8 + 16);
    t.eq("redirected header option", &redb[48..56], &[4u8, 2, 0, 0, 0, 0, 0, 0][..]);
    t.eq("redirect parsed", back(&redb), Some(Ok(red)));
}

// what a receiver drops
fn validation(t: &mut T) {
    let (ll1, ll2) = (Ipv6::link_local_of(MAC1), Ipv6::link_local_of(MAC2));
    let parse = |b: &[u8]| Icmp6View::parse(b).map(|v| Nd::parse(&v));
    let mut ns = Nd::ns(ll2, Some(MAC1)).encode(ll1, ll2.solicited_node());
    ns[1] = 1;
    t.eq("code not 0", parse(&ns), Some(Err(NdErr::Code(1))));
    let short = [135u8, 0, 0, 0, 0, 0, 0, 0, 0xFE, 0x80];
    t.eq("too short", parse(&short), Some(Err(NdErr::Short)));
    let mut zero = Nd::rs(Some(MAC1)).encode(ll1, Ipv6::ALL_ROUTERS);
    zero[9] = 0;
    t.eq("option of length 0", parse(&zero), Some(Err(NdErr::BadOption)));
    let mut past = Nd::rs(Some(MAC1)).encode(ll1, Ipv6::ALL_ROUTERS);
    past[9] = 2;
    t.eq("option past the end", parse(&past), Some(Err(NdErr::BadOption)));
    let group = Nd::ns(Ipv6::ALL_NODES, None).encode(ll1, Ipv6::ALL_NODES);
    t.eq("group as target", parse(&group), Some(Err(NdErr::Multicast)));
    t.eq("not nd", Icmp6View::parse(&Icmp6::EchoRequest { id: 1, seq: 1 }.encode(ll1, ll2, &[])).map(|v| Nd::parse(&v)), Some(Err(NdErr::NotNd)));
    // the rules that need the addresses and the hop limit
    let ra = Nd::new(NdMsg::RouterAdvert(Ra { hop_limit: 64, lifetime: 1800, ..Ra::default() }), Vec::new());
    t.eq("ra from a global address", ra.check_ip(a6("2001:db8::1"), Ipv6::ALL_NODES, 255), Some(NdErr::Source));
    t.eq("hop limit 64", ra.check_ip(ll2, Ipv6::ALL_NODES, 64), Some(NdErr::HopLimit));
    t.eq("its text", NdErr::HopLimit.text(), "hop limit is not 255: it did not come from this link");
    t.eq("good ra", ra.check_ip(ll2, Ipv6::ALL_NODES, 255), None);
    let dad = Nd::ns(ll1, None);
    t.eq("dad to a unicast address", dad.check_ip(Ipv6::UNSPECIFIED, ll1, 255), Some(NdErr::DadDest));
    t.eq("good dad", dad.check_ip(Ipv6::UNSPECIFIED, ll1.solicited_node(), 255), None);
    t.eq("dad with an address option", Nd::ns(ll1, Some(MAC1)).check_ip(Ipv6::UNSPECIFIED, ll1.solicited_node(), 255), Some(NdErr::SourceLla));
    t.check("solicited na to a group", Nd::na(ll2, false, true, true, None).check_ip(ll2, Ipv6::ALL_NODES, 255) == Some(NdErr::SolicitedToGroup) && Nd::na(ll2, false, false, true, None).check_ip(ll2, Ipv6::ALL_NODES, 255).is_none());
}

// the fragment header: a 3000-byte echo into 1280-byte fragments and back
fn fragments(t: &mut T) {
    let (s, d) = (a6("2001:db8::1"), a6("2001:db8:2::2"));
    let data: Vec<u8> = (0..3000u32).map(|i| (i & 0xFF) as u8).collect();
    let m = Icmp6::EchoRequest { id: 7, seq: 3 }.encode(s, d, &data);
    let pkt = Ipv6Hdr::new(s, d, ipv6::NH_ICMPV6, 64).packet(&m);
    t.eq("small packets stay whole", ipv6::fragment(&pkt[..], 9000, 1).map(|v| v.len()), Some(1));
    let parts = ipv6::fragment(&pkt, 1280, 0x0A0B_0C0D).unwrap_or_default();
    t.eq("fragment sizes", parts.iter().map(|p| p.len()).collect::<Vec<_>>(), alloc::vec![1280, 1280, 40 + 8 + 3008 - 2 * 1232]);
    let hdrs: Vec<(usize, bool, u32, u8)> = parts.iter().filter_map(|p| Ipv6View::parse(p)).filter_map(|v| ipv6::frag(&v).map(|(f, _)| (f.offset, f.more, f.id, f.next))).collect();
    t.eq("fragment headers", hdrs, alloc::vec![(0, true, 0x0A0B_0C0D, 58), (1232, true, 0x0A0B_0C0D, 58), (2464, false, 0x0A0B_0C0D, 58)]);
    t.eq("next header is 44", parts.iter().map(|p| p[6]).collect::<Vec<_>>(), alloc::vec![44, 44, 44]);
    t.eq("fragment header bytes", &parts[1][40..48], &[58u8, 0, 0x04, 0xD1, 0x0A, 0x0B, 0x0C, 0x0D][..]);
    // reassembled in any order
    let mut r = Reassembly6::new();
    let mut whole = None;
    for i in [2usize, 0, 1] {
        let Some(v) = Ipv6View::parse(&parts[i]) else { continue };
        let Some((f, at)) = ipv6::frag(&v) else { continue };
        t.eq("key", ipv6::reassembly_key(&v, &f), (s, d, 0x0A0B_0C0D));
        if let Ok(Some(p)) = r.add(&v, &f, at) {
            whole = Some(p);
        }
    }
    t.eq("reassembled", whole.as_deref(), Some(&pkt[..]));
    // an overlap drops the packet (RFC 5722), a bad length is a parameter problem
    let mut r = Reassembly6::new();
    let v0 = Ipv6View::parse(&parts[0]);
    let mut odd = parts[1].clone();
    odd[42] = 0x04;
    odd[43] = 0xC9; // offset 1224: overlaps the first fragment
    let v1 = Ipv6View::parse(&odd);
    if let (Some(a), Some(b)) = (v0, v1) {
        let (fa, aa) = ipv6::frag(&a).unwrap_or((ipv6::FragHdr { next: 0, offset: 0, more: false, id: 0 }, 0));
        let (fb, ab) = ipv6::frag(&b).unwrap_or((fa, 0));
        t.eq("first fragment", r.add(&a, &fa, aa), Ok(None));
        t.eq("overlap", r.add(&b, &fb, ab), Err(ReasmErr::Overlap));
        t.eq("dead after an overlap", r.add(&a, &fa, aa), Err(ReasmErr::Overlap));
    }
    let mut bad = parts[0].clone();
    bad.truncate(1279);
    netpkt::put16(&mut bad, 4, 1239);
    if let Some(v) = Ipv6View::parse(&bad)
        && let Some((f, at)) = ipv6::frag(&v)
    {
        t.eq("data not a multiple of 8", Reassembly6::new().add(&v, &f, at), Err(ReasmErr::BadLength));
    }
    t.eq("ext headers", Ipv6View::parse(&parts[0]).and_then(|v| ipv6::ext_headers(&v)).map(|e| e.iter().map(|x| (x.kind, x.off, x.len, x.next)).collect::<Vec<_>>()), Some(alloc::vec![(44, 0, 8, 58)]));
    t.eq("upper layer after the fragment header", Ipv6View::parse(&parts[0]).and_then(|v| ipv6::upper_layer(&v)), Some((58, 8)));
    let mut hl = pkt.clone();
    t.eq("hop limit down", (ipv6::dec_hop_limit(&mut hl), hl[7]), (63, 63));
}

// layers, fields with their bytes, problems and summaries
fn dissector(t: &mut T) {
    let (ll1, ll2) = (Ipv6::link_local_of(MAC1), Ipv6::link_local_of(MAC2));
    let sol2 = ll2.solicited_node();
    let ns = Nd::ns(ll2, Some(MAC1)).encode(ll1, sol2);
    let f = frame(ll1, sol2, 255, &ns, Mac::of_ipv6_multicast(sol2), MAC1);
    t.eq("ns stack", decode::stack(&f).layers().iter().map(|l| l.0).collect::<Vec<_>>(), alloc::vec![Proto::Eth, Proto::Ipv6, Proto::Icmpv6]);
    t.check("ns mask", decode::proto_mask(&f) & (decode::P_IPV6 | decode::P_ICMPV6) == decode::P_IPV6 | decode::P_ICMPV6);
    t.eq("ns info", decode::info(&f), (Proto::Icmpv6, String::from("Neighbor Solicitation for fe80::4a:ff:fe00:201 from 02:4a:00:00:01:01")));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == Proto::Icmpv6);
    t.check("ns layer", l.is_some_and(|l| l.problems.is_empty() && l.off == 54 && l.len == 32));
    let field = |name: &str| l.and_then(|l| l.fields.iter().find(|x| x.name == name)).map(|x| (x.value.clone(), x.off, x.len));
    t.eq("type field", field("Type"), Some((String::from("135 (Neighbor Solicitation)"), 54, 1)));
    t.eq("checksum field", field("Checksum"), Some((String::from("0x75bb [correct]"), 56, 2)));
    t.eq("target field", field("Target Address"), Some((String::from("fe80::4a:ff:fe00:201"), 62, 16)));
    t.eq("option field", field("Source link-layer address"), Some((String::from("02:4a:00:00:01:01"), 78, 8)));
    // a broken checksum and a hop limit that is not 255
    let mut bad = f.clone();
    bad[57] ^= 0xFF;
    let lb = decode::layers(&bad);
    t.check("bad checksum shown", lb.iter().any(|l| l.proto == Proto::Icmpv6 && l.problems.iter().any(|p| p.starts_with("bad ICMPv6 checksum (should be 0x75bb)"))));
    let f64 = frame(ll1, sol2, 64, &ns, Mac::of_ipv6_multicast(sol2), MAC1);
    t.check("hop limit 64 shown", decode::layers(&f64).iter().any(|l| l.proto == Proto::Icmpv6 && l.problems.iter().any(|p| p.contains("hop limit is not 255"))));
    // advertisements and echo
    let na = Nd::na(ll2, true, true, true, Some(MAC2)).encode(ll2, ll1);
    t.eq("na info", decode::info(&frame(ll2, ll1, 255, &na, MAC1, MAC2)).1, String::from("Neighbor Advertisement fe80::4a:ff:fe00:201 (rtr, sol, ovr) is at 02:4a:00:00:02:01"));
    let pi = PrefixInfo { len: 64, on_link: true, auto: true, valid: icmpv6::INFINITE, preferred: 604_800, prefix: a6("2001:db8:1::") };
    let ra = Nd::new(NdMsg::RouterAdvert(Ra { hop_limit: 64, lifetime: 1800, ..Ra::default() }), alloc::vec![NdOpt::SrcLla(MAC2), NdOpt::Prefix(pi)]).encode(ll2, Ipv6::ALL_NODES);
    let rf = frame(ll2, Ipv6::ALL_NODES, 255, &ra, Mac::of_ipv6_multicast(Ipv6::ALL_NODES), MAC2);
    t.eq("ra info", decode::info(&rf).1, String::from("Router Advertisement from 02:4a:00:00:02:01"));
    let rl = decode::layers(&rf);
    let pfx = rl.iter().flat_map(|l| l.fields.iter()).find(|x| x.name == "Prefix information").map(|x| x.value.clone());
    t.eq("prefix field", pfx, Some(String::from("2001:db8:1::/64, on-link, autonomous, valid lifetime infinity, preferred lifetime 604800 s")));
    let life = rl.iter().flat_map(|l| l.fields.iter()).find(|x| x.name == "Router lifetime").map(|x| (x.value.clone(), x.off, x.len));
    t.eq("router lifetime", life, Some((String::from("1800 s"), 60, 2)));
    let (s, d) = (a6("2001:db8::1"), a6("2001:db8::2"));
    let e = Icmp6::EchoRequest { id: 0x1234, seq: 1 }.encode(s, d, b"abcd");
    let ef = frame(s, d, 64, &e, MAC2, MAC1);
    t.eq("echo info", decode::info(&ef).1, String::from("Echo (ping) request id=0x1234, seq=1, hop limit=64"));
    let req = Ipv6Hdr::new(s, d, ipv6::NH_ICMPV6, 1).packet(&e);
    let te = Icmp6::TimeExceeded { code: 0 }.encode(a6("2001:db8:1::1"), s, &req);
    let tf = frame(a6("2001:db8:1::1"), s, 64, &te, MAC1, MAC2);
    t.eq("time exceeded info", decode::info(&tf).1, String::from("Time Exceeded (hop limit exceeded in transit)"));
    let quoted = decode::layers(&tf).iter().flat_map(|l| l.fields.iter()).find(|x| x.name == "Quoted packet").map(|x| x.value.clone());
    t.eq("quoted field", quoted, Some(String::from("2001:db8::1 -> 2001:db8::2, ICMPv6")));
    // a fragmented echo: the first fragment shows the start of ICMPv6, the later ones data
    let data: Vec<u8> = (0..2000u32).map(|i| i as u8).collect();
    let big = Ipv6Hdr::new(s, d, ipv6::NH_ICMPV6, 64).packet(&Icmp6::EchoRequest { id: 9, seq: 9 }.encode(s, d, &data));
    let parts = ipv6::fragment(&big, 1280, 0x42).unwrap_or_default();
    let fr: Vec<Vec<u8>> = parts
        .iter()
        .map(|p| {
            let mut q = Pkt::new(HEADROOM, p);
            EthHdr { dst: MAC2, src: MAC1, ethertype: ET_IPV6 }.push_onto(&mut q);
            q.into_vec()
        })
        .collect();
    t.eq("first fragment stack", fr.first().map(|x| decode::stack(x).layers().iter().map(|l| (l.0, l.2)).collect::<Vec<_>>()), Some(alloc::vec![(Proto::Eth, 14), (Proto::Ipv6, 48), (Proto::Icmpv6, 1232)]));
    t.eq("later fragment stack", fr.get(1).map(|x| decode::stack(x).layers().iter().map(|l| l.0).collect::<Vec<_>>()), Some(alloc::vec![Proto::Eth, Proto::Ipv6, Proto::Data]));
    t.eq("later fragment info", fr.get(1).map(|x| decode::info(x).1), Some(String::from("IPv6 fragment (off=1232 more=n ident=0x00000042 nxt=58)")));
    let fh = fr.first().map(|x| decode::layers(x)).and_then(|ls| ls.iter().flat_map(|l| l.fields.iter()).find(|x| x.name == "Fragment Header").map(|x| (x.value.clone(), x.off, x.len)));
    t.eq("fragment header field", fh, Some((String::from("offset 0, more fragments, identification 0x00000042, next header ICMPv6 (58)"), 54, 8)));
    let fc = fr.first().map(|x| decode::layers(x)).and_then(|ls| ls.iter().find(|l| l.proto == Proto::Icmpv6).map(|l| (l.problems.len(), l.fields.iter().find(|x| x.name == "Checksum").map(|x| x.value.clone()))));
    t.check("no checksum verdict on a first fragment", fc.is_some_and(|(n, v)| n == 0 && v.is_some_and(|v| v.ends_with("[the message goes on in later fragments]"))));
}
