// field -- arithmetic in GF(2^255 - 19) for X25519 and Ed25519: five 51-bit limbs, u128
// products, no branches or table lookups on values (cswap / cmov with masks)

use crate::le64;

const M51: u64 = (1 << 51) - 1;

/// a field element; limbs below 2^52 after every operation
#[derive(Clone, Copy, Debug)]
pub struct Fe(pub [u64; 5]);

impl Fe {
    pub const ZERO: Fe = Fe([0; 5]);
    pub const ONE: Fe = Fe([1, 0, 0, 0, 0]);

    pub const fn small(v: u64) -> Fe {
        Fe([v, 0, 0, 0, 0])
    }

    /// the element of 32 little-endian bytes (bit 255 ignored; values >= p are accepted)
    pub fn from_bytes(b: &[u8; 32]) -> Fe {
        Fe([
            le64(&b[0..]) & M51,
            (le64(&b[6..]) >> 3) & M51,
            (le64(&b[12..]) >> 6) & M51,
            (le64(&b[19..]) >> 1) & M51,
            (le64(&b[24..]) >> 12) & M51,
        ])
    }

    /// canonical little-endian encoding (fully reduced)
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut t = self.carry().carry().0;
        // q = 1 when t >= p
        let mut q = (t[0] + 19) >> 51;
        q = (t[1] + q) >> 51;
        q = (t[2] + q) >> 51;
        q = (t[3] + q) >> 51;
        q = (t[4] + q) >> 51;
        t[0] += 19 * q;
        t[1] += t[0] >> 51;
        t[0] &= M51;
        t[2] += t[1] >> 51;
        t[1] &= M51;
        t[3] += t[2] >> 51;
        t[2] &= M51;
        t[4] += t[3] >> 51;
        t[3] &= M51;
        t[4] &= M51;
        let w = [
            t[0] | (t[1] << 51),
            (t[1] >> 13) | (t[2] << 38),
            (t[2] >> 26) | (t[3] << 25),
            (t[3] >> 39) | (t[4] << 12),
        ];
        let mut out = [0u8; 32];
        for (o, x) in out.chunks_exact_mut(8).zip(w) {
            o.copy_from_slice(&x.to_le_bytes());
        }
        out
    }

    /// propagate carries (limbs below 2^51 + 2^13 afterwards)
    #[inline(always)]
    fn carry(&self) -> Fe {
        let mut t = self.0;
        let c = t[0] >> 51;
        t[0] &= M51;
        t[1] += c;
        let c = t[1] >> 51;
        t[1] &= M51;
        t[2] += c;
        let c = t[2] >> 51;
        t[2] &= M51;
        t[3] += c;
        let c = t[3] >> 51;
        t[3] &= M51;
        t[4] += c;
        let c = t[4] >> 51;
        t[4] &= M51;
        t[0] += c * 19;
        Fe(t)
    }

    #[inline(always)]
    pub fn add(&self, b: &Fe) -> Fe {
        let (a, b) = (&self.0, &b.0);
        Fe([a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3], a[4] + b[4]]).carry()
    }

    /// a - b (computed as a + 16p - b)
    #[inline(always)]
    pub fn sub(&self, b: &Fe) -> Fe {
        let (a, b) = (&self.0, &b.0);
        Fe([
            (a[0] + 0x7f_ffff_ffff_fed0) - b[0],
            (a[1] + 0x7f_ffff_ffff_fff0) - b[1],
            (a[2] + 0x7f_ffff_ffff_fff0) - b[2],
            (a[3] + 0x7f_ffff_ffff_fff0) - b[3],
            (a[4] + 0x7f_ffff_ffff_fff0) - b[4],
        ])
        .carry()
    }

    pub fn neg(&self) -> Fe {
        Fe::ZERO.sub(self)
    }

    #[inline(always)]
    pub fn mul(&self, b: &Fe) -> Fe {
        let a = &self.0;
        let b = &b.0;
        let m = |x: u64, y: u64| x as u128 * y as u128;
        let (b1, b2, b3, b4) = (b[1] * 19, b[2] * 19, b[3] * 19, b[4] * 19);
        let r0 = m(a[0], b[0]) + m(a[1], b4) + m(a[2], b3) + m(a[3], b2) + m(a[4], b1);
        let mut r1 = m(a[0], b[1]) + m(a[1], b[0]) + m(a[2], b4) + m(a[3], b3) + m(a[4], b2);
        let mut r2 = m(a[0], b[2]) + m(a[1], b[1]) + m(a[2], b[0]) + m(a[3], b4) + m(a[4], b3);
        let mut r3 = m(a[0], b[3]) + m(a[1], b[2]) + m(a[2], b[1]) + m(a[3], b[0]) + m(a[4], b4);
        let mut r4 = m(a[0], b[4]) + m(a[1], b[3]) + m(a[2], b[2]) + m(a[3], b[1]) + m(a[4], b[0]);
        r1 += r0 >> 51;
        let t0 = r0 as u64 & M51;
        r2 += r1 >> 51;
        let t1 = r1 as u64 & M51;
        r3 += r2 >> 51;
        let t2 = r2 as u64 & M51;
        r4 += r3 >> 51;
        let t3 = r3 as u64 & M51;
        let c = (r4 >> 51) as u64;
        let t4 = r4 as u64 & M51;
        let t0 = t0 + c * 19;
        let t1 = t1 + (t0 >> 51);
        Fe([t0 & M51, t1, t2, t3, t4])
    }

    #[inline(always)]
    pub fn square(&self) -> Fe {
        self.mul(self)
    }

    /// self^(2^n)
    pub fn square_n(&self, n: u32) -> Fe {
        let mut x = *self;
        for _ in 0..n {
            x = x.square();
        }
        x
    }

    /// multiply by a small constant (< 2^20)
    pub fn mul_small(&self, k: u64) -> Fe {
        self.mul(&Fe::small(k))
    }

    /// (self^(2^250 - 1), self^11)
    fn pow_2_250_1(&self) -> (Fe, Fe) {
        let z = self;
        let z2 = z.square();
        let z8 = z2.square_n(2);
        let z9 = z.mul(&z8);
        let z11 = z2.mul(&z9);
        let z22 = z11.square();
        let z_5_0 = z9.mul(&z22);
        let z_10_0 = z_5_0.square_n(5).mul(&z_5_0);
        let z_20_0 = z_10_0.square_n(10).mul(&z_10_0);
        let z_40_0 = z_20_0.square_n(20).mul(&z_20_0);
        let z_50_0 = z_40_0.square_n(10).mul(&z_10_0);
        let z_100_0 = z_50_0.square_n(50).mul(&z_50_0);
        let z_200_0 = z_100_0.square_n(100).mul(&z_100_0);
        let z_250_0 = z_200_0.square_n(50).mul(&z_50_0);
        (z_250_0, z11)
    }

    /// 1 / self (0 for 0)
    pub fn invert(&self) -> Fe {
        let (t, z11) = self.pow_2_250_1();
        t.square_n(5).mul(&z11)
    }

    /// self^((p - 5) / 8)
    pub fn pow_p58(&self) -> Fe {
        let (t, _) = self.pow_2_250_1();
        t.square_n(2).mul(self)
    }

    pub fn is_zero(&self) -> bool {
        let b = self.to_bytes();
        crate::ct_eq(&b, &[0u8; 32])
    }

    /// the "sign": the low bit of the canonical encoding
    pub fn is_negative(&self) -> bool {
        self.to_bytes()[0] & 1 == 1
    }

    pub fn ct_equal(&self, b: &Fe) -> bool {
        crate::ct_eq(&self.to_bytes(), &b.to_bytes())
    }

    /// swap a and b when swap == 1 (0 / 1, constant time)
    #[inline(always)]
    pub fn cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
        let mask = 0u64.wrapping_sub(swap);
        for i in 0..5 {
            let t = mask & (a.0[i] ^ b.0[i]);
            a.0[i] ^= t;
            b.0[i] ^= t;
        }
    }

    /// b when choice == 1, else self (constant time)
    #[inline(always)]
    pub fn select(&self, b: &Fe, choice: u64) -> Fe {
        let mask = 0u64.wrapping_sub(choice);
        let mut r = self.0;
        for (i, x) in r.iter_mut().enumerate() {
            *x ^= mask & (*x ^ b.0[i]);
        }
        Fe(r)
    }
}
