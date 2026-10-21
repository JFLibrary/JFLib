// aead -- ChaCha20-Poly1305 (RFC 8439) and XChaCha20-Poly1305 (192-bit nonces: random nonces
// are safe)
//
//   seal_in_place(key, nonce, aad, buf) -> tag          open_in_place(.., buf, &tag) -> ok
//   seal(key, nonce, aad, plain) -> ciphertext ‖ tag    open(.., sealed) -> Some(plain)
//   (x_* : the same with a 24-byte nonce)

use alloc::vec::Vec;

use crate::chacha;
use crate::poly1305::Poly1305;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const XNONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

fn tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let block0 = chacha::block(key, 0, nonce);
    let mut otk = [0u8; 32];
    otk.copy_from_slice(&block0[..32]);
    let mut p = Poly1305::new(&otk);
    crate::zeroize(&mut otk);
    p.update(aad);
    p.pad16();
    p.update(ct);
    p.pad16();
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lens[8..].copy_from_slice(&(ct.len() as u64).to_le_bytes());
    p.update(&lens);
    p.finalize()
}

/// encrypt buf in place -> the tag
pub fn seal_in_place(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], buf: &mut [u8]) -> [u8; 16] {
    chacha::xor(key, 1, nonce, buf);
    tag(key, nonce, aad, buf)
}

/// check the tag and decrypt buf in place -> false (buf unchanged) on a wrong tag
pub fn open_in_place(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], buf: &mut [u8], t: &[u8; 16]) -> bool {
    let want = tag(key, nonce, aad, buf);
    if !crate::ct_eq(&want, t) {
        return false;
    }
    chacha::xor(key, 1, nonce, buf);
    true
}

/// ciphertext ‖ tag
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plain.len() + TAG_LEN);
    out.extend_from_slice(plain);
    let t = seal_in_place(key, nonce, aad, &mut out);
    out.extend_from_slice(&t);
    out
}

/// the plaintext of ciphertext ‖ tag (None: too short or forged)
pub fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let n = sealed.len().checked_sub(TAG_LEN)?;
    let mut t = [0u8; 16];
    t.copy_from_slice(&sealed[n..]);
    let mut buf = sealed[..n].to_vec();
    open_in_place(key, nonce, aad, &mut buf, &t).then_some(buf)
}

/// XChaCha20-Poly1305: encrypt buf in place -> the tag
pub fn x_seal_in_place(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], buf: &mut [u8]) -> [u8; 16] {
    let (mut sub, n12) = chacha::xchacha_setup(key, nonce);
    let t = seal_in_place(&sub, &n12, aad, buf);
    crate::zeroize(&mut sub);
    t
}

/// XChaCha20-Poly1305: check and decrypt in place
pub fn x_open_in_place(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], buf: &mut [u8], t: &[u8; 16]) -> bool {
    let (mut sub, n12) = chacha::xchacha_setup(key, nonce);
    let ok = open_in_place(&sub, &n12, aad, buf, t);
    crate::zeroize(&mut sub);
    ok
}

/// XChaCha20-Poly1305: ciphertext ‖ tag
pub fn x_seal(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let (mut sub, n12) = chacha::xchacha_setup(key, nonce);
    let out = seal(&sub, &n12, aad, plain);
    crate::zeroize(&mut sub);
    out
}

/// XChaCha20-Poly1305: the plaintext of ciphertext ‖ tag
pub fn x_open(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (mut sub, n12) = chacha::xchacha_setup(key, nonce);
    let out = open(&sub, &n12, aad, sealed);
    crate::zeroize(&mut sub);
    out
}
