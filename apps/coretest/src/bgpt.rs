// bgpt -- checks of netpkt::bgp and its dissector: OPEN with its capabilities (2- and 4-octet AS),
// UPDATE with withdrawn routes, every path attribute (AS_SEQUENCE and AS_SET in 2- and 4-octet
// form, the extended length bit, an unknown optional transitive attribute), NOTIFICATION,
// KEEPALIVE and ROUTE-REFRESH byte for byte, round trips bytes -> record -> bytes, the stream
// framing, malformed messages and the NOTIFICATION code and subcode a receiver answers them
// with (RFC 4271 6.1 to 6.3), whole frames from Ethernet down.  The expected bytes were built
// independently (Python, struct).

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::bgp::{self, Attr, AttrVal, Cap, Message, Notification, Open, Segment, Update};
use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::T;

fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace().filter_map(|x| u8::from_str_radix(x, 16).ok()).collect()
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or_default()
}

const OPEN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2d 01 04 fd e9 00 b4 01 01 01 01 10 02 0e 01 04 00 01 00 01 02 00 41 04 00 00 fd e9";
const OPEN4: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2d 01 04 5b a0 00 5a 09 09 09 09 10 02 0e 01 04 00 01 00 01 02 00 41 04 fa 56 ea 01";
const KEEPALIVE: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 13 04";
const UPD: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 40 02 00 03 10 0a 09 00 1f 40 01 01 00 40 02 0a 02 02 00 00 fd ea 00 00 fd eb 40 03 04 0a 00 0c 02 80 04 04 00 00 00 00 18 0a 02 00 10 c0 a8";
const UPD2: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 57 02 00 00 00 3d 40 01 01 02 50 02 00 0a 02 01 fd ea 01 02 fd f2 fd f3 40 03 04 0a 00 00 01 40 05 04 00 00 00 c8 40 06 00 c0 07 06 fd ea 02 02 02 02 c0 08 08 fd e9 00 64 ff ff ff 01 e0 63 03 01 02 03 10 0a 0a";
const NOTIF: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 17 03 02 02 fd ea";
const RR: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 17 05 00 01 00 01";
const WD: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 1b 02 00 04 18 0a 02 00 00 00";

const B_MARKER: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff fe 00 13 04";
const B_SHORT: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 12 04";
const B_LONG: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 10 01 02";
const B_TYPE: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 13 09";
const B_KALEN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 14 04 00";
const B_VERSION: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 1d 01 03 fd e9 00 b4 01 01 01 01 00";
const B_HOLD: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 1d 01 04 fd e9 00 02 01 01 01 01 00";
const B_ID: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 1d 01 04 fd e9 00 b4 00 00 00 00 00";
const B_PARAM: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 21 01 04 fd e9 00 b4 01 01 01 01 04 01 02 00 00";
const B_NOHOP: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 22 02 00 00 00 07 40 01 01 00 40 02 00 18 0a 02 00";
const B_ORIGIN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 29 02 00 00 00 0e 40 01 01 03 40 02 00 40 03 04 0a 00 00 01 18 0a 02 00";
const B_FLAGS: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 29 02 00 00 00 0e c0 01 01 00 40 02 00 40 03 04 0a 00 00 01 18 0a 02 00";
const B_HOPLEN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2a 02 00 00 00 0f 40 01 01 00 40 02 00 40 03 05 0a 00 00 01 00 18 0a 02 00";
const B_TWICE: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2d 02 00 00 00 12 40 01 01 00 40 01 01 00 40 02 00 40 03 04 0a 00 00 01 18 0a 02 00";
const B_WDLEN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 17 02 00 28 00 00";
const B_NLRI: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2b 02 00 00 00 0e 40 01 01 00 40 02 00 40 03 04 0a 00 00 01 21 0a 00 00 00 00";
const B_PATH: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2f 02 00 00 00 14 40 01 01 00 40 02 06 03 01 00 00 fd e9 40 03 04 0a 00 00 01 18 0a 02 00";
const B_WELLKNOWN: &str = "ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff 00 2d 02 00 00 00 12 40 01 01 00 40 02 00 40 03 04 0a 00 00 01 40 63 01 01 18 0a 02 00";

