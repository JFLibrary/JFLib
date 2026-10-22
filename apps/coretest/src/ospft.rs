// ospft -- checks of netpkt::ospf and its dissector: the five packet types byte for byte with
// their checksum (RFC 2328 A.3), a simple password, LSAs of the types 1 to 5 with their Fletcher
// checksum (12.1.7), round trips view -> record -> bytes, which of two instances is newer (13.1),
// a whole frame from Ethernet down, and the problems a receiver refuses.  The expected bytes were
// computed independently (Python, struct; the Fletcher check bytes found by trying every pair
// until C0 and C1 are zero, RFC 905 B.2).

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_OSPF};
use netpkt::ospf::{self, Body, Dd, ExternalLsa, Header, Hello, Lsa, LsaBody, LsaHeader, LsaKey, NetworkLsa, OspfView, Packet, RouterLink, RouterLsa, SummaryLsa};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::{Ipv4, Mac};

use crate::T;

/// bytes from hex digits separated by spaces
fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace().filter_map(|x| u8::from_str_radix(x, 16).ok()).collect()
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

const HELLO: &str = "02 01 00 30 01 01 01 01 00 00 00 00 ca 91 00 00 00 00 00 00 00 00 00 00 ff ff ff 00 00 0a 02 01 00 00 00 28 0a 00 0c 02 0a 00 0c 01 02 02 02 02";
const HELLO_PW: &str = "02 01 00 30 01 01 01 01 00 00 00 00 ca 90 00 01 73 65 63 72 65 74 00 00 ff ff ff 00 00 0a 02 01 00 00 00 28 0a 00 0c 02 0a 00 0c 01 02 02 02 02";
const ROUTER: &str = "00 01 02 01 01 01 01 01 01 01 01 01 80 00 00 03 f3 9b 00 3c 03 00 00 03 0a 00 0c 02 0a 00 0c 01 02 00 00 01 0a 00 01 00 ff ff ff 00 03 00 00 0a 03 03 03 03 0a 00 0d 01 01 00 00 30";
const NETWORK: &str = "00 05 02 02 0a 00 0c 02 02 02 02 02 80 00 00 01 a0 8d 00 20 ff ff ff 00 02 02 02 02 01 01 01 01";
const SUMMARY: &str = "00 0a 02 03 0a 00 03 00 02 02 02 02 80 00 00 02 52 e0 00 1c ff ff ff 00 00 00 00 14";
const ASBR: &str = "00 0a 02 04 04 04 04 04 02 02 02 02 80 00 00 01 ae 8e 00 1c 00 00 00 00 00 00 00 07";
const EXTERNAL: &str = "00 00 02 05 00 00 00 00 04 04 04 04 80 00 00 01 74 4d 00 24 00 00 00 00 80 00 00 01 00 00 00 00 00 00 00 00";
const EXTERNAL1: &str = "00 03 02 05 c0 a8 05 00 04 04 04 04 80 00 00 04 30 04 00 24 ff ff ff 00 00 00 00 14 0a 00 2d 05 00 00 00 4d";
const DD0: &str = "02 02 00 20 02 02 02 02 00 00 00 00 50 44 00 00 00 00 00 00 00 00 00 00 05 dc 02 07 00 00 a1 b2";
const DD1: &str = "02 02 00 48 01 01 01 01 00 00 00 00 9b 87 00 00 00 00 00 00 00 00 00 00 05 dc 02 00 00 00 a1 b2 00 01 02 01 01 01 01 01 01 01 01 01 80 00 00 03 f3 9b 00 3c 00 05 02 02 0a 00 0c 02 02 02 02 02 80 00 00 01 a0 8d 00 20";
const LSR: &str = "02 03 00 30 01 01 01 01 00 00 00 00 dd bd 00 00 00 00 00 00 00 00 00 00 00 00 00 01 01 01 01 01 01 01 01 01 00 00 00 02 0a 00 0c 02 02 02 02 02";
const LSU: &str = "02 04 00 78 02 02 02 02 00 00 00 00 e1 8d 00 00 00 00 00 00 00 00 00 00 00 00 00 02 00 02 02 01 01 01 01 01 01 01 01 01 80 00 00 03 f3 9b 00 3c 03 00 00 03 0a 00 0c 02 0a 00 0c 01 02 00 00 01 0a 00 01 00 ff ff ff 00 03 00 00 0a 03 03 03 03 0a 00 0d 01 01 00 00 30 00 06 02 02 0a 00 0c 02 02 02 02 02 80 00 00 01 a0 8d 00 20 ff ff ff 00 02 02 02 02 01 01 01 01";
const ACK: &str = "02 05 00 40 01 01 01 01 00 00 00 01 45 1a 00 00 00 00 00 00 00 00 00 00 00 01 02 01 01 01 01 01 01 01 01 01 80 00 00 03 f3 9b 00 3c 00 05 02 02 0a 00 0c 02 02 02 02 02 80 00 00 01 a0 8d 00 20";
const FRAME: &str = "01 00 5e 00 00 05 02 4a 00 00 01 01 08 00 45 c0 00 44 00 07 00 00 01 59 c2 94 0a 00 0c 01 e0 00 00 05 02 01 00 30 01 01 01 01 00 00 00 00 ca 91 00 00 00 00 00 00 00 00 00 00 ff ff ff 00 00 0a 02 01 00 00 00 28 0a 00 0c 02 0a 00 0c 01 02 02 02 02";

