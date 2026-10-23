// dns -- the Domain Name System message format (RFC 1035; AAAA RFC 3596; negative answers
// RFC 2308; the EDNS(0) OPT record of RFC 6891 is kept as raw data)
//
//   Header { id, qr, opcode, aa, tc, rd, ra, ad, cd, rcode }   flags() / from_flags(id, f)
//   Question { name, qtype, qclass }
//   Rr { name, rtype, class, ttl, data: RData }      Rr::new(name, ttl, data)
//   RData            A, Aaaa, Ns, Cname, Ptr, Mx { pref, host }, Soa(Soa), Txt(strings),
//                    Other(bytes); rtype() / text()
//   Message { header, questions, answers, authority, additional }
//                    parse(b) (names with compression pointers; loops, forward pointers and
//                    names over 255 bytes are refused) / to_bytes() (names compressed against
//                    earlier ones, RFC 1035 4.1.4) / to_bytes_max(limit) -> (bytes, truncated):
//                    whole records only, TC set when something was left out /
//                    query(id, name, qtype, rd) / reply_to(&query)
//   read_name(msg, off) -> Option<(String, usize)> / write_name(out, name, &mut Compressor)
//   name_eq / is_subdomain(name, zone) / parent(name) / reverse_v4(ip) / parse_reverse_v4
//   TYPE_* / CLASS_IN / RCODE_* / OPCODE_QUERY; type_name / type_by_name / rcode_name
//
// Names are text without the final dot ("www.example.com"; the root is ""); a dot or a
// backslash inside a label and bytes outside printable ASCII are written \. \\ \DDD as in
// zone files.  Comparisons ignore ASCII case (RFC 4343).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{Ipv4, Ipv6};
use crate::{be16, be32, put16};

pub const DNS_HDR: usize = 12;
/// the largest answer over UDP without EDNS (RFC 1035 4.2.1)
pub const UDP_MAX: usize = 512;

pub const TYPE_A: u16 = 1;
pub const TYPE_NS: u16 = 2;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_SOA: u16 = 6;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_MX: u16 = 15;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;
pub const TYPE_OPT: u16 = 41;
pub const TYPE_ANY: u16 = 255;
pub const CLASS_IN: u16 = 1;

pub const OPCODE_QUERY: u8 = 0;
pub const RCODE_NOERROR: u8 = 0;
pub const RCODE_FORMERR: u8 = 1;
pub const RCODE_SERVFAIL: u8 = 2;
pub const RCODE_NXDOMAIN: u8 = 3;
pub const RCODE_NOTIMP: u8 = 4;
pub const RCODE_REFUSED: u8 = 5;

/// "A", "MX", ... ("TYPE99" for others, RFC 3597)
pub fn type_name(t: u16) -> String {
    let s = match t {
        TYPE_A => "A",
        TYPE_NS => "NS",
        TYPE_CNAME => "CNAME",
        TYPE_SOA => "SOA",
        TYPE_PTR => "PTR",
        TYPE_MX => "MX",
        TYPE_TXT => "TXT",
        TYPE_AAAA => "AAAA",
        TYPE_SRV => "SRV",
        TYPE_OPT => "OPT",
        TYPE_ANY => "ANY",
        _ => return format!("TYPE{}", t),
    };
    String::from(s)
}

/// the type of a mnemonic ("a", "MX", "TYPE99")
pub fn type_by_name(s: &str) -> Option<u16> {
    let u = s.to_ascii_uppercase();
    Some(match u.as_str() {
        "A" => TYPE_A,
        "NS" => TYPE_NS,
        "CNAME" => TYPE_CNAME,
        "SOA" => TYPE_SOA,
        "PTR" => TYPE_PTR,
        "MX" => TYPE_MX,
        "TXT" => TYPE_TXT,
        "AAAA" => TYPE_AAAA,
        "SRV" => TYPE_SRV,
        "OPT" => TYPE_OPT,
        "ANY" => TYPE_ANY,
        _ => return u.strip_prefix("TYPE").and_then(|n| n.parse().ok()),
    })
}

