// crypto -- the primitives BackupCraft (and anything else) builds on, in plain Rust
//
//   sha2      Sha256 / Sha384 / Sha512, sha256 / sha384 / sha512(data)         (FIPS 180-4)
//   hmac      Hmac<H>, hmac_sha256 / hmac_sha512, hkdf_sha256 / hkdf_sha512,
//             pbkdf2_sha256 (import of foreign key files only)                (RFC 2104/5869/8018)
//   blake2b   Blake2b (keyed, 1..64 byte digests)                             (RFC 7693)
//   blake3    Blake3 (hash, keyed_hash, derive_key, any output length)
//   argon2    argon2id(password, salt, params, out), PHC strings hash / verify (RFC 9106)
//   chacha    ChaCha20 (IETF), HChaCha20, XChaCha20                           (RFC 8439)
//   poly1305  Poly1305 one-time authenticator                                 (RFC 8439)
//   aead      ChaCha20-Poly1305 and XChaCha20-Poly1305 seal / open            (RFC 8439)
//   x25519    Diffie-Hellman over Curve25519                                  (RFC 7748)
//   ed25519   signatures, strict verification                                 (RFC 8032)
//   sealbox   anonymous public-key encryption: X25519 + HKDF-SHA256 + XChaCha20-Poly1305
//   aes, gcm  AES-128/256 (bitsliced, constant time; both directions), AES-GCM seal / open
//             (FIPS 197, SP 800-38D)
//   bn        big integers modulo odd numbers (public values: variable time; pow_ct for a
//             secret exponent)
//   rsa       RSA PKCS #1 v1.5 and PSS verification                            (RFC 8017)
//   ecdsa     ECDSA verification on P-256 / P-384                              (FIPS 186-4)
//   p256      Diffie-Hellman on P-256 in constant time (complete formulas)      (SEC 1)
//   b64       Base64 (standard and URL alphabet, with or without padding)
//   ct_eq / zeroize / hex
//   sha1      Sha1, sha1(data); with hmac_sha1, pbkdf2 (any Hash) and pbkdf2_sha1 in hmac:
//             only for protocols defined with SHA-1 (WPA2-Personal)          (FIPS 180-4)
//   keywrap   aes_wrap / aes_unwrap: the AES key wrap                         (RFC 3394)
//   cmac      cmac_aes: AES-CMAC                                              (RFC 4493)
//   md5       Md5, md5(data), md5_parts(parts); with hmac_md5 in hmac: only for protocols
//             defined with MD5 (RADIUS); broken for collisions                 (RFC 1321, 2104)
//   rsa also  PrivateKey: generate from the caller's bytes, PKCS #1 v1.5 signing (RFC 8017)
//   eappwd    the sums of EAP-PWD on P-256: pwe, commit, shared, confirm, keys  (RFC 5931)
//
// No randomness here: callers pass random bytes in (plat::fill_random on Windows, Linux, Apple
// and Android; in the browser the page's crypto.getRandomValues), so everything is
// deterministic and testable.  Secret-dependent work avoids branches and table lookups
// (field arithmetic with 51-bit limbs, masked conditional swaps and selects); every mask made
// from secret data passes ct_mask, so the optimizer cannot turn a select back into a branch.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod aead;
pub mod aes;
pub mod argon2;
pub mod b64;
pub mod blake2b;
pub mod blake3;
pub mod bn;
pub mod chacha;
pub mod ecdsa;
pub mod ed25519;
mod field;
pub mod gcm;
pub mod hmac;
pub mod p256;
pub mod poly1305;
pub mod rsa;
mod scalar;
pub mod sealbox;
pub mod sha2;
pub mod x25519;
pub mod sha1;
pub mod keywrap;
pub mod cmac;
pub mod md5;
pub mod eappwd;

use alloc::string::String;
use alloc::vec::Vec;

