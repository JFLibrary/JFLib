// NEON backend: part of the ARM64 baseline, so no check at run time and no second register
// width (lib.rs: Neon128 as Sse2, Scalar256 as Avx2).  Every result is SSE2's bit for bit
// (apps/portabletest compares them with the scalar code).  Where the obvious instruction
// computes something else:
//   float arithmetic   ARM's default NaN is positive, SSE's "indefinite" negative, and a
//                      signalling second operand wins over a quiet first one.  A result with a
//                      NaN takes a cold path with SSE's rule, per lane (rt::portable::fx, as
//                      in the scalar code)
//   maxf / minf / ...  fmax and fmin differ on NaN and signed zeros: compare and select
//   float to int       fcvtzs / fcvtns saturate and give 0 for NaN, SSE 0x80000000
//   srl16<N> / ...     the _n_ intrinsics refuse what SSE allows (N = 0, N > width) at compile
//                      time, even on a branch not taken: shifts by a constant register, which
//                      LLVM turns back into the immediate instruction
//   shuf16<IMM> / ...  no such instructions: tbl with a constant table, which LLVM turns into
//                      the cheapest shuffle there is
use crate::Simd;
use core::arch::aarch64::*;
use rt::portable::fx;

/// NEON: 128-bit registers (the ARM64 baseline)
#[derive(Clone, Copy)]
pub struct Neon128;

type R = uint8x16_t;

#[inline(always)]
fn chk(b: &[u8], n: usize) {
    assert!(b.len() >= n, "simd: slice too short");
}

// SAFETY (every neon! block): NEON is part of the ARM64 baseline, enabled on every target this
// code is built for; the intrinsics only need the feature (no memory access).
macro_rules! neon {
    ($e:expr) => {
        unsafe { $e }
    };
}

// the lanes of a register, and back; no instruction
#[inline(always)]
fn i8s(a: R) -> int8x16_t {
    neon!(vreinterpretq_s8_u8(a))
}
#[inline(always)]
fn u16s(a: R) -> uint16x8_t {
    neon!(vreinterpretq_u16_u8(a))
}
#[inline(always)]
fn i16s(a: R) -> int16x8_t {
    neon!(vreinterpretq_s16_u8(a))
}
#[inline(always)]
fn u32s(a: R) -> uint32x4_t {
    neon!(vreinterpretq_u32_u8(a))
}
#[inline(always)]
fn i32s(a: R) -> int32x4_t {
    neon!(vreinterpretq_s32_u8(a))
}
#[inline(always)]
fn u64s(a: R) -> uint64x2_t {
    neon!(vreinterpretq_u64_u8(a))
}
#[inline(always)]
fn f32s(a: R) -> float32x4_t {
    neon!(vreinterpretq_f32_u8(a))
}
#[inline(always)]
fn f64s(a: R) -> float64x2_t {
    neon!(vreinterpretq_f64_u8(a))
}
trait Lanes: Copy {
    fn r(self) -> R;
}
macro_rules! lanes {
    ($t:ty, $to:ident) => {
        impl Lanes for $t {
            #[inline(always)]
            fn r(self) -> R {
                neon!($to(self))
            }
        }
    };
}
lanes!(int8x16_t, vreinterpretq_u8_s8);
lanes!(uint16x8_t, vreinterpretq_u8_u16);
lanes!(int16x8_t, vreinterpretq_u8_s16);
lanes!(uint32x4_t, vreinterpretq_u8_u32);
lanes!(int32x4_t, vreinterpretq_u8_s32);
lanes!(uint64x2_t, vreinterpretq_u8_u64);
lanes!(float32x4_t, vreinterpretq_u8_f32);
lanes!(float64x2_t, vreinterpretq_u8_f64);

