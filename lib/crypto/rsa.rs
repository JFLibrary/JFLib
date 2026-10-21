// rsa -- RSA signature verification (RFC 8017): PKCS #1 v1.5 and PSS with MGF1
//
//   PublicKey::new(n, e)                      big-endian modulus (1024..8192 bits) and exponent
//   key.verify_pkcs1(hash, digest, sig)       the encoding is rebuilt and compared whole
//   key.verify_pss(hash, digest, sig, salt)   salt length given, or None: any
//
// Public operations only (variable time is fine).

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