pub fn run(t: &mut T) {
    messages(t);
    attributes(t);
    stream(t);
    malformed(t);
    frames(t);
}

/// records written byte for byte, read back the same
fn messages(t: &mut T) {
    let caps = alloc::vec![Cap::Mp { afi: bgp::AFI_IPV4, safi: bgp::SAFI_UNICAST }, Cap::RouteRefresh, Cap::As4(65001)];
    let open = Message::Open(Open::new(65001, 180, ip("1.1.1.1"), caps));
    t.eq("open bytes", open.to_bytes(true), unhex(OPEN));
    t.eq("open back", Message::read(&unhex(OPEN), true), Ok(open.clone()));
    // a 4-octet AS: AS_TRANS in the header, the AS in the capability
    let caps = alloc::vec![Cap::Mp { afi: bgp::AFI_IPV4, safi: bgp::SAFI_UNICAST }, Cap::RouteRefresh, Cap::As4(4_200_000_001)];
    let o4 = Open::new(4_200_000_001, 90, ip("9.9.9.9"), caps);
    t.eq("as_trans", (o4.asn, o4.peer_as(), o4.as4()), (bgp::AS_TRANS, 4_200_000_001, Some(4_200_000_001)));
    t.eq("open4 bytes", Message::Open(o4.clone()).to_bytes(true), unhex(OPEN4));
    match Message::read(&unhex(OPEN4), true) {
        Ok(Message::Open(o)) => t.check("open4 caps", o.has_cap(bgp::CAP_ROUTE_REFRESH) && o.ipv4_unicast() && o.peer_as() == 4_200_000_001 && o.hold == 90),
        other => t.check(&alloc::format!("open4 read {:?}", other), false),
    }
    t.eq("keepalive", Message::Keepalive.to_bytes(true), unhex(KEEPALIVE));
    t.eq("keepalive back", Message::read(&unhex(KEEPALIVE), true), Ok(Message::Keepalive));
    let n = Message::Notification(Notification { code: bgp::E_OPEN, sub: bgp::OE_PEER_AS, data: alloc::vec![0xfd, 0xea] });
    t.eq("notification", n.to_bytes(true), unhex(NOTIF));
    t.eq("notification back", Message::read(&unhex(NOTIF), true), Ok(n));
    t.eq("notification text", Notification::new(bgp::E_CEASE, bgp::CE_RESET).text(), String::from("Cease / Administrative Reset"));
    let rr = Message::RouteRefresh { afi: bgp::AFI_IPV4, safi: bgp::SAFI_UNICAST };
    t.eq("route refresh", rr.to_bytes(true), unhex(RR));
    t.eq("route refresh back", Message::read(&unhex(RR), true), Ok(rr));
    let wd = Message::Update(Update { withdrawn: alloc::vec![pf("10.2.0.0/24")], attrs: Vec::new(), nlri: Vec::new() });
    t.eq("withdrawal", wd.to_bytes(true), unhex(WD));
    t.eq("withdrawal back", Message::read(&unhex(WD), true), Ok(wd));
    // an UPDATE with 4-octet numbers
    let u = Update {
        withdrawn: alloc::vec![pf("10.9.0.0/16")],
        attrs: alloc::vec![
            Attr::new(AttrVal::Origin(bgp::ORIGIN_IGP)),
            Attr::new(AttrVal::AsPath(alloc::vec![Segment::seq(alloc::vec![65002, 65003])])),
            Attr::new(AttrVal::NextHop(ip("10.0.12.2"))),
            Attr::new(AttrVal::Med(0)),
        ],
        nlri: alloc::vec![pf("10.2.0.0/24"), pf("192.168.0.0/16")],
    };
    t.eq("update bytes", Message::Update(u.clone()).to_bytes(true), unhex(UPD));
    t.eq("update back", Message::read(&unhex(UPD), true), Ok(Message::Update(u.clone())));
    t.eq("update getters", (u.origin(), u.next_hop(), u.med(), u.local_pref(), u.as_path().map(bgp::path_len)), (Some(0), Some(ip("10.0.12.2")), Some(0), None, Some(2)));
    t.eq("wire length", u.wire_len(true), unhex(UPD).len());
}