/// "NOERROR", "NXDOMAIN", ...
pub fn rcode_name(r: u8) -> &'static str {
    match r {
        RCODE_NOERROR => "NOERROR",
        RCODE_FORMERR => "FORMERR",
        RCODE_SERVFAIL => "SERVFAIL",
        RCODE_NXDOMAIN => "NXDOMAIN",
        RCODE_NOTIMP => "NOTIMP",
        RCODE_REFUSED => "REFUSED",
        _ => "RCODE?",
    }
}

/// why a message could not be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsErr {
    /// it ends inside a header, a name or a record
    Short,
    /// a label, a pointer or the length of a name is wrong
    BadName,
    /// the data of a record does not fit its type or length
    BadRecord,
}

/// the fixed header
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub id: u16,
    /// a response
    pub qr: bool,
    pub opcode: u8,
    /// authoritative answer
    pub aa: bool,
    /// truncated
    pub tc: bool,
    /// recursion desired
    pub rd: bool,
    /// recursion available
    pub ra: bool,
    /// authentic data (DNSSEC)
    pub ad: bool,
    /// checking disabled (DNSSEC)
    pub cd: bool,
    pub rcode: u8,
}

impl Header {
    /// the 16 flag bits
    pub fn flags(&self) -> u16 {
        (self.qr as u16) << 15
            | ((self.opcode & 15) as u16) << 11
            | (self.aa as u16) << 10
            | (self.tc as u16) << 9
            | (self.rd as u16) << 8
            | (self.ra as u16) << 7
            | (self.ad as u16) << 5
            | (self.cd as u16) << 4
            | (self.rcode & 15) as u16
    }
    pub fn from_flags(id: u16, f: u16) -> Header {
        Header {
            id,
            qr: f & 0x8000 != 0,
            opcode: ((f >> 11) & 15) as u8,
            aa: f & 0x0400 != 0,
            tc: f & 0x0200 != 0,
            rd: f & 0x0100 != 0,
            ra: f & 0x0080 != 0,
            ad: f & 0x0020 != 0,
            cd: f & 0x0010 != 0,
            rcode: (f & 15) as u8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

/// the data of a start of authority record
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Soa {
    /// the primary name server
    pub mname: String,
    /// the mailbox of the responsible person (the first dot is the @)
    pub rname: String,
    pub serial: u32,
    pub refresh: u32,
    pub retry: u32,
    pub expire: u32,
    /// the TTL of negative answers (RFC 2308)
    pub minimum: u32,
}

/// the data of a resource record
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    A(Ipv4),
    Aaaa(Ipv6),
    Ns(String),
    Cname(String),
    Ptr(String),
    Mx { pref: u16, host: String },
    Soa(Soa),
    /// character strings of up to 255 bytes each
    Txt(Vec<Vec<u8>>),
    /// any other type, as it was on the wire
    Other(Vec<u8>),
}

impl RData {
    /// the record type of the data (None for Other)
    pub fn rtype(&self) -> Option<u16> {
        Some(match self {
            RData::A(_) => TYPE_A,
            RData::Aaaa(_) => TYPE_AAAA,
            RData::Ns(_) => TYPE_NS,
            RData::Cname(_) => TYPE_CNAME,
            RData::Ptr(_) => TYPE_PTR,
            RData::Mx { .. } => TYPE_MX,
            RData::Soa(_) => TYPE_SOA,
            RData::Txt(_) => TYPE_TXT,
            RData::Other(_) => return None,
        })
    }
    /// the data in zone-file form ("10.0.0.1", "10 mail.example.com", "\"text\"")
    pub fn text(&self) -> String {
        match self {
            RData::A(a) => format!("{}", a),
            RData::Aaaa(a) => format!("{}", a),
            RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) => n.clone(),
            RData::Mx { pref, host } => format!("{} {}", pref, host),
            RData::Soa(s) => format!("{} {} {} {} {} {} {}", s.mname, s.rname, s.serial, s.refresh, s.retry, s.expire, s.minimum),
            RData::Txt(v) => {
                let parts: Vec<String> = v
                    .iter()
                    .map(|s| {
                        let mut t = String::from("\"");
                        for &c in s {
                            match c {
                                b'"' | b'\\' => {
                                    t.push('\\');
                                    t.push(c as char);
                                }
                                0x20..=0x7E => t.push(c as char),
                                _ => t.push_str(&format!("\\{:03}", c)),
                            }
                        }
                        t.push('"');
                        t
                    })
                    .collect();
                parts.join(" ")
            }
            RData::Other(b) => {
                let mut t = format!("\\# {}", b.len());
                if !b.is_empty() {
                    t.push(' ');
                    for c in b {
                        t.push_str(&format!("{:02x}", c));
                    }
                }
                t
            }
        }
    }
}

/// a resource record
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rr {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

impl Rr {
    /// a record of class IN; the type follows the data
    pub fn new(name: &str, ttl: u32, data: RData) -> Rr {
        Rr { name: String::from(name), rtype: data.rtype().unwrap_or(0), class: CLASS_IN, ttl, data }
    }
}

/// a whole message
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub questions: Vec<Question>,
    pub answers: Vec<Rr>,
    pub authority: Vec<Rr>,
    pub additional: Vec<Rr>,
}

// ---------------------------------------------------------------- names

/// the labels of a name in text form (None: empty label, label over 63 bytes, name over 255
/// bytes on the wire, bad escape)
pub fn labels(name: &str) -> Option<Vec<Vec<u8>>> {
    let b = name.as_bytes();
    let mut out: Vec<Vec<u8>> = Vec::new();
    if b.is_empty() || b == b"." {
        return Some(out);
    }
    let mut cur: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' {
            let d = b.get(i + 1..i + 4);
            match d {
                Some(d) if d.iter().all(|x| x.is_ascii_digit()) => {
                    let v = (d[0] - b'0') as u32 * 100 + (d[1] - b'0') as u32 * 10 + (d[2] - b'0') as u32;
                    if v > 255 {
                        return None;
                    }
                    cur.push(v as u8);
                    i += 4;
                }
                _ => {
                    cur.push(*b.get(i + 1)?);
                    i += 2;
                }
            }
            continue;
        }
        if c == b'.' {
            if cur.is_empty() {
                return None;
            }
            out.push(core::mem::take(&mut cur));
            i += 1;
            // one final dot is the root
            if i == b.len() {
                break;
            }
            continue;
        }
        cur.push(c);
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    } else if !b.ends_with(b".") {
        return None;
    }
    if out.iter().any(|l| l.len() > 63) || out.iter().map(|l| l.len() + 1).sum::<usize>() + 1 > 255 {
        return None;
    }
    Some(out)
}

