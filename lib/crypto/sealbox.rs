// sealbox -- anonymous public-key encryption: whoever holds the recipient's X25519 public key
// can seal, only the secret key opens.  (Recovery kits: the repository key sealed to an
// offline key pair.)
//
//   sealed = ephemeral public (32) ‖ nonce (24) ‖ XChaCha20-Poly1305(key, nonce, aad, plain)
//   key    = HKDF-SHA256(salt = ephemeral public ‖ recipient public,
//                        ikm = X25519(ephemeral secret, recipient public),
//                        info = "JFLib sealbox v1")

use alloc::vec::Vec;

use crate::{aead, hkdf_sha256, x25519};

pub const OVERHEAD: usize = 32 + 24 + 16;
const INFO: &[u8] = b"JFLib sealbox v1";

fn key(shared: &[u8; 32], eph_pub: &[u8; 32], recipient: &[u8; 32]) -> [u8; 32] {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(eph_pub);
    salt[32..].copy_from_slice(recipient);
    let mut k = [0u8; 32];
    hkdf_sha256(&salt, shared, INFO, &mut k);
    k
}

/// seal `plain` for `recipient`; `random` = 32 bytes ephemeral secret ‖ 24 bytes nonce
/// (None: a degenerate recipient key)
pub fn seal(recipient: &[u8; 32], random: &[u8; 56], aad: &[u8], plain: &[u8]) -> Option<Vec<u8>> {
    let mut eph = [0u8; 32];
    eph.copy_from_slice(&random[..32]);
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&random[32..]);
    let eph_pub = x25519::public_key(&eph);
    let mut shared = x25519::shared(&eph, recipient)?;
    crate::zeroize(&mut eph);
    let mut k = key(&shared, &eph_pub, recipient);
    crate::zeroize(&mut shared);
    let mut out = Vec::with_capacity(plain.len() + OVERHEAD);
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&aead::x_seal(&k, &nonce, aad, plain));
    crate::zeroize(&mut k);
    Some(out)
}

/// open a sealed box with the recipient's secret key (None: not for this key or altered)
pub fn open(secret: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < OVERHEAD {
        return None;
    }
    let mut eph_pub = [0u8; 32];
    eph_pub.copy_from_slice(&sealed[..32]);
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&sealed[32..56]);
    let recipient = x25519::public_key(secret);
    let mut shared = x25519::shared(secret, &eph_pub)?;
    let mut k = key(&shared, &eph_pub, &recipient);
    crate::zeroize(&mut shared);
    let out = aead::x_open(&k, &nonce, aad, &sealed[56..]);
    crate::zeroize(&mut k);
    out
}
