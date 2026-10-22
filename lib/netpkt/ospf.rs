// ospf -- OSPF version 2 packets and LSAs byte for byte (RFC 2328 appendix A): the 24-byte
// header with its checksum and authentication field, the five packet types, LSA headers with
// the Fletcher checksum of RFC 2328 12.1.7 (RFC 905 annex B), router-, network-, summary- and
// AS-external-LSAs, and how two instances of an LSA compare (13.1)
//
//   OspfView::parse(b)       version, kind, length, router ID, area, checksum, authentication
//                            type and field, body(); checksum_ok() (the IP checksum of the
//                            packet without the authentication field, A.3.1); None when the
//                            bytes are shorter than the header or than its length field
//   Header { kind, router, area, autype, auth }   the common header
//   Packet { hdr, body }     read(&view) (Err with the reason: OspfErr::text) / to_bytes()
//                            (length and checksum filled in; with AU_CRYPTO the checksum stays
//                            0 and no digest is appended)
//   Body::Hello(Hello)       network mask, HelloInterval, options, priority, RouterDeadInterval,
//                            DR, BDR, the neighbours seen
//        Dd(Dd)              interface MTU, options, the I / M / MS flags, DD sequence number,
//                            LSA headers
//        Lsr(Vec<LsaKey>)    the LSAs asked for (type, Link State ID, advertising router)
//        Lsu(Vec<Vec<u8>>)   whole LSAs as they travel (flooding forwards them byte for byte)
//        Ack(Vec<LsaHeader>) the LSA headers acknowledged
//   LsaHeader                age, options, type, Link State ID, advertising router, sequence
//                            number, checksum, length; read / write / of(lsa) / key()
//   Lsa { hdr, body }        parse(lsa) (Err: LsaErr) / to_bytes() (length and Fletcher
//                            checksum filled in); LsaBody::Router (flags V / E / B and links
//                            with metrics), Network (mask, attached routers), Summary (types 3
//                            and 4: mask and metric), External (mask, type 1 or 2, metric,
//                            forwarding address, route tag), Other (bytes of an unknown type)
//   lsa_checksum(lsa) / lsa_checksum_ok(lsa)   the Fletcher checksum over the LSA without its
//                            age, C0 and C1 both zero for a correct one
//   set_age(lsa, age)        a new LS age (outside the checksum)
//   compare(a, b)            which instance is more recent: sequence number (signed), checksum,
//                            MaxAge, an age difference beyond MaxAgeDiff (13.1)
//   type_name, lsa_name, options_text, dd_flags_text, area_text   names for dissectors
//
// Constants: VERSION, the packet types T_*, ALL_SPF_ROUTERS (224.0.0.5), ALL_D_ROUTERS
// (224.0.0.6), authentication types AU_*, option bits OPT_*, the DD flags DD_I, DD_M, DD_MS,
// LSA types LSA_*, router-LSA link types LINK_* and flags RTR_B / RTR_E / RTR_V, the
// architectural constants of appendix B (MAX_AGE, LS_REFRESH_TIME, MIN_LS_INTERVAL, ...),
// INITIAL_SEQ / MAX_SEQ and LS_INFINITY.
//
// The codec reads and writes; what a receiver refuses because of its own state (mismatched
// timers, the area, the neighbours) is the protocol's business.  TOS metrics other than TOS 0
// are read and written but carry no meaning (RFC 2328 dropped TOS routing).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::addr::Ipv4;
use crate::checksum;
use crate::{be16, be32, put16, put32};

/// OSPF version 2
pub const VERSION: u8 = 2;
/// the common header, an LSA header, an entry of a Link State Request
pub const HDR_LEN: usize = 24;
pub const LSA_HDR_LEN: usize = 20;
pub const LSR_ENTRY_LEN: usize = 12;
/// the fixed part of a Hello and of a Database Description packet
pub const HELLO_LEN: usize = 20;
pub const DD_LEN: usize = 8;

/// packet types
pub const T_HELLO: u8 = 1;
pub const T_DD: u8 = 2;
pub const T_LSR: u8 = 3;
pub const T_LSU: u8 = 4;
pub const T_ACK: u8 = 5;

