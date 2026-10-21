// hmac -- HMAC (RFC 2104) over any Hash, HKDF (RFC 5869), PBKDF2 (RFC 8018)

use crate::sha2::{Sha256, Sha512};
use crate::{zeroize, Hash};

/// HMAC with a hash function H
#[derive(Clone)]
pub struct Hmac<H: Hash> {
    inner: H,
    outer: H,
}

impl<H: Hash> Hmac<H> {
    pub fn new(key: &[u8]) -> Hmac<H> {
        let mut k = [0u8; 128];
        if key.len() > H::BLOCK {
            let mut h = H::new();
            h.update(key);
            h.finish_into(&mut k[..H::OUT]);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut pad = [0u8; 128];
        for (p, k) in pad.iter_mut().zip(&k).take(H::BLOCK) {
            *p = k ^ 0x36;
        }
        let mut inner = H::new();
        inner.update(&pad[..H::BLOCK]);
        for (p, k) in pad.iter_mut().zip(&k).take(H::BLOCK) {
            *p = k ^ 0x5c;
        }
        let mut outer = H::new();
        outer.update(&pad[..H::BLOCK]);
        zeroize(&mut k);
        zeroize(&mut pad);
        Hmac { inner, outer }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// the tag into out[..H::OUT]
    pub fn finish_into(self, out: &mut [u8]) {
        let mut ih = [0u8; 64];
        self.inner.finish_into(&mut ih[..H::OUT]);
        let mut o = self.outer;
        o.update(&ih[..H::OUT]);
        o.finish_into(out);
    }
}

/// HMAC-SHA256 of a message
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new(key);
    m.update(data);
    let mut out = [0u8; 32];
    m.finish_into(&mut out);
    out
}

/// HMAC-SHA512 of a message
pub fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    let mut m = Hmac::<Sha512>::new(key);
    m.update(data);
    let mut out = [0u8; 64];
    m.finish_into(&mut out);
    out
}

/// HKDF-Extract: the pseudo-random key (into prk[..H::OUT])
pub fn hkdf_extract<H: Hash>(salt: &[u8], ikm: &[u8], prk: &mut [u8]) {
    let zero = [0u8; 64];
    let mut m = Hmac::<H>::new(if salt.is_empty() { &zero[..H::OUT] } else { salt });
    m.update(ikm);
    m.finish_into(prk);
}

/// HKDF-Expand: fill `out` (at most 255 * H::OUT bytes; panics beyond)
pub fn hkdf_expand<H: Hash>(prk: &[u8], info: &[u8], out: &mut [u8]) {
    assert!(out.len() <= 255 * H::OUT, "hkdf: output too long");
    let base = Hmac::<H>::new(prk);
    let mut t = [0u8; 64];
    let mut tlen = 0;
    for (i, chunk) in out.chunks_mut(H::OUT).enumerate() {
        let mut m = base.clone();
        m.update(&t[..tlen]);
        m.update(info);
        m.update(&[(i + 1) as u8]);
        m.finish_into(&mut t[..H::OUT]);
        tlen = H::OUT;
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
    zeroize(&mut t);
}

/// HKDF-SHA256 (extract and expand)
pub fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    let mut prk = [0u8; 32];
    hkdf_extract::<Sha256>(salt, ikm, &mut prk);
    hkdf_expand::<Sha256>(&prk, info, out);
    zeroize(&mut prk);
}

/// HKDF-SHA512 (extract and expand)
pub fn hkdf_sha512(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    let mut prk = [0u8; 64];
    hkdf_extract::<Sha512>(salt, ikm, &mut prk);
    hkdf_expand::<Sha512>(&prk, info, out);
    zeroize(&mut prk);
}

/// PBKDF2-HMAC-SHA256 (only for importing foreign key files; new keys use Argon2id)
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let base = Hmac::<Sha256>::new(password);
    for (i, chunk) in out.chunks_mut(32).enumerate() {
        let mut m = base.clone();
        m.update(salt);
        m.update(&(i as u32 + 1).to_be_bytes());
        let mut u = [0u8; 32];
        m.finish_into(&mut u);
        let mut acc = u;
        for _ in 1..iterations.max(1) {
            let mut m = base.clone();
            m.update(&u);
            m.finish_into(&mut u);
            for (a, b) in acc.iter_mut().zip(&u) {
                *a ^= b;
            }
        }
        chunk.copy_from_slice(&acc[..chunk.len()]);
        zeroize(&mut u);
        zeroize(&mut acc);
    }
}
