// rng -- deterministic pseudo-random numbers for simulations, tests and games (NOT for keys:
// those come from plat::fill_random)
//
//   SplitMix64::new(seed)      next_u64; derives seeds (Vigna's splitmix64)
//   Pcg32::new(seed, stream)   PCG-XSH-RR 64/32 (O'Neill), the reference pcg32_srandom_r: the
//                              same seed and stream give the same numbers on every target;
//                              different streams are independent sequences
//   Pcg32::derive(seed, id)    a generator for one of many objects (seed and id mixed through
//                              SplitMix64), so adding an object does not shift the others
//   next_u32 / next_u64 / below(n) (unbiased) / range(lo, hi) / f64 (0..1, 53 bits) /
//   chance(num, den) / chance_ppm(ppm) / fill(bytes) / shuffle(slice) / advance(delta)

/// SplitMix64: a 64-bit generator mostly used to expand one seed into many
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub const fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const PCG_MUL: u64 = 6_364_136_223_846_793_005;

/// PCG32 (XSH-RR, 64-bit state, 32-bit output)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// the generator of `seed` on `stream` (pcg32_srandom_r(seed, stream))
    pub fn new(seed: u64, stream: u64) -> Pcg32 {
        let mut g = Pcg32 { state: 0, inc: (stream << 1) | 1 };
        g.step();
        g.state = g.state.wrapping_add(seed);
        g.step();
        g
    }
    /// a generator for object `id` under a common seed
    pub fn derive(seed: u64, id: u64) -> Pcg32 {
        let mut m = SplitMix64::new(seed ^ id.wrapping_mul(0xD6E8_FEB8_6659_FD93));
        let s = m.next_u64();
        let st = m.next_u64();
        Pcg32::new(s, st)
    }
    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MUL).wrapping_add(self.inc);
    }
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xs = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xs.rotate_right(rot)
    }
    pub fn next_u64(&mut self) -> u64 {
        let hi = self.next_u32() as u64;
        (hi << 32) | self.next_u32() as u64
    }
    /// uniform in 0..n without modulo bias (n = 0 gives 0)
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let threshold = n.wrapping_neg() % n;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % n;
            }
        }
    }
    /// uniform in 0..n for a 64-bit n without modulo bias (n = 0 gives 0)
    fn below64(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let threshold = n.wrapping_neg() % n;
        loop {
            let r = self.next_u64();
            if r >= threshold {
                return r % n;
            }
        }
    }
    /// uniform in lo..=hi
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = hi as i128 - lo as i128 + 1; // 2 ..= 2^64
        if span <= u32::MAX as i128 {
            return lo + self.below(span as u32) as i64;
        }
        if span == 1i128 << 64 {
            return self.next_u64() as i64;
        }
        // wide ranges: rejection on 64 bits, as below()
        let span = span as u64;
        let threshold = span.wrapping_neg() % span;
        loop {
            let r = self.next_u64();
            if r >= threshold {
                return (lo as i128 + (r % span) as i128) as i64;
            }
        }
    }
    /// uniform in [0, 1) with 53 random bits
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
    /// true with probability num / den
    pub fn chance(&mut self, num: u32, den: u32) -> bool {
        num >= den || (den > 0 && self.below(den) < num)
    }
    /// true with probability ppm / 1 000 000 (loss and error rates)
    pub fn chance_ppm(&mut self, ppm: u32) -> bool {
        self.chance(ppm, 1_000_000)
    }
    pub fn fill(&mut self, out: &mut [u8]) {
        for c in out.chunks_mut(4) {
            let r = self.next_u32().to_le_bytes();
            c.copy_from_slice(&r[..c.len()]);
        }
    }
    /// Fisher-Yates
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        let mut i = v.len();
        while i > 1 {
            let j = if i <= u32::MAX as usize { self.below(i as u32) as usize } else { self.below64(i as u64) as usize };
            i -= 1;
            v.swap(i, j);
        }
    }
    /// skips `delta` outputs in O(log delta) (Brown's jump ahead for LCGs)
    pub fn advance(&mut self, mut delta: u64) {
        let (mut acc_mul, mut acc_add) = (1u64, 0u64);
        let (mut cur_mul, mut cur_add) = (PCG_MUL, self.inc);
        while delta > 0 {
            if delta & 1 != 0 {
                acc_mul = acc_mul.wrapping_mul(cur_mul);
                acc_add = acc_add.wrapping_mul(cur_mul).wrapping_add(cur_add);
            }
            cur_add = cur_mul.wrapping_add(1).wrapping_mul(cur_add);
            cur_mul = cur_mul.wrapping_mul(cur_mul);
            delta >>= 1;
        }
        self.state = acc_mul.wrapping_mul(self.state).wrapping_add(acc_add);
    }
}
