// md5 -- MD5 (RFC 1321), streaming and one-shot: only for protocols that are defined with it
// (RADIUS: the authenticators and the hiding of User-Password of RFC 2865, HMAC-MD5 for the
// Message-Authenticator of RFC 3579); it is broken for collisions, so new designs take SHA-256
//
//   Md5::new() / update(data) / finalize() -> [u8; 16]
//   md5(data)            one byte string
//   md5_parts(parts)     the concatenation of several, without copying them together (what
//                        RADIUS hashes: code, identifier, length, authenticator, attributes,
//                        secret)

use crate::Hash;

const H0: [u32; 4] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476];

/// the shift of each of the 64 steps
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// T[i] of RFC 1321 3.4: the integer part of 2^32 * abs(sin(i + 1))
const T: [u32; 64] = [
    0xD76A_A478, 0xE8C7_B756, 0x2420_70DB, 0xC1BD_CEEE, 0xF57C_0FAF, 0x4787_C62A, 0xA830_4613, 0xFD46_9501,
    0x6980_98D8, 0x8B44_F7AF, 0xFFFF_5BB1, 0x895C_D7BE, 0x6B90_1122, 0xFD98_7193, 0xA679_438E, 0x49B4_0821,
    0xF61E_2562, 0xC040_B340, 0x265E_5A51, 0xE9B6_C7AA, 0xD62F_105D, 0x0244_1453, 0xD8A1_E681, 0xE7D3_FBC8,
    0x21E1_CDE6, 0xC337_07D6, 0xF4D5_0D87, 0x455A_14ED, 0xA9E3_E905, 0xFCEF_A3F8, 0x676F_02D9, 0x8D2A_4C8A,
    0xFFFA_3942, 0x8771_F681, 0x6D9D_6122, 0xFDE5_380C, 0xA4BE_EA44, 0x4BDE_CFA9, 0xF6BB_4B60, 0xBEBF_BC70,
    0x289B_7EC6, 0xEAA1_27FA, 0xD4EF_3085, 0x0488_1D05, 0xD9D4_D039, 0xE6DB_99E5, 0x1FA2_7CF8, 0xC4AC_5665,
    0xF429_2244, 0x432A_FF97, 0xAB94_23A7, 0xFC93_A039, 0x655B_59C3, 0x8F0C_CC92, 0xFFEF_F47D, 0x8584_5DD1,
    0x6FA8_7E4F, 0xFE2C_E6E0, 0xA301_4314, 0x4E08_11A1, 0xF753_7E82, 0xBD3A_F235, 0x2AD7_D2BB, 0xEB86_D391,
];

fn compress(h: &mut [u32; 4], block: &[u8]) {
    let mut m = [0u32; 16];
    for (i, c) in block.chunks_exact(4).enumerate() {
        m[i] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    let [mut a, mut b, mut c, mut d] = *h;
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | (!b & d), i),
            16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let x = a.wrapping_add(f).wrapping_add(T[i]).wrapping_add(m[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(x.rotate_left(S[i]));
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d]) {
        *x = x.wrapping_add(y);
    }
}

/// MD5
#[derive(Clone)]
pub struct Md5 {
    h: [u32; 4],
    buf: [u8; 64],
    n: usize,
    total: u64,
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    pub const fn new() -> Md5 {
        Md5 { h: H0, buf: [0; 64], n: 0, total: 0 }
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

    /// the digest: the padding of RFC 1321 3.1, the bit length little-endian (3.2)
    pub fn finalize(mut self) -> [u8; 16] {
        let bits = self.total.wrapping_mul(8);
        let mut pad = [0u8; 72];
        pad[0] = 0x80;
        let n = if self.n < 56 { 56 - self.n } else { 120 - self.n };
        pad[n..n + 8].copy_from_slice(&bits.to_le_bytes());
        let total = self.total;
        self.update(&pad[..n + 8]);
        self.total = total;
        let mut out = [0u8; 16];
        for (o, w) in out.chunks_exact_mut(4).zip(self.h) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

impl Hash for Md5 {
    const BLOCK: usize = 64;
    const OUT: usize = 16;
    fn new() -> Self {
        Md5::new()
    }
    fn update(&mut self, data: &[u8]) {
        Md5::update(self, data)
    }
    fn finish_into(self, out: &mut [u8]) {
        out[..16].copy_from_slice(&self.finalize());
    }
}

/// MD5 of a byte string
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(data);
    h.finalize()
}

/// MD5 of the concatenation of `parts`
pub fn md5_parts(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    h.finalize()
}
