// simd -- SSE2 / AVX2 registers behind one safe interface, so a kernel is written once and
// runs on both instruction sets with bit-identical results (the assembly's BMV = 0 / 1
// macros)
//
//   trait Simd       a register type V and its operations, named after the SSE2 instructions;
//                    Sse2 (128-bit registers) and Avx2 (256-bit registers used as two
//                    independent 128-bit lanes: every operation works per 128-bit lane, so
//                    lane k computes exactly what SSE2 computes for its data)
//   trait Kernel     a computation generic over Simd (run::<S>)
//   dispatch(k)      runs k with Avx2 when rt::cpu reports CPU_AVX2, else with Sse2
//   dispatch_sse2(k) always Sse2 (tests)
//   has_avx2()
//
// Pixel helpers work on "pixel lanes": the 4 bytes of a pixel zero-extended to 4 u16 lanes,
// PX = 2 (SSE2) / 4 (AVX2) pixels per register, pixels 2k and 2k + 1 in 128-bit lane k.
//
// One of the four crates that may contain `unsafe` (rt, simd, plat, plugin).  Every
// intrinsic is an unsafe call (core::arch requires the feature on the calling function, even
// SSE2, the x86-64 baseline: sse!); Avx2 intrinsics are only reached through an Avx2 value,
// which exists only after the CPU check (Avx2::new) -- the precondition of all of them.
// Memory helpers check the slice lengths (panic, never an access outside the slice).
// Kernels must not put closures or function pointers between dispatch and the intrinsics
// (compiled outside the AVX2 context, every intrinsic would become a call); build_rs.py
// refuses compute crates with out-of-line core_arch functions.
#![no_std]

#[cfg(target_arch = "x86_64")]
mod native;
#[cfg(target_arch = "x86_64")]
pub use native::{Sse2, Avx2};
mod scalar;
pub use scalar::{Scalar, Scalar256};
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub use wasm::{Wasm128, Wasm128 as Sse2};
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub use scalar::Scalar256 as Avx2;
#[cfg(not(any(target_arch = "x86_64", all(target_arch = "wasm32", target_feature = "simd128"))))]
pub use scalar::{Scalar as Sse2, Scalar256 as Avx2};

pub use rt::cpu::CPU_AVX2;

/// AVX2 kernels may be used (CPU and OS support, not disabled by `--simd sse2`)
#[inline]
pub fn has_avx2() -> bool {
    rt::cpu::features() & CPU_AVX2 != 0
}

/// a computation written once for every instruction set
pub trait Kernel {
    type Out;
    fn run<S: Simd>(self, s: S) -> Self::Out;
}

/// run k with the best instruction set of this CPU
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn dispatch<K: Kernel>(k: K) -> K::Out {
    match Avx2::new() {
        // SAFETY: Avx2::new succeeded, so the CPU and the OS support AVX2.
        Some(a) => unsafe { run_avx2(k, a) },
        None => k.run(Sse2),
    }
}

/// run k with SSE2 (the baseline; tests compare it with dispatch)
#[inline]
pub fn dispatch_sse2<K: Kernel>(k: K) -> K::Out {
    k.run(Sse2)
}

/// Run a kernel with SIMD128 when enabled by the WASM bundle, otherwise scalar registers.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn dispatch<K: Kernel>(k: K) -> K::Out { k.run(Sse2) }

// the kernel compiled with AVX2 enabled (its #[inline(always)] code is inlined here)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn run_avx2<K: Kernel>(k: K, a: Avx2) -> K::Out {
    k.run(a)
}

/// operations of one register type; integer lanes are named by their width (8 / 16 / 32),
/// signed operations end in s, unsigned in u
pub trait Simd: Copy {
    type V: Copy;
    /// bytes per register
    const BYTES: usize;
    /// pixels per register in pixel lanes
    const PX: usize;

    fn zero(self) -> Self::V;
    fn splat8(self, v: u8) -> Self::V;
    fn splat16(self, v: u16) -> Self::V;
    fn splat32(self, v: u32) -> Self::V;
    fn splat64(self, v: u64) -> Self::V;
    fn splatf(self, v: f32) -> Self::V;

