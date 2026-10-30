// radiust -- checks of netpkt::radius and netpkt::telnet with their dissectors: the example
// packets of RFC 2865 7.1 and 7.2 byte for byte (the hidden password of "nemo", the Response
// Authenticators with the secret "xyzzy5461"), the hiding of User-Password both ways for every
// length, the Accounting-Request's authenticator (RFC 2866 3), the Message-Authenticator (RFC
// 3579 3.2) of requests and answers, Vendor-Specific attributes as bytes, what parse refuses;
// Telnet's IAC escaping, NVT line ends, a reader fed in pieces, parse with offsets, option
// negotiation after RFC 1143 (accepted, refused, never in a circle) and whole frames with their
// layers and summaries.

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::radius::{self, Attr, Bad, Packet, RadiusView};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::telnet::{self, Item, Options, Reader};
use netpkt::udp::UdpHdr;
use netpkt::{Ipv4, Mac};

use crate::T;

fn h(s: &str) -> Vec<u8> {
    crypto::unhex(&s.replace(' ', "")).expect("hex")
}

fn a16(b: &[u8]) -> [u8; 16] {
    let mut a = [0u8; 16];
    a.copy_from_slice(&b[..16]);
    a
}

const SECRET: &[u8] = b"xyzzy5461";
/// RFC 2865 7.1: nemo telnets to a host, the NAS asks with the password "arctangent"
const REQ_71: &str = "01 00 00 38 0f 40 3f 94 73 97 80 57 bd 83 d5 cb 98 f4 22 7a 01 06 6e 65 6d 6f 02 12 0d be 70 8d 93 d4 13 ce 31 96 e4 3f 78 2a 0a ee 04 06 c0 a8 01 10 05 06 00 00 00 03";
const ACC_71: &str = "02 00 00 26 86 fe 22 0e 76 24 ba 2a 10 05 f6 bf 9b 55 e0 b2 06 06 00 00 00 01 0f 06 00 00 00 00 0e 06 c0 a8 01 03";
/// RFC 2865 7.2: flopsy with CHAP and PPP
const REQ_72: &str = "01 01 00 47 2a ee 86 f0 8d 0d 55 96 9c a5 97 8e 0d 33 67 a2 01 08 66 6c 6f 70 73 79 03 13 16 e9 75 57 c3 16 18 58 95 f2 93 ff 63 44 07 72 75 04 06 c0 a8 01 10 05 06 00 00 00 14 06 06 00 00 00 02 07 06 00 00 00 01";
const ACC_72: &str = "02 01 00 38 15 ef bc 7d ab 26 cf a3 dc 34 d9 c0 3c 86 01 a4 06 06 00 00 00 02 07 06 00 00 00 01 08 06 ff ff ff fe 0a 06 00 00 00 02 0d 06 00 00 00 01 0c 06 00 00 05 dc";

/// a UDP datagram from 10.0.0.1:sport to 10.0.0.10:dport in a frame
fn udp_frame(sport: u16, dport: u16, msg: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 10]));
    let mut p = Pkt::new(HEADROOM, msg);
    UdpHdr::new(sport, dport).push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 10]), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

/// a TCP segment from 10.0.0.1:sport to 10.0.0.10:dport in a frame
fn tcp_frame(sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 10]));
    let mut p = Pkt::new(HEADROOM, data);
    TcpHdr { src_port: sport, dst_port: dport, seq: 1000, ack: 2000, flags: ACK | PSH, window: 4128, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_TCP, 255).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 10]), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

pub fn run(t: &mut T) {
    rfc_examples(t);
    passwords(t);
    accounting(t);
    message_auth(t);
    records(t);
    refused(t);
    radius_frames(t);
    telnet_bytes(t);
    telnet_reader(t);
    telnet_options(t);
    telnet_frames(t);
}