fn push_label_text(s: &mut String, l: &[u8]) {
    for &c in l {
        match c {
            b'.' | b'\\' => {
                s.push('\\');
                s.push(c as char);
            }
            0x21..=0x7E => s.push(c as char),
            _ => s.push_str(&format!("\\{:03}", c)),
        }
    }
}

/// the name at `off` of a message and the offset after it (where the name ends in place, not
/// where a pointer led)
pub fn read_name(msg: &[u8], off: usize) -> Option<(String, usize)> {
    let mut s = String::new();
    let mut p = off;
    let mut end = None;
    let mut wire = 1usize;
    loop {
        let l = *msg.get(p)? as usize;
        match l & 0xC0 {
            0x00 => {
                if l == 0 {
                    return Some((s, end.unwrap_or(p + 1)));
                }
                let lab = msg.get(p + 1..p + 1 + l)?;
                wire += l + 1;
                if wire > 255 {
                    return None;
                }
                if !s.is_empty() {
                    s.push('.');
                }
                push_label_text(&mut s, lab);
                p += 1 + l;
            }
            0xC0 => {
                let q = ((l & 0x3F) << 8) | *msg.get(p + 1)? as usize;
                // only backwards: no loops
                if q >= p {
                    return None;
                }
                if end.is_none() {
                    end = Some(p + 2);
                }
                p = q;
            }
            _ => return None,
        }
    }
}

