// blake2b -- BLAKE2b (RFC 7693): 1..64 byte digests, optional key of up to 64 bytes;
// Argon2 builds on it (with its long-output variant `long`)

use alloc::vec::Vec;

use crate::le64;

const IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
    0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];

const SIGMA: [[usize; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

#[inline(always)]
fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn compress(h: &mut [u64; 8], block: &[u8; 128], t: u128, last: bool) {
    let mut m = [0u64; 16];
    for (i, w) in m.iter_mut().enumerate() {
        *w = le64(&block[i * 8..]);
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= t as u64;
    v[13] ^= (t >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    for s in &SIGMA {
        g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// BLAKE2b
#[derive(Clone)]
pub struct Blake2b {
    h: [u64; 8],
    buf: [u8; 128],
    n: usize,
    t: u128,
    out_len: usize,
}

impl Blake2b {
    /// a hasher for `out_len` (1..=64) byte digests, keyed when `key` is not empty (<= 64 bytes)
    pub fn new(out_len: usize, key: &[u8]) -> Blake2b {
        assert!((1..=64).contains(&out_len) && key.len() <= 64, "blake2b: bad parameters");
        let mut h = IV;
        h[0] ^= 0x0101_0000 ^ ((key.len() as u64) << 8) ^ out_len as u64;
        let mut s = Blake2b { h, buf: [0; 128], n: 0, t: 0, out_len };
        if !key.is_empty() {
            let mut block = [0u8; 128];
            block[..key.len()].copy_from_slice(key);
            s.update(&block);
            crate::zeroize(&mut block);
        }
        s
    }

    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            // the last block is compressed in finalize (with the final flag)
            if self.n == 128 {
                self.t = self.t.wrapping_add(128);
                let b = self.buf;
                compress(&mut self.h, &b, self.t, false);
                self.n = 0;
            }
            let take = (128 - self.n).min(data.len());
            self.buf[self.n..self.n + take].copy_from_slice(&data[..take]);
            self.n += take;
            data = &data[take..];
        }
    }

    /// the digest into out[..out_len]
    pub fn finalize_into(mut self, out: &mut [u8]) {
        self.t = self.t.wrapping_add(self.n as u128);
        for b in &mut self.buf[self.n..] {
            *b = 0;
        }
        let b = self.buf;
        compress(&mut self.h, &b, self.t, true);
        let mut full = [0u8; 64];
        for (o, w) in full.chunks_exact_mut(8).zip(self.h) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out[..self.out_len].copy_from_slice(&full[..self.out_len]);
    }

    pub fn finalize(self) -> Vec<u8> {
        let mut out = alloc::vec![0u8; self.out_len];
        self.finalize_into(&mut out);
        out
    }
}

/// BLAKE2b of the concatenated parts, `out.len()` (1..=64) bytes
pub fn blake2b(out: &mut [u8], key: &[u8], parts: &[&[u8]]) {
    let mut h = Blake2b::new(out.len(), key);
    for p in parts {
        h.update(p);
    }
    h.finalize_into(out);
}

/// Argon2's variable-length hash H' (RFC 9106 3.3) of the concatenated parts
pub fn long(out: &mut [u8], parts: &[&[u8]]) {
    let t = (out.len() as u32).to_le_bytes();
    if out.len() <= 64 {
        let mut h = Blake2b::new(out.len(), &[]);
        h.update(&t);
        for p in parts {
            h.update(p);
        }
        h.finalize_into(out);
        return;
    }
    let mut v = [0u8; 64];
    let mut h = Blake2b::new(64, &[]);
    h.update(&t);
    for p in parts {
        h.update(p);
    }
    h.finalize_into(&mut v);
    let r = out.len().div_ceil(32) - 2;
    out[..32].copy_from_slice(&v[..32]);
    for i in 1..r {
        let prev = v;
        blake2b(&mut v, &[], &[&prev]);
        out[i * 32..i * 32 + 32].copy_from_slice(&v[..32]);
    }
    let rest = out.len() - 32 * r;
    let prev = v;
    blake2b(&mut out[32 * r..], &[], &[&prev]);
    debug_assert_eq!(out.len() - 32 * r, rest);
}
