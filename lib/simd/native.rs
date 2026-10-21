// Native SSE2/AVX2 backend, kept byte-for-byte in arithmetic and instruction order.
use core::arch::x86_64::*;
use crate::{Simd, has_avx2};

// ---------------------------------------------------------------- SSE2

/// SSE2: 128-bit registers (the x86-64 baseline)
#[derive(Clone, Copy)]
pub struct Sse2;

#[inline(always)]
fn chk(b: &[u8], n: usize) {
    assert!(b.len() >= n, "simd: slice too short");
}

// SAFETY (every sse! block): SSE2 is part of the x86-64 baseline, enabled on every target
// this code is built for; the intrinsics only need the feature (no memory access).
macro_rules! sse {
    ($e:expr) => {
        unsafe { $e }
    };
}

impl Simd for Sse2 {
    type V = __m128i;
    const BYTES: usize = 16;
    const PX: usize = 2;

    #[inline(always)]
    fn zero(self) -> __m128i {
        sse!(_mm_setzero_si128())
    }
    #[inline(always)]
    fn splat8(self, v: u8) -> __m128i {
        sse!(_mm_set1_epi8(v as i8))
    }
    #[inline(always)]
    fn splat16(self, v: u16) -> __m128i {
        sse!(_mm_set1_epi16(v as i16))
    }
    #[inline(always)]
    fn splat32(self, v: u32) -> __m128i {
        sse!(_mm_set1_epi32(v as i32))
    }
    #[inline(always)]
    fn splat64(self, v: u64) -> __m128i {
        sse!(_mm_set1_epi64x(v as i64))
    }
    #[inline(always)]
    fn splatf(self, v: f32) -> __m128i {
        sse!(_mm_castps_si128(_mm_set1_ps(v)))
    }