/// RFC 2865 7.1 and 7.2: the packets read and written again, the password, the answers
fn rfc_examples(t: &mut T) {
    let req = h(REQ_71);
    let Ok(p) = Packet::parse(&req) else {
        t.check("7.1 request parses", false);
        return;
    };
    t.eq("7.1 header", (p.code, p.id, req.len()), (radius::ACCESS_REQUEST, 0, 56));
    t.eq("7.1 user", p.text(radius::A_USER_NAME), Some(String::from("nemo")));
    t.eq("7.1 nas", (p.addr(radius::A_NAS_IP_ADDRESS), p.int(radius::A_NAS_PORT)), (Some(Ipv4([192, 168, 1, 16])), Some(3)));
    let hidden = p.attr(radius::A_USER_PASSWORD).map(|a| a.value.clone()).unwrap_or_default();
    t.eq("7.1 password revealed", radius::reveal_password(&hidden, SECRET, &p.auth, crypto::md5_parts), Some(b"arctangent".to_vec()));
    t.eq("7.1 password hidden", radius::hide_password(b"arctangent", SECRET, &p.auth, crypto::md5_parts), hidden);
    t.eq("7.1 round trip", p.to_bytes(), req.clone());
    let acc = h(ACC_71);
    let ra = a16(&req[4..20]);
    t.check("7.1 response authenticator", radius::check_response_auth(&acc, &ra, SECRET, crypto::md5_parts));
    t.eq("7.1 response authenticator value", radius::response_auth(&acc, &ra, SECRET, crypto::md5_parts).to_vec(), acc[4..20].to_vec());
    t.check("7.1 wrong secret", !radius::check_response_auth(&acc, &ra, b"xyzzy5460", crypto::md5_parts));
    match Packet::parse(&acc) {
        Ok(a) => {
            t.eq("7.1 accept", (a.code, a.id, a.int(radius::A_SERVICE_TYPE), a.int(radius::A_LOGIN_SERVICE), a.addr(radius::A_LOGIN_IP_HOST)), (radius::ACCESS_ACCEPT, 0, Some(radius::SERVICE_LOGIN), Some(0), Some(Ipv4([192, 168, 1, 3]))));
            // a new answer written from the record gets the same authenticator
            let mut b = Packet { auth: [0; 16], ..a.clone() }.to_bytes();
            radius::set_response_auth(&mut b, &ra, SECRET, crypto::md5_parts);
            t.eq("7.1 answer written again", b, acc.clone());
        }
        Err(_) => t.check("7.1 accept parses", false),
    }
    let req2 = h(REQ_72);
    let acc2 = h(ACC_72);
    match (Packet::parse(&req2), Packet::parse(&acc2)) {
        (Ok(p), Ok(a)) => {
            t.eq("7.2 request", (p.id, p.text(radius::A_USER_NAME), p.attr(radius::A_CHAP_PASSWORD).map(|c| c.value.len()), p.int(radius::A_SERVICE_TYPE), p.int(radius::A_FRAMED_PROTOCOL)), (1, Some(String::from("flopsy")), Some(17), Some(radius::SERVICE_FRAMED), Some(1)));
            t.eq("7.2 accept", (a.int(radius::A_FRAMED_MTU), a.addr(radius::A_FRAMED_IP_ADDRESS), a.int(radius::A_FRAMED_COMPRESSION)), (Some(1500), Some(Ipv4([255, 255, 255, 254])), Some(1)));
            t.check("7.2 response authenticator", radius::check_response_auth(&acc2, &p.auth, SECRET, crypto::md5_parts));
            t.eq("7.2 round trips", (p.to_bytes(), a.to_bytes()), (req2.clone(), acc2.clone()));
        }
        _ => t.check("7.2 parses", false),
    }
}