/// every OSPF router listens to AllSPFRouters, the DR and the BDR to AllDRouters (A.1)
pub const ALL_SPF_ROUTERS: Ipv4 = Ipv4([224, 0, 0, 5]);
pub const ALL_D_ROUTERS: Ipv4 = Ipv4([224, 0, 0, 6]);

/// authentication types (appendix D)
pub const AU_NULL: u16 = 0;
pub const AU_SIMPLE: u16 = 1;
pub const AU_CRYPTO: u16 = 2;

/// option bits (A.2; DN and O came later: RFC 4576, RFC 5250)
pub const OPT_E: u8 = 0x02;
pub const OPT_MC: u8 = 0x04;
pub const OPT_NP: u8 = 0x08;
pub const OPT_EA: u8 = 0x10;
pub const OPT_DC: u8 = 0x20;
pub const OPT_O: u8 = 0x40;
pub const OPT_DN: u8 = 0x80;

/// the flags of a Database Description packet: Init, More, Master/Slave
pub const DD_I: u8 = 0x04;
pub const DD_M: u8 = 0x02;
pub const DD_MS: u8 = 0x01;

/// LSA types
pub const LSA_ROUTER: u8 = 1;
pub const LSA_NETWORK: u8 = 2;
pub const LSA_SUMMARY: u8 = 3;
pub const LSA_ASBR: u8 = 4;
pub const LSA_EXTERNAL: u8 = 5;

/// the links of a router-LSA
pub const LINK_P2P: u8 = 1;
pub const LINK_TRANSIT: u8 = 2;
pub const LINK_STUB: u8 = 3;
pub const LINK_VIRTUAL: u8 = 4;

/// the flags of a router-LSA: area border router, AS boundary router, virtual link endpoint
pub const RTR_B: u8 = 0x01;
pub const RTR_E: u8 = 0x02;
pub const RTR_V: u8 = 0x04;

/// architectural constants (appendix B), in seconds
pub const LS_REFRESH_TIME: u16 = 1800;
pub const MIN_LS_INTERVAL: u16 = 5;
pub const MIN_LS_ARRIVAL: u16 = 1;
pub const MAX_AGE: u16 = 3600;
pub const CHECK_AGE: u16 = 300;
pub const MAX_AGE_DIFF: u16 = 900;
/// the metric of an unreachable destination in summary- and AS-external-LSAs
pub const LS_INFINITY: u32 = 0xFF_FFFF;
/// LS sequence numbers are signed: the first one, the largest one (12.1.6)
pub const INITIAL_SEQ: u32 = 0x8000_0001;
pub const MAX_SEQ: u32 = 0x7FFF_FFFF;

/// the name of a packet type as capture tools show it
pub fn type_name(t: u8) -> &'static str {
    match t {
        T_HELLO => "Hello Packet",
        T_DD => "DB Description",
        T_LSR => "LS Request",
        T_LSU => "LS Update",
        T_ACK => "LS Acknowledge",
        _ => "Unknown",
    }
}

/// the name of an LSA type
pub fn lsa_name(t: u8) -> &'static str {
    match t {
        LSA_ROUTER => "Router-LSA",
        LSA_NETWORK => "Network-LSA",
        LSA_SUMMARY => "Summary-LSA (IP network)",
        LSA_ASBR => "Summary-LSA (ASBR)",
        LSA_EXTERNAL => "AS-External-LSA",
        _ => "Unknown LSA",
    }
}

/// the set option bits: "E, MC" ("none" without any)
pub fn options_text(o: u8) -> String {
    let names = [(OPT_DN, "DN"), (OPT_O, "O"), (OPT_DC, "DC"), (OPT_EA, "EA"), (OPT_NP, "N/P"), (OPT_MC, "MC"), (OPT_E, "E")];
    let v: Vec<&str> = names.iter().filter(|(b, _)| o & b != 0).map(|(_, n)| *n).collect();
    if v.is_empty() { String::from("none") } else { v.join(", ") }
}

/// the flags of a Database Description packet: "I, M, MS" ("none")
pub fn dd_flags_text(f: u8) -> String {
    let names = [(DD_I, "I"), (DD_M, "M"), (DD_MS, "MS")];
    let v: Vec<&str> = names.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect();
    if v.is_empty() { String::from("none") } else { v.join(", ") }
}

