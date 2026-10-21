// ops -- whole-image operations on 32-bit surfaces (PARGB / ARGB / XRGB)  [src/img/imgops.s]
//
//   flip_h(sf)                                   mirror left <-> right (in place)  [img_flip_h]
//   flip_v(sf)                                   mirror top <-> bottom (in place)  [img_flip_v]
//   rotate180(sf)                                (in place)                     [img_rotate180]
//   rotate90(src, dir) -> Result<Surface, i32>   h x w; dir >= 0 clockwise, < 0 counter-
//                                                clockwise                      [img_rotate90]
//   crop(src, x, y, w, h) -> Result<Surface>     w x h; parts outside src transparent [img_crop]
//   resize_canvas(src, w, h, fill) -> Result<>   w x h filled with fill (premultiplied like
//                                                Dc::clear), src copied to its top-left
//                                                                          [img_resize_canvas]
//   blit_copy(dst, src, x, y)                    copy src to (x, y), no blending, clipped to
//                                                dst only (not to a draw context's clip)
//                                                                              [gfx_blit_copy]
//   scale(src, w, h, bilinear) -> Result<>       16.16 fixed-point nearest / bilinear (7-bit
//                                                fractions), edges clamped        [img_scale]
//   skew(src, kx, ky, fill) -> Result<>          x' = x + kx y / 1000, y' = y + ky x / 1000,
//                                                nearest, forward mapped; dst grows to fit
//                                                                                  [img_skew]
//   Band, Band::whole(sf), bands(sf, k)          disjoint row ranges of a surface, so worker
//                                                threads can write different rows at once
//                                                (the bands of rstest's rt_bands)
//
// Results keep the source format; the Err value is ERR_FAIL (-1, the assembly's -1) when a
// surface size is out of range.  Rows are 4 bytes per pixel (blit_copy, scale, skew and the
// flips copy whole pixels).

use alloc::vec::Vec;

use gfx::{Dc, FMT_A8, Surface};

use crate::ERR_FAIL;

const SURF_MAX: i32 = 65536;

/// rows y0 .. y1 (exclusive) of a surface of w x h pixels: `px` holds them from row y0 on,
/// row y at (y - y0) * stride.  What one worker of a parallel loop may write.
pub struct Band<'a> {
    pub px: &'a mut [u8],
    pub w: i32,
    pub h: i32,
    pub fmt: u8,
    pub stride: usize,
    pub y0: i32,
    pub y1: i32,
}

impl<'a> Band<'a> {
    /// every row of sf
    pub fn whole(sf: &'a mut Surface) -> Band<'a> {
        Band { w: sf.w, h: sf.h, fmt: sf.fmt, stride: sf.stride as usize, y0: 0, y1: sf.h, px: &mut sf.px[..] }
    }

    /// row y (y0 <= y < y1) from its first pixel to the end of the band
    #[inline]
    pub fn row(&mut self, y: i32) -> &mut [u8] {
        let o = (y - self.y0) as usize * self.stride;
        &mut self.px[o..]
    }
}

/// k bands of rows i * h / k .. (i + 1) * h / k (k < 1: one band)
pub fn bands(sf: &mut Surface, k: usize) -> Vec<Band<'_>> {
    let k = k.max(1);
    let (w, h, fmt, stride) = (sf.w, sf.h, sf.fmt, sf.stride as usize);
    let hh = h.max(0) as u64;
    let mut out = Vec::with_capacity(k);
    let mut rest: &mut [u8] = &mut sf.px[..];
    let mut y = 0i32;
    for i in 0..k {
        let y1 = ((i as u64 + 1) * hh / k as u64) as i32;
        let take = if i + 1 == k { rest.len() } else { ((y1 - y) as usize * stride).min(rest.len()) };
        let (a, b) = core::mem::take(&mut rest).split_at_mut(take);
        out.push(Band { px: a, w, h, fmt, stride, y0: y, y1 });
        rest = b;
        y = y1;
    }
    out
}

