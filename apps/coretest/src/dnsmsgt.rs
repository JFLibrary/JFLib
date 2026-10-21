// dnsmsgt -- checks of netpkt::dns: the compression example of RFC 1035 4.1.4 (F.ISI.ARPA at
// offset 20, FOO.F.ISI.ARPA at 40, ARPA at 64, the root at 92), the standard query and answer
// of RFC 1035 6.2.1 (SRI-NIC.ARPA, written by hand following section 4), every record type,
// truncation at 512 bytes, refused names and pointers, the helpers for names and the dissector

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use netpkt::decode::{self, Proto};
use netpkt::dns::{self, Compressor, DnsErr, Header, Message, RData, Rr, Soa};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::udp::UdpHdr;
use netpkt::{Ipv4, Ipv6, Mac};

use crate::T;

pub fn run(t: &mut T) {
    rfc_compression(t);
    rfc_query(t);
    records(t);
    truncation(t);
    refused(t);
    names(t);
    dissect(t);
}

fn rfc_compression(t: &mut T) {
    // RFC 1035 4.1.4: the names written one after another into one message
    let mut b = vec![0u8; 20];
    let mut c = Compressor::new();
    t.check("write F.ISI.ARPA", dns::write_name(&mut b, "F.ISI.ARPA", &mut c));
    b.resize(40, 0);
    t.check("write FOO.F.ISI.ARPA", dns::write_name(&mut b, "FOO.F.ISI.ARPA", &mut c));
    b.resize(64, 0);
    t.check("write ARPA", dns::write_name(&mut b, "ARPA", &mut c));
    b.resize(92, 0);
    t.check("write root", dns::write_name(&mut b, "", &mut c));
    t.eq("F.ISI.ARPA at 20", &b[20..32], &[1, b'F', 3, b'I', b'S', b'I', 4, b'A', b'R', b'P', b'A', 0][..]);
    t.eq("FOO and a pointer to 20", &b[40..46], &[3, b'F', b'O', b'O', 0xC0, 20][..]);
    t.eq("ARPA as a pointer to 26", &b[64..66], &[0xC0, 26][..]);
    t.eq("root", (b.len(), b[92]), (93, 0));
    t.eq("read 20", dns::read_name(&b, 20), Some((String::from("F.ISI.ARPA"), 32)));
    t.eq("read 40", dns::read_name(&b, 40), Some((String::from("FOO.F.ISI.ARPA"), 46)));
    t.eq("read 64", dns::read_name(&b, 64), Some((String::from("ARPA"), 66)));
    t.eq("read 92", dns::read_name(&b, 92), Some((String::new(), 93)));
    // case does not matter for the pointers (RFC 4343)
    let mut d = Vec::new();
    let mut c = Compressor::new();
    dns::write_name(&mut d, "www.Example.com", &mut c);
    dns::write_name(&mut d, "mail.example.COM", &mut c);
    t.eq("case-insensitive pointer", &d[17..], &[4, b'm', b'a', b'i', b'l', 0xC0, 4][..]);
    let mut c = Compressor::new();
    c.off = true;
    let mut d = Vec::new();
    dns::write_name(&mut d, "a.b", &mut c);
    dns::write_name(&mut d, "a.b", &mut c);
    t.eq("compression off", d.len(), 10);
}

/// RFC 1035 6.2.1: a query for SRI-NIC.ARPA, type A, and the authoritative answer with the
/// two addresses 26.0.0.73 and 10.0.0.51 (TTL 86400); id 0x1234
fn sri_nic() -> (Vec<u8>, Vec<u8>) {
    let mut q = vec![0x12, 0x34, 0x00, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(&[7, b'S', b'R', b'I', b'-', b'N', b'I', b'C', 4, b'A', b'R', b'P', b'A', 0, 0, 1, 0, 1]);
    let mut r = q.clone();
    r[2] = 0x84; // QR, AA
    r[7] = 2;
    for a in [[26, 0, 0, 73], [10, 0, 0, 51]] {
        r.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0x00, 0x01, 0x51, 0x80, 0, 4]);
        r.extend_from_slice(&a);
    }
    (q, r)
}

