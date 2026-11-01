// x509 -- certificates of a small private PKI without I/O and without a clock (RFC 5280; Ed25519,
// RFC 8410): written and read as DER and checked against anchors at a time the caller passes in.
// netio::x509 is the other X.509 of the library, with the web's RSA and ECDSA keys, on a crate that
// needs plat
//
//   CertSpec { serial, cn, org, not_before, not_after, public_key, is_ca, dns, ips }
//   spec.sign(issuer: Option<&Cert>, key) -> DER      issuer None: self-signed with its own name
//   Cert::parse(der) -> Option<Cert>                  subject and issuer (common name, organization),
//                                                     validity, the Ed25519 key, the CA flag, names;
//                                                     other keys parse with key None (they verify nothing)
//   cert.names(host)                                  a DNS name (case-insensitive, one wildcard
//                                                     label) or an address of its alternative names
//   cert.fingerprint() / fingerprint_text()           SHA-256 of the DER; in hex with colons
//   cert.signed_by(&issuer), cert.self_signed()       the signature checks out
//   check_chain(&[leaf, ...], anchors, host, now) -> faults   what is wrong with it: F_* bits, 0 for
//                                                     nothing
//
// Not here: revocation, name constraints, key usage checks beyond the CA flag, certificate policies.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ed25519::{self, Keypair};

use crate::{IpAddr, Ipv4, Ipv6};

/// the time of a certificate has passed
pub const F_EXPIRED: u32 = 1 << 0;
/// its time has not come yet
pub const F_NOT_YET: u32 = 1 << 1;
/// the name asked for is not among its names
pub const F_NAME: u32 = 1 << 2;
/// signed by itself, and not an anchor
pub const F_SELF_SIGNED: u32 = 1 << 3;
/// no path to an anchor
pub const F_UNKNOWN_ISSUER: u32 = 1 << 4;
/// a signature that does not check out, or a key this code cannot read
pub const F_BAD_SIGNATURE: u32 = 1 << 5;

const OID_ED25519: &[u8] = &[0x2B, 101, 112];
const OID_CN: &[u8] = &[0x55, 4, 3];
const OID_O: &[u8] = &[0x55, 4, 10];
const OID_BASIC: &[u8] = &[0x55, 29, 19];
const OID_KEY_USAGE: &[u8] = &[0x55, 29, 15];
const OID_EKU: &[u8] = &[0x55, 29, 37];
const OID_SAN: &[u8] = &[0x55, 29, 17];
const OID_SKI: &[u8] = &[0x55, 29, 14];
const OID_AKI: &[u8] = &[0x55, 29, 35];
// id-kp-serverAuth
const OID_SERVER_AUTH: &[u8] = &[0x2B, 6, 1, 5, 5, 7, 3, 1];

const T_BOOL: u8 = 0x01;
const T_INT: u8 = 0x02;
const T_BITS: u8 = 0x03;
const T_OCTETS: u8 = 0x04;
const T_OID: u8 = 0x06;
const T_UTF8: u8 = 0x0C;
const T_UTC: u8 = 0x17;
const T_GEN: u8 = 0x18;
const T_SEQ: u8 = 0x30;
const T_SET: u8 = 0x31;

// ---------------------------------------------------------------- DER

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 5);
    out.push(tag);
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend_from_slice(&[0x81, n as u8]);
    } else if n < 0x10000 {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    } else {
        out.extend_from_slice(&[0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(T_SEQ, &parts.concat())
}

fn bits(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len() + 1);
    v.push(0);
    v.extend_from_slice(b);
    tlv(T_BITS, &v)
}

// a cursor over DER: definite lengths only, as short as they can be
#[derive(Clone, Copy)]
struct Der<'a> {
    b: &'a [u8],
}

impl<'a> Der<'a> {
    fn new(b: &'a [u8]) -> Der<'a> {
        Der { b }
    }
    fn is_empty(&self) -> bool {
        self.b.is_empty()
    }
    fn peek(&self) -> Option<u8> {
        self.b.first().copied()
    }
    // the next element: its tag, its content, the whole of it
    fn next(&mut self) -> Option<(u8, &'a [u8], &'a [u8])> {
        let tag = *self.b.first()?;
        let first = *self.b.get(1)? as usize;
        let (len, head) = if first < 0x80 {
            (first, 2)
        } else {
            let k = first & 0x7F;
            if k == 0 || k > 3 || self.b.len() < 2 + k {
                return None;
            }
            let mut n = 0usize;
            for &c in &self.b[2..2 + k] {
                n = n << 8 | c as usize;
            }
            if n < 0x80 || (k > 1 && n < 1 << (8 * (k - 1))) {
                return None;
            }
            (n, 2 + k)
        };
        let end = head.checked_add(len)?;
        if self.b.len() < end {
            return None;
        }
        let (whole, rest) = self.b.split_at(end);
        self.b = rest;
        Some((tag, &whole[head..], whole))
    }
    fn read(&mut self, tag: u8) -> Option<&'a [u8]> {
        let mut c = *self;
        let (t, v, _) = c.next()?;
        if t != tag {
            return None;
        }
        *self = c;
        Some(v)
    }
    fn raw(&mut self, tag: u8) -> Option<&'a [u8]> {
        let mut c = *self;
        let (t, _, w) = c.next()?;
        if t != tag {
            return None;
        }
        *self = c;
        Some(w)
    }
    fn seq(&mut self) -> Option<Der<'a>> {
        self.read(T_SEQ).map(Der::new)
    }
    fn bits(&mut self) -> Option<&'a [u8]> {
        let c = self.read(T_BITS)?;
        (c.first() == Some(&0)).then(|| &c[1..])
    }
}