/// an area ID as dissectors show it: "0.0.0.0 (Backbone)", "0.0.0.1"
pub fn area_text(a: u32) -> String {
    if a == 0 { String::from("0.0.0.0 (Backbone)") } else { format!("{}", Ipv4::from_u32(a)) }
}

// ---------------------------------------------------------------- checksums

/// the Internet checksum of a packet without its authentication field (A.3.1): the header up to
/// the authentication type and the body, the checksum field taken as zero
fn packet_checksum(b: &[u8]) -> u16 {
    let mut s = checksum::sum(0, &b[..12]);
    s = checksum::sum(s, &b[14..16]);
    s = checksum::sum(s, &b[HDR_LEN..]);
    checksum::finish(s)
}

/// C0 and C1 of Fletcher's checksum over `d` (mod 255), the bytes at `skip` and `skip + 1` taken
/// as zero when `skip` is given
fn fletcher_sums(d: &[u8], skip: Option<usize>) -> (u32, u32) {
    let (mut c0, mut c1) = (0u32, 0u32);
    for (i, &x) in d.iter().enumerate() {
        let x = match skip {
            Some(k) if i == k || i == k + 1 => 0,
            _ => x as u32,
        };
        c0 = (c0 + x) % 255;
        c1 = (c1 + c0) % 255;
    }
    (c0, c1)
}

/// the Fletcher checksum of an LSA (RFC 2328 12.1.7): over the bytes from the options field to
/// the end of the LSA (the LS age left out), the checksum field taken as zero; the two check
/// bytes X and Y of RFC 905 B.4 that make C0 and C1 zero, a zero byte written as 255
pub fn lsa_checksum(lsa: &[u8]) -> u16 {
    if lsa.len() < LSA_HDR_LEN {
        return 0;
    }
    let len = (be16(lsa, 18) as usize).clamp(LSA_HDR_LEN, lsa.len());
    let d = &lsa[2..len];
    // the checksum field is at 16 in the LSA, at 14 in what the sum covers (1-based: 15)
    let (c0, c1) = fletcher_sums(d, Some(14));
    let l = d.len() as i64;
    let mut x = ((l - 15) * c0 as i64 - c1 as i64).rem_euclid(255);
    if x == 0 {
        x = 255;
    }
    let mut y = (-(c0 as i64) - x).rem_euclid(255);
    if y == 0 {
        y = 255;
    }
    ((x as u16) << 8) | y as u16
}

/// an LSA's checksum is correct: C0 and C1 over the bytes after the age are both zero (RFC 905
/// B.2); a checksum of 0 is never correct
pub fn lsa_checksum_ok(lsa: &[u8]) -> bool {
    if lsa.len() < LSA_HDR_LEN || be16(lsa, 16) == 0 {
        return false;
    }
    let len = be16(lsa, 18) as usize;
    if len < LSA_HDR_LEN || len > lsa.len() {
        return false;
    }
    fletcher_sums(&lsa[2..len], None) == (0, 0)
}

/// writes a new LS age into an LSA (the checksum does not cover it)
pub fn set_age(lsa: &mut [u8], age: u16) {
    if lsa.len() >= 2 {
        put16(lsa, 0, age);
    }
}

// ---------------------------------------------------------------- LSA headers

/// what identifies an LSA: its type, Link State ID and advertising router (12.1)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct LsaKey {
    pub kind: u8,
    pub id: Ipv4,
    pub adv: Ipv4,
}

/// the 20-byte header of an LSA
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct LsaHeader {
    /// seconds since it was originated
    pub age: u16,
    pub options: u8,
    pub kind: u8,
    pub id: Ipv4,
    pub adv: Ipv4,
    /// signed: INITIAL_SEQ (0x80000001) is the smallest one used
    pub seq: u32,
    pub checksum: u16,
    /// the whole LSA with this header
    pub len: u16,
}

