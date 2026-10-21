// Portable scalar implementations of the complete Simd contract. Registers are
// little-endian bytes; shuffles/packs work per 128-bit lane just like the native path.
// Kept available on native targets so tests compare the actual implementations.

use crate::Simd;
#[cfg(target_arch = "wasm32")]
use rt::fx as portable_fx;
#[cfg(not(target_arch = "wasm32"))]
use rt::portable::fx as portable_fx;

/// Portable 128-bit register backend, the first WebAssembly baseline.
#[derive(Clone, Copy)]
pub struct Scalar;
/// Portable pair of 128-bit lanes, useful for AVX2-equivalent conformance tests.
#[derive(Clone, Copy)]
pub struct Scalar256;
impl Scalar256 {
    /// AVX2 hardware is not provided by this compatibility type.
    pub fn new() -> Option<Self> {
        None
    }
}

#[inline(always)]
fn u16at(a: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([a[i], a[i + 1]])
}
#[inline(always)]
fn u32at(a: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([a[i], a[i + 1], a[i + 2], a[i + 3]])
}
#[inline(always)]
fn u64at(a: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([
        a[i],
        a[i + 1],
        a[i + 2],
        a[i + 3],
        a[i + 4],
        a[i + 5],
        a[i + 6],
        a[i + 7],
    ])
}