fn rfc_query(t: &mut T) {
    let (qb, rb) = sri_nic();
    let q = Message::query(0x1234, "SRI-NIC.ARPA", dns::TYPE_A, false);
    t.eq("query bytes", q.to_bytes(), qb.clone());
    t.eq("query parses", Message::parse(&qb), Ok(q.clone()));
    let mut r = Message::reply_to(&q);
    r.header.aa = true;
    r.answers = vec![Rr::new("SRI-NIC.ARPA", 86_400, RData::A(Ipv4([26, 0, 0, 73]))), Rr::new("SRI-NIC.ARPA", 86_400, RData::A(Ipv4([10, 0, 0, 51])))];
    t.eq("answer bytes", r.to_bytes(), rb.clone());
    match Message::parse(&rb) {
        Ok(m) => {
            t.eq("answer header", m.header, Header { id: 0x1234, qr: true, aa: true, ..Header::default() });
            t.eq("answer addresses", m.answers.iter().map(|a| a.data.clone()).collect::<Vec<_>>(), vec![RData::A(Ipv4([26, 0, 0, 73])), RData::A(Ipv4([10, 0, 0, 51]))]);
            t.eq("answer names", m.answers.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["SRI-NIC.ARPA", "SRI-NIC.ARPA"]);
        }
        Err(e) => t.check(&format!("answer parses: {:?}", e), false),
    }
    t.eq("flags", (Header::from_flags(0, 0x8180).flags(), Header::from_flags(0, 0x8180).ra, Header::from_flags(0, 0x8183).rcode), (0x8180, true, dns::RCODE_NXDOMAIN));
}

fn records(t: &mut T) {
    let soa = Soa { mname: String::from("ns.lab"), rname: String::from("hostmaster.lab"), serial: 2026101901, refresh: 3600, retry: 600, expire: 86_400, minimum: 300 };
    let mut m = Message::reply_to(&Message::query(7, "lab", dns::TYPE_ANY, true));
    m.header.aa = true;
    m.header.ra = true;
    m.answers = vec![
        Rr::new("lab", 3600, RData::Soa(soa.clone())),
        Rr::new("lab", 3600, RData::Ns(String::from("ns.lab"))),
        Rr::new("lab", 3600, RData::Mx { pref: 10, host: String::from("mail.lab") }),
        Rr::new("www.lab", 300, RData::Cname(String::from("web.lab"))),
        Rr::new("web.lab", 300, RData::A(Ipv4([10, 0, 2, 20]))),
        Rr::new("web.lab", 300, RData::Aaaa(Ipv6::parse("fd00::20").unwrap_or(Ipv6::UNSPECIFIED))),
        Rr::new("20.2.0.10.in-addr.arpa", 300, RData::Ptr(String::from("web.lab"))),
        Rr::new("lab", 300, RData::Txt(vec![b"v=spf1 -all".to_vec(), b"say \"hi\"".to_vec()])),
        Rr { name: String::from("lab"), rtype: 99, class: dns::CLASS_IN, ttl: 5, data: RData::Other(vec![1, 2, 3]) },
    ];
    m.additional = vec![Rr { name: String::new(), rtype: dns::TYPE_OPT, class: 1232, ttl: 0, data: RData::Other(Vec::new()) }];
    let b = m.to_bytes();
    t.eq("every type round trip", Message::parse(&b), Ok(m.clone()));
    t.eq(
        "text forms",
        m.answers.iter().map(|r| r.data.text()).collect::<Vec<_>>(),
        vec![
            String::from("ns.lab hostmaster.lab 2026101901 3600 600 86400 300"),
            String::from("ns.lab"),
            String::from("10 mail.lab"),
            String::from("web.lab"),
            String::from("10.0.2.20"),
            String::from("fd00::20"),
            String::from("web.lab"),
            String::from("\"v=spf1 -all\" \"say \\\"hi\\\"\""),
            String::from("\\# 3 010203"),
        ],
    );
    t.eq("type names", [dns::type_name(1), dns::type_name(28), dns::type_name(99)], [String::from("A"), String::from("AAAA"), String::from("TYPE99")]);
    t.eq("type by name", [dns::type_by_name("mx"), dns::type_by_name("TYPE99"), dns::type_by_name("bogus")], [Some(dns::TYPE_MX), Some(99), None]);
    t.eq("rcode names", [dns::rcode_name(0), dns::rcode_name(3), dns::rcode_name(5)], ["NOERROR", "NXDOMAIN", "REFUSED"]);
    // compression shrinks the repeated names: "lab" is written once
    let plain = m.answers.iter().filter(|r| r.name == "lab").count();
    t.check("compressed", plain == 5 && b.windows(4).filter(|w| *w == b"\x03lab").count() == 1);
}