impl LsaHeader {
    /// a header from its 20 bytes (the caller has checked the length)
    pub fn read(b: &[u8]) -> LsaHeader {
        LsaHeader {
            age: be16(b, 0),
            options: b[2],
            kind: b[3],
            id: Ipv4([b[4], b[5], b[6], b[7]]),
            adv: Ipv4([b[8], b[9], b[10], b[11]]),
            seq: be32(b, 12),
            checksum: be16(b, 16),
            len: be16(b, 18),
        }
    }
    /// the header of an LSA (None when it is shorter than a header)
    pub fn of(lsa: &[u8]) -> Option<LsaHeader> {
        if lsa.len() < LSA_HDR_LEN { None } else { Some(LsaHeader::read(lsa)) }
    }
    /// writes the header into 20 bytes
    pub fn write(&self, b: &mut [u8]) {
        put16(b, 0, self.age);
        b[2] = self.options;
        b[3] = self.kind;
        b[4..8].copy_from_slice(&self.id.0);
        b[8..12].copy_from_slice(&self.adv.0);
        put32(b, 12, self.seq);
        put16(b, 16, self.checksum);
        put16(b, 18, self.len);
    }
    pub fn key(&self) -> LsaKey {
        LsaKey { kind: self.kind, id: self.id, adv: self.adv }
    }
}

/// which of two instances of the same LSA is more recent (RFC 2328 13.1): Greater when `a` is,
/// Equal when they are the same instance
pub fn compare(a: &LsaHeader, b: &LsaHeader) -> Ordering {
    let (sa, sb) = (a.seq as i32, b.seq as i32);
    if sa != sb {
        return sa.cmp(&sb);
    }
    if a.checksum != b.checksum {
        return a.checksum.cmp(&b.checksum);
    }
    let (ma, mb) = (a.age >= MAX_AGE, b.age >= MAX_AGE);
    if ma != mb {
        return if ma { Ordering::Greater } else { Ordering::Less };
    }
    if a.age.abs_diff(b.age) > MAX_AGE_DIFF {
        // the younger one is more recent
        return b.age.cmp(&a.age);
    }
    Ordering::Equal
}

// ---------------------------------------------------------------- LSAs

/// one link of a router-LSA (A.4.2)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct RouterLink {
    /// LINK_P2P: the neighbour's router ID; LINK_TRANSIT: the DR's interface address;
    /// LINK_STUB: the network
    pub id: Ipv4,
    /// the router's interface address (LINK_P2P, LINK_TRANSIT; an interface index when
    /// unnumbered) or the network mask (LINK_STUB)
    pub data: Ipv4,
    pub kind: u8,
    /// the TOS 0 metric
    pub metric: u16,
    /// further TOS metrics: (TOS, metric)
    pub tos: Vec<(u8, u16)>,
}

/// the body of a router-LSA
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct RouterLsa {
    /// RTR_V, RTR_E, RTR_B
    pub flags: u8,
    pub links: Vec<RouterLink>,
}

/// the body of a network-LSA (A.4.3)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct NetworkLsa {
    pub mask: Ipv4,
    /// the routers fully adjacent to the DR, the DR itself included
    pub routers: Vec<Ipv4>,
}

/// the body of a summary-LSA of type 3 or 4 (A.4.4; type 4 has the mask 0)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct SummaryLsa {
    pub mask: Ipv4,
    /// 24 bits; LS_INFINITY: unreachable
    pub metric: u32,
    pub tos: Vec<(u8, u32)>,
}

/// the TOS 0 part of an AS-external-LSA and its further TOS entries (A.4.5)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct ExternalLsa {
    pub mask: Ipv4,
    /// the E bit: a type 2 metric (larger than any path inside the AS)
    pub e2: bool,
    /// 24 bits
    pub metric: u32,
    /// where traffic for the destination should go (0.0.0.0: to the AS boundary router)
    pub fwd: Ipv4,
    pub tag: u32,
    /// further TOS entries: (E bit and TOS, metric, forwarding address, tag)
    pub tos: Vec<(u8, u32, Ipv4, u32)>,
}

/// what an LSA says
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LsaBody {
    Router(RouterLsa),
    Network(NetworkLsa),
    Summary(SummaryLsa),
    External(ExternalLsa),
    /// an LSA of another type: its bytes after the header
    Other(Vec<u8>),
}

