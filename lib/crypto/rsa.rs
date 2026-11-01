// rsa -- RSA signatures (RFC 8017): PKCS #1 v1.5 and PSS verification with MGF1, PKCS #1 v1.5
// signing, key generation
//
//   PublicKey::new(n, e)                      big-endian modulus (1024..8192 bits) and exponent
//   key.verify_pkcs1(hash, digest, sig)       the encoding is rebuilt and compared whole
//   key.verify_pss(hash, digest, sig, salt)   salt length given, or None: any
//   PrivateKey::generate(bits, rand)          exponent 65537, two primes of bits / 2 from the
//                                             caller's random bytes (the same bytes give the
//                                             same key)
//   PrivateKey::from_parts(n, e, d, p, q)     big-endian numbers, checked against each other
//   key.sign_pkcs1(hash, digest)              RSASSA-PKCS1-v1_5; the exponent in constant time
//   key.n() / e() / d() / p() / q() / iqmp()  big-endian without leading zeros
//
// Public operations are variable time (that is fine for them).  Signing uses pow_ct; generate is
// variable time and meant for simulators and tests, where the random bytes are no secret either.

use alloc::vec::Vec;

use crate::bn::{self, Mont};
use crate::HashAlg;

pub struct PublicKey {
    m: Mont,
    e: Vec<u8>,
    /// modulus length in bytes and bits
    k: usize,
    bits: usize,
}

impl PublicKey {
    pub fn new(n: &[u8], e: &[u8]) -> Option<PublicKey> {
        let n = &n[n.iter().position(|&b| b != 0)?..];
        let limbs = bn::from_be(n, 0);
        let bits = bn::bits(&limbs);
        if !(1024..=8192).contains(&bits) {
            return None;
        }
        let e = Vec::from(&e[e.iter().position(|&b| b != 0)?..]);
        if e.len() > 8 || (e.len() == 1 && e[0] < 3) || e.last()? & 1 == 0 {
            return None;
        }
        Some(PublicKey { m: Mont::new(&limbs)?, e, k: n.len(), bits })
    }

    pub fn bits(&self) -> usize {
        self.bits
    }

    /// s^e mod n as k bytes (None when s >= n or has the wrong length)
    fn public_op(&self, sig: &[u8]) -> Option<Vec<u8>> {
        if sig.len() != self.k {
            return None;
        }
        let s = bn::from_be(sig, self.m.limbs());
        if bn::cmp(&s, &self.m.n) != core::cmp::Ordering::Less {
            return None;
        }
        Some(bn::to_be(&self.m.pow(&s, &self.e), self.k))
    }

    /// RSASSA-PKCS1-v1_5 (the DigestInfo with or without the NULL parameter)
    pub fn verify_pkcs1(&self, hash: HashAlg, digest: &[u8], sig: &[u8]) -> bool {
        let Some(em) = self.public_op(sig) else { return false };
        if digest.len() != hash.len() {
            return false;
        }
        let oid_last = match hash {
            HashAlg::Sha256 => 1,
            HashAlg::Sha384 => 2,
            HashAlg::Sha512 => 3,
        };
        let oid = [0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, oid_last];
        [true, false].iter().any(|&with_null| {
            let alg_len = oid.len() + if with_null { 2 } else { 0 };
            let mut t = Vec::new();
            t.extend_from_slice(&[0x30, (alg_len + 2 + 2 + digest.len()) as u8, 0x30, alg_len as u8]);
            t.extend_from_slice(&oid);
            if with_null {
                t.extend_from_slice(&[0x05, 0x00]);
            }
            t.extend_from_slice(&[0x04, digest.len() as u8]);
            t.extend_from_slice(digest);
            if self.k < t.len() + 11 {
                return false;
            }
            let mut want = alloc::vec![0xFFu8; self.k];
            want[0] = 0;
            want[1] = 1;
            let at = self.k - t.len();
            want[at - 1] = 0;
            want[at..].copy_from_slice(&t);
            want == em
        })
    }