// ---------------------------------------------------------------- times

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn digits(b: &[u8]) -> Option<i64> {
    let mut n = 0i64;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (c - b'0') as i64;
    }
    Some(n)
}

fn parse_time(tag: u8, c: &[u8]) -> Option<i64> {
    let (y, rest) = match tag {
        T_UTC if c.len() == 13 => {
            let yy = digits(&c[..2])?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &c[2..])
        }
        T_GEN if c.len() == 15 => (digits(&c[..4])?, &c[4..]),
        _ => return None,
    };
    if rest[10] != b'Z' {
        return None;
    }
    let (mo, d, h, mi, s) = (digits(&rest[0..2])?, digits(&rest[2..4])?, digits(&rest[4..6])?, digits(&rest[6..8])?, digits(&rest[8..10])?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dim = match mo {
        2 => 28 + leap as i64,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&mo) || !(1..=dim).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

// RFC 5280 4.1.2.5: UTCTime up to 2049, GeneralizedTime from 2050 on
fn encode_time(t: i64) -> Vec<u8> {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    let (h, mi, se) = (s / 3600, s / 60 % 60, s % 60);
    if (1950..2050).contains(&y) {
        tlv(T_UTC, format!("{:02}{:02}{:02}{:02}{:02}{:02}Z", y % 100, m, d, h, mi, se).as_bytes())
    } else {
        tlv(T_GEN, format!("{:04}{:02}{:02}{:02}{:02}{:02}Z", y, m, d, h, mi, se).as_bytes())
    }
}

// ---------------------------------------------------------------- writing

fn name_der(cn: &str, org: &str) -> Vec<u8> {
    let attr = |oid: &[u8], v: &str| tlv(T_SET, &seq(&[&tlv(T_OID, oid), &tlv(T_UTF8, v.as_bytes())]));
    let mut parts = Vec::new();
    if !org.is_empty() {
        parts.extend_from_slice(&attr(OID_O, org));
    }
    parts.extend_from_slice(&attr(OID_CN, cn));
    tlv(T_SEQ, &parts)
}

fn ext(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    if critical {
        seq(&[&tlv(T_OID, oid), &tlv(T_BOOL, &[0xFF]), &tlv(T_OCTETS, value)])
    } else {
        seq(&[&tlv(T_OID, oid), &tlv(T_OCTETS, value)])
    }
}

/// what a new certificate says
#[derive(Clone, Debug, Default)]
pub struct CertSpec {
    pub serial: [u8; 16],
    pub cn: String,
    pub org: String,
    /// seconds since 1970
    pub not_before: i64,
    pub not_after: i64,
    pub public_key: [u8; 32],
    pub is_ca: bool,
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
}

impl CertSpec {
    /// the DER certificate, signed by `key`; `issuer`: the certificate that issues it (None:
    /// self-signed with its own name)
    pub fn sign(&self, issuer: Option<&Cert>, key: &Keypair) -> Vec<u8> {
        let alg = seq(&[&tlv(T_OID, OID_ED25519)]);
        let mut serial = self.serial;
        serial[0] = (serial[0] & 0x7F) | 0x40; // positive, and no leading zero to strip
        let subject = name_der(&self.cn, &self.org);
        let issuer_name = issuer.map_or(subject.clone(), |i| i.subject_der.clone());
        let validity = seq(&[&encode_time(self.not_before), &encode_time(self.not_after)]);
        let spki = seq(&[&alg, &bits(&self.public_key)]);
        let ski = crypto::sha256(&self.public_key);
        let mut exts: Vec<Vec<u8>> = Vec::new();
        if self.is_ca {
            exts.push(ext(OID_BASIC, true, &seq(&[&tlv(T_BOOL, &[0xFF]), &tlv(T_INT, &[0])])));
            exts.push(ext(OID_KEY_USAGE, true, &tlv(T_BITS, &[1, 0x06])));
        } else {
            exts.push(ext(OID_BASIC, true, &seq(&[])));
            exts.push(ext(OID_KEY_USAGE, true, &tlv(T_BITS, &[7, 0x80])));
            exts.push(ext(OID_EKU, false, &seq(&[&tlv(T_OID, OID_SERVER_AUTH)])));
        }
        if !self.dns.is_empty() || !self.ips.is_empty() {
            let mut names = Vec::new();
            for d in &self.dns {
                names.extend_from_slice(&tlv(0x82, d.as_bytes()));
            }
            for ip in &self.ips {
                match ip {
                    IpAddr::V4(a) => names.extend_from_slice(&tlv(0x87, &a.0)),
                    IpAddr::V6(a) => names.extend_from_slice(&tlv(0x87, &a.0)),
                }
            }
            exts.push(ext(OID_SAN, false, &tlv(T_SEQ, &names)));
        }
        exts.push(ext(OID_SKI, false, &tlv(T_OCTETS, &ski[..20])));
        let aki = issuer.and_then(|i| i.key).map_or(ski, |k| crypto::sha256(&k));
        exts.push(ext(OID_AKI, false, &seq(&[&tlv(0x80, &aki[..20])])));
        let ext_all = tlv(T_SEQ, &exts.concat());
        let tbs = seq(&[&tlv(0xA0, &tlv(T_INT, &[2])), &tlv(T_INT, &serial), &alg, &issuer_name, &validity, &subject, &spki, &tlv(0xA3, &ext_all)]);
        let sig = key.sign(&tbs);
        seq(&[&tbs, &alg, &bits(&sig)])
    }
}

// ---------------------------------------------------------------- reading

// common name and organization of a Name
fn name_parts(raw: &[u8]) -> Option<(String, String)> {
    let mut d = Der::new(raw).seq()?;
    let (mut cn, mut org) = (String::new(), String::new());
    while !d.is_empty() {
        let mut set = Der::new(d.read(T_SET)?);
        while !set.is_empty() {
            let mut atv = set.seq()?;
            let oid = atv.read(T_OID)?;
            let (_, v, _) = atv.next()?;
            let text = String::from_utf8_lossy(v).into_owned();
            if oid == OID_CN {
                cn = text;
            } else if oid == OID_O {
                org = text;
            }
        }
    }
    Some((cn, org))
}

/// a certificate as this code reads it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cert {
    pub der: Vec<u8>,
    tbs: Vec<u8>,
    signature: Vec<u8>,
    ed25519_signed: bool,
    subject_der: Vec<u8>,
    issuer_der: Vec<u8>,
    pub serial: Vec<u8>,
    pub subject_cn: String,
    pub subject_org: String,
    pub issuer_cn: String,
    pub issuer_org: String,
    /// seconds since 1970
    pub not_before: i64,
    pub not_after: i64,
    /// its Ed25519 key (None: another kind of key)
    pub key: Option<[u8; 32]>,
    pub is_ca: bool,
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
}

