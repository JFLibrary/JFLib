// key -- private keys that sign: Ed25519, ECDSA on P-256 / P-384 and RSA
//
//   PrivateKey::from(ed25519::Keypair)        PrivateKey::ecdsa(curve, secret)
//   PrivateKey::from_der(der)                 PKCS #8 (RFC 5958: Ed25519, EC, RSA), SEC 1's
//                                             ECPrivateKey (RFC 5915), PKCS #1's RSAPrivateKey
//                                             (RFC 8017 A.1.2); not encrypted ones
//   key.to_pkcs8() -> Option<DER>             to store it (secret)
//   key.public_key() -> PubKey                key.spki() -> SubjectPublicKeyInfo (DER)
//   key.sign(alg, data, random) -> Option<signature>   Ed25519; ECDSA (DER, RFC 6979, `random`
//                                             mixed in as extra entropy); RSA PKCS #1 v1.5 or PSS
//                                             (the salt from `random`)
//   key.cert_alg()                            what it signs certificates with
//
// An ECDSA key is read without a branch on its bytes and checked to be in [1, n - 1]; a public
// key stored with it must be its own.  An RSA key is read once (its numbers are checked against
// each other in variable time, as a program does at start), then signs in constant time.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crypto::ecdsa::Curve;
use crypto::ed25519::Keypair;
use crypto::rsa;
use crypto::HashAlg;

use crate::der::{self, Der};
use crate::{oid_is, PubKey, SigAlg, OID_EC, OID_ED25519, OID_P256, OID_P384, OID_RSA};

/// a private key that signs (Ed25519, ECDSA on P-256 or P-384, RSA)
#[derive(Clone)]
pub struct PrivateKey {
    k: Kind,
}

#[derive(Clone)]
enum Kind {
    Ed25519(Keypair),
    Ec { curve: Curve, secret: Vec<u8>, public: Vec<u8> },
    Rsa(Box<rsa::PrivateKey>),
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        if let Kind::Ec { secret, .. } = &mut self.k {
            crypto::zeroize(secret);
        }
    }
}

impl core::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let what = match &self.k {
            Kind::Ed25519(_) => "Ed25519",
            Kind::Ec { curve: Curve::P256, .. } => "ECDSA P-256",
            Kind::Ec { .. } => "ECDSA P-384",
            Kind::Rsa(..) => "RSA",
        };
        write!(f, "PrivateKey({})", what)
    }
}

impl From<Keypair> for PrivateKey {
    fn from(k: Keypair) -> PrivateKey {
        PrivateKey { k: Kind::Ed25519(k) }
    }
}

fn curve_oid(curve: Curve) -> Option<&'static [u64]> {
    match curve {
        Curve::P256 => Some(OID_P256),
        Curve::P384 => Some(OID_P384),
        _ => None,
    }
}

fn curve_of(oid: &[u8]) -> Option<Curve> {
    if oid_is(oid, OID_P256) {
        Some(Curve::P256)
    } else if oid_is(oid, OID_P384) {
        Some(Curve::P384)
    } else {
        None
    }
}

/// the bytes of an INTEGER of a key, its leading zero byte dropped (a secret's value is not
/// looked at: whether a zero byte leads is what the length tells anyway)
fn secret_int(b: &[u8], len: usize) -> Option<&[u8]> {
    match b.len() {
        0 => None,
        n if n == len + 1 => Some(&b[1..]),
        n if n <= len => Some(b),
        _ => None,
    }
}

impl PrivateKey {
    /// an ECDSA key from its secret scalar (curve.bytes() big-endian bytes in [1, n - 1])
    pub fn ecdsa(curve: Curve, secret: &[u8]) -> Option<PrivateKey> {
        let public = crypto::ecdsa::public_key(curve, secret)?;
        Some(PrivateKey { k: Kind::Ec { curve, secret: Vec::from(secret), public } })
    }

