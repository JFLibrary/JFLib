// ocsp -- the Online Certificate Status Protocol (RFC 6960, the lightweight profile of RFC
// 5019): what TLS staples (RFC 6066 8, RFC 8446 4.4.2.1) and what a private CA answers
//
//   CertId::new(&cert, &issuer)             the certificate's id (SHA-1, as RFC 5019 asks;
//                                           CertId::with_hash for SHA-256 and longer)
//   request(&[id], nonce) -> DER            an OCSPRequest (unsigned, RFC 6960 4.1)
//   Request::parse(der)                     the ids and the nonce of a request (a responder)
//   check(der, &cert, &issuer, now) -> Result<Response, String>
//                                           a response about `cert`: signed by its issuer or by
//                                           a responder the issuer delegated to, current at
//                                           `now`; Response { status, this_update, .. }
//   ResponseBuilder { status, .. }.sign(&cert, &issuer, responder, &key) -> DER
//   error_response(code) -> DER             an unsuccessful one (tryLater, unauthorized, ..)
//
// A response is current from thisUpdate (5 minutes of clock skew allowed) to nextUpdate (and
// 5 minutes after it); one without nextUpdate for a day after thisUpdate, as browsers take it.
// Delegated responders (RFC 6960 4.2.2.2) must be issued directly by the issuer, carry
// id-kp-OCSPSigning and be valid; their own revocation is not checked (they carry
// id-pkix-ocsp-nocheck as a rule).  Unknown critical extensions refuse a response.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::der::{self, Der};
use crate::{oid_is, parse_time, sig_alg, Cert, PrivateKey, SigAlg};

const OID_SHA1: &[u64] = &[1, 3, 14, 3, 2, 26];
const OID_SHA256: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 1];
const OID_SHA384: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 2];
const OID_SHA512: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 3];
const OID_BASIC: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 48, 1, 1];
const OID_NONCE: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 48, 1, 2];

/// the clock skew taken at either end of a response's time
pub const SKEW: i64 = 300;
/// how long a response without nextUpdate counts as current after its thisUpdate
pub const NO_NEXT_UPDATE: i64 = 86_400;

/// the response statuses of an unsuccessful OCSPResponse (RFC 6960 4.2.1)
pub const MALFORMED_REQUEST: u8 = 1;
pub const INTERNAL_ERROR: u8 = 2;
pub const TRY_LATER: u8 = 3;
pub const SIG_REQUIRED: u8 = 5;
pub const UNAUTHORIZED: u8 = 6;

/// the hash of a CertID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdHash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl IdHash {
    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            IdHash::Sha1 => Vec::from(&crypto::sha1(data)[..]),
            IdHash::Sha256 => crypto::HashAlg::Sha256.digest(data),
            IdHash::Sha384 => crypto::HashAlg::Sha384.digest(data),
            IdHash::Sha512 => crypto::HashAlg::Sha512.digest(data),
        }
    }

    fn arcs(self) -> &'static [u64] {
        match self {
            IdHash::Sha1 => OID_SHA1,
            IdHash::Sha256 => OID_SHA256,
            IdHash::Sha384 => OID_SHA384,
            IdHash::Sha512 => OID_SHA512,
        }
    }

    /// an AlgorithmIdentifier's content (parameters absent or NULL)
    fn of(alg: &[u8]) -> Option<IdHash> {
        let mut d = Der::new(alg);
        let o = d.read(der::OID)?;
        d.optional(der::NULL);
        if !d.is_empty() {
            return None;
        }
        [IdHash::Sha1, IdHash::Sha256, IdHash::Sha384, IdHash::Sha512].into_iter().find(|h| oid_is(o, h.arcs()))
    }
}

/// which certificate a request or response is about (RFC 6960 4.1.1)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertId {
    pub hash: IdHash,
    /// the hash of the issuer's name (its DER)
    pub name_hash: Vec<u8>,
    /// the hash of the issuer's public key (the bytes of its subjectPublicKey)
    pub key_hash: Vec<u8>,
    /// the certificate's serial number (the INTEGER's content)
    pub serial: Vec<u8>,
}

impl CertId {
    /// the id of `cert` with SHA-1 (RFC 5019 2.1.1: what responders take); None when `issuer` is
    /// not the one that names it or its key cannot be read
    pub fn new(cert: &Cert, issuer: &Cert) -> Option<CertId> {
        CertId::with_hash(cert, issuer, IdHash::Sha1)
    }