pub use blake3::Blake3;
pub use hmac::{hkdf_sha256, hkdf_sha512, hmac_sha256, hmac_sha512, pbkdf2_sha256, Hmac};
pub use sha2::{sha256, sha384, sha512, Sha256, Sha384, Sha512};
pub use sha1::{sha1, Sha1};
pub use hmac::{hmac_sha1, pbkdf2, pbkdf2_sha1};
pub use keywrap::{aes_unwrap, aes_wrap};
pub use cmac::cmac_aes;
pub use md5::{md5, md5_parts, Md5};
pub use hmac::hmac_md5;

/// a hash function for Hmac / HKDF / PBKDF2
pub trait Hash: Clone {
    /// block size in bytes (at most 128)
    const BLOCK: usize;
    /// digest size in bytes (at most 64)
    const OUT: usize;
    fn new() -> Self;
    fn update(&mut self, data: &[u8]);
    /// the digest into out[..OUT]
    fn finish_into(self, out: &mut [u8]);
}

/// the hash functions of signatures
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HashAlg {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlg {
    pub fn len(self) -> usize {
        match self {
            HashAlg::Sha256 => 32,
            HashAlg::Sha384 => 48,
            HashAlg::Sha512 => 64,
        }
    }

    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            HashAlg::Sha256 => sha256(data).to_vec(),
            HashAlg::Sha384 => sha384(data).to_vec(),
            HashAlg::Sha512 => sha512(data).to_vec(),
        }
    }
}

/// equality of two byte strings in time that depends only on their lengths
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    core::hint::black_box(d) == 0
}

/// overwrite a secret (the write is kept even though nothing reads the buffer afterwards)
pub fn zeroize(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        *b = 0;
    }
    core::hint::black_box(&buf);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// zeroize for words (like zeroize)
pub(crate) fn zeroize_words(buf: &mut [u64]) {
    for w in buf.iter_mut() {
        *w = 0;
    }
    core::hint::black_box(&buf);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// a mask made from secret data (0 or all ones), hidden from the optimizer.  Without it LLVM
/// sees that the value is one of the two and may compile `(a & m) | (b & !m)` back into a
/// branch, or a select over all entries of a table into a lookup of one.  On x86-64 with Rust
/// 1.98 it did both: branches for the final subtractions of p256's field arithmetic and of
/// scalar::reduce_limbs, a jump table for the window of p256::scalar_mul.  black_box is a best
/// effort, so the machine code of the secret paths is what has to be checked: tools/ct_check.py
/// runs them with different secrets and compares the branches they take.
#[inline(always)]
pub(crate) fn ct_mask(m: u64) -> u64 {
    core::hint::black_box(m)
}

/// lower-case hex (without a table: keys are printed this way too)
pub fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        for n in [b >> 4, b & 15] {
            // '0' + n, and 'a' - '0' - 10 more above 9
            let above9 = ct_mask(((9i32 - n as i32) >> 31) as u32 as u64) as u8;
            s.push((b'0' + n + (above9 & (b'a' - b'0' - 10))) as char);
        }
    }
    s
}

/// the bytes of a hex string (upper or lower case; None on odd length or other characters).
/// Every character is looked at before the answer, without a branch on it
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    // (value, all ones when c is a hex digit)
    let nib = |c: u8| -> (u8, u8) {
        let c = c as i32;
        let range = |lo: i32, hi: i32| ct_mask((((lo - 1 - c) & (c - hi - 1)) >> 31) as u32 as u64) as u8;
        let (dig, low, up) = (range(48, 57), range(97, 102), range(65, 70));
        let v = (dig & (c - 48) as u8) | (low & (c - 87) as u8) | (up & (c - 55) as u8);
        (v, dig | low | up)
    };
    let mut good = 0xFFu8;
    let mut out = Vec::with_capacity(b.len() / 2);
    for p in b.chunks(2) {
        let ((hi, a), (lo, z)) = (nib(p[0]), nib(p[1]));
        good &= a & z;
        out.push(hi << 4 | lo);
    }
    if good == 0 {
        zeroize(&mut out);
        return None;
    }
    Some(out)
}

#[inline(always)]
pub(crate) fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline(always)]
pub(crate) fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