/// remembers where names were written so later names can point there (RFC 1035 4.1.4)
#[derive(Clone, Debug, Default)]
pub struct Compressor {
    at: BTreeMap<Vec<u8>, u16>,
    /// off: every name is written in full
    pub off: bool,
}

impl Compressor {
    pub fn new() -> Compressor {
        Compressor::default()
    }
    /// forgets the names written at `off` or later (the output was cut back there)
    pub fn forget_from(&mut self, off: usize) {
        self.at.retain(|_, a| (*a as usize) < off);
    }
}

/// the wire form of labels[i..], lowercased: the key of a suffix
fn suffix_key(ls: &[Vec<u8>]) -> Vec<u8> {
    let mut k = Vec::new();
    for l in ls {
        k.push(l.len() as u8);
        k.extend(l.iter().map(|c| c.to_ascii_lowercase()));
    }
    k
}

/// appends a name in wire form (with pointers to earlier names when the compressor allows);
/// false when the name is not valid
pub fn write_name(out: &mut Vec<u8>, name: &str, c: &mut Compressor) -> bool {
    let Some(ls) = labels(name) else {
        return false;
    };
    for i in 0..ls.len() {
        if !c.off {
            let key = suffix_key(&ls[i..]);
            if let Some(&at) = c.at.get(&key) {
                let p = 0xC000 | at;
                out.extend_from_slice(&p.to_be_bytes());
                return true;
            }
            if out.len() < 0x4000 {
                c.at.insert(key, out.len() as u16);
            }
        }
        out.push(ls[i].len() as u8);
        out.extend_from_slice(&ls[i]);
    }
    out.push(0);
    true
}

/// two names are the same (ASCII case ignored)
pub fn name_eq(a: &str, b: &str) -> bool {
    a.trim_end_matches('.').eq_ignore_ascii_case(b.trim_end_matches('.'))
}

/// `name` is `zone` or lies below it ("www.lab" in "lab"; every name lies in the root "")
pub fn is_subdomain(name: &str, zone: &str) -> bool {
    let (n, z) = (name.trim_end_matches('.'), zone.trim_end_matches('.'));
    if z.is_empty() || n.eq_ignore_ascii_case(z) {
        return true;
    }
    let (nb, zb) = (n.as_bytes(), z.as_bytes());
    nb.len() > zb.len() + 1 && nb[nb.len() - zb.len() - 1] == b'.' && nb[nb.len() - zb.len()..].eq_ignore_ascii_case(zb)
}

/// the name without its first label ("lab" for "www.lab", "" for "lab")
pub fn parent(name: &str) -> &str {
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'.' => return &name[i + 1..],
            _ => i += 1,
        }
    }
    ""
}

/// the PTR name of an address ("4.3.2.1.in-addr.arpa")
pub fn reverse_v4(ip: Ipv4) -> String {
    format!("{}.{}.{}.{}.in-addr.arpa", ip.0[3], ip.0[2], ip.0[1], ip.0[0])
}

/// the address of a PTR name ("4.3.2.1.in-addr.arpa" -> 1.2.3.4)
pub fn parse_reverse_v4(name: &str) -> Option<Ipv4> {
    let n = name.trim_end_matches('.');
    let lower = n.to_ascii_lowercase();
    let head = lower.strip_suffix(".in-addr.arpa")?;
    let parts: Vec<&str> = head.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut a = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        a[3 - i] = p.parse().ok()?;
    }
    Some(Ipv4(a))
}