fn truncation(t: &mut T) {
    let mut m = Message::reply_to(&Message::query(9, "big.test", dns::TYPE_A, true));
    m.answers = (0..40u8).map(|i| Rr::new("big.test", 60, RData::A(Ipv4([10, 9, 0, i])))).collect();
    let (b, cut) = m.to_bytes_max(dns::UDP_MAX);
    // 12 header + 14 question + 16 per compressed answer: 30 answers fit into 512
    t.eq("truncated", (cut, b.len()), (true, 12 + 14 + 30 * 16));
    match Message::parse(&b) {
        Ok(r) => t.eq("tc and count", (r.header.tc, r.answers.len()), (true, 30)),
        Err(e) => t.check(&format!("truncated parses: {:?}", e), false),
    }
    let (b2, cut2) = m.to_bytes_max(usize::MAX);
    t.eq("whole", (cut2, b2.len()), (false, 12 + 14 + 40 * 16));
    // a record with an invalid name is left out, the others stay
    let mut bad = Message::reply_to(&Message::query(1, "x.test", dns::TYPE_A, false));
    bad.answers = vec![Rr::new("a..b", 1, RData::A(Ipv4([1, 1, 1, 1]))), Rr::new("x.test", 1, RData::Cname(String::from("y.test")))];
    let pb = bad.to_bytes();
    t.eq("invalid record skipped", Message::parse(&pb).map(|m| (m.header.tc, m.answers.len(), m.answers[0].data.clone())), Ok((false, 1, RData::Cname(String::from("y.test")))));
}

fn refused(t: &mut T) {
    let (_, rb) = sri_nic();
    t.eq("short header", Message::parse(&rb[..11]), Err(DnsErr::Short));
    t.eq("cut record", Message::parse(&rb[..rb.len() - 2]), Err(DnsErr::Short));
    // a pointer to itself, a pointer forward, label type 0x80
    let mut l = rb.clone();
    l[30] = 0xC0;
    l[31] = 30;
    t.eq("pointer loop", Message::parse(&l), Err(DnsErr::BadName));
    let mut f = rb.clone();
    f[31] = 50;
    t.eq("pointer forward", Message::parse(&f), Err(DnsErr::BadName));
    let mut e = rb.clone();
    e[12] = 0x87;
    t.eq("extended label", Message::parse(&e), Err(DnsErr::BadName));
    // an A record of five bytes
    let mut a = rb.clone();
    a[41] = 5;
    a.push(0);
    t.eq("bad A length", Message::parse(&a).map(|_| ()), Err(DnsErr::BadRecord));
    // 128 labels of one byte make a name of 257 bytes
    let long: Vec<String> = (0..128).map(|_| String::from("a")).collect();
    t.eq("name too long", dns::labels(&long.join(".")), None);
    let ok: Vec<String> = (0..127).map(|_| String::from("a")).collect();
    t.eq("name of 255", dns::labels(&ok.join(".")).map(|l| l.len()), Some(127));
    t.eq("label of 64", dns::labels(&"x".repeat(64)), None);
    t.eq("empty label", dns::labels("a..b"), None);
}