/// why an LSA cannot be read
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LsaErr {
    /// fewer bytes than its header or its length field
    Short,
    /// the length field does not fit what the body needs
    Length,
}

impl LsaErr {
    pub fn text(self) -> &'static str {
        match self {
            LsaErr::Short => "shorter than its header or its length",
            LsaErr::Length => "its length does not fit its body",
        }
    }
}

/// an LSA as a record
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lsa {
    pub hdr: LsaHeader,
    pub body: LsaBody,
}

impl Lsa {
    /// a new instance: age 0; to_bytes fills in length and checksum
    pub fn new(kind: u8, id: Ipv4, adv: Ipv4, seq: u32, options: u8, body: LsaBody) -> Lsa {
        Lsa { hdr: LsaHeader { age: 0, options, kind, id, adv, seq, checksum: 0, len: 0 }, body }
    }
    pub fn key(&self) -> LsaKey {
        self.hdr.key()
    }

    /// reads an LSA (the bytes may go on after it: its length field says where it ends)
    pub fn parse(b: &[u8]) -> Result<Lsa, LsaErr> {
        let hdr = LsaHeader::of(b).ok_or(LsaErr::Short)?;
        let len = hdr.len as usize;
        if len < LSA_HDR_LEN || len > b.len() {
            return Err(LsaErr::Short);
        }
        let d = &b[LSA_HDR_LEN..len];
        let body = match hdr.kind {
            LSA_ROUTER => {
                if d.len() < 4 {
                    return Err(LsaErr::Length);
                }
                let n = be16(d, 2) as usize;
                let mut links = Vec::with_capacity(n);
                let mut o = 4;
                for _ in 0..n {
                    let l = d.get(o..o + 12).ok_or(LsaErr::Length)?;
                    let ntos = l[9] as usize;
                    let t = d.get(o + 12..o + 12 + 4 * ntos).ok_or(LsaErr::Length)?;
                    links.push(RouterLink {
                        id: Ipv4([l[0], l[1], l[2], l[3]]),
                        data: Ipv4([l[4], l[5], l[6], l[7]]),
                        kind: l[8],
                        metric: be16(l, 10),
                        tos: t.chunks_exact(4).map(|c| (c[0], be16(c, 2))).collect(),
                    });
                    o += 12 + 4 * ntos;
                }
                if o != d.len() {
                    return Err(LsaErr::Length);
                }
                LsaBody::Router(RouterLsa { flags: d[0], links })
            }
            LSA_NETWORK => {
                if d.len() < 4 || d.len() % 4 != 0 {
                    return Err(LsaErr::Length);
                }
                LsaBody::Network(NetworkLsa { mask: Ipv4([d[0], d[1], d[2], d[3]]), routers: d[4..].chunks_exact(4).map(|c| Ipv4([c[0], c[1], c[2], c[3]])).collect() })
            }
            LSA_SUMMARY | LSA_ASBR => {
                if d.len() < 8 || d.len() % 4 != 0 {
                    return Err(LsaErr::Length);
                }
                LsaBody::Summary(SummaryLsa {
                    mask: Ipv4([d[0], d[1], d[2], d[3]]),
                    metric: be32(d, 4) & LS_INFINITY,
                    tos: d[8..].chunks_exact(4).map(|c| (c[0], be32(c, 0) & LS_INFINITY)).collect(),
                })
            }
            LSA_EXTERNAL => {
                if d.len() < 16 || (d.len() - 4) % 12 != 0 {
                    return Err(LsaErr::Length);
                }
                let tos = d[16..].chunks_exact(12).map(|c| (c[0], be32(c, 0) & LS_INFINITY, Ipv4([c[4], c[5], c[6], c[7]]), be32(c, 8))).collect();
                LsaBody::External(ExternalLsa {
                    mask: Ipv4([d[0], d[1], d[2], d[3]]),
                    e2: d[4] & 0x80 != 0,
                    metric: be32(d, 4) & LS_INFINITY,
                    fwd: Ipv4([d[8], d[9], d[10], d[11]]),
                    tag: be32(d, 12),
                    tos,
                })
            }
            _ => LsaBody::Other(d.to_vec()),
        };
        Ok(Lsa { hdr, body })
    }

