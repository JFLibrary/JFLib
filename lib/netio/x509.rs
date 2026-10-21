// x509 -- certificates (RFC 5280): the private PKI of BackupCraft (Ed25519, RFC 8410; built
// and signed here) and the public web PKI (RSA and P-256 / P-384 keys; RSA PKCS #1 v1.5 / PSS
// and ECDSA signatures with SHA-256 / 384 / 512), parsed strictly
//
//   Cert::parse(der) -> Option<Cert>        subject / issuer, validity, key, CA flag, names,
//                                           key and extended key usage, name constraints,
//                                           unknown critical extensions noticed
//   cert.fingerprint() -> SHA-256 (hex)     cert.signed_by(&issuer) -> bool
//   cert.verify(alg, data, sig)             a signature made with the certificate's key
//   verify_chain(&[leaf, ...], &anchors, now, name, usage) -> Result<(), String>   builds a
//                                           path through the given intermediates to an anchor
//   CertBuilder { .. }.sign(&issuer_cert_or_self, &issuer_key) -> DER
//
// Not checked: revocation (CRL / OCSP), certificate policies; name constraints cover DNS
// names and IP addresses (the names a TLS server is reached by).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ecdsa::Curve;
use crypto::ed25519::Keypair;
use crypto::HashAlg;
use plat::net::IpAddr;

use crate::der::{self, Der};

pub const OID_ED25519: &[u64] = &[1, 3, 101, 112];
const OID_CN: &[u64] = &[2, 5, 4, 3];
const OID_O: &[u64] = &[2, 5, 4, 10];
const OID_BASIC: &[u64] = &[2, 5, 29, 19];
const OID_KEY_USAGE: &[u64] = &[2, 5, 29, 15];
const OID_EKU: &[u64] = &[2, 5, 29, 37];
const OID_SAN: &[u64] = &[2, 5, 29, 17];
const OID_SKI: &[u64] = &[2, 5, 29, 14];
const OID_AKI: &[u64] = &[2, 5, 29, 35];
const OID_NAME_CONSTRAINTS: &[u64] = &[2, 5, 29, 30];
const OID_POLICIES: &[u64] = &[2, 5, 29, 32];
const OID_POLICY_CONSTRAINTS: &[u64] = &[2, 5, 29, 36];
const OID_INHIBIT_ANY: &[u64] = &[2, 5, 29, 54];
const OID_RSA: &[u64] = &[1, 2, 840, 113549, 1, 1, 1];
const OID_RSA_PSS: &[u64] = &[1, 2, 840, 113549, 1, 1, 10];
const OID_MGF1: &[u64] = &[1, 2, 840, 113549, 1, 1, 8];
const OID_RSA_SHA256: &[u64] = &[1, 2, 840, 113549, 1, 1, 11];
const OID_RSA_SHA384: &[u64] = &[1, 2, 840, 113549, 1, 1, 12];
const OID_RSA_SHA512: &[u64] = &[1, 2, 840, 113549, 1, 1, 13];
const OID_EC: &[u64] = &[1, 2, 840, 10045, 2, 1];
const OID_P256: &[u64] = &[1, 2, 840, 10045, 3, 1, 7];
const OID_P384: &[u64] = &[1, 3, 132, 0, 34];
const OID_ECDSA_SHA256: &[u64] = &[1, 2, 840, 10045, 4, 3, 2];
const OID_ECDSA_SHA384: &[u64] = &[1, 2, 840, 10045, 4, 3, 3];
const OID_ECDSA_SHA512: &[u64] = &[1, 2, 840, 10045, 4, 3, 4];
const OID_SHA256: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 1];
const OID_SHA384: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 2];
const OID_SHA512: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 3];
const OID_SERVER_AUTH: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 3, 1];
const OID_CLIENT_AUTH: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 3, 2];

/// key usage bits as they appear in the first byte
pub const KU_DIGITAL_SIGNATURE: u8 = 0x80;
pub const KU_KEY_CERT_SIGN: u8 = 0x04;
pub const KU_CRL_SIGN: u8 = 0x02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage {
    Server,
    Client,
}