fn hdr(kind: u8, router: &str, area: u32) -> Header {
    Header { kind, router: ip(router), area, autype: ospf::AU_NULL, auth: [0; 8] }
}

fn hello() -> Hello {
    Hello { mask: ip("255.255.255.0"), interval: 10, options: ospf::OPT_E, priority: 1, dead: 40, dr: ip("10.0.12.2"), bdr: ip("10.0.12.1"), neighbors: alloc::vec![ip("2.2.2.2")] }
}

fn router_lsa() -> Lsa {
    let links = alloc::vec![
        RouterLink { id: ip("10.0.12.2"), data: ip("10.0.12.1"), kind: ospf::LINK_TRANSIT, metric: 1, tos: Vec::new() },
        RouterLink { id: ip("10.0.1.0"), data: ip("255.255.255.0"), kind: ospf::LINK_STUB, metric: 10, tos: Vec::new() },
        RouterLink { id: ip("3.3.3.3"), data: ip("10.0.13.1"), kind: ospf::LINK_P2P, metric: 48, tos: Vec::new() },
    ];
    let mut l = Lsa::new(ospf::LSA_ROUTER, ip("1.1.1.1"), ip("1.1.1.1"), 0x8000_0003, ospf::OPT_E, LsaBody::Router(RouterLsa { flags: ospf::RTR_B | ospf::RTR_E, links }));
    l.hdr.age = 1;
    l
}

fn network_lsa() -> Lsa {
    let mut l = Lsa::new(ospf::LSA_NETWORK, ip("10.0.12.2"), ip("2.2.2.2"), ospf::INITIAL_SEQ, ospf::OPT_E, LsaBody::Network(NetworkLsa { mask: ip("255.255.255.0"), routers: alloc::vec![ip("2.2.2.2"), ip("1.1.1.1")] }));
    l.hdr.age = 5;
    l
}

pub fn run(t: &mut T) {
    packets(t);
    lsas(t);
    exchange(t);
    newer(t);
    frame(t);
    problems(t);
}

fn packets(t: &mut T) {
    let p = Packet { hdr: hdr(ospf::T_HELLO, "1.1.1.1", 0), body: Body::Hello(hello()) };
    let b = p.to_bytes();
    t.eq("hello bytes", &b, &unhex(HELLO));
    match OspfView::parse(&b) {
        Some(v) => {
            t.eq("hello header", (v.version(), v.kind(), v.len(), v.router(), v.area(), v.autype()), (2, ospf::T_HELLO, 48, ip("1.1.1.1"), 0, ospf::AU_NULL));
            t.eq("hello checksum", (v.checksum(), v.computed_checksum(), v.checksum_ok()), (0xca91, 0xca91, true));
            t.eq("hello round trip", Packet::read(&v), Ok(p.clone()));
        }
        None => t.check("hello parses", false),
    }
    // a simple password: the checksum leaves the authentication field out (A.3.1, D.3)
    let pw = Packet { hdr: Header { autype: ospf::AU_SIMPLE, auth: *b"secret\0\0", ..p.hdr }, body: p.body.clone() };
    let b = pw.to_bytes();
    t.eq("password bytes", &b, &unhex(HELLO_PW));
    t.check("password checksum", OspfView::parse(&b).is_some_and(|v| v.checksum_ok() && v.auth() == *b"secret\0\0"));
    // trailing bytes (Ethernet padding) are no part of the packet
    let mut padded = unhex(HELLO);
    padded.extend_from_slice(&[0, 0, 0, 0]);
    t.check("padding ignored", OspfView::parse(&padded).is_some_and(|v| v.len() == 48 && v.checksum_ok()));
    t.eq("names", (ospf::type_name(ospf::T_HELLO), ospf::type_name(ospf::T_DD), ospf::type_name(ospf::T_LSR), ospf::type_name(ospf::T_LSU), ospf::type_name(ospf::T_ACK)), ("Hello Packet", "DB Description", "LS Request", "LS Update", "LS Acknowledge"));
    t.eq("groups", (ospf::ALL_SPF_ROUTERS, ospf::ALL_D_ROUTERS), (ip("224.0.0.5"), ip("224.0.0.6")));
    t.eq("options text", (ospf::options_text(0x02), ospf::options_text(0x42), ospf::options_text(0)), (String::from("E"), String::from("O, E"), String::from("none")));
}

