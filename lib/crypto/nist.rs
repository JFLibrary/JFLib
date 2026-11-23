// nist -- the NIST prime curves P-256 and P-384 (y^2 = x^3 - 3x + b, FIPS 186-5, SP 800-186) in
// constant time, for any number of 64-bit limbs: what p384 (Diffie-Hellman) and ecdsa's signing
// compute with.  p256.rs keeps its own four-limb code (Diffie-Hellman, EAP-PWD's group).
//
// Field elements and scalars in Montgomery form, multiplied by CIOS with the modulus's
// -m^-1 mod 2^64; additions and subtractions end in masked corrections, never a branch on the
// values.  Points in projective coordinates with the complete addition formulas of Renes,
// Costello and Batina (2016, algorithm 4, a = -3), so doubling is the same formula and there
// are no exceptional cases.  Scalar multiplication by four-bit windows, every window reading the
// whole table.  Inversions are powers to m - 2: a fixed sequence for the public exponent.
// Montgomery constants are worked out at compile time (const fn) from the modulus alone.

/// a modulus of L limbs (odd, its top limb not zero) with its Montgomery constants
pub(crate) struct Modulus<const L: usize> {
    pub(crate) m: [u64; L],
    /// -m^-1 mod 2^64
    m0: u64,
    /// R^2 mod m (R = 2^(64 L))
    rr: [u64; L],
}

#[inline(always)]
fn adc(a: u64, b: u64, c: u64) -> (u64, u64) {
    let t = a as u128 + b as u128 + c as u128;
    (t as u64, (t >> 64) as u64)
}

#[inline(always)]
fn sbb(a: u64, b: u64, borrow: u64) -> (u64, u64) {
    let t = (a as u128).wrapping_sub(b as u128 + borrow as u128);
    (t as u64, (t >> 127) as u64)
}

/// a + b c + carry -> (low, high)
#[inline(always)]
fn mac(a: u64, b: u64, c: u64, carry: u64) -> (u64, u64) {
    let t = a as u128 + b as u128 * c as u128 + carry as u128;
    (t as u64, (t >> 64) as u64)
}

/// keep a where the mask is all ones, b where it is zero
#[inline(always)]
fn select<const L: usize>(keep_a: u64, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
    let mut r = [0u64; L];
    for i in 0..L {
        r[i] = (a[i] & keep_a) | (b[i] & !keep_a);
    }
    r
}

