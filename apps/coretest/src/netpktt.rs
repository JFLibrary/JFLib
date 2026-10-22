// netpktt -- checks of netpkt: RFC checksum vectors, address text forms (RFC 5952), header
// round trips (view -> record -> bytes), fragmentation and reassembly, the dissector and
// pcapng files.  Expected checksums of whole segments were computed independently (Python);
// the ICMP one is also the well-known value of a Windows ping (id 1, seq 1).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use netpkt::arp::{Arp, ArpView};
use netpkt::checksum;
use netpkt::decode::{self, Proto};
use netpkt::eth::{self, EthHdr, EthView, VlanTag, L2};
use netpkt::icmp::{self, Icmp, IcmpView};
use netpkt::ipv4::{self, Ipv4Hdr, Ipv4View, Reassembly};
use netpkt::ipv6::{Ipv6Hdr, Ipv6View};
use netpkt::pcapng;
use netpkt::tcp::{self, TcpHdr, TcpOpts, TcpView};
use netpkt::udp::{UdpHdr, UdpView};
use netpkt::{be16, IpAddr, Ipv4, Ipv6, Mac, Pkt, Prefix4, Prefix6};

use crate::T;

const MAC_A: Mac = Mac([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
const MAC_B: Mac = Mac([0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
const IP_A: Ipv4 = Ipv4([10, 0, 0, 1]);
const IP_B: Ipv4 = Ipv4([10, 0, 0, 2]);
const PING_DATA: &[u8] = b"abcdefghijklmnopqrstuvwabcdefghi";

pub fn run(t: &mut T) {
    checksums(t);
    addresses(t);
    ipv6_text(t);
    icmp_frame(t);
    udp_tcp(t);
    arp_vlan(t);
    fragments(t);
    errors(t);
    ipv6_udp(t);
    pcap(t);
    pkt_growth(t);
    nat_rewrite(t);
}

fn checksums(t: &mut T) {
    // RFC 1071 section 3 example
    let d = [0x00, 0x01, 0xF2, 0x03, 0xF4, 0xF5, 0xF6, 0xF7];
    t.eq("rfc1071 sum", checksum::fold(checksum::sum(0, &d)), 0xDDF2);
    t.eq("rfc1071 checksum", checksum::checksum(&d), 0x220D);
    // the same sum in two pieces (only the last may be odd)
    t.eq("sum split", checksum::fold(checksum::sum(checksum::sum(0, &d[..4]), &d[4..])), 0xDDF2);
    // a classic IPv4 header (checksum 0xb861)
    let mut h = [0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0xB8, 0x61, 0xC0, 0xA8, 0x00, 0x01, 0xC0, 0xA8, 0x00, 0xC7];
    t.check("ipv4 header verifies", checksum::verify(&h));
    let mut z = h;
    z[10] = 0;
    z[11] = 0;
    t.eq("ipv4 header checksum", checksum::checksum(&z), 0xB861);
    t.eq("crc32 check value", checksum::crc32(b"123456789"), 0xCBF4_3926);
    // TTL decrement with the incremental update equals a full recomputation
    let ttl = ipv4::dec_ttl(&mut h);
    t.eq("dec_ttl value", ttl, 0x3F);
    t.check("dec_ttl keeps the checksum valid", checksum::verify(&h));
    let mut z = h;
    z[10] = 0;
    z[11] = 0;
    t.eq("dec_ttl = recomputed", be16(&h, 10), checksum::checksum(&z));
    let mut one = h;
    one[8] = 1;
    let mut z = one;
    z[10] = 0;
    z[11] = 0;
    let ck = checksum::checksum(&z);
    one[10..12].copy_from_slice(&ck.to_be_bytes());
    t.eq("dec_ttl to 0", ipv4::dec_ttl(&mut one), 0);
    t.check("dec_ttl to 0 valid", checksum::verify(&one));
    t.eq("dec_ttl at 0 stays", ipv4::dec_ttl(&mut one), 0);
}

fn addresses(t: &mut T) {
    t.eq("v4 parse", Ipv4::parse("192.168.0.1"), Some(Ipv4([192, 168, 0, 1])));
    for bad in ["192.168.0.01", "256.1.1.1", "1.2.3", "1.2.3.4.5", "1..2.3", "a.b.c.d", " 1.2.3.4", ""] {
        t.eq(&format!("v4 reject {:?}", bad), Ipv4::parse(bad), None);
    }
    t.eq("v4 display", format!("{}", Ipv4::new(10, 20, 30, 40)), String::from("10.20.30.40"));
    t.eq("mask len 24", Ipv4::parse("255.255.255.0").and_then(|m| m.mask_len()), Some(24));
    t.eq("mask len 0", Ipv4::parse("0.0.0.0").and_then(|m| m.mask_len()), Some(0));
    t.eq("mask len 32", Ipv4::parse("255.255.255.255").and_then(|m| m.mask_len()), Some(32));
    t.eq("mask non-contiguous", Ipv4::parse("255.0.255.0").and_then(|m| m.mask_len()), None);
    t.eq("mask of 20", Ipv4::mask_of(20), Ipv4([255, 255, 240, 0]));
    let p = Prefix4::parse("10.1.2.3/24").unwrap_or_default();
    t.eq("prefix network", p.network(), Ipv4([10, 1, 2, 0]));
    t.eq("prefix broadcast", p.broadcast(), Ipv4([10, 1, 2, 255]));
    t.check("prefix contains", p.contains(Ipv4([10, 1, 2, 200])) && !p.contains(Ipv4([10, 1, 3, 1])));
    t.eq("prefix hosts /24", p.host_count(), 254);
    t.eq("prefix first/last", (p.first_host(), p.last_host()), (Ipv4([10, 1, 2, 1]), Ipv4([10, 1, 2, 254])));
    let p31 = Prefix4::parse("10.0.0.0/31").unwrap_or_default();
    t.eq("prefix /31 hosts", (p31.host_count(), p31.first_host(), p31.last_host()), (2, Ipv4([10, 0, 0, 0]), Ipv4([10, 0, 0, 1])));
    t.eq("prefix /32 hosts", Prefix4::parse("1.2.3.4").map(|p| p.host_count()), Some(1));
    t.eq("prefix display", format!("{}", p), String::from("10.1.2.3/24"));
    t.eq("prefix reject /33", Prefix4::parse("10.0.0.0/33"), None);
    t.eq("prefix reject /024", Prefix4::parse("10.0.0.0/024"), None);
    t.eq("prefix from mask", Prefix4::from_mask(IP_A, Ipv4([255, 255, 0, 0])).map(|p| p.len), Some(16));
    t.eq("prefix from wildcard", Prefix4::from_wildcard(IP_A, Ipv4([0, 0, 0, 255])).map(|p| p.len), Some(24));
    t.eq("prefix wildcard", p.wildcard(), Ipv4([0, 0, 0, 255]));
    let big = Prefix4::parse("10.0.0.0/8").unwrap_or_default();
    t.check("prefix covers", big.covers(&p) && !p.covers(&big) && big.overlaps(&p));
    // non-contiguous ACL wildcard: even hosts of 10.0.0.0/24
    t.check("wildcard even", netpkt::wildcard_match(Ipv4([10, 0, 0, 6]), Ipv4([10, 0, 0, 0]), Ipv4([0, 0, 0, 254])));
    t.check("wildcard odd", !netpkt::wildcard_match(Ipv4([10, 0, 0, 7]), Ipv4([10, 0, 0, 0]), Ipv4([0, 0, 0, 254])));
    t.check("v4 classes", Ipv4([172, 20, 1, 1]).is_private() && !Ipv4([172, 32, 1, 1]).is_private() && Ipv4([169, 254, 3, 4]).is_link_local() && Ipv4([239, 1, 1, 1]).is_multicast());
    // MAC text forms
    let m = Mac::parse("00:11:22:33:44:55");
    t.eq("mac colon", m, Some(MAC_A));
    t.eq("mac dash", Mac::parse("00-11-22-33-44-55"), Some(MAC_A));
    t.eq("mac cisco", Mac::parse("0011.2233.4455"), Some(MAC_A));
    t.eq("mac plain", Mac::parse("001122334455"), Some(MAC_A));
    t.eq("mac reject", Mac::parse("00:11:22:33:44"), None);
    t.eq("mac reject mixed", Mac::parse("00:11-22:33:44:55"), None);
    t.eq("mac display", format!("{}", MAC_B), String::from("02:aa:bb:cc:dd:ee"));
    t.eq("mac cisco out", MAC_A.cisco(), String::from("0011.2233.4455"));
    t.check("mac bits", Mac::BROADCAST.is_broadcast() && Mac::BROADCAST.is_multicast() && MAC_B.is_local() && !MAC_A.is_local());
    t.eq("mac of v4 group", Mac::of_ipv4_multicast(Ipv4([224, 0, 0, 5])), Mac([1, 0, 0x5E, 0, 0, 5]));
    t.eq("mac of v4 group high", Mac::of_ipv4_multicast(Ipv4([239, 255, 1, 2])), Mac([1, 0, 0x5E, 0x7F, 1, 2]));
    t.eq("mac of v6 group", Mac::of_ipv6_multicast(Ipv6::ALL_NODES), Mac([0x33, 0x33, 0, 0, 0, 1]));
    t.eq("mac u64", Mac::from_u64(MAC_B.to_u64()), MAC_B);
    t.eq("ipaddr v4", IpAddr::parse("10.0.0.1"), Some(IpAddr::V4(IP_A)));
    t.eq("ipaddr v6", IpAddr::parse("::1").map(|a| format!("{}", a)), Some(String::from("::1")));
}

fn ipv6_text(t: &mut T) {
    // RFC 5952 canonical forms
    let cases = [
        ("2001:0db8:0000:0000:0000:0000:0000:0001", "2001:db8::1"),
        ("2001:db8:0:0:1:0:0:1", "2001:db8::1:0:0:1"),
        ("2001:db8:0:1:1:1:1:1", "2001:db8:0:1:1:1:1:1"),
        ("2001:DB8::0:1", "2001:db8::1"),
        ("::", "::"),
        ("::1", "::1"),
        ("1::", "1::"),
        ("fe80::200:5aee:feaa:20a2", "fe80::200:5aee:feaa:20a2"),
        ("::ffff:192.0.2.1", "::ffff:192.0.2.1"),
        ("64:ff9b::192.0.2.33", "64:ff9b::c000:221"),
        ("1:0:0:2:0:0:0:3", "1:0:0:2::3"),
    ];
    for (i, o) in cases {
        t.eq(&format!("v6 {}", i), Ipv6::parse(i).map(|a| format!("{}", a)), Some(String::from(o)));
    }
    for bad in ["1:::2", "1:2:3:4:5:6:7:8:9", "12345::", "1::2::3", ":1:2", "1:2:3:4:5:6:7", "g::1", "::1.2.3"] {
        t.eq(&format!("v6 reject {:?}", bad), Ipv6::parse(bad), None);
    }
    let a = Ipv6::parse("2001:db8::1:2:3").unwrap_or_default();
    t.eq("solicited node", format!("{}", a.solicited_node()), String::from("ff02::1:ff02:3"));
    t.eq("link local eui64", format!("{}", Ipv6::link_local_of(MAC_A)), String::from("fe80::211:22ff:fe33:4455"));
    t.check("v6 classes", Ipv6::parse("fe80::1").is_some_and(|a| a.is_link_local()) && Ipv6::parse("fd00::1").is_some_and(|a| a.is_unique_local()) && Ipv6::ALL_ROUTERS.is_multicast());
    let p = Prefix6::parse("2001:db8:1::5/48").unwrap_or_default();
    t.eq("prefix6 network", format!("{}", p.network()), String::from("2001:db8:1::"));
    t.check("prefix6 contains", p.contains(Ipv6::parse("2001:db8:1:ffff::1").unwrap_or_default()) && !p.contains(Ipv6::parse("2001:db8:2::1").unwrap_or_default()));
    t.eq("v4 mapped", Ipv4([192, 0, 2, 1]).to_ipv6_mapped().to_ipv4_mapped(), Some(Ipv4([192, 0, 2, 1])));
}

/// an ICMP echo request as a Windows host sends it
fn ping_frame(ttl: u8) -> Vec<u8> {
    let mut p = Pkt::new(netpkt::pkt::HEADROOM, PING_DATA);
    Icmp::EchoRequest { id: 1, seq: 1 }.push_onto(&mut p);
    let mut ih = Ipv4Hdr::new(IP_A, IP_B, ipv4::PROTO_ICMP, ttl);
    ih.id = 0x1C46;
    ih.push_onto(&mut p);
    EthHdr { dst: MAC_B, src: MAC_A, ethertype: eth::ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

fn icmp_frame(t: &mut T) {
    let f = ping_frame(128);
    t.eq("ping frame length", f.len(), 14 + 20 + 8 + 32);
    let e = EthView::parse(&f);
    t.check("eth parse", e.is_some_and(|e| e.dst() == MAC_B && e.src() == MAC_A && e.ethertype() == eth::ET_IPV4));
    let ip = Ipv4View::parse(&f[14..]);
    t.check("ip parse", ip.is_some_and(|v| v.checksum_ok() && v.ttl() == 128 && v.proto() == ipv4::PROTO_ICMP && v.total_len() == 60));
    let ic = IcmpView::parse(&f[34..]);
    t.eq("icmp checksum (windows ping)", ic.map(|v| v.checksum()), Some(0x4D5A));
    t.check("icmp verifies", ic.is_some_and(|v| v.checksum_ok()));
    t.eq("icmp record", ic.map(|v| Icmp::read(&v)), Some(Icmp::EchoRequest { id: 1, seq: 1 }));
    let reply = Icmp::EchoReply { id: 1, seq: 1 }.encode(PING_DATA);
    t.check("icmp encode verifies", checksum::verify(&reply));
    // header record round trip: view -> record -> bytes
    if let Some(v) = ip {
        let h = Ipv4Hdr::read(&v);
        let mut out = vec![0u8; h.len()];
        h.write(&mut out, v.payload().len());
        t.eq("ipv4 record round trip", out.as_slice(), v.header());
    }
    // the dissector
    let st = decode::stack(&f);
    t.eq("stack", st.layers().iter().map(|l| l.0).collect::<Vec<_>>(), vec![Proto::Eth, Proto::Ipv4, Proto::Icmp]);
    t.eq("proto mask", decode::proto_mask(&f), decode::P_ETH | decode::P_IPV4 | decode::P_ICMP);
    let (top, info) = decode::info(&f);
    t.eq("info top", top, Proto::Icmp);
    t.eq("info text", info, String::from("Echo (ping) request id=0x0001, seq=1, ttl=128"));
    let ls = decode::layers(&f);
    t.eq("layers", ls.len(), 3);
    let in_frame = ls.iter().all(|l| l.fields.iter().all(|fl| (fl.off + fl.len) as usize <= f.len()));
    t.check("field ranges inside the frame", in_frame);
    t.check("no problems", ls.iter().all(|l| l.problems.is_empty()));
    let ttl = ls.get(1).and_then(|l| l.fields.iter().find(|fl| fl.name == "Time to Live"));
    t.check("ttl field range", ttl.is_some_and(|fl| fl.off == 22 && fl.len == 1 && fl.value == "128"));
    // a damaged header shows a problem with the right checksum
    let mut bad = f.clone();
    bad[24] ^= 0xFF;
    let ls = decode::layers(&bad);
    t.check("bad checksum reported", ls.get(1).is_some_and(|l| l.problems.iter().any(|p| p.contains("should be 0x"))));
    // Ethernet padding of a short frame shows as a trailer
    let mut arp = EthHdr { dst: Mac::BROADCAST, src: MAC_A, ethertype: eth::ET_ARP }.frame(&{
        let mut a = [0u8; 28];
        Arp::request(MAC_A, IP_A, IP_B).write(&mut a);
        a
    });
    eth::pad(&mut arp);
    t.eq("padded arp", arp.len(), 60);
    t.eq("arp trailer", decode::stack(&arp).trailer, 18);
}

fn udp_tcp(t: &mut T) {
    let mut p = Pkt::new(netpkt::pkt::HEADROOM, b"hello");
    UdpHdr::new(5353, 53).push_v4(&mut p, IP_A, IP_B);
    t.eq("udp checksum", be16(p.bytes(), 6), 0x92E1);
    let u = UdpView::parse(p.bytes());
    t.check("udp verifies", u.is_some_and(|u| u.checksum_ok_v4(IP_A, IP_B) && u.payload() == b"hello" && u.dst_port() == 53));
    let mut seg = p.bytes().to_vec();
    seg[9] ^= 1;
    t.check("udp corrupt detected", !UdpView::parse(&seg).is_some_and(|u| u.checksum_ok_v4(IP_A, IP_B)));
    seg[6] = 0;
    seg[7] = 0;
    t.check("udp zero checksum accepted (v4)", UdpView::parse(&seg).is_some_and(|u| u.checksum_ok_v4(IP_A, IP_B)));
    t.check("udp length bound", UdpView::parse(&p.bytes()[..7]).is_none());

    let opts = TcpOpts { mss: Some(1460), sack_ok: true, ts: Some((12345, 0)), wscale: Some(7), ..TcpOpts::default() };
    t.eq("tcp opts length", opts.len(), 20);
    let h = TcpHdr { src_port: 49152, dst_port: 80, seq: 1000, ack: 0, flags: tcp::SYN, window: 64240, urgent: 0, opts };
    let mut p = Pkt::empty();
    h.push_v4(&mut p, IP_A, IP_B);
    t.eq("tcp header length", p.len(), 40);
    t.eq("tcp syn checksum", be16(p.bytes(), 16), 0x3EA2);
    let v = TcpView::parse(p.bytes());
    t.check("tcp verifies", v.is_some_and(|v| v.checksum_ok_v4(IP_A, IP_B)));
    t.eq("tcp opts round trip", v.map(|v| v.opts()), Some(opts));
    t.eq("tcp record round trip", v.map(|v| TcpHdr::read(&v)), Some(h));
    t.eq("tcp seg_len syn", v.map(|v| v.seg_len()), Some(1));
    t.eq("tcp flags", tcp::flags_str(tcp::SYN | tcp::ACK), String::from("SYN, ACK"));
    t.check("seq compare wraps", tcp::seq_lt(0xFFFF_FFF0, 0x10) && tcp::seq_gt(0x10, 0xFFFF_FFF0) && tcp::seq_le(5, 5) && tcp::seq_ge(5, 5));
    // SACK blocks
    let mut o = TcpOpts::default();
    o.sack_n = 2;
    o.sack[0] = (100, 200);
    o.sack[1] = (300, 400);
    let mut b = vec![0u8; o.len()];
    o.write(&mut b);
    t.eq("sack round trip", TcpOpts::parse(&b), o);
    // malformed options end the walk without panicking
    t.eq("tcp opts malformed", TcpOpts::parse(&[2, 9, 1]), TcpOpts::default());
    // a full SYN frame through the dissector
    let mut ih = Ipv4Hdr::new(IP_A, IP_B, ipv4::PROTO_TCP, 64);
    ih.df = true;
    ih.push_onto(&mut p);
    EthHdr { dst: MAC_B, src: MAC_A, ethertype: eth::ET_IPV4 }.push_onto(&mut p);
    let f = p.into_vec();
    let (top, info) = decode::info(&f);
    t.eq("syn top", top, Proto::Tcp);
    t.eq("syn info", info, String::from("49152 \u{2192} 80 [SYN] Seq=1000 Win=64240 Len=0 MSS=1460 WS=128 SACK_PERM TSval=12345 TSecr=0"));
    t.check("syn layers clean", decode::layers(&f).iter().all(|l| l.problems.is_empty()));
}

fn arp_vlan(t: &mut T) {
    let req = Arp::request(MAC_A, IP_A, IP_B);
    let mut b = [0u8; 28];
    req.write(&mut b);
    let v = ArpView::parse(&b);
    t.eq("arp round trip", v.map(|v| Arp::read(&v)), Some(req));
    let rep = Arp::reply_to(&req, MAC_B, IP_B);
    t.check("arp reply", rep.tha == MAC_A && rep.tpa == IP_A && rep.spa == IP_B && rep.op == 2);
    t.check("arp gratuitous", Arp::announce(MAC_A, IP_A).is_gratuitous() && Arp::probe(MAC_A, IP_A).is_probe());
    let f = EthHdr { dst: Mac::BROADCAST, src: MAC_A, ethertype: eth::ET_ARP }.frame(&b);
    t.eq("arp info", decode::info(&f).1, String::from("Who has 10.0.0.2? Tell 10.0.0.1"));
    let mut rb = [0u8; 28];
    rep.write(&mut rb);
    let rf = EthHdr { dst: MAC_A, src: MAC_B, ethertype: eth::ET_ARP }.frame(&rb);
    t.eq("arp reply info", decode::info(&rf).1, String::from("10.0.0.2 is at 02:aa:bb:cc:dd:ee"));
    // 802.1Q
    let tagged = eth::tagged(&f, VlanTag { pcp: 5, dei: false, vid: 10 });
    t.eq("vlan tag", eth::tag_of(&tagged), Some(VlanTag { pcp: 5, dei: false, vid: 10 }));
    t.check("vlan l2", L2::parse(&tagged).is_some_and(|l| l.ethertype == eth::ET_ARP && l.off == 18 && l.tag.is_some_and(|t| t.vid == 10)));
    t.eq("vlan strip", eth::untagged(&tagged), f.clone());
    t.eq("vlan stack", decode::stack(&tagged).layers().iter().map(|l| l.0).collect::<Vec<_>>(), vec![Proto::Eth, Proto::Vlan, Proto::Arp]);
    t.eq("vlan tci", VlanTag::from_tci(VlanTag { pcp: 7, dei: true, vid: 4094 }.tci()), VlanTag { pcp: 7, dei: true, vid: 4094 });
    // 802.3 + LLC (a spanning tree BPDU)
    let mut stp = vec![0u8; 14 + 3 + 35];
    EthHdr { dst: Mac::STP, src: MAC_A, ethertype: 38 }.write(&mut stp);
    eth::Llc::STP.write(&mut stp[14..]);
    t.eq("stp stack", decode::stack(&stp).layers().iter().map(|l| l.0).collect::<Vec<_>>(), vec![Proto::Eth, Proto::Llc, Proto::Stp]);
}

fn udp_packet(len: usize, df: bool) -> Vec<u8> {
    let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
    let mut p = Pkt::new(netpkt::pkt::HEADROOM, &data);
    UdpHdr::new(40000, 9).push_v4(&mut p, IP_A, IP_B);
    let mut h = Ipv4Hdr::new(IP_A, IP_B, ipv4::PROTO_UDP, 64);
    h.id = 0x4242;
    h.df = df;
    h.push_onto(&mut p);
    p.into_vec()
}

fn fragments(t: &mut T) {
    let pkt = udp_packet(3000, false);
    t.eq("fits unchanged", ipv4::fragment(&pkt, 9000).map(|v| v.len()), Some(1));
    let frags = ipv4::fragment(&pkt, 1500).unwrap_or_default();
    t.eq("fragment count", frags.len(), 3);
    let offs: Vec<(usize, bool, usize)> = frags.iter().filter_map(|f| Ipv4View::parse(f).map(|v| (v.frag_off(), v.mf(), v.payload().len()))).collect();
    t.eq("fragment offsets", offs, vec![(0, true, 1480), (1480, true, 1480), (2960, false, 48)]);
    t.check("fragments valid", frags.iter().all(|f| f.len() <= 1500 && Ipv4View::parse(f).is_some_and(|v| v.checksum_ok())));
    t.eq("df refuses", ipv4::fragment(&udp_packet(3000, true), 1500).map(|v| v.len()), None);
    // reassembly in another order, with a duplicate
    let mut r = Reassembly::new();
    let views: Vec<Ipv4View> = frags.iter().filter_map(|f| Ipv4View::parse(f)).collect();
    t.eq("reasm 3rd", views.get(2).and_then(|v| r.add(v)), None);
    t.eq("reasm 1st", views.first().and_then(|v| r.add(v)), None);
    t.eq("reasm duplicate", views.get(2).and_then(|v| r.add(v)), None);
    let whole = views.get(1).and_then(|v| r.add(v));
    t.eq("reassembled = original", whole, Some(pkt.clone()));
    // fragmenting a fragment keeps absolute offsets
    let again = frags.first().and_then(|f| ipv4::fragment(f, 600)).unwrap_or_default();
    let offs: Vec<(usize, bool)> = again.iter().filter_map(|f| Ipv4View::parse(f).map(|v| (v.frag_off(), v.mf()))).collect();
    t.eq("refragment", offs, vec![(0, true), (576, true), (1152, true)]);
    // a fragment's payload in the dissector is data, not UDP
    let st = decode::stack(&EthHdr { dst: MAC_B, src: MAC_A, ethertype: eth::ET_IPV4 }.frame(&frags[1]));
    t.eq("fragment stack", st.top(), Proto::Data);
}

fn errors(t: &mut T) {
    let orig = udp_packet(100, false);
    let quote = icmp::error_quote(&orig);
    t.eq("quote length", quote.len(), 28);
    let msg = Icmp::Unreachable { code: icmp::UNREACH_PORT, mtu: 0 }.encode(quote);
    let v = IcmpView::parse(&msg);
    t.check("error verifies", v.is_some_and(|v| v.checksum_ok() && v.is_error()));
    let q = v.and_then(|v| v.quoted());
    t.check("quoted packet", q.is_some_and(|q| q.src() == IP_A && q.dst() == IP_B && q.proto() == ipv4::PROTO_UDP && q.is_truncated()));
    t.eq("error name", icmp::name(3, 3), "Destination unreachable (Port unreachable)");
    let fn_ = Icmp::Unreachable { code: icmp::UNREACH_FRAG_NEEDED, mtu: 1400 }.encode(quote);
    t.eq("frag needed mtu", IcmpView::parse(&fn_).map(|v| Icmp::read(&v)), Some(Icmp::Unreachable { code: 4, mtu: 1400 }));
    t.eq("strict view refuses a quote", Ipv4View::parse(quote).is_none(), true);
}

fn ipv6_udp(t: &mut T) {
    let (s, d) = (Ipv6::parse("2001:db8::1").unwrap_or_default(), Ipv6::parse("2001:db8::2").unwrap_or_default());
    let mut p = Pkt::new(netpkt::pkt::HEADROOM, b"hello v6");
    UdpHdr::new(1234, 53).push_v6(&mut p, s, d);
    Ipv6Hdr::new(s, d, 17, 64).push_onto(&mut p);
    let v = Ipv6View::parse(p.bytes());
    t.check("v6 parse", v.is_some_and(|v| v.src() == s && v.dst() == d && v.payload_len() == 16 && v.hop_limit() == 64));
    let u = v.and_then(|v| UdpView::parse(v.payload()));
    t.check("v6 udp checksum", u.is_some_and(|u| u.checksum_ok_v6(s, d)));
    EthHdr { dst: MAC_B, src: MAC_A, ethertype: eth::ET_IPV6 }.push_onto(&mut p);
    let f = p.into_vec();
    t.eq("v6 stack", decode::stack(&f).layers().iter().map(|l| l.0).collect::<Vec<_>>(), vec![Proto::Eth, Proto::Ipv6, Proto::Udp, Proto::Dns]);
    // "hello v6" is no DNS message: only the DNS dissector complains
    t.check("v6 layers clean", decode::layers(&f).iter().filter(|l| l.proto != Proto::Dns).all(|l| l.problems.is_empty()));
    t.check("v6 payload is no dns", decode::layers(&f).iter().any(|l| l.proto == Proto::Dns && !l.problems.is_empty()));
    t.eq("v6 info", decode::info(&f).1, String::from("1234 \u{2192} 53 Len=8"));
}

fn pcap(t: &mut T) {
    let mut w = pcapng::Writer::new("JFNetworker test");
    let a = w.add_interface("R1 Gi0/0", pcapng::LINKTYPE_ETHERNET, 0);
    let b = w.add_interface("SW1 Gi0/1", pcapng::LINKTYPE_ETHERNET, 0);
    let f1 = ping_frame(128);
    w.packet(a, 1_500_000_123, &f1);
    w.packet_with_comment(b, 1_500_300_000, &f1[..61], "cut");
    let bytes = w.finish();
    t.eq("pcapng magic", &bytes[..4], &[0x0A, 0x0D, 0x0D, 0x0A][..]);
    t.check("pcapng aligned", bytes.len() % 4 == 0);
    match pcapng::read(&bytes) {
        Ok(c) => {
            t.eq("pcapng app", c.app.as_str(), "JFNetworker test");
            t.eq("pcapng ifaces", c.interfaces.iter().map(|i| (i.name.as_str(), i.link, i.units)).collect::<Vec<_>>(), vec![("R1 Gi0/0", 1, 1_000_000_000), ("SW1 Gi0/1", 1, 1_000_000_000)]);
            t.eq("pcapng packets", c.packets.len(), 2);
            t.check("pcapng packet 1", c.packets.first().is_some_and(|p| p.iface == 0 && p.t_ns == 1_500_000_123 && p.data == f1));
            t.check("pcapng packet 2", c.packets.get(1).is_some_and(|p| p.iface == 1 && p.data.len() == 61 && p.comment == "cut"));
        }
        Err(e) => t.check(&format!("pcapng read {:?}", e), false),
    }
    t.check("pcapng rejects garbage", pcapng::read(b"not a capture file at all......").is_err());
}

fn pkt_growth(t: &mut T) {
    let mut p = Pkt::new(2, b"xy");
    p.push(10).copy_from_slice(b"0123456789");
    t.eq("pkt grows", p.bytes(), &b"0123456789xy"[..]);
    t.check("pkt pull", p.pull(10) && p.bytes() == b"xy" && !p.pull(3));
    p.extend(b"z");
    p.truncate(2);
    t.eq("pkt tail", p.into_vec(), b"xy".to_vec());
}

// netpkt::nat: a translated packet equals the one made fresh with the new addresses and ports,
// checksums included (RFC 1624 against a full computation); ICMP errors with their quote
// (RFC 5508); fragments; checksum::update
fn nat_rewrite(t: &mut T) {
    use netpkt::nat::{self, Flow, Side};
    let g = Ipv4([203, 0, 113, 1]);
    let l = Ipv4([192, 168, 1, 20]);
    let udp = |src: Ipv4, sp: u16, dst: Ipv4, dp: u16, zero: bool| {
        let mut p = Pkt::new(netpkt::pkt::HEADROOM, b"translate me, please");
        UdpHdr::new(sp, dp).push_v4(&mut p, src, dst);
        if zero {
            p.bytes_mut()[6] = 0;
            p.bytes_mut()[7] = 0;
        }
        let mut h = Ipv4Hdr::new(src, dst, ipv4::PROTO_UDP, 63);
        h.id = 0x1234;
        h.push_onto(&mut p);
        p.into_vec()
    };
    // UDP: source address and port
    let mut a = udp(IP_A, 40000, IP_B, 9, false);
    t.check("nat udp", nat::rewrite(&mut a, Side::Src, g, Some(5002)));
    t.eq("nat udp = fresh", a, udp(g, 5002, IP_B, 9, false));
    // a UDP checksum of 0 means none and stays so
    let mut z = udp(IP_A, 40000, IP_B, 9, true);
    nat::rewrite(&mut z, Side::Src, g, Some(5002));
    t.eq("nat udp without checksum", z, udp(g, 5002, IP_B, 9, true));
    // TCP: destination address and port (a SYN with options)
    let tcp = |dst: Ipv4, dp: u16| {
        let opts = TcpOpts { mss: Some(1460), ..TcpOpts::default() };
        let h = TcpHdr { src_port: 51000, dst_port: dp, seq: 77, ack: 0, flags: tcp::SYN, window: 64240, urgent: 0, opts };
        let mut p = Pkt::new(netpkt::pkt::HEADROOM, b"");
        h.push_v4(&mut p, IP_A, dst);
        Ipv4Hdr::new(IP_A, dst, ipv4::PROTO_TCP, 64).push_onto(&mut p);
        p.into_vec()
    };
    let mut s = tcp(g, 80);
    nat::rewrite(&mut s, Side::Dst, l, Some(8080));
    t.eq("nat tcp = fresh", s, tcp(l, 8080));
    // an ICMP echo: the identifier is its port, the ICMP checksum follows
    let echo = |src: Ipv4, id: u16| {
        let mut p = Pkt::new(netpkt::pkt::HEADROOM, PING_DATA);
        Icmp::EchoRequest { id, seq: 3 }.push_onto(&mut p);
        Ipv4Hdr::new(src, IP_B, ipv4::PROTO_ICMP, 64).push_onto(&mut p);
        p.into_vec()
    };
    let mut e = echo(IP_A, 7);
    nat::rewrite(&mut e, Side::Src, g, Some(9));
    t.eq("nat echo = fresh", e.clone(), echo(g, 9));
    let f = Flow::of(&e);
    t.eq("flow of an echo", f.map(|f| (f.proto, f.src, f.sport, f.dport, f.icmp, f.ports)), Some((ipv4::PROTO_ICMP, g, 9, 9, Some((8, 0)), true)));
    // an ICMP error quoting a whole UDP datagram (as Linux does): the quote's source goes back
    // to the inside host, the error to it
    let err = |quoted: &[u8], dst: Ipv4| {
        let mut p = Pkt::new(netpkt::pkt::HEADROOM, quoted);
        Icmp::Unreachable { code: icmp::UNREACH_PORT, mtu: 0 }.push_onto(&mut p);
        Ipv4Hdr::new(IP_B, dst, ipv4::PROTO_ICMP, 64).push_onto(&mut p);
        p.into_vec()
    };
    let out = udp(g, 5002, IP_B, 9, false);
    let mut long = err(&out, g);
    t.eq("quoted flow", Flow::quoted(&long).map(|q| (q.src, q.sport, q.dst, q.dport)), Some((g, 5002, IP_B, 9)));
    t.check("nat quote", nat::rewrite_quoted(&mut long, Side::Src, l, Some(40000)) && nat::rewrite(&mut long, Side::Dst, l, None));
    t.eq("nat long quote = fresh", long, err(&udp(l, 40000, IP_B, 9, false), l));
    // the IOS quote (header and 8 bytes) of a TCP segment: no TCP checksum in it
    let seg = tcp(g, 80);
    let mut short = err(icmp::error_quote(&seg), IP_A);
    nat::rewrite_quoted(&mut short, Side::Dst, l, Some(8080));
    t.eq("nat short quote = fresh", short, err(icmp::error_quote(&tcp(l, 8080)), IP_A));
    t.check("nat quote refused without one", !nat::rewrite_quoted(&mut udp(IP_A, 1, IP_B, 2, false), Side::Src, g, None));
    // fragments: the first carries the port, the others only the address; the datagram adds up
    let big: Vec<u8> = {
        let data: Vec<u8> = (0..3000u32).map(|i| (i * 13 + 5) as u8).collect();
        let mut p = Pkt::new(netpkt::pkt::HEADROOM, &data);
        UdpHdr::new(40000, 9).push_v4(&mut p, IP_A, IP_B);
        let mut h = Ipv4Hdr::new(IP_A, IP_B, ipv4::PROTO_UDP, 64);
        h.id = 0x55;
        h.push_onto(&mut p);
        p.into_vec()
    };
    let parts = ipv4::fragment(&big, 1500).unwrap_or_default();
    let mut r = Reassembly::new();
    let mut whole = None;
    for (i, mut part) in parts.into_iter().enumerate() {
        let fl = Flow::of(&part);
        t.check("fragment flow", fl.is_some_and(|f| f.later == (i > 0) && f.ports == (i == 0)));
        nat::rewrite(&mut part, Side::Src, g, Some(5002));
        t.check("fragment header", Ipv4View::parse(&part).is_some_and(|v| v.checksum_ok() && v.src() == g));
        whole = Ipv4View::parse(&part).and_then(|v| r.add(&v)).or(whole);
    }
    let ok = whole.as_deref().and_then(Ipv4View::parse).is_some_and(|v| UdpView::parse(v.payload()).is_some_and(|u| u.src_port() == 5002 && u.checksum_ok_v4(g, IP_B)));
    t.check("fragments translated add up", ok);
    // checksum::update over a run of bytes equals a new computation
    let mut blk: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(37)).collect();
    let ck = checksum::checksum(&blk);
    let old = blk[10..19].to_vec();
    for b in blk[10..19].iter_mut() {
        *b ^= 0xA5;
    }
    t.eq("checksum update", checksum::update(ck, &old, &blk[10..19]), checksum::checksum(&blk));
}