    /// a new ECDSA key from `rand` (the platform's random bytes): drawn again until it is in
    /// [1, n - 1]
    pub fn generate_ecdsa(curve: Curve, rand: &mut dyn FnMut(&mut [u8])) -> PrivateKey {
        let mut buf = alloc::vec![0u8; curve.bytes()];
        loop {
            rand(&mut buf);
            if let Some(k) = PrivateKey::ecdsa(curve, &buf) {
                crypto::zeroize(&mut buf);
                return k;
            }
        }
    }

    /// an RSA key
    pub fn rsa(key: rsa::PrivateKey) -> Option<PrivateKey> {
        key.public()?;
        Some(PrivateKey { k: Kind::Rsa(Box::new(key)) })
    }

    /// a key in DER: PKCS #8 (PrivateKeyInfo / OneAsymmetricKey), an ECPrivateKey with its curve
    /// or an RSAPrivateKey; None for anything else (an encrypted PKCS #8 key among them)
    pub fn from_der(b: &[u8]) -> Option<PrivateKey> {
        pkcs8(b).or_else(|| ec_private_key(b, None)).or_else(|| rsa_private_key(b))
    }

    /// the PKCS #8 form (to store the key; secret); None only for a curve without a name here
    pub fn to_pkcs8(&self) -> Option<Vec<u8>> {
        let v0 = der::small_int(0);
        match &self.k {
            Kind::Ed25519(kp) => Some(crate::ed25519_pkcs8(kp.seed())),
            Kind::Ec { curve, secret, public } => {
                let alg = der::seq(&[&der::oid(OID_EC), &der::oid(curve_oid(*curve)?)]);
                // ECPrivateKey without the curve (the algorithm names it), with the public key
                let inner = der::seq(&[&der::small_int(1), &der::octets(secret), &der::tlv(0xa1, &der::bits(public))]);
                let out = der::seq(&[&v0, &alg, &der::octets(&inner)]);
                let mut inner = inner;
                crypto::zeroize(&mut inner);
                Some(out)
            }
            Kind::Rsa(k) => {
                // RSAPrivateKey: version 0, n, e, d, p, q, d mod (p - 1), d mod (q - 1), q^-1 mod p
                let parts = [k.n(), Vec::from(k.e()), Vec::from(k.d()), Vec::from(k.p()), Vec::from(k.q()), k.dp(), k.dq(), k.iqmp()];
                let mut ints: Vec<Vec<u8>> = parts.iter().map(|x| der::int_u(x)).collect();
                let refs: Vec<&[u8]> = core::iter::once(&v0[..]).chain(ints.iter().map(|x| x.as_slice())).collect();
                let mut inner = der::seq(&refs);
                let out = der::seq(&[&v0, &der::seq(&[&der::oid(OID_RSA), &[der::NULL, 0]]), &der::octets(&inner)]);
                crypto::zeroize(&mut inner);
                for mut x in parts.into_iter().chain(ints.drain(..)) {
                    crypto::zeroize(&mut x);
                }
                Some(out)
            }
        }
    }

    /// the public half
    pub fn public_key(&self) -> PubKey {
        match &self.k {
            Kind::Ed25519(kp) => PubKey::Ed25519(kp.public),
            Kind::Ec { curve, public, .. } => PubKey::Ec { curve: *curve, point: public.clone() },
            Kind::Rsa(k) => PubKey::Rsa { n: Vec::from(crate::strip_zeros(&k.n())), e: Vec::from(crate::strip_zeros(k.e())) },
        }
    }

    /// the public half as a SubjectPublicKeyInfo (DER), as certificates and CSRs carry it
    pub fn spki(&self) -> Vec<u8> {
        spki_of(&self.public_key())
    }

    /// the RSA modulus's bits (0 for other keys)
    pub fn rsa_bits(&self) -> usize {
        match &self.k {
            Kind::Rsa(k) => k.bits(),
            _ => 0,
        }
    }