impl Cert {
    /// a certificate from DER; None: not one (or trailing bytes, or a version before v3)
    pub fn parse(bytes: &[u8]) -> Option<Cert> {
        let mut top = Der::new(bytes);
        let mut cert = top.seq()?;
        if !top.is_empty() {
            return None;
        }
        let tbs_raw = cert.raw(T_SEQ)?;
        let alg = cert.read(T_SEQ)?;
        let sig = cert.bits()?;
        if !cert.is_empty() {
            return None;
        }
        let mut t = Der::new(tbs_raw).seq()?;
        let is_ed = |a: &[u8]| {
            let mut d = Der::new(a);
            d.read(T_OID) == Some(OID_ED25519) && d.is_empty()
        };
        let mut c = Cert { der: Vec::from(bytes), tbs: Vec::from(tbs_raw), signature: Vec::from(sig), ed25519_signed: is_ed(alg), ..Cert::default() };
        let ver = t.read(0xA0)?;
        if Der::new(ver).read(T_INT)? != [2] {
            return None;
        }
        c.serial = Vec::from(t.read(T_INT)?);
        if t.read(T_SEQ)? != alg {
            return None;
        }
        c.issuer_der = Vec::from(t.raw(T_SEQ)?);
        (c.issuer_cn, c.issuer_org) = name_parts(&c.issuer_der)?;
        let mut val = t.seq()?;
        let (tag, nb, _) = val.next()?;
        c.not_before = parse_time(tag, nb)?;
        let (tag, na, _) = val.next()?;
        c.not_after = parse_time(tag, na)?;
        c.subject_der = Vec::from(t.raw(T_SEQ)?);
        (c.subject_cn, c.subject_org) = name_parts(&c.subject_der)?;
        let mut spki = t.seq()?;
        let kalg = spki.read(T_SEQ)?;
        let key = spki.bits()?;
        if is_ed(kalg) && key.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            c.key = Some(k);
        }
        if t.peek() == Some(0xA3) {
            let e = t.read(0xA3)?;
            let mut list = Der::new(e).seq()?;
            while !list.is_empty() {
                let mut x = list.seq()?;
                let id = x.read(T_OID)?;
                if x.peek() == Some(T_BOOL) {
                    x.read(T_BOOL)?;
                }
                let val = x.read(T_OCTETS)?;
                if id == OID_BASIC {
                    let mut b = Der::new(val).seq()?;
                    if b.peek() == Some(T_BOOL) {
                        c.is_ca = b.read(T_BOOL)?.first().is_some_and(|v| *v != 0);
                    }
                } else if id == OID_SAN {
                    let mut k = Der::new(val).seq()?;
                    while let Some((tag, v, _)) = k.next() {
                        match tag {
                            0x82 => c.dns.push(String::from_utf8_lossy(v).to_ascii_lowercase()),
                            0x87 if v.len() == 4 => c.ips.push(IpAddr::V4(Ipv4([v[0], v[1], v[2], v[3]]))),
                            0x87 if v.len() == 16 => {
                                let mut a = [0u8; 16];
                                a.copy_from_slice(v);
                                c.ips.push(IpAddr::V6(Ipv6(a)));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if !t.is_empty() {
            return None;
        }
        Some(c)
    }

    /// SHA-256 of the DER
    pub fn fingerprint(&self) -> [u8; 32] {
        crypto::sha256(&self.der)
    }

    /// the fingerprint in hex with colons, as certificate dialogs write it
    pub fn fingerprint_text(&self) -> String {
        let v: Vec<String> = self.fingerprint().iter().map(|b| format!("{:02X}", b)).collect();
        v.join(":")
    }

    /// issuer's key made the signature (and the names fit)
    pub fn signed_by(&self, issuer: &Cert) -> bool {
        if self.issuer_der != issuer.subject_der || !self.ed25519_signed {
            return false;
        }
        let (Some(k), Ok(sig)) = (issuer.key, <[u8; 64]>::try_from(self.signature.as_slice())) else { return false };
        ed25519::verify(&k, &self.tbs, &sig)
    }

    /// it says it issued itself, and the signature checks out
    pub fn self_signed(&self) -> bool {
        self.signed_by(self)
    }

    /// it names this host: a DNS name (one wildcard label at the left), or an address; only its
    /// alternative names count (RFC 9110 4.3.4), the common name does not
    pub fn names(&self, host: &str) -> bool {
        if let Some(ip) = IpAddr::parse(host.trim_matches(['[', ']'])) {
            return self.ips.contains(&ip);
        }
        let h = host.trim_end_matches('.').to_ascii_lowercase();
        self.dns.iter().any(|d| *d == h || d.strip_prefix("*.").is_some_and(|base| base.contains('.') && h.split_once('.').is_some_and(|(label, rest)| !label.is_empty() && rest == base)))
    }

    /// what the names are, for a dialog
    pub fn name_list(&self) -> Vec<String> {
        let mut v = self.dns.clone();
        for ip in &self.ips {
            v.push(format!("{}", ip));
        }
        v
    }

    /// bytes it holds (an estimate for a budget)
    pub fn bytes(&self) -> usize {
        self.der.len() + self.tbs.len() + self.subject_der.len() + self.issuer_der.len() + self.signature.len() + 128
    }
}

/// what is wrong with a chain (leaf first) for this host at this time: F_* bits, 0 for nothing
pub fn check_chain(chain: &[Cert], anchors: &[Cert], host: &str, now: i64) -> u32 {
    let Some(leaf) = chain.first() else { return F_UNKNOWN_ISSUER };
    let mut f = 0;
    if !leaf.names(host) {
        f |= F_NAME;
    }
    if leaf.key.is_none() {
        f |= F_BAD_SIGNATURE;
    }
    let mut used = alloc::vec![false; chain.len()];
    let mut cur = leaf;
    used[0] = true;
    loop {
        if now >= cur.not_after {
            f |= F_EXPIRED;
        }
        if now < cur.not_before {
            f |= F_NOT_YET;
        }
        // an anchor, by its very bytes or by its signature
        if anchors.iter().any(|a| a.der == cur.der || (a.is_ca && cur.signed_by(a))) {
            return f;
        }
        let next = chain.iter().enumerate().find(|(i, c)| !used[*i] && c.is_ca && cur.signed_by(c));
        match next {
            Some((i, c)) => {
                used[i] = true;
                cur = c;
            }
            None => break,
        }
    }
    if cur.self_signed() {
        f | F_SELF_SIGNED
    } else if cur.issuer_der == cur.subject_der {
        f | F_BAD_SIGNATURE
    } else {
        f | F_UNKNOWN_ISSUER
    }
}
