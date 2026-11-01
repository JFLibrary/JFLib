// snmpv3t -- checks of netpkt::snmpv3 (part of the group sshcore): the key localisation of RFC 3414
// A.3 for MD5 and SHA, the same for SHA-2 (RFC 7860) against hashlib, the tags against Python's
// hmac, AES-128-CFB of RFC 3826 against openssl (an IV of boots, time and salt, a last piece
// shorter than a block), messages there and back, sign and check, parts the parser refuses, levels,
// engine IDs and the dissector of a plain and an encrypted message

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::ber::Oid;
use netpkt::decode;
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::snmp::{Kind, Pdu, VarBind, PORT};
use netpkt::snmpv3::{self as v3, AuthProto, Level, PrivProto, Scoped, ScopedPdu, Usm, V3};
use netpkt::udp::UdpHdr;
use netpkt::{Ipv4, Mac};

use crate::T;

fn h(s: &str) -> Vec<u8> {
    crypto::unhex(&s.replace(' ', "")).expect("hex")
}

const ENGINE: &str = "000000000000000000000002";

fn keys(t: &mut T) {
    let e = h(ENGINE);
    let cases = [
        (AuthProto::Md5, "9faf3283884e92834ebc9847d8edd963", "526f5eed9fcce26f8964c2930787d82b", "e9950435bdcce0350b4de17a"),
        (AuthProto::Sha1, "9fb5cc0381497b3793528939ff788d5d79145211", "6695febc9288e36282235fc7151f128497b38f3f", "83786e0d5de03ff3fe8eb804"),
        (AuthProto::Sha256, "ab51014d1e077f6017df2b12bee5f5aa72993177e9bb569c4dff5a4ca0b4afac", "8982e0e549e866db361a6b625d84cccc11162d453ee8ce3a6445c2d6776f0f8b", "4f9c72cdaa9f82c3aef49fa897224cd6cf49e967d4fb3e6d"),
        (AuthProto::Sha384, "e06eccdf2c68a06ed034723c9c26e0db3b669e1e2efed49150b55377a2e98f383c86fb836857444654b287c93f51ff64", "3b298f16164a11184279d5432bf169e2d2a48307de02b3d3f7e2b4f36eb6f0455a53689a3937eea07319a633d2ccba78", "4ef994a784f996fe3196fa6bc4b134ee886cf80330162e11004a5232039d0cb6"),
        (AuthProto::Sha512, "7e4396de5aadc77be853819b98c9406265b3a9c37cc3176569847a4e4f6fba63dd3a73d04924d31a63f95a601f9385af6be4ed1b37f87d040f7c6ed6f8d38a91", "22a5a36cedfcc085807a128d7bc6c2382167ad6c0dbc5fdff856740f3d84c099ad1ea87a8db096714d9788bd544047c9021e4229ce27e4c0a69250adfcffbb0b", "e4a254427842478e5d42afb95e1484669777cd48c9f2a5a4eb18c3b0749bd4ca363dee5e839a25859c07d704d9773b14"),
    ];
    let msg: Vec<u8> = (0..64).collect();
    for (a, ku, kul, tag) in cases {
        let k = v3::password_to_key(a, b"maplesyrup");
        t.eq(&alloc::format!("{} password to key", a.name()), k.clone(), h(ku));
        let l = v3::localize(a, &k, &e);
        t.eq(&alloc::format!("{} localized", a.name()), l.clone(), h(kul));
        t.eq(&alloc::format!("{} tag", a.name()), v3::tag(a, &l, &msg), h(tag));
    }
    t.check("no password, no key", v3::password_to_key(AuthProto::Sha1, b"").is_empty());
    t.eq("a privacy key is 16 bytes", v3::priv_key(&h("6695febc9288e36282235fc7151f128497b38f3f")), h("6695febc9288e36282235fc7151f1284"));
}