/// AS_SET, 2-octet numbers, the extended length bit, optional attributes kept byte for byte
fn attributes(t: &mut T) {
    let b = unhex(UPD2);
    let m = Message::read(&b, false);
    let Ok(Message::Update(u)) = &m else {
        t.check(&alloc::format!("update2 read {:?}", m), false);
        return;
    };
    t.eq("round trip", Message::Update(u.clone()).to_bytes(false), b.clone());
    let path = u.as_path().map(|s| s.to_vec()).unwrap_or_default();
    t.eq("path", path.clone(), alloc::vec![Segment::seq(alloc::vec![65002]), Segment { set: true, asns: alloc::vec![65010, 65011] }]);
    t.eq("path text", bgp::path_text(&path), String::from("65002 {65010,65011}"));
    t.eq("set counts once", bgp::path_len(&path), 2);
    t.eq("extended length kept", u.attrs.iter().find(|a| a.kind() == bgp::A_AS_PATH).map(|a| a.flags), Some(0x50));
    t.eq("local pref", u.local_pref(), Some(200));
    t.check("atomic", u.get(bgp::A_ATOMIC_AGGREGATE) == Some(&AttrVal::AtomicAggregate));
    t.eq("aggregator", u.get(bgp::A_AGGREGATOR).cloned(), Some(AttrVal::Aggregator { asn: 65002, id: ip("2.2.2.2") }));
    t.eq("communities", u.communities().map(|c| c.iter().map(|x| bgp::community_text(*x)).collect::<Vec<_>>()), Some(alloc::vec![String::from("65001:100"), String::from("no-export")]));
    t.eq("unknown kept", u.get(99).cloned(), Some(AttrVal::Other { kind: 99, data: alloc::vec![1, 2, 3] }));
    // the same path on a 4-octet session reads differently: the dissector's guess
    let v4 = Attr::new(AttrVal::AsPath(alloc::vec![Segment::seq(alloc::vec![65002, 4_200_000_001])])).value(true);
    let v2 = Attr::new(AttrVal::AsPath(alloc::vec![Segment::seq(alloc::vec![65002, 65003])])).value(false);
    t.check("guess 4", bgp::as_path_4(&v4) && !bgp::as_path_4(&v2));
    // a 4-octet AS towards an old speaker is AS_TRANS
    let big = Attr::new(AttrVal::AsPath(alloc::vec![Segment::seq(alloc::vec![4_200_000_001, 65002])])).value(false);
    t.eq("as_trans in path", big, alloc::vec![2, 2, 0x5b, 0xa0, 0xfd, 0xea]);
    // a long value gets the extended length bit by itself
    let long = Attr::new(AttrVal::Communities((0..70).collect()));
    let mut w = Vec::new();
    long.write(&mut w, true);
    t.eq("extended by itself", (w[0], w[2], w[3], w.len()), (0xd0, 1, 24, 284));
    t.eq("flags of types", (Attr::new(AttrVal::Origin(0)).flags, Attr::new(AttrVal::Med(1)).flags, Attr::new(AttrVal::Communities(Vec::new())).flags), (0x40, 0x80, 0xc0));
    t.eq("prefix bytes", (bgp::prefix_len(&pf("0.0.0.0/0")), bgp::prefix_len(&pf("10.0.0.0/8")), bgp::prefix_len(&pf("10.1.2.0/23"))), (1, 2, 4));
    t.eq("prefixes read", bgp::read_prefixes(&[0x18, 10, 2, 0, 0, 0x20, 1, 2, 3, 4]), Ok(alloc::vec![pf("10.2.0.0/24"), pf("0.0.0.0/0"), pf("1.2.3.4/32")]));
    t.eq("origin codes", (bgp::origin_code(0), bgp::origin_code(1), bgp::origin_code(2)), ("i", "e", "?"));
}

