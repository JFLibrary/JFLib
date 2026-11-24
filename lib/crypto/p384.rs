// p384 -- Diffie-Hellman on NIST P-384 (SEC 1, RFC 8446 4.2.8.2) in constant time
//
//   public_key(&secret) -> [u8; 97]          (panics on an invalid secret) 04 || x || y of secret * G
//   shared(&secret, &peer_point) -> Option<[u8; 48]>   the x coordinate of secret * peer;
//                                            None for a point that is not on the curve
//   secret_from(random) -> Option<[u8; 48]>  a scalar in [1, n - 1] from 48 random bytes
//
// The arithmetic is nist.rs's: six limbs in Montgomery form, complete addition formulas,
// four-bit windows that read the whole table.

use crate::nist::{self, P384};

/// a scalar in [1, n - 1] from random bytes (None: draw again); only the answer is a branch
pub fn secret_from(random: &[u8; 48]) -> Option<[u8; 48]> {
    (P384.n.in_range_mask(&nist::from_be::<6>(random)) != 0).then_some(*random)
}

/// the public key of a secret scalar: 04 || x || y
pub fn public_key(secret: &[u8; 48]) -> [u8; 97] {
    assert!(secret_from(secret).is_some(), "p384 secret outside [1, n - 1]");
    let mut out = [0u8; 97];
    out.copy_from_slice(&P384.public_key(secret));
    out
}

/// the shared secret (x coordinate) with a peer's public point (04 || x || y)
pub fn shared(secret: &[u8; 48], peer: &[u8]) -> Option<[u8; 48]> {
    let q = P384.point(peer)?;
    // the group has prime order and no cofactor: only a secret that is 0 mod n gives the identity
    let (finite, x, _) = P384.affine(&P384.mul(secret, &q));
    if finite == 0 {
        return None;
    }
    let mut out = [0u8; 48];
    nist::to_be(&x, &mut out);
    Some(out)
}