    /// RSASSA-PSS with MGF1 of the same hash
    pub fn verify_pss(&self, hash: HashAlg, digest: &[u8], sig: &[u8], salt_len: Option<usize>) -> bool {
        let Some(full) = self.public_op(sig) else { return false };
        let h_len = hash.len();
        let em_bits = self.bits - 1;
        let em_len = em_bits.div_ceil(8);
        // the encoded message is the low em_len bytes; anything above must be zero
        let (high, em) = full.split_at(self.k - em_len);
        if high.iter().any(|&b| b != 0) || digest.len() != h_len || em_len < h_len + 2 || em[em_len - 1] != 0xbc {
            return false;
        }
        let db_len = em_len - h_len - 1;
        let (masked, h) = (&em[..db_len], &em[db_len..em_len - 1]);
        let top_bits = 8 * em_len - em_bits;
        if top_bits > 0 && masked[0] >> (8 - top_bits) != 0 {
            return false;
        }
        let mask = mgf1(hash, h, db_len);
        let mut db: Vec<u8> = masked.iter().zip(&mask).map(|(a, b)| a ^ b).collect();
        if top_bits > 0 {
            db[0] &= 0xFF >> top_bits;
        }
        let Some(one) = db.iter().position(|&b| b != 0) else { return false };
        if db[one] != 1 {
            return false;
        }
        let salt = &db[one + 1..];
        if salt_len.is_some_and(|s| s != salt.len()) {
            return false;
        }
        let mut m = alloc::vec![0u8; 8];
        m.extend_from_slice(digest);
        m.extend_from_slice(salt);
        hash.digest(&m) == h
    }
}

/// MGF1 (RFC 8017 B.2.1)
pub fn mgf1(hash: HashAlg, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 64);
    let mut c = 0u32;
    while out.len() < len {
        let mut x = Vec::from(seed);
        x.extend_from_slice(&c.to_be_bytes());
        out.extend_from_slice(&hash.digest(&x));
        c += 1;
    }
    out.truncate(len);
    out
}

// ---------------------------------------------------------------- private keys

/// the primes below 2000 without 2, for a quick look at a candidate before Miller-Rabin
fn small_primes() -> Vec<u32> {
    let mut v: Vec<u32> = Vec::new();
    for c in (3..2000u32).step_by(2) {
        if v.iter().take_while(|&&p| p * p <= c).all(|&p| c % p != 0) {
            v.push(c);
        }
    }
    v
}

/// a mod m for a small m
fn rem_small(a: &[u64], m: u32) -> u32 {
    let mut r = 0u128;
    for &x in a.iter().rev() {
        r = ((r << 64) | x as u128) % m as u128;
    }
    r as u32
}

/// a * b
fn mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut r = alloc::vec![0u64; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut c = 0u128;
        for (j, &y) in b.iter().enumerate() {
            let t = r[i + j] as u128 + x as u128 * y as u128 + c;
            r[i + j] = t as u64;
            c = t >> 64;
        }
        r[i + b.len()] = c as u64;
    }
    r
}

/// a - 1 for an odd a
fn minus_one(a: &[u64]) -> Vec<u64> {
    let mut v = Vec::from(a);
    v[0] &= !1;
    v
}

/// Miller-Rabin with `rounds` random bases
fn is_probable_prime(n: &[u64], rounds: u32, rand: &mut dyn FnMut(&mut [u8])) -> bool {
    use core::cmp::Ordering::{Equal, Less};
    let Some(m) = Mont::new(n) else { return false };
    let n1 = minus_one(n);
    let mut s = 0usize;
    while (n1[s / 64] >> (s % 64)) & 1 == 0 {
        s += 1;
    }
    // d = (n - 1) >> s, big-endian for pow
    let mut d = alloc::vec![0u64; n1.len()];
    for (i, x) in d.iter_mut().enumerate() {
        let (w, b) = (i + s / 64, s % 64);
        let lo = n1.get(w).copied().unwrap_or(0) >> b;
        let hi = if b == 0 { 0 } else { n1.get(w + 1).copied().unwrap_or(0) << (64 - b) };
        *x = lo | hi;
    }
    let dbe = bn::to_be(&d, d.len() * 8);
    let one = bn::from_be(&[1], m.limbs());
    let mut buf = alloc::vec![0u8; bn::bits(n).div_ceil(8)];
    for _ in 0..rounds {
        rand(&mut buf);
        // a base below n (its top bit is set, at a whole byte); 0, 1 and n - 1 say nothing
        buf[0] &= 0x7F;
        let a = bn::from_be(&buf, m.limbs());
        if bn::cmp(&a, &m.n) != Less || bn::bits(&a) < 2 || bn::cmp(&a, &n1) == Equal {
            continue;
        }
        let mut x = m.pow(&a, &dbe);
        if bn::cmp(&x, &one) == Equal || bn::cmp(&x, &n1) == Equal {
            continue;
        }
        let mut witness = true;
        for _ in 1..s {
            let mx = m.to_mont(&x);
            x = m.from_mont(&m.mul(&mx, &mx));
            if bn::cmp(&x, &n1) == Equal {
                witness = false;
                break;
            }
        }
        if witness {
            return false;
        }
    }
    true
}