/// User-Password hidden and revealed for every length, the 16-byte blocks, at most 128 bytes
fn passwords(t: &mut T) {
    let auth = a16(&h("0f403f9473978057bd83d5cb98f4227a"));
    let mut ok = true;
    for n in 0..=128usize {
        let pw: Vec<u8> = (0..n).map(|i| b'a' + (i % 26) as u8).collect();
        let hid = radius::hide_password(&pw, b"s3cret", &auth, crypto::md5_parts);
        let len_ok = hid.len() == n.max(1).div_ceil(16) * 16;
        let back = radius::reveal_password(&hid, b"s3cret", &auth, crypto::md5_parts);
        ok &= len_ok && back == Some(pw.clone());
    }
    t.check("hide and reveal 0 .. 128 bytes", ok);
    let long: Vec<u8> = (0..140).map(|i| i as u8 | 1).collect();
    let hid = radius::hide_password(&long, b"s3cret", &auth, crypto::md5_parts);
    t.eq("at most 128 bytes", (hid.len(), radius::reveal_password(&hid, b"s3cret", &auth, crypto::md5_parts)), (128, Some(long[..128].to_vec())));
    let other = radius::reveal_password(&radius::hide_password(b"geheim", b"right", &auth, crypto::md5_parts), b"wrong", &auth, crypto::md5_parts);
    t.check("another secret reveals something else", other.is_some_and(|p| p != b"geheim"));
    t.eq("no multiple of 16", (radius::reveal_password(&[1; 15], b"s", &auth, crypto::md5_parts), radius::reveal_password(&[], b"s", &auth, crypto::md5_parts), radius::reveal_password(&[1; 144], b"s", &auth, crypto::md5_parts)), (None, None, None));
    // the second block depends on the first block of the result (the chain of 5.2)
    let two = radius::hide_password(b"0123456789abcdefXY", b"k", &auth, crypto::md5_parts);
    let b2 = crypto::md5_parts(&[b"k", &two[..16]]);
    t.eq("chained block", two[16] ^ b2[0], b'X');
}

/// RFC 2866 3: the Request Authenticator of an Accounting-Request, its answer
fn accounting(t: &mut T) {
    let mut p = Packet::new(radius::ACCOUNTING_REQUEST, 7, [0; 16]);
    p.push(Attr::int(radius::A_ACCT_STATUS_TYPE, radius::ACCT_START)).push(Attr::text(radius::A_ACCT_SESSION_ID, "0000002A")).push(Attr::text(radius::A_USER_NAME, "admin")).push(Attr::addr(radius::A_NAS_IP_ADDRESS, Ipv4([10, 0, 0, 1])));
    let mut b = p.to_bytes();
    radius::set_accounting_auth(&mut b, b"secret", crypto::md5_parts);
    t.eq("accounting authenticator", b[4..20].to_vec(), crypto::md5_parts(&[&b[..4], &[0u8; 16], &b[20..], b"secret"]).to_vec());
    t.check("accounting checks", radius::check_accounting_auth(&b, b"secret", crypto::md5_parts));
    t.check("accounting: wrong secret", !radius::check_accounting_auth(&b, b"secreT", crypto::md5_parts));
    let mut bad = b.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    t.check("accounting: changed on the way", !radius::check_accounting_auth(&bad, b"secret", crypto::md5_parts));
    let req_auth = a16(&b[4..20]);
    let mut resp = Packet::new(radius::ACCOUNTING_RESPONSE, 7, [0; 16]).to_bytes();
    radius::set_response_auth(&mut resp, &req_auth, b"secret", crypto::md5_parts);
    t.check("accounting response", resp.len() == 20 && radius::check_response_auth(&resp, &req_auth, b"secret", crypto::md5_parts));
    match Packet::parse(&b) {
        Ok(q) => t.eq("accounting attributes", (q.int(radius::A_ACCT_STATUS_TYPE), q.text(radius::A_ACCT_SESSION_ID)), (Some(radius::ACCT_START), Some(String::from("0000002A")))),
        Err(_) => t.check("accounting parses", false),
    }
}

