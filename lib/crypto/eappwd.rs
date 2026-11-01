// eappwd -- the sums of EAP-PWD (RFC 5931) on group 19 (P-256) with HMAC-SHA-256 as the random
// function and the PRF: a password-only EAP method that needs no certificates and derives an
// MSK and an EMSK.  Nothing of its packets here (netpkt::eap) and no state (eapm)
//
//   GROUP, RAND_FUNC, PRF                       the ciphersuite this knows; ciphersuite() its bytes
//   kdf(key, label, n)                          n bytes of the KDF of RFC 5931 2.5
//   pwe(token, peer_id, server_id, password)    the password element: hunting and pecking (2.8.3)
//   commit(pwe, rand, mask)                     (Scalar, Element) of one side, from its two random
//                                               scalars (2.8.5.1); None to draw again
//   shared(pwe, rand, peer_scalar, peer_element)   ss, the x coordinate of k (2.8.5.2); None for
//                                               what a peer must not send
//   confirm(ss, element_a, scalar_a, element_b, scalar_b)   H(ss | own | other's | ciphersuite)
//                                               (2.8.5.3)
//   keys(ss, confirm_p, confirm_s, scalar_p, scalar_s)   (MSK, EMSK) (2.9)
//
// H(x) is HMAC-SHA-256 with a key of 32 zero bytes.  This looks for PWE the plain way, which
// shows by its time how many rounds a password needs; RFC 8146 and later hostapd run a fixed
// number of them.  The simulator has no such observer.

use alloc::vec::Vec;

use crate::hmac::hmac_sha256;
use crate::p256;

/// the group's number in IANA's registry: P-256
pub const GROUP: u16 = 19;
/// the random function: HMAC-SHA-256
pub const RAND_FUNC: u8 = 1;
/// the PRF: HMAC-SHA-256
pub const PRF: u8 = 1;

const HUNT: &[u8] = b"EAP-pwd Hunting And Pecking";

/// group | random function | PRF
pub fn ciphersuite() -> [u8; 4] {
    let g = GROUP.to_be_bytes();
    [g[0], g[1], RAND_FUNC, PRF]
}

/// H over the concatenation of the parts
fn h(parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Vec::new();
    for p in parts {
        m.extend_from_slice(p);
    }
    hmac_sha256(&[0u8; 32], &m)
}

/// K(i) = HMAC(key, K(i-1) | i | label | L) with i and L (in bits) as 16-bit numbers, as many of
/// them as `n` bytes take
pub fn kdf(key: &[u8], label: &[u8], n: usize) -> Vec<u8> {
    let bits = ((n * 8) as u16).to_be_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(n + 32);
    let mut last: Vec<u8> = Vec::new();
    let mut i = 1u16;
    while out.len() < n {
        let mut m = last.clone();
        m.extend_from_slice(&i.to_be_bytes());
        m.extend_from_slice(label);
        m.extend_from_slice(&bits);
        last = hmac_sha256(key, &m).to_vec();
        out.extend_from_slice(&last);
        i += 1;
    }
    out.truncate(n);
    out
}

/// the password element: a point both sides find from the password and their names; the counter
/// starts at 1, the lowest bit of pwd-seed picks y
pub fn pwe(token: &[u8; 4], peer_id: &[u8], server_id: &[u8], password: &[u8]) -> [u8; 64] {
    let mut counter = 0u8;
    loop {
        counter = counter.wrapping_add(1);
        let seed = h(&[token, peer_id, server_id, password, &[counter]]);
        let value = kdf(&seed, HUNT, 32);
        let mut x = [0u8; 32];
        x.copy_from_slice(&value);
        if let Some(y) = p256::y_for_x(&x, seed[31] & 1 == 1) {
            let mut out = [0u8; 64];
            out[..32].copy_from_slice(&x);
            out[32..].copy_from_slice(&y);
            return out;
        }
    }
}

/// Scalar = rand + mask mod n, Element = -(mask * PWE); None when a random value is no scalar or
/// Scalar comes to 0 or 1
pub fn commit(pwe: &[u8; 64], rand: &[u8; 32], mask: &[u8; 32]) -> Option<([u8; 32], [u8; 64])> {
    if !p256::scalar_ok(rand) || !p256::scalar_ok(mask) {
        return None;
    }
    let scalar = p256::scalar_add(rand, mask);
    let mut one = [0u8; 32];
    one[31] = 1;
    if !p256::scalar_ok(&scalar) || scalar == one {
        return None;
    }
    Some((scalar, p256::neg_point(&p256::mul_point(mask, pwe)?)))
}

/// the x coordinate of k = rand * (peer_scalar * PWE + peer_element); None for a scalar out of
/// range, an element off the curve, or k at the identity
pub fn shared(pwe: &[u8; 64], rand: &[u8; 32], peer_scalar: &[u8; 32], peer_element: &[u8; 64]) -> Option<[u8; 32]> {
    if !p256::scalar_ok(peer_scalar) || !p256::on_curve(peer_element) {
        return None;
    }
    let sum = p256::add_points(&p256::mul_point(peer_scalar, pwe)?, peer_element)?;
    let k = p256::mul_point(rand, &sum)?;
    let mut ss = [0u8; 32];
    ss.copy_from_slice(&k[..32]);
    Some(ss)
}

/// Confirm of a side: H(ss | its element | its scalar | the other's element | scalar | ciphersuite)
pub fn confirm(ss: &[u8; 32], element_a: &[u8; 64], scalar_a: &[u8; 32], element_b: &[u8; 64], scalar_b: &[u8; 32]) -> [u8; 32] {
    h(&[ss, element_a, scalar_a, element_b, scalar_b, &ciphersuite()])
}

/// MK = H(ss | Confirm_P | Confirm_S), stretched with the session id (the type byte 52 and
/// H(ciphersuite | Scalar_P | Scalar_S)) to 128 bytes: the MSK, then the EMSK
pub fn keys(ss: &[u8; 32], confirm_p: &[u8; 32], confirm_s: &[u8; 32], scalar_p: &[u8; 32], scalar_s: &[u8; 32]) -> ([u8; 64], [u8; 64]) {
    let mk = h(&[ss, confirm_p, confirm_s]);
    let sid = h(&[&ciphersuite(), scalar_p, scalar_s]);
    let mut session = alloc::vec![52u8];
    session.extend_from_slice(&sid);
    let b = kdf(&mk, &session, 128);
    let (mut msk, mut emsk) = ([0u8; 64], [0u8; 64]);
    msk.copy_from_slice(&b[..64]);
    emsk.copy_from_slice(&b[64..]);
    (msk, emsk)
}
