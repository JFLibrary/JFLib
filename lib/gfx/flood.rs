// flood -- flood fill (magic wand / paint bucket region) into a tiled mask   [src/gfx/flood.s]
//
//   flood_fill_mask(src, x, y, tolerance_pct, flags, out) -> 0 / ERR_FAIL
//
// src: 32-bit surface (FMT_ARGB straight; the colour test works on the raw B, G, R, A
// bytes); out: a TMask at least as large as src.  out is cleared, then every pixel that
// matches the seed colour (x, y) and -- without FF_GLOBAL -- is 4-connected to the seed
// through matching pixels becomes 255.  ERR_FAIL: bad surface, mask too small, seed outside
// src (out is cleared when it is usable).
//
// Colour test = paint.net's CheckColor, a = seed colour, b = candidate:
//   sum = sum over c in R, G, B of floor((1 + (a.c - b.c)^2) * a.A / 256) + (a.A - b.A)^2
//   tol = round(tolerance_pct * 255 / 100) (pct clamped to 0..100), match: sum <= tol*tol*4
// As in the assembly: a match bitmap M (1 bit per pixel, rows of ceil(w / 64) u64) computed
// per row on demand by a SIMD kernel (FF_MATCH4, integer arithmetic, so SSE2 and AVX2 give
// the same bits); the contiguous fill is a scanline fill on the bitmaps with an explicit stack
// of run starts, filled runs move from M to the fill bitmap F (word operations); finally
// 64 x 64 bit blocks become Empty / Full slots or tiles.

use alloc::vec;
use alloc::vec::Vec;

use super::tmask::TMask;
use super::{ERR_FAIL, FMT_A8, Surface};
use simd::{Kernel, Simd};

pub const FF_GLOBAL: u32 = 1; // every matching pixel, not only the connected ones

// ---- the match kernel (FF_TERMS / FF_MATCH4): BYTES / 4 pixels per step

// d^2 lanes (B G R A) -> terms: B, G, R ((1 + d^2) * seed.A) >> 8, A d^2.  aa8 = seed.A << 8:
// the high word of (1 + d^2) * (A << 8) is the product >> 8 (it stays below 2^24), the
// assembly's pmullw / pmulhuw / shift / or pair in one multiply (LLVM would otherwise
// rebuild that pair as a slow 32-bit pmulld)
#[inline(always)]
fn terms<S: Simd>(s: S, r: S::V, aa8: S::V) -> S::V {
    let t = s.add16(r, s.splat16(1));
    let x = s.mulhiu16(t, aa8);
    s.or(s.and(x, s.splat64(0x0000_FFFF_FFFF_FFFF)), s.and(r, s.splat64(0xFFFF_0000_0000_0000)))
}

// match bits of the BYTES / 4 pixels in v
#[inline(always)]
fn match_bits<S: Simd>(s: S, v: S::V, seed: S::V, aa8: S::V, thr: S::V) -> u32 {
    let z = s.zero();
    let d = s.or(s.subus8(v, seed), s.subus8(seed, v)); // |a - b| per byte
    let lo = s.unpacklo8(d, z);
    let hi = s.unpackhi8(d, z);
    let lo = terms(s, s.mullo16(lo, lo), aa8); // d^2 <= 65025
    let hi = terms(s, s.mullo16(hi, hi), aa8);
    let (bias, w1) = (s.splat16(0x8000), s.splat16(1));
    let lo = s.madd16(s.xor(lo, bias), w1); // pair sums - 65536
    let hi = s.madd16(s.xor(hi, bias), w1);
    let lo = s.add32(lo, s.shuf32::<0xB1>(lo)); // [s0, s0, s1, s1] - 131072
    let hi = s.add32(hi, s.shuf32::<0xB1>(hi));
    let sums = s.shufps::<0x88>(lo, hi); // [s0, s1, s2, s3] - 131072
    s.movemask32(s.cmpgts32(thr, sums)) // sum <= tol*tol*4
}

// one row of the match bitmap
struct RowK<'a> {
    px: &'a [u8], // w pixels
    out: &'a mut [u64],
    seed: u32,
    thr: u32, // tol*tol*4 + 1 - 131072
}