/// RFC 3579 3.2: the Message-Authenticator of a request (over its own authenticator) and of an
/// answer (over the request's), computed before the Response Authenticator
fn message_auth(t: &mut T) {
    let ra = a16(&h("a1a2a3a4a5a6a7a8a9aaabacadaeafb0"));
    let mut p = Packet::new(radius::ACCESS_REQUEST, 3, ra);
    p.push(Attr::text(radius::A_USER_NAME, "admin")).push(Attr::bytes(radius::A_USER_PASSWORD, &radius::hide_password(b"geheim", b"k3y", &ra, crypto::md5_parts))).push(Attr::message_auth());
    let mut b = p.to_bytes();
    t.check("set in a request", radius::set_message_auth(&mut b, None, b"k3y", crypto::hmac_md5));
    // the value: HMAC-MD5 over the packet with its field as zeros
    let mut z = b.clone();
    let at = z.len() - 16;
    for x in &mut z[at..] {
        *x = 0;
    }
    t.eq("request value", b[at..].to_vec(), crypto::hmac_md5(b"k3y", &z).to_vec());
    t.eq("request checks", radius::check_message_auth(&b, None, b"k3y", crypto::hmac_md5), Some(true));
    t.eq("request: wrong secret", radius::check_message_auth(&b, None, b"key", crypto::hmac_md5), Some(false));
    let plain = Packet::new(radius::ACCESS_REQUEST, 4, ra).to_bytes();
    t.eq("none there", radius::check_message_auth(&plain, None, b"k3y", crypto::hmac_md5), None);
    let mut plain2 = plain.clone();
    t.check("none to set", !radius::set_message_auth(&mut plain2, None, b"k3y", crypto::hmac_md5));
    // an answer: the request's authenticator in the field, then the Response Authenticator
    let mut a = Packet::new(radius::ACCESS_ACCEPT, 3, [0; 16]);
    a.push(Attr::int(radius::A_SERVICE_TYPE, radius::SERVICE_ADMINISTRATIVE)).push(Attr::message_auth());
    let mut ab = a.to_bytes();
    t.check("set in an answer", radius::set_message_auth(&mut ab, Some(&ra), b"k3y", crypto::hmac_md5));
    radius::set_response_auth(&mut ab, &ra, b"k3y", crypto::md5_parts);
    t.eq("answer checks", (radius::check_message_auth(&ab, Some(&ra), b"k3y", crypto::hmac_md5), radius::check_response_auth(&ab, &ra, b"k3y", crypto::md5_parts)), (Some(true), true));
    t.eq("answer to another request", radius::check_message_auth(&ab, Some(&[0; 16]), b"k3y", crypto::hmac_md5), Some(false));
}

/// attributes as records: names, kinds, values, Vendor-Specific bytes, long values cut
fn records(t: &mut T) {
    t.eq("names", (radius::attr_name(1), radius::attr_name(26), radius::attr_name(49), radius::attr_name(80), radius::attr_name(200)), (String::from("User-Name"), String::from("Vendor-Specific"), String::from("Acct-Terminate-Cause"), String::from("Message-Authenticator"), String::from("Attribute 200")));
    t.eq("codes", (radius::code_name(1), radius::code_name(5), radius::code_name(11), radius::code_name(99)), ("Access-Request", "Accounting-Response", "Access-Challenge", "Unknown"));
    t.eq("values", (radius::value_name(6, 6), radius::value_name(61, 5), radius::value_name(40, 2), radius::value_name(49, 4), radius::value_name(6, 99)), (Some("Administrative-User"), Some("Virtual"), Some("Stop"), Some("Idle-Timeout"), None));
    let v = Attr::vsa(4242, b"level=15");
    t.eq("vendor-specific", (v.typ, v.vendor().map(|(n, d)| (n, d.to_vec()))), (radius::A_VENDOR_SPECIFIC, Some((4242, b"level=15".to_vec()))));
    let mut p = Packet::new(radius::ACCESS_ACCEPT, 9, [7; 16]);
    p.push(v.clone()).push(Attr::text(radius::A_REPLY_MESSAGE, "Hallo"));
    let back = Packet::parse(&p.to_bytes());
    t.eq("vendor-specific round trip", back.map(|q| q.attrs), Ok(alloc::vec![v, Attr::text(radius::A_REPLY_MESSAGE, "Hallo")]));
    let long = Attr::bytes(radius::A_STATE, &[5u8; 300]);
    t.eq("values cut at 253 bytes", long.value.len(), 253);
    t.eq("text of a value", (radius::value_text(2, &[0; 16]), radius::value_text(6, &[0, 0, 0, 6]), radius::value_text(4, &[10, 0, 0, 1]), radius::value_text(1, b"nemo")), (String::from("(hidden, 16 bytes)"), String::from("Administrative-User (6)"), String::from("10.0.0.1"), String::from("\"nemo\"")));
}