    // ---- memory
    /// BYTES bytes
    fn load(self, b: &[u8]) -> Self::V;
    fn store(self, b: &mut [u8], v: Self::V);
    /// 4 * PX bytes (PX pixels) zero-extended to pixel lanes
    fn load_px(self, b: &[u8]) -> Self::V;
    /// pixel lanes saturated to bytes (packuswb), 4 * PX bytes
    fn store_px(self, b: &mut [u8], v: Self::V);
    /// PX bytes, byte i to the 4 u16 lanes of pixel i
    fn bcast_px(self, b: &[u8]) -> Self::V;
    /// 4 * PX bytes as they are ("raw" pixels: SSE2 the low 8 bytes, AVX2 the low 16)
    fn load_raw(self, b: &[u8]) -> Self::V;
    fn store_raw(self, b: &mut [u8], v: Self::V);
    /// raw pixels to pixel lanes
    fn widen_px(self, v: Self::V) -> Self::V;
    /// 4 bytes into the low dword of every 128-bit lane (BYTES / 4 bytes), the rest 0
    fn load_lane32(self, b: &[u8]) -> Self::V;
    /// BYTES / 4 i32 values
    fn load_i32(self, b: &[i32]) -> Self::V;
    fn store_i32(self, b: &mut [i32], v: Self::V);
    /// movd: v in dword 0, the rest 0 / dword 0 of v
    fn from_i32(self, v: i32) -> Self::V;
    fn to_i32(self, v: Self::V) -> i32;

    // ---- 8-bit lanes
    fn add8(self, a: Self::V, b: Self::V) -> Self::V;
    fn sub8(self, a: Self::V, b: Self::V) -> Self::V;
    fn addus8(self, a: Self::V, b: Self::V) -> Self::V;
    fn subus8(self, a: Self::V, b: Self::V) -> Self::V;
    fn maxu8(self, a: Self::V, b: Self::V) -> Self::V;
    fn minu8(self, a: Self::V, b: Self::V) -> Self::V;
    fn cmpeq8(self, a: Self::V, b: Self::V) -> Self::V;
    fn avgu8(self, a: Self::V, b: Self::V) -> Self::V;
    /// bit i = top bit of byte i (BYTES bits)
    fn movemask8(self, a: Self::V) -> u32;
    fn unpacklo8(self, a: Self::V, b: Self::V) -> Self::V;
    fn unpackhi8(self, a: Self::V, b: Self::V) -> Self::V;
    fn packus16(self, a: Self::V, b: Self::V) -> Self::V;

    // ---- 16-bit lanes
    fn add16(self, a: Self::V, b: Self::V) -> Self::V;
    fn sub16(self, a: Self::V, b: Self::V) -> Self::V;
    fn subus16(self, a: Self::V, b: Self::V) -> Self::V;
    fn mullo16(self, a: Self::V, b: Self::V) -> Self::V;
    fn mulhiu16(self, a: Self::V, b: Self::V) -> Self::V;
    fn mins16(self, a: Self::V, b: Self::V) -> Self::V;
    fn maxs16(self, a: Self::V, b: Self::V) -> Self::V;
    fn cmpeq16(self, a: Self::V, b: Self::V) -> Self::V;
    fn cmpgts16(self, a: Self::V, b: Self::V) -> Self::V;
    fn srl16<const N: i32>(self, a: Self::V) -> Self::V;
    fn sll16<const N: i32>(self, a: Self::V) -> Self::V;
    /// pshuflw + pshufhw with the same immediate (per 64-bit half)
    fn shuf16<const IMM: i32>(self, a: Self::V) -> Self::V;
    fn unpacklo16(self, a: Self::V, b: Self::V) -> Self::V;
    fn unpackhi16(self, a: Self::V, b: Self::V) -> Self::V;
    fn packs32(self, a: Self::V, b: Self::V) -> Self::V;
    /// pmaddwd: i32 lanes = a0 * b0 + a1 * b1 of the signed 16-bit pairs
    fn madd16(self, a: Self::V, b: Self::V) -> Self::V;

    // ---- 32-bit lanes
    fn add32(self, a: Self::V, b: Self::V) -> Self::V;
    fn sub32(self, a: Self::V, b: Self::V) -> Self::V;
    fn cmpeq32(self, a: Self::V, b: Self::V) -> Self::V;
    fn cmpgts32(self, a: Self::V, b: Self::V) -> Self::V;
    /// bit i = top bit of dword i (BYTES / 4 bits)
    fn movemask32(self, a: Self::V) -> u32;
    /// pshufd (per 128-bit lane)
    fn shuf32<const IMM: i32>(self, a: Self::V) -> Self::V;
    /// shufps on integer registers: dwords 0, 1 from a, 2, 3 from b (per 128-bit lane)
    fn shufps<const IMM: i32>(self, a: Self::V, b: Self::V) -> Self::V;
    fn srl32<const N: i32>(self, a: Self::V) -> Self::V;
    fn sll32<const N: i32>(self, a: Self::V) -> Self::V;
    fn sra32<const N: i32>(self, a: Self::V) -> Self::V;
    /// pslldq / psrldq: byte shifts per 128-bit lane
    fn bsll<const N: i32>(self, a: Self::V) -> Self::V;
    fn bsrl<const N: i32>(self, a: Self::V) -> Self::V;

