// ecdsa -- ECDSA on NIST P-256 and P-384 (FIPS 186-5, SEC 1, RFC 6979)
//
//   verify(curve, public_key, digest, r, s)   public key uncompressed (04 || x || y), checked
//                                             to lie on the curve; r, s big-endian integers
//   verify_p1363(curve, public_key, digest, sig)   sig = r || s, each curve.bytes() long (IEEE
//                                             P1363: JOSE, COSE, WebCrypto)
//   sign(curve, secret, hash, digest, extra) -> Option<r || s>   deterministic nonces (RFC 6979
//                                             with `hash`, the digest's hash); `extra`, when not
//                                             empty, goes into the nonce as RFC 6979 3.6's
//                                             additional data: 32 random bytes there hedge against
//                                             faults and a poor random source alike
//   public_key(curve, secret) -> Option<04 || x || y>    secret_from(curve, random) -> Option<secret>
//   der_from_p1363(sig) / p1363_from_der(curve, der)     r || s <-> the DER Ecdsa-Sig-Value
//                                             (strict: minimal lengths and integers, nothing after)
//
// Verification: Jacobian coordinates in the Montgomery domain (a = -3 formulas), u1 G + u2 Q by
// Shamir's trick; public data only, variable time.  Signing: nist.rs in constant time (complete
// formulas, windows read in full, inverses as powers to n - 2); a nonce outside [1, n - 1], or
// an r or s of 0, draws the next one as RFC 6979 says (about 2^-32 on P-256, which shows only that
// a nonce was refused, nothing of the one taken).

use alloc::vec::Vec;

use crate::bn::{self, Limbs, Mont};
use crate::nist::{self, P256, P384};
use crate::HashAlg;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Curve {
    P256,
    P384,
}

struct Params {
    p: &'static str,
    n: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
    bytes: usize,
}

fn params(c: Curve) -> Params {
    match c {
        Curve::P256 => Params {
            p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
            n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
            b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
            gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
            gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
            bytes: 32,
        },
        Curve::P384 => Params {
            p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
            n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
            b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
            gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
            gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
            bytes: 48,
        },
    }
}

impl Curve {
    /// the size of a coordinate / scalar in bytes
    pub fn bytes(self) -> usize {
        params(self).bytes
    }
}

fn hex_limbs(h: &str, k: usize) -> Limbs {
    bn::from_be(&crate::unhex(h).unwrap_or_default(), k)
}

/// a point in Jacobian coordinates, Montgomery form; z = 0 is the point at infinity
#[derive(Clone)]
struct Pt {
    x: Limbs,
    y: Limbs,
    z: Limbs,
}

struct Field {
    m: Mont,
    one: Limbs,
}

impl Field {
    fn mul(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.mul(a, b)
    }
    fn sq(&self, a: &[u64]) -> Limbs {
        self.m.mul(a, a)
    }
    fn add(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.add(a, b)
    }
    fn sub(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.sub(a, b)
    }
    fn small(&self, a: &[u64], k: u32) -> Limbs {
        let mut r = a.to_vec();
        for _ in 1..k {
            r = self.add(&r, a);
        }
        r
    }

    fn double(&self, p: &Pt) -> Pt {
        if bn::is_zero(&p.z) || bn::is_zero(&p.y) {
            return Pt { x: self.one.clone(), y: self.one.clone(), z: alloc::vec![0; self.one.len()] };
        }
        let delta = self.sq(&p.z);
        let gamma = self.sq(&p.y);
        let beta = self.mul(&p.x, &gamma);
        let alpha = self.small(&self.mul(&self.sub(&p.x, &delta), &self.add(&p.x, &delta)), 3);
        let x3 = self.sub(&self.sq(&alpha), &self.small(&beta, 8));
        let z3 = self.sub(&self.sub(&self.sq(&self.add(&p.y, &p.z)), &gamma), &delta);
        let y3 = self.sub(&self.mul(&alpha, &self.sub(&self.small(&beta, 4), &x3)), &self.small(&self.sq(&gamma), 8));
        Pt { x: x3, y: y3, z: z3 }
    }