impl<const L: usize> Modulus<L> {
    pub(crate) const fn new(m: [u64; L]) -> Modulus<L> {
        // -m^-1 mod 2^64 by Newton's iteration (each step doubles the correct bits)
        let mut inv = 1u64;
        let mut i = 0;
        while i < 6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
            i += 1;
        }
        // R^2 mod m: 1 doubled 2 * 64 L times, m subtracted whenever the value reaches it
        let mut r = [0u64; L];
        r[0] = 1;
        let mut step = 0;
        while step < 128 * L {
            let top = r[L - 1] >> 63;
            let mut j = L - 1;
            while j > 0 {
                r[j] = r[j] << 1 | r[j - 1] >> 63;
                j -= 1;
            }
            r[0] <<= 1;
            // r >= m (with the bit shifted out on top): subtract
            let mut ge = top == 1;
            if !ge {
                ge = true;
                let mut k = L;
                while k > 0 {
                    k -= 1;
                    if r[k] != m[k] {
                        ge = r[k] > m[k];
                        break;
                    }
                }
            }
            if ge {
                let mut br = 0u64;
                let mut k = 0;
                while k < L {
                    let t = (r[k] as u128).wrapping_sub(m[k] as u128 + br as u128);
                    r[k] = t as u64;
                    br = (t >> 127) as u64;
                    k += 1;
                }
            }
            step += 1;
        }
        Modulus { m, m0: inv.wrapping_neg(), rr: r }
    }

    /// a b R^-1 mod m (a, b < m)
    pub(crate) fn mul(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let m = &self.m;
        let mut t = [0u64; L];
        let mut hi = 0u64;
        for &bi in b.iter() {
            let mut c = 0;
            for j in 0..L {
                (t[j], c) = mac(t[j], a[j], bi, c);
            }
            let (s, hi2) = adc(hi, c, 0);
            hi = s;
            let q = t[0].wrapping_mul(self.m0);
            let (_, mut c) = mac(t[0], q, m[0], 0);
            for j in 1..L {
                (t[j - 1], c) = mac(t[j], q, m[j], c);
            }
            let (s, c2) = adc(hi, c, 0);
            t[L - 1] = s;
            hi = hi2 + c2;
        }
        // t + hi 2^(64 L) < 2 m: subtract m unless that borrows
        let mut s = [0u64; L];
        let mut br = 0;
        for i in 0..L {
            (s[i], br) = sbb(t[i], m[i], br);
        }
        let (_, br) = sbb(hi, 0, br);
        select(crate::ct_mask(0u64.wrapping_sub(br)), &t, &s)
    }

    /// (a + b) mod m (a, b < m)
    pub(crate) fn add(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let mut r = [0u64; L];
        let mut c = 0;
        for i in 0..L {
            (r[i], c) = adc(a[i], b[i], c);
        }
        let mut s = [0u64; L];
        let mut br = 0;
        for i in 0..L {
            (s[i], br) = sbb(r[i], self.m[i], br);
        }
        let (_, br) = sbb(c, 0, br);
        select(crate::ct_mask(0u64.wrapping_sub(br)), &r, &s)
    }

    /// (a - b) mod m (a, b < m)
    pub(crate) fn sub(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let mut r = [0u64; L];
        let mut br = 0;
        for i in 0..L {
            (r[i], br) = sbb(a[i], b[i], br);
        }
        let mask = crate::ct_mask(0u64.wrapping_sub(br));
        let mut c = 0;
        for i in 0..L {
            (r[i], c) = adc(r[i], self.m[i] & mask, c);
        }
        r
    }

    pub(crate) fn to_mont(&self, a: &[u64; L]) -> [u64; L] {
        self.mul(a, &self.rr)
    }

    pub(crate) fn from_mont(&self, a: &[u64; L]) -> [u64; L] {
        let mut one = [0u64; L];
        one[0] = 1;
        self.mul(a, &one)
    }

    /// 1 in Montgomery form
    pub(crate) fn one(&self) -> [u64; L] {
        let mut one = [0u64; L];
        one[0] = 1;
        self.to_mont(&one)
    }

    /// a^-1 for a prime m (Montgomery form in and out; 0 gives 0): a^(m - 2), squarings and
    /// multiplications in an order that depends on m only
    pub(crate) fn inv(&self, a: &[u64; L]) -> [u64; L] {
        let mut e = self.m;
        e[0] -= 2; // m is odd and above 2: no borrow
        let mut r = self.one();
        for i in (0..64 * L).rev() {
            r = self.mul(&r, &r);
            // the exponent is public: a branch on its bits shows nothing
            if (e[i / 64] >> (i % 64)) & 1 == 1 {
                r = self.mul(&r, a);
            }
        }
        r
    }

    /// a < m, for public values only
    pub(crate) fn below(&self, a: &[u64; L]) -> bool {
        for i in (0..L).rev() {
            if a[i] != self.m[i] {
                return a[i] < self.m[i];
            }
        }
        false
    }

    /// all ones when a lies in [1, m - 1], else 0; no branch on a
    pub(crate) fn in_range_mask(&self, a: &[u64; L]) -> u64 {
        let mut br = 0;
        for i in 0..L {
            br = sbb(a[i], self.m[i], br).1;
        }
        let any = a.iter().fold(0u64, |x, &w| x | w);
        let nonzero = (any | any.wrapping_neg()) >> 63;
        crate::ct_mask(0u64.wrapping_sub(br & nonzero))
    }

    /// a mod m for a < 2 m (one masked subtraction)
    pub(crate) fn reduce_once(&self, a: &[u64; L]) -> [u64; L] {
        let mut s = [0u64; L];
        let mut br = 0;
        for i in 0..L {
            (s[i], br) = sbb(a[i], self.m[i], br);
        }
        select(crate::ct_mask(0u64.wrapping_sub(br)), a, &s)
    }
}

/// big-endian bytes (exactly 8 L) -> limbs
pub(crate) fn from_be<const L: usize>(b: &[u8]) -> [u64; L] {
    let mut r = [0u64; L];
    for (i, w) in r.iter_mut().enumerate() {
        let at = 8 * (L - 1 - i);
        *w = u64::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3], b[at + 4], b[at + 5], b[at + 6], b[at + 7]]);
    }
    r
}

/// limbs -> 8 L big-endian bytes into out
pub(crate) fn to_be<const L: usize>(a: &[u64; L], out: &mut [u8]) {
    for (i, w) in a.iter().enumerate() {
        let at = 8 * (L - 1 - i);
        out[at..at + 8].copy_from_slice(&w.to_be_bytes());
    }
}

/// a curve y^2 = x^3 - 3x + b over the field of p, its group order n and base point G
pub(crate) struct Curve<const L: usize> {
    pub(crate) p: Modulus<L>,
    pub(crate) n: Modulus<L>,
    b: [u64; L],
    gx: [u64; L],
    gy: [u64; L],
}

