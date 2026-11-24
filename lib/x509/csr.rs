// csr -- certificate requests (PKCS #10, RFC 2986): what ACME's finalize sends (RFC 8555 7.4)
// and what a private CA reads before it signs
//
//   CsrBuilder { dns, ips, cn, must_staple }.sign(&key) -> Option<DER>
//   Csr::parse(der) -> Option<Csr>     the subject, the key, the requested names; its signature
//                                      checked (the requester holds the key)
//
// The names go into an extensionRequest (PKCS #9) as a subject alternative name; the common
// name is optional (ACME's CAs take the names from the extension).

use alloc::string::String;
use alloc::vec::Vec;

use crate::der::{self, Der};
use crate::{ascii, dns_name_ok, oid_is, pub_key, sig_alg, IpAddr, PrivateKey, PubKey, SigAlg};

const OID_EXT_REQUEST: &[u64] = &[1, 2, 840, 113549, 1, 9, 14];
const OID_SAN: &[u64] = &[2, 5, 29, 17];
const OID_TLS_FEATURE: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 24];
const OID_CN: &[u64] = &[2, 5, 4, 3];

/// a request to build
#[derive(Clone, Debug, Default)]
pub struct CsrBuilder {
    /// the common name (empty: none; ACME takes at most 64 bytes, RFC 5280's ub-common-name)
    pub cn: String,
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
    /// ask for a certificate that must come with a stapled OCSP response (RFC 7633)
    pub must_staple: bool,
}

impl CsrBuilder {
    /// a request for these DNS names (the first one also the common name when it fits)
    pub fn for_names(names: &[&str]) -> CsrBuilder {
        let cn = names.first().filter(|n| n.len() <= 64).map(|n| String::from(*n)).unwrap_or_default();
        CsrBuilder { cn, dns: names.iter().map(|n| String::from(*n)).collect(), ..CsrBuilder::default() }
    }

    /// the DER request, signed with `key` (ECDSA with its curve's hash, RSA PKCS #1 v1.5 with
    /// SHA-256, Ed25519); None when a name is not a DNS name in the preferred syntax
    pub fn sign(&self, key: &PrivateKey) -> Option<Vec<u8>> {
        let subject = if self.cn.is_empty() { der::seq(&[]) } else { der::seq(&[&der::set(&[&der::seq(&[&der::oid(OID_CN), &der::utf8(&self.cn)])])]) };
        let mut exts: Vec<Vec<u8>> = Vec::new();
        if !self.dns.is_empty() || !self.ips.is_empty() {
            let mut names: Vec<Vec<u8>> = Vec::new();
            for d in &self.dns {
                if !dns_name_ok(&d.to_ascii_lowercase()) {
                    return None;
                }
                names.push(der::tlv(0x82, d.as_bytes()));
            }
            for ip in &self.ips {
                names.push(match ip {
                    IpAddr::V4(a) => der::tlv(0x87, a),
                    IpAddr::V6(a) => der::tlv(0x87, a),
                });
            }
            let parts: Vec<&[u8]> = names.iter().map(|v| v.as_slice()).collect();
            exts.push(crate::ext(OID_SAN, false, &der::seq(&parts)));
        }
        if self.must_staple {
            exts.push(crate::ext(OID_TLS_FEATURE, false, &der::seq(&[&der::small_int(crate::TLS_FEATURE_STATUS_REQUEST as u64)])));
        }
        let attrs = if exts.is_empty() {
            der::explicit(0, &[])
        } else {
            let parts: Vec<&[u8]> = exts.iter().map(|v| v.as_slice()).collect();
            der::explicit(0, &der::seq(&[&der::oid(OID_EXT_REQUEST), &der::set(&[&der::seq(&parts)])]))
        };
        let info = der::seq(&[&der::small_int(0), &subject, &key.spki(), &attrs]);
        let alg = key.cert_alg();
        let alg_der = crate::alg_id(alg)?;
        let sig = key.sign(alg, &info, &[])?;
        Some(der::seq(&[&info, &alg_der, &der::bits(&sig)]))
    }
}