    fn add_pts(&self, p: &Pt, q: &Pt) -> Pt {
        if bn::is_zero(&p.z) {
            return q.clone();
        }
        if bn::is_zero(&q.z) {
            return p.clone();
        }
        let z1z1 = self.sq(&p.z);
        let z2z2 = self.sq(&q.z);
        let u1 = self.mul(&p.x, &z2z2);
        let u2 = self.mul(&q.x, &z1z1);
        let s1 = self.mul(&self.mul(&p.y, &q.z), &z2z2);
        let s2 = self.mul(&self.mul(&q.y, &p.z), &z1z1);
        let h = self.sub(&u2, &u1);
        let r = self.small(&self.sub(&s2, &s1), 2);
        if bn::is_zero(&h) {
            return if bn::is_zero(&r) { self.double(p) } else { Pt { x: self.one.clone(), y: self.one.clone(), z: alloc::vec![0; self.one.len()] } };
        }
        let i = self.sq(&self.small(&h, 2));
        let j = self.mul(&h, &i);
        let v = self.mul(&u1, &i);
        let x3 = self.sub(&self.sub(&self.sq(&r), &j), &self.small(&v, 2));
        let y3 = self.sub(&self.mul(&r, &self.sub(&v, &x3)), &self.small(&self.mul(&s1, &j), 2));
        let z3 = self.mul(&self.sub(&self.sub(&self.sq(&self.add(&p.z, &q.z)), &z1z1), &z2z2), &h);
        Pt { x: x3, y: y3, z: z3 }
    }
}

/// verify an ECDSA signature over `digest` (the hash of the message, any length)
pub fn verify(curve: Curve, public_key: &[u8], digest: &[u8], r: &[u8], s: &[u8]) -> bool {
    let pr = params(curve);
    let nb = pr.bytes;
    let k = nb.div_ceil(8);
    if public_key.len() != 1 + 2 * nb || public_key[0] != 4 {
        return false;
    }
    let p = hex_limbs(pr.p, k);
    let n = hex_limbs(pr.n, k);
    let (Some(fm), Some(nm)) = (Mont::new(&p), Mont::new(&n)) else { return false };
    let f = Field { one: fm.to_mont(&[1]), m: fm };
    // r, s in [1, n-1]
    let strip = |x: &[u8]| -> Option<Limbs> {
        let x = &x[x.iter().position(|&b| b != 0).unwrap_or(x.len())..];
        if x.len() > nb {
            return None;
        }
        let v = bn::from_be(x, k);
        (!bn::is_zero(&v) && bn::cmp(&v, &n) == core::cmp::Ordering::Less).then_some(v)
    };
    let (Some(r), Some(s)) = (strip(r), strip(s)) else { return false };
    // the public key: coordinates below p, on the curve y^2 = x^3 - 3x + b
    let qx = bn::from_be(&public_key[1..1 + nb], k);
    let qy = bn::from_be(&public_key[1 + nb..], k);
    if bn::cmp(&qx, &p) != core::cmp::Ordering::Less || bn::cmp(&qy, &p) != core::cmp::Ordering::Less {
        return false;
    }
    let (mx, my) = (f.m.to_mont(&qx), f.m.to_mont(&qy));
    let b = f.m.to_mont(&hex_limbs(pr.b, k));
    let rhs = f.add(&f.sub(&f.mul(&f.sq(&mx), &mx), &f.small(&mx, 3)), &b);
    if f.sq(&my) != rhs {
        return false;
    }
    // e: the leftmost bits of the digest, as many as n has
    let n_bits = bn::bits(&n);
    let mut e = bn::from_be(&digest[..digest.len().min(nb)], k + 1);
    let have = digest.len().min(nb) * 8;
    if have > n_bits {
        let sh = have - n_bits;
        let mut c = 0u64;
        for x in e.iter_mut().rev() {
            let nc = *x << (64 - sh);
            *x = *x >> sh | c;
            c = nc;
        }
    }
    let e = nm.reduce_once(&e);
    // w = s^-1, u1 = e w, u2 = r w (mod n)
    let w = nm.inv_mont(&nm.to_mont(&s));
    let u1 = nm.from_mont(&nm.mul(&nm.to_mont(&e), &w));
    let u2 = nm.from_mont(&nm.mul(&nm.to_mont(&r), &w));
    let g = Pt { x: f.m.to_mont(&hex_limbs(pr.gx, k)), y: f.m.to_mont(&hex_limbs(pr.gy, k)), z: f.one.clone() };
    let q = Pt { x: mx, y: my, z: f.one.clone() };
    let gq = f.add_pts(&g, &q);
    let (b1, b2) = (bn::to_be(&u1, nb), bn::to_be(&u2, nb));
    let mut acc = Pt { x: f.one.clone(), y: f.one.clone(), z: alloc::vec![0; k] };
    for i in 0..nb * 8 {
        acc = f.double(&acc);
        let bit = |b: &[u8]| b[i / 8] >> (7 - i % 8) & 1 == 1;
        match (bit(&b1), bit(&b2)) {
            (true, true) => acc = f.add_pts(&acc, &gq),
            (true, false) => acc = f.add_pts(&acc, &g),
            (false, true) => acc = f.add_pts(&acc, &q),
            _ => {}
        }
    }
    if bn::is_zero(&acc.z) {
        return false;
    }
    // affine x = X / Z^2, then mod n
    let zinv = f.m.inv_mont(&acc.z);
    let x = f.m.from_mont(&f.mul(&acc.x, &f.sq(&zinv)));
    let xn = nm.reduce_once(&x);
    xn == r
}