pub(crate) static P256: Curve<4> = Curve {
    p: Modulus::new([0xFFFF_FFFF_FFFF_FFFF, 0x0000_0000_FFFF_FFFF, 0x0000_0000_0000_0000, 0xFFFF_FFFF_0000_0001]),
    n: Modulus::new([0xF3B9_CAC2_FC63_2551, 0xBCE6_FAAD_A717_9E84, 0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_0000_0000]),
    b: [0x3BCE_3C3E_27D2_604B, 0x651D_06B0_CC53_B0F6, 0xB3EB_BD55_7698_86BC, 0x5AC6_35D8_AA3A_93E7],
    gx: [0xF4A1_3945_D898_C296, 0x7703_7D81_2DEB_33A0, 0xF8BC_E6E5_63A4_40F2, 0x6B17_D1F2_E12C_4247],
    gy: [0xCBB6_4068_37BF_51F5, 0x2BCE_3357_6B31_5ECE, 0x8EE7_EB4A_7C0F_9E16, 0x4FE3_42E2_FE1A_7F9B],
};

pub(crate) static P384: Curve<6> = Curve {
    p: Modulus::new([
        0x0000_0000_FFFF_FFFF, 0xFFFF_FFFF_0000_0000, 0xFFFF_FFFF_FFFF_FFFE,
        0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFF,
    ]),
    n: Modulus::new([
        0xECEC_196A_CCC5_2973, 0x581A_0DB2_48B0_A77A, 0xC763_4D81_F437_2DDF,
        0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFF,
    ]),
    b: [
        0x2A85_C8ED_D3EC_2AEF, 0xC656_398D_8A2E_D19D, 0x0314_088F_5013_875A,
        0x181D_9C6E_FE81_4112, 0x988E_056B_E3F8_2D19, 0xB331_2FA7_E23E_E7E4,
    ],
    gx: [
        0x3A54_5E38_7276_0AB7, 0x5502_F25D_BF55_296C, 0x59F7_41E0_8254_2A38,
        0x6E1D_3B62_8BA7_9B98, 0x8EB1_C71E_F320_AD74, 0xAA87_CA22_BE8B_0537,
    ],
    gy: [
        0x7A43_1D7C_90EA_0E5F, 0x0A60_B1CE_1D7E_819D, 0xE9DA_3113_B5F0_B8C0,
        0xF8F4_1DBD_289A_147C, 0x5D9E_98BF_9292_DC29, 0x3617_DE4A_9626_2C6F,
    ],
};

/// a projective point (X : Y : Z) in Montgomery form; (0 : 1 : 0) is the identity
#[derive(Clone, Copy)]
pub(crate) struct Pt<const L: usize> {
    x: [u64; L],
    y: [u64; L],
    z: [u64; L],
}

impl<const L: usize> Curve<L> {
    /// the size of a coordinate or scalar in bytes
    pub(crate) const BYTES: usize = 8 * L;

    fn identity(&self) -> Pt<L> {
        Pt { x: [0; L], y: self.p.one(), z: [0; L] }
    }

    pub(crate) fn generator(&self) -> Pt<L> {
        Pt { x: self.p.to_mont(&self.gx), y: self.p.to_mont(&self.gy), z: self.p.one() }
    }

    /// P + Q, complete (RCB 2016, algorithm 4); b in Montgomery form
    fn add(&self, b: &[u64; L], p: &Pt<L>, q: &Pt<L>) -> Pt<L> {
        let f = &self.p;
        let (x1, y1, z1, x2, y2, z2) = (&p.x, &p.y, &p.z, &q.x, &q.y, &q.z);
        let mut t0 = f.mul(x1, x2);
        let mut t1 = f.mul(y1, y2);
        let mut t2 = f.mul(z1, z2);
        let mut t3 = f.add(x1, y1);
        let mut t4 = f.add(x2, y2);
        t3 = f.mul(&t3, &t4);
        t4 = f.add(&t0, &t1);
        t3 = f.sub(&t3, &t4);
        t4 = f.add(y1, z1);
        let mut x3 = f.add(y2, z2);
        t4 = f.mul(&t4, &x3);
        x3 = f.add(&t1, &t2);
        t4 = f.sub(&t4, &x3);
        x3 = f.add(x1, z1);
        let mut y3 = f.add(x2, z2);
        x3 = f.mul(&x3, &y3);
        y3 = f.add(&t0, &t2);
        y3 = f.sub(&x3, &y3);
        let mut z3 = f.mul(b, &t2);
        x3 = f.sub(&y3, &z3);
        z3 = f.add(&x3, &x3);
        x3 = f.add(&x3, &z3);
        z3 = f.sub(&t1, &x3);
        x3 = f.add(&t1, &x3);
        y3 = f.mul(b, &y3);
        t1 = f.add(&t2, &t2);
        t2 = f.add(&t1, &t2);
        y3 = f.sub(&y3, &t2);
        y3 = f.sub(&y3, &t0);
        t1 = f.add(&y3, &y3);
        y3 = f.add(&t1, &y3);
        t1 = f.add(&t0, &t0);
        t0 = f.add(&t1, &t0);
        t0 = f.sub(&t0, &t2);
        t1 = f.mul(&t4, &y3);
        t2 = f.mul(&t0, &y3);
        y3 = f.mul(&x3, &z3);
        y3 = f.add(&y3, &t2);
        x3 = f.mul(&t3, &x3);
        x3 = f.sub(&x3, &t1);
        z3 = f.mul(&t4, &z3);
        t1 = f.mul(&t3, &t0);
        z3 = f.add(&z3, &t1);
        Pt { x: x3, y: y3, z: z3 }
    }