// surf_alloc: zeroed, sides 0 .. 65536 (unsigned compare: negative sizes fail too)
fn alloc(w: i32, h: i32, fmt: u8) -> Result<Surface, i32> {
    if w as u32 > SURF_MAX as u32 || h as u32 > SURF_MAX as u32 {
        return Err(ERR_FAIL);
    }
    Ok(Surface::new(w, h, fmt))
}

#[inline]
fn px_at(sf: &Surface, x: usize, y: usize) -> u32 {
    let o = y * sf.stride as usize + x * 4;
    u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]])
}

#[inline]
fn put(sf: &mut Surface, x: usize, y: usize, v: u32) {
    let o = y * sf.stride as usize + x * 4;
    sf.px[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// mirror left <-> right  [img_flip_h]
pub fn flip_h(sf: &mut Surface) {
    let (w, stride) = (sf.w.max(0) as usize, sf.stride as usize);
    let half = w / 2; // pixel pairs (an odd middle pixel stays)
    for y in 0..sf.h.max(0) as usize {
        let row = &mut sf.px[y * stride..y * stride + w * 4];
        let (left, rest) = row.split_at_mut(half * 4);
        let right = &mut rest[(w - 2 * half) * 4..];
        for (a, b) in left.chunks_exact_mut(4).zip(right.chunks_exact_mut(4).rev()) {
            let t: [u8; 4] = [a[0], a[1], a[2], a[3]];
            a.copy_from_slice(b);
            b.copy_from_slice(&t);
        }
    }
}

/// mirror top <-> bottom  [img_flip_v]
pub fn flip_v(sf: &mut Surface) {
    let (n, stride) = (sf.w.max(0) as usize * 4, sf.stride as usize);
    let (mut t, mut b) = (0i32, sf.h - 1);
    while t < b {
        let (top, bot) = sf.px.split_at_mut(b as usize * stride);
        top[t as usize * stride..t as usize * stride + n].swap_with_slice(&mut bot[..n]);
        t += 1;
        b -= 1;
    }
}

/// [img_rotate180]
pub fn rotate180(sf: &mut Surface) {
    flip_h(sf);
    flip_v(sf);
}

/// a new h x w surface: clockwise dst(H-1-y, x) = src(x, y) (dir >= 0), counter-clockwise
/// dst(y, W-1-x) = src(x, y)  [img_rotate90]
pub fn rotate90(src: &Surface, dir: i32) -> Result<Surface, i32> {
    let mut dst = alloc(src.h, src.w, src.fmt)?;
    let (w, h) = (src.w.max(0) as usize, src.h.max(0) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = px_at(src, x, y);
            if dir >= 0 {
                put(&mut dst, h - 1 - y, x, p);
            } else {
                put(&mut dst, y, w - 1 - x, p);
            }
        }
    }
    Ok(dst)
}

/// copy src to (x, y) of dst without blending, clipped to dst  [gfx_blit_copy]
pub fn blit_copy(dst: &mut Surface, src: &Surface, x: i32, y: i32) {
    for r in 0..src.h.max(0) {
        let dy = y.wrapping_add(r);
        if dy as u32 >= dst.h as u32 {
            continue; // also negative
        }
        let (mut dx, mut sx) = (x, 0i32);
        if dx < 0 {
            sx = sx.wrapping_sub(dx);
            dx = 0;
        }
        let cnt = src.w.wrapping_sub(sx).min(dst.w.wrapping_sub(dx));
        if cnt <= 0 {
            continue;
        }
        let n = cnt as usize * 4;
        let d = dy as usize * dst.stride as usize + dx as usize * 4;
        let s = r as usize * src.stride as usize + sx as usize * 4;
        dst.px[d..d + n].copy_from_slice(&src.px[s..s + n]);
    }
}

/// the w x h part of src at (x, y); outside parts transparent  [img_crop]
pub fn crop(src: &Surface, x: i32, y: i32, w: i32, h: i32) -> Result<Surface, i32> {
    let mut dst = alloc(w, h, src.fmt)?;
    blit_copy(&mut dst, src, x.wrapping_neg(), y.wrapping_neg());
    Ok(dst)
}

// gfx_begin + gfx_clear on a fresh target (fill premultiplied unless opaque)
fn clear(sf: &mut Surface, fill: u32) {
    Dc::new(sf.w, sf.h).clear(sf, fill);
}

/// w x h filled with fill, src copied to the top-left corner  [img_resize_canvas]
pub fn resize_canvas(src: &Surface, w: i32, h: i32, fill: u32) -> Result<Surface, i32> {
    let mut dst = alloc(w, h, src.fmt)?;
    clear(&mut dst, fill);
    blit_copy(&mut dst, src, 0, 0);
    Ok(dst)
}

/// w x h nearest / bilinear copy (16.16 steps, pixel centres)  [img_scale]
pub fn scale(src: &Surface, w: i32, h: i32, bilinear: bool) -> Result<Surface, i32> {
    if w <= 0 || h <= 0 {
        return Err(ERR_FAIL);
    }
    let mut dst = alloc(w, h, src.fmt)?;
    let xstep = ((src.w as u32 as u64) << 16) / w as u64;
    let ystep = ((src.h as u32 as u64) << 16) / h as u64;
    if bilinear && src.w > 0 && src.h > 0 {
        scale_bilinear(src, &mut dst, xstep, ystep);
        return Ok(dst);
    }
    for y in 0..h as usize {
        let fy = (y as u64).wrapping_mul(ystep).wrapping_add(ystep >> 1);
        for x in 0..w as usize {
            let fx = (x as u64).wrapping_mul(xstep).wrapping_add(xstep >> 1);
            let v = if bilinear {
                sample_bilinear(src, fx as i64, fy as i64)
            } else {
                px_at(src, (fx >> 16) as usize, ((fy >> 16) as i32) as usize)
            };
            put(&mut dst, x, y, v);
        }
    }
    Ok(dst)
}

// the bilinear scale separably: sample_bilinear's results with the column positions computed
// once and every source row lerped horizontally once (shared by the destination rows that use
// it).  The asm's lerp per byte, a + ((b - a) * f) >> 7 (psubw, pmullw, psraw, paddw; f < 128),
// equals (a * (128 - f) + b * f) >> 7: every term is non-negative and below 2^15, so the four
// bytes of a pixel are lerped at once in the 16-bit lanes of a u64 (from paint-tools' select.rs)
fn scale_bilinear(src: &Surface, dst: &mut Surface, xstep: u64, ystep: u64) {
    let (w, h) = (dst.w as usize, dst.h as usize);
    // sample point (pixel centres at + 0.5) -> (i0, i1, fraction) along one axis
    let axis = |i: usize, step: u64, n: i32| {
        let c = ((i as u64).wrapping_mul(step).wrapping_add(step >> 1) as i64).wrapping_sub(32768);
        let i0 = c >> 16;
        let m = (n - 1) as i64;
        (i0.max(0).min(m) as usize, (i0 + 1).max(0).min(m) as usize, ((c & 0xFFFF) >> 9) as u64)
    };
    let cols: Vec<(usize, usize, u64)> = (0..w).map(|x| axis(x, xstep, src.w)).collect();
    const M: u64 = 0x00FF_00FF_00FF_00FF;
    let spread = |p: u32| {
        let p = p as u64;
        (p & 0xFF) | (p & 0xFF00) << 8 | (p & 0xFF_0000) << 16 | (p & 0xFF00_0000) << 24
    };
    let lerp = |a: u64, b: u64, f: u64| ((a * (128 - f) + b * f) >> 7) & M;
    let hrow = |r: usize, out: &mut Vec<u64>| {
        out.clear();
        out.extend(cols.iter().map(|&(xa, xb, fx)| lerp(spread(px_at(src, xa, r)), spread(px_at(src, xb, r)), fx)));
    };
    let (mut h0, mut h1): (Vec<u64>, Vec<u64>) = (Vec::with_capacity(w), Vec::with_capacity(w));
    let (mut r0, mut r1) = (usize::MAX, usize::MAX);
    let ds = dst.stride as usize;
    for y in 0..h {
        let (ya, yb, fy) = axis(y, ystep, src.h);
        if ya != r0 {
            if ya == r1 {
                core::mem::swap(&mut h0, &mut h1);
                r1 = r0;
            } else {
                hrow(ya, &mut h0);
            }
            r0 = ya;
        }
        if yb != r1 {
            hrow(yb, &mut h1);
            r1 = yb;
        }
        let drow = &mut dst.px[y * ds..y * ds + w * 4];
        for (d, (&t, &b)) in drow.chunks_exact_mut(4).zip(h0.iter().zip(h1.iter())) {
            let v = lerp(t, b, fy);
            let px = (v & 0xFF) | (v >> 8 & 0xFF00) | (v >> 16 & 0xFF_0000) | (v >> 24 & 0xFF00_0000);
            d.copy_from_slice(&(px as u32).to_le_bytes());
        }
    }
}

// 16.16 sample point (pixel centres at + 0.5) with 7-bit fractions, edges clamped
fn sample_bilinear(src: &Surface, fx: i64, fy: i64) -> u32 {
    let cx = fx.wrapping_sub(32768);
    let cy = fy.wrapping_sub(32768);
    let (x0, y0) = (cx >> 16, cy >> 16);
    let fxw = ((cx & 0xFFFF) >> 9) as i16;
    let fyw = ((cy & 0xFFFF) >> 9) as i16;
    let (wm, hm) = ((src.w - 1) as i64, (src.h - 1) as i64);
    let cl = |v: i64, m: i64| v.max(0).min(m) as usize;
    let (xa, xb) = (cl(x0, wm), cl(x0 + 1, wm));
    let (ya, yb) = (cl(y0, hm), cl(y0 + 1, hm));
    let (p00, p10) = (px_at(src, xa, ya), px_at(src, xb, ya));
    let (p01, p11) = (px_at(src, xa, yb), px_at(src, xb, yb));
    // per channel, 16-bit signed lanes: a + ((b - a) * f) >> 7 (psubw, pmullw, psraw, paddw)
    let lerp = |a: i16, b: i16, f: i16| a.wrapping_add(b.wrapping_sub(a).wrapping_mul(f) >> 7);
    let mut out = 0u32;
    for k in 0..4 {
        let c = |p: u32| ((p >> (k * 8)) & 255) as i16;
        let t = lerp(c(p00), c(p10), fxw);
        let b = lerp(c(p01), c(p11), fxw);
        let v = lerp(t, b, fyw).clamp(0, 255) as u32; // packuswb
        out |= v << (k * 8);
    }
    out
}

/// shear by kx, ky in 1/1000 (nearest, every source pixel mapped forward); the dst grows by
/// |kx h / 1000| x |ky w / 1000| and is filled with fill first  [img_skew]
pub fn skew(src: &Surface, kx: i32, ky: i32, fill: u32) -> Result<Surface, i32> {
    let rx = kx.wrapping_mul(src.h) / 1000;
    let ry = ky.wrapping_mul(src.w) / 1000;
    let ox = if rx < 0 { rx.wrapping_neg() } else { 0 };
    let oy = if ry < 0 { ry.wrapping_neg() } else { 0 };
    let dw = rx.wrapping_abs().wrapping_add(src.w);
    let dh = ry.wrapping_abs().wrapping_add(src.h);
    let mut dst = alloc(dw, dh, src.fmt)?;
    clear(&mut dst, fill);
    for y in 0..src.h.max(0) {
        for x in 0..src.w.max(0) {
            let xd = (kx.wrapping_mul(y) / 1000).wrapping_add(x).wrapping_add(ox);
            let yd = (ky.wrapping_mul(x) / 1000).wrapping_add(y).wrapping_add(oy);
            if (xd as u32) < dst.w as u32 && (yd as u32) < dst.h as u32 {
                let p = px_at(src, x as usize, y as usize);
                put(&mut dst, xd as usize, yd as usize, p);
            }
        }
    }
    Ok(dst)
}

/// bytes per pixel of a format (A8: 1, else 4)
#[inline]
pub fn bpp(fmt: u8) -> usize {
    if fmt == FMT_A8 { 1 } else { 4 }
}
