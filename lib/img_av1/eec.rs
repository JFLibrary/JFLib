// eec -- AV1 encoder: range encoder (libaom's od_ec_enc, the inverse of the specification's
// symbol decoder) and symbol output / cost estimation of the tile encoder  [src/img/av1_eec.s]
//
//   Ec::new()                   ae_ecinit: a started range encoder
//   Ec::put(cdf, s, n)          ae_put: symbol s of a CDF array with n symbols (specification
//                               layout: n values, then the adaptation counter): encoded and the
//                               CDF adapted, or (est) its Q8 cost added to bits
//   Ec::lit(v, nbits)           ae_lit: nbits literal bits of v (MSB first): bools, or 256 each
//   cost(cdf, s) -> u32         ae_cost: Q8 bits of symbol s (ftx::COST)
//   Ec::done() -> Vec<u8>       ae_ecdone: the tile's bytes: the minimal flush (trailing one
//                               bit), carries propagated
//
// Model: tools/av1_enc_ref.py (ECEnc, adapt_cdf, EncCoder, EstCoder, cost).  low is kept in 64
// bits (no overflow; libaom's 32-bit window gives the same bytes).  The precarry buffer grows
// as a Vec (the assembly's ec_room doubling; out of memory aborts, R6 rule).

use alloc::vec::Vec;

use crate::ftx::COST;

const EC_PCAP0: usize = 32768; // initial precarry entries

/// the range encoder and the estimate state of a tile job (ET_LOW .. ET_BITS)
pub struct Ec {
    low: u64,
    rng: u32,
    cnt: i32,
    pre: Vec<u16>, // precarry buffer (ET_PRE / ET_OFFS)
    pub est: bool, // ET_EST: symbols only add their cost to bits (EstCoder)
    pub bits: u64, // ET_BITS: Q8 bits (estimate mode)
}

impl Default for Ec {
    fn default() -> Self {
        Self::new()
    }
}

/// ae_cost: Q8 bits of symbol s of a CDF array
#[inline]
pub fn cost(cdf: &[u16], s: u32) -> u32 {
    let s = s as usize;
    let mut p = cdf[s] as u32;
    if s != 0 {
        p = p.wrapping_sub(cdf[s - 1] as u32);
    }
    COST[(p >> 5).min(1023) as usize] as u32
}

impl Ec {
    /// ae_ecinit
    pub fn new() -> Ec {
        Ec { low: 0, rng: 0x8000, cnt: -9, pre: Vec::with_capacity(EC_PCAP0), est: false, bits: 0 }
    }

    /// ae_put
    #[inline]
    pub fn put(&mut self, cdf: &mut [u16], s: u32, n: u32) {
        if self.est {
            self.bits += cost(cdf, s) as u64;
        } else {
            self.encode(cdf, s, n);
        }
    }

    // ec_encode: od_ec_encode_q15 of symbol s (fl = 32768 - cdf[s - 1], fh = 32768 - cdf[s]),
    // normalisation, then the CDF update of read_symbol
    fn encode(&mut self, cdf: &mut [u16], s: u32, n: u32) {
        let su = s as usize;
        let nu = n as usize;
        let fl = if s != 0 { 32768 - cdf[su - 1] as u32 } else { 32768 };
        let fh = 32768 - cdf[su] as u32;
        self.encode_q15(fl, fh, s, n);
        // adaptation (rate 3 + (cnt > 15) + (cnt > 31) + Min(FloorLog2(n), 2))
        let cnt = cdf[nu] as u32;
        let rate = 4 + (n >= 4) as u32 + (cnt > 15) as u32 + (cnt > 31) as u32;
        cdf[nu] = (cnt + (cnt < 32) as u32) as u16;
        for (i, c) in cdf[..nu - 1].iter_mut().enumerate() {
            let v = *c as u32;
            *c = if (i as u32) < s { v - (v >> rate) } else { v + ((32768 - v) >> rate) } as u16;
        }
    }

    // the interval update and od_ec_enc_normalize
    #[inline]
    fn encode_q15(&mut self, fl: u32, fh: u32, s: u32, n: u32) {
        let mut r = self.rng;
        let mut low = self.low;
        let r8 = r >> 8;
        // 32-bit arithmetic as the assembly (wrapping where it could)
        let ns4 = (n.wrapping_sub(s)).wrapping_mul(4);
        let v = ((r8 * (fh >> 6)) >> 1).wrapping_add(ns4).wrapping_sub(4);
        if fl < 32768 {
            let u = ((r8 * (fl >> 6)) >> 1).wrapping_add(ns4);
            low = low.wrapping_add(r.wrapping_sub(u) as u64);
            r = u.wrapping_sub(v);
        } else {
            r = r.wrapping_sub(v);
        }
        // normalisation
        let d = r.leading_zeros() as i32 - 16; // 16 - bit length of rng (15 - bsr)
        let mut c = self.cnt;
        let mut sv = c + d;
        if sv >= 0 {
            c += 16;
            let mut m = (1u64 << c) - 1;
            if sv >= 8 {
                self.pre.push((low >> c) as u16);
                low &= m;
                m >>= 8;
                c -= 8;
            }
            self.pre.push((low >> c) as u16);
            sv = c + d - 24;
            low &= m;
        }
        self.cnt = sv;
        self.low = low << d;
        self.rng = r << d;
    }

    // ec_bool: a bool with probability 1/2 (temporary CDF {16384, 32768}: no adaptation)
    #[inline]
    fn bool(&mut self, b: u32) {
        let fl = if b != 0 { 32768 - 16384 } else { 32768 };
        let fh = if b != 0 { 0 } else { 32768 - 16384 };
        self.encode_q15(fl, fh, b, 2);
    }

    /// ae_lit
    pub fn lit(&mut self, v: u32, nbits: u32) {
        if self.est {
            self.bits += (nbits as u64) << 8;
            return;
        }
        let mut k = nbits;
        while k != 0 {
            k -= 1;
            self.bool((v >> k) & 1);
        }
    }

    /// ae_ecdone: the tile's bytes
    pub fn done(mut self) -> Vec<u8> {
        let mut c = self.cnt;
        let mut s = c + 10;
        let mut e = ((self.low + 0x3FFF) & !0x3FFF) | 0x4000;
        if s > 0 {
            c += 16;
            let mut m = (1u64 << c) - 1;
            loop {
                self.pre.push((e >> c) as u16);
                e &= m;
                m >>= 8;
                c -= 8;
                s -= 8;
                if s <= 0 {
                    break;
                }
            }
        }
        let mut out = alloc::vec![0u8; self.pre.len()];
        let mut carry = 0u32;
        for (o, &p) in out.iter_mut().zip(self.pre.iter()).rev() {
            carry += p as u32;
            *o = carry as u8;
            carry >>= 8;
        }
        out
    }
}