    /// the Ed25519 key pair, if it is one
    pub fn ed25519(&self) -> Option<&Keypair> {
        match &self.k {
            Kind::Ed25519(kp) => Some(kp),
            _ => None,
        }
    }

    /// whether this key makes signatures of `alg` (ECDSA: of its curve's size or any hash, as
    /// X.509 and TLS 1.2 allow; TLS 1.3 ties the hash to the curve itself)
    pub fn can(&self, alg: SigAlg) -> bool {
        matches!(
            (&self.k, alg),
            (Kind::Ed25519(_), SigAlg::Ed25519) | (Kind::Ec { .. }, SigAlg::Ecdsa(HashAlg::Sha256 | HashAlg::Sha384 | HashAlg::Sha512)) | (Kind::Rsa(..), SigAlg::RsaPkcs1(HashAlg::Sha256 | HashAlg::Sha384 | HashAlg::Sha512)) | (Kind::Rsa(..), SigAlg::RsaPss(HashAlg::Sha256 | HashAlg::Sha384 | HashAlg::Sha512, _))
        )
    }

    /// the algorithm it signs certificates with: Ed25519, ECDSA with SHA-256 (P-256) or
    /// SHA-384 (P-384), RSA PKCS #1 v1.5 with SHA-256
    pub fn cert_alg(&self) -> SigAlg {
        match &self.k {
            Kind::Ed25519(_) => SigAlg::Ed25519,
            Kind::Ec { curve: Curve::P256, .. } => SigAlg::Ecdsa(HashAlg::Sha256),
            Kind::Ec { .. } => SigAlg::Ecdsa(HashAlg::Sha384),
            Kind::Rsa(..) => SigAlg::RsaPkcs1(HashAlg::Sha256),
        }
    }

    /// a signature of `data` with `alg` (None when the key does not make it): Ed25519's 64
    /// bytes; ECDSA as DER (Ecdsa-Sig-Value), its nonce by RFC 6979 with `random` mixed in (empty:
    /// the same signature each time); RSA PKCS #1 v1.5, or PSS with MGF1 of the same hash and a
    /// salt of the first bytes of `random` (as long as the salt length, the hash length when
    /// the algorithm leaves it open; a shorter `random` gives None)
    pub fn sign(&self, alg: SigAlg, data: &[u8], random: &[u8]) -> Option<Vec<u8>> {
        if !self.can(alg) {
            return None;
        }
        match (&self.k, alg) {
            (Kind::Ed25519(kp), _) => Some(Vec::from(&kp.sign(data)[..])),
            (Kind::Ec { curve, secret, .. }, SigAlg::Ecdsa(h)) => {
                let rs = crypto::ecdsa::sign(*curve, secret, h, &h.digest(data), random)?;
                crypto::ecdsa::der_from_p1363(&rs)
            }
            (Kind::Rsa(k), SigAlg::RsaPkcs1(h)) => k.sign_pkcs1(h, &h.digest(data)),
            (Kind::Rsa(k), SigAlg::RsaPss(h, salt)) => {
                let n = salt.unwrap_or(h.len());
                k.sign_pss(h, &h.digest(data), random.get(..n)?)
            }
            _ => None,
        }
    }
}

/// the SubjectPublicKeyInfo of a public key (empty for PubKey::Other)
pub fn spki_of(key: &PubKey) -> Vec<u8> {
    match key {
        PubKey::Ed25519(pk) => der::seq(&[&der::seq(&[&der::oid(OID_ED25519)]), &der::bits(pk)]),
        PubKey::Ec { curve, point } => match curve_oid(*curve) {
            Some(c) => der::seq(&[&der::seq(&[&der::oid(OID_EC), &der::oid(c)]), &der::bits(point)]),
            None => Vec::new(),
        },
        PubKey::Rsa { n, e } => der::seq(&[&der::seq(&[&der::oid(OID_RSA), &[der::NULL, 0]]), &der::bits(&der::seq(&[&der::int_u(n), &der::int_u(e)]))]),
        PubKey::Other => Vec::new(),
    }
}