// the lanes of a result with a NaN, by SSE's rule; a and b as in the operation
#[cold]
#[inline(never)]
fn sse_nan32(a: float32x4_t, b: float32x4_t, r: float32x4_t) -> float32x4_t {
    let (mut x, mut y, mut z) = ([0f32; 4], [0f32; 4], [0f32; 4]);
    // SAFETY: 16 writable and readable bytes in each array.
    unsafe {
        vst1q_f32(x.as_mut_ptr(), a);
        vst1q_f32(y.as_mut_ptr(), b);
        vst1q_f32(z.as_mut_ptr(), r);
        for i in 0..4 {
            z[i] = fx::arithmetic32(x[i], y[i], z[i]);
        }
        vld1q_f32(z.as_ptr())
    }
}
#[cold]
#[inline(never)]
fn sse_nan64(a: float64x2_t, b: float64x2_t, r: float64x2_t) -> float64x2_t {
    let (mut x, mut y, mut z) = ([0f64; 2], [0f64; 2], [0f64; 2]);
    // SAFETY: 16 writable and readable bytes in each array.
    unsafe {
        vst1q_f64(x.as_mut_ptr(), a);
        vst1q_f64(y.as_mut_ptr(), b);
        vst1q_f64(z.as_mut_ptr(), r);
        for i in 0..2 {
            z[i] = fx::arithmetic64(x[i], y[i], z[i]);
        }
        vld1q_f64(z.as_ptr())
    }
}
// fmaxv / fmaxp pass on a NaN of any lane; a NaN in an operand always is one in r
#[inline(always)]
fn result32(a: float32x4_t, b: float32x4_t, r: float32x4_t) -> R {
    if neon!(vmaxvq_f32(r)).is_nan() {
        sse_nan32(a, b, r).r()
    } else {
        r.r()
    }
}
#[inline(always)]
fn result64(a: float64x2_t, b: float64x2_t, r: float64x2_t) -> R {
    if neon!(vmaxvq_f64(r)).is_nan() {
        sse_nan64(a, b, r).r()
    } else {
        r.r()
    }
}

// tbl: bytes of a by index, 0 for an index out of range (255)
#[inline(always)]
fn tbl(a: R, index: &[u8; 16]) -> R {
    // SAFETY: 16 readable bytes.
    unsafe { vqtbl1q_u8(a, vld1q_u8(index.as_ptr())) }
}
const fn shuf16_index(imm: i32) -> [u8; 16] {
    let mut t = [0; 16];
    let mut i = 0;
    while i < 8 {
        let word = i / 4 * 4 + ((imm >> (i % 4 * 2)) & 3) as usize;
        t[i * 2] = (word * 2) as u8;
        t[i * 2 + 1] = (word * 2 + 1) as u8;
        i += 1;
    }
    t
}
const fn shuf32_index(imm: i32) -> [u8; 16] {
    let mut t = [0; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = (((imm >> (i / 4 * 2)) & 3) * 4) as u8 + (i % 4) as u8;
        i += 1;
    }
    t
}
const fn bsll_index(n: i32) -> [u8; 16] {
    let mut t = [255; 16];
    let mut i = 0;
    while i < 16 {
        if (n as u32) <= i {
            t[i as usize] = (i - n as u32) as u8;
        }
        i += 1;
    }
    t
}
const fn bsrl_index(n: i32) -> [u8; 16] {
    let mut t = [255; 16];
    let mut i = 0;
    while i < 16 {
        if (n as u32) < 16 - i {
            t[i as usize] = (i + n as u32) as u8;
        }
        i += 1;
    }
    t
}

const TWO31F: f32 = 2147483648.0;
const TWO31D: f64 = 2147483648.0;

impl Simd for Neon128 {
    type V = R;
    const BYTES: usize = 16;
    const PX: usize = 2;

    #[inline(always)]
    fn zero(self) -> R {
        neon!(vdupq_n_u8(0))
    }
    #[inline(always)]
    fn splat8(self, v: u8) -> R {
        neon!(vdupq_n_u8(v))
    }
    #[inline(always)]
    fn splat16(self, v: u16) -> R {
        neon!(vdupq_n_u16(v)).r()
    }
    #[inline(always)]
    fn splat32(self, v: u32) -> R {
        neon!(vdupq_n_u32(v)).r()
    }
    #[inline(always)]
    fn splat64(self, v: u64) -> R {
        neon!(vdupq_n_u64(v)).r()
    }
    #[inline(always)]
    fn splatf(self, v: f32) -> R {
        neon!(vdupq_n_f32(v)).r()
    }