/// a public key
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PubKey {
    /// a key type this code does not use
    #[default]
    Other,
    Ed25519([u8; 32]),
    /// modulus and exponent, big-endian
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// an uncompressed point (04 || x || y)
    Ec { curve: Curve, point: Vec<u8> },
}

/// a signature algorithm (of certificates and of TLS handshakes)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SigAlg {
    #[default]
    Unknown,
    Ed25519,
    RsaPkcs1(HashAlg),
    /// salt length None: any (TLS fixes it to the hash length)
    RsaPss(HashAlg, Option<usize>),
    Ecdsa(HashAlg),
}

/// DNS and IP name constraints of a CA (other forms are recorded as present only)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameConstraints {
    pub permitted_dns: Vec<String>,
    pub excluded_dns: Vec<String>,
    /// address and mask (4 + 4 or 16 + 16 bytes)
    pub permitted_ip: Vec<Vec<u8>>,
    pub excluded_ip: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cert {
    pub der: Vec<u8>,
    pub tbs: Vec<u8>,
    pub serial: Vec<u8>,
    /// the raw DER of the issuer and subject names (compared byte for byte)
    pub issuer: Vec<u8>,
    pub subject: Vec<u8>,
    pub subject_cn: String,
    pub not_before: i64,
    pub not_after: i64,
    /// Ed25519 public key (empty for other key types)
    pub public_key: Vec<u8>,
    pub key: PubKey,
    pub ed25519_signed: bool,
    pub sig_alg: SigAlg,
    pub signature: Vec<u8>,
    /// 1 or 3
    pub version: u8,
    pub name_constraints: Option<NameConstraints>,
    pub is_ca: bool,
    pub path_len: Option<u64>,
    pub key_usage: Option<u8>,
    pub eku: Vec<Vec<u8>>,
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
    pub ski: Vec<u8>,
    /// a critical extension this code does not understand (such a certificate is rejected)
    pub unknown_critical: bool,
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn digits(s: &[u8]) -> Option<i64> {
    if s.is_empty() || !s.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(s.iter().fold(0i64, |a, c| a * 10 + (c - b'0') as i64))
}

/// UTCTime / GeneralizedTime (Z, seconds) -> seconds since 1970
fn parse_time(tag: u8, c: &[u8]) -> Option<i64> {
    let (y, rest) = match tag {
        der::UTC_TIME if c.len() == 13 => {
            let yy = digits(&c[..2])?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &c[2..])
        }
        der::GEN_TIME if c.len() == 15 => (digits(&c[..4])?, &c[4..]),
        _ => return None,
    };
    if rest[10] != b'Z' {
        return None;
    }
    let (mo, d, h, mi, s) = (digits(&rest[0..2])?, digits(&rest[2..4])?, digits(&rest[4..6])?, digits(&rest[6..8])?, digits(&rest[8..10])?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

fn encode_time(t: i64) -> Vec<u8> {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    let (h, mi, se) = (s / 3600, s / 60 % 60, s % 60);
    if (1950..2050).contains(&y) {
        der::tlv(der::UTC_TIME, format!("{:02}{:02}{:02}{:02}{:02}{:02}Z", y % 100, m, d, h, mi, se).as_bytes())
    } else {
        der::tlv(der::GEN_TIME, format!("{:04}{:02}{:02}{:02}{:02}{:02}Z", y, m, d, h, mi, se).as_bytes())
    }
}

fn is_ed25519_alg(alg: &[u8]) -> bool {
    let mut d = Der::new(alg);
    d.read(der::OID) == Some(&der::oid_bytes(OID_ED25519)[..]) && d.is_empty()
}

fn oid_is(o: &[u8], arcs: &[u64]) -> bool {
    o == der::oid_bytes(arcs)
}

fn hash_of(o: &[u8]) -> Option<HashAlg> {
    if oid_is(o, OID_SHA256) {
        Some(HashAlg::Sha256)
    } else if oid_is(o, OID_SHA384) {
        Some(HashAlg::Sha384)
    } else if oid_is(o, OID_SHA512) {
        Some(HashAlg::Sha512)
    } else {
        None
    }
}

/// an AlgorithmIdentifier's hash (parameters absent or NULL)
fn hash_alg_id(alg: &[u8]) -> Option<HashAlg> {
    let mut d = Der::new(alg);
    let h = hash_of(d.read(der::OID)?)?;
    d.optional(der::NULL);
    d.is_empty().then_some(h)
}

/// the signature algorithm of an AlgorithmIdentifier's content
fn sig_alg(alg: &[u8]) -> SigAlg {
    if is_ed25519_alg(alg) {
        return SigAlg::Ed25519;
    }
    let mut d = Der::new(alg);
    let Some(o) = d.read(der::OID) else { return SigAlg::Unknown };
    for (arcs, h) in [(OID_RSA_SHA256, HashAlg::Sha256), (OID_RSA_SHA384, HashAlg::Sha384), (OID_RSA_SHA512, HashAlg::Sha512)] {
        if oid_is(o, arcs) {
            d.optional(der::NULL);
            return if d.is_empty() { SigAlg::RsaPkcs1(h) } else { SigAlg::Unknown };
        }
    }
    for (arcs, h) in [(OID_ECDSA_SHA256, HashAlg::Sha256), (OID_ECDSA_SHA384, HashAlg::Sha384), (OID_ECDSA_SHA512, HashAlg::Sha512)] {
        if oid_is(o, arcs) {
            return if d.is_empty() { SigAlg::Ecdsa(h) } else { SigAlg::Unknown };
        }
    }
    if oid_is(o, OID_RSA_PSS) {
        // RSASSA-PSS-params: [0] hash, [1] MGF1(hash), [2] salt length; SHA-1 defaults refused
        let pss = |mut d: Der| -> Option<SigAlg> {
            let mut p = d.seq()?;
            let h = hash_alg_id(Der::new(p.read(0xa0)?).read(der::SEQUENCE)?)?;
            let mut mgf = Der::new(Der::new(p.read(0xa1)?).read(der::SEQUENCE)?);
            if !oid_is(mgf.read(der::OID)?, OID_MGF1) || hash_alg_id(mgf.read(der::SEQUENCE)?)? != h {
                return None;
            }
            let salt = match p.optional(0xa2) {
                Some(x) => Der::new(x).u64()? as usize,
                None => 20,
            };
            if let Some(t) = p.optional(0xa3) {
                if Der::new(t).u64()? != 1 {
                    return None;
                }
            }
            (p.is_empty() && d.is_empty()).then_some(SigAlg::RsaPss(h, Some(salt)))
        };
        return pss(d).unwrap_or(SigAlg::Unknown);
    }
    SigAlg::Unknown
}

/// the key of a SubjectPublicKeyInfo
fn pub_key(alg: &[u8], key: &[u8]) -> PubKey {
    if is_ed25519_alg(alg) {
        return match <[u8; 32]>::try_from(key) {
            Ok(k) => PubKey::Ed25519(k),
            Err(_) => PubKey::Other,
        };
    }
    let mut d = Der::new(alg);
    let Some(o) = d.read(der::OID) else { return PubKey::Other };
    if oid_is(o, OID_RSA) {
        let rsa = || -> Option<PubKey> {
            let mut k = Der::new(key).seq()?;
            let n = k.read(der::INTEGER)?;
            let e = k.read(der::INTEGER)?;
            (k.is_empty() && n.first().is_some_and(|b| b & 0x80 == 0)).then(|| PubKey::Rsa { n: Vec::from(n), e: Vec::from(e) })
        };
        return rsa().unwrap_or(PubKey::Other);
    }
    if oid_is(o, OID_EC) {
        let Some(c) = d.read(der::OID) else { return PubKey::Other };
        let curve = if oid_is(c, OID_P256) {
            Curve::P256
        } else if oid_is(c, OID_P384) {
            Curve::P384
        } else {
            return PubKey::Other;
        };
        if key.len() == 1 + 2 * curve.bytes() && key[0] == 4 {
            return PubKey::Ec { curve, point: Vec::from(key) };
        }
    }
    PubKey::Other
}

/// GeneralSubtrees: DNS names and IP ranges
fn subtrees(b: &[u8], dns: &mut Vec<String>, ips: &mut Vec<Vec<u8>>) -> Option<()> {
    let mut d = Der::new(b);
    while !d.is_empty() {
        let mut st = d.seq()?;
        let (tag, v, _) = st.next()?;
        match tag {
            0x82 => dns.push(String::from_utf8_lossy(v).to_ascii_lowercase()),
            0x87 if v.len() == 8 || v.len() == 32 => ips.push(Vec::from(v)),
            0x87 => return None,
            _ => {} // directory names, e-mail addresses, URIs: not names of a TLS server
        }
    }
    Some(())
}

/// the common name of a Name
fn cn_of(name: &[u8]) -> String {
    let mut d = Der::new(name);
    while let Some(set) = d.read(der::SET) {
        let mut s = Der::new(set);
        while let Some(atv) = s.seq() {
            let mut a = atv;
            if a.read(der::OID) == Some(&der::oid_bytes(OID_CN)[..]) {
                if let Some((_, v, _)) = a.next() {
                    return String::from_utf8_lossy(v).into_owned();
                }
            }
        }
    }
    String::new()
}

/// DER INTEGER content -> unsigned big-endian bytes
fn unsigned(b: &[u8]) -> &[u8] {
    let i = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    &b[i..]
}

impl Cert {
    pub fn parse(bytes: &[u8]) -> Option<Cert> {
        let mut top = Der::new(bytes);
        let mut cert = top.seq()?;
        if !top.is_empty() {
            return None;
        }
        let tbs_raw = cert.read_raw(der::SEQUENCE)?;
        let alg = cert.read(der::SEQUENCE)?;
        let sig = cert.bits()?;
        if !cert.is_empty() {
            return None;
        }
        let mut t = Der::new(tbs_raw).seq()?;
        let sa = sig_alg(alg);
        let mut c = Cert { der: Vec::from(bytes), tbs: Vec::from(tbs_raw), signature: Vec::from(sig), ed25519_signed: sa == SigAlg::Ed25519, sig_alg: sa, ..Cert::default() };
        // version [0] EXPLICIT INTEGER: v3, or v1 without it (old root certificates)
        c.version = match t.optional(0xa0) {
            Some(v) if Der::new(v).u64()? == 2 => 3,
            Some(_) => return None,
            None => 1,
        };
        c.serial = Vec::from(t.read(der::INTEGER)?);
        let inner_alg = t.read(der::SEQUENCE)?;
        if inner_alg != alg {
            return None;
        }
        c.issuer = Vec::from(t.read_raw(der::SEQUENCE)?);
        let mut val = t.seq()?;
        let (tag, nb, _) = val.next()?;
        c.not_before = parse_time(tag, nb)?;
        let (tag, na, _) = val.next()?;
        c.not_after = parse_time(tag, na)?;
        let subj = t.read_raw(der::SEQUENCE)?;
        c.subject = Vec::from(subj);
        c.subject_cn = cn_of(Der::new(subj).read(der::SEQUENCE)?);
        let mut spki = t.seq()?;
        let kalg = spki.read(der::SEQUENCE)?;
        let key = spki.bits()?;
        c.key = pub_key(kalg, key);
        if let PubKey::Ed25519(k) = &c.key {
            c.public_key = Vec::from(&k[..]);
        }
        // optional issuerUniqueID [1], subjectUniqueID [2]: not accepted
        if let Some(ext) = t.read(0xa3) {
            if c.version != 3 {
                return None;
            }
            let mut list = Der::new(ext).seq()?;
            let mut seen: Vec<Vec<u8>> = Vec::new();
            while !list.is_empty() {
                let mut e = list.seq()?;
                let id = Vec::from(e.read(der::OID)?);
                if seen.contains(&id) {
                    return None; // an extension twice
                }
                seen.push(id.clone());
                let critical = if e.peek_tag() == Some(der::BOOLEAN) { e.bool()? } else { false };
                let val = e.read(der::OCTET_STRING)?;
                let is = |o: &[u64]| id == der::oid_bytes(o);
                if is(OID_BASIC) {
                    let mut b = Der::new(val).seq()?;
                    if b.peek_tag() == Some(der::BOOLEAN) {
                        c.is_ca = b.bool()?;
                    }
                    if b.peek_tag() == Some(der::INTEGER) {
                        c.path_len = Some(b.u64()?);
                    }
                } else if is(OID_KEY_USAGE) {
                    let mut k = Der::new(val);
                    let bs = k.read(der::BIT_STRING)?;
                    c.key_usage = Some(*bs.get(1).unwrap_or(&0));
                } else if is(OID_EKU) {
                    let mut k = Der::new(val).seq()?;
                    while let Some(o) = k.read(der::OID) {
                        c.eku.push(Vec::from(o));
                    }
                } else if is(OID_SAN) {
                    let mut k = Der::new(val).seq()?;
                    while let Some((tag, v, _)) = k.next() {
                        match tag {
                            0x82 => c.dns.push(String::from_utf8_lossy(v).to_ascii_lowercase()),
                            0x87 if v.len() == 4 => c.ips.push(IpAddr::V4([v[0], v[1], v[2], v[3]])),
                            0x87 if v.len() == 16 => {
                                let mut a = [0u8; 16];
                                a.copy_from_slice(v);
                                c.ips.push(IpAddr::V6(a));
                            }
                            _ => {}
                        }
                    }
                } else if is(OID_SKI) {
                    c.ski = Vec::from(Der::new(val).read(der::OCTET_STRING)?);
                } else if is(OID_NAME_CONSTRAINTS) {
                    let mut nc = NameConstraints::default();
                    let mut k = Der::new(val).seq()?;
                    if let Some(p) = k.optional(0xa0) {
                        subtrees(p, &mut nc.permitted_dns, &mut nc.permitted_ip)?;
                    }
                    if let Some(x) = k.optional(0xa1) {
                        subtrees(x, &mut nc.excluded_dns, &mut nc.excluded_ip)?;
                    }
                    c.name_constraints = Some(nc);
                } else if is(OID_AKI) || is(OID_POLICIES) || is(OID_POLICY_CONSTRAINTS) || is(OID_INHIBIT_ANY) {
                    // known; policies are not enforced
                } else if critical {
                    c.unknown_critical = true;
                }
            }
        }
        if !t.is_empty() {
            return None;
        }
        Some(c)
    }

    /// SHA-256 of the certificate (hex)
    pub fn fingerprint(&self) -> String {
        crypto::hex(&crypto::sha256(&self.der))
    }

    /// whether a signature over `data` was made with this certificate's key
    pub fn verify(&self, alg: SigAlg, data: &[u8], sig: &[u8]) -> bool {
        match (&self.key, alg) {
            (PubKey::Ed25519(pk), SigAlg::Ed25519) => match <[u8; 64]>::try_from(sig) {
                Ok(s) => crypto::ed25519::verify(pk, data, &s),
                Err(_) => false,
            },
            (PubKey::Rsa { n, e }, SigAlg::RsaPkcs1(h)) => crypto::rsa::PublicKey::new(n, e).is_some_and(|k| k.verify_pkcs1(h, &h.digest(data), sig)),
            (PubKey::Rsa { n, e }, SigAlg::RsaPss(h, salt)) => crypto::rsa::PublicKey::new(n, e).is_some_and(|k| k.verify_pss(h, &h.digest(data), sig, salt)),
            (PubKey::Ec { curve, point }, SigAlg::Ecdsa(h)) => {
                // Ecdsa-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }
                let rs = || -> Option<(&[u8], &[u8])> {
                    let mut d = Der::new(sig).seq()?;
                    let r = d.read(der::INTEGER)?;
                    let s = d.read(der::INTEGER)?;
                    d.is_empty().then_some((r, s))
                };
                rs().is_some_and(|(r, s)| crypto::ecdsa::verify(*curve, point, &h.digest(data), unsigned(r), unsigned(s)))
            }
            _ => false,
        }
    }

    /// whether `issuer` signed this certificate
    pub fn signed_by(&self, issuer: &Cert) -> bool {
        self.issuer == issuer.subject && issuer.verify(self.sig_alg, &self.tbs, &self.signature)
    }

    fn eku_allows(&self, u: Usage) -> bool {
        if self.eku.is_empty() {
            return true;
        }
        let want = der::oid_bytes(if u == Usage::Server { OID_SERVER_AUTH } else { OID_CLIENT_AUTH });
        self.eku.iter().any(|e| *e == want)
    }

    /// whether the certificate names this host (DNS name, case-insensitive; IP address)
    pub fn names(&self, host: &str) -> bool {
        if let Some(ip) = IpAddr::parse(host.trim_matches(['[', ']'])) {
            return self.ips.contains(&ip);
        }
        let h = host.to_ascii_lowercase();
        self.dns.iter().any(|d| *d == h || (d.starts_with("*.") && h.split_once('.').is_some_and(|(_, rest)| rest == &d[2..])))
    }
}

/// whether a DNS name lies in a constraint subtree ("example.com": it and its subdomains;
/// ".example.com": subdomains only)
fn dns_in(name: &str, base: &str) -> bool {
    let name = name.trim_start_matches("*.");
    if let Some(b) = base.strip_prefix('.') {
        return name.ends_with(base) && name.len() > b.len();
    }
    name == base || (name.len() > base.len() && name.ends_with(base) && name.as_bytes()[name.len() - base.len() - 1] == b'.')
}

fn ip_in(ip: &IpAddr, range: &[u8]) -> bool {
    let a: &[u8] = match ip {
        IpAddr::V4(a) => a,
        IpAddr::V6(a) => a,
    };
    let half = range.len() / 2;
    a.len() == half && a.iter().zip(&range[..half]).zip(&range[half..]).all(|((x, net), m)| x & m == net & m)
}

/// the leaf's names against the name constraints of a CA on its path
fn constraints_allow(leaf: &Cert, nc: &NameConstraints) -> bool {
    for d in &leaf.dns {
        if nc.excluded_dns.iter().any(|b| dns_in(d, b)) || (!nc.permitted_dns.is_empty() && !nc.permitted_dns.iter().any(|b| dns_in(d, b))) {
            return false;
        }
    }
    for ip in &leaf.ips {
        if nc.excluded_ip.iter().any(|r| ip_in(ip, r)) || (!nc.permitted_ip.is_empty() && !nc.permitted_ip.iter().any(|r| ip_in(ip, r))) {
            return false;
        }
    }
    true
}

/// check a chain (leaf first) against trusted anchors: a path from the leaf through the other
/// certificates of the chain (in any order) to an anchor
pub fn verify_chain(chain: &[Cert], anchors: &[Cert], now: i64, name: Option<&str>, usage: Usage) -> Result<(), String> {
    let leaf = chain.first().ok_or("kein Zertifikat")?;
    if leaf.unknown_critical {
        return Err(String::from("unbekannte kritische Erweiterung"));
    }
    if now < leaf.not_before || now > leaf.not_after {
        return Err(String::from("Zertifikat abgelaufen oder noch nicht gültig"));
    }
    if !leaf.eku_allows(usage) {
        return Err(String::from("Zertifikat nicht für diesen Zweck"));
    }
    if leaf.key_usage.is_some_and(|k| k & KU_DIGITAL_SIGNATURE == 0) {
        return Err(String::from("Zertifikat darf nicht signieren"));
    }
    if let Some(n) = name {
        if !leaf.names(n) {
            return Err(format!("Zertifikat gilt nicht für {}", n));
        }
    }
    let mut why = String::from("keine vertrauenswürdige Zertifizierungsstelle");
    let mut used = alloc::vec![false; chain.len()];
    if path(leaf, chain, anchors, now, 0, &mut used, &mut why) { Ok(()) } else { Err(why) }
}

/// a path from `cur` upwards (depth: CAs between the leaf and cur)
fn path(cur: &Cert, chain: &[Cert], anchors: &[Cert], now: i64, depth: usize, used: &mut [bool], why: &mut String) -> bool {
    if depth > 8 {
        *why = String::from("Zertifikatskette zu lang");
        return false;
    }
    let leaf = &chain[0];
    for a in anchors.iter().filter(|a| a.subject == cur.issuer) {
        if !cur.signed_by(a) {
            continue;
        }
        if now < a.not_before || now > a.not_after {
            *why = String::from("die Zertifizierungsstelle ist abgelaufen");
            continue;
        }
        if !(a.is_ca || a.version == 1) || a.key_usage.is_some_and(|k| k & KU_KEY_CERT_SIGN == 0) {
            *why = String::from("Vertrauensanker ist keine Zertifizierungsstelle");
            continue;
        }
        if a.name_constraints.as_ref().is_some_and(|nc| !constraints_allow(leaf, nc)) {
            *why = String::from("der Name ist für diese Zertifizierungsstelle nicht erlaubt");
            continue;
        }
        return true;
    }
    for i in 1..chain.len() {
        let next = &chain[i];
        if used[i] || next.subject != cur.issuer || !cur.signed_by(next) {
            continue;
        }
        if !next.is_ca || next.unknown_critical || now < next.not_before || now > next.not_after
            || next.path_len.is_some_and(|p| (p as usize) < depth) || next.key_usage.is_some_and(|k| k & KU_KEY_CERT_SIGN == 0)
        {
            *why = String::from("ungültige Zwischenzertifizierungsstelle");
            continue;
        }
        if next.name_constraints.as_ref().is_some_and(|nc| !constraints_allow(leaf, nc)) {
            *why = String::from("der Name ist für diese Zertifizierungsstelle nicht erlaubt");
            continue;
        }
        used[i] = true;
        if path(next, chain, anchors, now, depth + 1, used, why) {
            return true;
        }
        used[i] = false;
    }
    false
}

fn name(cn: &str, org: &str) -> Vec<u8> {
    let mut rdns = Vec::new();
    if !org.is_empty() {
        rdns.push(der::set(&[&der::seq(&[&der::oid(OID_O), &der::utf8(org)])]));
    }
    rdns.push(der::set(&[&der::seq(&[&der::oid(OID_CN), &der::utf8(cn)])]));
    let parts: Vec<&[u8]> = rdns.iter().map(|v| v.as_slice()).collect();
    der::seq(&parts)
}

fn ext(oid: &[u64], critical: bool, value: &[u8]) -> Vec<u8> {
    if critical {
        der::seq(&[&der::oid(oid), &der::boolean(true), &der::octets(value)])
    } else {
        der::seq(&[&der::oid(oid), &der::octets(value)])
    }
}

/// what a new certificate says
#[derive(Clone, Debug, Default)]
pub struct CertBuilder {
    pub serial: [u8; 16],
    pub cn: String,
    pub org: String,
    pub not_before: i64,
    pub not_after: i64,
    pub public_key: [u8; 32],
    pub is_ca: bool,
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
    pub server_auth: bool,
    pub client_auth: bool,
}

impl CertBuilder {
    /// the DER certificate, signed by `key`; `issuer`: the issuing certificate (None: self
    /// signed with the builder's own name)
    pub fn sign(&self, issuer: Option<&Cert>, key: &Keypair) -> Vec<u8> {
        let alg = der::seq(&[&der::oid(OID_ED25519)]);
        let mut serial = self.serial;
        serial[0] &= 0x7f; // positive
        let subject = name(&self.cn, &self.org);
        let issuer_name = issuer.map_or(subject.clone(), |i| i.subject.clone());
        let validity = der::seq(&[&encode_time(self.not_before), &encode_time(self.not_after)]);
        let spki = der::seq(&[&alg, &der::bits(&self.public_key)]);
        let ski = crypto::sha256(&self.public_key);
        let mut exts: Vec<Vec<u8>> = Vec::new();
        if self.is_ca {
            exts.push(ext(OID_BASIC, true, &der::seq(&[&der::boolean(true), &der::small_int(0)])));
            exts.push(ext(OID_KEY_USAGE, true, &der::tlv(der::BIT_STRING, &[1, KU_KEY_CERT_SIGN | KU_CRL_SIGN])));
        } else {
            exts.push(ext(OID_BASIC, true, &der::seq(&[])));
            exts.push(ext(OID_KEY_USAGE, true, &der::tlv(der::BIT_STRING, &[7, KU_DIGITAL_SIGNATURE])));
            let mut eku: Vec<Vec<u8>> = Vec::new();
            if self.server_auth {
                eku.push(der::oid(OID_SERVER_AUTH));
            }
            if self.client_auth {
                eku.push(der::oid(OID_CLIENT_AUTH));
            }
            if !eku.is_empty() {
                let parts: Vec<&[u8]> = eku.iter().map(|v| v.as_slice()).collect();
                exts.push(ext(OID_EKU, false, &der::seq(&parts)));
            }
        }
        if !self.dns.is_empty() || !self.ips.is_empty() {
            let mut names: Vec<Vec<u8>> = self.dns.iter().map(|d| der::tlv(0x82, d.as_bytes())).collect();
            for ip in &self.ips {
                names.push(match ip {
                    IpAddr::V4(a) => der::tlv(0x87, a),
                    IpAddr::V6(a) => der::tlv(0x87, a),
                });
            }
            let parts: Vec<&[u8]> = names.iter().map(|v| v.as_slice()).collect();
            exts.push(ext(OID_SAN, false, &der::seq(&parts)));
        }
        exts.push(ext(OID_SKI, false, &der::octets(&ski[..20])));
        let aki_src = issuer.map_or(ski[..20].to_vec(), |i| if i.ski.is_empty() { crypto::sha256(&i.public_key)[..20].to_vec() } else { i.ski.clone() });
        exts.push(ext(OID_AKI, false, &der::seq(&[&der::tlv(0x80, &aki_src)])));
        let ext_parts: Vec<&[u8]> = exts.iter().map(|v| v.as_slice()).collect();
        let tbs = der::seq(&[
            &der::explicit(0, &der::small_int(2)),
            &der::int_u(&serial),
            &alg,
            &issuer_name,
            &validity,
            &subject,
            &spki,
            &der::explicit(3, &der::seq(&ext_parts)),
        ]);
        let sig = key.sign(&tbs);
        der::seq(&[&tbs, &alg, &der::bits(&sig)])
    }
}

/// the PKCS#8 form of an Ed25519 private key (for interoperable PEM files)
pub fn ed25519_pkcs8(seed: &[u8; 32]) -> Vec<u8> {
    der::seq(&[&der::small_int(0), &der::seq(&[&der::oid(OID_ED25519)]), &der::octets(&der::octets(seed))])
}

/// the seed of a PKCS#8 Ed25519 private key
pub fn ed25519_from_pkcs8(b: &[u8]) -> Option<[u8; 32]> {
    let mut d = Der::new(b).seq()?;
    if d.u64()? != 0 || !is_ed25519_alg(d.read(der::SEQUENCE)?) {
        return None;
    }
    let inner = d.read(der::OCTET_STRING)?;
    let seed = Der::new(inner).read(der::OCTET_STRING)?;
    (seed.len() == 32).then(|| {
        let mut s = [0u8; 32];
        s.copy_from_slice(seed);
        s
    })
}