// ---------------------------------------------------------------- reading

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn u16(&mut self) -> Result<u16, DnsErr> {
        if self.p + 2 > self.b.len() {
            return Err(DnsErr::Short);
        }
        let v = be16(self.b, self.p);
        self.p += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, DnsErr> {
        if self.p + 4 > self.b.len() {
            return Err(DnsErr::Short);
        }
        let v = be32(self.b, self.p);
        self.p += 4;
        Ok(v)
    }
    fn name(&mut self) -> Result<String, DnsErr> {
        if self.p >= self.b.len() {
            return Err(DnsErr::Short);
        }
        let (n, e) = read_name(self.b, self.p).ok_or(DnsErr::BadName)?;
        self.p = e;
        Ok(n)
    }
    fn rr(&mut self) -> Result<Rr, DnsErr> {
        let name = self.name()?;
        let rtype = self.u16()?;
        let class = self.u16()?;
        let ttl = self.u32()?;
        let len = self.u16()? as usize;
        let start = self.p;
        let end = start + len;
        if end > self.b.len() {
            return Err(DnsErr::Short);
        }
        let d = &self.b[start..end];
        let data = match rtype {
            TYPE_A if len == 4 => RData::A(Ipv4([d[0], d[1], d[2], d[3]])),
            TYPE_AAAA if len == 16 => RData::Aaaa(Ipv6(d.try_into().unwrap_or([0; 16]))),
            TYPE_A | TYPE_AAAA => return Err(DnsErr::BadRecord),
            TYPE_NS | TYPE_CNAME | TYPE_PTR => {
                let n = self.name()?;
                match rtype {
                    TYPE_NS => RData::Ns(n),
                    TYPE_CNAME => RData::Cname(n),
                    _ => RData::Ptr(n),
                }
            }
            TYPE_MX => {
                let pref = self.u16()?;
                RData::Mx { pref, host: self.name()? }
            }
            TYPE_SOA => {
                let mname = self.name()?;
                let rname = self.name()?;
                let (serial, refresh, retry, expire, minimum) = (self.u32()?, self.u32()?, self.u32()?, self.u32()?, self.u32()?);
                RData::Soa(Soa { mname, rname, serial, refresh, retry, expire, minimum })
            }
            TYPE_TXT => {
                let mut v = Vec::new();
                let mut q = 0;
                while q < d.len() {
                    let l = d[q] as usize;
                    if q + 1 + l > d.len() {
                        return Err(DnsErr::BadRecord);
                    }
                    v.push(d[q + 1..q + 1 + l].to_vec());
                    q += 1 + l;
                }
                RData::Txt(v)
            }
            _ => RData::Other(d.to_vec()),
        };
        // names inside the data must end exactly at its length
        if matches!(rtype, TYPE_NS | TYPE_CNAME | TYPE_PTR | TYPE_MX | TYPE_SOA) && self.p != end {
            return Err(DnsErr::BadRecord);
        }
        self.p = end;
        Ok(Rr { name, rtype, class, ttl, data })
    }
}

// ---------------------------------------------------------------- writing

fn write_rr(out: &mut Vec<u8>, r: &Rr, c: &mut Compressor) -> bool {
    if !write_name(out, &r.name, c) {
        return false;
    }
    out.extend_from_slice(&r.rtype.to_be_bytes());
    out.extend_from_slice(&r.class.to_be_bytes());
    out.extend_from_slice(&r.ttl.to_be_bytes());
    let lenpos = out.len();
    out.extend_from_slice(&[0, 0]);
    let ok = match &r.data {
        RData::A(a) => {
            out.extend_from_slice(&a.0);
            true
        }
        RData::Aaaa(a) => {
            out.extend_from_slice(&a.0);
            true
        }
        RData::Ns(n) | RData::Cname(n) | RData::Ptr(n) => write_name(out, n, c),
        RData::Mx { pref, host } => {
            out.extend_from_slice(&pref.to_be_bytes());
            write_name(out, host, c)
        }
        RData::Soa(s) => {
            let ok = write_name(out, &s.mname, c) && write_name(out, &s.rname, c);
            for v in [s.serial, s.refresh, s.retry, s.expire, s.minimum] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            ok
        }
        RData::Txt(v) => {
            for s in v {
                let l = s.len().min(255);
                out.push(l as u8);
                out.extend_from_slice(&s[..l]);
            }
            true
        }
        RData::Other(b) => {
            out.extend_from_slice(b);
            true
        }
    };
    let len = out.len() - lenpos - 2;
    if len > 0xFFFF {
        return false;
    }
    put16(out, lenpos, len as u16);
    ok
}

