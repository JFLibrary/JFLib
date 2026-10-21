// poly1305 -- the Poly1305 one-time authenticator (RFC 8439), 44/44/42-bit limbs

use crate::le64;

const M44: u64 = 0xfff_ffff_ffff;
const M42: u64 = 0x3ff_ffff_ffff;

/// a Poly1305 state for one message under a one-time key
#[derive(Clone)]
pub struct Poly1305 {
    r: [u64; 3],
    s: [u64; 2],
    h: [u64; 3],
    pad: [u64; 2],
    buf: [u8; 16],
    n: usize,
}

impl Poly1305 {
    pub fn new(key: &[u8; 32]) -> Poly1305 {
        let t0 = le64(&key[0..]);
        let t1 = le64(&key[8..]);
        let r0 = t0 & 0xffc_0fff_ffff;
        let r1 = ((t0 >> 44) | (t1 << 20)) & 0xfff_ffc0_ffff;
        let r2 = (t1 >> 24) & 0x00f_ffff_fc0f;
        Poly1305 {
            r: [r0, r1, r2],
            s: [r1 * 20, r2 * 20],
            h: [0; 3],
            pad: [le64(&key[16..]), le64(&key[24..])],
            buf: [0; 16],
            n: 0,
        }
    }

    fn blocks(&mut self, m: &[u8], hibit: u64) {
        let [r0, r1, r2] = self.r;
        let [s1, s2] = self.s;
        let [mut h0, mut h1, mut h2] = self.h;
        for b in m.chunks_exact(16) {
            let t0 = le64(b);
            let t1 = le64(&b[8..]);
            h0 += t0 & M44;
            h1 += ((t0 >> 44) | (t1 << 20)) & M44;
            h2 += ((t1 >> 24) & M42) | hibit;
            let d0 = h0 as u128 * r0 as u128 + h1 as u128 * s2 as u128 + h2 as u128 * s1 as u128;
            let mut d1 = h0 as u128 * r1 as u128 + h1 as u128 * r0 as u128 + h2 as u128 * s2 as u128;
            let mut d2 = h0 as u128 * r2 as u128 + h1 as u128 * r1 as u128 + h2 as u128 * r0 as u128;
            let mut c = (d0 >> 44) as u64;
            h0 = d0 as u64 & M44;
            d1 += c as u128;
            c = (d1 >> 44) as u64;
            h1 = d1 as u64 & M44;
            d2 += c as u128;
            c = (d2 >> 42) as u64;
            h2 = d2 as u64 & M42;
            h0 += c * 5;
            c = h0 >> 44;
            h0 &= M44;
            h1 += c;
        }
        self.h = [h0, h1, h2];
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if self.n > 0 {
            let take = (16 - self.n).min(data.len());
            self.buf[self.n..self.n + take].copy_from_slice(&data[..take]);
            self.n += take;
            data = &data[take..];
            if self.n < 16 {
                return;
            }
            let b = self.buf;
            self.blocks(&b, 1 << 40);
            self.n = 0;
        }
        let full = data.len() & !15;
        self.blocks(&data[..full], 1 << 40);
        let rest = &data[full..];
        self.buf[..rest.len()].copy_from_slice(rest);
        self.n = rest.len();
    }

    /// feed zero bytes up to the next multiple of 16 (the AEAD padding)
    pub fn pad16(&mut self) {
        if self.n > 0 {
            let z = [0u8; 16];
            let k = 16 - self.n;
            self.update(&z[..k]);
        }
    }

    pub fn finalize(mut self) -> [u8; 16] {
        if self.n > 0 {
            let mut b = [0u8; 16];
            b[..self.n].copy_from_slice(&self.buf[..self.n]);
            b[self.n] = 1;
            self.blocks(&b, 0);
        }
        let [mut h0, mut h1, mut h2] = self.h;
        // full carry
        let mut c = h1 >> 44;
        h1 &= M44;
        h2 += c;
        c = h2 >> 42;
        h2 &= M42;
        h0 += c * 5;
        c = h0 >> 44;
        h0 &= M44;
        h1 += c;
        c = h1 >> 44;
        h1 &= M44;
        h2 += c;
        c = h2 >> 42;
        h2 &= M42;
        h0 += c * 5;
        c = h0 >> 44;
        h0 &= M44;
        h1 += c;
        // h - p: take it when it does not underflow
        let mut g0 = h0 + 5;
        c = g0 >> 44;
        g0 &= M44;
        let mut g1 = h1 + c;
        c = g1 >> 44;
        g1 &= M44;
        let mut g2 = h2.wrapping_add(c).wrapping_sub(1 << 42);
        let mask = (g2 >> 63).wrapping_sub(1);
        g0 &= mask;
        g1 &= mask;
        g2 &= mask;
        let keep = !mask;
        h0 = (h0 & keep) | g0;
        h1 = (h1 & keep) | g1;
        h2 = (h2 & keep) | g2;
        // + s
        let [t0, t1] = self.pad;
        h0 += t0 & M44;
        c = h0 >> 44;
        h0 &= M44;
        h1 += (((t0 >> 44) | (t1 << 20)) & M44) + c;
        c = h1 >> 44;
        h1 &= M44;
        h2 += ((t1 >> 24) & M42) + c;
        h2 &= M42;
        let lo = h0 | (h1 << 44);
        let hi = (h1 >> 20) | (h2 << 24);
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&lo.to_le_bytes());
        out[8..].copy_from_slice(&hi.to_le_bytes());
        out
    }
}

/// the Poly1305 tag of a message
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let mut p = Poly1305::new(key);
    p.update(msg);
    p.finalize()
}