/// a prime of exactly `bits` bits with its two top bits set (two of them multiply to twice
/// its length) and p mod 65537 != 1 (so 65537 has an inverse)
fn gen_prime(bits: usize, rand: &mut dyn FnMut(&mut [u8])) -> Vec<u64> {
    let primes = small_primes();
    let mut buf = alloc::vec![0u8; bits / 8];
    loop {
        rand(&mut buf);
        buf[0] |= 0xC0;
        let last = buf.len() - 1;
        buf[last] |= 1;
        let base = bn::from_be(&buf, 0);
        let rems: Vec<u32> = primes.iter().map(|&p| rem_small(&base, p)).collect();
        // the next numbers of the form base + 2 i
        'next: for i in 0..2048u32 {
            let step = i * 2;
            for (r, &p) in rems.iter().zip(&primes) {
                if (*r + step % p) % p == 0 {
                    continue 'next;
                }
            }
            let mut c = base.clone();
            let mut carry = step as u64;
            for x in c.iter_mut() {
                let (s, o) = x.overflowing_add(carry);
                *x = s;
                carry = o as u64;
            }
            if carry != 0 || bn::bits(&c) != bits {
                break;
            }
            if rem_small(&c, 65537) != 1 && is_probable_prime(&c, 16, rand) {
                return c;
            }
        }
    }
}

/// an RSA private key
#[derive(Clone)]
pub struct PrivateKey {
    m: Mont,
    k: usize,
    e: Vec<u8>,
    d: Vec<u8>,
    p: Vec<u8>,
    q: Vec<u8>,
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        crate::zeroize(&mut self.d);
        crate::zeroize(&mut self.p);
        crate::zeroize(&mut self.q);
    }
}

/// big-endian bytes of limbs without leading zeros
fn be_min(a: &[u64]) -> Vec<u8> {
    let b = bn::to_be(a, a.len() * 8);
    let at = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    Vec::from(&b[at..])
}

impl PrivateKey {
    /// a new key of `bits` bits (1024 to 8192, a multiple of 16): exponent 65537, primes made
    /// from `rand`
    pub fn generate(bits: usize, rand: &mut dyn FnMut(&mut [u8])) -> Option<PrivateKey> {
        if !(1024..=8192).contains(&bits) || bits % 16 != 0 {
            return None;
        }
        loop {
            let p = gen_prime(bits / 2, rand);
            let q = gen_prime(bits / 2, rand);
            if p == q {
                continue;
            }
            let (p, q) = if bn::cmp(&p, &q) == core::cmp::Ordering::Less { (q, p) } else { (p, q) };
            let n = mul(&p, &q);
            if bn::bits(&n) != bits {
                continue;
            }
            let phi = mul(&minus_one(&p), &minus_one(&q));
            // d = (1 + k phi) / 65537, where k = -phi^-1 mod 65537 (a prime: Fermat)
            const E: u64 = 65537;
            let mut inv = 1u64;
            let (mut b, mut x) = (rem_small(&phi, E as u32) as u64, E - 2);
            while x > 0 {
                if x & 1 == 1 {
                    inv = inv * b % E;
                }
                b = b * b % E;
                x >>= 1;
            }
            let k = (E - inv) % E;
            let mut t = mul(&phi, &[k]);
            let mut carry = 1u64;
            for w in t.iter_mut() {
                let (s, o) = w.overflowing_add(carry);
                *w = s;
                carry = o as u64;
            }
            let mut rem = 0u128;
            for w in t.iter_mut().rev() {
                let cur = (rem << 64) | *w as u128;
                *w = (cur / E as u128) as u64;
                rem = cur % E as u128;
            }
            if rem != 0 {
                continue;
            }
            return PrivateKey::from_parts(&be_min(&n), &[1, 0, 1], &be_min(&t), &be_min(&p), &be_min(&q));
        }
    }