    // ---- bits
    fn and(self, a: Self::V, b: Self::V) -> Self::V;
    /// !a & b (pandn)
    fn andnot(self, a: Self::V, b: Self::V) -> Self::V;
    fn or(self, a: Self::V, b: Self::V) -> Self::V;
    fn xor(self, a: Self::V, b: Self::V) -> Self::V;

    // ---- f32 lanes
    fn cvt_i32_f32(self, a: Self::V) -> Self::V;
    /// truncating (0x80000000 for NaN / out of range)
    fn cvtt_f32_i32(self, a: Self::V) -> Self::V;
    /// rounding to nearest even (cvtps2dq)
    fn cvtr_f32_i32(self, a: Self::V) -> Self::V;
    fn addf(self, a: Self::V, b: Self::V) -> Self::V;
    fn subf(self, a: Self::V, b: Self::V) -> Self::V;
    fn mulf(self, a: Self::V, b: Self::V) -> Self::V;
    fn divf(self, a: Self::V, b: Self::V) -> Self::V;

    // ---- f64 lanes (BYTES / 8 of them); bit operations: and / andnot / or / xor
    fn splatd(self, v: f64) -> Self::V;
    fn addd(self, a: Self::V, b: Self::V) -> Self::V;
    fn subd(self, a: Self::V, b: Self::V) -> Self::V;
    fn muld(self, a: Self::V, b: Self::V) -> Self::V;
    fn divd(self, a: Self::V, b: Self::V) -> Self::V;
    fn sqrtd(self, a: Self::V) -> Self::V;
    /// maxpd / minpd: b when unordered
    fn maxd(self, a: Self::V, b: Self::V) -> Self::V;
    fn mind(self, a: Self::V, b: Self::V) -> Self::V;
    /// cmpltpd: all ones where a < b (false when unordered)
    fn cmpltd(self, a: Self::V, b: Self::V) -> Self::V;
    /// cvtdq2pd: the first BYTES / 8 i32 lanes to f64
    fn cvt_i32_f64(self, a: Self::V) -> Self::V;
    /// cvttpd2dq: f64 lanes to the first BYTES / 8 i32 lanes (truncating, 0x80000000 when
    /// out of range), the rest 0
    fn cvtt_f64_i32(self, a: Self::V) -> Self::V;
    /// the first BYTES / 8 i32 lanes
    fn store_i32_half(self, b: &mut [i32], v: Self::V);

    // R6-jpeg, R6-xf: the JPEG kernels (img_jpeg dsp)
    /// psraw: arithmetic right shift of the i16 lanes
    fn sra16<const N: i32>(self, a: Self::V) -> Self::V;
    /// pmulhw: high 16 bits of the signed 16-bit products
    fn mulhis16(self, a: Self::V, b: Self::V) -> Self::V;
    /// packsswb: i16 lanes of a, then b, saturated to i8
    fn packs16(self, a: Self::V, b: Self::V) -> Self::V;
    /// BYTES / 2 i16 values
    fn load_i16(self, b: &[i16]) -> Self::V;
    fn store_i16(self, b: &mut [i16], v: Self::V);
    /// 2 * BYTES bytes: per 128-bit lane k, lane k of lo then lane k of hi (SSE2: lo, hi;
    /// AVX2: vperm2i128) -- 2 x 4 pixels per lane in pixel order
    fn store_lanes2(self, b: &mut [u8], lo: Self::V, hi: Self::V);
    /// the inverse of store_lanes2: 2 * BYTES bytes -> (lo, hi)
    fn load_lanes2(self, b: &[u8]) -> (Self::V, Self::V);
    /// 8 i16 per 128-bit lane from separate sources: lane 0 from a, lane 1 from b (SSE2: a)
    fn load_i16_lanes(self, a: &[i16], b: &[i16]) -> Self::V;
    /// the low 8 bytes of every 128-bit lane to separate places: lane 0 to a, lane 1 to b
    /// (SSE2: a)
    fn store_raw_lanes(self, a: &mut [u8], b: &mut [u8], v: Self::V);
    /// (main agent) psadbw: per 8-byte group the sum of |a - b| (u8) in its u64 lane
    fn sadu8(self, a: Self::V, b: Self::V) -> Self::V;
    /// (main agent) u64 lanes
    fn add64(self, a: Self::V, b: Self::V) -> Self::V;