macro_rules! backend {
    ($name:ident, $n:expr) => {
        impl Simd for $name {
            type V = [u8; $n];
            const BYTES: usize = $n;
            const PX: usize = $n / 8;
            #[inline(always)]
            fn zero(self) -> Self::V {
                [0; $n]
            }
            #[inline(always)]
            fn splat8(self, v: u8) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = v;
                }
                out
            }
            #[inline(always)]
            fn splat16(self, v: u16) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(&(v).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn splat32(self, v: u32) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(&(v).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn splat64(self, v: u64) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(&(v).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn splatf(self, v: f32) -> Self::V {
                self.splat32(v.to_bits())
            }
            #[inline(always)]
            fn splatd(self, v: f64) -> Self::V {
                self.splat64(v.to_bits())
            }
            #[inline(always)]
            fn load(self, b: &[u8]) -> Self::V {
                let mut out = [0; $n];
                out.copy_from_slice(&b[..$n]);
                out
            }
            #[inline(always)]
            fn store(self, b: &mut [u8], v: Self::V) -> () {
                b[..$n].copy_from_slice(&v);
            }
            #[inline(always)]
            fn load_raw(self, b: &[u8]) -> Self::V {
                let mut out = [0; $n];
                out[..$n / 2].copy_from_slice(&b[..$n / 2]);
                out
            }
            #[inline(always)]
            fn store_raw(self, b: &mut [u8], v: Self::V) -> () {
                b[..$n / 2].copy_from_slice(&v[..$n / 2]);
            }
            #[inline(always)]
            fn widen_px(self, v: Self::V) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 2 {
                    out[i * 2] = v[i];
                }
                out
            }
            #[inline(always)]
            fn load_px(self, b: &[u8]) -> Self::V {
                self.widen_px(self.load_raw(b))
            }
            #[inline(always)]
            fn store_px(self, b: &mut [u8], v: Self::V) -> () {
                for i in 0..$n / 2 {
                    b[i] = (u16at(&v, i * 2) as i16).clamp(0, 255) as u8;
                }
            }
            #[inline(always)]
            fn bcast_px(self, b: &[u8]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 8 {
                    for j in 0..4 {
                        out[i * 8 + j * 2] = b[i];
                    }
                }
                out
            }
            #[inline(always)]
            fn load_lane32(self, b: &[u8]) -> Self::V {
                let mut out = [0; $n];
                for lane in 0..$n / 16 {
                    out[lane * 16..lane * 16 + 4].copy_from_slice(&b[lane * 4..lane * 4 + 4]);
                }
                out
            }
            #[inline(always)]
            fn from_i32(self, v: i32) -> Self::V {
                let mut out = [0; $n];
                out[..4].copy_from_slice(&v.to_le_bytes());
                out
            }
            #[inline(always)]
            fn to_i32(self, v: Self::V) -> i32 {
                u32at(&v, 0) as i32
            }
            #[inline(always)]
            fn load_i16(self, b: &[i16]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 2 {
                    out[i * 2..(i + 1) * 2].copy_from_slice(&(b[i]).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn store_i16(self, b: &mut [i16], v: Self::V) -> () {
                for i in 0..$n / 2 {
                    b[i] = u16at(&v, i * 2) as i16;
                }
            }
            #[inline(always)]
            fn load_u16(self, b: &[u16]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 2 {
                    out[i * 2..(i + 1) * 2].copy_from_slice(&(b[i]).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn store_u16(self, b: &mut [u16], v: Self::V) -> () {
                for i in 0..$n / 2 {
                    b[i] = u16at(&v, i * 2) as u16;
                }
            }
            #[inline(always)]
            fn load_i32(self, b: &[i32]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 4 {
                    out[i * 4..(i + 1) * 4].copy_from_slice(&(b[i]).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn store_i32(self, b: &mut [i32], v: Self::V) -> () {
                for i in 0..$n / 4 {
                    b[i] = u32at(&v, i * 4) as i32;
                }
            }
            #[inline(always)]
            fn load_f32(self, b: &[f32]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 4 {
                    out[i * 4..(i + 1) * 4].copy_from_slice(&(b[i].to_bits()).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn store_f32(self, b: &mut [f32], v: Self::V) -> () {
                for i in 0..$n / 4 {
                    b[i] = f32::from_bits(u32at(&v, i * 4));
                }
            }
            #[inline(always)]
            fn load_u16_lo(self, b: &[u16]) -> Self::V {
                let mut out = [0; $n];
                for i in 0..4 {
                    out[i * 2..i * 2 + 2].copy_from_slice(&b[i].to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn store_u16_lo(self, b: &mut [u16], v: Self::V) -> () {
                for i in 0..4 {
                    b[i] = u16at(&v, i * 2);
                }
            }
            #[inline(always)]
            fn store_i32_half(self, b: &mut [i32], v: Self::V) -> () {
                for i in 0..$n / 8 {
                    b[i] = u32at(&v, i * 4) as i32;
                }
            }
            #[inline(always)]
            fn add8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].wrapping_add(b[i]);
                }
                out
            }
            #[inline(always)]
            fn sub8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].wrapping_sub(b[i]);
                }
                out
            }
            #[inline(always)]
            fn cmpeq8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = if a[i] == b[i] { u8::MAX } else { 0 };
                }
                out
            }
            #[inline(always)]
            fn add16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2]
                        .copy_from_slice(&(u16at(&a, i).wrapping_add(u16at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn sub16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2]
                        .copy_from_slice(&(u16at(&a, i).wrapping_sub(u16at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn cmpeq16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(if u16at(&a, i) == u16at(&b, i) {
                            u16::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn add32(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4]
                        .copy_from_slice(&(u32at(&a, i).wrapping_add(u32at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn sub32(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4]
                        .copy_from_slice(&(u32at(&a, i).wrapping_sub(u32at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn cmpeq32(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if u32at(&a, i) == u32at(&b, i) {
                            u32::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn add64(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8]
                        .copy_from_slice(&(u64at(&a, i).wrapping_add(u64at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn subus8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].saturating_sub(b[i]);
                }
                out
            }
            #[inline(always)]
            fn addus8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].saturating_add(b[i]);
                }
                out
            }
            #[inline(always)]
            fn maxu8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].max(b[i]);
                }
                out
            }
            #[inline(always)]
            fn minu8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i].min(b[i]);
                }
                out
            }
            #[inline(always)]
            fn avgu8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = ((a[i] as u16 + b[i] as u16 + 1) >> 1) as u8;
                }
                out
            }
            #[inline(always)]
            fn adds8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = (a[i] as i8).saturating_add(b[i] as i8) as u8;
                }
                out
            }
            #[inline(always)]
            fn subs8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = (a[i] as i8).saturating_sub(b[i] as i8) as u8;
                }
                out
            }
            #[inline(always)]
            fn subus16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(u16at(&a, i).saturating_sub(u16at(&b, i))).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn mullo16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2]
                        .copy_from_slice(&(u16at(&a, i).wrapping_mul(u16at(&b, i))).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn mulhiu16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(((u16at(&a, i) as u32 * u16at(&b, i) as u32) >> 16) as u16).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn mulhis16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(((u16at(&a, i) as i16 as i32 * u16at(&b, i) as i16 as i32) >> 16) as u16)
                            .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn mins16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &((u16at(&a, i) as i16).min(u16at(&b, i) as i16) as u16).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn maxs16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &((u16at(&a, i) as i16).max(u16at(&b, i) as i16) as u16).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cmpgts16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(if (u16at(&a, i) as i16) > (u16at(&b, i) as i16) {
                            u16::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn srl16<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(if (N as u32) < 16 {
                            u16at(&a, i) >> (N as u32)
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sll16<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(if (N as u32) < 16 {
                            u16at(&a, i) << (N as u32)
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sra16<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(((u16at(&a, i) as i16) >> (N as u32).min(15)) as u16).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cmpgts32(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if (u32at(&a, i) as i32) > (u32at(&b, i) as i32) {
                            u32::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn srl32<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if (N as u32) < 32 {
                            u32at(&a, i) >> (N as u32)
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sll32<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if (N as u32) < 32 {
                            u32at(&a, i) << (N as u32)
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sra32<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(((u32at(&a, i) as i32) >> (N as u32).min(31)) as u32).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn srl16x(self, a: Self::V, n: u32) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(2) {
                    out[i..i + 2].copy_from_slice(
                        &(if n < 16 { u16at(&a, i) >> n } else { 0 }).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn movemask8(self, a: Self::V) -> u32 {
                let mut out = 0;
                for i in (0..$n).step_by(1) {
                    out |= ((a[i] >> 7) as u32) << (i / 1);
                }
                out
            }
            #[inline(always)]
            fn movemask32(self, a: Self::V) -> u32 {
                let mut out = 0;
                for i in (0..$n).step_by(4) {
                    out |= ((u32at(&a, i) >> 31) as u32) << (i / 4);
                }
                out
            }
            #[inline(always)]
            fn unpacklo8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in (0..8).step_by(1) {
                        let dst = lane + j * 2;
                        let src = lane + 0 + j;
                        out[dst..dst + 1].copy_from_slice(&a[src..src + 1]);
                        out[dst + 1..dst + 2].copy_from_slice(&b[src..src + 1]);
                    }
                }
                out
            }
            #[inline(always)]
            fn unpackhi8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in (0..8).step_by(1) {
                        let dst = lane + j * 2;
                        let src = lane + 8 + j;
                        out[dst..dst + 1].copy_from_slice(&a[src..src + 1]);
                        out[dst + 1..dst + 2].copy_from_slice(&b[src..src + 1]);
                    }
                }
                out
            }
            #[inline(always)]
            fn unpacklo16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in (0..8).step_by(2) {
                        let dst = lane + j * 2;
                        let src = lane + 0 + j;
                        out[dst..dst + 2].copy_from_slice(&a[src..src + 2]);
                        out[dst + 2..dst + 4].copy_from_slice(&b[src..src + 2]);
                    }
                }
                out
            }
            #[inline(always)]
            fn unpackhi16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in (0..8).step_by(2) {
                        let dst = lane + j * 2;
                        let src = lane + 8 + j;
                        out[dst..dst + 2].copy_from_slice(&a[src..src + 2]);
                        out[dst + 2..dst + 4].copy_from_slice(&b[src..src + 2]);
                    }
                }
                out
            }
            #[inline(always)]
            fn packus16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for half in 0..2 {
                        let src = if half == 0 { &a } else { &b };
                        for j in 0..8 {
                            let i = lane + half * 8 + j * 1;
                            out[i] = (u16at(src, lane + j * 2) as i16).clamp(0, 255) as u8;
                        }
                    }
                }
                out
            }
            #[inline(always)]
            fn packs16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for half in 0..2 {
                        let src = if half == 0 { &a } else { &b };
                        for j in 0..8 {
                            let i = lane + half * 8 + j * 1;
                            out[i] = (u16at(src, lane + j * 2) as i16).clamp(-128, 127) as u8;
                        }
                    }
                }
                out
            }
            #[inline(always)]
            fn packs32(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for half in 0..2 {
                        let src = if half == 0 { &a } else { &b };
                        for j in 0..4 {
                            let i = lane + half * 8 + j * 2;
                            out[i..i + 2].copy_from_slice(
                                &((u32at(src, lane + j * 4) as i32).clamp(-32768, 32767) as u16)
                                    .to_le_bytes(),
                            );
                        }
                    }
                }
                out
            }
            #[inline(always)]
            fn packssdw(self, a: Self::V, b: Self::V) -> Self::V {
                self.packs32(a, b)
            }
            #[inline(always)]
            fn packuswb_px(self, v: Self::V) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 2 {
                    out[i] = (u16at(&v, i * 2) as i16).clamp(0, 255) as u8;
                }
                for i in 0..$n / 2 {
                    out[$n / 2 + i] = out[i % 8];
                }
                out
            }
            #[inline(always)]
            fn madd16(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &((u16at(&a, i) as i16 as i32)
                            .wrapping_mul(u16at(&b, i) as i16 as i32)
                            .wrapping_add((u16at(&a, i + 2) as i16 as i32).wrapping_mul(u16at(
                                &b,
                                i + 2,
                            )
                                as i16
                                as i32)) as u32)
                            .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn shuf16<const IMM: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0; $n];
                for half in (0..$n).step_by(8) {
                    for j in 0..4 {
                        let src = half + ((IMM as usize >> (j * 2)) & 3) * 2;
                        out[half + j * 2..half + j * 2 + 2].copy_from_slice(&a[src..src + 2]);
                    }
                }
                out
            }
            #[inline(always)]
            fn shuf32<const IMM: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in 0..4 {
                        let src = lane + ((IMM as usize >> (j * 2)) & 3) * 4;
                        out[lane + j * 4..lane + j * 4 + 4].copy_from_slice(&a[src..src + 4]);
                    }
                }
                out
            }
            #[inline(always)]
            fn shufps<const IMM: i32>(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(16) {
                    for j in 0..4 {
                        let src = lane + ((IMM as usize >> (j * 2)) & 3) * 4;
                        let from = if j < 2 { &a } else { &b };
                        out[lane + j * 4..lane + j * 4 + 4].copy_from_slice(&from[src..src + 4]);
                    }
                }
                out
            }
            #[inline(always)]
            fn bsll<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0; $n];
                let n = N as usize;
                if n < 16 {
                    for lane in (0..$n).step_by(16) {
                        out[lane + n..lane + 16].copy_from_slice(&a[lane..lane + 16 - n]);
                    }
                }
                out
            }
            #[inline(always)]
            fn bsrl<const N: i32>(self, a: Self::V) -> Self::V {
                let mut out = [0; $n];
                let n = N as usize;
                if n < 16 {
                    for lane in (0..$n).step_by(16) {
                        out[lane..lane + 16 - n].copy_from_slice(&a[lane + n..lane + 16]);
                    }
                }
                out
            }
            #[inline(always)]
            fn and(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i] & b[i];
                }
                out
            }
            #[inline(always)]
            fn andnot(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = !a[i] & b[i];
                }
                out
            }
            #[inline(always)]
            fn or(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i] | b[i];
                }
                out
            }
            #[inline(always)]
            fn xor(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(1) {
                    out[i] = a[i] ^ b[i];
                }
                out
            }
            #[inline(always)]
            fn addf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(portable_fx::arithmetic32(
                            f32::from_bits(u32at(&a, i)),
                            f32::from_bits(u32at(&b, i)),
                            f32::from_bits(u32at(&a, i)) + f32::from_bits(u32at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn subf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(portable_fx::arithmetic32(
                            f32::from_bits(u32at(&a, i)),
                            f32::from_bits(u32at(&b, i)),
                            f32::from_bits(u32at(&a, i)) - f32::from_bits(u32at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn mulf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(portable_fx::arithmetic32(
                            f32::from_bits(u32at(&a, i)),
                            f32::from_bits(u32at(&b, i)),
                            f32::from_bits(u32at(&a, i)) * f32::from_bits(u32at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn divf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(portable_fx::arithmetic32(
                            f32::from_bits(u32at(&a, i)),
                            f32::from_bits(u32at(&b, i)),
                            f32::from_bits(u32at(&a, i)) / f32::from_bits(u32at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn maxf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if f32::from_bits(u32at(&a, i)) > f32::from_bits(u32at(&b, i)) {
                            u32at(&a, i)
                        } else {
                            u32at(&b, i)
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn minf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if f32::from_bits(u32at(&a, i)) < f32::from_bits(u32at(&b, i)) {
                            u32at(&a, i)
                        } else {
                            u32at(&b, i)
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cmpltf(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(if f32::from_bits(u32at(&a, i)) < f32::from_bits(u32at(&b, i)) {
                            u32::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sqrtf(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(rt::fx::sqrt(f32::from_bits(u32at(&a, i))).to_bits()).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn addd(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(portable_fx::arithmetic64(
                            f64::from_bits(u64at(&a, i)),
                            f64::from_bits(u64at(&b, i)),
                            f64::from_bits(u64at(&a, i)) + f64::from_bits(u64at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn subd(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(portable_fx::arithmetic64(
                            f64::from_bits(u64at(&a, i)),
                            f64::from_bits(u64at(&b, i)),
                            f64::from_bits(u64at(&a, i)) - f64::from_bits(u64at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn muld(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(portable_fx::arithmetic64(
                            f64::from_bits(u64at(&a, i)),
                            f64::from_bits(u64at(&b, i)),
                            f64::from_bits(u64at(&a, i)) * f64::from_bits(u64at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn divd(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(portable_fx::arithmetic64(
                            f64::from_bits(u64at(&a, i)),
                            f64::from_bits(u64at(&b, i)),
                            f64::from_bits(u64at(&a, i)) / f64::from_bits(u64at(&b, i)),
                        )
                        .to_bits())
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn maxd(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(if f64::from_bits(u64at(&a, i)) > f64::from_bits(u64at(&b, i)) {
                            u64at(&a, i)
                        } else {
                            u64at(&b, i)
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn mind(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(if f64::from_bits(u64at(&a, i)) < f64::from_bits(u64at(&b, i)) {
                            u64at(&a, i)
                        } else {
                            u64at(&b, i)
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cmpltd(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(if f64::from_bits(u64at(&a, i)) < f64::from_bits(u64at(&b, i)) {
                            u64::MAX
                        } else {
                            0
                        })
                        .to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn sqrtd(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &(rt::fx::sqrtd(f64::from_bits(u64at(&a, i))).to_bits()).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cvt_i32_f32(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4]
                        .copy_from_slice(&((u32at(&a, i) as i32 as f32).to_bits()).to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn cvtt_f32_i32(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(rt::fx::cvtt(f32::from_bits(u32at(&a, i))) as u32).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cvtr_f32_i32(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(4) {
                    out[i..i + 4].copy_from_slice(
                        &(rt::fx::cvt(f32::from_bits(u32at(&a, i))) as u32).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cvt_i32_f64(self, a: Self::V) -> Self::V {
                let mut out = [0u8; $n];
                for i in (0..$n).step_by(8) {
                    out[i..i + 8].copy_from_slice(
                        &((u32at(&a, i / 2) as i32 as f64).to_bits()).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn cvtt_f64_i32(self, a: Self::V) -> Self::V {
                let mut out = [0; $n];
                for i in 0..$n / 8 {
                    out[i * 4..i * 4 + 4].copy_from_slice(
                        &rt::fx::cvttd(f64::from_bits(u64at(&a, i * 8))).to_le_bytes(),
                    );
                }
                out
            }
            #[inline(always)]
            fn store_lanes2(self, b: &mut [u8], lo: Self::V, hi: Self::V) -> () {
                for lane in 0..$n / 16 {
                    b[lane * 32..lane * 32 + 16].copy_from_slice(&lo[lane * 16..lane * 16 + 16]);
                    b[lane * 32 + 16..lane * 32 + 32]
                        .copy_from_slice(&hi[lane * 16..lane * 16 + 16]);
                }
            }
            #[inline(always)]
            fn load_lanes2(self, b: &[u8]) -> (Self::V, Self::V) {
                let mut lo = [0; $n];
                let mut hi = [0; $n];
                for lane in 0..$n / 16 {
                    lo[lane * 16..lane * 16 + 16].copy_from_slice(&b[lane * 32..lane * 32 + 16]);
                    hi[lane * 16..lane * 16 + 16]
                        .copy_from_slice(&b[lane * 32 + 16..lane * 32 + 32]);
                }
                (lo, hi)
            }
            #[inline(always)]
            fn load_i16_lanes(self, a: &[i16], b: &[i16]) -> Self::V {
                let mut out = [0; $n];
                for lane in 0..$n / 16 {
                    let from = if lane == 0 { a } else { b };
                    for i in 0..8 {
                        let dst = lane * 16 + i * 2;
                        out[dst..dst + 2].copy_from_slice(&from[i].to_le_bytes());
                    }
                }
                out
            }
            #[inline(always)]
            fn store_raw_lanes(self, a: &mut [u8], b: &mut [u8], v: Self::V) -> () {
                a[..8].copy_from_slice(&v[..8]);
                if $n > 16 {
                    b[..8].copy_from_slice(&v[16..24]);
                }
            }
            #[inline(always)]
            fn load_f32_lanes(self, a: &[f32], b: &[f32]) -> Self::V {
                let mut out = [0; $n];
                for lane in 0..$n / 16 {
                    let from = if lane == 0 { a } else { b };
                    for i in 0..4 {
                        let dst = lane * 16 + i * 4;
                        out[dst..dst + 4].copy_from_slice(&from[i].to_bits().to_le_bytes());
                    }
                }
                out
            }
            #[inline(always)]
            fn store_f32_lanes(self, a: &mut [f32], b: &mut [f32], v: Self::V) -> () {
                for i in 0..4 {
                    a[i] = f32::from_bits(u32at(&v, i * 4));
                }
                if $n > 16 {
                    for i in 0..4 {
                        b[i] = f32::from_bits(u32at(&v, 16 + i * 4));
                    }
                }
            }
            #[inline(always)]
            fn sadu8(self, a: Self::V, b: Self::V) -> Self::V {
                let mut out = [0; $n];
                for lane in (0..$n).step_by(8) {
                    let mut sum = 0u64;
                    for i in 0..8 {
                        sum += a[lane + i].abs_diff(b[lane + i]) as u64;
                    }
                    out[lane..lane + 8].copy_from_slice(&sum.to_le_bytes());
                }
                out
            }
            #[inline(always)]
            fn ptest_raw(self, a: Self::V, b: Self::V) -> u32 {
                let mut z = 0;
                let mut c = 0;
                for i in 0..$n / 2 {
                    z |= a[i] & b[i];
                    c |= !a[i] & b[i];
                }
                if z == 0 {
                    1
                } else if c == 0 {
                    2
                } else {
                    0
                }
            }
            #[inline(always)]
            fn ptestz_raw(self, a: Self::V, b: Self::V) -> bool {
                self.ptest_raw(a, b) == 1
            }
        }
    };
}
backend!(Scalar, 16);
backend!(Scalar256, 32);
