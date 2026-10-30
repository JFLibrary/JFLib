// snmpt -- checks of netpkt::ber, netpkt::snmp, netpkt::mib and the SNMP dissector: BER of every
// type byte for byte (X.690 and RFC 2578: shortest integers, unsigned values with their leading
// zero, base-128 subidentifiers, long lengths), what the reader refuses (indefinite and too long
// lengths, inner lengths beyond their element, OIDs not in their shortest form or unended,
// integers without contents or too long, counters out of range, NULL with contents, trailing
// bytes), a GetRequest of SNMPv1 against its well-known 40 bytes, round trips of every PDU and of
// the v1 Trap, what version 1 does not have, the error mapping of RFC 3584, the OIDs of
// notifications, the MIB's order, names and values, and whole frames with their layers and
// summaries.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use netpkt::ber::{self, Bad, Oid, Reader, Value, Writer};
use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::mib;
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::snmp::{self, Body, Kind, Message, Pdu, TrapV1, VarBind, Version};
use netpkt::udp::UdpHdr;
use netpkt::{Ipv4, Mac};

use crate::T;

fn s(x: &str) -> String {
    String::from(x)
}

fn oid(x: &str) -> Oid {
    Oid::parse(x).unwrap_or_default()
}

/// an SNMP message in a frame from 10.0.0.50:sport to 10.0.0.1:dport
fn frame(sport: u16, dport: u16, msg: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 50]), Ipv4([10, 0, 0, 1]));
    let mut p = Pkt::new(HEADROOM, msg);
    UdpHdr::new(sport, dport).push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 1]), src: Mac([2, 0, 0, 0, 0, 50]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

/// one value as a whole element
fn enc(v: &Value) -> Vec<u8> {
    let mut w = Writer::new();
    w.value(v);
    w.into_vec()
}

/// one element read back as a value
fn dec(b: &[u8]) -> Result<Value, Bad> {
    let mut r = Reader::new(b);
    let v = r.value()?;
    r.done()?;
    Ok(v)
}

pub fn run(t: &mut T) {
    encodings(t);
    refused(t);
    messages(t);
    versions(t);
    notifications(t);
    names(t);
    frames(t);
}

fn encodings(t: &mut T) {
    // INTEGER in its shortest two's complement (X.690 8.3)
    for (v, want) in [(0i64, &[0x02, 0x01, 0x00][..]), (127, &[0x02, 0x01, 0x7F]), (128, &[0x02, 0x02, 0x00, 0x80]), (256, &[0x02, 0x02, 0x01, 0x00]), (-1, &[0x02, 0x01, 0xFF]), (-128, &[0x02, 0x01, 0x80]), (-129, &[0x02, 0x02, 0xFF, 0x7F]), (2_147_483_647, &[0x02, 0x04, 0x7F, 0xFF, 0xFF, 0xFF]), (-2_147_483_648, &[0x02, 0x04, 0x80, 0x00, 0x00, 0x00])] {
        t.eq(&alloc::format!("INTEGER {}", v), enc(&Value::Int(v)), Vec::from(want));
        t.eq(&alloc::format!("INTEGER {} back", v), dec(want), Ok(Value::Int(v)));
    }
    // the unsigned types take a leading zero before a high bit (RFC 2578 7.1.6 .. 7.1.10)
    t.eq("Counter32 max", enc(&Value::Counter32(u32::MAX)), alloc::vec![0x41, 0x05, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]);
    t.eq("Gauge32 0", enc(&Value::Gauge32(0)), alloc::vec![0x42, 0x01, 0x00]);
    t.eq("Gauge32 1 Gbit/s", enc(&Value::Gauge32(1_000_000_000)), alloc::vec![0x42, 0x04, 0x3B, 0x9A, 0xCA, 0x00]);
    t.eq("TimeTicks 500", enc(&Value::TimeTicks(500)), alloc::vec![0x43, 0x02, 0x01, 0xF4]);
    t.eq("Counter64 max", enc(&Value::Counter64(u64::MAX)), alloc::vec![0x46, 0x09, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    t.eq("IpAddress", enc(&Value::Ip(Ipv4([10, 0, 0, 1]))), alloc::vec![0x40, 0x04, 10, 0, 0, 1]);
    t.eq("NULL", enc(&Value::Null), alloc::vec![0x05, 0x00]);
    t.eq("OCTET STRING", enc(&Value::text("R1")), alloc::vec![0x04, 0x02, b'R', b'1']);
    t.eq("exceptions", [enc(&Value::NoSuchObject), enc(&Value::NoSuchInstance), enc(&Value::EndOfMibView)], [alloc::vec![0x80, 0x00], alloc::vec![0x81, 0x00], alloc::vec![0x82, 0x00]]);
    // OBJECT IDENTIFIER: 40 X + Y, then base 128 (X.690 8.19; its example {2 100 3})
    t.eq("OID sysName.0", oid("1.3.6.1.2.1.1.5.0").encode(), alloc::vec![0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x05, 0x00]);
    t.eq("OID of X.690", oid("2.100.3").encode(), alloc::vec![0x81, 0x34, 0x03]);
    t.eq("OID enterprise 32473", oid("1.3.6.1.4.1.32473.1.4").encode(), alloc::vec![0x2B, 0x06, 0x01, 0x04, 0x01, 0x81, 0xFD, 0x59, 0x01, 0x04]);
    t.eq("OID arc 2^32-1", Oid::decode(&oid("1.3.4294967295").encode()), Ok(oid("1.3.4294967295")));
    t.eq("OID back", dec(&[0x06, 0x03, 0x81, 0x34, 0x03]), Ok(Value::Oid(oid("2.100.3"))));
    t.eq("OID zeroDotZero", enc(&Value::Oid(oid("0.0"))), alloc::vec![0x06, 0x01, 0x00]);
    // lengths: the long form from 128 on, shortest when written, longer ones read (RFC 3417 8)
    let long = Value::Str(alloc::vec![b'a'; 200]);
    t.eq("length 200", enc(&long)[..3].to_vec(), alloc::vec![0x04, 0x81, 0xC8]);
    t.eq("length 300", ber::length_bytes(300), alloc::vec![0x82, 0x01, 0x2C]);
    t.eq("length 127", ber::length_bytes(127), alloc::vec![0x7F]);
    t.eq("a long form for 5", dec(&[0x04, 0x81, 0x05, b'h', b'e', b'l', b'l', b'o']), Ok(Value::text("hello")));
    t.eq("four length octets", dec(&[0x04, 0x84, 0, 0, 0, 1, b'x']), Ok(Value::text("x")));
    for v in [Value::Int(-77), Value::text("Serverraum"), Value::Null, Value::Oid(oid("1.3.6.1.6.3.1.1.5.3")), Value::Ip(Ipv4([255, 255, 255, 0])), Value::Counter32(7), Value::Gauge32(4_000_000_000), Value::TimeTicks(8_640_000), Value::Opaque(alloc::vec![1, 2]), Value::Counter64(1 << 40), Value::NoSuchInstance] {
        t.eq(&alloc::format!("round trip {}", v.type_name()), dec(&enc(&v)), Ok(v.clone()));
    }
    // the order of SNMP: a prefix comes before what it starts
    let mut list = alloc::vec![oid("1.3.6.1.2.1.2.2.1.10.1"), oid("1.3.6.1.2.1.2.2.1.2.10"), oid("1.3.6.1.2.1.2.2.1.2"), oid("1.3.6.1.2.1.2.2.1.2.2")];
    list.sort();
    t.eq("lexicographic", list.iter().map(|o| o.to_string()).collect::<Vec<_>>(), alloc::vec![s("1.3.6.1.2.1.2.2.1.2"), s("1.3.6.1.2.1.2.2.1.2.2"), s("1.3.6.1.2.1.2.2.1.2.10"), s("1.3.6.1.2.1.2.2.1.10.1")]);
    t.eq("dotted with a leading dot", Oid::parse(".1.3.6.1").map(|o| o.to_string()), Some(s("1.3.6.1")));
    t.eq("bad OIDs", [Oid::parse("1"), Oid::parse("3.1"), Oid::parse("1.40"), Oid::parse("1.3.x"), Oid::parse("1..3"), Oid::parse("1.3.99999999999")], [None, None, None, None, None, None]);
    t.check("starts with", oid("1.3.6.1.2.1.1.5.0").starts_with(&[1, 3, 6, 1, 2, 1, 1]) && !oid("1.3.6.1.2.1.10").starts_with(&[1, 3, 6, 1, 2, 1, 1]));
}

fn refused(t: &mut T) {
    t.eq("indefinite length", dec(&[0x04, 0x80, 0x00, 0x00]), Err(Bad::Indefinite));
    t.eq("five length octets", dec(&[0x04, 0x85, 0, 0, 0, 0, 1, b'x']), Err(Bad::Length));
    t.eq("beyond the buffer", dec(&[0x04, 0x05, b'a', b'b']), Err(Bad::Truncated));
    t.eq("no length", dec(&[0x04]), Err(Bad::Truncated));
    t.eq("long form cut", dec(&[0x04, 0x82, 0x01]), Err(Bad::Truncated));
    // an inner length beyond the element around it
    let mut r = Reader::new(&[0x30, 0x03, 0x04, 0x05, b'a', b'b', b'c']);
    let inner = r.seq(ber::T_SEQUENCE).and_then(|mut q| q.octets().map(Vec::from));
    t.eq("inner beyond outer", inner, Err(Bad::Truncated));
    t.eq("OID with 0x80", dec(&[0x06, 0x03, 0x2B, 0x80, 0x01]), Err(Bad::Oid));
    t.eq("OID unended", dec(&[0x06, 0x02, 0x2B, 0x86]), Err(Bad::Oid));
    t.eq("OID empty", dec(&[0x06, 0x00]), Err(Bad::Oid));
    t.eq("OID arc beyond 32 bits", dec(&[0x06, 0x07, 0x2B, 0x90, 0x80, 0x80, 0x80, 0x80, 0x00]), Err(Bad::Oid));
    t.eq("INTEGER empty", dec(&[0x02, 0x00]), Err(Bad::IntSize));
    t.eq("INTEGER of 9 octets", dec(&[0x02, 0x09, 0, 0, 0, 0, 0, 0, 0, 0, 1]), Err(Bad::IntSize));
    t.eq("Integer32 range", dec(&[0x02, 0x05, 0x00, 0x80, 0x00, 0x00, 0x00]), Err(Bad::Range));
    t.eq("Counter32 of 33 bits", dec(&[0x41, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]), Err(Bad::Range));
    t.eq("negative Gauge32", dec(&[0x42, 0x01, 0xFF]), Err(Bad::Range));
    t.eq("Counter64 of 9 octets without the zero", dec(&[0x46, 0x09, 1, 0, 0, 0, 0, 0, 0, 0, 0]), Err(Bad::IntSize));
    t.eq("NULL with contents", dec(&[0x05, 0x01, 0x00]), Err(Bad::Size));
    t.eq("IpAddress of 3", dec(&[0x40, 0x03, 10, 0, 0]), Err(Bad::Size));
    t.eq("noSuchObject with contents", dec(&[0x80, 0x01, 0x00]), Err(Bad::Size));
    t.eq("constructed OCTET STRING", dec(&[0x24, 0x03, 0x04, 0x01, b'a']), Err(Bad::Tag(0x24)));
    t.eq("multi-octet tag", dec(&[0x1F, 0x81, 0x01, 0x00]), Err(Bad::Tag(0x1F)));
    t.eq("trailing bytes", dec(&[0x05, 0x00, 0x00]), Err(Bad::Trailing));
}

/// the GetRequest of SNMPv1 for sysDescr.0 with community "public" and request id 1
const GET_V1: [u8; 40] = [
    0x30, 0x26, 0x02, 0x01, 0x00, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xA0, 0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0E, 0x30, 0x0C, 0x06, 0x08, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00,
];

fn messages(t: &mut T) {
    let get = Message::new(Version::V1, "public", Pdu::new(Kind::Get, 1, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.1.1.0"))]));
    t.eq("GetRequest v1 bytes", get.to_bytes(), Vec::from(&GET_V1[..]));
    t.eq("GetRequest v1 parsed", Message::parse(&GET_V1), Ok(get.clone()));
    t.eq("community text", get.community_text(), s("public"));
    // every PDU of 2c and the bulk fields
    let vbs = alloc::vec![VarBind::new(oid("1.3.6.1.2.1.1.5.0"), Value::text("R1")), VarBind::new(oid("1.3.6.1.2.1.1.3.0"), Value::TimeTicks(123_456))];
    for k in [Kind::Get, Kind::GetNext, Kind::Response, Kind::Set, Kind::Inform, Kind::TrapV2, Kind::Report] {
        let m = Message::new(Version::V2c, "private", Pdu::new(k, -5, vbs.clone()));
        t.eq(&alloc::format!("round trip {}", k.name()), Message::parse(&m.to_bytes()), Ok(m.clone()));
        t.eq(&alloc::format!("tag of {}", k.name()), Kind::of_tag(k.tag()), Some(k));
    }
    let bulk = Message::new(Version::V2c, "public", Pdu::bulk(77, 1, 10, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.1.3")), VarBind::null(oid("1.3.6.1.2.1.2.2.1.2"))]));
    let back = Message::parse(&bulk.to_bytes());
    t.eq("GetBulk round trip", back.clone(), Ok(bulk.clone()));
    t.eq("GetBulk fields", back.ok().and_then(|m| m.pdu().map(|p| (p.non_repeaters(), p.max_repetitions()))), Some((1, 10)));
    t.eq("negative bulk counts are 0", (Pdu::bulk(1, -3, -1, Vec::new()).non_repeaters(), Pdu::bulk(1, -3, -1, Vec::new()).max_repetitions()), (0, 0));
    // an error response
    let mut err = Pdu::new(Kind::Response, 9, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.1.9.0"))]);
    err.error_status = snmp::E_NO_SUCH_NAME;
    err.error_index = 1;
    let m = Message::new(Version::V1, "public", err.clone());
    t.eq("error round trip", Message::parse(&m.to_bytes()).ok().and_then(|x| x.pdu().cloned()), Some(err));
    // the v1 Trap: enterprise, agent address, generic, specific, time stamp
    let trap = Message {
        version: Version::V1,
        community: Vec::from(&b"public"[..]),
        body: Body::Trap(TrapV1 { enterprise: oid("1.3.6.1.4.1.32473.1.4"), agent_addr: Ipv4([10, 0, 0, 1]), generic: snmp::G_LINK_DOWN, specific: 0, timestamp: 500, varbinds: alloc::vec![VarBind::new(oid("1.3.6.1.2.1.2.2.1.1.2"), Value::Int(2))] }),
    };
    let tb = trap.to_bytes();
    t.eq("trap round trip", Message::parse(&tb), Ok(trap.clone()));
    t.eq("trap kind", trap.kind(), Kind::TrapV1);
    t.eq("trap start", tb[..4].to_vec(), alloc::vec![0x30, (tb.len() - 2) as u8, 0x02, 0x01]);
    // a message of 300 bytes: the long length form at the top
    let big = Message::new(Version::V2c, "public", Pdu::new(Kind::Response, 1, alloc::vec![VarBind::new(oid("1.3.6.1.2.1.1.1.0"), Value::Str(alloc::vec![b'x'; 300]))]));
    let bb = big.to_bytes();
    t.eq("long message", (bb[0], bb[1], Message::parse(&bb).is_ok()), (0x30, 0x82, true));
}

fn versions(t: &mut T) {
    let mut v3 = Vec::from(&GET_V1[..]);
    v3[4] = 3;
    t.eq("SNMPv3 is no message here", Message::parse(&v3), Err(snmp::Bad::Version(3)));
    let bulk_v1 = Message::new(Version::V2c, "public", Pdu::bulk(1, 0, 5, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.1"))]));
    let mut b = bulk_v1.to_bytes();
    b[4] = 0;
    t.eq("GetBulk in version 1", Message::parse(&b), Err(snmp::Bad::Pdu(0xA5)));
    let c64 = Message::new(Version::V2c, "public", Pdu::new(Kind::Response, 1, alloc::vec![VarBind::new(oid("1.3.6.1.2.1.31.1.1.1.6.1"), Value::Counter64(5))]));
    let mut b = c64.to_bytes();
    b[4] = 0;
    t.eq("Counter64 in version 1", Message::parse(&b), Err(snmp::Bad::V1Type(ber::T_COUNTER64)));
    let ex = Message::new(Version::V2c, "public", Pdu::new(Kind::Response, 1, alloc::vec![VarBind::new(oid("1.3.6.1.2.1.1.9.0"), Value::NoSuchObject)]));
    let mut b = ex.to_bytes();
    b[4] = 0;
    t.eq("an exception in version 1", Message::parse(&b), Err(snmp::Bad::V1Type(ber::T_NO_SUCH_OBJECT)));
    let mut trap2c = Vec::from(&GET_V1[..]);
    trap2c[4] = 1;
    trap2c[13] = 0xA4;
    t.eq("v1 Trap in 2c", Message::parse(&trap2c), Err(snmp::Bad::Pdu(0xA4)));
    let mut junk = Vec::from(&GET_V1[..]);
    junk.push(0);
    t.eq("bytes after the message", Message::parse(&junk), Err(snmp::Bad::Ber(Bad::Trailing)));
    t.eq("cut message", Message::parse(&GET_V1[..30]), Err(snmp::Bad::Ber(Bad::Truncated)));
    let mut no_null = Vec::from(&GET_V1[..]);
    no_null[38] = 0x09;
    t.eq("an unknown value type", Message::parse(&no_null), Err(snmp::Bad::Ber(Bad::Tag(0x09))));
    // the error statuses of 2c in a version-1 response (RFC 3584 4.4)
    let map: Vec<i32> = [snmp::E_NO_ERROR, snmp::E_TOO_BIG, snmp::E_GEN_ERR, snmp::E_WRONG_TYPE, snmp::E_WRONG_LENGTH, snmp::E_WRONG_VALUE, snmp::E_INCONSISTENT_VALUE, snmp::E_NO_ACCESS, snmp::E_NOT_WRITABLE, snmp::E_NO_CREATION, snmp::E_INCONSISTENT_NAME, snmp::E_RESOURCE_UNAVAILABLE, snmp::E_COMMIT_FAILED].iter().map(|&e| snmp::v1_error(e)).collect();
    t.eq("v1 errors", map, alloc::vec![0, 1, 5, 3, 3, 3, 3, 2, 2, 2, 2, 5, 5]);
    t.eq("error names", [snmp::error_name(2), snmp::error_name(6), snmp::error_name(17), snmp::error_name(99)], ["noSuchName", "noAccess", "notWritable", "unknown"]);
    t.eq("versions", (Version::parse("2c"), Version::parse("1"), Version::parse("3"), Version::V2c.number()), (Some(Version::V2c), Some(Version::V1), None, 1));
    t.check("kinds per version", Kind::GetBulk.in_v2c() && !Kind::GetBulk.in_v1() && Kind::TrapV1.in_v1() && !Kind::TrapV1.in_v2c() && !Kind::Inform.in_v1());
}

fn notifications(t: &mut T) {
    t.eq("v2 OID of linkDown", snmp::trap_oid(snmp::G_LINK_DOWN, &oid("1.3.6.1.4.1.32473.1.4"), 0), oid("1.3.6.1.6.3.1.1.5.3"));
    t.eq("v2 OID of coldStart", snmp::trap_oid(snmp::G_COLD_START, &oid("1.3.6.1.4.1.32473.1.4"), 0).arcs().to_vec(), Vec::from(&snmp::COLD_START[..]));
    t.eq("enterprise specific", snmp::trap_oid(snmp::G_ENTERPRISE, &oid("1.3.6.1.4.1.32473.1.4"), 7), oid("1.3.6.1.4.1.32473.1.4.0.7"));
    t.eq("generic of linkUp", snmp::generic_of(&oid("1.3.6.1.6.3.1.1.5.4")), Some(snmp::G_LINK_UP));
    t.eq("generic of an enterprise one", snmp::generic_of(&oid("1.3.6.1.4.1.32473.0.7")), None);
    let vbs = alloc::vec![VarBind::new(Oid::new(&snmp::SYS_UPTIME_0), Value::TimeTicks(42)), VarBind::new(Oid::new(&snmp::SNMP_TRAP_OID_0), Value::Oid(Oid::new(&snmp::LINK_UP)))];
    t.eq("the notification", snmp::notification(&vbs).map(|o| o.to_string()), Some(s("1.3.6.1.6.3.1.1.5.4")));
    t.eq("generic names", [snmp::generic_name(0), snmp::generic_name(4), snmp::generic_name(6)], ["coldStart", "authenticationFailure", "enterpriseSpecific"]);
}

fn names(t: &mut T) {
    let objs = mib::OBJECTS;
    t.check("objects in OID order", objs.windows(2).all(|w| w[0].oid < w[1].oid));
    let mut names: Vec<&str> = objs.iter().map(|o| o.name).collect();
    names.sort();
    names.dedup();
    t.eq("names unique", names.len(), objs.len());
    t.eq("lookup with module", mib::lookup("SNMPv2-MIB::sysName.0"), Some(oid("1.3.6.1.2.1.1.5.0")));
    t.eq("lookup bare", mib::lookup("ifDescr.2"), Some(oid("1.3.6.1.2.1.2.2.1.2.2")));
    t.eq("lookup a table", mib::lookup("IF-MIB::ifXTable"), Some(oid("1.3.6.1.2.1.31.1.1")));
    t.eq("lookup numeric", (mib::lookup(".1.3.6.1.2.1.1.5.0"), mib::lookup("1.3.6.1.2.1.1")), (Some(oid("1.3.6.1.2.1.1.5.0")), Some(oid("1.3.6.1.2.1.1"))));
    t.eq("lookup iso", mib::lookup("iso.3.6.1.2.1.1.5.0"), Some(oid("1.3.6.1.2.1.1.5.0")));
    t.eq("lookup refused", [mib::lookup("FOO-MIB::sysName.0"), mib::lookup("sysNmae.0"), mib::lookup("sysName.x"), mib::lookup("")], [None, None, None, None]);
    t.eq("name of an instance", mib::name_of(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 2, 3], false), s("IF-MIB::ifDescr.3"));
    t.eq("name of the enterprise", mib::name_of(&[1, 3, 6, 1, 4, 1, 32473, 1, 4], false), s("SNMPv2-SMI::enterprises.32473.1.4"));
    t.eq("name numeric", mib::name_of(&[1, 3, 6, 1, 2, 1, 1, 5, 0], true), s(".1.3.6.1.2.1.1.5.0"));
    t.eq("name of a route", mib::name_of(&oid("1.3.6.1.2.1.4.24.4.1.1.10.0.0.0.255.255.255.0.0.0.0.0.0").0, false), s("IP-FORWARD-MIB::ipCidrRouteDest.10.0.0.0.255.255.255.0.0.0.0.0.0"));
    t.eq("short name", mib::short_name(&[1, 3, 6, 1, 2, 1, 1, 5, 0]), s("sysName.0"));
    t.eq("a notification", mib::name_of(&snmp::LINK_DOWN, false), s("IF-MIB::linkDown"));
    t.eq("unknown below iso", mib::name_of(&[1, 2, 999], false), s("SNMPv2-SMI::iso.2.999"));
    t.eq("children of system", mib::children(&[1, 3, 6, 1, 2, 1, 1]).iter().map(|o| o.name).collect::<Vec<_>>(), alloc::vec!["sysDescr", "sysObjectID", "sysUpTime", "sysContact", "sysName", "sysLocation", "sysServices"]);
    t.eq("children of ifTable", mib::children(&[1, 3, 6, 1, 2, 1, 2, 2]).iter().map(|o| o.name).collect::<Vec<_>>(), alloc::vec!["ifEntry"]);
    t.eq("columns of ifEntry", mib::children(&[1, 3, 6, 1, 2, 1, 2, 2, 1]).len(), 22);
    t.eq("children of mib-2", mib::children(&[1, 3, 6, 1, 2, 1]).iter().map(|o| o.name).collect::<Vec<_>>(), alloc::vec!["system", "interfaces", "ip", "snmp", "ifMIB"]);
    t.check("column of an instance", mib::column_of(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 7, 1]).is_some_and(|o| o.name == "ifAdminStatus" && o.access == mib::Access::ReadWrite) && mib::column_of(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 7]).is_none());
    // values as net-snmp prints them
    let show = |o: &str, v: Value| mib::show(&oid(o).0, &v, false);
    t.eq("show string", show("1.3.6.1.2.1.1.5.0", Value::text("R1")), s("STRING: R1"));
    t.eq("show enum", show("1.3.6.1.2.1.2.2.1.7.1", Value::Int(1)), s("INTEGER: up(1)"));
    t.eq("show ifType", show("1.3.6.1.2.1.2.2.1.3.1", Value::Int(6)), s("INTEGER: ethernetCsmacd(6)"));
    t.eq("show plain integer", show("1.3.6.1.2.1.2.2.1.4.1", Value::Int(1500)), s("INTEGER: 1500"));
    t.eq("show ticks", show("1.3.6.1.2.1.1.3.0", Value::TimeTicks(500)), s("Timeticks: (500) 0:00:05.00"));
    t.eq("ticks of a day", mib::ticks_text(8_640_001), s("(8640001) 1 day, 0:00:00.01"));
    t.eq("ticks of days", mib::ticks_text(26_284_567), s("(26284567) 3 days, 1:00:45.67"));
    t.eq("show MAC", show("1.3.6.1.2.1.2.2.1.6.1", Value::Str(alloc::vec![0x02, 0x4A, 0, 0, 1, 1])), s("STRING: 2:4a:0:0:1:1"));
    t.eq("show OID", show("1.3.6.1.2.1.1.2.0", Value::Oid(oid("1.3.6.1.4.1.32473.1.4"))), s("OID: SNMPv2-SMI::enterprises.32473.1.4"));
    t.eq("show OID numeric", mib::show(&[1, 3, 6, 1, 2, 1, 1, 2, 0], &Value::Oid(oid("1.3.6.1.4.1.32473.1.4")), true), s("OID: .1.3.6.1.4.1.32473.1.4"));
    t.eq("show counters", (show("1.3.6.1.2.1.2.2.1.10.1", Value::Counter32(5)), show("1.3.6.1.2.1.31.1.1.1.6.1", Value::Counter64(7)), show("1.3.6.1.2.1.2.2.1.5.1", Value::Gauge32(1_000_000_000))), (s("Counter32: 5"), s("Counter64: 7"), s("Gauge32: 1000000000")));
    t.eq("show an address", show("1.3.6.1.2.1.4.20.1.1.10.0.0.1", Value::Ip(Ipv4([10, 0, 0, 1]))), s("IpAddress: 10.0.0.1"));
    t.eq("show octets", (show("1.3.6.9", Value::text("abc")), show("1.3.6.9", Value::Str(alloc::vec![0, 0xFF]))), (s("STRING: \"abc\""), s("Hex-STRING: 00 FF ")));
    t.eq("bare", (mib::bare(&oid("1.3.6.1.2.1.2.2.1.8.1").0, &Value::Int(2), false), mib::bare(&oid("1.3.6.1.2.1.1.3.0").0, &Value::TimeTicks(500), false)), (s("down"), s("0:00:05.00")));
}

fn frames(t: &mut T) {
    let get = Message::new(Version::V2c, "public", Pdu::new(Kind::Get, 4660, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.1.5.0"))]));
    let f = frame(40000, snmp::PORT, &get.to_bytes());
    t.eq("mask", decode::proto_mask(&f) & (decode::P_SNMP | decode::P_UDP), decode::P_SNMP | decode::P_UDP);
    t.eq("get summary", decode::info(&f), (Proto::Snmp, s("GetRequest 4660: SNMPv2-MIB::sysName.0")));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == Proto::Snmp);
    t.eq("get title", l.map(|l| l.title.clone()), Some(s("Simple Network Management Protocol, v2c, GetRequest")));
    let fields: Vec<(&str, String)> = l.map(|l| l.fields.iter().map(|f| (f.name, f.value.clone())).collect()).unwrap_or_default();
    t.eq(
        "get fields",
        fields,
        alloc::vec![
            ("Version", s("v2c (1)")),
            ("Community", s("public")),
            ("PDU", s("GetRequest (0xa0)")),
            ("Request ID", s("4660")),
            ("Error status", s("noError (0)")),
            ("Error index", s("0")),
            ("Variable bindings", s("1 binding")),
            ("Object Name", s("SNMPv2-MIB::sysName.0 (1.3.6.1.2.1.1.5.0)")),
            ("Value", s("NULL")),
        ]
    );
    // the community's field covers its element (Ethernet 14 + IPv4 20 + UDP 8, then 30 xx 02 01 01)
    t.eq("community range", l.and_then(|l| l.fields.get(1)).map(|f| (f.off, f.len)), Some((47, 8)));
    t.eq("no problems", l.map(|l| l.problems.clone()), Some(Vec::new()));
    let mut resp = Pdu::new(Kind::Response, 4660, alloc::vec![VarBind::new(oid("1.3.6.1.2.1.1.5.0"), Value::text("R1"))]);
    let r = frame(snmp::PORT, 40000, &Message::new(Version::V2c, "public", resp.clone()).to_bytes());
    t.eq("response summary", decode::info(&r).1, s("Response 4660: SNMPv2-MIB::sysName.0 = STRING: R1"));
    resp.error_status = snmp::E_NOT_WRITABLE;
    resp.error_index = 1;
    let e = frame(snmp::PORT, 40000, &Message::new(Version::V2c, "public", resp).to_bytes());
    t.eq("error summary", decode::info(&e).1, s("Response 4660: notWritable (index 1)"));
    let vbs = alloc::vec![VarBind::new(Oid::new(&snmp::SYS_UPTIME_0), Value::TimeTicks(500)), VarBind::new(Oid::new(&snmp::SNMP_TRAP_OID_0), Value::Oid(Oid::new(&snmp::LINK_DOWN))), VarBind::new(oid("1.3.6.1.2.1.2.2.1.1.2"), Value::Int(2))];
    let tr = frame(49152, snmp::PORT_TRAP, &Message::new(Version::V2c, "public", Pdu::new(Kind::TrapV2, 17, vbs)).to_bytes());
    t.eq("trap summary", decode::info(&tr), (Proto::Snmp, s("SNMPv2-Trap 17: IF-MIB::linkDown")));
    let lt = decode::layers(&tr);
    t.check("trap values", lt.iter().find(|l| l.proto == Proto::Snmp).is_some_and(|l| l.fields.iter().any(|f| f.name == "Value" && f.value == "OID: IF-MIB::linkDown") && l.fields.iter().any(|f| f.name == "Value" && f.value == "Timeticks: (500) 0:00:05.00")));
    let bulk = frame(40000, snmp::PORT, &Message::new(Version::V2c, "public", Pdu::bulk(5, 0, 10, alloc::vec![VarBind::null(oid("1.3.6.1.2.1.2.2.1.2"))])).to_bytes());
    t.eq("bulk summary", decode::info(&bulk).1, s("GetBulkRequest 5: N=0 M=10 IF-MIB::ifDescr"));
    t.check("bulk fields", decode::layers(&bulk).iter().find(|l| l.proto == Proto::Snmp).is_some_and(|l| l.fields.iter().any(|f| f.name == "Max-repetitions" && f.value == "10")));
    // a v1 Trap on port 162
    let v1 = Message { version: Version::V1, community: Vec::from(&b"public"[..]), body: Body::Trap(TrapV1 { enterprise: oid("1.3.6.1.4.1.32473.1.4"), agent_addr: Ipv4([10, 0, 0, 1]), generic: snmp::G_LINK_UP, specific: 0, timestamp: 900, varbinds: Vec::new() }) };
    let v1f = frame(49152, snmp::PORT_TRAP, &v1.to_bytes());
    t.eq("v1 trap summary", decode::info(&v1f).1, s("Trap (v1): linkUp, agent 10.0.0.1"));
    t.check("v1 trap fields", decode::layers(&v1f).iter().find(|l| l.proto == Proto::Snmp).is_some_and(|l| l.fields.iter().any(|f| f.name == "Generic trap" && f.value == "linkUp (3)") && l.fields.iter().any(|f| f.name == "Enterprise" && f.value == "SNMPv2-SMI::enterprises.32473.1.4 (1.3.6.1.4.1.32473.1.4)")));
    // malformed: the problem names it, the summary says so
    let mut cut = get.to_bytes();
    cut.truncate(cut.len() - 3);
    let bad = frame(40000, snmp::PORT, &cut);
    t.eq("malformed problem", decode::layers(&bad).iter().find(|l| l.proto == Proto::Snmp).map(|l| l.problems.clone()), Some(alloc::vec![s("the data ends before the element it announces")]));
    t.check("malformed summary", decode::info(&bad).1.starts_with("SNMP (malformed: "));
    let mut v3 = get.to_bytes();
    v3[4] = 3;
    let v3f = frame(40000, snmp::PORT, &v3);
    t.eq("v3 problem", decode::layers(&v3f).iter().find(|l| l.proto == Proto::Snmp).map(|l| l.problems.clone()), Some(alloc::vec![s("SNMPv3 (version 3), which this dissector does not take apart")]));
}