/// messages out of a byte stream
fn stream(t: &mut T) {
    let mut s = unhex(OPEN);
    s.extend_from_slice(&unhex(KEEPALIVE));
    t.eq("first", bgp::frame(&s), Ok(Some(45)));
    t.eq("second", bgp::frame(&s[45..]), Ok(Some(19)));
    t.eq("partial header", bgp::frame(&s[..10]), Ok(None));
    t.eq("partial body", bgp::frame(&s[..30]), Ok(None));
    t.check("view", bgp::BgpView::parse(&s).is_some_and(|v| v.len() == 45 && v.kind() == bgp::T_OPEN && v.marker_ok() && v.body().len() == 26));
}

/// what a receiver refuses, and the NOTIFICATION it sends for it
fn malformed(t: &mut T) {
    let err = |h: &str, as4: bool| Message::read(&unhex(h), as4).err().map(|e| (e.code, e.sub, e.data));
    let attr = |b: &str, from: usize, n: usize| unhex(b)[from..from + n].to_vec();
    t.eq("marker", err(B_MARKER, true), Some((1, 1, Vec::new())));
    t.eq("too short", err(B_SHORT, true), Some((1, 2, alloc::vec![0, 18])));
    t.eq("too long", err(B_LONG, true), Some((1, 2, alloc::vec![0x10, 0x01])));
    t.eq("type", err(B_TYPE, true), Some((1, 3, alloc::vec![9])));
    t.eq("keepalive length", err(B_KALEN, true), Some((1, 2, alloc::vec![0, 20])));
    t.eq("version", err(B_VERSION, true), Some((2, 1, alloc::vec![0, 4])));
    t.eq("hold time", err(B_HOLD, true), Some((2, 6, Vec::new())));
    t.eq("identifier", err(B_ID, true), Some((2, 3, Vec::new())));
    t.eq("parameter", err(B_PARAM, true), Some((2, 4, Vec::new())));
    t.eq("missing next hop", err(B_NOHOP, true), Some((3, 3, alloc::vec![3])));
    t.eq("origin", err(B_ORIGIN, true), Some((3, 6, attr(B_ORIGIN, 23, 4))));
    t.eq("flags", err(B_FLAGS, true), Some((3, 4, attr(B_FLAGS, 23, 4))));
    t.eq("length", err(B_HOPLEN, true), Some((3, 5, attr(B_HOPLEN, 30, 8))));
    t.eq("twice", err(B_TWICE, true), Some((3, 1, Vec::new())));
    t.eq("withdrawn length", err(B_WDLEN, true), Some((3, 1, Vec::new())));
    t.eq("network field", err(B_NLRI, true), Some((3, 10, Vec::new())));
    t.eq("as path", err(B_PATH, true), Some((3, 11, Vec::new())));
    t.eq("well-known", err(B_WELLKNOWN, true), Some((3, 2, attr(B_WELLKNOWN, 37, 4))));
    t.eq("names", (bgp::error_name(2, 2), bgp::error_name(3, 3), bgp::error_name(4, 0), bgp::code_name(6)), ("Bad Peer AS", "Missing Well-known Attribute", "Hold Timer Expired", "Cease"));
    // a 2-octet path read as 4-octet numbers does not fit
    t.eq("2 as 4", err(UPD2, true).map(|e| (e.0, e.1)), Some((3, 11)));
}

