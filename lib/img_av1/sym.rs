// sym -- AV1 symbol decoder (specification 8.2): range decoder with CDF adaptation
//                                                                    [src/img/av1_sym.s]
//
//   Sd::init(d)                 init_symbol( size ) on the tile data d
//   Sd::sym(d, cdf, n, dcu)     read_symbol( cdf ) for an alphabet of n symbols (cdf: n + 1 u16,
//                               adapted unless disable_cdf_update)
//   Sd::bool(d)                 read_bool( )
//   Sd::lit(d, n)               read_literal( n ) = L(n)
//   Sd::ns(d, n)                NS(n)
//
// State: SymbolValue and SymbolRange as in the specification (SymbolMaxBits is not read); the bits are fetched
// through a left-aligned 64-bit cache (`cbits` valid bits) filled from d[buf..].  Past the end of
// the tile data the cache supplies zero bits, which is what the specification's numBits /
// paddedData rule does.

/// the symbol decoder state (TS_SBUF .. TS_SMAX)
#[derive(Clone, Copy, Default)]
pub(crate) struct Sd {
    buf: usize,  // next byte of the tile data
    cache: u64,  // bit cache (left aligned)
    cbits: i32,  // valid bits in the cache
    val: u32,    // SymbolValue
    rng: u32,    // SymbolRange (SymbolMaxBits is never read: not kept)
}

impl Sd {
    /// av1_syminit: init_symbol( d.len() )
    pub fn init(&mut self, d: &[u8]) {
        let mut cache = 0u64;
        let mut bits = 0i32;
        let mut p = 0usize;
        while bits <= 56 && p < d.len() {
            cache |= (d[p] as u64) << (56 - bits);
            bits += 8;
            p += 1;
        }
        self.buf = p;
        // buf << ( 15 - numBits ) = the first 15 bits of the cache (zeros past the end)
        let v = (cache >> 49) as u32;
        self.cache = cache << 15;
        self.cbits = (bits - 15).max(0);
        self.val = v ^ 0x7FFF;
        self.rng = 0x8000;
    }

    /// SYMNORM: renormalise with the new range rng (> 0) and value val
    #[inline(always)]
    fn norm(&mut self, d: &[u8], rng: u32, val: u32) {
        let bits = 15i32.wrapping_sub(31i32.wrapping_sub(rng.leading_zeros() as i32));
        if bits == 0 {
            self.rng = rng;
            self.val = val;
            return;
        }
        let b = bits as u32;
        self.rng = rng.wrapping_shl(b);
        let mut val = val.wrapping_shl(b) | (1u32.wrapping_shl(b).wrapping_sub(1)); // ((SymbolValue + 1) << bits) - 1
        let mut cb = self.cbits;
        let mut cache = self.cache;
        if cb < bits {
            let p = self.buf;
            if let Some(w) = d.get(p..p + 8) {
                // the bytes while cb <= 56: k = ( 56 - cb ) / 8 + 1 of them, as one big-endian word
                let k = ((56 - cb) >> 3) as u32 + 1;
                let v = u64::from_be_bytes([w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]);
                let t = if k == 8 { v } else { v >> (64 - 8 * k) };
                cache |= t << ((56 - cb) & 7);
                cb += 8 * k as i32;
                self.buf = p + k as usize;
            } else {
                let mut p = p;
                while cb <= 56 && p < d.len() {
                    cache |= (d[p] as u64) << (56 - cb);
                    cb += 8;
                    p += 1;
                }
                self.buf = p;
            }
        }
        let nb = (cache >> (64 - b)) as u32; // next bits (zero past the end)
        self.cache = cache << b;
        self.cbits = (cb - bits).max(0);
        val ^= nb;
        self.val = val;
    }

    /// av1_sym: read_symbol( cdf ), n symbols; adapts cdf unless dcu (disable_cdf_update)
    #[inline(always)]
    pub fn sym(&mut self, d: &[u8], cdf: &mut [u16], n: usize, dcu: bool) -> u32 {
        let r8 = self.rng >> 8;
        let val = self.val;
        let mut s = 0usize;
        let mut minp = (n as u32).wrapping_mul(4).wrapping_sub(4); // EC_MIN_PROB * ( N - symbol - 1 )
        let mut cur;
        loop {
            cur = ((32768u32.wrapping_sub(cdf[s] as u32) >> 6).wrapping_mul(r8) >> 1).wrapping_add(minp);
            if val >= cur {
                break;
            }
            s += 1;
            minp = minp.wrapping_sub(4);
        }
        let prev = if s == 0 {
            self.rng
        } else {
            ((32768u32.wrapping_sub(cdf[s - 1] as u32) >> 6).wrapping_mul(r8) >> 1)
                .wrapping_add(minp)
                .wrapping_add(4)
        };
        self.norm(d, prev.wrapping_sub(cur), val.wrapping_sub(cur));
        if !dcu {
            // adaptation
            let cnt = cdf[n] as u32;
            let mut rate = if n >= 4 { 5 } else { 4 }; // 3 + Min( FloorLog2( N ), 2 )
            if cnt > 15 {
                rate += 1;
            }
            if cnt > 31 {
                rate += 1;
            }
            if cnt < 32 {
                cdf[n] = cdf[n].wrapping_add(1);
            }
            for (i, c) in cdf[..n - 1].iter_mut().enumerate() {
                let v = *c as u32;
                *c = if i < s { v - (v >> rate) } else { v.wrapping_add(32768u32.wrapping_sub(v) >> rate) } as u16;
            }
        }
        s as u32
    }

    /// av1_bool: read_bool( ): read_symbol of {1 << 14, 1 << 15, 0} (no adaptation)
    #[inline]
    pub fn bool(&mut self, d: &[u8]) -> u32 {
        let rng = self.rng;
        let val = self.val;
        let cur = ((rng >> 8) << 7).wrapping_add(4); // cur of symbol 0
        if val < cur {
            self.norm(d, cur, val);
            1
        } else {
            self.norm(d, rng.wrapping_sub(cur), val.wrapping_sub(cur));
            0
        }
    }

    /// av1_lit: read_literal( n )
    pub fn lit(&mut self, d: &[u8], n: u32) -> u32 {
        let mut x = 0u32;
        for _ in 0..n {
            let b = self.bool(d);
            x = x.wrapping_mul(2).wrapping_add(b);
        }
        x
    }

    /// av1_ns: NS( n ) (n >= 1)
    pub fn ns(&mut self, d: &[u8], n: u32) -> u32 {
        let w = 32 - n.leading_zeros(); // FloorLog2( n ) + 1
        let m = 1u32.wrapping_shl(w).wrapping_sub(n);
        let v = self.lit(d, w - 1);
        if v < m {
            return v;
        }
        let extra = self.bool(d);
        (v.wrapping_mul(2)).wrapping_add(extra).wrapping_sub(m)
    }
}