/// a request read and its signature checked
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Csr {
    pub der: Vec<u8>,
    /// the subject (DER of the Name)
    pub subject: Vec<u8>,
    pub cn: String,
    /// the SubjectPublicKeyInfo (DER), for CertBuilder::spki
    pub spki: Vec<u8>,
    pub key: PubKey,
    /// the names of the requested subject alternative name (DNS names lower case)
    pub dns: Vec<String>,
    pub ips: Vec<IpAddr>,
    /// the requested TLS features (RFC 7633)
    pub tls_features: Vec<u16>,
    pub sig_alg: SigAlg,
}

impl Csr {
    /// a request: version 0, a key this crate reads, a signature by that key; the names of an
    /// extensionRequest (other extensions are left to the CA, which decides what it signs).
    /// None when it is malformed or the signature does not verify
    pub fn parse(b: &[u8]) -> Option<Csr> {
        let mut top = Der::new(b);
        let mut req = top.seq()?;
        if !top.is_empty() {
            return None;
        }
        let info = req.read_raw(der::SEQUENCE)?;
        let alg = sig_alg(req.read(der::SEQUENCE)?);
        let sig = req.bits()?;
        if !req.is_empty() || alg == SigAlg::Unknown {
            return None;
        }
        let mut i = Der::new(info).seq()?;
        if i.u64()? != 0 {
            return None;
        }
        let subject = Vec::from(i.read_raw(der::SEQUENCE)?);
        let spki = i.read_raw(der::SEQUENCE)?;
        let mut sp = Der::new(spki).seq()?;
        let kalg = sp.read(der::SEQUENCE)?;
        let bits = sp.bits()?;
        if !sp.is_empty() {
            return None;
        }
        let key = pub_key(kalg, bits);
        if key == PubKey::Other {
            return None;
        }
        let mut c = Csr {
            der: Vec::from(b),
            cn: crate::cn_of(Der::new(&subject).read(der::SEQUENCE)?),
            subject,
            spki: Vec::from(spki),
            key,
            sig_alg: alg,
            ..Csr::default()
        };
        // attributes [0] IMPLICIT SET OF Attribute
        let attrs = i.read(0xa0)?;
        if !i.is_empty() {
            return None;
        }
        let mut a = Der::new(attrs);
        while !a.is_empty() {
            let mut attr = a.seq()?;
            let id = attr.read(der::OID)?;
            let mut vals = Der::new(attr.read(der::SET)?);
            if !attr.is_empty() {
                return None;
            }
            if !oid_is(id, OID_EXT_REQUEST) {
                continue;
            }
            let mut exts = vals.seq()?;
            if !vals.is_empty() {
                return None;
            }
            while !exts.is_empty() {
                let mut e = exts.seq()?;
                let eid = e.read(der::OID)?;
                if e.peek_tag() == Some(der::BOOLEAN) {
                    e.bool()?;
                }
                let val = e.read(der::OCTET_STRING)?;
                if !e.is_empty() {
                    return None;
                }
                if oid_is(eid, OID_SAN) {
                    let mut k = Der::new(val).seq()?;
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
                                let mut x = [0u8; 16];
                                x.copy_from_slice(v);
                                c.ips.push(IpAddr::V6(x));
                            }
                            _ => return None,
                        }
                    }
                } else if oid_is(eid, OID_TLS_FEATURE) {
                    let mut k = Der::new(val).seq()?;
                    while !k.is_empty() {
                        c.tls_features.push(u16::try_from(k.u64()?).ok()?);
                    }
                }
            }
        }
        // the requester holds the key
        let probe = crate::Cert { key: c.key.clone(), ..crate::Cert::default() };
        probe.verify(alg, info, sig).then_some(c)
    }
}
