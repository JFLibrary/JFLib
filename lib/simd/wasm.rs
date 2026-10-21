// WASM SIMD128 backend. The module is compiled only with +simd128; all callers
// therefore have the required feature. SSE lane ordering, saturation, oversized
// shifts and NaN/overflow conversion rules are preserved (no relaxed SIMD/FMA).
use crate::Simd;
use core::arch::wasm32::*;

/// Native 128-bit WebAssembly SIMD registers, selected by the browser bundle.
#[derive(Clone, Copy)]
pub struct Wasm128;

#[inline(always)]
fn vector(bytes: [u8; 16]) -> v128 {
    // SAFETY: v128 and [u8;16] have the same size and every bit pattern is valid.
    unsafe { core::mem::transmute(bytes) }
}

impl Simd for Wasm128 {
    type V = v128;
    const BYTES: usize = 16;
    const PX: usize = 2;
    #[inline(always)]
    fn zero(self) -> Self::V {
        u8x16_splat(0)
    }
    #[inline(always)]
    fn splat8(self, v: u8) -> Self::V {
        u8x16_splat(v)
    }
    #[inline(always)]
    fn splat16(self, v: u16) -> Self::V {
        u16x8_splat(v)
    }
    #[inline(always)]
    fn splat32(self, v: u32) -> Self::V {
        u32x4_splat(v)
    }
    #[inline(always)]
    fn splat64(self, v: u64) -> Self::V {
        u64x2_splat(v)
    }
    #[inline(always)]
    fn splatf(self, v: f32) -> Self::V {
        f32x4_splat(v)
    }
    #[inline(always)]
    fn load(self, b: &[u8]) -> Self::V {
        assert!(b.len() >= 16);
        // SAFETY: the length check provides 16 readable bytes; v128_load is unaligned.
        unsafe { v128_load(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store(self, b: &mut [u8], v: Self::V) {
        assert!(b.len() >= 16);
        // SAFETY: the length check provides 16 writable bytes; v128_store is unaligned.
        unsafe { v128_store(b.as_mut_ptr().cast(), v) }
    }
    #[inline(always)]
    fn load_px(self, b: &[u8]) -> Self::V {
        self.widen_px(self.load_raw(b))
    }
    #[inline(always)]
    fn store_px(self, b: &mut [u8], v: Self::V) {
        self.store_raw(b, self.packuswb_px(v));
    }
    #[inline(always)]
    fn bcast_px(self, b: &[u8]) -> Self::V {
        let a = u16x8_splat(b[0] as u16);
        let c = u16x8_splat(b[1] as u16);
        i64x2_shuffle::<0, 2>(a, c)
    }
    #[inline(always)]
    fn load_raw(self, b: &[u8]) -> Self::V {
        assert!(b.len() >= 8);
        // SAFETY: eight readable bytes; the intrinsic zeroes the upper half.
        unsafe { v128_load64_zero(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_raw(self, b: &mut [u8], v: Self::V) {
        b[..8].copy_from_slice(&i64x2_extract_lane::<0>(v).to_le_bytes());
    }
    #[inline(always)]
    fn widen_px(self, v: Self::V) -> Self::V {
        u16x8_extend_low_u8x16(v)
    }
    #[inline(always)]
    fn load_lane32(self, b: &[u8]) -> Self::V {
        self.from_i32(i32::from_le_bytes(b[..4].try_into().unwrap()))
    }
    #[inline(always)]
    fn load_i32(self, b: &[i32]) -> Self::V {
        assert!(b.len() >= 4);
        // SAFETY: the length check provides 16 readable bytes; v128_load is unaligned.
        unsafe { v128_load(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_i32(self, b: &mut [i32], v: Self::V) {
        assert!(b.len() >= 4);
        // SAFETY: the length check provides 16 writable bytes; v128_store is unaligned.
        unsafe { v128_store(b.as_mut_ptr().cast(), v) }
    }
    #[inline(always)]
    fn from_i32(self, v: i32) -> Self::V {
        i32x4_replace_lane::<0>(i32x4_splat(0), v)
    }
    #[inline(always)]
    fn to_i32(self, v: Self::V) -> i32 {
        i32x4_extract_lane::<0>(v)
    }
    #[inline(always)]
    fn add8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_add(a, b)
    }
    #[inline(always)]
    fn sub8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_sub(a, b)
    }
    #[inline(always)]
    fn addus8(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_add_sat(a, b)
    }
    #[inline(always)]
    fn subus8(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_sub_sat(a, b)
    }
    #[inline(always)]
    fn maxu8(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_max(a, b)
    }
    #[inline(always)]
    fn minu8(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_min(a, b)
    }
    #[inline(always)]
    fn cmpeq8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_eq(a, b)
    }
    #[inline(always)]
    fn avgu8(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_avgr(a, b)
    }
    #[inline(always)]
    fn movemask8(self, a: Self::V) -> u32 {
        i8x16_bitmask(a) as u32
    }
    #[inline(always)]
    fn unpacklo8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_shuffle::<0, 16, 1, 17, 2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23>(a, b)
    }
    #[inline(always)]
    fn unpackhi8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_shuffle::<8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30, 15, 31>(a, b)
    }
    #[inline(always)]
    fn packus16(self, a: Self::V, b: Self::V) -> Self::V {
        u8x16_narrow_i16x8(a, b)
    }
    #[inline(always)]
    fn add16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_add(a, b)
    }
    #[inline(always)]
    fn sub16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_sub(a, b)
    }
    #[inline(always)]
    fn subus16(self, a: Self::V, b: Self::V) -> Self::V {
        u16x8_sub_sat(a, b)
    }
    #[inline(always)]
    fn mullo16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_mul(a, b)
    }
    #[inline(always)]
    fn mulhiu16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_shuffle::<1, 3, 5, 7, 9, 11, 13, 15>(
            u32x4_extmul_low_u16x8(a, b),
            u32x4_extmul_high_u16x8(a, b),
        )
    }
    #[inline(always)]
    fn mins16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_min(a, b)
    }
    #[inline(always)]
    fn maxs16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_max(a, b)
    }
    #[inline(always)]
    fn cmpeq16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_eq(a, b)
    }
    #[inline(always)]
    fn cmpgts16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_gt(a, b)
    }
    #[inline(always)]
    fn srl16<const N: i32>(self, a: Self::V) -> Self::V {
        if (N as u32) >= 16 {
            self.zero()
        } else {
            u16x8_shr(a, N as u32)
        }
    }
    #[inline(always)]
    fn sll16<const N: i32>(self, a: Self::V) -> Self::V {
        if (N as u32) >= 16 {
            self.zero()
        } else {
            i16x8_shl(a, N as u32)
        }
    }
    #[inline(always)]
    fn shuf16<const IMM: i32>(self, a: Self::V) -> Self::V {
        let mut mask = [0u8; 16];
        for i in 0..8 {
            for j in 0..2 {
                mask[i * 2 + j] =
                    ((i / 4) * 8) as u8 + (((IMM >> ((i % 4) * 2)) & 3) * 2) as u8 + j as u8;
            }
        }
        i8x16_swizzle(a, vector(mask))
    }
    #[inline(always)]
    fn unpacklo16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_shuffle::<0, 8, 1, 9, 2, 10, 3, 11>(a, b)
    }
    #[inline(always)]
    fn unpackhi16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_shuffle::<4, 12, 5, 13, 6, 14, 7, 15>(a, b)
    }
    #[inline(always)]
    fn packs32(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_narrow_i32x4(a, b)
    }
    #[inline(always)]
    fn madd16(self, a: Self::V, b: Self::V) -> Self::V {
        i32x4_dot_i16x8(a, b)
    }
    #[inline(always)]
    fn add32(self, a: Self::V, b: Self::V) -> Self::V {
        i32x4_add(a, b)
    }
    #[inline(always)]
    fn sub32(self, a: Self::V, b: Self::V) -> Self::V {
        i32x4_sub(a, b)
    }
    #[inline(always)]
    fn cmpeq32(self, a: Self::V, b: Self::V) -> Self::V {
        i32x4_eq(a, b)
    }
    #[inline(always)]
    fn cmpgts32(self, a: Self::V, b: Self::V) -> Self::V {
        i32x4_gt(a, b)
    }
    #[inline(always)]
    fn movemask32(self, a: Self::V) -> u32 {
        i32x4_bitmask(a) as u32
    }
    #[inline(always)]
    fn shuf32<const IMM: i32>(self, a: Self::V) -> Self::V {
        let mut mask = [0u8; 16];
        for i in 0..4 {
            for j in 0..4 {
                mask[i * 4 + j] = (((IMM >> (i * 2)) & 3) * 4) as u8 + j as u8;
            }
        }
        i8x16_swizzle(a, vector(mask))
    }
    #[inline(always)]
    fn shufps<const IMM: i32>(self, a: Self::V, b: Self::V) -> Self::V {
        let x = self.shuf32::<IMM>(a);
        let y = self.shuf32::<IMM>(b);
        i64x2_shuffle::<0, 3>(x, y)
    }
    #[inline(always)]
    fn srl32<const N: i32>(self, a: Self::V) -> Self::V {
        if (N as u32) >= 32 {
            self.zero()
        } else {
            u32x4_shr(a, N as u32)
        }
    }
    #[inline(always)]
    fn sll32<const N: i32>(self, a: Self::V) -> Self::V {
        if (N as u32) >= 32 {
            self.zero()
        } else {
            i32x4_shl(a, N as u32)
        }
    }
    #[inline(always)]
    fn sra32<const N: i32>(self, a: Self::V) -> Self::V {
        i32x4_shr(a, (N as u32).min(31))
    }
    #[inline(always)]
    fn bsll<const N: i32>(self, a: Self::V) -> Self::V {
        let mut mask = [255u8; 16];
        if (N as u32) < 16 {
            for i in N as usize..16 {
                mask[i] = (i - N as usize) as u8;
            }
        }
        i8x16_swizzle(a, vector(mask))
    }
    #[inline(always)]
    fn bsrl<const N: i32>(self, a: Self::V) -> Self::V {
        let mut mask = [255u8; 16];
        if (N as u32) < 16 {
            for i in 0..16 - N as usize {
                mask[i] = (i + N as usize) as u8;
            }
        }
        i8x16_swizzle(a, vector(mask))
    }
    #[inline(always)]
    fn and(self, a: Self::V, b: Self::V) -> Self::V {
        v128_and(a, b)
    }
    #[inline(always)]
    fn andnot(self, a: Self::V, b: Self::V) -> Self::V {
        v128_andnot(b, a)
    }
    #[inline(always)]
    fn or(self, a: Self::V, b: Self::V) -> Self::V {
        v128_or(a, b)
    }
    #[inline(always)]
    fn xor(self, a: Self::V, b: Self::V) -> Self::V {
        v128_xor(a, b)
    }
    #[inline(always)]
    fn cvt_i32_f32(self, a: Self::V) -> Self::V {
        f32x4_convert_i32x4(a)
    }
    #[inline(always)]
    fn cvtt_f32_i32(self, a: Self::V) -> Self::V {
        v128_bitselect(
            i32x4_trunc_sat_f32x4(a),
            i32x4_splat(i32::MIN),
            v128_and(
                f32x4_ge(a, f32x4_splat(-2147483648.0)),
                f32x4_lt(a, f32x4_splat(2147483648.0)),
            ),
        )
    }
    #[inline(always)]
    fn cvtr_f32_i32(self, a: Self::V) -> Self::V {
        self.cvtt_f32_i32(f32x4_nearest(a))
    }
    #[inline(always)]
    fn addf(self, a: Self::V, b: Self::V) -> Self::V {
        f32x4_add(a, b)
    }
    #[inline(always)]
    fn subf(self, a: Self::V, b: Self::V) -> Self::V {
        f32x4_sub(a, b)
    }
    #[inline(always)]
    fn mulf(self, a: Self::V, b: Self::V) -> Self::V {
        f32x4_mul(a, b)
    }
    #[inline(always)]
    fn divf(self, a: Self::V, b: Self::V) -> Self::V {
        f32x4_div(a, b)
    }
    #[inline(always)]
    fn splatd(self, v: f64) -> Self::V {
        f64x2_splat(v)
    }
    #[inline(always)]
    fn addd(self, a: Self::V, b: Self::V) -> Self::V {
        f64x2_add(a, b)
    }
    #[inline(always)]
    fn subd(self, a: Self::V, b: Self::V) -> Self::V {
        f64x2_sub(a, b)
    }
    #[inline(always)]
    fn muld(self, a: Self::V, b: Self::V) -> Self::V {
        f64x2_mul(a, b)
    }
    #[inline(always)]
    fn divd(self, a: Self::V, b: Self::V) -> Self::V {
        f64x2_div(a, b)
    }
    #[inline(always)]
    fn sqrtd(self, a: Self::V) -> Self::V {
        f64x2_sqrt(a)
    }
    #[inline(always)]
    fn maxd(self, a: Self::V, b: Self::V) -> Self::V {
        v128_bitselect(a, b, f64x2_gt(a, b))
    }
    #[inline(always)]
    fn mind(self, a: Self::V, b: Self::V) -> Self::V {
        v128_bitselect(a, b, f64x2_lt(a, b))
    }
    #[inline(always)]
    fn cmpltd(self, a: Self::V, b: Self::V) -> Self::V {
        f64x2_lt(a, b)
    }
    #[inline(always)]
    fn cvt_i32_f64(self, a: Self::V) -> Self::V {
        f64x2_convert_low_i32x4(a)
    }
    #[inline(always)]
    fn cvtt_f64_i32(self, a: Self::V) -> Self::V {
        let valid = v128_and(
            f64x2_ge(a, f64x2_splat(-2147483648.0)),
            f64x2_lt(a, f64x2_splat(2147483648.0)),
        );
        let mask = i32x4_shuffle::<0, 2, 4, 4>(valid, self.zero());
        v128_bitselect(
            i32x4_trunc_sat_f64x2_zero(a),
            i32x4(i32::MIN, i32::MIN, 0, 0),
            mask,
        )
    }
    #[inline(always)]
    fn store_i32_half(self, b: &mut [i32], v: Self::V) {
        b[0] = i32x4_extract_lane::<0>(v);
        b[1] = i32x4_extract_lane::<1>(v);
    }
    #[inline(always)]
    fn sra16<const N: i32>(self, a: Self::V) -> Self::V {
        i16x8_shr(a, (N as u32).min(15))
    }
    #[inline(always)]
    fn mulhis16(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_shuffle::<1, 3, 5, 7, 9, 11, 13, 15>(
            i32x4_extmul_low_i16x8(a, b),
            i32x4_extmul_high_i16x8(a, b),
        )
    }
    #[inline(always)]
    fn packs16(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_narrow_i16x8(a, b)
    }
    #[inline(always)]
    fn load_i16(self, b: &[i16]) -> Self::V {
        assert!(b.len() >= 8);
        // SAFETY: the length check provides 16 readable bytes; v128_load is unaligned.
        unsafe { v128_load(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_i16(self, b: &mut [i16], v: Self::V) {
        assert!(b.len() >= 8);
        // SAFETY: the length check provides 16 writable bytes; v128_store is unaligned.
        unsafe { v128_store(b.as_mut_ptr().cast(), v) }
    }
    #[inline(always)]
    fn store_lanes2(self, b: &mut [u8], lo: Self::V, hi: Self::V) {
        self.store(b, lo);
        self.store(&mut b[16..], hi);
    }
    #[inline(always)]
    fn load_lanes2(self, b: &[u8]) -> (Self::V, Self::V) {
        (self.load(b), self.load(&b[16..]))
    }
    #[inline(always)]
    fn load_i16_lanes(self, a: &[i16], b: &[i16]) -> Self::V {
        let _ = b;
        self.load_i16(a)
    }
    #[inline(always)]
    fn store_raw_lanes(self, a: &mut [u8], b: &mut [u8], v: Self::V) {
        let _ = b;
        self.store_raw(a, v);
    }
    #[inline(always)]
    fn sadu8(self, a: Self::V, b: Self::V) -> Self::V {
        let d = v128_or(u8x16_sub_sat(a, b), u8x16_sub_sat(b, a));
        let sums = u32x4_extadd_pairwise_u16x8(u16x8_extadd_pairwise_u8x16(d));
        u64x2(
            u32x4_extract_lane::<0>(sums) as u64 + u32x4_extract_lane::<1>(sums) as u64,
            u32x4_extract_lane::<2>(sums) as u64 + u32x4_extract_lane::<3>(sums) as u64,
        )
    }
    #[inline(always)]
    fn add64(self, a: Self::V, b: Self::V) -> Self::V {
        i64x2_add(a, b)
    }
    #[inline(always)]
    fn maxf(self, a: Self::V, b: Self::V) -> Self::V {
        v128_bitselect(a, b, f32x4_gt(a, b))
    }
    #[inline(always)]
    fn minf(self, a: Self::V, b: Self::V) -> Self::V {
        v128_bitselect(a, b, f32x4_lt(a, b))
    }
    #[inline(always)]
    fn sqrtf(self, a: Self::V) -> Self::V {
        f32x4_sqrt(a)
    }
    #[inline(always)]
    fn load_u16(self, b: &[u16]) -> Self::V {
        assert!(b.len() >= 8);
        // SAFETY: the length check provides 16 readable bytes; v128_load is unaligned.
        unsafe { v128_load(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_u16(self, b: &mut [u16], v: Self::V) {
        assert!(b.len() >= 8);
        // SAFETY: the length check provides 16 writable bytes; v128_store is unaligned.
        unsafe { v128_store(b.as_mut_ptr().cast(), v) }
    }
    #[inline(always)]
    fn load_u16_lo(self, b: &[u16]) -> Self::V {
        assert!(b.len() >= 4);
        // SAFETY: four u16 values provide eight readable bytes.
        unsafe { v128_load64_zero(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_u16_lo(self, b: &mut [u16], v: Self::V) {
        let bytes = i64x2_extract_lane::<0>(v).to_le_bytes();
        for i in 0..4 {
            b[i] = u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
        }
    }
    #[inline(always)]
    fn srl16x(self, a: Self::V, n: u32) -> Self::V {
        if n >= 16 {
            self.zero()
        } else {
            u16x8_shr(a, n)
        }
    }
    #[inline(always)]
    fn load_f32(self, b: &[f32]) -> Self::V {
        assert!(b.len() >= 4);
        // SAFETY: the length check provides 16 readable bytes; v128_load is unaligned.
        unsafe { v128_load(b.as_ptr().cast()) }
    }
    #[inline(always)]
    fn store_f32(self, b: &mut [f32], v: Self::V) {
        assert!(b.len() >= 4);
        // SAFETY: the length check provides 16 writable bytes; v128_store is unaligned.
        unsafe { v128_store(b.as_mut_ptr().cast(), v) }
    }
    #[inline(always)]
    fn adds8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_add_sat(a, b)
    }
    #[inline(always)]
    fn subs8(self, a: Self::V, b: Self::V) -> Self::V {
        i8x16_sub_sat(a, b)
    }
    #[inline(always)]
    fn cmpltf(self, a: Self::V, b: Self::V) -> Self::V {
        f32x4_lt(a, b)
    }
    #[inline(always)]
    fn load_f32_lanes(self, a: &[f32], b: &[f32]) -> Self::V {
        let _ = b;
        self.load_f32(a)
    }
    #[inline(always)]
    fn store_f32_lanes(self, a: &mut [f32], b: &mut [f32], v: Self::V) {
        let _ = b;
        self.store_f32(a, v);
    }
    #[inline(always)]
    fn packssdw(self, a: Self::V, b: Self::V) -> Self::V {
        i16x8_narrow_i32x4(a, b)
    }
    #[inline(always)]
    fn packuswb_px(self, v: Self::V) -> Self::V {
        u8x16_narrow_i16x8(v, v)
    }
    #[inline(always)]
    fn ptest_raw(self, a: Self::V, b: Self::V) -> u32 {
        let ab = i64x2_extract_lane::<0>(v128_and(a, b));
        if ab == 0 {
            1
        } else if i64x2_extract_lane::<0>(v128_andnot(b, a)) == 0 {
            2
        } else {
            0
        }
    }
    #[inline(always)]
    fn ptestz_raw(self, a: Self::V, b: Self::V) -> bool {
        i64x2_extract_lane::<0>(v128_and(a, b)) == 0
    }
}