impl Message {
    /// a query for one name
    pub fn query(id: u16, name: &str, qtype: u16, rd: bool) -> Message {
        Message {
            header: Header { id, rd, ..Header::default() },
            questions: alloc::vec![Question { name: String::from(name), qtype, qclass: CLASS_IN }],
            ..Message::default()
        }
    }
    /// an empty response to a query: same id, opcode, question and recursion wish
    pub fn reply_to(q: &Message) -> Message {
        Message {
            header: Header { id: q.header.id, qr: true, opcode: q.header.opcode, rd: q.header.rd, cd: q.header.cd, ..Header::default() },
            questions: q.questions.clone(),
            ..Message::default()
        }
    }

    pub fn parse(b: &[u8]) -> Result<Message, DnsErr> {
        if b.len() < DNS_HDR {
            return Err(DnsErr::Short);
        }
        let header = Header::from_flags(be16(b, 0), be16(b, 2));
        let (qd, an, ns, ar) = (be16(b, 4), be16(b, 6), be16(b, 8), be16(b, 10));
        let mut r = Rd { b, p: DNS_HDR };
        let mut questions = Vec::new();
        for _ in 0..qd {
            let name = r.name()?;
            let qtype = r.u16()?;
            let qclass = r.u16()?;
            questions.push(Question { name, qtype, qclass });
        }
        let mut sec = |n: u16| -> Result<Vec<Rr>, DnsErr> { (0..n).map(|_| r.rr()).collect() };
        let answers = sec(an)?;
        let authority = sec(ns)?;
        let additional = sec(ar)?;
        Ok(Message { header, questions, answers, authority, additional })
    }

    /// the message in wire form, names compressed
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_max(usize::MAX).0
    }

    /// at most `limit` bytes: records that do not fit are left out whole and TC is set
    /// (RFC 2181 9: true when something was left out); a record with an invalid name is
    /// skipped
    pub fn to_bytes_max(&self, limit: usize) -> (Vec<u8>, bool) {
        let mut out = alloc::vec![0u8; DNS_HDR];
        let mut c = Compressor::new();
        let mut counts = [0u16; 4];
        let mut cut = false;
        for q in &self.questions {
            if counts[0] == u16::MAX {
                cut = true;
                break;
            }
            let mark = out.len();
            let ok = write_name(&mut out, &q.name, &mut c);
            out.extend_from_slice(&q.qtype.to_be_bytes());
            out.extend_from_slice(&q.qclass.to_be_bytes());
            if !ok || out.len() > limit {
                out.truncate(mark);
                c.forget_from(mark);
                if ok {
                    cut = true;
                    break;
                }
                continue;
            }
            counts[0] += 1;
        }
        for (i, sec) in [&self.answers, &self.authority, &self.additional].into_iter().enumerate() {
            if cut {
                break;
            }
            for r in sec {
                if counts[i + 1] == u16::MAX {
                    cut = true;
                    break;
                }
                let mark = out.len();
                let ok = write_rr(&mut out, r, &mut c);
                if !ok || out.len() > limit {
                    out.truncate(mark);
                    c.forget_from(mark);
                    if ok {
                        cut = true;
                        break;
                    }
                    continue;
                }
                counts[i + 1] += 1;
            }
        }
        let mut h = self.header;
        h.tc |= cut;
        put16(&mut out, 0, h.id);
        put16(&mut out, 2, h.flags());
        for (i, n) in counts.iter().enumerate() {
            put16(&mut out, 4 + 2 * i, *n);
        }
        (out, cut)
    }
}