// ---------------------------------------------------------------- P1363 and DER

/// verify an r || s signature (IEEE P1363: each half curve.bytes() long)
pub fn verify_p1363(curve: Curve, public_key: &[u8], digest: &[u8], sig: &[u8]) -> bool {
    let nb = curve.bytes();
    sig.len() == 2 * nb && verify(curve, public_key, digest, &sig[..nb], &sig[nb..])
}

/// a DER INTEGER of a big-endian unsigned number (minimal, with a zero byte before a high bit)
fn der_uint(x: &[u8], out: &mut Vec<u8>) {
    let x = &x[x.iter().position(|&b| b != 0).unwrap_or(x.len().saturating_sub(1))..];
    let pad = x[0] & 0x80 != 0;
    out.push(0x02);
    out.push((x.len() + pad as usize) as u8);
    if pad {
        out.push(0);
    }
    out.extend_from_slice(x);
}

/// the DER Ecdsa-Sig-Value (SEQUENCE { r INTEGER, s INTEGER }) of an r || s signature; None
/// for an empty or odd length, or halves over 125 bytes
pub fn der_from_p1363(sig: &[u8]) -> Option<Vec<u8>> {
    if sig.is_empty() || sig.len() % 2 != 0 || sig.len() > 250 {
        return None;
    }
    let (r, s) = sig.split_at(sig.len() / 2);
    let mut body = Vec::with_capacity(sig.len() + 6);
    der_uint(r, &mut body);
    der_uint(s, &mut body);
    let mut out = Vec::with_capacity(body.len() + 3);
    out.push(0x30);
    if body.len() >= 0x80 {
        out.push(0x81);
    }
    out.push(body.len() as u8);
    out.extend_from_slice(&body);
    Some(out)
}

