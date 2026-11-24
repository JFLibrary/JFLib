// x509 -- certificates (RFC 5280) for all of JFLib: netio's TLS and https, netpkt's TLS core and
// simulators.  A program's own private PKI (Ed25519, RFC 8410; built and signed here) and the
// public web PKI (RSA and P-256 / P-384 keys; RSA PKCS #1 v1.5 / PSS and ECDSA signatures with
// SHA-256 / 384 / 512), parsed strictly; no I/O, no clock (the caller passes the time)
//
//   der                                     the DER reader and writer certificates need
//   IpAddr                                  addresses in names (octets, parse, RFC 5952 text)
//   Cert::parse(der) -> Option<Cert>        subject / issuer, validity, key, CA flag, names,
//                                           key and extended key usage, name constraints,
//                                           unknown critical extensions noticed
//   cert.fingerprint() -> SHA-256 (hex)     fingerprint_bytes(), fingerprint_text() (colons)
//   cert.signed_by(&issuer) -> bool         self_signed(), names(host), name_list(), bytes()
//   cert.verify(alg, data, sig)             a signature made with the certificate's key
//   verify_chain(&[leaf, ...], &anchors, now, name, usage) -> Result<(), String>   builds a
//                                           path through the given intermediates to an anchor
//   CertBuilder { .. }.sign(&issuer_cert_or_self, &issuer_key) -> DER     (Ed25519)
//   CertBuilder { spki, .. }.sign_key(issuer, &PrivateKey, extra) -> DER   any key, signed by any
//   PrivateKey: Ed25519, ECDSA (P-256 / P-384), RSA; PKCS #8 / SEC 1 / PKCS #1 (key.rs)
//   ocsp::check(der, &cert, &issuer, now)   an OCSP response (RFC 6960); request, CertId,
//                                           ResponseBuilder for a private CA's responder (ocsp.rs)
//   CsrBuilder { dns, .. }.sign(&key), Csr::parse   certificate requests (PKCS #10, csr.rs)
//
// Not checked: CRLs, certificate policies (OCSP: the ocsp module; a certificate's responders,
// caIssuers and TLS features are read).  Name constraints are processed
// for DNS names, IP addresses, e-mail addresses (rfc822Name), URIs (their host) and directory
// names (a byte-wise prefix of the subject's RDNs); a CA that constrains another form refuses
// every certificate below it that has a name of that form (RFC 5280 4.2.1.10).  They apply to
// the leaf and to the intermediates below the CA, self-issued ones excepted (6.1.3 (b)).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod csr;
pub mod der;
mod ip;
mod key;
pub mod ocsp;

pub use csr::{Csr, CsrBuilder};
pub use ip::IpAddr;
pub use key::{spki_of, PrivateKey};

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ecdsa::Curve;
use crypto::ed25519::Keypair;
use crypto::HashAlg;

use crate::der::Der;

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
const OID_ANY_EKU: &[u64] = &[2, 5, 29, 37, 0];
const OID_OCSP_SIGNING: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 3, 9];
const OID_AIA: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 1];
const OID_AD_OCSP: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 48, 1];
const OID_AD_CA_ISSUERS: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 48, 2];
const OID_TLS_FEATURE: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 24];
const OID_OCSP_NOCHECK: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 48, 1, 5];

/// the TLS feature of a certificate that must come with a stapled OCSP response (RFC 7633:
/// status_request)
pub const TLS_FEATURE_STATUS_REQUEST: u16 = 5;

/// key usage bits as they appear in the first byte
pub const KU_DIGITAL_SIGNATURE: u8 = 0x80;
pub const KU_KEY_CERT_SIGN: u8 = 0x04;
pub const KU_CRL_SIGN: u8 = 0x02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Usage {
    Server,
    Client,
}

/// a public key
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PubKey {
    /// a key type this code does not use
    #[default]
    Other,
    Ed25519([u8; 32]),
    /// modulus and exponent, big-endian without leading zero bytes
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// an uncompressed point (04 || x || y)
    Ec { curve: Curve, point: Vec<u8> },
}

/// a signature algorithm (of certificates and of TLS handshakes)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SigAlg {
    #[default]
    Unknown,
    Ed25519,
    RsaPkcs1(HashAlg),
    /// salt length None: any (TLS fixes it to the hash length)
    RsaPss(HashAlg, Option<usize>),
    Ecdsa(HashAlg),
}