    pub fn with_hash(cert: &Cert, issuer: &Cert, hash: IdHash) -> Option<CertId> {
        if cert.issuer != issuer.subject {
            return None;
        }
        let (_, key) = issuer.spki_parts()?;
        Some(CertId { hash, name_hash: hash.digest(&issuer.subject), key_hash: hash.digest(key), serial: cert.serial.clone() })
    }

    fn encode(&self) -> Vec<u8> {
        der::seq(&[
            &der::seq(&[&der::oid(self.hash.arcs()), &[der::NULL, 0]]),
            &der::octets(&self.name_hash),
            &der::octets(&self.key_hash),
            &der::tlv(der::INTEGER, &self.serial),
        ])
    }

    fn parse(d: &mut Der) -> Option<CertId> {
        let mut c = d.seq()?;
        let hash = IdHash::of(c.read(der::SEQUENCE)?)?;
        let name_hash = Vec::from(c.read(der::OCTET_STRING)?);
        let key_hash = Vec::from(c.read(der::OCTET_STRING)?);
        let serial = Vec::from(c.read(der::INTEGER)?);
        (c.is_empty() && !serial.is_empty()).then_some(CertId { hash, name_hash, key_hash, serial })
    }
}

/// an Extension's DER of the nonce (RFC 8954: an OCTET STRING of 1 to 32 bytes)
fn nonce_ext(nonce: &[u8]) -> Vec<u8> {
    der::seq(&[&der::oid(OID_NONCE), &der::octets(&der::octets(nonce))])
}

/// an OCSPRequest for these certificates, unsigned; `nonce` (1 to 32 bytes) binds the answer
/// to it (responders of RFC 5019 answer from a cache and leave it out)
pub fn request(ids: &[CertId], nonce: Option<&[u8]>) -> Vec<u8> {
    let reqs: Vec<Vec<u8>> = ids.iter().map(|id| der::seq(&[&id.encode()])).collect();
    let parts: Vec<&[u8]> = reqs.iter().map(|v| v.as_slice()).collect();
    let list = der::seq(&parts);
    let tbs = match nonce {
        Some(n) => der::seq(&[&list, &der::explicit(2, &der::seq(&[&nonce_ext(n)]))]),
        None => der::seq(&[&list]),
    };
    der::seq(&[&tbs])
}

/// what a request asks (for a responder)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub ids: Vec<CertId>,
    pub nonce: Option<Vec<u8>>,
}

impl Request {
    /// an OCSPRequest: version 1, no requestor name, no signature (a signed one is not taken:
    /// None), at most 64 certificates
    pub fn parse(b: &[u8]) -> Option<Request> {
        let mut top = Der::new(b);
        let mut req = top.seq()?;
        let mut tbs = req.seq()?;
        if !top.is_empty() || !req.is_empty() {
            return None;
        }
        if let Some(v) = tbs.optional(0xa0) {
            if Der::new(v).u64()? != 0 {
                return None;
            }
        }
        if tbs.peek_tag() == Some(0xa1) {
            return None; // requestorName: a signed request
        }
        let mut list = tbs.seq()?;
        let mut ids = Vec::new();
        while !list.is_empty() {
            let mut one = list.seq()?;
            ids.push(CertId::parse(&mut one)?);
            one.optional(0xa0);
            if !one.is_empty() || ids.len() > 64 {
                return None;
            }
        }
        let mut nonce = None;
        if let Some(e) = tbs.optional(0xa2) {
            nonce = find_nonce(e)?;
        }
        (tbs.is_empty() && !ids.is_empty()).then_some(Request { ids, nonce })
    }
}

/// the nonce of an [n] EXPLICIT Extensions; None: malformed, or an unknown critical extension
fn find_nonce(e: &[u8]) -> Option<Option<Vec<u8>>> {
    let mut nonce = None;
    let mut exts = Der::new(e).seq()?;
    while !exts.is_empty() {
        let mut x = exts.seq()?;
        let id = x.read(der::OID)?;
        let critical = if x.peek_tag() == Some(der::BOOLEAN) { x.bool()? } else { false };
        let val = x.read(der::OCTET_STRING)?;
        if !x.is_empty() {
            return None;
        }
        if oid_is(id, OID_NONCE) {
            // RFC 8954 2.1: an OCTET STRING; older responders put the bytes in directly
            let mut d = Der::new(val);
            let inner = d.read(der::OCTET_STRING).filter(|_| d.is_empty());
            nonce = Some(Vec::from(inner.unwrap_or(val)));
        } else if critical {
            return None;
        }
    }
    Some(nonce)
}

