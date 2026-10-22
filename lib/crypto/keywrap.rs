// keywrap -- the AES key wrap of RFC 3394 (the default initial value A6A6A6A6A6A6A6A6): keys
// that travel encrypted under a key-encrypting key, as WPA2 and WPA3 hand the group key to a
// station in message 3 of the 4-way handshake
//
//   aes_wrap(kek, plain) -> Option<Vec<u8>>     plain: a multiple of 8 bytes, at least 16;
//                                               the result is 8 bytes longer
//   aes_unwrap(kek, wrapped) -> Option<Vec<u8>> None when the integrity check fails
//
// kek is a 16- or 32-byte AES key.

use alloc::vec::Vec;

use crate::aes::Aes;

const IV: [u8; 8] = [0xA6; 8];

/// wraps `plain` (n * 8 bytes, n >= 2) under `kek`
pub fn aes_wrap(kek: &[u8], plain: &[u8]) -> Option<Vec<u8>> {
    if plain.len() < 16 || plain.len() % 8 != 0 {
        return None;
    }
    let aes = Aes::new(kek)?;
    let n = plain.len() / 8;
    let mut a = IV;
    let mut r: Vec<[u8; 8]> = plain.chunks_exact(8).map(|c| [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).collect();
    for j in 0..6u64 {
        for (i, ri) in r.iter_mut().enumerate() {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            b[8..].copy_from_slice(ri);
            aes.encrypt(&mut b);
            let t = j * n as u64 + i as u64 + 1;
            a.copy_from_slice(&b[..8]);
            for (x, y) in a.iter_mut().zip(t.to_be_bytes()) {
                *x ^= y;
            }
            ri.copy_from_slice(&b[8..]);
        }
    }
    let mut out = Vec::with_capacity(plain.len() + 8);
    out.extend_from_slice(&a);
    for ri in &r {
        out.extend_from_slice(ri);
    }
    Some(out)
}

/// unwraps what aes_wrap made; None when it is not a multiple of 8 bytes of at least 24 or the
/// check value does not come out
pub fn aes_unwrap(kek: &[u8], wrapped: &[u8]) -> Option<Vec<u8>> {
    if wrapped.len() < 24 || wrapped.len() % 8 != 0 {
        return None;
    }
    let aes = Aes::new(kek)?;
    let n = wrapped.len() / 8 - 1;
    let mut a = [0u8; 8];
    a.copy_from_slice(&wrapped[..8]);
    let mut r: Vec<[u8; 8]> = wrapped[8..].chunks_exact(8).map(|c| [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).collect();
    for j in (0..6u64).rev() {
        for i in (0..n).rev() {
            let t = j * n as u64 + i as u64 + 1;
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            for (x, y) in b[..8].iter_mut().zip(t.to_be_bytes()) {
                *x ^= y;
            }
            b[8..].copy_from_slice(&r[i]);
            aes.decrypt(&mut b);
            a.copy_from_slice(&b[..8]);
            r[i].copy_from_slice(&b[8..]);
        }
    }
    if !crate::ct_eq(&a, &IV) {
        return None;
    }
    Some(r.concat())
}