/// the name constraints of a CA
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameConstraints {
    pub permitted_dns: Vec<String>,
    pub excluded_dns: Vec<String>,
    /// address and mask (4 + 4 or 16 + 16 bytes)
    pub permitted_ip: Vec<Vec<u8>>,
    pub excluded_ip: Vec<Vec<u8>>,
    /// rfc822Name subtrees, lower case: a mailbox ("a@example.com"), all mailboxes of a host
    /// ("example.com") or of the subdomains of a domain (".example.com")
    pub permitted_email: Vec<String>,
    pub excluded_email: Vec<String>,
    /// uniformResourceIdentifier subtrees, lower case: the URI's host ("example.com") or the
    /// subdomains of a domain (".example.com")
    pub permitted_uri: Vec<String>,
    pub excluded_uri: Vec<String>,
    /// directoryName subtrees (the DER of a Name): the subject begins with its RDNs
    pub permitted_dn: Vec<Vec<u8>>,
    pub excluded_dn: Vec<Vec<u8>>,
    /// the GeneralName tags of constrained forms not processed here (otherName 0xa0,
    /// x400Address 0xa3, ediPartyName 0xa5, registeredID 0x88)
    pub other_forms: Vec<u8>,
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
    /// the subject's organization, the issuer's common name and organization (for dialogs)
    pub subject_org: String,
    pub issuer_cn: String,
    pub issuer_org: String,
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
    /// rfc822Name entries of the subject alternative name and emailAddress attributes of the
    /// subject (lower case): what e-mail name constraints apply to
    pub emails: Vec<String>,
    /// uniformResourceIdentifier entries of the subject alternative name
    pub uris: Vec<String>,
    /// directoryName entries of the subject alternative name (the DER of each Name)
    pub dirs: Vec<Vec<u8>>,
    /// the tags of subject alternative names of other forms (otherName 0xa0, x400Address 0xa3,
    /// ediPartyName 0xa5, registeredID 0x88)
    pub other_names: Vec<u8>,
    pub ski: Vec<u8>,
    /// the keyIdentifier of the authority key identifier (empty without one): which key of the
    /// issuer signed it (ACME's renewal information names a certificate by it)
    pub aki: Vec<u8>,
    /// the OCSP responders of the authority information access (URIs, RFC 5280 4.2.2.1)
    pub ocsp_urls: Vec<String>,
    /// where the issuer's certificate is (caIssuers URIs of the authority information access)
    pub ca_issuers: Vec<String>,
    /// the TLS features the certificate demands (RFC 7633): TLS_FEATURE_STATUS_REQUEST is
    /// "must staple", a TLS client refuses it without a stapled OCSP response
    pub tls_features: Vec<u16>,
    /// id-pkix-ocsp-nocheck (RFC 6960 4.2.2.2.1): an OCSP responder's certificate that is not
    /// checked for revocation itself
    pub ocsp_nocheck: bool,
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

pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
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
pub(crate) fn parse_time(tag: u8, c: &[u8]) -> Option<i64> {
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

pub(crate) fn oid_is(o: &[u8], arcs: &[u64]) -> bool {
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
pub(crate) fn sig_alg(alg: &[u8]) -> SigAlg {
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
pub(crate) fn pub_key(alg: &[u8], key: &[u8]) -> PubKey {
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
            (k.is_empty() && n.first().is_some_and(|b| b & 0x80 == 0)).then(|| PubKey::Rsa { n: Vec::from(strip_zeros(n)), e: Vec::from(strip_zeros(e)) })
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

/// the subtrees of one kind (permitted or excluded) of a NameConstraints
#[derive(Default)]
struct Subtrees {
    dns: Vec<String>,
    ips: Vec<Vec<u8>>,
    email: Vec<String>,
    uri: Vec<String>,
    dn: Vec<Vec<u8>>,
}

/// GeneralSubtrees (SIZE (1..MAX)); minimum and maximum must be absent (RFC 5280 4.2.1.10)
fn subtrees(b: &[u8], out: &mut Subtrees, other: &mut Vec<u8>) -> Option<()> {
    let mut d = Der::new(b);
    if d.is_empty() {
        return None;
    }
    while !d.is_empty() {
        let mut st = d.seq()?;
        let (tag, v, _) = st.next()?;
        if !st.is_empty() {
            return None;
        }
        match tag {
            0x81 => out.email.push(ascii(v)?.to_ascii_lowercase()),
            0x82 => out.dns.push(ascii(v)?.to_ascii_lowercase()),
            0x86 => out.uri.push(ascii(v)?.to_ascii_lowercase()),
            0x87 if v.len() == 8 || v.len() == 32 => out.ips.push(Vec::from(v)),
            0xa4 => out.dn.push(Vec::from(Der::new(v).read_raw(der::SEQUENCE)?)),
            0xa0 | 0xa3 | 0xa5 | 0x88 => {
                if !other.contains(&tag) {
                    other.push(tag);
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// an IA5String's bytes as text (None for bytes above 0x7f)
pub(crate) fn ascii(v: &[u8]) -> Option<String> {
    v.is_ascii().then(|| String::from_utf8_lossy(v).into_owned())
}

/// a dNSName of a subject alternative name in the preferred name syntax (RFC 5280 4.2.1.6,
/// RFC 1034 3.5 with RFC 1123 2.1): labels of letters, digits and inner hyphens, at most 63
/// bytes each and 253 in all; "*" only as the whole first label
pub(crate) fn dns_name_ok(n: &str) -> bool {
    let rest = n.strip_prefix("*.").unwrap_or(n);
    if n.is_empty() || n.len() > 253 || rest.is_empty() {
        return false;
    }
    rest.split('.').all(|l| {
        let b = l.as_bytes();
        !b.is_empty() && b.len() <= 63 && b[0] != b'-' && b[b.len() - 1] != b'-' && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    })
}

const OID_EMAIL_ATTR: &[u64] = &[1, 2, 840, 113549, 1, 9, 1];

/// the emailAddress attributes of a Name
fn emails_of(name: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut d = Der::new(name);
    while let Some(set) = d.read(der::SET) {
        let mut s = Der::new(set);
        while let Some(atv) = s.seq() {
            let mut a = atv;
            if a.read(der::OID) == Some(&der::oid_bytes(OID_EMAIL_ATTR)[..]) {
                if let Some((_, v, _)) = a.next() {
                    out.push(String::from_utf8_lossy(v).to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// the common name of a Name
pub(crate) fn cn_of(name: &[u8]) -> String {
    attr_of(name, OID_CN)
}

/// the first attribute of a type in a Name (common name, organization, ...)
fn attr_of(name: &[u8], oid: &[u64]) -> String {
    let mut d = Der::new(name);
    while let Some(set) = d.read(der::SET) {
        let mut s = Der::new(set);
        while let Some(atv) = s.seq() {
            let mut a = atv;
            if a.read(der::OID) == Some(&der::oid_bytes(oid)[..]) {
                if let Some((_, v, _)) = a.next() {
                    return String::from_utf8_lossy(v).into_owned();
                }
            }
        }
    }
    String::new()
}

/// an RSA key of a certificate: at least 2048 bits and a whole number of bytes (the CA/Browser
/// Forum's floor; crypto::rsa takes 1024 bits and more, for simulators)
fn rsa_key(n: &[u8], e: &[u8]) -> Option<crypto::rsa::PublicKey> {
    crypto::rsa::PublicKey::new(n, e).filter(|k| k.bits() >= 2048 && k.bits() % 8 == 0)
}

/// the content of a non-negative DER INTEGER -> its big-endian bytes without the sign byte;
/// None when it is empty, negative or not in the shortest form (a 00 before a byte below 0x80)
/// the bytes after the leading zero bytes of a number (all of them for 0: no bytes)
fn strip_zeros(b: &[u8]) -> &[u8] {
    &b[b.iter().position(|&x| x != 0).unwrap_or(b.len())..]
}

fn der_unsigned(b: &[u8]) -> Option<&[u8]> {
    match b {
        [] => None,
        [0, next, ..] if next & 0x80 == 0 => None,
        [0, rest @ ..] if !rest.is_empty() => Some(rest),
        [first, ..] if first & 0x80 != 0 => None,
        _ => Some(b),
    }
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
        c.subject_org = attr_of(Der::new(subj).read(der::SEQUENCE)?, OID_O);
        c.issuer_cn = cn_of(Der::new(&c.issuer).read(der::SEQUENCE)?);
        c.issuer_org = attr_of(Der::new(&c.issuer).read(der::SEQUENCE)?, OID_O);
        c.emails = emails_of(Der::new(subj).read(der::SEQUENCE)?);
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
                    // KeyPurposeId SEQUENCE SIZE (1..MAX): an empty one would allow every purpose
                    let mut k = Der::new(val).seq()?;
                    while let Some(o) = k.read(der::OID) {
                        c.eku.push(Vec::from(o));
                    }
                    if c.eku.is_empty() || !k.is_empty() {
                        return None;
                    }
                } else if is(OID_SAN) {
                    // GeneralNames SIZE (1..MAX), every entry well-formed: a name that cannot be
                    // read could not be held against a name constraint either
                    let mut k = Der::new(val).seq()?;
                    if k.is_empty() {
                        return None;
                    }
                    while !k.is_empty() {
                        let (tag, v, _) = k.next()?;
                        match tag {
                            0x82 => {
                                let n = ascii(v)?.to_ascii_lowercase();
                                if !dns_name_ok(&n) {
                                    return None;
                                }
                                c.dns.push(n);
                            }
                            0x87 if v.len() == 4 => c.ips.push(IpAddr::V4([v[0], v[1], v[2], v[3]])),
                            0x87 if v.len() == 16 => {
                                let mut a = [0u8; 16];
                                a.copy_from_slice(v);
                                c.ips.push(IpAddr::V6(a));
                            }
                            0x81 => c.emails.push(ascii(v)?.to_ascii_lowercase()),
                            0x86 => c.uris.push(ascii(v)?.to_ascii_lowercase()),
                            0xa4 => c.dirs.push(Vec::from(Der::new(v).read_raw(der::SEQUENCE)?)),
                            0xa0 | 0xa3 | 0xa5 | 0x88 => c.other_names.push(tag),
                            _ => return None,
                        }
                    }
                } else if is(OID_SKI) {
                    if critical {
                        return None; // MUST be non-critical (RFC 5280 4.2.1.2)
                    }
                    c.ski = Vec::from(Der::new(val).read(der::OCTET_STRING)?);
                } else if is(OID_NAME_CONSTRAINTS) {
                    // permittedSubtrees or excludedSubtrees MUST be present (RFC 5280 4.2.1.10)
                    let mut k = Der::new(val).seq()?;
                    let (p, x) = (k.optional(0xa0), k.optional(0xa1));
                    if (p.is_none() && x.is_none()) || !k.is_empty() {
                        return None;
                    }
                    let (mut pt, mut xt, mut other) = (Subtrees::default(), Subtrees::default(), Vec::new());
                    if let Some(p) = p {
                        subtrees(p, &mut pt, &mut other)?;
                    }
                    if let Some(x) = x {
                        subtrees(x, &mut xt, &mut other)?;
                    }
                    c.name_constraints = Some(NameConstraints {
                        permitted_dns: pt.dns,
                        excluded_dns: xt.dns,
                        permitted_ip: pt.ips,
                        excluded_ip: xt.ips,
                        permitted_email: pt.email,
                        excluded_email: xt.email,
                        permitted_uri: pt.uri,
                        excluded_uri: xt.uri,
                        permitted_dn: pt.dn,
                        excluded_dn: xt.dn,
                        other_forms: other,
                    });
                } else if is(OID_AKI) && critical {
                    return None; // MUST be non-critical (RFC 5280 4.2.1.1)
                } else if is(OID_AKI) {
                    // keyIdentifier [0], when there is one; the rest is not looked at
                    if let Some(mut k) = Der::new(val).seq() {
                        if let Some(id) = k.optional(0x80) {
                            c.aki = Vec::from(id);
                        }
                    }
                } else if is(OID_AIA) {
                    // AuthorityInfoAccessSyntax: SEQUENCE SIZE (1..MAX) OF AccessDescription
                    // (accessMethod, accessLocation); MUST be non-critical (RFC 5280 4.2.2.1)
                    if critical {
                        return None;
                    }
                    let mut k = Der::new(val).seq()?;
                    if k.is_empty() {
                        return None;
                    }
                    while !k.is_empty() {
                        let mut ad = k.seq()?;
                        let method = ad.read(der::OID)?;
                        let (tag, loc, _) = ad.next()?;
                        if !ad.is_empty() {
                            return None;
                        }
                        if tag == 0x86 {
                            let uri = ascii(loc)?;
                            if oid_is(method, OID_AD_OCSP) {
                                c.ocsp_urls.push(uri);
                            } else if oid_is(method, OID_AD_CA_ISSUERS) {
                                c.ca_issuers.push(uri);
                            }
                        }
                    }
                } else if is(OID_TLS_FEATURE) {
                    // Features ::= SEQUENCE OF INTEGER (RFC 7633 6), each a TLS extension type
                    let mut k = Der::new(val).seq()?;
                    while !k.is_empty() {
                        let v = k.u64()?;
                        c.tls_features.push(u16::try_from(v).ok()?);
                    }
                } else if is(OID_OCSP_NOCHECK) {
                    // its value is NULL
                    if val != [der::NULL, 0] {
                        return None;
                    }
                    c.ocsp_nocheck = true;
                } else if is(OID_POLICIES) || is(OID_POLICY_CONSTRAINTS) || is(OID_INHIBIT_ANY) {
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

    /// its SubjectPublicKeyInfo as it stands in the certificate (DER): what keys are pinned by
    /// and a certificate request carries
    pub fn spki(&self) -> Vec<u8> {
        self.spki_parts().map(|(whole, _)| Vec::from(whole)).unwrap_or_default()
    }

    /// the SubjectPublicKeyInfo and the bytes of its subjectPublicKey (the BIT STRING's)
    pub(crate) fn spki_parts(&self) -> Option<(&[u8], &[u8])> {
        let mut t = Der::new(&self.tbs).seq()?;
        t.optional(0xa0);
        for tag in [der::INTEGER, der::SEQUENCE, der::SEQUENCE, der::SEQUENCE, der::SEQUENCE] {
            t.read(tag)?;
        }
        let whole = t.read_raw(der::SEQUENCE)?;
        let mut s = Der::new(whole).seq()?;
        s.read(der::SEQUENCE)?;
        Some((whole, s.bits()?))
    }

    /// whether its extended key usage allows signing OCSP responses (id-kp-OCSPSigning: a
    /// responder its CA delegated to, RFC 6960 4.2.2.2)
    pub fn ocsp_signer(&self) -> bool {
        let want = der::oid_bytes(OID_OCSP_SIGNING);
        self.eku.iter().any(|e| *e == want)
    }

    /// whether it must come with a stapled OCSP response (RFC 7633)
    pub fn must_staple(&self) -> bool {
        self.tls_features.contains(&TLS_FEATURE_STATUS_REQUEST)
    }

    /// SHA-256 of the certificate
    pub fn fingerprint_bytes(&self) -> [u8; 32] {
        crypto::sha256(&self.der)
    }

    /// the fingerprint in hex with colons, as certificate dialogs write it
    pub fn fingerprint_text(&self) -> String {
        let v: Vec<String> = self.fingerprint_bytes().iter().map(|b| format!("{:02X}", b)).collect();
        v.join(":")
    }

    /// it names itself as issuer and its own key made the signature
    pub fn self_signed(&self) -> bool {
        self.signed_by(self)
    }

    /// its Ed25519 key (None for another kind)
    pub fn ed25519_key(&self) -> Option<[u8; 32]> {
        match &self.key {
            PubKey::Ed25519(k) => Some(*k),
            _ => None,
        }
    }

    /// what its names are, for a dialog: the DNS names, then the addresses
    pub fn name_list(&self) -> Vec<String> {
        let mut v = self.dns.clone();
        for ip in &self.ips {
            v.push(format!("{}", ip));
        }
        v
    }

    /// bytes it holds (an estimate for a memory budget)
    pub fn bytes(&self) -> usize {
        self.der.len() + self.tbs.len() + self.subject.len() + self.issuer.len() + self.signature.len() + 256
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
            (PubKey::Rsa { n, e }, SigAlg::RsaPkcs1(h)) => rsa_key(n, e).is_some_and(|k| k.verify_pkcs1(h, &h.digest(data), sig)),
            (PubKey::Rsa { n, e }, SigAlg::RsaPss(h, salt)) => rsa_key(n, e).is_some_and(|k| k.verify_pss(h, &h.digest(data), sig, salt)),
            (PubKey::Ec { curve, point }, SigAlg::Ecdsa(h)) => {
                // Ecdsa-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }, strictly DER: nothing
                // after the SEQUENCE or in it after s, both integers in their shortest form, so a
                // signature has one encoding (Wycheproof's InvalidEncoding cases)
                let rs = || -> Option<(&[u8], &[u8])> {
                    let mut outer = Der::new(sig);
                    let mut d = outer.seq()?;
                    let r = der_unsigned(d.read(der::INTEGER)?)?;
                    let s = der_unsigned(d.read(der::INTEGER)?)?;
                    (outer.is_empty() && d.is_empty()).then_some((r, s))
                };
                rs().is_some_and(|(r, s)| crypto::ecdsa::verify(*curve, point, &h.digest(data), r, s))
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

    // a CA on the path whose extended key usage names purposes passes on only those (or any
    // purpose: anyExtendedKeyUsage); without the extension it does not restrict
    fn ca_eku_allows(&self, u: Usage) -> bool {
        let any = der::oid_bytes(OID_ANY_EKU);
        self.eku_allows(u) || self.eku.iter().any(|e| *e == any)
    }

    /// whether the certificate names this host (DNS name, case-insensitive; IP address)
    pub fn names(&self, host: &str) -> bool {
        if let Some(ip) = IpAddr::parse(host.trim_matches(['[', ']'])) {
            return self.ips.contains(&ip);
        }
        let h = host.to_ascii_lowercase();
        let h = h.strip_suffix('.').unwrap_or(&h);
        self.dns.iter().any(|d| {
            d == h
                || d.strip_prefix("*.").is_some_and(|base| {
                    base.contains('.') && h.split_once('.').is_some_and(|(label, rest)| !label.is_empty() && rest == base)
                })
        })
    }
}

/// whether a DNS name lies in a constraint subtree ("example.com": it and its subdomains;
/// ".example.com": subdomains only)
fn dns_in(name: &str, base: &str) -> bool {
    if base.is_empty() {
        return true;
    }
    if let Some(b) = base.strip_prefix('.') {
        return name.ends_with(base) && name.len() > b.len();
    }
    name == base || below(name, base)
}

// name is a subdomain of base ("a.b.c" of "b.c")
fn below(name: &str, base: &str) -> bool {
    name.len() > base.len() && name.ends_with(base) && name.as_bytes()[name.len() - base.len() - 1] == b'.'
}

/// a wildcard "*.d" stands for every name one label below d: whether every one of them lies
/// in a constraint subtree (every: a permitted one must hold them all) or some of them does
/// (an excluded one must hold none: "*.example.com" stands for "host.example.com" too)
fn wild_in(d: &str, base: &str, every: bool) -> bool {
    if base.is_empty() {
        return true;
    }
    let b = base.strip_prefix('.').unwrap_or(base);
    // x.d lies below b for every x when d is b or below it (also in ".b": x.d is a subdomain)
    if d == b || below(d, b) {
        return true;
    }
    // a plain base one label below d is one of the names
    !every && !base.starts_with('.') && below(b, d) && !b[..b.len() - d.len() - 1].contains('.')
}

fn ip_in(ip: &IpAddr, range: &[u8]) -> bool {
    let a: &[u8] = match ip {
        IpAddr::V4(a) => a,
        IpAddr::V6(a) => a,
    };
    let half = range.len() / 2;
    a.len() == half && a.iter().zip(&range[..half]).zip(&range[half..]).all(|((x, net), m)| x & m == net & m)
}

/// the RDNs of a Name (the DER of a SEQUENCE of SETs), each as its whole DER
fn rdns(name: &[u8]) -> Option<Vec<&[u8]>> {
    let mut d = Der::new(name).seq()?;
    let mut out = Vec::new();
    while !d.is_empty() {
        out.push(d.read_raw(der::SET)?);
    }
    Some(out)
}

/// whether a Name lies in a directoryName subtree: it begins with the subtree's RDNs, byte for
/// byte (no string preparation)
fn dn_in(name: &[u8], base: &[u8]) -> bool {
    match (rdns(name), rdns(base)) {
        (Some(n), Some(b)) => b.len() <= n.len() && n.iter().zip(&b).all(|(x, y)| x == y),
        _ => false,
    }
}

/// whether an e-mail address lies in an rfc822Name subtree: the mailbox itself, every mailbox
/// of a host ("example.com"), or of the subdomains of a domain (".example.com")
fn email_in(addr: &str, base: &str) -> bool {
    if base.contains('@') {
        return addr == base;
    }
    let Some((_, host)) = addr.rsplit_once('@') else { return false };
    match base.strip_prefix('.') {
        Some(_) => host.ends_with(base),
        None => host == base,
    }
}

/// the host of a URI ("scheme://[user@]host[:port]/..."); None without an authority or with an
/// address instead of a name (RFC 5280: such a URI cannot pass a URI constraint)
fn uri_host(uri: &str) -> Option<&str> {
    let rest = uri.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let host = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    if host.starts_with('[') {
        return None;
    }
    let host = host.split(':').next()?;
    (!host.is_empty() && IpAddr::parse(host).is_none()).then_some(host)
}

/// whether a URI's host lies in a uniformResourceIdentifier subtree
fn uri_in(host: &str, base: &str) -> bool {
    match base.strip_prefix('.') {
        Some(_) => host.ends_with(base),
        None => host == base,
    }
}

/// a certificate's names against the name constraints of a CA above it on the path
fn constraints_allow(c: &Cert, nc: &NameConstraints) -> bool {
    // permitted: every name of a form must lie in one subtree of that form (when there are
    // any); excluded: none may lie in one
    fn check<T>(names: &[T], permitted: &[String], excluded: &[String], within: impl Fn(&T, &str, bool) -> bool) -> bool {
        names.iter().all(|n| !excluded.iter().any(|b| within(n, b, false)) && (permitted.is_empty() || permitted.iter().any(|b| within(n, b, true))))
    }
    // a wildcard is checked for the names it stands for, not for its base alone
    let dns_ok = check(&c.dns, &nc.permitted_dns, &nc.excluded_dns, |d: &String, b: &str, every| match d.strip_prefix("*.") {
        Some(w) => wild_in(w, b, every),
        None => dns_in(d, b),
    });
    let ip_ok = c.ips.iter().all(|ip| !nc.excluded_ip.iter().any(|r| ip_in(ip, r)) && (nc.permitted_ip.is_empty() || nc.permitted_ip.iter().any(|r| ip_in(ip, r))));
    let email_ok = check(&c.emails, &nc.permitted_email, &nc.excluded_email, |e: &String, b: &str, _| email_in(e, b));
    let uri_ok = if nc.permitted_uri.is_empty() && nc.excluded_uri.is_empty() {
        true
    } else {
        c.uris.iter().all(|u| match uri_host(u) {
            Some(h) => !nc.excluded_uri.iter().any(|b| uri_in(h, b)) && (nc.permitted_uri.is_empty() || nc.permitted_uri.iter().any(|b| uri_in(h, b))),
            None => false,
        })
    };
    // directory names: the subject (when it has RDNs) and the directoryName alternative names
    let subject_has_rdns = rdns(&c.subject).is_some_and(|r| !r.is_empty());
    let dn_ok = (!subject_has_rdns || dn_ok_one(&c.subject, nc)) && c.dirs.iter().all(|n| dn_ok_one(n, nc));
    // a form constrained here that this code does not process: refused (RFC 5280 4.2.1.10)
    let other_ok = !c.other_names.iter().any(|t| nc.other_forms.contains(t));
    dns_ok && ip_ok && email_ok && uri_ok && dn_ok && other_ok
}

fn dn_ok_one(name: &[u8], nc: &NameConstraints) -> bool {
    !nc.excluded_dn.iter().any(|b| dn_in(name, b)) && (nc.permitted_dn.is_empty() || nc.permitted_dn.iter().any(|b| dn_in(name, b)))
}

/// a Name without RDNs (a CA's subject MUST NOT be one, RFC 5280 4.1.2.6)
fn empty_name(name: &[u8]) -> bool {
    rdns(name).is_none_or(|r| r.is_empty())
}

/// a certificate whose issuer and subject are the same name (RFC 5280 6.1: self-issued)
fn self_issued(c: &Cert) -> bool {
    c.issuer == c.subject
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
    // keyCertSign only with cA, name constraints only in CAs (RFC 5280 4.2.1.9, 4.2.1.10)
    if (leaf.key_usage.is_some_and(|k| k & KU_KEY_CERT_SIGN != 0) && !leaf.is_ca) || (leaf.name_constraints.is_some() && !leaf.is_ca) {
        return Err(String::from("Zertifikat mit Erweiterungen einer Zertifizierungsstelle"));
    }
    if let Some(n) = name {
        if !leaf.names(n) {
            return Err(format!("Zertifikat gilt nicht für {}", n));
        }
    }
    let mut why = String::from("keine vertrauenswürdige Zertifizierungsstelle");
    let mut used = alloc::vec![false; chain.len()];
    let mut below = alloc::vec![0usize];
    let mut budget = 64u32;
    if path(chain, anchors, now, usage, &mut used, &mut below, &mut budget, &mut why) { Ok(()) } else { Err(why) }
}

/// whether a CA may sign above the certificates of `below` (indices into chain, leaf first):
/// its path length counts the intermediates that are not self-issued (RFC 5280 6.1.4 (l)), its
/// name constraints apply to the leaf and to those intermediates (6.1.3 (b))
fn ca_admits(ca: &Cert, chain: &[Cert], below: &[usize]) -> Result<(), &'static str> {
    let intermediates = below[1..].iter().filter(|&&i| !self_issued(&chain[i])).count();
    if ca.path_len.is_some_and(|p| (p as usize) < intermediates) {
        return Err("die Zertifikatskette ist länger, als die Zertifizierungsstelle erlaubt");
    }
    if let Some(nc) = &ca.name_constraints {
        let named = below.iter().enumerate().filter(|&(k, &i)| k == 0 || !self_issued(&chain[i]));
        if !named.into_iter().all(|(_, &i)| constraints_allow(&chain[i], nc)) {
            return Err("der Name ist für diese Zertifizierungsstelle nicht erlaubt");
        }
    }
    Ok(())
}

/// a path upwards from the last certificate of `below`
#[allow(clippy::too_many_arguments)]
fn path(chain: &[Cert], anchors: &[Cert], now: i64, usage: Usage, used: &mut [bool], below: &mut Vec<usize>, budget: &mut u32, why: &mut String) -> bool {
    if below.len() > 9 {
        *why = String::from("Zertifikatskette zu lang");
        return false;
    }
    let cur = &chain[*below.last().unwrap_or(&0)];
    for a in anchors.iter().filter(|a| a.subject == cur.issuer) {
        if !cur.signed_by(a) {
            continue;
        }
        if now < a.not_before || now > a.not_after {
            *why = String::from("die Zertifizierungsstelle ist abgelaufen");
            continue;
        }
        if !(a.is_ca || a.version == 1) || a.unknown_critical || a.key_usage.is_some_and(|k| k & KU_KEY_CERT_SIGN == 0) || empty_name(&a.subject) {
            *why = String::from("Vertrauensanker ist keine Zertifizierungsstelle");
            continue;
        }
        if let Err(e) = ca_admits(a, chain, below) {
            *why = String::from(e);
            continue;
        }
        if !a.ca_eku_allows(usage) {
            *why = String::from("die Zertifizierungsstelle ist nicht für diesen Zweck");
            continue;
        }
        return true;
    }
    for i in 1..chain.len() {
        let next = &chain[i];
        if used[i] || next.subject != cur.issuer {
            continue;
        }
        // certificates with equal names and keys would otherwise make this a factorial search
        // of signature checks
        if *budget == 0 {
            *why = String::from("Zertifikatskette zu komplex");
            return false;
        }
        *budget -= 1;
        if !cur.signed_by(next) {
            continue;
        }
        if !next.is_ca || next.unknown_critical || now < next.not_before || now > next.not_after || next.key_usage.is_some_and(|k| k & KU_KEY_CERT_SIGN == 0) || empty_name(&next.subject) {
            *why = String::from("ungültige Zwischenzertifizierungsstelle");
            continue;
        }
        if let Err(e) = ca_admits(next, chain, below) {
            *why = String::from(e);
            continue;
        }
        // a CA whose extended key usage names other purposes cannot certify this one
        if !next.ca_eku_allows(usage) {
            *why = String::from("die Zertifizierungsstelle ist nicht für diesen Zweck");
            continue;
        }
        used[i] = true;
        below.push(i);
        if path(chain, anchors, now, usage, used, below, budget, why) {
            return true;
        }
        below.pop();
        used[i] = false;
    }
    false
}

pub(crate) fn name(cn: &str, org: &str) -> Vec<u8> {
    let mut rdns = Vec::new();
    if !org.is_empty() {
        rdns.push(der::set(&[&der::seq(&[&der::oid(OID_O), &der::utf8(org)])]));
    }
    rdns.push(der::set(&[&der::seq(&[&der::oid(OID_CN), &der::utf8(cn)])]));
    let parts: Vec<&[u8]> = rdns.iter().map(|v| v.as_slice()).collect();
    der::seq(&parts)
}

pub(crate) fn ext(oid: &[u64], critical: bool, value: &[u8]) -> Vec<u8> {
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
    /// the subject's key as a SubjectPublicKeyInfo (PrivateKey::spki, spki_of); empty: the
    /// Ed25519 key `public_key`
    pub spki: Vec<u8>,
    /// OCSP responders to name in the authority information access (http URIs)
    pub ocsp_urls: Vec<String>,
    /// a leaf that must come with a stapled OCSP response (the TLS feature status_request,
    /// RFC 7633)
    pub must_staple: bool,
    /// a leaf that signs OCSP responses for its issuer (id-kp-OCSPSigning, with
    /// id-pkix-ocsp-nocheck)
    pub ocsp_signing: bool,
}

/// the AlgorithmIdentifier of a signature (None for PSS, which certificates here do not use)
pub(crate) fn alg_id(alg: SigAlg) -> Option<Vec<u8>> {
    let hash_oid = |h: HashAlg, rsa: bool| match (h, rsa) {
        (HashAlg::Sha256, true) => Some(OID_RSA_SHA256),
        (HashAlg::Sha384, true) => Some(OID_RSA_SHA384),
        (HashAlg::Sha512, true) => Some(OID_RSA_SHA512),
        (HashAlg::Sha256, false) => Some(OID_ECDSA_SHA256),
        (HashAlg::Sha384, false) => Some(OID_ECDSA_SHA384),
        (HashAlg::Sha512, false) => Some(OID_ECDSA_SHA512),
        _ => None,
    };
    match alg {
        SigAlg::Ed25519 => Some(der::seq(&[&der::oid(OID_ED25519)])),
        SigAlg::Ecdsa(h) => Some(der::seq(&[&der::oid(hash_oid(h, false)?)])),
        SigAlg::RsaPkcs1(h) => Some(der::seq(&[&der::oid(hash_oid(h, true)?), &[der::NULL, 0]])),
        _ => None,
    }
}

/// a DER Extension (oid, the critical flag, the DER value) for CertBuilder::sign_ext
pub fn extension(oid: &[u64], critical: bool, value: &[u8]) -> Vec<u8> {
    ext(oid, critical, value)
}

impl CertBuilder {
    /// the DER certificate, signed by `key`; `issuer`: the issuing certificate (None: self
    /// signed with the builder's own name)
    pub fn sign(&self, issuer: Option<&Cert>, key: &Keypair) -> Vec<u8> {
        self.sign_ext(issuer, key, &[])
    }

    /// sign with further extensions (each one from `extension`: name constraints, a CA's
    /// extended key usage ...); one with the OID of an extension the builder writes itself
    /// replaces that one (basic constraints without a path length, other key usages)
    pub fn sign_ext(&self, issuer: Option<&Cert>, key: &Keypair, extra: &[Vec<u8>]) -> Vec<u8> {
        let alg = der::seq(&[&der::oid(OID_ED25519)]);
        self.build(issuer, &alg, extra, &mut |tbs| Some(Vec::from(&key.sign(tbs)[..]))).unwrap_or_default()
    }

    /// the DER certificate for the key `spki` (any key), signed by `key` (any key: ECDSA with
    /// the hash of its curve, RSA with PKCS #1 v1.5 and SHA-256, Ed25519) with further
    /// extensions as sign_ext; None when `spki` is not a SubjectPublicKeyInfo
    pub fn sign_key(&self, issuer: Option<&Cert>, key: &PrivateKey, extra: &[Vec<u8>]) -> Option<Vec<u8>> {
        let alg = key.cert_alg();
        let id = alg_id(alg)?;
        self.build(issuer, &id, extra, &mut |tbs| key.sign(alg, tbs, &[]))
    }

    fn build(&self, issuer: Option<&Cert>, alg: &[u8], extra: &[Vec<u8>], sign: &mut dyn FnMut(&[u8]) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
        let mut serial = self.serial;
        serial[0] &= 0x7f; // positive
        let subject = name(&self.cn, &self.org);
        let issuer_name = issuer.map_or(subject.clone(), |i| i.subject.clone());
        let validity = der::seq(&[&encode_time(self.not_before), &encode_time(self.not_after)]);
        let (spki, key_bits) = if self.spki.is_empty() {
            (der::seq(&[&der::seq(&[&der::oid(OID_ED25519)]), &der::bits(&self.public_key)]), Vec::from(&self.public_key[..]))
        } else {
            let mut d = Der::new(&self.spki).seq()?;
            d.read(der::SEQUENCE)?;
            let bits = Vec::from(d.bits()?);
            (self.spki.clone(), bits)
        };
        let ski = crypto::sha256(&key_bits);
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
            if self.ocsp_signing {
                eku.push(der::oid(OID_OCSP_SIGNING));
                exts.push(ext(OID_OCSP_NOCHECK, false, &[der::NULL, 0]));
            }
            if self.must_staple {
                exts.push(ext(OID_TLS_FEATURE, false, &der::seq(&[&der::small_int(TLS_FEATURE_STATUS_REQUEST as u64)])));
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
        if !self.ocsp_urls.is_empty() {
            let ads: Vec<Vec<u8>> = self.ocsp_urls.iter().map(|u| der::seq(&[&der::oid(OID_AD_OCSP), &der::tlv(0x86, u.as_bytes())])).collect();
            let parts: Vec<&[u8]> = ads.iter().map(|v| v.as_slice()).collect();
            exts.push(ext(OID_AIA, false, &der::seq(&parts)));
        }
        exts.push(ext(OID_SKI, false, &der::octets(&ski[..20])));
        let aki_src = issuer.map_or(ski[..20].to_vec(), |i| if i.ski.is_empty() { crypto::sha256(&i.public_key)[..20].to_vec() } else { i.ski.clone() });
        exts.push(ext(OID_AKI, false, &der::seq(&[&der::tlv(0x80, &aki_src)])));
        let oid_of = |e: &[u8]| Der::new(e).seq().and_then(|mut d| d.read(der::OID).map(Vec::from));
        let replaced: Vec<Vec<u8>> = extra.iter().filter_map(|e| oid_of(e)).collect();
        exts.retain(|e| oid_of(e).is_none_or(|o| !replaced.contains(&o)));
        exts.extend(extra.iter().cloned());
        let ext_parts: Vec<&[u8]> = exts.iter().map(|v| v.as_slice()).collect();
        let tbs = der::seq(&[
            &der::explicit(0, &der::small_int(2)),
            &der::int_u(&serial),
            alg,
            &issuer_name,
            &validity,
            &subject,
            &spki,
            &der::explicit(3, &der::seq(&ext_parts)),
        ]);
        let sig = sign(&tbs)?;
        Some(der::seq(&[&tbs, alg, &der::bits(&sig)]))
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