fn lsas(t: &mut T) {
    let cases: [(&str, Lsa, &str); 2] = [("router", router_lsa(), ROUTER), ("network", network_lsa(), NETWORK)];
    for (name, l, want) in cases {
        let b = l.to_bytes();
        t.eq(&alloc::format!("{} bytes", name), &b, &unhex(want));
        t.check(&alloc::format!("{} checksum correct", name), ospf::lsa_checksum_ok(&b));
        t.eq(&alloc::format!("{} checksum computed", name), ospf::lsa_checksum(&b), u16::from_be_bytes([b[16], b[17]]));
        let back = Lsa::parse(&b).map(|x| Lsa { hdr: LsaHeader { checksum: 0, len: 0, ..x.hdr }, ..x });
        t.eq(&alloc::format!("{} round trip", name), back, Ok(l.clone()));
    }
    // summary-LSAs of type 3 and 4, AS-external-LSAs of both metric types
    let mut s3 = Lsa::new(ospf::LSA_SUMMARY, ip("10.0.3.0"), ip("2.2.2.2"), 0x8000_0002, ospf::OPT_E, LsaBody::Summary(SummaryLsa { mask: ip("255.255.255.0"), metric: 20, tos: Vec::new() }));
    s3.hdr.age = 10;
    t.eq("summary bytes", s3.to_bytes(), unhex(SUMMARY));
    let mut s4 = Lsa::new(ospf::LSA_ASBR, ip("4.4.4.4"), ip("2.2.2.2"), ospf::INITIAL_SEQ, ospf::OPT_E, LsaBody::Summary(SummaryLsa { mask: Ipv4::UNSPECIFIED, metric: 7, tos: Vec::new() }));
    s4.hdr.age = 10;
    t.eq("asbr summary bytes", s4.to_bytes(), unhex(ASBR));
    let e2 = Lsa::new(ospf::LSA_EXTERNAL, Ipv4::UNSPECIFIED, ip("4.4.4.4"), ospf::INITIAL_SEQ, ospf::OPT_E, LsaBody::External(ExternalLsa { mask: Ipv4::UNSPECIFIED, e2: true, metric: 1, fwd: Ipv4::UNSPECIFIED, tag: 0, tos: Vec::new() }));
    t.eq("external type 2 bytes", e2.to_bytes(), unhex(EXTERNAL));
    let mut e1 = Lsa::new(ospf::LSA_EXTERNAL, ip("192.168.5.0"), ip("4.4.4.4"), 0x8000_0004, ospf::OPT_E, LsaBody::External(ExternalLsa { mask: ip("255.255.255.0"), e2: false, metric: 20, fwd: ip("10.0.45.5"), tag: 77, tos: Vec::new() }));
    e1.hdr.age = 3;
    t.eq("external type 1 bytes", e1.to_bytes(), unhex(EXTERNAL1));
    for (name, v) in [("summary", SUMMARY), ("asbr", ASBR), ("external", EXTERNAL), ("external1", EXTERNAL1)] {
        let b = unhex(v);
        t.check(&alloc::format!("{} checksum correct", name), ospf::lsa_checksum_ok(&b));
    }
    match Lsa::parse(&unhex(EXTERNAL1)).map(|x| x.body) {
        Ok(LsaBody::External(e)) => t.eq("external read", (e.mask, e.e2, e.metric, e.fwd, e.tag), (ip("255.255.255.0"), false, 20, ip("10.0.45.5"), 77)),
        _ => t.check("external parses", false),
    }
    // the age is outside the checksum; any other byte changes it
    let mut b = unhex(ROUTER);
    ospf::set_age(&mut b, 1800);
    t.check("new age, same checksum", ospf::lsa_checksum_ok(&b) && LsaHeader::of(&b).is_some_and(|h| h.age == 1800));
    b[30] ^= 1;
    t.check("changed body, bad checksum", !ospf::lsa_checksum_ok(&b));
    let mut z = unhex(ROUTER);
    z[16] = 0;
    z[17] = 0;
    t.check("checksum 0 is never right", !ospf::lsa_checksum_ok(&z));
    // the check bytes are 1 to 255 (a zero is written as 255)
    let ck = ospf::lsa_checksum(&unhex(NETWORK));
    t.check("check bytes nonzero", ck >> 8 != 0 && ck & 0xFF != 0);
    t.eq("lsa names", (ospf::lsa_name(1), ospf::lsa_name(2), ospf::lsa_name(3), ospf::lsa_name(4), ospf::lsa_name(5)), ("Router-LSA", "Network-LSA", "Summary-LSA (IP network)", "Summary-LSA (ASBR)", "AS-External-LSA"));
    // an unknown type keeps its bytes
    let other = Lsa::new(9, ip("1.2.3.4"), ip("1.1.1.1"), ospf::INITIAL_SEQ, 0, LsaBody::Other(alloc::vec![1, 2, 3, 4]));
    let ob = other.to_bytes();
    t.check("opaque bytes kept", ospf::lsa_checksum_ok(&ob) && Lsa::parse(&ob).is_ok_and(|x| x.body == LsaBody::Other(alloc::vec![1, 2, 3, 4])));
}