/// the status of a certificate
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Good,
    /// since `time` (Unix seconds), for `reason` (CRLReason, RFC 5280 5.3.1) when given
    Revoked { time: i64, reason: Option<u8> },
    /// the responder does not know the certificate
    Unknown,
}

/// a checked response
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: Status,
    /// Unix seconds
    pub this_update: i64,
    pub next_update: Option<i64>,
    pub produced_at: i64,
    /// the nonce it carries (compare it with the request's)
    pub nonce: Option<Vec<u8>>,
}

impl Response {
    /// until when it counts as current (nextUpdate, or a day after thisUpdate)
    pub fn expires(&self) -> i64 {
        self.next_update.unwrap_or(self.this_update + NO_NEXT_UPDATE)
    }

    /// when a server fetches the next one: halfway between thisUpdate and the end (as RFC 5019
    /// 6.2 and the CAs advise), at least an hour before the end
    pub fn refresh_at(&self) -> i64 {
        let end = self.expires();
        (self.this_update + (end - self.this_update) / 2).min(end - 3600).max(self.this_update)
    }
}

/// a GeneralizedTime of OCSP: "YYYYMMDDHHMMSSZ", fractions of a second dropped (RFC 6960 takes
/// RFC 5280's form, some responders add them)
fn gen_time(d: &mut Der) -> Option<i64> {
    let c = d.read(der::GEN_TIME)?;
    if c.len() > 16 && c[14] == b'.' && c.last() == Some(&b'Z') && c[15..c.len() - 1].iter().all(|x| x.is_ascii_digit()) {
        let mut v = Vec::from(&c[..14]);
        v.push(b'Z');
        return parse_time(der::GEN_TIME, &v);
    }
    parse_time(der::GEN_TIME, c)
}

fn encode_gen_time(t: i64) -> Vec<u8> {
    let (y, m, d) = crate::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    der::tlv(der::GEN_TIME, format!("{:04}{:02}{:02}{:02}{:02}{:02}Z", y, m, d, s / 3600, s / 60 % 60, s % 60).as_bytes())
}

/// whether a ResponderID names this certificate
fn names_responder(rid: (u8, &[u8]), c: &Cert) -> bool {
    match rid {
        (0xa1, name) => name == c.subject.as_slice(),
        (0xa2, key) => Der::new(key).read(der::OCTET_STRING).is_some_and(|h| c.spki_parts().is_some_and(|(_, k)| crypto::ct_eq(h, &crypto::sha1(k)))),
        _ => false,
    }
}

