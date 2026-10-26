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
//
// No randomness here: callers pass random bytes in (plat::fill_random), so everything is
// deterministic and testable.  Secret-dependent work avoids branches and table lookups
// (field arithmetic with 51-bit limbs, masked conditional swaps and selects).
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

use alloc::string::String;
use alloc::vec::Vec;

pub use blake3::Blake3;
pub use hmac::{hkdf_sha256, hkdf_sha512, hmac_sha256, hmac_sha512, pbkdf2_sha256, Hmac};
pub use sha2::{sha256, sha384, sha512, Sha256, Sha384, Sha512};
pub use sha1::{sha1, Sha1};
pub use hmac::{hmac_sha1, pbkdf2, pbkdf2_sha1};
pub use keywrap::{aes_unwrap, aes_wrap};
pub use cmac::cmac_aes;

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

/// lower-case hex
pub fn hex(data: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

/// the bytes of a hex string (upper or lower case; None on odd length or other characters)
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.chunks(2).map(|p| Some(nib(p[0])? << 4 | nib(p[1])?)).collect()
}

#[inline(always)]
pub(crate) fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline(always)]
pub(crate) fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