fn cfb(t: &mut T) {
    let key = h("526f5eed9fcce26f8964c2930787d82b");
    let salt = h("0102030405060708");
    let plain = b"scoped pdu of 37 bytes, not a block..";
    let want = h("7b26dea7b5f6b2b8ea343badb4456a193c10721d9848f5b318132f39acdb383e7390cf34d3");
    t.eq("AES-128-CFB", v3::encrypt(&key, 7, 1234, &salt, plain), Some(want.clone()));
    t.eq("and back", v3::decrypt(&key, 7, 1234, &salt, &want), Some(Vec::from(&plain[..])));
    t.check("other boots, other bytes", v3::decrypt(&key, 8, 1234, &salt, &want) != Some(Vec::from(&plain[..])));
    t.check("a salt of 8 bytes", v3::encrypt(&key, 7, 1234, &salt[..4], plain).is_none());
    t.check("a key of 16 bytes", v3::encrypt(&key[..8], 7, 1234, &salt, plain).is_none());
}

fn get(id: i32) -> Pdu {
    Pdu::new(Kind::Get, id, alloc::vec![VarBind::null(Oid::new(&[1, 3, 6, 1, 2, 1, 1, 5, 0]))])
}

fn messages(t: &mut T) {
    let e = h("80007ed903020000000001");
    let plain = V3 {
        id: 77,
        max_size: 65507,
        flags: v3::F_REPORTABLE,
        model: 3,
        usm: Usm { engine_id: e.clone(), boots: 3, time: 99, user: Vec::from(&b"admin"[..]), auth: Vec::new(), privacy: Vec::new() },
        scoped: Scoped::Plain(ScopedPdu { context_engine: e.clone(), context_name: Vec::new(), pdu: get(5) }),
    };
    let b = plain.to_bytes();
    t.eq("a plain message back", V3::parse(&b), Ok(plain.clone()));
    t.eq("level and reportable", (plain.level(), plain.reportable()), (Some(Level::NoAuthNoPriv), true));
    t.eq("netpkt::snmp keeps refusing it", netpkt::snmp::Message::parse(&b).err(), Some(netpkt::snmp::Bad::Version(3)));
    let mut bad = b.clone();
    bad[4] = 1;
    t.check("another version", V3::parse(&bad).is_err());
    t.check("cut off", V3::parse(&b[..b.len() - 2]).is_err());
    let mut trailing = b.clone();
    trailing.push(0);
    t.check("bytes after it", V3::parse(&trailing).is_err());
    // signed: the zeros become the tag, it checks; a changed byte does not
    let key = v3::localize(AuthProto::Sha1, &v3::password_to_key(AuthProto::Sha1, b"maplesyrup"), &e);
    let mut s = plain.clone();
    s.flags |= v3::F_AUTH;
    s.usm.auth = alloc::vec![0u8; 12];
    let mut sb = s.to_bytes();
    t.check("sign", v3::sign(&mut sb, AuthProto::Sha1, &key));
    t.check("the tag went in", V3::parse(&sb).is_ok_and(|m| m.usm.auth != alloc::vec![0u8; 12] && m.usm.auth.len() == 12));
    t.check("check", v3::check(&sb, AuthProto::Sha1, &key));
    t.check("another key", !v3::check(&sb, AuthProto::Sha1, &v3::localize(AuthProto::Sha1, &v3::password_to_key(AuthProto::Sha1, b"maplesirup"), &e)));
    t.check("another protocol", !v3::check(&sb, AuthProto::Sha256, &key));
    let n = sb.len();
    let mut changed = sb.clone();
    changed[n - 1] ^= 1;
    t.check("a changed byte", !v3::check(&changed, AuthProto::Sha1, &key));
    // encrypted
    let salt = h("0102030405060708");
    let sp = ScopedPdu { context_engine: e.clone(), context_name: Vec::new(), pdu: get(6) };
    let c = v3::encrypt(&key[..16], 3, 99, &salt, &sp.to_bytes()).unwrap_or_default();
    let mut x = s.clone();
    x.flags = v3::F_AUTH | v3::F_PRIV;
    x.usm.privacy = salt.clone();
    x.scoped = Scoped::Encrypted(c.clone());
    let xb = x.to_bytes();
    let back = V3::parse(&xb);
    t.check("an encrypted message back", back.as_ref().is_ok_and(|m| m.plain().is_none() && m.level() == Some(Level::AuthPriv)));
    let opened = v3::decrypt(&key[..16], 3, 99, &salt, &c).and_then(|p| ScopedPdu::parse(&p).ok());
    t.eq("its PDU", opened, Some(sp.clone()));
    // levels, protocols, engine IDs, reports
    t.eq("level names", (Level::parse("authPriv"), Level::parse("noauth"), Level::parse("x")), (Some(Level::AuthPriv), Some(Level::NoAuthNoPriv), None));
    t.eq("flags", (Level::of_flags(0), Level::of_flags(1), Level::of_flags(3), Level::of_flags(2), Level::of_flags(7)), (Some(Level::NoAuthNoPriv), Some(Level::AuthNoPriv), Some(Level::AuthPriv), None, Some(Level::AuthPriv)));
    t.eq("protocols", (AuthProto::parse("SHA"), AuthProto::parse("sha-256"), AuthProto::parse("DES"), PrivProto::parse("AES"), PrivProto::parse("DES")), (Some(AuthProto::Sha1), Some(AuthProto::Sha256), None, Some(PrivProto::Aes128), None));
    t.eq("engine ID of a MAC", v3::engine_id_mac(Mac([2, 0, 0, 0, 0, 1])), h("80007ed903020000000001"));
    t.eq("engine ID text", v3::engine_id_text(&e), String::from("80007ed903020000000001"));
    t.eq("engine ID read", v3::parse_engine_id("80:00:7e:d9:03"), Some(h("80007ed903")));
    t.check("too short", v3::parse_engine_id("8000").is_none());
    t.eq("report names", (v3::report_name(&v3::USM_STATS_WRONG_DIGESTS), v3::report_name(&v3::UNKNOWN_CONTEXTS), v3::report_name(&[1, 2])), ("usmStatsWrongDigests", "snmpUnknownContexts", "unknown counter"));
    // the dissector
    let f = frame(&b);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Snmp).expect("SNMP layer");
    let field = |n: &str| l.fields.iter().find(|x| x.name == n).map(|x| x.value.clone());
    t.eq("msgFlags", field("msgFlags"), Some(String::from("0x04 (reportable)")));
    t.eq("the user", field("msgUserName"), Some(String::from("admin")));
    t.eq("msgID", field("msgID"), Some(String::from("77")));
    t.eq("the model", field("msgSecurityModel"), Some(String::from("USM (3)")));
    t.eq("the PDU", field("PDU"), Some(String::from("GetRequest (0xa0)")));
    t.check("no problems", l.problems.is_empty());
    t.eq("its summary", decode::info(&f).1, String::from("SNMPv3 GetRequest 5: user admin (noAuthNoPriv)"));
    let f = frame(&xb);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Snmp).expect("SNMP layer");
    let field = |n: &str| l.fields.iter().find(|x| x.name == n).map(|x| x.value.clone());
    t.eq("encrypted: its flags", field("msgFlags"), Some(String::from("0x03 (auth, priv)")));
    t.eq("encrypted: the PDU", field("Encrypted scopedPDU"), Some(String::from("verschlüsselt")));
    t.eq("encrypted: no PDU field", field("PDU"), None);
    t.eq("encrypted: its summary", decode::info(&f).1, String::from("SNMPv3 verschlüsselt: user admin (authPriv)"));
}

fn frame(data: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]));
    let mut p = Pkt::new(HEADROOM, data);
    UdpHdr::new(40000, PORT).push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 1]), src: Mac([2, 0, 0, 0, 0, 2]), ethertype: ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

pub fn run(t: &mut T) {
    keys(t);
    cfb(t);
    messages(t);
}