/// a response about `cert`, which `issuer` issued, at `now` (Unix seconds): its structure, its
/// signature (the issuer, or a responder certificate in it that the issuer issued for OCSP), the
/// entry for `cert`, its time.  Ok with the status (a revoked one too: the caller decides);
/// Err why it cannot be used (an unsuccessful response, tryLater among them)
pub fn check(b: &[u8], cert: &Cert, issuer: &Cert, now: i64) -> Result<Response, String> {
    let bad = || String::from("malformed OCSP response");
    let mut top = Der::new(b);
    let mut resp = top.seq().ok_or_else(bad)?;
    if !top.is_empty() {
        return Err(bad());
    }
    let st = resp.read(der::ENUMERATED).ok_or_else(bad)?;
    if st != [0] {
        return Err(match st {
            [TRY_LATER] => String::from("the OCSP responder is busy (tryLater)"),
            [UNAUTHORIZED] => String::from("the OCSP responder does not answer for this certificate (unauthorized)"),
            [x] => format!("the OCSP responder refused (status {})", x),
            _ => bad(),
        });
    }
    let mut rb = Der::new(resp.read(0xa0).ok_or_else(bad)?).seq().ok_or_else(bad)?;
    if !resp.is_empty() {
        return Err(bad());
    }
    if !oid_is(rb.read(der::OID).ok_or_else(bad)?, OID_BASIC) {
        return Err(String::from("an OCSP response of another type than basic"));
    }
    let basic = rb.read(der::OCTET_STRING).ok_or_else(bad)?;
    if !rb.is_empty() {
        return Err(bad());
    }
    let mut bd = Der::new(basic);
    let mut br = bd.seq().ok_or_else(bad)?;
    if !bd.is_empty() {
        return Err(bad());
    }
    let tbs_raw = br.read_raw(der::SEQUENCE).ok_or_else(bad)?;
    let alg = sig_alg(br.read(der::SEQUENCE).ok_or_else(bad)?);
    let sig = br.bits().ok_or_else(bad)?;
    let mut certs = Vec::new();
    if let Some(c) = br.optional(0xa0) {
        let mut list = Der::new(c).seq().ok_or_else(bad)?;
        while let Some(one) = list.read_raw(der::SEQUENCE) {
            if certs.len() >= 4 {
                return Err(String::from("too many certificates in the OCSP response"));
            }
            certs.push(Cert::parse(one).ok_or_else(bad)?);
        }
        if !list.is_empty() {
            return Err(bad());
        }
    }
    if !br.is_empty() || alg == SigAlg::Unknown {
        return Err(bad());
    }
    // ResponseData
    let mut t = Der::new(tbs_raw).seq().ok_or_else(bad)?;
    if let Some(v) = t.optional(0xa0) {
        if Der::new(v).u64() != Some(0) {
            return Err(bad());
        }
    }
    let rid = match t.next() {
        Some((tag @ 0xa1, c, _)) => (tag, Der::new(c).read_raw(der::SEQUENCE).filter(|n| n.len() == c.len()).ok_or_else(bad)?),
        Some((tag @ 0xa2, c, _)) => (tag, c),
        _ => return Err(bad()),
    };
    let produced_at = gen_time(&mut t).ok_or_else(bad)?;
    let mut list = t.seq().ok_or_else(bad)?;
    let mut nonce = None;
    if let Some(e) = t.optional(0xa1) {
        nonce = find_nonce(e).ok_or_else(|| String::from("an OCSP response with an unknown critical extension"))?;
    }
    if !t.is_empty() {
        return Err(bad());
    }
    // the signer: the issuer, or a responder it issued for OCSP and named by the ResponderID
    let delegated = |c: &&Cert| c.signed_by(issuer) && c.ocsp_signer() && !c.unknown_critical && now >= c.not_before && now <= c.not_after;
    let signer = core::iter::once(issuer).chain(certs.iter().filter(delegated)).find(|c| names_responder(rid, c));
    let signer = signer.ok_or_else(|| String::from("the OCSP response is not signed by the certificate's issuer or a responder it named"))?;
    if !signer.verify(alg, tbs_raw, sig) {
        return Err(String::from("the OCSP response's signature does not verify"));
    }
    // the entry of this certificate
    if cert.issuer != issuer.subject {
        return Err(String::from("the issuer given is not the certificate's"));
    }
    let mut found = None;
    while !list.is_empty() {
        let mut one = list.seq().ok_or_else(bad)?;
        let id = CertId::parse(&mut one).ok_or_else(bad)?;
        let status = match one.next() {
            Some((0x80, [], _)) => Status::Good,
            Some((0x82, [], _)) => Status::Unknown,
            Some((0xa1, c, _)) => {
                let mut r = Der::new(c);
                let time = gen_time(&mut r).ok_or_else(bad)?;
                let reason = match r.optional(0xa0) {
                    Some(x) => match Der::new(x).read(der::ENUMERATED) {
                        Some([v]) if *v <= 10 && *v != 7 => Some(*v),
                        _ => return Err(bad()),
                    },
                    None => None,
                };
                if !r.is_empty() {
                    return Err(bad());
                }
                Status::Revoked { time, reason }
            }
            _ => return Err(bad()),
        };
        let this_update = gen_time(&mut one).ok_or_else(bad)?;
        let next_update = match one.optional(0xa0) {
            Some(x) => {
                let mut n = Der::new(x);
                let v = gen_time(&mut n).ok_or_else(bad)?;
                if !n.is_empty() {
                    return Err(bad());
                }
                Some(v)
            }
            None => None,
        };
        if let Some(e) = one.optional(0xa1) {
            find_nonce(e).ok_or_else(|| String::from("an OCSP response with an unknown critical extension"))?;
        }
        if !one.is_empty() {
            return Err(bad());
        }
        let ours = CertId::with_hash(cert, issuer, id.hash).is_some_and(|mine| mine == id);
        if ours && found.is_none() {
            found = Some((status, this_update, next_update));
        }
    }
    let (status, this_update, next_update) = found.ok_or_else(|| String::from("the OCSP response is about another certificate"))?;
    if next_update.is_some_and(|n| n < this_update) {
        return Err(bad());
    }
    let r = Response { status, this_update, next_update, produced_at, nonce };
    if this_update > now + SKEW {
        return Err(String::from("the OCSP response is not valid yet"));
    }
    if now > r.expires() + SKEW {
        return Err(String::from("the OCSP response is out of date"));
    }
    Ok(r)
}

