// Scalar IEEE-754 operations matching the native SSE conversion and operand-selection
// rules. Integer square root avoids libm imports and gives correctly rounded results.

/// Round a finite binary64 value to its nearest integer, with ties to even.
pub fn nearest(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4503599627370496.0 || x == 0.0 {
        return x;
    }
    let t = x as i64;
    let f = x - t as f64;
    let r = if f > 0.5 || (f == 0.5 && t & 1 != 0) {
        t + 1
    } else if f < -0.5 || (f == -0.5 && t & 1 != 0) {
        t - 1
    } else {
        t
    };
    if r == 0 {
        f64::from_bits(x.to_bits() & (1 << 63))
    } else {
        r as f64
    }
}

/// Select and quiet NaN payloads as scalar SSE arithmetic does, including invalid operations.
#[inline]
pub fn arithmetic32(a: f32, b: f32, result: f32) -> f32 {
    if a.is_nan() {
        f32::from_bits(a.to_bits() | 0x00400000)
    } else if b.is_nan() {
        f32::from_bits(b.to_bits() | 0x00400000)
    } else if result.is_nan() {
        f32::from_bits(0xffc00000)
    } else {
        result
    }
}

/// Binary64 SSE arithmetic NaN payload selection.
#[inline]
pub fn arithmetic64(a: f64, b: f64, result: f64) -> f64 {
    if a.is_nan() {
        f64::from_bits(a.to_bits() | 0x0008000000000000)
    } else if b.is_nan() {
        f64::from_bits(b.to_bits() | 0x0008000000000000)
    } else if result.is_nan() {
        f64::from_bits(0xfff8000000000000)
    } else {
        result
    }
}

/// SSE truncating f32 to i32 conversion, including the indefinite overflow value.
#[inline]
pub fn cvtt(x: f32) -> i32 {
    cvttd(x as f64)
}
/// SSE nearest-even f32 to i32 conversion.
#[inline]
pub fn cvt(x: f32) -> i32 {
    cvtd(x as f64)
}
/// SSE truncating f64 to i32 conversion.
#[inline]
pub fn cvttd(x: f64) -> i32 {
    if !(-2147483648.0..2147483648.0).contains(&x) {
        i32::MIN
    } else {
        x as i32
    }
}
/// SSE nearest-even f64 to i32 conversion.
#[inline]
pub fn cvtd(x: f64) -> i32 {
    cvttd(nearest(x))
}
/// SSE truncating f64 to i64 conversion.
#[inline]
pub fn cvttd64(x: f64) -> i64 {
    if !(-9223372036854775808.0..9223372036854775808.0).contains(&x) {
        i64::MIN
    } else {
        x as i64
    }
}
/// SSE nearest-even f64 to i64 conversion.
#[inline]
pub fn cvtd64(x: f64) -> i64 {
    cvttd64(nearest(x))
}

fn isqrt(mut n: u128) -> (u128, u128) {
    let mut root = 0;
    let mut bit = 1u128 << 126;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if n >= root + bit {
            n -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    (root, n)
}

fn sqrt_bits(bits: u64, mantissa: u32, exp_bits: u32) -> u64 {
    let sign = 1u64 << (mantissa + exp_bits);
    let emask = (1u64 << exp_bits) - 1;
    let frac_mask = (1u64 << mantissa) - 1;
    let exponent = (bits >> mantissa) & emask;
    let fraction = bits & frac_mask;
    if exponent == emask && fraction != 0 {
        return bits | (1u64 << (mantissa - 1));
    }
    if bits & !sign == 0 {
        return bits;
    }
    if bits & sign != 0 {
        return sign | (emask << mantissa) | (1u64 << (mantissa - 1));
    }
    if exponent == emask {
        return bits;
    }
    let bias = (1i32 << (exp_bits - 1)) - 1;
    let (mut m, mut e) = if exponent == 0 {
        let sh = fraction.leading_zeros() - (63 - mantissa);
        (fraction << sh, 1 - bias - sh as i32)
    } else {
        (fraction | (1u64 << mantissa), exponent as i32 - bias)
    };
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }
    let (mut r, remainder) = isqrt((m as u128) << mantissa);
    if remainder > r {
        r += 1;
    }
    let mut re = e / 2;
    if r == 1u128 << (mantissa + 1) {
        r >>= 1;
        re += 1;
    }
    (((re + bias) as u64) << mantissa) | (r as u64 & frac_mask)
}

/// Correctly rounded binary32 square root; preserves signed zero and quiets NaNs.
#[inline]
pub fn sqrt(x: f32) -> f32 {
    f32::from_bits(sqrt_bits(x.to_bits() as u64, 23, 8) as u32)
}
/// Correctly rounded binary64 square root; preserves signed zero and quiets NaNs.
#[inline]
pub fn sqrtd(x: f64) -> f64 {
    f64::from_bits(sqrt_bits(x.to_bits(), 52, 11))
}
/// SSE minimum: return the second operand for equal or unordered operands.
#[inline]
pub fn minss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}
/// SSE maximum: return the second operand for equal or unordered operands.
#[inline]
pub fn maxss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}
/// Binary64 SSE minimum.
#[inline]
pub fn minsd(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}
/// Binary64 SSE maximum.
#[inline]
pub fn maxsd(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}
/// Native floor contract: truncate, then decrement if necessary.
#[inline]
pub fn floor_i(x: f32) -> i32 {
    let t = cvtt(x);
    if (t as f32) > x { t.wrapping_sub(1) } else { t }
}
/// Native ceiling contract: truncate, then increment if necessary.
#[inline]
pub fn ceil_i(x: f32) -> i32 {
    let t = cvtt(x);
    if (t as f32) < x { t.wrapping_add(1) } else { t }
}
/// Rounded division by 255 for 0..=65025.
#[inline]
pub fn div255(x: u32) -> u32 {
    ((x + 128) * 257) >> 16
}