    // ---- memory (vld1 / vst1 on bytes: no alignment asked)
    #[inline(always)]
    fn load(self, b: &[u8]) -> R {
        chk(b, 16);
        // SAFETY: 16 readable bytes (checked).
        unsafe { vld1q_u8(b.as_ptr()) }
    }
    #[inline(always)]
    fn store(self, b: &mut [u8], v: R) {
        chk(b, 16);
        // SAFETY: 16 writable bytes (checked).
        unsafe { vst1q_u8(b.as_mut_ptr(), v) }
    }
    #[inline(always)]
    fn load_px(self, b: &[u8]) -> R {
        self.widen_px(self.load_raw(b))
    }
    #[inline(always)]
    fn store_px(self, b: &mut [u8], v: R) {
        chk(b, 8);
        // SAFETY: 8 writable bytes (checked).
        unsafe { vst1_u8(b.as_mut_ptr(), vqmovun_s16(i16s(v))) }
    }
    #[inline(always)]
    fn bcast_px(self, b: &[u8]) -> R {
        chk(b, 2);
        neon!(vcombine_u16(vdup_n_u16(b[0] as u16), vdup_n_u16(b[1] as u16))).r()
    }
    #[inline(always)]
    fn load_raw(self, b: &[u8]) -> R {
        chk(b, 8);
        // SAFETY: 8 readable bytes (checked).
        unsafe { vcombine_u8(vld1_u8(b.as_ptr()), vdup_n_u8(0)) }
    }
    #[inline(always)]
    fn store_raw(self, b: &mut [u8], v: R) {
        chk(b, 8);
        // SAFETY: 8 writable bytes (checked).
        unsafe { vst1_u8(b.as_mut_ptr(), vget_low_u8(v)) }
    }
    #[inline(always)]
    fn widen_px(self, v: R) -> R {
        neon!(vmovl_u8(vget_low_u8(v))).r()
    }
    #[inline(always)]
    fn load_lane32(self, b: &[u8]) -> R {
        chk(b, 4);
        self.from_i32(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    #[inline(always)]
    fn load_i32(self, b: &[i32]) -> R {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { vld1q_u8(b.as_ptr() as *const u8) }
    }
    #[inline(always)]
    fn store_i32(self, b: &mut [i32], v: R) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { vst1q_u8(b.as_mut_ptr() as *mut u8, v) }
    }
    #[inline(always)]
    fn from_i32(self, v: i32) -> R {
        neon!(vsetq_lane_s32::<0>(v, vdupq_n_s32(0))).r()
    }
    #[inline(always)]
    fn to_i32(self, v: R) -> i32 {
        neon!(vgetq_lane_s32::<0>(i32s(v)))
    }

    // ---- 8-bit lanes
    #[inline(always)]
    fn add8(self, a: R, b: R) -> R {
        neon!(vaddq_u8(a, b))
    }
    #[inline(always)]
    fn sub8(self, a: R, b: R) -> R {
        neon!(vsubq_u8(a, b))
    }
    #[inline(always)]
    fn addus8(self, a: R, b: R) -> R {
        neon!(vqaddq_u8(a, b))
    }
    #[inline(always)]
    fn subus8(self, a: R, b: R) -> R {
        neon!(vqsubq_u8(a, b))
    }
    #[inline(always)]
    fn maxu8(self, a: R, b: R) -> R {
        neon!(vmaxq_u8(a, b))
    }
    #[inline(always)]
    fn minu8(self, a: R, b: R) -> R {
        neon!(vminq_u8(a, b))
    }
    #[inline(always)]
    fn cmpeq8(self, a: R, b: R) -> R {
        neon!(vceqq_u8(a, b))
    }
    #[inline(always)]
    fn avgu8(self, a: R, b: R) -> R {
        neon!(vrhaddq_u8(a, b))
    }
    #[inline(always)]
    fn movemask8(self, a: R) -> u32 {
        const BIT: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        // SAFETY: 16 readable bytes.
        unsafe {
            let bits = vandq_u8(vcltzq_s8(i8s(a)), vld1q_u8(BIT.as_ptr()));
            let p = vpaddq_u8(bits, bits);
            let p = vpaddq_u8(p, p);
            let p = vpaddq_u8(p, p);
            vgetq_lane_u16::<0>(u16s(p)) as u32
        }
    }
    #[inline(always)]
    fn unpacklo8(self, a: R, b: R) -> R {
        neon!(vzip1q_u8(a, b))
    }
    #[inline(always)]
    fn unpackhi8(self, a: R, b: R) -> R {
        neon!(vzip2q_u8(a, b))
    }
    #[inline(always)]
    fn packus16(self, a: R, b: R) -> R {
        neon!(vqmovun_high_s16(vqmovun_s16(i16s(a)), i16s(b)))
    }

    // ---- 16-bit lanes
    #[inline(always)]
    fn add16(self, a: R, b: R) -> R {
        neon!(vaddq_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn sub16(self, a: R, b: R) -> R {
        neon!(vsubq_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn subus16(self, a: R, b: R) -> R {
        neon!(vqsubq_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn mullo16(self, a: R, b: R) -> R {
        neon!(vmulq_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn mulhiu16(self, a: R, b: R) -> R {
        let (x, y) = (u16s(a), u16s(b));
        // SAFETY: see neon!.
        unsafe {
            let lo = vmull_u16(vget_low_u16(x), vget_low_u16(y));
            let hi = vmull_high_u16(x, y);
            vuzp2q_u16(u16s(lo.r()), u16s(hi.r())).r()
        }
    }
    #[inline(always)]
    fn mins16(self, a: R, b: R) -> R {
        neon!(vminq_s16(i16s(a), i16s(b))).r()
    }
    #[inline(always)]
    fn maxs16(self, a: R, b: R) -> R {
        neon!(vmaxq_s16(i16s(a), i16s(b))).r()
    }
    #[inline(always)]
    fn cmpeq16(self, a: R, b: R) -> R {
        neon!(vceqq_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn cmpgts16(self, a: R, b: R) -> R {
        neon!(vcgtq_s16(i16s(a), i16s(b))).r()
    }
    #[inline(always)]
    fn srl16<const N: i32>(self, a: R) -> R {
        if (N as u32) >= 16 {
            self.zero()
        } else {
            neon!(vshlq_u16(u16s(a), vdupq_n_s16(-(N as i16)))).r()
        }
    }
    #[inline(always)]
    fn sll16<const N: i32>(self, a: R) -> R {
        if (N as u32) >= 16 {
            self.zero()
        } else {
            neon!(vshlq_u16(u16s(a), vdupq_n_s16(N as i16))).r()
        }
    }
    #[inline(always)]
    fn shuf16<const IMM: i32>(self, a: R) -> R {
        tbl(a, const { &shuf16_index(IMM) })
    }
    #[inline(always)]
    fn unpacklo16(self, a: R, b: R) -> R {
        neon!(vzip1q_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn unpackhi16(self, a: R, b: R) -> R {
        neon!(vzip2q_u16(u16s(a), u16s(b))).r()
    }
    #[inline(always)]
    fn packs32(self, a: R, b: R) -> R {
        neon!(vqmovn_high_s32(vqmovn_s32(i32s(a)), i32s(b))).r()
    }
    #[inline(always)]
    fn madd16(self, a: R, b: R) -> R {
        let (x, y) = (i16s(a), i16s(b));
        // SAFETY: see neon!.
        unsafe {
            let lo = vmull_s16(vget_low_s16(x), vget_low_s16(y));
            let hi = vmull_high_s16(x, y);
            vpaddq_s32(lo, hi).r()
        }
    }

    // ---- 32-bit lanes
    #[inline(always)]
    fn add32(self, a: R, b: R) -> R {
        neon!(vaddq_u32(u32s(a), u32s(b))).r()
    }
    #[inline(always)]
    fn sub32(self, a: R, b: R) -> R {
        neon!(vsubq_u32(u32s(a), u32s(b))).r()
    }
    #[inline(always)]
    fn cmpeq32(self, a: R, b: R) -> R {
        neon!(vceqq_u32(u32s(a), u32s(b))).r()
    }
    #[inline(always)]
    fn cmpgts32(self, a: R, b: R) -> R {
        neon!(vcgtq_s32(i32s(a), i32s(b))).r()
    }
    #[inline(always)]
    fn movemask32(self, a: R) -> u32 {
        const BIT: [u32; 4] = [1, 2, 4, 8];
        // SAFETY: 16 readable bytes.
        unsafe { vaddvq_u32(vandq_u32(vcltzq_s32(i32s(a)), vld1q_u32(BIT.as_ptr()))) }
    }
    #[inline(always)]
    fn shuf32<const IMM: i32>(self, a: R) -> R {
        tbl(a, const { &shuf32_index(IMM) })
    }
    #[inline(always)]
    fn shufps<const IMM: i32>(self, a: R, b: R) -> R {
        neon!(vcombine_u8(vget_low_u8(self.shuf32::<IMM>(a)), vget_high_u8(self.shuf32::<IMM>(b))))
    }
    #[inline(always)]
    fn srl32<const N: i32>(self, a: R) -> R {
        if (N as u32) >= 32 {
            self.zero()
        } else {
            neon!(vshlq_u32(u32s(a), vdupq_n_s32(-N))).r()
        }
    }
    #[inline(always)]
    fn sll32<const N: i32>(self, a: R) -> R {
        if (N as u32) >= 32 {
            self.zero()
        } else {
            neon!(vshlq_u32(u32s(a), vdupq_n_s32(N))).r()
        }
    }
    #[inline(always)]
    fn sra32<const N: i32>(self, a: R) -> R {
        neon!(vshlq_s32(i32s(a), vdupq_n_s32(-((N as u32).min(31) as i32)))).r()
    }
    #[inline(always)]
    fn bsll<const N: i32>(self, a: R) -> R {
        tbl(a, const { &bsll_index(N) })
    }
    #[inline(always)]
    fn bsrl<const N: i32>(self, a: R) -> R {
        tbl(a, const { &bsrl_index(N) })
    }

    // ---- bits
    #[inline(always)]
    fn and(self, a: R, b: R) -> R {
        neon!(vandq_u8(a, b))
    }
    #[inline(always)]
    fn andnot(self, a: R, b: R) -> R {
        neon!(vbicq_u8(b, a))
    }
    #[inline(always)]
    fn or(self, a: R, b: R) -> R {
        neon!(vorrq_u8(a, b))
    }
    #[inline(always)]
    fn xor(self, a: R, b: R) -> R {
        neon!(veorq_u8(a, b))
    }

    // ---- f32 lanes
    #[inline(always)]
    fn cvt_i32_f32(self, a: R) -> R {
        neon!(vcvtq_f32_s32(i32s(a))).r()
    }
    #[inline(always)]
    fn cvtt_f32_i32(self, a: R) -> R {
        let x = f32s(a);
        // (too negative saturates to what SSE gives anyway)
        neon!(vbslq_s32(vcltq_f32(x, vdupq_n_f32(TWO31F)), vcvtq_s32_f32(x), vdupq_n_s32(i32::MIN))).r()
    }
    #[inline(always)]
    fn cvtr_f32_i32(self, a: R) -> R {
        let x = f32s(a);
        neon!(vbslq_s32(vcltq_f32(x, vdupq_n_f32(TWO31F)), vcvtnq_s32_f32(x), vdupq_n_s32(i32::MIN))).r()
    }
    #[inline(always)]
    fn addf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        result32(x, y, neon!(vaddq_f32(x, y)))
    }
    #[inline(always)]
    fn subf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        result32(x, y, neon!(vsubq_f32(x, y)))
    }
    #[inline(always)]
    fn mulf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        result32(x, y, neon!(vmulq_f32(x, y)))
    }
    #[inline(always)]
    fn divf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        result32(x, y, neon!(vdivq_f32(x, y)))
    }

    // ---- f64 lanes
    #[inline(always)]
    fn splatd(self, v: f64) -> R {
        neon!(vdupq_n_f64(v)).r()
    }
    #[inline(always)]
    fn addd(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        result64(x, y, neon!(vaddq_f64(x, y)))
    }
    #[inline(always)]
    fn subd(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        result64(x, y, neon!(vsubq_f64(x, y)))
    }
    #[inline(always)]
    fn muld(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        result64(x, y, neon!(vmulq_f64(x, y)))
    }
    #[inline(always)]
    fn divd(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        result64(x, y, neon!(vdivq_f64(x, y)))
    }
    #[inline(always)]
    fn sqrtd(self, a: R) -> R {
        let x = f64s(a);
        result64(x, x, neon!(vsqrtq_f64(x)))
    }
    #[inline(always)]
    fn maxd(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        neon!(vbslq_f64(vcgtq_f64(x, y), x, y)).r()
    }
    #[inline(always)]
    fn mind(self, a: R, b: R) -> R {
        let (x, y) = (f64s(a), f64s(b));
        neon!(vbslq_f64(vcltq_f64(x, y), x, y)).r()
    }
    #[inline(always)]
    fn cmpltd(self, a: R, b: R) -> R {
        neon!(vcltq_f64(f64s(a), f64s(b))).r()
    }
    #[inline(always)]
    fn cvt_i32_f64(self, a: R) -> R {
        neon!(vcvtq_f64_s64(vmovl_s32(vget_low_s32(i32s(a))))).r()
    }
    #[inline(always)]
    fn cvtt_f64_i32(self, a: R) -> R {
        let x = f64s(a);
        // SAFETY: see neon!.
        unsafe {
            // (too negative saturates twice to what SSE gives anyway)
            let v = vqmovn_s64(vcvtq_s64_f64(x));
            let ok = vmovn_u64(vcltq_f64(x, vdupq_n_f64(TWO31D)));
            vcombine_s32(vbsl_s32(ok, v, vdup_n_s32(i32::MIN)), vdup_n_s32(0)).r()
        }
    }
    #[inline(always)]
    fn store_i32_half(self, b: &mut [i32], v: R) {
        assert!(b.len() >= 2, "simd: slice too short");
        // SAFETY: 8 writable bytes (checked).
        unsafe { vst1_u8(b.as_mut_ptr() as *mut u8, vget_low_u8(v)) }
    }

    // R6-jpeg, R6-xf
    #[inline(always)]
    fn sra16<const N: i32>(self, a: R) -> R {
        neon!(vshlq_s16(i16s(a), vdupq_n_s16(-((N as u32).min(15) as i16)))).r()
    }
    #[inline(always)]
    fn mulhis16(self, a: R, b: R) -> R {
        let (x, y) = (i16s(a), i16s(b));
        // SAFETY: see neon!.
        unsafe {
            let lo = vmull_s16(vget_low_s16(x), vget_low_s16(y));
            let hi = vmull_high_s16(x, y);
            vuzp2q_u16(u16s(lo.r()), u16s(hi.r())).r()
        }
    }
    #[inline(always)]
    fn packs16(self, a: R, b: R) -> R {
        neon!(vqmovn_high_s16(vqmovn_s16(i16s(a)), i16s(b))).r()
    }
    #[inline(always)]
    fn load_i16(self, b: &[i16]) -> R {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { vld1q_u8(b.as_ptr() as *const u8) }
    }
    #[inline(always)]
    fn store_i16(self, b: &mut [i16], v: R) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { vst1q_u8(b.as_mut_ptr() as *mut u8, v) }
    }
    #[inline(always)]
    fn store_lanes2(self, b: &mut [u8], lo: R, hi: R) {
        chk(b, 32);
        // SAFETY: 32 writable bytes (checked).
        unsafe {
            vst1q_u8(b.as_mut_ptr(), lo);
            vst1q_u8(b.as_mut_ptr().add(16), hi);
        }
    }
    #[inline(always)]
    fn load_lanes2(self, b: &[u8]) -> (R, R) {
        chk(b, 32);
        // SAFETY: 32 readable bytes (checked).
        unsafe { (vld1q_u8(b.as_ptr()), vld1q_u8(b.as_ptr().add(16))) }
    }
    #[inline(always)]
    fn load_i16_lanes(self, a: &[i16], _b: &[i16]) -> R {
        self.load_i16(a)
    }
    #[inline(always)]
    fn store_raw_lanes(self, a: &mut [u8], _b: &mut [u8], v: R) {
        self.store_raw(a, v);
    }
    #[inline(always)]
    fn sadu8(self, a: R, b: R) -> R {
        neon!(vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vabdq_u8(a, b))))).r()
    }
    #[inline(always)]
    fn add64(self, a: R, b: R) -> R {
        neon!(vaddq_u64(u64s(a), u64s(b))).r()
    }

    // R6-icc
    #[inline(always)]
    fn maxf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        neon!(vbslq_f32(vcgtq_f32(x, y), x, y)).r()
    }
    #[inline(always)]
    fn minf(self, a: R, b: R) -> R {
        let (x, y) = (f32s(a), f32s(b));
        neon!(vbslq_f32(vcltq_f32(x, y), x, y)).r()
    }
    #[inline(always)]
    fn sqrtf(self, a: R) -> R {
        let x = f32s(a);
        result32(x, x, neon!(vsqrtq_f32(x)))
    }

    // r9-av1
    #[inline(always)]
    fn load_u16(self, b: &[u16]) -> R {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { vld1q_u8(b.as_ptr() as *const u8) }
    }
    #[inline(always)]
    fn store_u16(self, b: &mut [u16], v: R) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { vst1q_u8(b.as_mut_ptr() as *mut u8, v) }
    }
    #[inline(always)]
    fn load_u16_lo(self, b: &[u16]) -> R {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 readable bytes (checked).
        unsafe { vcombine_u8(vld1_u8(b.as_ptr() as *const u8), vdup_n_u8(0)) }
    }
    #[inline(always)]
    fn store_u16_lo(self, b: &mut [u16], v: R) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 writable bytes (checked).
        unsafe { vst1_u8(b.as_mut_ptr() as *mut u8, vget_low_u8(v)) }
    }
    #[inline(always)]
    fn srl16x(self, a: R, n: u32) -> R {
        if n >= 16 {
            self.zero()
        } else {
            neon!(vshlq_u16(u16s(a), vdupq_n_s16(-(n as i16)))).r()
        }
    }

    // R9-jxl
    #[inline(always)]
    fn load_f32(self, b: &[f32]) -> R {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { vld1q_u8(b.as_ptr() as *const u8) }
    }
    #[inline(always)]
    fn store_f32(self, b: &mut [f32], v: R) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { vst1q_u8(b.as_mut_ptr() as *mut u8, v) }
    }

    // R9-webp
    #[inline(always)]
    fn adds8(self, a: R, b: R) -> R {
        neon!(vqaddq_s8(i8s(a), i8s(b))).r()
    }
    #[inline(always)]
    fn subs8(self, a: R, b: R) -> R {
        neon!(vqsubq_s8(i8s(a), i8s(b))).r()
    }

    // r9-dds
    #[inline(always)]
    fn cmpltf(self, a: R, b: R) -> R {
        neon!(vcltq_f32(f32s(a), f32s(b))).r()
    }

    // jxl-speed
    #[inline(always)]
    fn load_f32_lanes(self, a: &[f32], _b: &[f32]) -> R {
        self.load_f32(a)
    }
    #[inline(always)]
    fn store_f32_lanes(self, a: &mut [f32], _b: &mut [f32], v: R) {
        self.store_f32(a, v);
    }

    // perf-blend
    #[inline(always)]
    fn packssdw(self, a: R, b: R) -> R {
        self.packs32(a, b)
    }
    #[inline(always)]
    fn packuswb_px(self, v: R) -> R {
        // SAFETY: see neon!.
        unsafe {
            let p = vqmovun_s16(i16s(v));
            vcombine_u8(p, p)
        }
    }
    #[inline(always)]
    fn ptest_raw(self, a: R, b: R) -> u32 {
        let (a, b) = (neon!(vgetq_lane_u64::<0>(u64s(a))), neon!(vgetq_lane_u64::<0>(u64s(b))));
        if a & b == 0 {
            1
        } else if !a & b == 0 {
            2
        } else {
            0
        }
    }
    #[inline(always)]
    fn ptestz_raw(self, a: R, b: R) -> bool {
        neon!(vgetq_lane_u64::<0>(u64s(self.and(a, b)))) == 0
    }
}