fn exchange(t: &mut T) {
    let rh = LsaHeader::of(&unhex(ROUTER)).unwrap_or_default();
    let nh = LsaHeader::of(&unhex(NETWORK)).unwrap_or_default();
    t.eq("lsa header read", (rh.age, rh.options, rh.kind, rh.id, rh.adv, rh.seq, rh.checksum, rh.len), (1, 2, 1, ip("1.1.1.1"), ip("1.1.1.1"), 0x8000_0003, 0xf39b, 60));
    t.eq("key", nh.key(), LsaKey { kind: 2, id: ip("10.0.12.2"), adv: ip("2.2.2.2") });
    // the master's first DD and the slave's answer
    let dd0 = Packet { hdr: hdr(ospf::T_DD, "2.2.2.2", 0), body: Body::Dd(Dd { mtu: 1500, options: ospf::OPT_E, flags: ospf::DD_I | ospf::DD_M | ospf::DD_MS, seq: 0xa1b2, lsas: Vec::new() }) };
    t.eq("dd init bytes", dd0.to_bytes(), unhex(DD0));
    let dd1 = Packet { hdr: hdr(ospf::T_DD, "1.1.1.1", 0), body: Body::Dd(Dd { mtu: 1500, options: ospf::OPT_E, flags: 0, seq: 0xa1b2, lsas: alloc::vec![rh, nh] }) };
    let b = dd1.to_bytes();
    t.eq("dd answer bytes", &b, &unhex(DD1));
    t.check("dd round trip", OspfView::parse(&b).is_some_and(|v| v.checksum_ok() && Packet::read(&v) == Ok(dd1.clone())));
    t.eq("dd flags text", (ospf::dd_flags_text(7), ospf::dd_flags_text(2), ospf::dd_flags_text(0)), (String::from("I, M, MS"), String::from("M"), String::from("none")));
    let lsr = Packet { hdr: hdr(ospf::T_LSR, "1.1.1.1", 0), body: Body::Lsr(alloc::vec![rh.key(), nh.key()]) };
    let b = lsr.to_bytes();
    t.eq("lsr bytes", &b, &unhex(LSR));
    t.check("lsr round trip", OspfView::parse(&b).is_some_and(|v| Packet::read(&v) == Ok(lsr.clone())));
    // an update carries the LSAs byte for byte, with the ages they travel with
    let mut r = unhex(ROUTER);
    ospf::set_age(&mut r, 2);
    let mut n = unhex(NETWORK);
    ospf::set_age(&mut n, 6);
    let lsu = Packet { hdr: hdr(ospf::T_LSU, "2.2.2.2", 0), body: Body::Lsu(alloc::vec![r.clone(), n.clone()]) };
    let b = lsu.to_bytes();
    t.eq("lsu bytes", &b, &unhex(LSU));
    match OspfView::parse(&b) {
        Some(v) => {
            t.eq("lsu spans", v.lsu_spans(), Some(alloc::vec![(28, 60), (88, 32)]));
            t.eq("lsu round trip", Packet::read(&v), Ok(lsu.clone()));
        }
        None => t.check("lsu parses", false),
    }
    let ack = Packet { hdr: hdr(ospf::T_ACK, "1.1.1.1", 1), body: Body::Ack(alloc::vec![rh, nh]) };
    let b = ack.to_bytes();
    t.eq("ack bytes", &b, &unhex(ACK));
    t.check("ack round trip", OspfView::parse(&b).is_some_and(|v| v.area() == 1 && Packet::read(&v) == Ok(ack.clone())));
    t.eq("area text", (ospf::area_text(0), ospf::area_text(1)), (String::from("0.0.0.0 (Backbone)"), String::from("0.0.0.1")));
}