/// one DER element of this tag: (contents, what follows); lengths in their shortest form only
fn der_tlv(b: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    if b.len() < 2 || b[0] != tag {
        return None;
    }
    let (len, at) = match b[1] {
        l if l < 0x80 => (l as usize, 2),
        0x81 if b.len() > 2 && b[2] >= 0x80 => (b[2] as usize, 3),
        _ => return None,
    };
    if b.len() - at < len {
        return None;
    }
    Some((&b[at..at + len], &b[at + len..]))
}

/// r || s (each curve.bytes() long) from a DER Ecdsa-Sig-Value; None unless it is strict DER: one
/// SEQUENCE of two minimal, non-negative INTEGERs no longer than the curve's size, nothing after
pub fn p1363_from_der(curve: Curve, der: &[u8]) -> Option<Vec<u8>> {
    let nb = curve.bytes();
    let (body, rest) = der_tlv(der, 0x30)?;
    let (r, body) = der_tlv(body, 0x02)?;
    let (s, body) = der_tlv(body, 0x02)?;
    if !rest.is_empty() || !body.is_empty() {
        return None;
    }
    let mut out = alloc::vec![0u8; 2 * nb];
    for (v, half) in [(r, 0), (s, nb)] {
        if v.is_empty() || v[0] & 0x80 != 0 || (v.len() > 1 && v[0] == 0 && v[1] & 0x80 == 0) {
            return None;
        }
        let v = if v[0] == 0 && v.len() > 1 { &v[1..] } else { v };
        if v.len() > nb {
            return None;
        }
        out[half + nb - v.len()..half + nb].copy_from_slice(v);
    }
    Some(out)
}

// ---------------------------------------------------------------- keys and signing

/// a secret key in [1, n - 1] from curve.bytes() random bytes (None: draw again, or another
/// length); only the answer is a branch
pub fn secret_from(curve: Curve, random: &[u8]) -> Option<Vec<u8>> {
    if random.len() != curve.bytes() {
        return None;
    }
    let ok = match curve {
        Curve::P256 => P256.n.in_range_mask(&nist::from_be::<4>(random)),
        Curve::P384 => P384.n.in_range_mask(&nist::from_be::<6>(random)),
    };
    (ok != 0).then(|| Vec::from(random))
}

/// the public key (04 || x || y) of a secret key; None when the secret is not in [1, n - 1]
pub fn public_key(curve: Curve, secret: &[u8]) -> Option<Vec<u8>> {
    secret_from(curve, secret)?;
    Some(match curve {
        Curve::P256 => P256.public_key(secret),
        Curve::P384 => P384.public_key(secret),
    })
}

/// sign a digest (`hash`'s output, which RFC 6979 also derives the nonce with): r || s, each
/// curve.bytes() long.  None when the secret is not in [1, n - 1] or the digest is not
/// hash.len() bytes.  The same secret, digest and extra give the same signature
pub fn sign(curve: Curve, secret: &[u8], hash: HashAlg, digest: &[u8], extra: &[u8]) -> Option<Vec<u8>> {
    match curve {
        Curve::P256 => sign_on(&P256, secret, hash, digest, extra),
        Curve::P384 => sign_on(&P384, secret, hash, digest, extra),
    }
}

/// HMAC_K(data) replaces k (the old key overwritten)
fn rekey(hash: HashAlg, k: &mut Vec<u8>, data: &[u8]) {
    let mut next = hash.hmac(k, data);
    core::mem::swap(k, &mut next);
    crate::zeroize(&mut next);
}

/// V = HMAC_K(V)
fn step(hash: HashAlg, k: &[u8], v: &mut Vec<u8>) {
    let mut next = hash.hmac(k, v);
    core::mem::swap(v, &mut next);
    crate::zeroize(&mut next);
}