    /// the body as bytes
    fn body_bytes(&self) -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        match &self.body {
            LsaBody::Router(r) => {
                b.extend_from_slice(&[r.flags, 0]);
                b.extend_from_slice(&(r.links.len() as u16).to_be_bytes());
                for l in &r.links {
                    b.extend_from_slice(&l.id.0);
                    b.extend_from_slice(&l.data.0);
                    b.extend_from_slice(&[l.kind, l.tos.len() as u8]);
                    b.extend_from_slice(&l.metric.to_be_bytes());
                    for (t, m) in &l.tos {
                        b.extend_from_slice(&[*t, 0]);
                        b.extend_from_slice(&m.to_be_bytes());
                    }
                }
            }
            LsaBody::Network(n) => {
                b.extend_from_slice(&n.mask.0);
                for r in &n.routers {
                    b.extend_from_slice(&r.0);
                }
            }
            LsaBody::Summary(s) => {
                b.extend_from_slice(&s.mask.0);
                b.extend_from_slice(&(s.metric & LS_INFINITY).to_be_bytes());
                for (t, m) in &s.tos {
                    b.extend_from_slice(&(((*t as u32) << 24) | (m & LS_INFINITY)).to_be_bytes());
                }
            }
            LsaBody::External(e) => {
                b.extend_from_slice(&e.mask.0);
                let first = if e.e2 { 0x8000_0000 } else { 0 } | (e.metric & LS_INFINITY);
                b.extend_from_slice(&first.to_be_bytes());
                b.extend_from_slice(&e.fwd.0);
                b.extend_from_slice(&e.tag.to_be_bytes());
                for (t, m, f, tag) in &e.tos {
                    b.extend_from_slice(&(((*t as u32) << 24) | (m & LS_INFINITY)).to_be_bytes());
                    b.extend_from_slice(&f.0);
                    b.extend_from_slice(&tag.to_be_bytes());
                }
            }
            LsaBody::Other(d) => b.extend_from_slice(d),
        }
        b
    }

    /// the LSA as bytes: the header with the length and the Fletcher checksum filled in (the
    /// record's age stays as it is)
    pub fn to_bytes(&self) -> Vec<u8> {
        let body = self.body_bytes();
        let mut b = alloc::vec![0u8; LSA_HDR_LEN + body.len()];
        let h = LsaHeader { len: b.len() as u16, checksum: 0, ..self.hdr };
        h.write(&mut b[..LSA_HDR_LEN]);
        b[LSA_HDR_LEN..].copy_from_slice(&body);
        let ck = lsa_checksum(&b);
        put16(&mut b, 16, ck);
        b
    }
}

// ---------------------------------------------------------------- packets

/// the common header (A.3.1)
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Header {
    pub kind: u8,
    /// the sender's router ID
    pub router: Ipv4,
    pub area: u32,
    pub autype: u16,
    /// the authentication field: zeros (AU_NULL), the password (AU_SIMPLE), key ID, length and
    /// sequence number (AU_CRYPTO)
    pub auth: [u8; 8],
}

/// a Hello packet (A.3.2)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Hello {
    pub mask: Ipv4,
    /// HelloInterval in seconds
    pub interval: u16,
    pub options: u8,
    pub priority: u8,
    /// RouterDeadInterval in seconds
    pub dead: u32,
    /// the interface addresses of the DR and the BDR as the sender sees them (0.0.0.0: none)
    pub dr: Ipv4,
    pub bdr: Ipv4,
    /// the router IDs it heard Hellos from within RouterDeadInterval
    pub neighbors: Vec<Ipv4>,
}

/// a Database Description packet (A.3.3)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Dd {
    /// the largest IP datagram the interface sends without fragmentation
    pub mtu: u16,
    pub options: u8,
    /// DD_I, DD_M, DD_MS
    pub flags: u8,
    pub seq: u32,
    pub lsas: Vec<LsaHeader>,
}

/// the body of a packet
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Body {
    Hello(Hello),
    Dd(Dd),
    Lsr(Vec<LsaKey>),
    /// whole LSAs (each as long as its length field)
    Lsu(Vec<Vec<u8>>),
    Ack(Vec<LsaHeader>),
}