/// a response to build and sign (a private CA's responder, tests)
#[derive(Clone, Debug)]
pub struct ResponseBuilder {
    pub status: Status,
    pub this_update: i64,
    /// None: newer information is always there (a response without nextUpdate)
    pub next_update: Option<i64>,
    pub produced_at: i64,
    /// the request's nonce, to answer it
    pub nonce: Option<Vec<u8>>,
    /// the responder named by the SHA-1 of its key (byKey, RFC 5019 2.2.3) instead of its name
    pub by_key: bool,
    pub hash: IdHash,
    /// further responseExtensions (each from x509::extension)
    pub extensions: Vec<Vec<u8>>,
}

impl ResponseBuilder {
    /// good, current from `now` for four days, by key
    pub fn good(now: i64) -> ResponseBuilder {
        ResponseBuilder { status: Status::Good, this_update: now, next_update: Some(now + 4 * 86_400), produced_at: now, nonce: None, by_key: true, hash: IdHash::Sha1, extensions: Vec::new() }
    }

    /// the DER response about `cert`, signed with `key`: the issuer's key (`responder` None), or
    /// that of a responder certificate the issuer issued for OCSP (it goes into the response);
    /// None when the key signs nothing a certificate names
    pub fn sign(&self, cert: &Cert, issuer: &Cert, responder: Option<&Cert>, key: &PrivateKey) -> Option<Vec<u8>> {
        let id = CertId::with_hash(cert, issuer, self.hash)?;
        let signer = responder.unwrap_or(issuer);
        let rid = if self.by_key {
            let (_, k) = signer.spki_parts()?;
            der::explicit(2, &der::octets(&crypto::sha1(k)))
        } else {
            der::explicit(1, &signer.subject)
        };
        let status = match self.status {
            Status::Good => alloc::vec![0x80, 0],
            Status::Unknown => alloc::vec![0x82, 0],
            Status::Revoked { time, reason } => {
                let mut c = encode_gen_time(time);
                if let Some(r) = reason {
                    c.extend_from_slice(&der::explicit(0, &der::tlv(der::ENUMERATED, &[r])));
                }
                der::tlv(0xa1, &c)
            }
        };
        let mut single = Vec::new();
        single.extend_from_slice(&id.encode());
        single.extend_from_slice(&status);
        single.extend_from_slice(&encode_gen_time(self.this_update));
        if let Some(n) = self.next_update {
            single.extend_from_slice(&der::explicit(0, &encode_gen_time(n)));
        }
        let mut data = Vec::new();
        data.extend_from_slice(&rid);
        data.extend_from_slice(&encode_gen_time(self.produced_at));
        data.extend_from_slice(&der::seq(&[&der::tlv(der::SEQUENCE, &single)]));
        let mut exts: Vec<Vec<u8>> = self.nonce.iter().map(|n| nonce_ext(n)).collect();
        exts.extend(self.extensions.iter().cloned());
        if !exts.is_empty() {
            let parts: Vec<&[u8]> = exts.iter().map(|v| v.as_slice()).collect();
            data.extend_from_slice(&der::explicit(1, &der::seq(&parts)));
        }
        let tbs = der::tlv(der::SEQUENCE, &data);
        let alg = key.cert_alg();
        let alg_der = crate::alg_id(alg)?;
        let sig = key.sign(alg, &tbs, &[])?;
        let mut basic = Vec::new();
        basic.extend_from_slice(&tbs);
        basic.extend_from_slice(&alg_der);
        basic.extend_from_slice(&der::bits(&sig));
        if let Some(r) = responder {
            basic.extend_from_slice(&der::explicit(0, &der::seq(&[&r.der])));
        }
        let basic = der::tlv(der::SEQUENCE, &basic);
        let bytes = der::seq(&[&der::oid(OID_BASIC), &der::octets(&basic)]);
        Some(der::seq(&[&der::tlv(der::ENUMERATED, &[0]), &der::explicit(0, &bytes)]))
    }
}

/// an unsuccessful OCSPResponse (MALFORMED_REQUEST, INTERNAL_ERROR, TRY_LATER, SIG_REQUIRED,
/// UNAUTHORIZED): no responseBytes, nothing signed
pub fn error_response(code: u8) -> Vec<u8> {
    der::seq(&[&der::tlv(der::ENUMERATED, &[code])])
}