    #[inline(always)]
    fn load(self, b: &[u8]) -> __m128i {
        chk(b, 16);
        // SAFETY: 16 readable bytes (checked); unaligned load.
        unsafe { _mm_loadu_si128(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store(self, b: &mut [u8], v: __m128i) {
        chk(b, 16);
        // SAFETY: 16 writable bytes (checked); unaligned store.
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn load_px(self, b: &[u8]) -> __m128i {
        chk(b, 8);
        // SAFETY: 8 readable bytes (checked).
        let v = unsafe { _mm_loadl_epi64(b.as_ptr() as *const __m128i) };
        sse!(_mm_unpacklo_epi8(v, _mm_setzero_si128()))
    }
    #[inline(always)]
    fn store_px(self, b: &mut [u8], v: __m128i) {
        chk(b, 8);
        let p = sse!(_mm_packus_epi16(v, v));
        // SAFETY: 8 writable bytes (checked).
        unsafe { _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, p) }
    }
    #[inline(always)]
    fn load_raw(self, b: &[u8]) -> __m128i {
        chk(b, 8);
        // SAFETY: 8 readable bytes (checked).
        unsafe { _mm_loadl_epi64(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store_raw(self, b: &mut [u8], v: __m128i) {
        chk(b, 8);
        // SAFETY: 8 writable bytes (checked).
        unsafe { _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn widen_px(self, v: __m128i) -> __m128i {
        sse!(_mm_unpacklo_epi8(v, _mm_setzero_si128()))
    }
    #[inline(always)]
    fn load_lane32(self, b: &[u8]) -> __m128i {
        chk(b, 4);
        sse!(_mm_cvtsi32_si128(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i32))
    }
    #[inline(always)]
    fn load_i32(self, b: &[i32]) -> __m128i {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { _mm_loadu_si128(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store_i32(self, b: &mut [i32], v: __m128i) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn from_i32(self, v: i32) -> __m128i {
        sse!(_mm_cvtsi32_si128(v))
    }
    #[inline(always)]
    fn to_i32(self, v: __m128i) -> i32 {
        sse!(_mm_cvtsi128_si32(v))
    }
    #[inline(always)]
    fn bcast_px(self, b: &[u8]) -> __m128i {
        chk(b, 2);
        let v = sse!(_mm_cvtsi32_si128(u16::from_le_bytes([b[0], b[1]]) as i32));
        let v = sse!(_mm_unpacklo_epi8(v, v));
        let v = sse!(_mm_unpacklo_epi16(v, v));
        sse!(_mm_unpacklo_epi8(v, _mm_setzero_si128()))
    }

    #[inline(always)]
    fn add8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_add_epi8(a, b))
    }
    #[inline(always)]
    fn sub8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_sub_epi8(a, b))
    }
    #[inline(always)]
    fn addus8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_adds_epu8(a, b))
    }
    #[inline(always)]
    fn subus8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_subs_epu8(a, b))
    }
    #[inline(always)]
    fn maxu8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_max_epu8(a, b))
    }
    #[inline(always)]
    fn minu8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_min_epu8(a, b))
    }
    #[inline(always)]
    fn cmpeq8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_cmpeq_epi8(a, b))
    }
    #[inline(always)]
    fn avgu8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_avg_epu8(a, b))
    }
    #[inline(always)]
    fn movemask8(self, a: __m128i) -> u32 {
        sse!(_mm_movemask_epi8(a) as u32)
    }
    #[inline(always)]
    fn unpacklo8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_unpacklo_epi8(a, b))
    }
    #[inline(always)]
    fn unpackhi8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_unpackhi_epi8(a, b))
    }
    #[inline(always)]
    fn packus16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_packus_epi16(a, b))
    }

    #[inline(always)]
    fn add16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_add_epi16(a, b))
    }
    #[inline(always)]
    fn sub16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_sub_epi16(a, b))
    }
    #[inline(always)]
    fn subus16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_subs_epu16(a, b))
    }
    #[inline(always)]
    fn mullo16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_mullo_epi16(a, b))
    }
    #[inline(always)]
    fn mulhiu16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_mulhi_epu16(a, b))
    }
    #[inline(always)]
    fn mins16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_min_epi16(a, b))
    }
    #[inline(always)]
    fn maxs16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_max_epi16(a, b))
    }
    #[inline(always)]
    fn cmpeq16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_cmpeq_epi16(a, b))
    }
    #[inline(always)]
    fn cmpgts16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_cmpgt_epi16(a, b))
    }
    #[inline(always)]
    fn srl16<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_srli_epi16::<N>(a))
    }
    #[inline(always)]
    fn sll16<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_slli_epi16::<N>(a))
    }
    #[inline(always)]
    fn shuf16<const IMM: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_shufflehi_epi16::<IMM>(_mm_shufflelo_epi16::<IMM>(a)))
    }
    #[inline(always)]
    fn unpacklo16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_unpacklo_epi16(a, b))
    }
    #[inline(always)]
    fn unpackhi16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_unpackhi_epi16(a, b))
    }
    #[inline(always)]
    fn packs32(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_packs_epi32(a, b))
    }
    #[inline(always)]
    fn madd16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_madd_epi16(a, b))
    }

    #[inline(always)]
    fn add32(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_add_epi32(a, b))
    }
    #[inline(always)]
    fn sub32(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_sub_epi32(a, b))
    }
    #[inline(always)]
    fn cmpeq32(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_cmpeq_epi32(a, b))
    }
    #[inline(always)]
    fn cmpgts32(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_cmpgt_epi32(a, b))
    }
    #[inline(always)]
    fn movemask32(self, a: __m128i) -> u32 {
        sse!(_mm_movemask_ps(_mm_castsi128_ps(a))) as u32
    }
    #[inline(always)]
    fn shuf32<const IMM: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_shuffle_epi32::<IMM>(a))
    }
    #[inline(always)]
    fn shufps<const IMM: i32>(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_shuffle_ps::<IMM>(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn srl32<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_srli_epi32::<N>(a))
    }
    #[inline(always)]
    fn sll32<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_slli_epi32::<N>(a))
    }
    #[inline(always)]
    fn sra32<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_srai_epi32::<N>(a))
    }
    #[inline(always)]
    fn bsll<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_slli_si128::<N>(a))
    }
    #[inline(always)]
    fn bsrl<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_srli_si128::<N>(a))
    }

    #[inline(always)]
    fn and(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_and_si128(a, b))
    }
    #[inline(always)]
    fn andnot(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_andnot_si128(a, b))
    }
    #[inline(always)]
    fn or(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_or_si128(a, b))
    }
    #[inline(always)]
    fn xor(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_xor_si128(a, b))
    }

    #[inline(always)]
    fn cvt_i32_f32(self, a: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_cvtepi32_ps(a)))
    }
    #[inline(always)]
    fn cvtt_f32_i32(self, a: __m128i) -> __m128i {
        sse!(_mm_cvttps_epi32(_mm_castsi128_ps(a)))
    }
    #[inline(always)]
    fn cvtr_f32_i32(self, a: __m128i) -> __m128i {
        sse!(_mm_cvtps_epi32(_mm_castsi128_ps(a)))
    }
    #[inline(always)]
    fn addf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_add_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn subf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_sub_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn mulf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_mul_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn divf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_div_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }

    #[inline(always)]
    fn splatd(self, v: f64) -> __m128i {
        sse!(_mm_castpd_si128(_mm_set1_pd(v)))
    }
    #[inline(always)]
    fn addd(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_add_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn subd(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_sub_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn muld(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_mul_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn divd(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_div_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn sqrtd(self, a: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_sqrt_pd(_mm_castsi128_pd(a))))
    }
    #[inline(always)]
    fn maxd(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_max_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn mind(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_min_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn cmpltd(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_cmplt_pd(_mm_castsi128_pd(a), _mm_castsi128_pd(b))))
    }
    #[inline(always)]
    fn cvt_i32_f64(self, a: __m128i) -> __m128i {
        sse!(_mm_castpd_si128(_mm_cvtepi32_pd(a)))
    }
    #[inline(always)]
    fn cvtt_f64_i32(self, a: __m128i) -> __m128i {
        sse!(_mm_cvttpd_epi32(_mm_castsi128_pd(a)))
    }
    #[inline(always)]
    fn store_i32_half(self, b: &mut [i32], v: __m128i) {
        assert!(b.len() >= 2, "simd: slice too short");
        // SAFETY: 8 writable bytes (checked).
        unsafe { _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, v) }
    }

    // R6-jpeg, R6-xf
    #[inline(always)]
    fn sra16<const N: i32>(self, a: __m128i) -> __m128i {
        sse!(_mm_srai_epi16::<N>(a))
    }
    #[inline(always)]
    fn mulhis16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_mulhi_epi16(a, b))
    }
    #[inline(always)]
    fn packs16(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_packs_epi16(a, b))
    }
    #[inline(always)]
    fn load_i16(self, b: &[i16]) -> __m128i {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { _mm_loadu_si128(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store_i16(self, b: &mut [i16], v: __m128i) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn store_lanes2(self, b: &mut [u8], lo: __m128i, hi: __m128i) {
        chk(b, 32);
        // SAFETY: 32 writable bytes (checked).
        unsafe {
            _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, lo);
            _mm_storeu_si128(b.as_mut_ptr().add(16) as *mut __m128i, hi);
        }
    }
    #[inline(always)]
    fn load_lanes2(self, b: &[u8]) -> (__m128i, __m128i) {
        chk(b, 32);
        // SAFETY: 32 readable bytes (checked).
        unsafe {
            (_mm_loadu_si128(b.as_ptr() as *const __m128i), _mm_loadu_si128(b.as_ptr().add(16) as *const __m128i))
        }
    }
    #[inline(always)]
    fn load_i16_lanes(self, a: &[i16], _b: &[i16]) -> __m128i {
        self.load_i16(a)
    }
    #[inline(always)]
    fn store_raw_lanes(self, a: &mut [u8], _b: &mut [u8], v: __m128i) {
        self.store_raw(a, v)
    }
    #[inline(always)]
    fn sadu8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_sad_epu8(a, b))
    }
    #[inline(always)]
    fn add64(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_add_epi64(a, b))
    }

    // R6-icc
    #[inline(always)]
    fn maxf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_max_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn minf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_min_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }
    #[inline(always)]
    fn sqrtf(self, a: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_sqrt_ps(_mm_castsi128_ps(a))))
    }

    // r9-av1
    #[inline(always)]
    fn load_u16(self, b: &[u16]) -> __m128i {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { _mm_loadu_si128(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store_u16(self, b: &mut [u16], v: __m128i) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn load_u16_lo(self, b: &[u16]) -> __m128i {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 readable bytes (checked).
        unsafe { _mm_loadl_epi64(b.as_ptr() as *const __m128i) }
    }
    #[inline(always)]
    fn store_u16_lo(self, b: &mut [u16], v: __m128i) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 writable bytes (checked).
        unsafe { _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, v) }
    }
    #[inline(always)]
    fn srl16x(self, a: __m128i, n: u32) -> __m128i {
        sse!(_mm_srl_epi16(a, _mm_cvtsi32_si128(n as i32)))
    }

    // R9-jxl
    #[inline(always)]
    fn load_f32(self, b: &[f32]) -> __m128i {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 readable bytes (checked).
        unsafe { _mm_castps_si128(_mm_loadu_ps(b.as_ptr())) }
    }
    #[inline(always)]
    fn store_f32(self, b: &mut [f32], v: __m128i) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked).
        unsafe { _mm_storeu_ps(b.as_mut_ptr(), _mm_castsi128_ps(v)) }
    }

    // R9-webp
    #[inline(always)]
    fn adds8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_adds_epi8(a, b))
    }
    #[inline(always)]
    fn subs8(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_subs_epi8(a, b))
    }

    // r9-dds
    #[inline(always)]
    fn cmpltf(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_castps_si128(_mm_cmplt_ps(_mm_castsi128_ps(a), _mm_castsi128_ps(b))))
    }

    // jxl-speed
    #[inline(always)]
    fn load_f32_lanes(self, a: &[f32], _b: &[f32]) -> __m128i {
        self.load_f32(a)
    }
    #[inline(always)]
    fn store_f32_lanes(self, a: &mut [f32], _b: &mut [f32], v: __m128i) {
        self.store_f32(a, v)
    }

    // perf-blend (the intrinsics: see the trait)
    #[inline(always)]
    fn packssdw(self, a: __m128i, b: __m128i) -> __m128i {
        sse!(_mm_packs_epi32(a, b))
    }
    #[inline(always)]
    fn packuswb_px(self, v: __m128i) -> __m128i {
        sse!(_mm_packus_epi16(v, v))
    }
    #[inline(always)]
    fn ptest_raw(self, a: __m128i, b: __m128i) -> u32 {
        let z = sse!(_mm_movemask_epi8(_mm_cmpeq_epi8(_mm_and_si128(a, b), _mm_setzero_si128()))) & 0xFF == 0xFF;
        let c = sse!(_mm_movemask_epi8(_mm_cmpeq_epi8(_mm_andnot_si128(a, b), _mm_setzero_si128()))) & 0xFF == 0xFF;
        if z {
            1
        } else if c {
            2
        } else {
            0
        }
    }
    #[inline(always)]
    fn ptestz_raw(self, a: __m128i, b: __m128i) -> bool {
        sse!(_mm_movemask_epi8(_mm_cmpeq_epi8(_mm_and_si128(a, b), _mm_setzero_si128()))) & 0xFF == 0xFF
    }
}