/// why a packet cannot be read
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OspfErr {
    /// not version 2
    Version(u8),
    /// not one of the five packet types
    Kind(u8),
    /// the body is shorter than its fixed part, or its entries do not fill it
    Body,
    /// an LSA of an update is cut or longer than the packet
    Lsa,
}

impl OspfErr {
    pub fn text(self) -> String {
        match self {
            OspfErr::Version(v) => format!("version {}, not 2", v),
            OspfErr::Kind(k) => format!("unknown packet type {}", k),
            OspfErr::Body => String::from("the body does not fit the packet type"),
            OspfErr::Lsa => String::from("an LSA does not fit into the packet"),
        }
    }
}

/// an OSPF packet as a record
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Packet {
    pub hdr: Header,
    pub body: Body,
}

/// a received OSPF packet (the bytes are cut to its length field)
#[derive(Clone, Copy, Debug)]
pub struct OspfView<'a> {
    b: &'a [u8],
}

impl<'a> OspfView<'a> {
    /// at least the header, and as many bytes as its length field says (more are ignored)
    pub fn parse(b: &'a [u8]) -> Option<OspfView<'a>> {
        if b.len() < HDR_LEN {
            return None;
        }
        let len = be16(b, 2) as usize;
        if len < HDR_LEN || len > b.len() {
            return None;
        }
        Some(OspfView { b: &b[..len] })
    }
    pub fn version(&self) -> u8 {
        self.b[0]
    }
    pub fn kind(&self) -> u8 {
        self.b[1]
    }
    pub fn len(&self) -> usize {
        self.b.len()
    }
    pub fn is_empty(&self) -> bool {
        self.b.is_empty()
    }
    pub fn router(&self) -> Ipv4 {
        Ipv4([self.b[4], self.b[5], self.b[6], self.b[7]])
    }
    pub fn area(&self) -> u32 {
        be32(self.b, 8)
    }
    pub fn checksum(&self) -> u16 {
        be16(self.b, 12)
    }
    pub fn autype(&self) -> u16 {
        be16(self.b, 14)
    }
    pub fn auth(&self) -> [u8; 8] {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[16..24]);
        a
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    pub fn body(&self) -> &'a [u8] {
        &self.b[HDR_LEN..]
    }
    /// the checksum the packet should carry
    pub fn computed_checksum(&self) -> u16 {
        packet_checksum(self.b)
    }
    /// the checksum is right (with cryptographic authentication there is none: always true)
    pub fn checksum_ok(&self) -> bool {
        self.autype() == AU_CRYPTO || self.checksum() == self.computed_checksum()
    }
    pub fn header(&self) -> Header {
        Header { kind: self.kind(), router: self.router(), area: self.area(), autype: self.autype(), auth: self.auth() }
    }
    /// where the LSAs of an update start and end within the packet: (offset, length); None when
    /// one does not fit
    pub fn lsu_spans(&self) -> Option<Vec<(usize, usize)>> {
        let b = self.body();
        if b.len() < 4 {
            return None;
        }
        let n = be32(b, 0) as usize;
        let mut v = Vec::new();
        let mut o = 4;
        for _ in 0..n {
            let h = LsaHeader::of(b.get(o..)?)?;
            let l = h.len as usize;
            if l < LSA_HDR_LEN || o + l > b.len() {
                return None;
            }
            v.push((HDR_LEN + o, l));
            o += l;
        }
        Some(v)
    }
}