    /// k P for a big-endian scalar of BYTES bytes, constant time (four-bit windows, the table
    /// read in full for each)
    pub(crate) fn mul(&self, k: &[u8], p: &Pt<L>) -> Pt<L> {
        let b = self.p.to_mont(&self.b);
        let id = self.identity();
        let mut table = [id; 16];
        for i in 1..16 {
            table[i] = self.add(&b, &table[i - 1], p);
        }
        let mut acc = id;
        for &byte in &k[..Self::BYTES] {
            for half in [byte >> 4, byte & 15] {
                for _ in 0..4 {
                    acc = self.add(&b, &acc, &acc);
                }
                let mut sel = id;
                for (i, e) in table.iter().enumerate() {
                    let m = crate::ct_mask(0u64.wrapping_sub(((i as u8 ^ half) as u64).wrapping_sub(1) >> 63));
                    sel.x = select(m, &e.x, &sel.x);
                    sel.y = select(m, &e.y, &sel.y);
                    sel.z = select(m, &e.z, &sel.z);
                }
                acc = self.add(&b, &acc, &sel);
            }
        }
        acc
    }

    /// the affine coordinates in normal form, and all ones when the point is not the identity
    /// (zeros and 0 when it is); no branch on the coordinates
    pub(crate) fn affine(&self, p: &Pt<L>) -> (u64, [u64; L], [u64; L]) {
        let any = p.z.iter().fold(0u64, |a, &w| a | w);
        let finite = crate::ct_mask(0u64.wrapping_sub((any | any.wrapping_neg()) >> 63));
        let zi = self.p.inv(&p.z);
        let x = self.p.from_mont(&self.p.mul(&p.x, &zi));
        let y = self.p.from_mont(&self.p.mul(&p.y, &zi));
        (finite, select(finite, &x, &[0; L]), select(finite, &y, &[0; L]))
    }

    /// x^3 - 3x + b in Montgomery form (x too)
    fn rhs(&self, mx: &[u64; L]) -> [u64; L] {
        let f = &self.p;
        let x3 = f.mul(&f.mul(mx, mx), mx);
        f.add(&f.sub(&x3, &f.add(&f.add(mx, mx), mx)), &f.to_mont(&self.b))
    }

    /// an uncompressed point from outside (04 || x || y), checked: coordinates below p and on
    /// the curve (public data, so the checks may branch)
    pub(crate) fn point(&self, sec1: &[u8]) -> Option<Pt<L>> {
        if sec1.len() != 1 + 2 * Self::BYTES || sec1[0] != 4 {
            return None;
        }
        let x = from_be::<L>(&sec1[1..1 + Self::BYTES]);
        let y = from_be::<L>(&sec1[1 + Self::BYTES..]);
        if !self.p.below(&x) || !self.p.below(&y) {
            return None;
        }
        let (mx, my) = (self.p.to_mont(&x), self.p.to_mont(&y));
        if self.p.mul(&my, &my) != self.rhs(&mx) {
            return None;
        }
        Some(Pt { x: mx, y: my, z: self.p.one() })
    }

    /// k G as 04 || x || y (k in [1, n - 1], which keeps it off the identity)
    pub(crate) fn public_key(&self, k: &[u8]) -> alloc::vec::Vec<u8> {
        let (_, x, y) = self.affine(&self.mul(k, &self.generator()));
        let mut out = alloc::vec![0u8; 1 + 2 * Self::BYTES];
        out[0] = 4;
        to_be(&x, &mut out[1..1 + Self::BYTES]);
        to_be(&y, &mut out[1 + Self::BYTES..]);
        out
    }
}