    // R6-icc: the ICC transform kernels (img_core icc_xform)
    /// maxps / minps: b when unordered
    fn maxf(self, a: Self::V, b: Self::V) -> Self::V;
    fn minf(self, a: Self::V, b: Self::V) -> Self::V;
    /// sqrtps
    fn sqrtf(self, a: Self::V) -> Self::V;

    // r9-av1: the CDEF kernel (img_av1 cdef)
    /// BYTES / 2 u16 values
    fn load_u16(self, b: &[u16]) -> Self::V;
    fn store_u16(self, b: &mut [u16], v: Self::V);
    /// movq: 4 u16 values into the low 64 bits (the rest 0) / the low 64 bits to 4 u16 values
    fn load_u16_lo(self, b: &[u16]) -> Self::V;
    fn store_u16_lo(self, b: &mut [u16], v: Self::V);
    /// psrlw with a register count: logical right shift of the 16-bit lanes by n (n > 15: 0)
    fn srl16x(self, a: Self::V, n: u32) -> Self::V;

    // R9-jxl: the JPEG XL render kernels (img_jxl)
    /// BYTES / 4 f32 values
    fn load_f32(self, b: &[f32]) -> Self::V;
    fn store_f32(self, b: &mut [f32], v: Self::V);

    // R9-webp: the VP8 loop filter (img_webp)
    /// paddsb: signed saturating add of the i8 lanes
    fn adds8(self, a: Self::V, b: Self::V) -> Self::V;
    /// psubsb: signed saturating subtract of the i8 lanes
    fn subs8(self, a: Self::V, b: Self::V) -> Self::V;

    // r9-dds: the BC encoders' pixel-set helpers (img_dds enc)
    /// cmpltps: all ones where a < b (false when unordered)
    fn cmpltf(self, a: Self::V, b: Self::V) -> Self::V;

    // jxl-speed: the JPEG XL writer's transforms (img_jxl dctk: 4 x 4 transposes per lane)
    /// 4 f32 per 128-bit lane from separate sources: lane 0 from a, lane 1 from b (SSE2: a)
    fn load_f32_lanes(self, a: &[f32], b: &[f32]) -> Self::V;
    /// the 4 f32 of every 128-bit lane to separate places: lane 0 to a, lane 1 to b (SSE2: a)
    fn store_f32_lanes(self, a: &mut [f32], b: &mut [f32], v: Self::V);

    // perf-blend: the blend-mode kernels (gfx blendmode).  core::arch writes the pack
    // intrinsics as generic smax / smin + shuffle; with AVX2 (SSE4.1's pmaxsd / pminsd
    // available) LLVM keeps the clamps in front of the pack: 4 (packs32) / 2 (packus16) more
    // instructions per pack.  On AVX2 these two are exactly one pack instruction (inline
    // assembly); SSE2 uses the intrinsics (packs32 is one instruction there, packus16 keeps
    // pmaxsw / pminsw: legacy-encoded inline assembly would hurt AVX2 code calling Sse2).
    /// packs32 (packssdw), on AVX2 without the clamps
    fn packssdw(self, a: Self::V, b: Self::V) -> Self::V;
    /// pixel lanes to raw pixels: packuswb (+ vpermq 0x08 on AVX2) as in store_px, on AVX2
    /// without the clamps; the inverse of widen_px
    fn packuswb_px(self, v: Self::V) -> Self::V;
    /// ptest on the raw-pixel bytes (SSE2: the low 8, AVX2: the low 16): 1 when a & b == 0
    /// (ZF), else 2 when !a & b == 0 (CF), else 0.  AVX2: vptest + jz + jnc, the CF case
    /// falls through (inline assembly: core::arch writes ptest as or-reductions, and LLVM
    /// then often builds shuffle / reduce sequences or loses the CF test); branch on the
    /// result, the branches thread through
    fn ptest_raw(self, a: Self::V, b: Self::V) -> u32;
    /// ptest ZF on the raw-pixel bytes: a & b == 0 (AVX2: vptest + jz)
    fn ptestz_raw(self, a: Self::V, b: Self::V) -> bool;
}
