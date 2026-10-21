// keysched -- the TLS 1.3 key schedule with SHA-256 (RFC 8446 7.1): HKDF-Expand-Label,
// Derive-Secret, traffic keys and finished keys; the TLS 1.2 PRF with SHA-256 (RFC 5246 5)

use alloc::vec::Vec;

use crypto::hmac::{hkdf_expand, hkdf_extract};
use crypto::Sha256;

pub type Secret = [u8; 32];

/// HKDF-Expand-Label(secret, label, context, len)
pub fn expand_label(secret: &[u8], label: &str, context: &[u8], out: &mut [u8]) {
    let full = [b"tls13 ", label.as_bytes()].concat();
    let mut info = Vec::with_capacity(4 + full.len() + context.len());
    info.extend_from_slice(&(out.len() as u16).to_be_bytes());
    info.push(full.len() as u8);
    info.extend_from_slice(&full);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand::<Sha256>(secret, &info, out);
}

/// Derive-Secret(secret, label, transcript hash)
pub fn derive(secret: &Secret, label: &str, hash: &[u8; 32]) -> Secret {
    let mut out = [0u8; 32];
    expand_label(secret, label, hash, &mut out);
    out
}

pub fn extract(salt: &[u8], ikm: &[u8]) -> Secret {
    let mut out = [0u8; 32];
    hkdf_extract::<Sha256>(salt, ikm, &mut out);
    out
}

/// the hash of no messages
pub fn empty_hash() -> [u8; 32] {
    crypto::sha256(b"")
}

/// the secrets of one handshake
pub struct Schedule {
    pub handshake: Secret,
    pub master: Secret,
}

impl Schedule {
    /// from the (EC)DHE shared secret (no PSK)
    pub fn new(ecdhe: &[u8; 32]) -> Schedule {
        let early = extract(&[0u8; 32], &[0u8; 32]);
        let derived = derive(&early, "derived", &empty_hash());
        let handshake = extract(&derived, ecdhe);
        let derived2 = derive(&handshake, "derived", &empty_hash());
        let master = extract(&derived2, &[0u8; 32]);
        Schedule { handshake, master }
    }
}

/// the AEAD key (of key_len bytes) and IV of a traffic secret
pub fn traffic_keys_len(secret: &Secret, key_len: usize) -> (Vec<u8>, [u8; 12]) {
    let mut key = alloc::vec![0u8; key_len];
    let mut iv = [0u8; 12];
    expand_label(secret, "key", &[], &mut key);
    expand_label(secret, "iv", &[], &mut iv);
    (key, iv)
}

/// TLS 1.2: PRF(secret, label, seed) with P_SHA256
pub fn prf12(secret: &[u8], label: &str, seed: &[u8], out: &mut [u8]) {
    let mut ls = Vec::from(label.as_bytes());
    ls.extend_from_slice(seed);
    let mut a = crypto::hmac_sha256(secret, &ls);
    let mut off = 0;
    while off < out.len() {
        let mut m = Vec::from(&a[..]);
        m.extend_from_slice(&ls);
        let block = crypto::hmac_sha256(secret, &m);
        let n = (out.len() - off).min(32);
        out[off..off + n].copy_from_slice(&block[..n]);
        off += n;
        a = crypto::hmac_sha256(secret, &a);
    }
}

/// the AEAD key and IV of a traffic secret
pub fn traffic_keys(secret: &Secret) -> ([u8; 32], [u8; 12]) {
    let mut key = [0u8; 32];
    let mut iv = [0u8; 12];
    expand_label(secret, "key", &[], &mut key);
    expand_label(secret, "iv", &[], &mut iv);
    (key, iv)
}

/// verify_data of a Finished message
pub fn finished(base: &Secret, transcript: &[u8; 32]) -> [u8; 32] {
    let mut fk = [0u8; 32];
    expand_label(base, "finished", &[], &mut fk);
    crypto::hmac_sha256(&fk, transcript)
}

/// the next traffic secret (KeyUpdate)
pub fn update(secret: &Secret) -> Secret {
    let mut out = [0u8; 32];
    expand_label(secret, "traffic upd", &[], &mut out);
    out
}