fn ip_at(b: &[u8], o: usize) -> Ipv4 {
    Ipv4([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl Packet {
    pub fn kind(&self) -> u8 {
        match &self.body {
            Body::Hello(_) => T_HELLO,
            Body::Dd(_) => T_DD,
            Body::Lsr(_) => T_LSR,
            Body::Lsu(_) => T_LSU,
            Body::Ack(_) => T_ACK,
        }
    }

    /// reads a packet; the checksum and the authentication are for the receiver to judge
    pub fn read(v: &OspfView) -> Result<Packet, OspfErr> {
        if v.version() != VERSION {
            return Err(OspfErr::Version(v.version()));
        }
        let b = v.body();
        let body = match v.kind() {
            T_HELLO => {
                if b.len() < HELLO_LEN || (b.len() - HELLO_LEN) % 4 != 0 {
                    return Err(OspfErr::Body);
                }
                Body::Hello(Hello {
                    mask: ip_at(b, 0),
                    interval: be16(b, 4),
                    options: b[6],
                    priority: b[7],
                    dead: be32(b, 8),
                    dr: ip_at(b, 12),
                    bdr: ip_at(b, 16),
                    neighbors: b[HELLO_LEN..].chunks_exact(4).map(|c| Ipv4([c[0], c[1], c[2], c[3]])).collect(),
                })
            }
            T_DD => {
                if b.len() < DD_LEN || (b.len() - DD_LEN) % LSA_HDR_LEN != 0 {
                    return Err(OspfErr::Body);
                }
                Body::Dd(Dd { mtu: be16(b, 0), options: b[2], flags: b[3], seq: be32(b, 4), lsas: b[DD_LEN..].chunks_exact(LSA_HDR_LEN).map(LsaHeader::read).collect() })
            }
            T_LSR => {
                if b.len() % LSR_ENTRY_LEN != 0 {
                    return Err(OspfErr::Body);
                }
                Body::Lsr(b.chunks_exact(LSR_ENTRY_LEN).map(|c| LsaKey { kind: be32(c, 0) as u8, id: ip_at(c, 4), adv: ip_at(c, 8) }).collect())
            }
            T_LSU => {
                let spans = v.lsu_spans().ok_or(OspfErr::Lsa)?;
                Body::Lsu(spans.iter().map(|&(o, l)| v.bytes()[o..o + l].to_vec()).collect())
            }
            T_ACK => {
                if b.len() % LSA_HDR_LEN != 0 {
                    return Err(OspfErr::Body);
                }
                Body::Ack(b.chunks_exact(LSA_HDR_LEN).map(LsaHeader::read).collect())
            }
            k => return Err(OspfErr::Kind(k)),
        };
        Ok(Packet { hdr: v.header(), body })
    }

    /// the packet as bytes: version 2, the type of the body, the length, the checksum (none with
    /// AU_CRYPTO)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; HDR_LEN];
        match &self.body {
            Body::Hello(h) => {
                b.extend_from_slice(&h.mask.0);
                b.extend_from_slice(&h.interval.to_be_bytes());
                b.extend_from_slice(&[h.options, h.priority]);
                b.extend_from_slice(&h.dead.to_be_bytes());
                b.extend_from_slice(&h.dr.0);
                b.extend_from_slice(&h.bdr.0);
                for n in &h.neighbors {
                    b.extend_from_slice(&n.0);
                }
            }
            Body::Dd(d) => {
                b.extend_from_slice(&d.mtu.to_be_bytes());
                b.extend_from_slice(&[d.options, d.flags]);
                b.extend_from_slice(&d.seq.to_be_bytes());
                for h in &d.lsas {
                    let mut x = [0u8; LSA_HDR_LEN];
                    h.write(&mut x);
                    b.extend_from_slice(&x);
                }
            }
            Body::Lsr(keys) => {
                for k in keys {
                    b.extend_from_slice(&(k.kind as u32).to_be_bytes());
                    b.extend_from_slice(&k.id.0);
                    b.extend_from_slice(&k.adv.0);
                }
            }
            Body::Lsu(lsas) => {
                b.extend_from_slice(&(lsas.len() as u32).to_be_bytes());
                for l in lsas {
                    b.extend_from_slice(l);
                }
            }
            Body::Ack(hs) => {
                for h in hs {
                    let mut x = [0u8; LSA_HDR_LEN];
                    h.write(&mut x);
                    b.extend_from_slice(&x);
                }
            }
        }
        b[0] = VERSION;
        b[1] = self.kind();
        let len = b.len() as u16;
        put16(&mut b, 2, len);
        b[4..8].copy_from_slice(&self.hdr.router.0);
        put32(&mut b, 8, self.hdr.area);
        put16(&mut b, 14, self.hdr.autype);
        if self.hdr.autype != AU_CRYPTO {
            let ck = packet_checksum(&b);
            put16(&mut b, 12, ck);
        }
        b[16..24].copy_from_slice(&self.hdr.auth);
        b
    }
}
