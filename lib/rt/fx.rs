// fx -- scalar SSE float operations with exactly the semantics the assembly uses
// (cvttss2si, cvtss2si with round-half-even, sqrtss, minss, maxss), so float results
// are bit-identical to ASMLIB.  Rust never fuses a multiply and an add into an FMA.

use core::arch::x86_64::{_mm_cvtss_f32, _mm_cvtss_si32, _mm_cvttss_si32, _mm_set_ss, _mm_sqrt_ss};

/// cvttss2si: truncate towards zero (0x80000000 for NaN / out of range)
#[inline]
pub fn cvtt(x: f32) -> i32 {
    // SAFETY: SSE is part of the x86-64 baseline.
    unsafe { _mm_cvttss_si32(_mm_set_ss(x)) }
}

/// cvtss2si: round to nearest, ties to even (the default MXCSR mode)
#[inline]
pub fn cvt(x: f32) -> i32 {
    // SAFETY: SSE is part of the x86-64 baseline.
    unsafe { _mm_cvtss_si32(_mm_set_ss(x)) }
}

#[inline]
pub fn sqrt(x: f32) -> f32 {
    // SAFETY: SSE is part of the x86-64 baseline.
    unsafe { _mm_cvtss_f32(_mm_sqrt_ss(_mm_set_ss(x))) }
}

/// minss a, b: a < b ? a : b (b when unordered)
#[inline]
pub fn minss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// maxss a, b: a > b ? a : b (b when unordered)
#[inline]
pub fn maxss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// floor / ceil the way f_floor / f_ceil do it: truncate, then step if needed
#[inline]
pub fn floor_i(x: f32) -> i32 {
    let t = cvtt(x);
    if (t as f32) > x { t - 1 } else { t }
}

#[inline]
pub fn ceil_i(x: f32) -> i32 {
    let t = cvtt(x);
    if (t as f32) < x { t + 1 } else { t }
}

/// round(x / 255) for x in 0..=65025: ((x + 128) * 257) >> 16 (pmulhuw), equal to
/// (x + 128 + ((x + 128) >> 8)) >> 8
#[inline]
pub fn div255(x: u32) -> u32 {
    ((x + 128) * 257) >> 16
}

// ---- f64 (SSE2 sd / pd forms)

/// sqrtsd
#[inline]
pub fn sqrtd(x: f64) -> f64 {
    use core::arch::x86_64::{_mm_cvtsd_f64, _mm_set_sd, _mm_sqrt_pd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvtsd_f64(_mm_sqrt_pd(_mm_set_sd(x))) }
}

/// maxsd a, b: a > b ? a : b (b when unordered)
#[inline]
pub fn maxsd(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// minsd a, b: a < b ? a : b (b when unordered)
#[inline]
pub fn minsd(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

/// cvttsd2si (32-bit): truncate towards zero (0x80000000 for NaN / out of range)
#[inline]
pub fn cvttd(x: f64) -> i32 {
    use core::arch::x86_64::{_mm_cvttsd_si32, _mm_set_sd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvttsd_si32(_mm_set_sd(x)) }
}

/// cvtsd2si (32-bit): round to nearest even (0x80000000 for NaN / out of range)
#[inline]
pub fn cvtd(x: f64) -> i32 {
    use core::arch::x86_64::{_mm_cvtsd_si32, _mm_set_sd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvtsd_si32(_mm_set_sd(x)) }
}

/// cvtsd2si (64-bit): round to nearest even (i64::MIN for NaN / out of range)
#[inline]
pub fn cvtd64(x: f64) -> i64 {
    use core::arch::x86_64::{_mm_cvtsd_si64, _mm_set_sd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvtsd_si64(_mm_set_sd(x)) }
}

/// cvttsd2si (64-bit): truncate (i64::MIN for NaN / out of range)
#[inline]
pub fn cvttd64(x: f64) -> i64 {
    use core::arch::x86_64::{_mm_cvttsd_si64, _mm_set_sd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvttsd_si64(_mm_set_sd(x)) }
}