// ---------------------------------------------------------------- AVX2

/// AVX2: 256-bit registers as two 128-bit lanes; only constructible when the CPU has AVX2
#[derive(Clone, Copy)]
pub struct Avx2(());

impl Avx2 {
    #[inline]
    pub fn new() -> Option<Avx2> {
        has_avx2().then_some(Avx2(()))
    }
}

// SAFETY (every unsafe block below unless stated otherwise): an Avx2 value exists only after
// Avx2::new found AVX2 support, which is the only precondition of these intrinsics.
macro_rules! avx {
    ($e:expr) => {
        unsafe { $e }
    };
}

impl Simd for Avx2 {
    type V = __m256i;
    const BYTES: usize = 32;
    const PX: usize = 4;

    #[inline(always)]
    fn zero(self) -> __m256i {
        avx!(_mm256_setzero_si256())
    }
    #[inline(always)]
    fn splat8(self, v: u8) -> __m256i {
        avx!(_mm256_set1_epi8(v as i8))
    }
    #[inline(always)]
    fn splat16(self, v: u16) -> __m256i {
        avx!(_mm256_set1_epi16(v as i16))
    }
    #[inline(always)]
    fn splat32(self, v: u32) -> __m256i {
        avx!(_mm256_set1_epi32(v as i32))
    }
    #[inline(always)]
    fn splat64(self, v: u64) -> __m256i {
        avx!(_mm256_set1_epi64x(v as i64))
    }
    #[inline(always)]
    fn splatf(self, v: f32) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_set1_ps(v)))
    }

    #[inline(always)]
    fn load(self, b: &[u8]) -> __m256i {
        chk(b, 32);
        // SAFETY: 32 readable bytes (checked), AVX2 present.
        unsafe { _mm256_loadu_si256(b.as_ptr() as *const __m256i) }
    }
    #[inline(always)]
    fn store(self, b: &mut [u8], v: __m256i) {
        chk(b, 32);
        // SAFETY: 32 writable bytes (checked), AVX2 present.
        unsafe { _mm256_storeu_si256(b.as_mut_ptr() as *mut __m256i, v) }
    }
    #[inline(always)]
    fn load_px(self, b: &[u8]) -> __m256i {
        chk(b, 16);
        // SAFETY: 16 readable bytes (checked), AVX2 present.
        unsafe { _mm256_cvtepu8_epi16(_mm_loadu_si128(b.as_ptr() as *const __m128i)) }
    }
    #[inline(always)]
    fn store_px(self, b: &mut [u8], v: __m256i) {
        chk(b, 16);
        // SAFETY: 16 writable bytes (checked), AVX2 present.
        unsafe {
            let p = _mm256_permute4x64_epi64::<0x08>(_mm256_packus_epi16(v, v));
            _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(p))
        }
    }
    #[inline(always)]
    fn load_raw(self, b: &[u8]) -> __m256i {
        chk(b, 16);
        // SAFETY: 16 readable bytes (checked), AVX2 present.
        unsafe { _mm256_castsi128_si256(_mm_loadu_si128(b.as_ptr() as *const __m128i)) }
    }
    #[inline(always)]
    fn store_raw(self, b: &mut [u8], v: __m256i) {
        chk(b, 16);
        // SAFETY: 16 writable bytes (checked), AVX2 present.
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(v)) }
    }
    #[inline(always)]
    fn widen_px(self, v: __m256i) -> __m256i {
        avx!(_mm256_cvtepu8_epi16(_mm256_castsi256_si128(v)))
    }
    #[inline(always)]
    fn load_lane32(self, b: &[u8]) -> __m256i {
        chk(b, 8);
        let d = |k: usize| u32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]) as i32;
        avx!(_mm256_setr_epi32(d(0), 0, 0, 0, d(4), 0, 0, 0))
    }
    #[inline(always)]
    fn load_i32(self, b: &[i32]) -> __m256i {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 32 readable bytes (checked), AVX2 present.
        unsafe { _mm256_loadu_si256(b.as_ptr() as *const __m256i) }
    }
    #[inline(always)]
    fn store_i32(self, b: &mut [i32], v: __m256i) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 32 writable bytes (checked), AVX2 present.
        unsafe { _mm256_storeu_si256(b.as_mut_ptr() as *mut __m256i, v) }
    }
    #[inline(always)]
    fn from_i32(self, v: i32) -> __m256i {
        avx!(_mm256_zextsi128_si256(_mm_cvtsi32_si128(v)))
    }
    #[inline(always)]
    fn to_i32(self, v: __m256i) -> i32 {
        avx!(_mm_cvtsi128_si32(_mm256_castsi256_si128(v)))
    }
    #[inline(always)]
    fn bcast_px(self, b: &[u8]) -> __m256i {
        chk(b, 4);
        let v = avx!(_mm_cvtsi32_si128(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i32));
        let v = avx!(_mm_unpacklo_epi8(v, v));
        let v = avx!(_mm_unpacklo_epi16(v, v));
        avx!(_mm256_cvtepu8_epi16(v))
    }

    #[inline(always)]
    fn add8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_add_epi8(a, b))
    }
    #[inline(always)]
    fn sub8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_sub_epi8(a, b))
    }
    #[inline(always)]
    fn addus8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_adds_epu8(a, b))
    }
    #[inline(always)]
    fn subus8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_subs_epu8(a, b))
    }
    #[inline(always)]
    fn maxu8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_max_epu8(a, b))
    }
    #[inline(always)]
    fn minu8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_min_epu8(a, b))
    }
    #[inline(always)]
    fn cmpeq8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_cmpeq_epi8(a, b))
    }
    #[inline(always)]
    fn avgu8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_avg_epu8(a, b))
    }
    #[inline(always)]
    fn movemask8(self, a: __m256i) -> u32 {
        avx!(_mm256_movemask_epi8(a)) as u32
    }
    #[inline(always)]
    fn unpacklo8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_unpacklo_epi8(a, b))
    }
    #[inline(always)]
    fn unpackhi8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_unpackhi_epi8(a, b))
    }
    #[inline(always)]
    fn packus16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_packus_epi16(a, b))
    }

    #[inline(always)]
    fn add16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_add_epi16(a, b))
    }
    #[inline(always)]
    fn sub16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_sub_epi16(a, b))
    }
    #[inline(always)]
    fn subus16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_subs_epu16(a, b))
    }
    #[inline(always)]
    fn mullo16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_mullo_epi16(a, b))
    }
    #[inline(always)]
    fn mulhiu16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_mulhi_epu16(a, b))
    }
    #[inline(always)]
    fn mins16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_min_epi16(a, b))
    }
    #[inline(always)]
    fn maxs16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_max_epi16(a, b))
    }
    #[inline(always)]
    fn cmpeq16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_cmpeq_epi16(a, b))
    }
    #[inline(always)]
    fn cmpgts16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_cmpgt_epi16(a, b))
    }
    #[inline(always)]
    fn srl16<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_srli_epi16::<N>(a))
    }
    #[inline(always)]
    fn sll16<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_slli_epi16::<N>(a))
    }
    #[inline(always)]
    fn shuf16<const IMM: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_shufflehi_epi16::<IMM>(_mm256_shufflelo_epi16::<IMM>(a)))
    }
    #[inline(always)]
    fn unpacklo16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_unpacklo_epi16(a, b))
    }
    #[inline(always)]
    fn unpackhi16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_unpackhi_epi16(a, b))
    }
    #[inline(always)]
    fn packs32(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_packs_epi32(a, b))
    }
    #[inline(always)]
    fn madd16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_madd_epi16(a, b))
    }

    #[inline(always)]
    fn add32(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_add_epi32(a, b))
    }
    #[inline(always)]
    fn sub32(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_sub_epi32(a, b))
    }
    #[inline(always)]
    fn cmpeq32(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_cmpeq_epi32(a, b))
    }
    #[inline(always)]
    fn cmpgts32(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_cmpgt_epi32(a, b))
    }
    #[inline(always)]
    fn movemask32(self, a: __m256i) -> u32 {
        avx!(_mm256_movemask_ps(_mm256_castsi256_ps(a))) as u32
    }
    #[inline(always)]
    fn shuf32<const IMM: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_shuffle_epi32::<IMM>(a))
    }
    #[inline(always)]
    fn shufps<const IMM: i32>(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_shuffle_ps::<IMM>(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn srl32<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_srli_epi32::<N>(a))
    }
    #[inline(always)]
    fn sll32<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_slli_epi32::<N>(a))
    }
    #[inline(always)]
    fn sra32<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_srai_epi32::<N>(a))
    }
    #[inline(always)]
    fn bsll<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_bslli_epi128::<N>(a))
    }
    #[inline(always)]
    fn bsrl<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_bsrli_epi128::<N>(a))
    }

    #[inline(always)]
    fn and(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_and_si256(a, b))
    }
    #[inline(always)]
    fn andnot(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_andnot_si256(a, b))
    }
    #[inline(always)]
    fn or(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_or_si256(a, b))
    }
    #[inline(always)]
    fn xor(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_xor_si256(a, b))
    }

    #[inline(always)]
    fn cvt_i32_f32(self, a: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_cvtepi32_ps(a)))
    }
    #[inline(always)]
    fn cvtt_f32_i32(self, a: __m256i) -> __m256i {
        avx!(_mm256_cvttps_epi32(_mm256_castsi256_ps(a)))
    }
    #[inline(always)]
    fn cvtr_f32_i32(self, a: __m256i) -> __m256i {
        avx!(_mm256_cvtps_epi32(_mm256_castsi256_ps(a)))
    }
    #[inline(always)]
    fn addf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_add_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn subf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_sub_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn mulf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_mul_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn divf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_div_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }

    #[inline(always)]
    fn splatd(self, v: f64) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_set1_pd(v)))
    }
    #[inline(always)]
    fn addd(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_add_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn subd(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_sub_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn muld(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_mul_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn divd(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_div_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn sqrtd(self, a: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_sqrt_pd(_mm256_castsi256_pd(a))))
    }
    #[inline(always)]
    fn maxd(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_max_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn mind(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_min_pd(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn cmpltd(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_cmp_pd::<_CMP_LT_OS>(_mm256_castsi256_pd(a), _mm256_castsi256_pd(b))))
    }
    #[inline(always)]
    fn cvt_i32_f64(self, a: __m256i) -> __m256i {
        avx!(_mm256_castpd_si256(_mm256_cvtepi32_pd(_mm256_castsi256_si128(a))))
    }
    #[inline(always)]
    fn cvtt_f64_i32(self, a: __m256i) -> __m256i {
        avx!(_mm256_zextsi128_si256(_mm256_cvttpd_epi32(_mm256_castsi256_pd(a))))
    }
    #[inline(always)]
    fn store_i32_half(self, b: &mut [i32], v: __m256i) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes (checked), AVX2 present.
        unsafe { _mm_storeu_si128(b.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(v)) }
    }

    // R6-jpeg, R6-xf
    #[inline(always)]
    fn sra16<const N: i32>(self, a: __m256i) -> __m256i {
        avx!(_mm256_srai_epi16::<N>(a))
    }
    #[inline(always)]
    fn mulhis16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_mulhi_epi16(a, b))
    }
    #[inline(always)]
    fn packs16(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_packs_epi16(a, b))
    }
    #[inline(always)]
    fn load_i16(self, b: &[i16]) -> __m256i {
        assert!(b.len() >= 16, "simd: slice too short");
        // SAFETY: 32 readable bytes (checked), AVX2 present.
        unsafe { _mm256_loadu_si256(b.as_ptr() as *const __m256i) }
    }
    #[inline(always)]
    fn store_i16(self, b: &mut [i16], v: __m256i) {
        assert!(b.len() >= 16, "simd: slice too short");
        // SAFETY: 32 writable bytes (checked), AVX2 present.
        unsafe { _mm256_storeu_si256(b.as_mut_ptr() as *mut __m256i, v) }
    }
    #[inline(always)]
    fn store_lanes2(self, b: &mut [u8], lo: __m256i, hi: __m256i) {
        chk(b, 64);
        // SAFETY: 64 writable bytes (checked), AVX2 present.
        unsafe {
            _mm256_storeu_si256(b.as_mut_ptr() as *mut __m256i, _mm256_permute2x128_si256::<0x20>(lo, hi));
            _mm256_storeu_si256(b.as_mut_ptr().add(32) as *mut __m256i, _mm256_permute2x128_si256::<0x31>(lo, hi));
        }
    }
    #[inline(always)]
    fn load_lanes2(self, b: &[u8]) -> (__m256i, __m256i) {
        chk(b, 64);
        // SAFETY: 64 readable bytes (checked), AVX2 present.
        unsafe {
            let a = _mm256_loadu_si256(b.as_ptr() as *const __m256i);
            let c = _mm256_loadu_si256(b.as_ptr().add(32) as *const __m256i);
            (_mm256_permute2x128_si256::<0x20>(a, c), _mm256_permute2x128_si256::<0x31>(a, c))
        }
    }
    #[inline(always)]
    fn load_i16_lanes(self, a: &[i16], b: &[i16]) -> __m256i {
        assert!(a.len() >= 8 && b.len() >= 8, "simd: slice too short");
        // SAFETY: 16 readable bytes in each source (checked), AVX2 present.
        unsafe {
            let lo = _mm_loadu_si128(a.as_ptr() as *const __m128i);
            let hi = _mm_loadu_si128(b.as_ptr() as *const __m128i);
            _mm256_inserti128_si256::<1>(_mm256_castsi128_si256(lo), hi)
        }
    }
    #[inline(always)]
    fn store_raw_lanes(self, a: &mut [u8], b: &mut [u8], v: __m256i) {
        chk(a, 8);
        chk(b, 8);
        // SAFETY: 8 writable bytes in each destination (checked), AVX2 present.
        unsafe {
            _mm_storel_epi64(a.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(v));
            _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, _mm256_extracti128_si256::<1>(v));
        }
    }
    #[inline(always)]
    fn sadu8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_sad_epu8(a, b))
    }
    #[inline(always)]
    fn add64(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_add_epi64(a, b))
    }

    // R6-icc
    #[inline(always)]
    fn maxf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_max_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn minf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_min_ps(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }
    #[inline(always)]
    fn sqrtf(self, a: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_sqrt_ps(_mm256_castsi256_ps(a))))
    }

    // r9-av1
    #[inline(always)]
    fn load_u16(self, b: &[u16]) -> __m256i {
        assert!(b.len() >= 16, "simd: slice too short");
        // SAFETY: 32 readable bytes (checked), AVX2 present.
        unsafe { _mm256_loadu_si256(b.as_ptr() as *const __m256i) }
    }
    #[inline(always)]
    fn store_u16(self, b: &mut [u16], v: __m256i) {
        assert!(b.len() >= 16, "simd: slice too short");
        // SAFETY: 32 writable bytes (checked), AVX2 present.
        unsafe { _mm256_storeu_si256(b.as_mut_ptr() as *mut __m256i, v) }
    }
    #[inline(always)]
    fn load_u16_lo(self, b: &[u16]) -> __m256i {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 readable bytes (checked), AVX2 present.
        unsafe { _mm256_zextsi128_si256(_mm_loadl_epi64(b.as_ptr() as *const __m128i)) }
    }
    #[inline(always)]
    fn store_u16_lo(self, b: &mut [u16], v: __m256i) {
        assert!(b.len() >= 4, "simd: slice too short");
        // SAFETY: 8 writable bytes (checked), AVX2 present.
        unsafe { _mm_storel_epi64(b.as_mut_ptr() as *mut __m128i, _mm256_castsi256_si128(v)) }
    }
    #[inline(always)]
    fn srl16x(self, a: __m256i, n: u32) -> __m256i {
        avx!(_mm256_srl_epi16(a, _mm_cvtsi32_si128(n as i32)))
    }

    // R9-jxl
    #[inline(always)]
    fn load_f32(self, b: &[f32]) -> __m256i {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 32 readable bytes (checked), AVX2 present.
        unsafe { _mm256_castps_si256(_mm256_loadu_ps(b.as_ptr())) }
    }
    #[inline(always)]
    fn store_f32(self, b: &mut [f32], v: __m256i) {
        assert!(b.len() >= 8, "simd: slice too short");
        // SAFETY: 32 writable bytes (checked), AVX2 present.
        unsafe { _mm256_storeu_ps(b.as_mut_ptr(), _mm256_castsi256_ps(v)) }
    }

    // R9-webp
    #[inline(always)]
    fn adds8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_adds_epi8(a, b))
    }
    #[inline(always)]
    fn subs8(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_subs_epi8(a, b))
    }

    // r9-dds
    #[inline(always)]
    fn cmpltf(self, a: __m256i, b: __m256i) -> __m256i {
        avx!(_mm256_castps_si256(_mm256_cmp_ps::<_CMP_LT_OS>(_mm256_castsi256_ps(a), _mm256_castsi256_ps(b))))
    }

    // jxl-speed
    #[inline(always)]
    fn load_f32_lanes(self, a: &[f32], b: &[f32]) -> __m256i {
        assert!(a.len() >= 4 && b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 readable bytes in each source (checked), AVX2 present.
        unsafe {
            let lo = _mm_loadu_ps(a.as_ptr());
            let hi = _mm_loadu_ps(b.as_ptr());
            _mm256_castps_si256(_mm256_insertf128_ps::<1>(_mm256_castps128_ps256(lo), hi))
        }
    }
    #[inline(always)]
    fn store_f32_lanes(self, a: &mut [f32], b: &mut [f32], v: __m256i) {
        assert!(a.len() >= 4 && b.len() >= 4, "simd: slice too short");
        // SAFETY: 16 writable bytes in each destination (checked), AVX2 present.
        unsafe {
            let f = _mm256_castsi256_ps(v);
            _mm_storeu_ps(a.as_mut_ptr(), _mm256_castps256_ps128(f));
            _mm_storeu_ps(b.as_mut_ptr(), _mm256_extractf128_ps::<1>(f));
        }
    }

    // perf-blend
    #[inline(always)]
    fn packssdw(self, a: __m256i, b: __m256i) -> __m256i {
        // SAFETY: AVX2 present (an Avx2 value exists), the precondition of avx2_packssdw.
        unsafe { avx2_packssdw(a, b) }
    }
    #[inline(always)]
    fn packuswb_px(self, v: __m256i) -> __m256i {
        // SAFETY: AVX2 present (an Avx2 value exists), the precondition of avx2_packuswb.
        let p = unsafe { avx2_packuswb(v, v) };
        avx!(_mm256_permute4x64_epi64::<0x08>(p))
    }
    #[inline(always)]
    fn ptest_raw(self, a: __m256i, b: __m256i) -> u32 {
        let (a, b) = avx!((_mm256_castsi256_si128(a), _mm256_castsi256_si128(b)));
        // SAFETY: vptest reads two registers and sets the flags, the jumps go to the label
        // blocks; AVX present (an Avx2 value exists).
        unsafe {
            core::arch::asm!("vptest {a}, {b}", "jz {z}", "jnc {m}", a = in(xmm_reg) a, b = in(xmm_reg) b,
                             z = label { return 1 }, m = label { return 0 }, options(nomem, nostack))
        };
        2
    }
    #[inline(always)]
    fn ptestz_raw(self, a: __m256i, b: __m256i) -> bool {
        let (a, b) = avx!((_mm256_castsi256_si128(a), _mm256_castsi256_si128(b)));
        // SAFETY: as ptest_raw.
        unsafe {
            core::arch::asm!("vptest {a}, {b}", "jz {z}", a = in(xmm_reg) a, b = in(xmm_reg) b,
                             z = label { return true }, options(nomem, nostack))
        };
        false
    }
}