fn newer(t: &mut T) {
    let base = LsaHeader { age: 100, options: 2, kind: 1, id: ip("1.1.1.1"), adv: ip("1.1.1.1"), seq: 0x8000_0005, checksum: 0x1234, len: 36 };
    let with = |f: &dyn Fn(&mut LsaHeader)| {
        let mut h = base;
        f(&mut h);
        h
    };
    t.eq("same instance", ospf::compare(&base, &base), Ordering::Equal);
    t.eq("higher sequence", ospf::compare(&with(&|h| h.seq = 0x8000_0006), &base), Ordering::Greater);
    // signed: 0x80000001 is the smallest, 0x7fffffff the largest
    t.eq("signed sequence", ospf::compare(&with(&|h| h.seq = 1), &with(&|h| h.seq = ospf::INITIAL_SEQ)), Ordering::Greater);
    t.eq("checksum decides", ospf::compare(&with(&|h| h.checksum = 0x1235), &base), Ordering::Greater);
    t.eq("maxage wins", ospf::compare(&with(&|h| h.age = ospf::MAX_AGE), &base), Ordering::Greater);
    t.eq("young beats old", ospf::compare(&with(&|h| h.age = 10), &with(&|h| h.age = 1000)), Ordering::Greater);
    t.eq("within maxagediff", ospf::compare(&with(&|h| h.age = 10), &with(&|h| h.age = 900)), Ordering::Equal);
}

fn frame(t: &mut T) {
    let p = Packet { hdr: hdr(ospf::T_HELLO, "1.1.1.1", 0), body: Body::Hello(hello()) };
    let mut k = Pkt::new(HEADROOM, &p.to_bytes());
    let mut h = Ipv4Hdr::new(ip("10.0.12.1"), ospf::ALL_SPF_ROUTERS, PROTO_OSPF, 1);
    h.id = 7;
    h.dscp = 48;
    h.push_onto(&mut k);
    EthHdr { dst: Mac::of_ipv4_multicast(ospf::ALL_SPF_ROUTERS), src: Mac([0x02, 0x4a, 0, 0, 1, 1]), ethertype: ET_IPV4 }.push_onto(&mut k);
    let f = k.into_vec();
    t.eq("frame bytes", &f, &unhex(FRAME));
    t.check("mask", decode::proto_mask(&f) & decode::P_OSPF != 0);
    let (top, info) = decode::info(&f);
    t.eq("summary", (top, info), (Proto::Ospf, String::from("Hello Packet, DR 10.0.12.2, BDR 10.0.12.1, 1 neighbor")));
    let ls = decode::layers(&f);
    match ls.iter().find(|l| l.proto == Proto::Ospf) {
        Some(l) => {
            t.eq("title", l.title.as_str(), "Open Shortest Path First (Hello Packet)");
            let field = |n: &str| l.fields.iter().find(|x| x.name == n).map(|x| (x.value.clone(), x.off, x.len));
            t.eq("router field", field("Source OSPF Router"), Some((String::from("1.1.1.1"), 38, 4)));
            t.eq("checksum field", field("Checksum"), Some((String::from("0xca91 [correct]"), 46, 2)));
            t.eq("dead field", field("Router Dead Interval [sec]"), Some((String::from("40"), 66, 4)));
            t.eq("neighbor field", field("Active Neighbor"), Some((String::from("2.2.2.2"), 78, 4)));
            t.check("no problems", l.problems.is_empty());
        }
        None => t.check("ospf layer", false),
    }
    // an update in a frame: every LSA with its header and links
    let lsu = unhex(LSU);
    let mut k = Pkt::new(HEADROOM, &lsu);
    Ipv4Hdr::new(ip("10.0.12.2"), ospf::ALL_SPF_ROUTERS, PROTO_OSPF, 1).push_onto(&mut k);
    EthHdr { dst: Mac::of_ipv4_multicast(ospf::ALL_SPF_ROUTERS), src: Mac([0x02, 0x4a, 0, 0, 2, 1]), ethertype: ET_IPV4 }.push_onto(&mut k);
    let f = k.into_vec();
    t.eq("update summary", decode::info(&f).1, String::from("LS Update, 2 LSAs"));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == Proto::Ospf);
    t.check("update fields", l.is_some_and(|l| {
        let lsas: Vec<&str> = l.fields.iter().filter(|x| x.name == "LSA").map(|x| x.value.as_str()).collect();
        let links = l.fields.iter().filter(|x| x.name == "Link").count();
        lsas.len() == 2 && lsas[0].starts_with("Router-LSA 1.1.1.1 from 1.1.1.1, seq 0x80000003, age 2, options E, checksum 0xf39b [correct]") && links == 3 && l.fields.iter().any(|x| x.name == "Attached Router" && x.value == "1.1.1.1") && l.problems.is_empty()
    }));
}