/// what a receiver discards (RFC 2865 3)
fn refused(t: &mut T) {
    let req = h(REQ_71);
    t.eq("short", RadiusView::parse(&req[..19]).err(), Some(Bad::Short));
    let mut l = req.clone();
    l[3] = 19;
    t.eq("length below 20", RadiusView::parse(&l).err(), Some(Bad::Length(19)));
    let mut l = req.clone();
    l[2] = 0x10;
    l[3] = 0x01;
    t.eq("length above 4096", RadiusView::parse(&l).err(), Some(Bad::Length(4097)));
    t.eq("shorter than its length", RadiusView::parse(&req[..50]).err(), Some(Bad::Short));
    let mut a = req.clone();
    a[21] = 1;
    t.eq("attribute length 1", RadiusView::parse(&a).err(), Some(Bad::Attr(20)));
    let mut a = req.clone();
    a[55 - 4] = 7;
    t.eq("attribute past the end", RadiusView::parse(&a).err(), Some(Bad::Attr(50)));
    let mut padded = req.clone();
    padded.extend_from_slice(&[0xEE; 6]);
    t.eq("padding after Length", RadiusView::parse(&padded).map(|v| (v.bytes().len(), v.attrs().len())), Ok((56, 4)));
}

/// RADIUS in frames: the summary, the layer with its fields, the old ports, a broken packet
fn radius_frames(t: &mut T) {
    let f = udp_frame(1645, 1812, &h(REQ_71));
    t.eq("info", decode::info(&f), (Proto::Radius, String::from("Access-Request id=0 (nemo)")));
    t.check("mask", decode::proto_mask(&f) & decode::P_RADIUS != 0);
    let ls = decode::layers(&f);
    let l = ls.last();
    t.eq("layer title", l.map(|l| l.title.clone()), Some(String::from("RADIUS Protocol, Access-Request (id 0)")));
    let fields: Vec<String> = l.map(|l| l.fields.iter().map(|x| x.value.clone()).collect()).unwrap_or_default();
    t.check("password stays hidden", fields.iter().any(|v| v == "User-Password (2): (hidden, 16 bytes)") && !fields.iter().any(|v| v.contains("arctangent")));
    t.check("attributes by name", fields.iter().any(|v| v == "User-Name (1): \"nemo\"") && fields.iter().any(|v| v == "NAS-IP-Address (4): 192.168.1.16") && fields.iter().any(|v| v == "NAS-Port (5): 3"));
    t.check("no problem", l.is_some_and(|l| l.problems.is_empty()));
    let acc = udp_frame(1812, 1645, &h(ACC_71));
    t.eq("info of the answer", decode::info(&acc).1, String::from("Access-Accept id=0"));
    let mut p = Packet::new(radius::ACCOUNTING_REQUEST, 12, [0; 16]);
    p.push(Attr::int(radius::A_ACCT_STATUS_TYPE, radius::ACCT_STOP)).push(Attr::text(radius::A_USER_NAME, "anna"));
    t.eq("accounting on 1646", decode::info(&udp_frame(1646, 1813, &p.to_bytes())), (Proto::Radius, String::from("Accounting-Request id=12 (Stop, anna)")));
    let mut bad = h(REQ_71);
    bad[3] = 0x50;
    let bl = decode::layers(&udp_frame(1645, 1812, &bad));
    t.check("broken packet: a problem", bl.last().is_some_and(|l| l.proto == Proto::Radius && !l.problems.is_empty()));
}

