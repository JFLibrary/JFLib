// x25519 -- Diffie-Hellman on Curve25519 (RFC 7748): Montgomery ladder with masked swaps
//
//   public_key(&secret) -> [u8; 32]
//   shared(&secret, &their_public) -> Option<[u8; 32]>   (None: an all-zero result, i.e. a
//                                                         low-order point was offered)

use crate::field::Fe;

const BASE: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

/// the scalar multiplication k * u (k clamped as RFC 7748 requires)
pub fn x25519(k: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut e = *k;
    e[0] &= 248;
    e[31] &= 127;
    e[31] |= 64;
    let x1 = Fe::from_bytes(u);
    let mut x2 = Fe::ONE;
    let mut z2 = Fe::ZERO;
    let mut x3 = x1;
    let mut z3 = Fe::ONE;
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let bit = ((e[t >> 3] >> (t & 7)) & 1) as u64;
        swap ^= bit;
        Fe::cswap(&mut x2, &mut x3, swap);
        Fe::cswap(&mut z2, &mut z3, swap);
        swap = bit;
        let a = x2.add(&z2);
        let aa = a.square();
        let b = x2.sub(&z2);
        let bb = b.square();
        let ee = aa.sub(&bb);
        let c = x3.add(&z3);
        let d = x3.sub(&z3);
        let da = d.mul(&a);
        let cb = c.mul(&b);
        x3 = da.add(&cb).square();
        z3 = x1.mul(&da.sub(&cb).square());
        x2 = aa.mul(&bb);
        z2 = ee.mul(&aa.add(&ee.mul_small(121665)));
    }
    Fe::cswap(&mut x2, &mut x3, swap);
    Fe::cswap(&mut z2, &mut z3, swap);
    crate::zeroize(&mut e);
    x2.mul(&z2.invert()).to_bytes()
}

/// the public key of a 32-byte secret
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    x25519(secret, &BASE)
}

/// the shared secret with a peer (None when it is all zero)
pub fn shared(secret: &[u8; 32], their_public: &[u8; 32]) -> Option<[u8; 32]> {
    let s = x25519(secret, their_public);
    if crate::ct_eq(&s, &[0u8; 32]) { None } else { Some(s) }
}