    /// a key from its numbers (big-endian); None when p q is not n, or d does not undo e for a
    /// test value
    pub fn from_parts(n: &[u8], e: &[u8], d: &[u8], p: &[u8], q: &[u8]) -> Option<PrivateKey> {
        let at = n.iter().position(|&b| b != 0)?;
        let n = &n[at..];
        let limbs = bn::from_be(n, 0);
        let (pl, ql) = (bn::from_be(p, 0), bn::from_be(q, 0));
        if bn::cmp(&mul(&pl, &ql), &limbs) != core::cmp::Ordering::Equal {
            return None;
        }
        let m = Mont::new(&limbs)?;
        let x = bn::from_be(&[0x42, 0x13, 0x37], m.limbs());
        let y = m.pow_ct(&x, d);
        if bn::cmp(&m.pow(&y, e), &x) != core::cmp::Ordering::Equal {
            return None;
        }
        Some(PrivateKey { k: n.len(), m, e: Vec::from(e), d: Vec::from(d), p: Vec::from(p), q: Vec::from(q) })
    }

    /// the modulus's bits
    pub fn bits(&self) -> usize {
        bn::bits(&self.m.n)
    }
    /// the modulus, big-endian
    pub fn n(&self) -> Vec<u8> {
        bn::to_be(&self.m.n, self.k)
    }
    pub fn e(&self) -> &[u8] {
        &self.e
    }
    pub fn d(&self) -> &[u8] {
        &self.d
    }
    pub fn p(&self) -> &[u8] {
        &self.p
    }
    pub fn q(&self) -> &[u8] {
        &self.q
    }
    /// q^-1 mod p, big-endian (what key files keep beside the primes; p > q)
    pub fn iqmp(&self) -> Vec<u8> {
        let Some(mp) = Mont::new(&bn::from_be(&self.p, 0)) else {
            return Vec::new();
        };
        let q = bn::from_be(&self.q, mp.limbs());
        be_min(&mp.from_mont(&mp.inv_mont(&mp.to_mont(&q))))
    }
    /// the public half
    pub fn public(&self) -> Option<PublicKey> {
        PublicKey::new(&self.n(), &self.e)
    }

    /// RSASSA-PKCS1-v1_5 of a digest: k bytes (None: the digest has the wrong length or the
    /// modulus is too short for it)
    pub fn sign_pkcs1(&self, hash: HashAlg, digest: &[u8]) -> Option<Vec<u8>> {
        if digest.len() != hash.len() {
            return None;
        }
        let oid_last = match hash {
            HashAlg::Sha256 => 1,
            HashAlg::Sha384 => 2,
            HashAlg::Sha512 => 3,
        };
        let oid = [0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, oid_last];
        let mut t = Vec::new();
        t.extend_from_slice(&[0x30, (oid.len() + 2 + 2 + 2 + digest.len()) as u8, 0x30, oid.len() as u8 + 2]);
        t.extend_from_slice(&oid);
        t.extend_from_slice(&[0x05, 0x00, 0x04, digest.len() as u8]);
        t.extend_from_slice(digest);
        if self.k < t.len() + 11 {
            return None;
        }
        let mut em = alloc::vec![0xFFu8; self.k];
        em[0] = 0;
        em[1] = 1;
        let at = self.k - t.len();
        em[at - 1] = 0;
        em[at..].copy_from_slice(&t);
        let x = bn::from_be(&em, self.m.limbs());
        Some(bn::to_be(&self.m.pow_ct(&x, &self.d), self.k))
    }
}