/// IAC escaping and the line ends of the network virtual terminal
fn telnet_bytes(t: &mut T) {
    t.eq("escape", telnet::escape(&[1, 255, 2, 255]), alloc::vec![1, 255, 255, 2, 255, 255]);
    t.eq("nvt line ends", telnet::to_nvt(b"a\nb\rc\r\nd"), b"a\r\nb\r\0c\r\nd".to_vec());
    t.eq("nvt iac", telnet::to_nvt(&[b'x', 255]), alloc::vec![b'x', 255, 255]);
    t.eq("commands", (telnet::cmd(telnet::IP), telnet::opt(telnet::WILL, telnet::OPT_ECHO)), ([255, 244], [255, 251, 1]));
    t.eq("names", (telnet::cmd_name(telnet::DO), telnet::cmd_name(telnet::AYT), telnet::opt_name(telnet::OPT_SGA), telnet::opt_name(99)), ("Do", "Are You There", String::from("Suppress Go Ahead"), String::from("Option 99")));
}

/// the reader: sequences split between pieces, escaped IAC, subnegotiation, NVT line ends
fn telnet_reader(t: &mut T) {
    let mut r = Reader::new(false);
    let mut items = r.push(&[b'h', b'i', telnet::IAC]);
    items.extend(r.push(&[telnet::WILL]));
    items.extend(r.push(&[telnet::OPT_ECHO, b'!', telnet::IAC, telnet::IAC]));
    t.eq("split sequence", items, alloc::vec![Item::Data(b"hi".to_vec()), Item::Opt { verb: telnet::WILL, opt: telnet::OPT_ECHO }, Item::Data(alloc::vec![b'!', 255])]);
    let mut r = Reader::new(false);
    let mut items = r.push(&[255, 250, 24, 0, b'x', 255]);
    items.extend(r.push(&[255, b'y', 255, 240, 255, 244]));
    t.eq("subnegotiation", items, alloc::vec![Item::Sub { opt: 24, data: alloc::vec![0, b'x', 255, b'y'] }, Item::Cmd(telnet::IP)]);
    let mut r = Reader::new(true);
    let mut d = Vec::new();
    for piece in [&b"show run\r"[..], b"\nnext\r\0x\r", b"\n"] {
        for it in r.push(piece) {
            if let Item::Data(x) = it {
                d.extend(x);
            }
        }
    }
    t.eq("nvt folded across pieces", String::from_utf8_lossy(&d).into_owned(), String::from("show run\nnext\rx\n"));
    let p = telnet::parse(&[255, 253, 1, b'h', b'i', 255, 255, 255, 244, 255]);
    t.eq(
        "parse with offsets",
        p,
        alloc::vec![(0, 3, Item::Opt { verb: telnet::DO, opt: 1 }), (3, 4, Item::Data(alloc::vec![b'h', b'i', 255])), (7, 2, Item::Cmd(telnet::IP)), (9, 1, Item::Partial(alloc::vec![255]))]
    );
    t.eq("parse a cut subnegotiation", telnet::parse(&[255, 250, 31, 0, 80]), alloc::vec![(0, 5, Item::Partial(alloc::vec![255, 250, 31, 0, 80]))]);
}