impl Kernel for RowK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let RowK { px, out, seed, thr } = self;
        let w = px.len() / 4;
        let nb = S::BYTES;
        let g = nb / 4; // pixels per step (divides 64)
        let (sv, aa8, tv) = (s.splat32(seed), s.splat16((seed >> 24 << 8) as u16), s.splat32(thr));
        let mut x = 0;
        for word in out.iter_mut() {
            let mut bits = 0u64;
            let mut k = 0;
            while k < 64 && x < w {
                let m = if w - x >= g {
                    match_bits(s, s.load(&px[x * 4..x * 4 + nb]), sv, aa8, tv) as u64
                } else {
                    // the last pixels through a zero-padded copy
                    let mut t = [0u8; 32];
                    t[..(w - x) * 4].copy_from_slice(&px[x * 4..w * 4]);
                    match_bits(s, s.load(&t), sv, aa8, tv) as u64 & ((1 << (w - x)) - 1)
                };
                bits |= m << k;
                k += g;
                x += g;
            }
            *word = bits;
        }
    }
}

// ---- the fill state

struct Ctx<'a> {
    src: &'a Surface,
    w: u32,
    ww: usize, // u64 words per bitmap row
    seed: u32,
    thr: u32,
    m: Vec<u64>,
    done: Vec<bool>,
}

impl Ctx<'_> {
    // the match bitmap row y (once)  [ff_row]
    fn row(&mut self, y: u32) {
        if self.done[y as usize] {
            return;
        }
        self.done[y as usize] = true;
        let o = y as usize * self.src.stride as usize;
        let px = &self.src.px[o..o + self.w as usize * 4];
        let out = &mut self.m[y as usize * self.ww..(y as usize + 1) * self.ww];
        simd::dispatch(RowK { px, out, seed: self.seed, thr: self.thr });
    }

    // push the start of every run of unfilled matching pixels of row y in [xl, xr]  [ff_scan]
    fn scan(&mut self, y: u32, xl: u32, xr: u32, stack: &mut Vec<(u32, u32)>) {
        self.row(y);
        let row = &self.m[y as usize * self.ww..(y as usize + 1) * self.ww];
        let mut carry = 0u64;
        for wi in xl >> 6..=xr >> 6 {
            let mut v = row[wi as usize];
            if wi == xl >> 6 {
                v &= u64::MAX << (xl & 63); // from xl
            }
            if wi == xr >> 6 {
                v &= u64::MAX >> ((xr & 63) ^ 63); // to xr
            }
            let mut starts = !(v << 1 | carry) & v;
            carry = v >> 63;
            while starts != 0 {
                stack.push((wi << 6 | starts.trailing_zeros(), y));
                starts &= starts - 1;
            }
        }
    }
}

// move the run [xl, xr] of row y from m to f  [ff_fill]
fn fill_run(m: &mut [u64], f: &mut [u64], xl: u32, xr: u32) {
    for wi in xl >> 6..=xr >> 6 {
        let mut b = u64::MAX;
        if wi == xr >> 6 {
            b >>= (xr & 63) ^ 63; // up to xr
        }
        if wi == xl >> 6 {
            b = b >> (xl & 63) << (xl & 63); // from xl
        }
        f[wi as usize] |= b;
        m[wi as usize] &= !b;
    }
}

