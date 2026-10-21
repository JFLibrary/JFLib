// emath -- float helpers of the new encoder (libjxl base/fast_math-inl.h, rational_polynomial-inl.h)
//                                                                          [jxl-plus: no asm]
//
//   fast_log2f(x) -> f32       FastLog2f (2,2 rational polynomial, L1 error ~4e-6)
//   fast_pow2f(x) -> f32       FastPow2f
//   fast_powf(b, e) -> f32     FastPowf = FastPow2f( FastLog2f( b ) * e )
//   ln(x) / exp(x) / log1p(x)  natural logarithm / exponential through the two above
//   floorf / roundf / ceilf    as the C functions (no_std has none)
//
// The encoder's heuristics only need these to be deterministic (the same bits on every
// thread and target); they are not part of any decoded value.

use crate::tab;

const LN2: f32 = core::f32::consts::LN_2;
const LOG2E: f32 = core::f32::consts::LOG2_E;

/// FastLog2f (libjxl): x > 0
#[inline]
pub fn fast_log2f(x: f32) -> f32 {
    let p = &tab::FASTPOW;
    let bits = x.to_bits() as i32;
    let exp_bits = bits.wrapping_sub(0x3f2a_aaab);
    let ex = exp_bits >> 23;
    let m = f32::from_bits(bits.wrapping_sub(ex.wrapping_shl(23)) as u32) - 1.0;
    let mut yp = p[2] * m + p[1];
    yp = yp * m + p[0];
    let mut yq = p[5] * m + p[4];
    yq = yq * m + p[3];
    yp / yq + ex as f32
}

/// FastPow2f (libjxl)
#[inline]
pub fn fast_pow2f(x: f32) -> f32 {
    let p = &tab::FASTPOW;
    let fl = floorf(x);
    let i = fl as i32;
    let pow2 = f32::from_bits((i.wrapping_add(127) as u32).wrapping_shl(23));
    let f = x - fl;
    let mut num = f + p[6];
    num = num * f + p[7];
    num = num * f + p[8];
    num = num * pow2;
    let mut den = f * p[9] + p[10];
    den = den * f + p[11];
    den = den * f + p[12];
    num / den
}

/// FastPowf: b > 0
#[inline]
pub fn fast_powf(b: f32, e: f32) -> f32 {
    fast_pow2f(fast_log2f(b) * e)
}

/// natural logarithm (x > 0)
#[inline]
pub fn ln(x: f32) -> f32 {
    fast_log2f(x) * LN2
}

/// e^x
#[inline]
pub fn exp(x: f32) -> f32 {
    fast_pow2f(x * LOG2E)
}

/// log(1 + x) for x >= 0 (small x exact to first order)
#[inline]
pub fn log1p(x: f32) -> f32 {
    if x < 1e-4 {
        return x - 0.5 * x * x;
    }
    ln(1.0 + x)
}

/// floor
#[inline]
pub fn floorf(x: f32) -> f32 {
    if !(x.abs() < 8_388_608.0) {
        return x; // large, inf or NaN: already integral
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

/// ceil
#[inline]
pub fn ceilf(x: f32) -> f32 {
    -floorf(-x)
}

/// round half away from zero (C roundf)
#[inline]
pub fn roundf(x: f32) -> f32 {
    if !(x.abs() < 8_388_608.0) {
        return x;
    }
    let t = floorf(x.abs() + 0.5);
    if x < 0.0 { -t } else { t }
}

/// rint: round half to even (the default FP rounding)
#[inline]
pub fn rintf(x: f32) -> f32 {
    rt::fx::cvt(x) as f32
}

/// f32 absolute value
#[inline(always)]
pub fn absf(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFF_FFFF)
}

/// sqrt
#[inline(always)]
pub fn sqrtf(x: f32) -> f32 {
    rt::fx::sqrt(x)
}

/// x^y for x > 0 in f64 precision terms of the fast functions (used for constants)
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    fast_powf(x, y)
}