/// option negotiation (RFC 855, 857, 858, the Q method of RFC 1143)
fn telnet_options(t: &mut T) {
    let mut srv = Options::new(&[telnet::OPT_ECHO, telnet::OPT_SGA], &[telnet::OPT_SGA]);
    let mut cli = Options::new(&[telnet::OPT_SGA], &[telnet::OPT_ECHO, telnet::OPT_SGA]);
    let w = srv.enable_us(telnet::OPT_SGA);
    t.eq("offer", w, Some([255, 251, 3]));
    let a = cli.received(telnet::WILL, telnet::OPT_SGA);
    t.eq("accepted", a, Some([255, 253, 3]));
    t.eq("no answer to the answer", srv.received(telnet::DO, telnet::OPT_SGA), None);
    t.check("in effect", srv.us(telnet::OPT_SGA) && cli.him(telnet::OPT_SGA));
    t.eq("a repeat changes nothing", cli.received(telnet::WILL, telnet::OPT_SGA), None);
    t.eq("refused for us", cli.received(telnet::DO, telnet::OPT_TTYPE), Some([255, 252, 24]));
    t.eq("refused for him", srv.received(telnet::WILL, telnet::OPT_NAWS), Some([255, 254, 31]));
    // ECHO on and off again (as a server hides a password)
    let on = srv.enable_us(telnet::OPT_ECHO);
    let ack = on.and_then(|o| cli.received(o[1], o[2]));
    let _ = ack.and_then(|o| srv.received(o[1], o[2]));
    t.check("echo on", srv.us(telnet::OPT_ECHO) && cli.him(telnet::OPT_ECHO));
    let off = srv.disable_us(telnet::OPT_ECHO);
    t.eq("echo off", off, Some([255, 252, 1]));
    let ack = off.and_then(|o| cli.received(o[1], o[2]));
    t.eq("client agrees", ack, Some([255, 254, 1]));
    t.eq("server is done", ack.and_then(|o| srv.received(o[1], o[2])), None);
    t.check("echo is off", !srv.us(telnet::OPT_ECHO) && !cli.him(telnet::OPT_ECHO));
    // two sides that keep answering whatever comes: the exchange ends
    let mut a = Options::new(&[1, 3], &[1, 3]);
    let mut b = Options::new(&[3], &[1]);
    let mut q: Vec<(bool, [u8; 3])> = Vec::new();
    for o in [a.enable_us(1), a.enable_us(3), a.enable_him(1), a.enable_him(3), a.enable_him(24)].into_iter().flatten() {
        q.push((true, o));
    }
    let mut n = 0;
    while let Some((from_a, m)) = q.pop() {
        n += 1;
        if n > 100 {
            break;
        }
        let ans = if from_a { b.received(m[1], m[2]) } else { a.received(m[1], m[2]) };
        if let Some(r) = ans {
            q.push((!from_a, r));
        }
    }
    t.check("no circle", n <= 10);
    t.check("what both agree on", a.us(1) && !a.us(3) && a.him(3) && !a.him(1) && !a.him(24) && b.us(3) && b.him(1) && !b.us(1) && !b.him(3));
}

/// Telnet in frames: commands and text in the summary and the layer
fn telnet_frames(t: &mut T) {
    let mut d = alloc::vec![255, 251, 1];
    d.extend_from_slice(b"Password: ");
    let f = tcp_frame(23, 51000, &d);
    t.eq("info", decode::info(&f), (Proto::Telnet, String::from("Will Echo, \"Password: \"")));
    t.check("mask", decode::proto_mask(&f) & decode::P_TELNET != 0);
    let ls = decode::layers(&f);
    let l = ls.last();
    let fields: Vec<(&str, String)> = l.map(|l| l.fields.iter().map(|x| (x.name, x.value.clone())).collect()).unwrap_or_default();
    t.eq("layer fields", fields, alloc::vec![("Command", String::from("Will Echo")), ("Data", String::from("Password: "))]);
    let line = tcp_frame(51000, 23, b"show ip interface brief\r\n");
    t.eq("typed line", decode::info(&line).1, String::from("\"show ip interface brief\\r\\n\""));
    let cut = tcp_frame(51000, 23, &[b'x', 255, 253]);
    t.check("cut command: a problem", decode::layers(&cut).last().is_some_and(|l| l.proto == Proto::Telnet && !l.problems.is_empty()));
}
