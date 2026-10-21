// chacha -- the ChaCha20 stream cipher (RFC 8439: 256-bit key, 96-bit nonce, 32-bit block
// counter), HChaCha20 and XChaCha20 (192-bit nonce; draft-irtf-cfrg-xchacha)

use crate::le32;

const SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

#[inline(always)]
fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn rounds(s: &mut [u32; 16]) {
    for _ in 0..10 {
        qr(s, 0, 4, 8, 12);
        qr(s, 1, 5, 9, 13);
        qr(s, 2, 6, 10, 14);
        qr(s, 3, 7, 11, 15);
        qr(s, 0, 5, 10, 15);
        qr(s, 1, 6, 11, 12);
        qr(s, 2, 7, 8, 13);
        qr(s, 3, 4, 9, 14);
    }
}

fn init(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u32; 16] {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&SIGMA);
    for i in 0..8 {
        s[4 + i] = le32(&key[i * 4..]);
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = le32(&nonce[i * 4..]);
    }
    s
}

/// one 64-byte keystream block
pub fn block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let init = init(key, counter, nonce);
    let mut s = init;
    rounds(&mut s);
    let mut out = [0u8; 64];
    for (i, o) in out.chunks_exact_mut(4).enumerate() {
        o.copy_from_slice(&s[i].wrapping_add(init[i]).to_le_bytes());
    }
    out
}

/// XOR the keystream (starting at block `counter`) into data
pub fn xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    let base = init(key, counter, nonce);
    for (n, chunk) in data.chunks_mut(64).enumerate() {
        let mut st = base;
        st[12] = counter.wrapping_add(n as u32);
        let init = st;
        rounds(&mut st);
        for (i, b) in chunk.iter_mut().enumerate() {
            let w = st[i / 4].wrapping_add(init[i / 4]);
            *b ^= (w >> (8 * (i % 4))) as u8;
        }
    }
}

/// HChaCha20: a subkey from a key and a 128-bit nonce
pub fn hchacha20(key: &[u8; 32], nonce: &[u8; 16]) -> [u8; 32] {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&SIGMA);
    for i in 0..8 {
        s[4 + i] = le32(&key[i * 4..]);
    }
    for i in 0..4 {
        s[12 + i] = le32(&nonce[i * 4..]);
    }
    rounds(&mut s);
    let mut out = [0u8; 32];
    for (i, w) in [s[0], s[1], s[2], s[3], s[12], s[13], s[14], s[15]].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// XChaCha20: the subkey and the ChaCha20 nonce for a 192-bit nonce
pub fn xchacha_setup(key: &[u8; 32], nonce: &[u8; 24]) -> ([u8; 32], [u8; 12]) {
    let mut n16 = [0u8; 16];
    n16.copy_from_slice(&nonce[..16]);
    let sub = hchacha20(key, &n16);
    let mut n12 = [0u8; 12];
    n12[4..].copy_from_slice(&nonce[16..]);
    (sub, n12)
}

/// XOR the XChaCha20 keystream (starting at block `counter`) into data
pub fn xchacha_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 24], data: &mut [u8]) {
    let (mut sub, n12) = xchacha_setup(key, nonce);
    xor(&sub, counter, &n12, data);
    crate::zeroize(&mut sub);
}
