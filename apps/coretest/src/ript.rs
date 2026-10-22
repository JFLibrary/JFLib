// ript -- checks of netpkt::rip and its dissector: version 2 responses and requests byte for
// byte (RFC 2453), version 1 entries without masks (RFC 1058), the authentication entry with a
// plain-text password, round trips view -> record -> bytes, and whole frames from Ethernet down
// with their checksums.  The expected frames were computed independently (Python, struct and
// the RFC 1071 sum).

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::rip::{self, Auth, Rip, RipView, Rte};
use netpkt::udp::{UdpHdr, PORT_RIP};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::T;

/// bytes from hex digits separated by spaces
fn unhex(s: &str) -> Vec<u8> {
    s.split_whitespace().filter_map(|x| u8::from_str_radix(x, 16).ok()).collect()
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or_default()
}

/// a RIP message in a frame: Ethernet, IPv4 (TTL 1, the given id), UDP 520 to 520
fn frame(src: Ipv4, dst: Ipv4, dmac: Mac, smac: Mac, id: u16, msg: &[u8]) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, msg);
    UdpHdr::new(PORT_RIP, PORT_RIP).push_v4(&mut p, src, dst);
    let mut h = Ipv4Hdr::new(src, dst, PROTO_UDP, 1);
    h.id = id;
    h.push_onto(&mut p);
    EthHdr { dst: dmac, src: smac, ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

pub fn run(t: &mut T) {
    messages(t);
    version1(t);
    authentication(t);
    frames(t);
    problems(t);
}

fn messages(t: &mut T) {
    let m = Rip::response(2, alloc::vec![Rte::v2(pf("10.0.1.0/24"), Ipv4::UNSPECIFIED, 1, 0), Rte::v2(pf("10.0.4.7/24"), Ipv4::UNSPECIFIED, rip::INFINITY, 0)]);
    let b = m.to_bytes();
    let want = unhex("02 02 00 00 00 02 00 00 0a 00 01 00 ff ff ff 00 00 00 00 00 00 00 00 01 00 02 00 00 0a 00 04 00 ff ff ff 00 00 00 00 00 00 00 00 10");
    t.eq("v2 response bytes", &b, &want);
    match RipView::parse(&b) {
        Some(v) => {
            t.eq("v2 header", (v.command(), v.version(), v.mbz(), v.count(), v.trailing()), (rip::CMD_RESPONSE, 2, 0, 2, 0));
            t.eq("v2 first entry", v.entry(0), Some(Rte { afi: 2, tag: 0, addr: Ipv4([10, 0, 1, 0]), mask: Ipv4([255, 255, 255, 0]), next_hop: Ipv4::UNSPECIFIED, metric: 1 }));
            t.eq("v2 prefix (host bits cleared)", v.entry(1).and_then(|e| e.prefix()), Some(pf("10.0.4.0/24")));
            t.eq("v2 round trip", Rip::read(&v), m.clone());
            t.check("no authentication", v.auth().is_none() && v.routes().count() == 2);
        }
        None => t.check("v2 parses", false),
    }
    // the request for the whole table (RFC 2453 3.9.1)
    let r = Rip::request_all(2);
    t.eq("request for the table", r.to_bytes(), unhex("01 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 10"));
    t.check("is the request for the table", r.is_request_all() && !m.is_request_all());
    let one = Rip { command: rip::CMD_REQUEST, version: 2, auth: None, entries: alloc::vec![Rte::v2(pf("10.0.2.0/24"), Ipv4::UNSPECIFIED, rip::INFINITY, 0)] };
    t.check("a request for one route is not", !one.is_request_all());
    // a route tag and a next hop
    let e = Rte::v2(pf("192.168.7.0/24"), Ipv4([10, 0, 12, 9]), 3, 0x0102);
    let mut eb = [0u8; 20];
    e.write(&mut eb);
    t.eq("tag and next hop", &eb[..], &unhex("00 02 01 02 c0 a8 07 00 ff ff ff 00 0a 00 0c 09 00 00 00 03")[..]);
    t.eq("entry read back", Rte::read(&eb), e);
    t.eq("default route", Rte::v2(pf("0.0.0.0/0"), Ipv4::UNSPECIFIED, 1, 0).prefix(), None);
    t.check("mask with gaps", Rte { mask: Ipv4([255, 0, 255, 0]), ..e }.prefix().is_none());
    t.eq("command names", (rip::command_name(1), rip::command_name(2), rip::command_name(9)), ("Request", "Response", "Unknown"));
    t.eq("group", rip::GROUP, Ipv4([224, 0, 0, 9]));
}

fn version1(t: &mut T) {
    let m = Rip::response(1, alloc::vec![Rte::v1(Ipv4([172, 16, 0, 0]), 2), Rte::v1(Ipv4([10, 1, 2, 0]), 1)]);
    let b = m.to_bytes();
    t.eq("v1 response bytes", &b, &unhex("02 01 00 00 00 02 00 00 ac 10 00 00 00 00 00 00 00 00 00 00 00 00 00 02 00 02 00 00 0a 01 02 00 00 00 00 00 00 00 00 00 00 00 00 01"));
    let v = RipView::parse(&b);
    t.check("v1 must-be-zero fields clear", v.is_some_and(|v| v.mbz() == 0 && v.entries().all(|e| e.mbz_clear() && e.prefix().is_none())));
    let mut bad = b.clone();
    bad[7] = 1;
    t.check("v1 tag set", RipView::parse(&bad).and_then(|v| v.entry(0)).is_some_and(|e| !e.mbz_clear()));
    t.eq("v1 request bytes", Rip::request_all(1).to_bytes(), unhex("01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 10"));
}

fn authentication(t: &mut T) {
    let mut m = Rip::response(2, alloc::vec![Rte::v2(pf("10.0.9.0/24"), Ipv4::UNSPECIFIED, 2, 0)]);
    m.auth = Some(Auth::password(b"secret"));
    let b = m.to_bytes();
    t.eq("authentication entry", &b[4..24], &unhex("ff ff 00 02 73 65 63 72 65 74 00 00 00 00 00 00 00 00 00 00")[..]);
    t.eq("message length", b.len(), 44);
    match RipView::parse(&b) {
        Some(v) => {
            t.eq("password", v.auth().and_then(|a| a.text()), Some(String::from("secret")));
            t.eq("routes after it", v.routes().map(|e| e.prefix()).collect::<Vec<_>>(), alloc::vec![Some(pf("10.0.9.0/24"))]);
            t.eq("record keeps it apart", Rip::read(&v), m.clone());
        }
        None => t.check("authenticated message parses", false),
    }
    let long = Auth::password(b"0123456789abcdefXYZ");
    t.eq("password cut at 16 bytes", long.text(), Some(String::from("0123456789abcdef")));
    t.eq("md5 has no text", Auth { kind: rip::AUTH_MD5, data: [7; 16] }.text(), None);
}

fn frames(t: &mut T) {
    let m = Rip::response(2, alloc::vec![Rte::v2(pf("10.0.1.0/24"), Ipv4::UNSPECIFIED, 1, 0), Rte::v2(pf("10.0.4.0/24"), Ipv4::UNSPECIFIED, rip::INFINITY, 0)]);
    let f = frame(Ipv4([10, 0, 12, 1]), rip::GROUP, Mac::of_ipv4_multicast(rip::GROUP), Mac([0x02, 0x4A, 0, 0, 1, 1]), 1, &m.to_bytes());
    // IPv4 checksum 0xc39a, UDP checksum 0xec52 (computed independently)
    let mut want = unhex("01 00 5e 00 00 09 02 4a 00 00 01 01 08 00 45 00 00 48 00 01 00 00 01 11 c3 9a 0a 00 0c 01 e0 00 00 09 02 08 02 08 00 34 ec 52");
    want.extend(unhex("02 02 00 00 00 02 00 00 0a 00 01 00 ff ff ff 00 00 00 00 00 00 00 00 01 00 02 00 00 0a 00 04 00 ff ff ff 00 00 00 00 00 00 00 00 10"));
    t.eq("v2 frame byte for byte", &f, &want);
    t.eq("proto mask", decode::proto_mask(&f) & (decode::P_RIP | decode::P_UDP | decode::P_IPV4), decode::P_RIP | decode::P_UDP | decode::P_IPV4);
    t.eq("summary", decode::info(&f), (Proto::Rip, String::from("RIPv2 Response, 2 routes")));
    let ls = decode::layers(&f);
    match ls.iter().find(|l| l.proto == Proto::Rip) {
        Some(l) => {
            t.eq("layer", (l.title.as_str(), l.off, l.len, l.problems.len()), ("Routing Information Protocol (RIPv2 Response)", 42, 44, 0));
            let fs: Vec<(&str, &str, u32, u32)> = l.fields.iter().map(|x| (x.name, x.value.as_str(), x.off, x.len)).collect();
            t.eq(
                "fields",
                fs,
                alloc::vec![
                    ("Command", "Response (2)", 42, 1),
                    ("Version", "RIPv2 (2)", 43, 1),
                    ("Must be zero", "0x0000", 44, 2),
                    ("IP Address", "10.0.1.0/24, Metric: 1", 46, 20),
                    ("IP Address", "10.0.4.0/24, Metric: 16 (unreachable)", 66, 20),
                ],
            );
        }
        None => t.check("rip layer", false),
    }
    t.check("udp checksum correct", ls.iter().any(|l| l.proto == Proto::Udp && l.problems.is_empty() && l.fields.iter().any(|x| x.name == "Checksum" && x.value == "0xec52 [correct]")));
    // a version 1 request for the whole table, broadcast (IPv4 0xf809, UDP 0x38e4)
    let f1 = frame(Ipv4([192, 168, 1, 1]), Ipv4::BROADCAST, Mac::BROADCAST, Mac([0x02, 0x4A, 0, 0, 2, 1]), 7, &Rip::request_all(1).to_bytes());
    t.eq(
        "v1 request frame byte for byte",
        &f1,
        &unhex("ff ff ff ff ff ff 02 4a 00 00 02 01 08 00 45 00 00 34 00 07 00 00 01 11 f8 09 c0 a8 01 01 ff ff ff ff 02 08 02 08 00 20 38 e4 01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 10"),
    );
    t.eq("v1 request summary", decode::info(&f1).1, String::from("RIPv1 Request for the whole table"));
    let req = decode::layers(&f1);
    t.check("request entry", req.iter().any(|l| l.proto == Proto::Rip && l.fields.iter().any(|x| x.name == "Request" && x.off == 46 && x.len == 20)));
    // authenticated, with a tag and a next hop
    let mut a = Rip::response(2, alloc::vec![Rte::v2(pf("192.168.7.0/24"), Ipv4([10, 0, 12, 9]), 3, 5)]);
    a.auth = Some(Auth::password(b"lab"));
    let fa = frame(Ipv4([10, 0, 12, 2]), rip::GROUP, Mac::of_ipv4_multicast(rip::GROUP), Mac([0x02, 0x4A, 0, 0, 3, 1]), 2, &a.to_bytes());
    t.eq("authenticated summary", decode::info(&fa).1, String::from("RIPv2 Response, 1 route, authenticated"));
    let la = decode::layers(&fa);
    let rl = la.iter().find(|l| l.proto == Proto::Rip);
    t.check("authentication field", rl.is_some_and(|l| l.fields.iter().any(|x| x.name == "Authentication" && x.value == "Simple Password: lab" && x.off == 46)));
    t.check("next hop and tag", rl.is_some_and(|l| l.fields.iter().any(|x| x.name == "IP Address" && x.value == "192.168.7.0/24, Metric: 3, Next Hop: 10.0.12.9, Route Tag: 5" && x.off == 66)));
    // a request for two routes
    let q = Rip { command: rip::CMD_REQUEST, version: 1, auth: None, entries: alloc::vec![Rte::v1(Ipv4([10, 0, 0, 0]), 16), Rte::v1(Ipv4([10, 1, 0, 0]), 16)] };
    let fq = frame(Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]), Mac([2, 0, 0, 0, 0, 2]), Mac([2, 0, 0, 0, 0, 1]), 3, &q.to_bytes());
    t.eq("request for entries", decode::info(&fq).1, String::from("RIPv1 Request, 2 entries"));
}