fn problems(t: &mut T) {
    let layer = |ospf_bytes: &[u8]| -> Option<decode::Layer> {
        let mut k = Pkt::new(HEADROOM, ospf_bytes);
        Ipv4Hdr::new(ip("10.0.12.1"), ospf::ALL_SPF_ROUTERS, PROTO_OSPF, 1).push_onto(&mut k);
        EthHdr { dst: Mac::of_ipv4_multicast(ospf::ALL_SPF_ROUTERS), src: Mac([0x02, 0x4a, 0, 0, 1, 1]), ethertype: ET_IPV4 }.push_onto(&mut k);
        decode::layers(&k.into_vec()).into_iter().find(|l| l.proto == Proto::Ospf)
    };
    // a wrong packet checksum
    let mut b = unhex(HELLO);
    b[13] ^= 0xFF;
    t.check("bad checksum seen", OspfView::parse(&b).is_some_and(|v| !v.checksum_ok()));
    t.check("bad checksum shown", layer(&b).is_some_and(|l| l.problems.iter().any(|p| p == "bad OSPF checksum (should be 0xca91)")));
    // an LSA with a wrong checksum inside an update
    let mut b = unhex(LSU);
    b[30 + 28] ^= 1;
    let v = Packet { hdr: hdr(ospf::T_LSU, "2.2.2.2", 0), body: Body::Lsu(alloc::vec![b[28..88].to_vec()]) }.to_bytes();
    t.check("bad lsa checksum shown", layer(&v).is_some_and(|l| l.problems.iter().any(|p| p.starts_with("bad LSA checksum of Router-LSA 1.1.1.1"))));
    // version 3, an unknown type, a short body
    let mut b = unhex(HELLO);
    b[0] = 3;
    t.check("version 3", layer(&b).is_some_and(|l| l.problems.iter().any(|p| p.starts_with("version 3"))));
    t.eq("version error", OspfView::parse(&b).map(|v| Packet::read(&v)), Some(Err(ospf::OspfErr::Version(3))));
    let mut b = unhex(HELLO);
    b[1] = 9;
    t.eq("kind error", OspfView::parse(&b).map(|v| Packet::read(&v)), Some(Err(ospf::OspfErr::Kind(9))));
    let mut b = unhex(HELLO);
    b.truncate(40);
    b[3] = 40;
    t.eq("short hello", OspfView::parse(&b).map(|v| Packet::read(&v)), Some(Err(ospf::OspfErr::Body)));
    // a length field beyond the bytes, an LSA longer than the update
    let mut b = unhex(HELLO);
    b[3] = 60;
    t.check("length beyond the bytes", OspfView::parse(&b).is_none());
    let mut b = unhex(LSU);
    b[28 + 19] = 0xF0;
    t.eq("lsa too long", OspfView::parse(&b).map(|v| Packet::read(&v)), Some(Err(ospf::OspfErr::Lsa)));
    t.eq("lsa cut", Lsa::parse(&unhex(ROUTER)[..40]), Err(ospf::LsaErr::Short));
    let mut short = unhex(ROUTER);
    short[23] = 4;
    t.eq("lsa links do not fit", Lsa::parse(&short), Err(ospf::LsaErr::Length));
}
