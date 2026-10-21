// gcm -- AES-GCM (NIST SP 800-38D) with 12-byte nonces and 16-byte tags, in constant time
//
//   AesGcm::new(key)                          16 (AES-128-GCM) or 32 bytes (AES-256-GCM)
//   seal_in_place(nonce, aad, data) -> tag    open_in_place(nonce, aad, data, tag) -> bool
//
// GHASH runs on the bit-reversed form of the blocks, where multiplication in GF(2^128) is a
// plain carry-less product modulo x^128 + x^7 + x^2 + x + 1.  The carry-less products come from
// integer multiplications of operands with three-bit holes (32-bit halves: at most eight
// partial products meet in one position, so no carry reaches the next used bit).

use crate::aes::Aes;

/// carry-less 32 x 32 -> 64 bits
#[inline(always)]
fn bmul32(x: u32, y: u32) -> u64 {
    let (x, y) = (x as u64, y as u64);
    let m = [0x1111_1111u64, 0x2222_2222, 0x4444_4444, 0x8888_8888];
    let xs = [x & m[0], x & m[1], x & m[2], x & m[3]];
    let ys = [y & m[0], y & m[1], y & m[2], y & m[3]];
    let p = |i: usize, j: usize| xs[i].wrapping_mul(ys[j]);
    let z0 = p(0, 0) ^ p(1, 3) ^ p(2, 2) ^ p(3, 1);
    let z1 = p(0, 1) ^ p(1, 0) ^ p(2, 3) ^ p(3, 2);
    let z2 = p(0, 2) ^ p(1, 1) ^ p(2, 0) ^ p(3, 3);
    let z3 = p(0, 3) ^ p(1, 2) ^ p(2, 1) ^ p(3, 0);
    (z0 & 0x1111_1111_1111_1111) | (z1 & 0x2222_2222_2222_2222) | (z2 & 0x4444_4444_4444_4444) | (z3 & 0x8888_8888_8888_8888)
}

/// carry-less 64 x 64 -> 128 bits (Karatsuba)
#[inline(always)]
fn clmul64(x: u64, y: u64) -> u128 {
    let (x0, x1, y0, y1) = (x as u32, (x >> 32) as u32, y as u32, (y >> 32) as u32);
    let lo = bmul32(x0, y0) as u128;
    let hi = bmul32(x1, y1) as u128;
    let mid = bmul32(x0 ^ x1, y0 ^ y1) as u128 ^ lo ^ hi;
    lo ^ (mid << 32) ^ (hi << 64)
}

/// a * b in GF(2^128), both in the bit-reversed form
fn gf_mul(a: u128, b: u128) -> u128 {
    let (a0, a1, b0, b1) = (a as u64, (a >> 64) as u64, b as u64, (b >> 64) as u64);
    let lo = clmul64(a0, b0);
    let hi = clmul64(a1, b1);
    let mid = clmul64(a0 ^ a1, b0 ^ b1) ^ lo ^ hi;
    let low = lo ^ (mid << 64);
    let high = hi ^ (mid >> 64);
    // x^128 = x^7 + x^2 + x + 1, twice (the first fold overflows by up to seven bits)
    let over = (high >> 127) ^ (high >> 126) ^ (high >> 121);
    let r = low ^ high ^ (high << 1) ^ (high << 2) ^ (high << 7);
    r ^ over ^ (over << 1) ^ (over << 2) ^ (over << 7)
}

/// reverse the bits inside each byte
#[inline(always)]
fn rev_bytes_bits(mut x: u128) -> u128 {
    const M1: u128 = 0x5555_5555_5555_5555_5555_5555_5555_5555;
    const M2: u128 = 0x3333_3333_3333_3333_3333_3333_3333_3333;
    const M4: u128 = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
    x = ((x >> 1) & M1) | ((x & M1) << 1);
    x = ((x >> 2) & M2) | ((x & M2) << 2);
    ((x >> 4) & M4) | ((x & M4) << 4)
}

/// a block as a GF(2^128) element (bit i = coefficient of x^i)
#[inline(always)]
fn elem(b: &[u8; 16]) -> u128 {
    rev_bytes_bits(u128::from_le_bytes(*b))
}

fn block_of(x: u128) -> [u8; 16] {
    rev_bytes_bits(x).to_le_bytes()
}

struct Ghash {
    h: u128,
    y: u128,
}

impl Ghash {
    /// absorb data, zero-padded to whole blocks
    fn update(&mut self, data: &[u8]) {
        for c in data.chunks(16) {
            let mut b = [0u8; 16];
            b[..c.len()].copy_from_slice(c);
            self.y = gf_mul(self.y ^ elem(&b), self.h);
        }
    }
}

pub struct AesGcm {
    aes: Aes,
    h: u128,
}

impl AesGcm {
    pub fn new(key: &[u8]) -> Option<AesGcm> {
        let aes = Aes::new(key)?;
        let mut h = [0u8; 16];
        aes.encrypt(&mut h);
        Some(AesGcm { aes, h: elem(&h) })
    }

    /// XOR the key stream of counter blocks 2, 3, ... into data
    fn ctr(&self, nonce: &[u8; 12], data: &mut [u8]) {
        let mut ctr = 2u32;
        for chunk in data.chunks_mut(64) {
            let mut blocks = [[0u8; 16]; 4];
            for b in blocks.iter_mut() {
                b[..12].copy_from_slice(nonce);
                b[12..].copy_from_slice(&ctr.to_be_bytes());
                ctr = ctr.wrapping_add(1);
            }
            self.aes.encrypt4(&mut blocks);
            for (i, x) in chunk.iter_mut().enumerate() {
                *x ^= blocks[i / 16][i % 16];
            }
        }
    }

    fn tag(&self, nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
        let mut g = Ghash { h: self.h, y: 0 };
        g.update(aad);
        g.update(ct);
        let mut lens = [0u8; 16];
        lens[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
        lens[8..].copy_from_slice(&((ct.len() as u64) * 8).to_be_bytes());
        g.update(&lens);
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;
        self.aes.encrypt(&mut j0);
        let mut t = block_of(g.y);
        for (a, b) in t.iter_mut().zip(j0) {
            *a ^= b;
        }
        t
    }

    /// encrypt data in place -> the tag
    pub fn seal_in_place(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8]) -> [u8; 16] {
        self.ctr(nonce, data);
        self.tag(nonce, aad, data)
    }

    /// check the tag, then decrypt in place (false: data untouched)
    pub fn open_in_place(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8], tag: &[u8; 16]) -> bool {
        if !crate::ct_eq(&self.tag(nonce, aad, data), tag) {
            return false;
        }
        self.ctr(nonce, data);
        true
    }
}