fn problems(t: &mut T) {
    let probs = |msg: &[u8]| -> Vec<String> {
        let f = frame(Ipv4([10, 0, 0, 1]), rip::GROUP, Mac::of_ipv4_multicast(rip::GROUP), Mac([2, 0, 0, 0, 0, 1]), 9, msg);
        decode::layers(&f).into_iter().find(|l| l.proto == Proto::Rip).map(|l| l.problems).unwrap_or_default()
    };
    let mut v1 = Rip::response(1, alloc::vec![Rte::v1(Ipv4([10, 0, 0, 0]), 1)]).to_bytes();
    v1[13] = 255;
    t.eq("v1 with a mask", probs(&v1), alloc::vec![String::from("a must-be-zero field is set: RIPv1 receivers ignore the message (RFC 1058)")]);
    let mut odd = Rip::response(2, alloc::vec![Rte::v2(pf("10.0.0.0/8"), Ipv4::UNSPECIFIED, 0, 0)]).to_bytes();
    odd.extend_from_slice(&[1, 2, 3]);
    t.eq("metric 0 and trailing bytes", probs(&odd), alloc::vec![String::from("metric 0 of 10.0.0.0 is out of range (1 to 16)"), String::from("3 bytes after the last entry")]);
    let mut gaps = Rip::response(2, alloc::vec![Rte::v2(pf("10.0.0.0/8"), Ipv4::UNSPECIFIED, 1, 0)]).to_bytes();
    gaps[14] = 255;
    t.eq("mask with gaps", probs(&gaps), alloc::vec![String::from("netmask 255.0.255.0 of 10.0.0.0 is not contiguous")]);
    let mut cmd = Rip::request_all(2).to_bytes();
    cmd[0] = 7;
    t.eq("unknown command", probs(&cmd), alloc::vec![String::from("command 7 is not a request or a response")]);
    t.eq("too short", probs(&[2, 2, 0]), alloc::vec![String::from("shorter than a RIP header")]);
    let many: Vec<Rte> = (0..26u8).map(|i| Rte::v2(Prefix4 { addr: Ipv4([10, i, 0, 0]), len: 16 }, Ipv4::UNSPECIFIED, 1, 0)).collect();
    t.eq("26 entries", probs(&Rip::response(2, many).to_bytes()), alloc::vec![String::from("26 entries, more than 25")]);
}