/// fill the region of the seed (x, y) into out  [flood_fill_mask]
pub fn flood_fill_mask(src: &Surface, x: i32, y: i32, tolerance_pct: i32, flags: u32, out: &mut TMask) -> i32 {
    if src.fmt == FMT_A8 || src.w <= 0 || src.h <= 0 || src.px.is_empty() {
        return ERR_FAIL;
    }
    if (out.w as i32) < src.w || (out.h as i32) < src.h {
        return ERR_FAIL;
    }
    out.clear(0);
    let (w, h) = (src.w as u32, src.h as u32);
    if x as u32 >= w || y as u32 >= h {
        return ERR_FAIL;
    }
    let (x, y) = (x as u32, y as u32);
    let o = y as usize * src.stride as usize + x as usize * 4;
    let seed = u32::from_le_bytes([src.px[o], src.px[o + 1], src.px[o + 2], src.px[o + 3]]);
    let pct = tolerance_pct.clamp(0, 100) as u32;
    let tol = (pct * 255 + 50) / 100;
    let ww = w.div_ceil(64) as usize;
    let thr = (tol * tol * 4).wrapping_add(1).wrapping_sub(131072);
    let mut c = Ctx { src, w, ww, seed, thr, m: vec![0u64; ww * h as usize], done: vec![false; h as usize] };
    if flags & FF_GLOBAL != 0 {
        for r in 0..h {
            c.row(r);
        }
        emit(&c.m, ww, w, h, out);
        return 0;
    }
    // contiguous: scanline fill; filled runs move from the match bitmap to f
    let mut f = vec![0u64; ww * h as usize];
    let mut stack: Vec<(u32, u32)> = Vec::with_capacity(4096);
    c.row(y);
    stack.push((x, y));
    while let Some((px, py)) = stack.pop() {
        let mrow = &c.m[py as usize * ww..(py as usize + 1) * ww];
        let wi = (px >> 6) as usize;
        let cur = mrow[wi];
        if cur >> (px & 63) & 1 == 0 {
            continue; // already filled
        }
        // extend left: xl = first pixel of the run of set bits containing x
        let mut z = !cur & !(u64::MAX << (px & 63)); // zero bits below x in this word
        let mut i = wi;
        let xl = loop {
            if z != 0 {
                break ((i as u32) << 6) + (63 - z.leading_zeros()) + 1;
            }
            if i == 0 {
                break 0;
            }
            i -= 1;
            z = !mrow[i];
        };
        // extend right: xr = last pixel of the run
        let mut z = !cur & (2u64 << (px & 63)).wrapping_neg(); // zero bits above x (0 at bit 63)
        let mut i = wi;
        let xr = loop {
            if z != 0 {
                break ((i as u32) << 6 | z.trailing_zeros()) - 1;
            }
            i += 1;
            if i >= ww {
                break w - 1;
            }
            z = !mrow[i];
        };
        let r = py as usize * ww..(py as usize + 1) * ww;
        fill_run(&mut c.m[r.clone()], &mut f[r], xl, xr);
        // runs above and below
        if py > 0 {
            c.scan(py - 1, xl, xr, &mut stack);
        }
        if py + 1 < h {
            c.scan(py + 1, xl, xr, &mut stack);
        }
    }
    emit(&f, ww, w, h, out);
    0
}

// byte i = 255 when bit i of the index is set  [ffc_bx8]
static BX8: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut j = 0;
    while j < 256 {
        let mut v = 0u64;
        let mut i = 0;
        while i < 8 {
            if j >> i & 1 != 0 {
                v |= 0xFF << (i * 8);
            }
            i += 1;
        }
        t[j] = v;
        j += 1;
    }
    t
};

// the bitmap into the mask: 64 x 64 blocks of zeros stay empty, blocks whose valid bits are
// all set become Full, others tiles of 0 / 255 bytes  [ff_emit]
fn emit(bm: &[u64], ww: usize, w: u32, h: u32, out: &mut TMask) {
    let mut ty = 0u32;
    while ty * 64 < h {
        let vh = (h - ty * 64).min(64) as usize;
        for tx in 0..ww as u32 {
            let vw = (w - tx * 64).min(64);
            let valid = if vw >= 64 { u64::MAX } else { (1u64 << vw) - 1 };
            let first = (ty * 64) as usize * ww + tx as usize;
            let (mut any, mut all) = (0u64, u64::MAX);
            for r in 0..vh {
                let v = bm[first + r * ww];
                any |= v;
                all &= v;
            }
            if any == 0 {
                continue;
            }
            if all & valid == valid {
                out.tile_set(tx, ty, true);
                continue;
            }
            if let Some(tile) = out.tile_w(tx, ty) {
                for r in 0..vh {
                    let v = bm[first + r * ww];
                    for (k, b) in tile[r * 64..r * 64 + 64].chunks_exact_mut(8).enumerate() {
                        b.copy_from_slice(&BX8[(v >> (k * 8)) as usize & 255].to_le_bytes());
                    }
                }
            }
        }
        ty += 1;
    }
}
