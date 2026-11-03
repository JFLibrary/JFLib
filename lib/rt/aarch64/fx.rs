// fx -- rt/fx.rs on ARM64: the scalar operations with the instructions of the CPU and the results
// of SSE (apps/portabletest compares them with portable/fx.rs).  They round the same way; what
// differs:
//   fcvtzs / fcvtns  saturate and give 0 for NaN, SSE gives the lowest value.  Too negative saturates
//                    to that anyway, so one comparison is enough.  (`as` is fcvtzs here.)
//   fsqrt            of a negative number gives ARM's default NaN (positive), SSE its "indefinite"
//                    (negative)
// No preserves_flags in the asm!: these instructions may set the flags of the FPSR, which nobody reads.

use core::arch::asm;

// the first values too high for the integer; ROUNDS_ABOVE_I32 ties to 2^31 (an even number)
const ABOVE_I32F: f32 = 2147483648.0;
const ABOVE_I32: f64 = 2147483648.0;
const ROUNDS_ABOVE_I32: f64 = 2147483647.5;
const ABOVE_I64: f64 = 9223372036854775808.0;

/// cvttss2si: truncate towards zero (0x80000000 for NaN / out of range)
#[inline]
pub fn cvtt(x: f32) -> i32 {
    if x < ABOVE_I32F { x as i32 } else { i32::MIN }
}

/// cvtss2si: round to nearest, ties to even
#[inline]
pub fn cvt(x: f32) -> i32 {
    let r: i32;
    // SAFETY: the instruction only computes one register from another.
    unsafe { asm!("fcvtns {0:w}, {1:s}", out(reg) r, in(vreg) x, options(pure, nomem, nostack)) };
    if x < ABOVE_I32F { r } else { i32::MIN }
}

/// sqrtss
#[inline]
pub fn sqrt(x: f32) -> f32 {
    let r: f32;
    // SAFETY: the instruction only computes one register from another.
    unsafe { asm!("fsqrt {0:s}, {1:s}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    if r.is_nan() { crate::portable::fx::arithmetic32(x, x, r) } else { r }
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
    if (t as f32) > x { t.wrapping_sub(1) } else { t }
}

#[inline]
pub fn ceil_i(x: f32) -> i32 {
    let t = cvtt(x);
    if (t as f32) < x { t.wrapping_add(1) } else { t }
}

/// round(x / 255) for x in 0..=65025: ((x + 128) * 257) >> 16 (pmulhuw), equal to
/// (x + 128 + ((x + 128) >> 8)) >> 8
#[inline]
pub fn div255(x: u32) -> u32 {
    debug_assert!(x < 1 << 24, "div255 is for x <= 65025");
    ((x + 128) * 257) >> 16
}

// ---- f64

/// sqrtsd
#[inline]
pub fn sqrtd(x: f64) -> f64 {
    let r: f64;
    // SAFETY: the instruction only computes one register from another.
    unsafe { asm!("fsqrt {0:d}, {1:d}", out(vreg) r, in(vreg) x, options(pure, nomem, nostack)) };
    if r.is_nan() { crate::portable::fx::arithmetic64(x, x, r) } else { r }
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
    if x < ABOVE_I32 { x as i32 } else { i32::MIN }
}

/// cvtsd2si (32-bit): round to nearest even (0x80000000 for NaN / out of range)
#[inline]
pub fn cvtd(x: f64) -> i32 {
    let r: i32;
    // SAFETY: the instruction only computes one register from another.
    unsafe { asm!("fcvtns {0:w}, {1:d}", out(reg) r, in(vreg) x, options(pure, nomem, nostack)) };
    if x < ROUNDS_ABOVE_I32 { r } else { i32::MIN }
}

/// cvtsd2si (64-bit): round to nearest even (i64::MIN for NaN / out of range)
#[inline]
pub fn cvtd64(x: f64) -> i64 {
    let r: i64;
    // SAFETY: the instruction only computes one register from another.
    unsafe { asm!("fcvtns {0:x}, {1:d}", out(reg) r, in(vreg) x, options(pure, nomem, nostack)) };
    if x < ABOVE_I64 { r } else { i64::MIN }
}

/// cvttsd2si (64-bit): truncate (i64::MIN for NaN / out of range)
#[inline]
pub fn cvttd64(x: f64) -> i64 {
    if x < ABOVE_I64 { x as i64 } else { i64::MIN }
}
