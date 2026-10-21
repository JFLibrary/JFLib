//! Portable integer slider mapping. The logarithmic path uses fixed-point binary
//! logarithms, so native and WebAssembly have identical rounding and need no libm.
use crate::consts::{Range, RG_LOG};

// log2(value) in Q40, for strictly positive i32 values. Q63 squaring fits in u128.
// This precision keeps even adjacent positive i32 values distinguishable.
fn log(value: i32) -> i64 {
    log_positive(value as u64)
}

fn log_positive(value: u64) -> i64 {
    let exponent = 63 - value.leading_zeros();
    let mut x = (value as u128) << (63 - exponent);
    let mut result = i64::from(exponent) << 40;
    for bit in (0..40).rev() {
        x = (x * x) >> 63;
        if x >= 1 << 64 {
            x >>= 1;
            result |= 1 << bit;
        }
    }
    result
}

impl Range {
    /// Map a value to the slider's usable pixel travel. RG_LOG applies to positive,
    /// increasing bounds; all other bounds use linear mapping. Degenerate ranges
    /// or travel return zero. Values outside the range are clamped.
    pub fn position(&self, value: i32, travel: i32) -> i32 {
        if travel <= 0 || self.max <= self.min { return 0; }
        let value = value.clamp(self.min, self.max);
        let (offset, span) = if self.flags & RG_LOG != 0 && self.min > 0 {
            (log(value) - log(self.min), log(self.max) - log(self.min))
        } else { (i64::from(value) - i64::from(self.min), i64::from(self.max) - i64::from(self.min)) };
        (i128::from(offset) * i128::from(travel) / i128::from(span)) as i32
    }

    /// Map pixel travel to the nearest integer value, before step snapping. The
    /// logarithmic inverse uses bounded binary search (at most 31 iterations).
    /// Keyboard steps and semantic values always remain in the original units.
    pub fn value_at(&self, position: i32, travel: i32) -> i32 {
        if travel <= 0 || self.max <= self.min { return self.min; }
        let position = position.clamp(0, travel);
        if position == 0 { return self.min; }
        if position == travel { return self.max; }
        if self.flags & RG_LOG == 0 || self.min <= 0 {
            let span = i64::from(self.max) - i64::from(self.min);
            return (i64::from(self.min) + (i64::from(position) * span + i64::from(travel / 2)) / i64::from(travel)) as i32;
        }
        let target = log(self.min) + (i128::from(log(self.max) - log(self.min)) * i128::from(position) / i128::from(travel)) as i64;
        let (mut low, mut high) = (self.min, self.max);
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if log(mid) <= target { low = mid; } else { high = mid; }
        }
        // Round in value space, not in logarithmic space: the boundary between
        // integers is low + 0.5, not their geometric mean.
        let midpoint = log_positive(low as u64 * 2 + 1) - (1_i64 << 40);
        if target < midpoint { low } else { high }
    }
}
