// sha1 -- SHA-1 (FIPS 180-4), streaming and one-shot: only for protocols that are defined with
// it (WPA2's PBKDF2 and PRF, HMAC-SHA1 of older standards); it is broken for collisions, so
// new designs take SHA-256

use crate::Hash;

const H1: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

fn compress(h: &mut [u32; 5], block: &[u8]) {
    let mut w = [0u32; 80];
    for (i, c) in block.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (i, &wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
        *x = x.wrapping_add(y);
    }
}

/// SHA-1
#[derive(Clone)]
pub struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    n: usize,
    total: u64,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1 {
    pub const fn new() -> Sha1 {
        Sha1 { h: H1, buf: [0; 64], n: 0, total: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.n > 0 {
            let take = (64 - self.n).min(data.len());
            self.buf[self.n..self.n + take].copy_from_slice(&data[..take]);
            self.n += take;
            data = &data[take..];
            if self.n < 64 {
                return;
            }
            let b = self.buf;
            compress(&mut self.h, &b);
            self.n = 0;
        }
        let mut blocks = data.chunks_exact(64);
        for b in &mut blocks {
            compress(&mut self.h, b);
        }
        let rest = blocks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.n = rest.len();
    }

    pub fn finalize(mut self) -> [u8; 20] {
        let bits = self.total.wrapping_mul(8);
        let mut pad = [0u8; 72];
        pad[0] = 0x80;
        let n = if self.n < 56 { 56 - self.n } else { 120 - self.n };
        pad[n..n + 8].copy_from_slice(&bits.to_be_bytes());
        let total = self.total;
        self.update(&pad[..n + 8]);
        self.total = total;
        let mut out = [0u8; 20];
        for (o, w) in out.chunks_exact_mut(4).zip(self.h) {
            o.copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

impl Hash for Sha1 {
    const BLOCK: usize = 64;
    const OUT: usize = 20;
    fn new() -> Self {
        Sha1::new()
    }
    fn update(&mut self, data: &[u8]) {
        Sha1::update(self, data)
    }
    fn finish_into(self, out: &mut [u8]) {
        out[..20].copy_from_slice(&self.finalize());
    }
}

/// SHA-1 of a byte string
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(data);
    h.finalize()
}