/// a segment with BGP in a frame: the stack, the summary, the fields
fn frames(t: &mut T) {
    let frame = |payload: &[u8], sport: u16, dport: u16| -> Vec<u8> {
        let mut k = Pkt::new(HEADROOM, payload);
        TcpHdr { src_port: sport, dst_port: dport, seq: 1000, ack: 2000, flags: ACK | PSH, window: 4128, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut k, ip("10.0.12.1"), ip("10.0.12.2"));
        Ipv4Hdr::new(ip("10.0.12.1"), ip("10.0.12.2"), PROTO_TCP, 1).push_onto(&mut k);
        EthHdr { dst: Mac([0x02, 0x4a, 0, 0, 2, 1]), src: Mac([0x02, 0x4a, 0, 0, 1, 1]), ethertype: ET_IPV4 }.push_onto(&mut k);
        k.into_vec()
    };
    let mut two = unhex(OPEN);
    two.extend_from_slice(&unhex(KEEPALIVE));
    let f = frame(&two, 179, 50000);
    t.check("mask", decode::proto_mask(&f) & decode::P_BGP != 0 && decode::proto_mask(&f) & decode::P_TCP != 0);
    t.eq("summary", decode::info(&f), (Proto::Bgp, String::from("OPEN Message, AS 65001, hold time 180, ID 1.1.1.1, KEEPALIVE Message")));
    let ls = decode::layers(&f);
    match ls.iter().find(|l| l.proto == Proto::Bgp) {
        Some(l) => {
            t.eq("title", l.title.as_str(), "Border Gateway Protocol - OPEN Message, KEEPALIVE Message");
            let at = 14 + 20 + 20;
            let field = |n: &str| l.fields.iter().find(|x| x.name == n).map(|x| (x.value.clone(), x.off as usize, x.len as usize));
            t.eq("my as", field("My AS"), Some((String::from("65001"), at + 20, 2)));
            t.eq("hold", field("Hold Time"), Some((String::from("180 s"), at + 22, 2)));
            t.eq("id", field("BGP Identifier"), Some((String::from("1.1.1.1"), at + 24, 4)));
            let caps: Vec<&str> = l.fields.iter().filter(|x| x.name == "Capability").map(|x| x.value.as_str()).collect();
            t.eq("caps", caps, alloc::vec!["Multiprotocol extensions: IPv4 unicast", "Route refresh (2)", "4-octet AS number: 65001"]);
            t.eq("types", l.fields.iter().filter(|x| x.name == "Type").count(), 2);
            t.check("no problems", l.problems.is_empty());
        }
        None => t.check("bgp layer", false),
    }
    // an UPDATE: attributes by name with their flags, prefixes
    let f = frame(&unhex(UPD), 50000, 179);
    t.eq("update summary", decode::info(&f).1, String::from("UPDATE Message, 2 routes, 1 withdrawn"));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == Proto::Bgp);
    t.check("update fields", l.is_some_and(|l| {
        let v = |n: &str| l.fields.iter().filter(|x| x.name == n).map(|x| x.value.as_str()).collect::<Vec<_>>();
        v("AS_PATH") == ["65002 65003 [Well-known, Transitive]"] && v("NLRI") == ["10.2.0.0/24", "192.168.0.0/16"] && v("Withdrawn Route") == ["10.9.0.0/16"] && v("MULTI_EXIT_DISC") == ["0 [Optional]"] && l.problems.is_empty()
    }));
    // the 2-octet one: the dissector guesses the width
    let f = frame(&unhex(UPD2), 50000, 179);
    let ls = decode::layers(&f);
    t.check("update2 path", ls.iter().find(|l| l.proto == Proto::Bgp).is_some_and(|l| l.fields.iter().any(|x| x.name == "AS_PATH" && x.value == "65002 {65010,65011} [Well-known, Transitive, Extended Length]") && l.problems.is_empty()));
    // what a receiver refuses is a problem with its NOTIFICATION
    let f = frame(&unhex(B_NOHOP), 50000, 179);
    let ls = decode::layers(&f);
    t.check("problem", ls.iter().find(|l| l.proto == Proto::Bgp).is_some_and(|l| l.problems == ["UPDATE: UPDATE Message Error / Missing Well-known Attribute (the receiver answers with NOTIFICATION 3/3)"]));
    t.eq("notification summary", decode::info(&frame(&unhex(NOTIF), 179, 50000)).1, String::from("NOTIFICATION Message, 2/2 Bad Peer AS"));
    // the middle of a message and its first part
    let b = unhex(UPD);
    t.eq("continuation", decode::info(&frame(&b[30..], 50000, 179)).1, String::from("Continuation"));
    t.eq("segment", decode::info(&frame(&b[..30], 50000, 179)).1, String::from("Segment of a message"));
}