// perf-blend: vpackssdw / vpackuswb as inline assembly, so LLVM cannot rewrite them into
// smax / smin + pack (see the trait).  #[inline] functions with the avx2 feature: inlined
// into the AVX2 kernels (run_avx2 has the feature), the ymm operands need it.
#[inline]
#[target_feature(enable = "avx2")]
fn avx2_packssdw(a: __m256i, b: __m256i) -> __m256i {
    let r: __m256i;
    // SAFETY: a register-only AVX2 instruction (no memory, stack or flags); this function
    // has the avx2 feature, so its callers checked for AVX2.
    unsafe {
        core::arch::asm!("vpackssdw {r}, {a}, {b}", r = lateout(ymm_reg) r, a = in(ymm_reg) a, b = in(ymm_reg) b,
                         options(pure, nomem, nostack, preserves_flags))
    };
    r
}

#[inline]
#[target_feature(enable = "avx2")]
fn avx2_packuswb(a: __m256i, b: __m256i) -> __m256i {
    let r: __m256i;
    // SAFETY: as avx2_packssdw.
    unsafe {
        core::arch::asm!("vpackuswb {r}, {a}, {b}", r = lateout(ymm_reg) r, a = in(ymm_reg) a, b = in(ymm_reg) b,
                         options(pure, nomem, nostack, preserves_flags))
    };
    r
}