fn sign_on<const L: usize>(c: &nist::Curve<L>, secret: &[u8], hash: HashAlg, digest: &[u8], extra: &[u8]) -> Option<Vec<u8>> {
    let nb = nist::Curve::<L>::BYTES;
    if secret.len() != nb || digest.len() != hash.len() {
        return None;
    }
    let mut d = nist::from_be::<L>(secret);
    if c.n.in_range_mask(&d) == 0 {
        return None;
    }
    // bits2int(digest) mod n: the leftmost 8 nb bits (n has exactly that many); below 2^(8 nb) < 2 n
    let take = digest.len().min(nb);
    let mut padded = alloc::vec![0u8; nb];
    padded[nb - take..].copy_from_slice(&digest[..take]);
    let e = c.n.reduce_once(&nist::from_be::<L>(&padded));
    // RFC 6979 3.2 (with 3.6's additional data): K and V from x = int2octets(d), h1 = bits2octets
    nist::to_be(&e, &mut padded);
    let hl = hash.len();
    let mut v = alloc::vec![1u8; hl];
    let mut k = alloc::vec![0u8; hl];
    let mut msg = Vec::with_capacity(hl + 1 + 2 * nb + extra.len());
    for sep in [0u8, 1] {
        msg.clear();
        msg.extend_from_slice(&v);
        msg.push(sep);
        msg.extend_from_slice(secret);
        msg.extend_from_slice(&padded);
        msg.extend_from_slice(extra);
        rekey(hash, &mut k, &msg);
        step(hash, &k, &mut v);
    }
    crate::zeroize(&mut msg);
    let mut t = Vec::with_capacity(nb + hl);
    let sig = loop {
        t.clear();
        while t.len() < nb {
            step(hash, &k, &mut v);
            t.extend_from_slice(&v);
        }
        // bits2int(T): the first nb bytes (qlen = 8 nb)
        let mut kk = nist::from_be::<L>(&t[..nb]);
        crate::zeroize(&mut t);
        let found = if c.n.in_range_mask(&kk) != 0 { sign_with(c, &d, &e, &kk) } else { None };
        crate::zeroize_words(&mut kk);
        if let Some(sig) = found {
            break sig;
        }
        // K = HMAC_K(V || 0x00), V = HMAC_K(V)
        let mut again = v.clone();
        again.push(0);
        rekey(hash, &mut k, &again);
        crate::zeroize(&mut again);
        step(hash, &k, &mut v);
    };
    crate::zeroize(&mut k);
    crate::zeroize(&mut v);
    crate::zeroize_words(&mut d);
    Some(sig)
}

/// r || s with the nonce k (in [1, n - 1]); None when r or s comes out 0
fn sign_with<const L: usize>(c: &nist::Curve<L>, d: &[u64; L], e: &[u64; L], k: &[u64; L]) -> Option<Vec<u8>> {
    let nb = nist::Curve::<L>::BYTES;
    let mut kb = alloc::vec![0u8; nb];
    nist::to_be(k, &mut kb);
    let (_, x, _) = c.affine(&c.mul(&kb, &c.generator()));
    crate::zeroize(&mut kb);
    // x < p < 2 n on both curves: one subtraction makes it x mod n
    let r = c.n.reduce_once(&x);
    let n = &c.n;
    // s = k^-1 (e + r d) mod n, in Montgomery form throughout
    let mut kinv = n.inv(&n.to_mont(k));
    let mut rd = n.mul(&n.to_mont(&r), &n.to_mont(d));
    let mut sum = n.add(&n.to_mont(e), &rd);
    let s = n.from_mont(&n.mul(&kinv, &sum));
    crate::zeroize_words(&mut kinv);
    crate::zeroize_words(&mut rd);
    crate::zeroize_words(&mut sum);
    // r and s are the public signature: these branches show what is published anyway
    if r.iter().all(|&w| w == 0) || s.iter().all(|&w| w == 0) {
        return None;
    }
    let mut out = alloc::vec![0u8; 2 * nb];
    nist::to_be(&r, &mut out[..nb]);
    nist::to_be(&s, &mut out[nb..]);
    Some(out)
}