fn names(t: &mut T) {
    t.eq("escapes", dns::labels("a\\.b.c\\032d.e\\\\"), Some(vec![b"a.b".to_vec(), b"c d".to_vec(), b"e\\".to_vec()]));
    let mut b = Vec::new();
    dns::write_name(&mut b, "a\\.b.c\\032d", &mut Compressor::new());
    t.eq("escaped text back", dns::read_name(&b, 0).map(|x| x.0), Some(String::from("a\\.b.c\\032d")));
    t.eq("final dot", dns::labels("lab."), dns::labels("lab"));
    t.eq("root forms", (dns::labels(""), dns::labels(".")), (Some(Vec::new()), Some(Vec::new())));
    t.check("name eq", dns::name_eq("WWW.Lab.", "www.lab") && !dns::name_eq("www.lab", "ww.lab"));
    t.check("subdomain", dns::is_subdomain("www.lab", "lab") && dns::is_subdomain("lab", "LAB") && dns::is_subdomain("x", "") && !dns::is_subdomain("xlab", "lab") && !dns::is_subdomain("lab", "www.lab"));
    t.eq("parent", (dns::parent("www.lab"), dns::parent("lab"), dns::parent("a\\.b.c")), ("lab", "", "c"));
    t.eq("reverse", dns::reverse_v4(Ipv4([10, 0, 2, 20])), String::from("20.2.0.10.in-addr.arpa"));
    t.eq("parse reverse", (dns::parse_reverse_v4("20.2.0.10.IN-ADDR.ARPA."), dns::parse_reverse_v4("2.0.10.in-addr.arpa")), (Some(Ipv4([10, 0, 2, 20])), None));
}

/// a UDP datagram 10.0.0.1:40000 -> 10.0.0.53:53 (or back) in an Ethernet frame
fn udp_frame(payload: &[u8], reply: bool) -> Vec<u8> {
    let (a, b) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 53]));
    let (src, dst, sp, dp) = if reply { (b, a, 53, 40000) } else { (a, b, 40000, 53) };
    let mut p = Pkt::new(HEADROOM, payload);
    UdpHdr::new(sp, dp).push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 2]), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

fn dissect(t: &mut T) {
    let (qb, rb) = sri_nic();
    let qf = udp_frame(&qb, false);
    t.eq("query info", decode::info(&qf), (Proto::Dns, String::from("Standard query 0x1234 A SRI-NIC.ARPA")));
    let rf = udp_frame(&rb, true);
    t.eq("answer info", decode::info(&rf), (Proto::Dns, String::from("Standard query response 0x1234 A SRI-NIC.ARPA A 26.0.0.73 A 10.0.0.51")));
    let mut nx = Message::reply_to(&Message::query(0x0042, "nosuch.lab", dns::TYPE_A, true));
    nx.header.rcode = dns::RCODE_NXDOMAIN;
    nx.authority = vec![Rr::new("lab", 300, RData::Soa(Soa { mname: String::from("ns.lab"), rname: String::from("hostmaster.lab"), serial: 1, refresh: 2, retry: 3, expire: 4, minimum: 300 }))];
    t.eq("nxdomain info", decode::info(&udp_frame(&nx.to_bytes(), true)).1, String::from("Standard query response 0x0042 No such name A nosuch.lab SOA ns.lab"));
    let ls = decode::layers(&rf);
    let Some(l) = ls.iter().find(|l| l.proto == Proto::Dns) else {
        t.check("dns layer", false);
        return;
    };
    t.eq("title", l.title.as_str(), "Domain Name System (response)");
    t.check("no problems", l.problems.is_empty());
    let f: Vec<(&str, &str, u32, u32)> = l.fields.iter().map(|x| (x.name, x.value.as_str(), x.off, x.len)).collect();
    t.eq("id and flags", (f[0], f[1].1), (("Transaction ID", "0x1234", 42, 2), "0x8400 Standard query response, No error"));
    t.eq("query field", f[6], ("Query", "SRI-NIC.ARPA: type A, class IN", 54, 18));
    t.eq("answer field", f[7], ("Answer", "SRI-NIC.ARPA: type A, class IN, addr 26.0.0.73, ttl 86400", 72, 16));
    t.eq("second answer", (f[8].2, f[8].3), (88, 16));
    let mut cut = rf.clone();
    cut[44] |= 0x02;
    t.check("tc noted", decode::layers(&cut).iter().any(|l| l.proto == Proto::Dns && l.problems.iter().any(|p| p.contains("truncated"))));
}