/// PKCS #8: version 0 (or 1 with a public key, RFC 5958), the algorithm, the key
fn pkcs8(b: &[u8]) -> Option<PrivateKey> {
    let mut top = Der::new(b);
    let mut d = top.seq()?;
    if !top.is_empty() || d.u64()? > 1 {
        return None;
    }
    let alg = d.read(der::SEQUENCE)?;
    let key = d.read(der::OCTET_STRING)?;
    // attributes [0] and the public key [1] may follow; neither is needed
    let mut a = Der::new(alg);
    let oid = a.read(der::OID)?;
    if oid_is(oid, OID_ED25519) {
        if !a.is_empty() {
            return None;
        }
        let seed: [u8; 32] = Der::new(key).read(der::OCTET_STRING)?.try_into().ok()?;
        return Some(PrivateKey::from(Keypair::from_seed(&seed)));
    }
    if oid_is(oid, OID_EC) {
        let curve = curve_of(a.read(der::OID)?)?;
        return if a.is_empty() { ec_private_key(key, Some(curve)) } else { None };
    }
    if oid_is(oid, OID_RSA) {
        a.optional(der::NULL);
        return if a.is_empty() { rsa_private_key(key) } else { None };
    }
    None
}

/// ECPrivateKey (RFC 5915): version 1, the secret, [0] the curve (which must agree with the
/// algorithm's when both are there), [1] the public key (which must be the secret's)
fn ec_private_key(b: &[u8], curve: Option<Curve>) -> Option<PrivateKey> {
    let mut top = Der::new(b);
    let mut d = top.seq()?;
    if !top.is_empty() || d.u64()? != 1 {
        return None;
    }
    let secret = d.read(der::OCTET_STRING)?;
    let named = match d.optional(0xa0) {
        Some(p) => Some(curve_of(Der::new(p).read(der::OID)?)?),
        None => None,
    };
    let curve = match (curve, named) {
        (Some(a), Some(b)) if a != b => return None,
        (Some(c), _) | (None, Some(c)) => c,
        (None, None) => return None,
    };
    let public = match d.optional(0xa1) {
        Some(p) => Some(Der::new(p).bits()?),
        None => None,
    };
    if !d.is_empty() || secret.len() > curve.bytes() {
        return None;
    }
    // a secret written without its leading zero bytes is padded to the curve's length
    let mut s = alloc::vec![0u8; curve.bytes()];
    s[curve.bytes() - secret.len()..].copy_from_slice(secret);
    let k = PrivateKey::ecdsa(curve, &s);
    crypto::zeroize(&mut s);
    let k = k?;
    // the public key stored with the secret must be the one it gives (compared without a branch
    // on the bytes; the answer is public)
    match (public, &k.k) {
        (Some(p), Kind::Ec { public, .. }) if !crypto::ct_eq(p, public) => None,
        _ => Some(k),
    }
}

/// RSAPrivateKey (RFC 8017 A.1.2), two primes: version 0, n, e, d, p, q, dp, dq, qinv
fn rsa_private_key(b: &[u8]) -> Option<PrivateKey> {
    let mut top = Der::new(b);
    let mut d = top.seq()?;
    if !top.is_empty() || d.u64()? != 0 {
        return None;
    }
    let n = crate::der_unsigned(d.read(der::INTEGER)?)?;
    let e = crate::der_unsigned(d.read(der::INTEGER)?)?;
    let k = n.len();
    let dd = secret_int(d.read(der::INTEGER)?, k)?;
    let p = secret_int(d.read(der::INTEGER)?, k)?;
    let q = secret_int(d.read(der::INTEGER)?, k)?;
    for _ in 0..3 {
        d.read(der::INTEGER)?;
    }
    if !d.is_empty() {
        return None;
    }
    PrivateKey::rsa(rsa::PrivateKey::from_parts(n, e, dd, p, q)?)
}
