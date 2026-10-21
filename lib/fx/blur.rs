// blur -- effect group "blur" (Weichzeichnen)                              [src/fx/fx_blur.s]
//
//   FXTAB            the group table
//
// Effects: bokeh_blur, fragment_blur, gaussian_blur, median, motion_blur, radial_blur,
// sketch_blur, square_blur, surface_blur, unfocus, zoom_blur
//   gaussian_blur  radius r (0..200): Gaussian with 12 sigma^2 = 2 r (r + 2), premultiplied,
//                  edges clamped (conv::sep_run)
//   square_blur    mean of the (2r+1)^2 square (box, running sums)
//   unfocus        mean over the paint.net disc (conv::disc_run)
//   median         per channel percentile of the disc (lochist; colour histograms skip alpha 0)
//   motion_blur    N = (1 + d) * 3 / 2 bilinear samples (1/16 px) along the direction, mean of
//                  the valid ones (interior pixels: collapsed taps, the same integer sum)
//   zoom_blur      paint.net 3's zoom blur in 64-bit fixed point, ColorBgra.Blend
//   radial_blur    paint.net 3's radial blur (64-bit), ColorBgra.Blend
//   fragment_blur  paint.net 3's FragmentEffect
//   surface_blur   histogram bilateral filter over the disc
//   bokeh_blur     disc mean in linear light with boosted highlights (f32 sums)
//   sketch_blur    three layers of stroke cells, 2R + 1 bilinear samples along each cell's
//                  direction

use alloc::vec;
use alloc::vec::Vec;

use gfx::FMT_ARGB;
use img_core::resample::{from_work, RS_GAMMA};
use rt::fx::{cvt, cvtd, cvttd, cvttd64, maxss, minss};
use simd::{Simd, Sse2};

use crate::conv::*;
use crate::lochist::*;
use crate::*;

const DEG2RAD: f64 = 0.017453292519943295;
const SIGN: u64 = 0x8000_0000_0000_0000;

#[inline]
fn neg(v: f64) -> f64 {
    f64::from_bits(v.to_bits() ^ SIGN)
}

/// s + {B*A, G*A, R*A, A} of a straight pixel (pmullw as the asm)  [FXB_ACC]
#[inline(always)]
pub(crate) fn acc_px(s: V, p: u32) -> V {
    let x = Sse2;
    let z = x.zero();
    let v = x.unpacklo8(x.from_i32(p as i32), z); // b g r a (u16)
    let prod = x.mullo16(v, x.shuf16::<0xFF>(v)); // b a, g a, r a, a a
    let m = x.splat64(0x0000_FFFF_FFFF_FFFF);
    let t = x.or(x.and(prod, m), x.andnot(m, v));
    x.add32(s, x.unpacklo16(t, z))
}

/// the source pixel (x, y) (inside the image)
#[inline(always)]
fn spx(sp: &[u8], stride: usize, x: usize, y: usize) -> u32 {
    let o = y * stride + x * 4;
    let b: [u8; 4] = sp[o..o + 4].try_into().unwrap();
    u32::from_le_bytes(b)
}

/// paint.net ColorBgra.Blend of alpha-weighted sums: n = 0 or sA = 0 -> 0, else c = sC / sA,
/// a = sA / n  [fxb_blend_out]
#[inline]
pub(crate) fn blend_out(v: V, n: u32) -> u32 {
    let mut t = [0i32; 4];
    Sse2.store_i32(&mut t, v);
    let s = t.map(|c| c as u32);
    let sa = s[3];
    if n == 0 || sa == 0 {
        return 0;
    }
    (sa / n) << 24 | (s[2] / sa) << 16 | (s[1] / sa) << 8 | s[0] / sa
}

// ---------------------------------------------------------------- Gaussian / square / unfocus

fn gauss_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 0, 200);
    if r == 0 {
        return copy_roi(a, dst);
    }
    let k = Kern::gauss(var12(r));
    match Sep::new(a, &k) {
        Some(sep) => sep_run(&sep, &mut EmitArgb(dst)),
        None => 0,
    }
}

fn square_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 0, 200);
    if r == 0 {
        return copy_roi(a, dst);
    }
    let k = Kern::boxk(r);
    match Sep::new(a, &k) {
        Some(sep) => sep_run(&sep, &mut EmitArgb(dst)),
        None => 0,
    }
}

fn unfocus_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 200);
    match Disc::new(a, r) {
        Some(mut d) => disc_run(&mut d, &mut DiscArgb(dst)),
        None => 0,
    }
}

// ---------------------------------------------------------------- Median

/// a pixel of per-channel percentiles (colour: ncol pixels, alpha: nall)  [fxb_pct4]
pub(crate) fn pct4(h: &Hist, pct: u32) -> u32 {
    let mut r = 0u32;
    if h.ncol != 0 {
        let t = (h.ncol.wrapping_mul(pct) / 100) as i32;
        for c in 0..3 {
            r |= lh_percentile(h, c, t) << (8 * c);
        }
    }
    let t = (h.nall.wrapping_mul(pct) / 100) as i32;
    r | lh_percentile(h, 3, t) << 24
}

fn median_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 200);
    let pct = pint(a, 1, 0, 100) as u32;
    lh_run(a, dst, r, LHS_DISC, true, |h, _, _| pct4(h, pct))
}

// ---------------------------------------------------------------- Motion Blur

const MB_STRIP: i32 = 32;

type V = <Sse2 as Simd>::V;

/// the two pixels of a register interleaved channel by channel (the pmaddwd operand)
#[inline(always)]
fn pair(s: Sse2, v: V) -> V {
    s.shuf16::<0xD8>(s.shuf32::<0xD8>(v))
}

/// bilinear sample (1/16 px, inside the image) from a W16 window: rows of bw pixels from
/// (bx0, by0); i32 sums += P00 w00 + P10 w10 + P01 w01 + P11 w11 (pmaddwd of pixel pairs;
/// at the right / bottom edge the missing neighbour has weight 0, so the pixel itself takes
/// its place exactly)  [SK_BILIN]
#[inline(always)]
fn bilin(acc: V, win: &[i16], bw: i32, bx0: i32, by0: i32, w: i32, h: i32, xs: i32, ys: i32) -> V {
    let s = Sse2;
    let (fx, ix) = (xs & 15, xs >> 4);
    let (fy, iy) = (ys & 15, ys >> 4);
    let rs = if iy + 1 < h { bw as usize * 4 } else { 0 };
    let o = (((iy - by0) * bw + ix - bx0) * 4) as usize;
    let (mut w00, mut w10) = ((16 - fx) * (16 - fy), fx * (16 - fy));
    let (mut w01, mut w11) = ((16 - fx) * fy, fx * fy);
    if ix + 1 >= w {
        w00 += w10;
        w10 = 0;
        w01 += w11;
        w11 = 0;
    }
    let a = pair(s, s.load_i16(&win[o..]));
    let b = pair(s, s.load_i16(&win[o + rs..]));
    let acc = s.add32(acc, s.madd16(a, s.splat32(w00 as u32 | (w10 as u32) << 16)));
    s.add32(acc, s.madd16(b, s.splat32(w01 as u32 | (w11 as u32) << 16)))
}

/// rint(f32(sum) * inv) per channel, saturated to i16
#[inline(always)]
fn mean16(acc: V, inv: f32, out: &mut [i16]) {
    let s = Sse2;
    let v = s.cvtr_f32_i32(s.mulf(s.cvt_i32_f32(acc), s.splatf(inv)));
    let mut t = [0i16; 8];
    s.store_i16(&mut t, s.packs32(v, v));
    out[..4].copy_from_slice(&t[..4]);
}

/// mean16 of two pixels (out: 8 values)
#[inline(always)]
fn mean16x2(a: V, b: V, inv: f32, out: &mut [i16]) {
    let s = Sse2;
    let f = s.splatf(inv);
    let va = s.cvtr_f32_i32(s.mulf(s.cvt_i32_f32(a), f));
    let vb = s.cvtr_f32_i32(s.mulf(s.cvt_i32_f32(b), f));
    s.store_i16(out, s.packs32(va, vb));
}

fn motion_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let (w, h) = (a.src.w, a.src.h);
    let d = pint(a, 1, 1, 200);
    let cen = pint(a, 2, 0, 1);
    let n = ((d + 1) as u32 * 3 >> 1) as i32;
    let (s, c) = gfx::xform::sincos((pdbl(a, 0) + 180.0) * DEG2RAD);
    let dd = d as f64;
    let mut ex = c * dd;
    let mut ey = neg(s * dd);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    if cen != 0 {
        ex *= 0.5;
        ey *= 0.5;
        sx = neg(ex);
        sy = neg(ey);
    }
    let mut pts = vec![(0i32, 0i32); n as usize];
    let (mut minpx, mut maxpx, mut minpy, mut maxpy) = (i32::MAX, -i32::MAX, i32::MAX, -i32::MAX);
    for (j, pt) in pts.iter_mut().enumerate() {
        let t = if n - 1 == 0 { 0.0 } else { j as i32 as f64 / (n - 1) as f64 };
        let px = cvtd(((ex - sx) * t + sx) * 16.0);
        let py = cvtd(((ey - sy) * t + sy) * 16.0);
        *pt = (px, py);
        minpx = minpx.min(px);
        maxpx = maxpx.max(px);
        minpy = minpy.min(py);
        maxpy = maxpy.max(py);
    }
    let (ixmin, ixmax) = (minpx >> 4, (maxpx >> 4) + 1);
    let (iymin, iymax) = (minpy >> 4, (maxpy >> 4) + 1);
    let gw = ixmax - ixmin + 1;
    let gh = iymax - iymin + 1;
    let mut grid = vec![0i32; (gw * gh) as usize + gw as usize + 2];
    for &(px, py) in &pts {
        let (fx, gx) = (px & 15, (px >> 4) - ixmin);
        let (fy, gy) = (py & 15, (py >> 4) - iymin);
        let i = (gy * gw + gx) as usize;
        grid[i] += (16 - fx) * (16 - fy);
        grid[i + 1] += fx * (16 - fy);
        grid[i + gw as usize] += (16 - fx) * fy;
        grid[i + gw as usize + 1] += fx * fy;
    }
    let bx0 = (rect[0] + ixmin).max(0);
    let bx1 = (rect[2] + ixmax).min(w);
    let bw = bx1 - bx0;
    // collapsed taps: (window offset, 16-bit weight), padded to pairs with (0, 0)
    let mut taps: Vec<(isize, u32)> = Vec::new();
    for gy in 0..gh {
        for gx in 0..gw {
            let wv = grid[(gy * gw + gx) as usize];
            if wv != 0 {
                taps.push((((gy + iymin) * bw + gx + ixmin) as isize * 4, wv as u16 as u32));
            }
        }
    }
    if taps.len() % 2 == 1 {
        taps.push((0, 0));
    }
    // tap pairs: (offset a, offset b, weights a | b << 16)
    let tp: Vec<(isize, isize, u32)> = taps.chunks_exact(2).map(|t| (t[0].0, t[1].0, t[0].1 | t[1].1 << 16)).collect();
    let invn = 1.0f32 / (n << 8) as f32;
    let xlo = -(minpx >> 4);
    let xhi = ((w - 1) << 4).wrapping_sub(maxpx) >> 4;
    let ylo = -(minpy >> 4);
    let yhi = ((h - 1) << 4).wrapping_sub(maxpy) >> 4;
    let tw = (rect[2] - rect[0]) as usize;
    let mut win = vec![0i16; ((MB_STRIP + gh + 1) * bw) as usize * 4 + 8];
    let mut out = vec![0i16; tw * 4];
    let s = Sse2;
    let mut sy0 = rect[1];
    while sy0 < rect[3] {
        let sy1 = (sy0 + MB_STRIP).min(rect[3]);
        let by0 = (sy0 + iymin).max(0);
        let by1 = (sy1 + iymax).min(h);
        for (k, y) in (by0..by1).enumerate() {
            let o = k * bw as usize * 4;
            load_row(&mut win[o..], a.src, y, bx0, bx1, 0);
        }
        for y in sy0..sy1 {
            if a.cancelled() {
                return ERR_CANCEL;
            }
            let (fx0, fx1) = if y >= ylo && y <= yhi { (xlo, xhi + 1) } else { (1, 0) };
            let mut x = rect[0];
            while x < rect[2] {
                let o = (x - rect[0]) as usize * 4;
                let pix = (((y - by0) * bw + x - bx0) * 4) as isize;
                if x >= fx0 && x + 4 <= fx1.min(rect[2]) {
                    // four pixels at once (integer sums: the order of the taps does not matter)
                    let (mut a0, mut a1, mut a2, mut a3) = (s.zero(), s.zero(), s.zero(), s.zero());
                    for t in &tp {
                        let pa: &[i16; 16] = win[(pix + t.0) as usize..][..16].try_into().unwrap();
                        let pb: &[i16; 16] = win[(pix + t.1) as usize..][..16].try_into().unwrap();
                        let wv = s.splat32(t.2);
                        let (al, ah) = (s.load_i16(&pa[..8]), s.load_i16(&pa[8..]));
                        let (bl, bh) = (s.load_i16(&pb[..8]), s.load_i16(&pb[8..]));
                        a0 = s.add32(a0, s.madd16(s.unpacklo16(al, bl), wv));
                        a1 = s.add32(a1, s.madd16(s.unpackhi16(al, bl), wv));
                        a2 = s.add32(a2, s.madd16(s.unpacklo16(ah, bh), wv));
                        a3 = s.add32(a3, s.madd16(s.unpackhi16(ah, bh), wv));
                    }
                    let o16: &mut [i16; 16] = (&mut out[o..o + 16]).try_into().unwrap();
                    mean16x2(a0, a1, invn, &mut o16[..8]);
                    mean16x2(a2, a3, invn, &mut o16[8..]);
                    x += 4;
                    continue;
                }
                if x >= fx0 && x < fx1 {
                    let mut acc = s.zero();
                    for t in &tp {
                        let a = s.load_i16(&win[(pix + t.0) as usize..]);
                        let b = s.load_i16(&win[(pix + t.1) as usize..]);
                        acc = s.add32(acc, s.madd16(s.unpacklo16(a, b), s.splat32(t.2)));
                    }
                    mean16(acc, invn, &mut out[o..]);
                } else {
                    let mut acc = s.zero();
                    let mut valid = 0i32;
                    for &(px, py) in &pts {
                        let xs = (x << 4) + px;
                        let ys = (y << 4) + py;
                        if xs < 0 || xs > (w - 1) << 4 || ys < 0 || ys > (h - 1) << 4 {
                            continue;
                        }
                        valid += 1;
                        acc = bilin(acc, &win, bw, bx0, by0, w, h, xs, ys);
                    }
                    if valid == 0 {
                        let q = pix as usize;
                        let v = [win[q], win[q + 1], win[q + 2], win[q + 3]];
                        out[o..o + 4].copy_from_slice(&v);
                    } else {
                        mean16(acc, 1.0f32 / (valid << 8) as f32, &mut out[o..]);
                    }
                }
                x += 1;
            }
            from_work(dst_px(dst, y, rect[0], tw as i32), &out, tw, FMT_ARGB, 0);
        }
        sy0 = sy1;
    }
    0
}

// ---------------------------------------------------------------- Zoom / Radial / Fragment

fn zoom_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let (w, h) = (a.src.w, a.src.h);
    let fz = pint(a, 0, 0, 100) as i64;
    let fcx = cvttd64(pdbl(a, 1) * w as f64 * 32768.0) + ((w as i64) << 15);
    let fcy = cvttd64(pdbl(a, 2) * h as f64 * 32768.0) + ((h as i64) << 15);
    let (sp, stride) = (&a.src.px[..], a.src.stride as usize);
    // the 64 sample columns of every x depend on x only, the sample rows on y only: tables of
    // byte offsets (u32::MAX / usize::MAX = outside the image; the samples are skipped as the
    // asm does, in the same order)
    let tw = (rect[2] - rect[0]) as usize;
    let mut ut = vec![0u32; tw * ZB_N];
    for (x, us) in (rect[0]..rect[2]).zip(ut.chunks_exact_mut(ZB_N)) {
        let mut fx = ((x as i64) << 16) - fcx;
        for o in us.iter_mut() {
            fx -= ((fx >> 4).wrapping_mul(fz)) >> 10;
            let u = (fx + fcx + 32768) >> 16;
            *o = if u < 0 || u >= w as i64 { u32::MAX } else { u as u32 * 4 };
        }
    }
    let mut vt = [0usize; ZB_N];
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let mut fy = ((y as i64) << 16) - fcy;
        for o in vt.iter_mut() {
            fy -= ((fy >> 4).wrapping_mul(fz)) >> 10;
            let v = (fy + fcy + 32768) >> 16;
            *o = if v < 0 || v >= h as i64 { usize::MAX } else { v as usize * stride };
        }
        let srow = &a.src_row(y)[rect[0] as usize * 4..rect[2] as usize * 4];
        let d = &mut dst_px(dst, y, rect[0], tw as i32)[..];
        for ((dp, sq), us) in d.chunks_exact_mut(4).zip(srow.chunks_exact(4)).zip(ut.chunks_exact(ZB_N)) {
            let mut s = acc_px(Sse2.zero(), u32::from_le_bytes([sq[0], sq[1], sq[2], sq[3]]));
            let mut n = 1u32;
            for (&uo, &vo) in us.iter().zip(vt.iter()) {
                if uo == u32::MAX || vo == usize::MAX {
                    continue;
                }
                let o = vo + uo as usize;
                let q: [u8; 4] = sp[o..o + 4].try_into().unwrap();
                s = acc_px(s, u32::from_le_bytes(q));
                n += 1;
            }
            dp.copy_from_slice(&blend_out(s, n).to_le_bytes());
        }
    }
    0
}

/// samples per pixel of the zoom blur
const ZB_N: usize = 64;

/// rotate (x, y) by fsr (fsr2 = fsr^2 >> 11)  [RB_ROT]
#[inline(always)]
fn rb_rot(x: &mut i64, y: &mut i64, fsr: i64, fsr2: i64) {
    let ra = ((*y >> 8).wrapping_mul(fsr)) >> 8;
    let rc = ((*x >> 14).wrapping_mul(fsr2)) >> 8;
    let rd = ((*x >> 8).wrapping_mul(fsr)) >> 8;
    let r8 = ((*y >> 14).wrapping_mul(fsr2)) >> 8;
    *x = x.wrapping_sub(ra).wrapping_sub(rc);
    *y = y.wrapping_add(rd).wrapping_sub(r8);
}

fn radial_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let (w, h) = (a.src.w, a.src.h);
    let q = pint(a, 3, 1, 5);
    let nn = (q * q + 30) * (q * q);
    let fr = cvttd64(pdbl(a, 0) * 3.141592653589793 * 65536.0 / 181.0);
    let fsr = fr / nn as i64;
    let fsr2 = fsr.wrapping_mul(fsr) >> 11;
    let w15 = (w as i64) << 15;
    let fcx = cvttd64(pdbl(a, 1) * w15 as f64) + w15;
    let h15 = (h as i64) << 15;
    let fcy = cvttd64(pdbl(a, 2) * h15 as f64) + h15;
    let (sp, stride) = (&a.src.px[..], a.src.stride as usize);
    // u = (x + fcx + 32768) >> 16 inside 0 .. w-1  <=>  0 <= x + fcx + 32768 < w << 16 (one
    // unsigned compare), the same for v
    let (bx, by) = (fcx + 32768, fcy + 32768);
    let (wl, hl) = ((w as u64) << 16, (h as u64) << 16);
    let sample = |s: &mut V, n: &mut u32, x: i64, y: i64| {
        let ux = x + bx;
        if ux as u64 >= wl {
            return;
        }
        let vy = y + by;
        if vy as u64 >= hl {
            return;
        }
        *s = acc_px(*s, spx(sp, stride, (ux >> 16) as usize, (vy >> 16) as usize));
        *n += 1;
    };
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let srow = a.src_row(y);
        let d = dst.row(y);
        for x in rect[0]..rect[2] {
            let mut s = acc_px(Sse2.zero(), px(srow, x as usize));
            let mut n = 1u32;
            let (mut x1, mut y1) = (((x as i64) << 16) - fcx, ((y as i64) << 16) - fcy);
            let (mut x2, mut y2) = (x1, y1);
            for _ in 0..nn {
                rb_rot(&mut x1, &mut y1, fsr, fsr2);
                sample(&mut s, &mut n, x1, y1);
                rb_rot(&mut x2, &mut y2, -fsr, fsr2);
                sample(&mut s, &mut n, x2, y2);
            }
            put(d, (x - a.x0) as usize, blend_out(s, n));
        }
    }
    0
}

/// Math.Round(x, MidpointRounding.AwayFromZero)  [fxb_round_away]
#[inline]
fn round_away(x: f64) -> i32 {
    let t = cvttd(f64::from_bits(x.to_bits() & !SIGN) + 0.5);
    if x >= 0.0 { t } else { t.wrapping_neg() }
}

fn fragment_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let n = pint(a, 0, 2, 50);
    let d = pint(a, 1, 0, 100) as f64;
    let t0 = (pdbl(a, 2) - 90.0) * 3.141592653589793 / 180.0;
    let step = 6.283185307179586 / n as f64;
    let offs: Vec<(i32, i32)> = (0..n)
        .map(|i| {
            let (s, c) = gfx::xform::sincos(i as f64 * step + t0);
            (round_away(neg(s) * d), round_away(neg(c) * d))
        })
        .collect();
    let (w, h) = (a.src.w, a.src.h);
    let (sp, stride) = (&a.src.px[..], a.src.stride as usize);
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let dr = dst.row(y);
        for x in rect[0]..rect[2] {
            let mut s = Sse2.zero();
            let mut cnt = 0u32;
            for &(ox, oy) in &offs {
                let sx = x - ox;
                let sy = y - oy;
                if sx < 0 || sx >= w || sy < 0 || sy >= h {
                    continue;
                }
                s = acc_px(s, spx(sp, stride, sx as usize, sy as usize));
                cnt += 1;
            }
            put(dr, (x - a.x0) as usize, blend_out(s, cnt));
        }
    }
    0
}

// ---------------------------------------------------------------- Surface Blur

/// one channel of the surface blur  [fxb_bilat]
fn bilat(ch: usize, h: &Hist, v: u32, wt: &[u32; 256]) -> u32 {
    let (fine, coarse) = (&h.fine[ch & 3], &h.coarse[ch & 3]);
    let (mut num, mut den) = (0u64, 0u64);
    let mut i = 0usize;
    while i < 256 {
        if coarse[i >> 4] == 0 {
            i = (i | 15) + 1;
            continue;
        }
        let c = fine[i];
        if c != 0 {
            let d = (i as i32 - v as i32).unsigned_abs() as usize;
            let hw = c.wrapping_mul(wt[d]) as u64;
            den += hw;
            num += hw * i as u64;
        }
        i += 1;
    }
    if den == 0 {
        return v;
    }
    ((num * 2 + den) / (den * 2)) as u32
}

fn surface_px(h: &Hist, wt: &[u32; 256]) -> u32 {
    let mut c = h.src;
    if c & 0xFF00_0000 == 0 {
        c = c & 0xFF00_0000 | pct4(h, 50) & 0x00FF_FFFF;
    }
    let mut r = 0u32;
    for ch in 0..4 {
        let v = c >> (8 * ch) & 255;
        let o = if ch != 3 && h.ncol == 0 { 0 } else { bilat(ch, h, v, wt) };
        r |= o << (8 * ch);
    }
    r
}

fn surface_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 100);
    let t = pint(a, 1, 1, 100);
    let c = -1.0 / ((t * t) * 2) as f64;
    let mut wt = [0u32; 256];
    for (d, v) in wt.iter_mut().enumerate() {
        *v = cvttd(exp(((d * d) as i32) as f64 * c) * 256.0 + 0.5) as u32;
    }
    lh_run(a, dst, r, LHS_DISC, true, |h, _, _| surface_px(h, &wt))
}

// ---------------------------------------------------------------- Bokeh Blur

struct Bokeh<'d, 'e> {
    dst: &'d mut Dst<'e>,
    th: f32,
    inv: f32,
    k: f32,
    w16: Vec<i16>,
    /// output row (working values)
    wo: Vec<i16>,
}

impl DiscIo for Bokeh<'_, '_> {
    /// boosted linear values (i32 x 4) of a source row  [fxb_bokeh_prep]
    fn prep(&mut self, d: &Disc, v: &mut [i32], y: i32, x0: i32, x1: i32) {
        load_row(&mut self.w16, d.src, y, x0, x1, FXL_GAMMA);
        for i in 0..(x1 - x0) as usize {
            let p = &self.w16[i * 4..i * 4 + 4];
            let (b, g, r, al) = (p[0] as u16 as i32, p[1] as u16 as i32, p[2] as u16 as i32, p[3] as u16 as i32);
            let lsum = r * 2126 + g * 7152 + b * 722;
            let mut f = 1.0f32;
            if al != 0 {
                let lum = lsum as f32 / (al * 10000) as f32;
                let t = minss(maxss((lum - self.th) * self.inv, 0.0), 1.0);
                f += t * t * self.k;
            }
            for c in 0..3 {
                v[i * 4 + c] = cvt(p[c] as u16 as i32 as f32 * f);
            }
            v[i * 4 + 3] = cvt(al as f32 * 1.0);
        }
    }

    /// f32 disc sums -> mean, highlight compression, sRGB output  [fxb_bokeh_emit]
    fn emit(&mut self, d: &Disc, y: i32, row: DiscRow) {
        let n = d.x1 - d.x0;
        if n <= 0 {
            return;
        }
        let DiscRow::F(r) = row else {
            return;
        };
        let inv = 1.0f32 / d.area as f32;
        let w = &mut self.wo[..n as usize * 4];
        for (i, q) in r[..n as usize * 4].chunks_exact(4).enumerate() {
            let mut m: [f32; 4] = core::array::from_fn(|c| q[c] * inv);
            let al = m[3];
            if al > 0.0 {
                for c in 0..3 {
                    if m[c] > al * 0.8 {
                        let x = m[c] as f64 / al as f64;
                        let e = exp((x - 0.8) * -5.0);
                        let xp = (1.0 - e) * 0.2 + 0.8;
                        m[c] = (xp * al as f64) as f32;
                    }
                }
            }
            for c in 0..4 {
                w[i * 4 + c] = cvt(m[c]).clamp(-32768, 32767) as i16;
            }
        }
        from_work(dst_px(self.dst, y, d.x0, n), w, n as usize, FMT_ARGB, RS_GAMMA);
    }
}

fn bokeh_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let i = pint(a, 1, 0, 100);
    let k = ((i * i) as f64 * 0.0025) as f32;
    let g = pint(a, 2, 0, 100);
    let th = 0.95 - g as f64 * 0.009;
    let r = pint(a, 0, 1, 100);
    let Some(mut d) = Disc::new(a, r) else {
        return 0;
    };
    d.flags = DSF_F32;
    d.mode = FXL_GAMMA;
    d.custom_prep = true;
    let n = (d.x1 - d.x0 + 2 * r + 8) as usize;
    let wo = vec![0i16; (d.x1 - d.x0) as usize * 4];
    let mut io = Bokeh { dst, th: th as f32, inv: (1.0 / (1.0 - th)) as f32, k, w16: vec![0i16; n * 4], wo };
    disc_run(&mut d, &mut io)
}

// ---------------------------------------------------------------- Sketch Blur

const SK_STRIP: i32 = 32;

/// floor of an f64 (|x| < 2^31)  [fxb_floor]
#[inline]
fn floor_d(x: f64) -> i32 {
    let t = cvttd(x);
    if t as f64 > x { t - 1 } else { t }
}

fn sketch_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let (w, h) = (a.src.w, a.src.h);
    let rr = pint(a, 0, 1, 50);
    let l = (2 * rr + 1) as f64;
    let inv = 1.0f32 / ((2 * rr + 1) * 768) as f32;
    let spread = pint(a, 2, 0, 180);
    let seed = a.params.0[3] as u32;
    let ang = pdbl(a, 1);
    let (sa, ca) = gfx::xform::sincos(ang * DEG2RAD);
    let mut ou = [0f64; 3];
    let mut ov = [0f64; 3];
    for lv in 0..3u32 {
        ou[lv as usize] = (hash32(lv, 0, seed, 7) & 0xFFFF) as f64 * 0.0000152587890625 * l;
        ov[lv as usize] = (hash32(lv, 1, seed, 7) & 0xFFFF) as f64 * 0.0000152587890625 * 4.0;
    }
    let bx0 = (rect[0] - (rr + 2)).max(0);
    let bx1 = (rect[2] + rr + 2).min(w);
    let bw = bx1 - bx0;
    // direction table: {rint(-256 R cos), rint(-256 R sin), rint(256 cos), rint(256 sin)}
    let mut dir = [[0i32; 4]; 256];
    for (r, dv) in dir.iter_mut().enumerate() {
        let t = ((r as i32 as f64 / 255.0 - 0.5) * spread as f64 + ang) * DEG2RAD;
        let (s, c) = gfx::xform::sincos(t);
        let m = rr as f64 * -256.0;
        *dv = [cvtd(c * m), cvtd(s * m), cvtd(c * 256.0), cvtd(s * 256.0)];
    }
    let tw = (rect[2] - rect[0]) as usize;
    let mut win = vec![0i16; ((2 * rr + SK_STRIP + 5) * bw) as usize * 4 + 8];
    let mut out = vec![0i16; tw * 4];
    let (xmax, ymax) = ((w - 1) << 4, (h - 1) << 4);
    let mut sy0 = rect[1];
    while sy0 < rect[3] {
        let sy1 = (sy0 + SK_STRIP).min(rect[3]);
        let by0 = (sy0 - (rr + 2)).max(0);
        let by1 = (sy1 + rr + 2).min(h);
        for (k, y) in (by0..by1).enumerate() {
            load_row(&mut win[k * bw as usize * 4..], a.src, y, bx0, bx1, 0);
        }
        for y in sy0..sy1 {
            if a.cancelled() {
                return ERR_CANCEL;
            }
            for x in rect[0]..rect[2] {
                let cx = x as f64 + 0.5;
                let cy = y as f64 + 0.5;
                let u = cx * ca + cy * sa;
                let v = cy * ca - cx * sa;
                let s = Sse2;
                let mut acc = s.zero();
                for lv in 0..3usize {
                    let m = floor_d((u + ou[lv]) / l);
                    let k = floor_d((v + ov[lv]) * 0.25);
                    let dv = &dir[(hash32(m as u32, k as u32, seed, lv as u32 + 1) & 255) as usize];
                    let mut pxx = (x << 8).wrapping_add(dv[0]);
                    let mut pyy = (y << 8).wrapping_add(dv[1]);
                    for _ in 0..2 * rr + 1 {
                        let xs = (pxx >> 4).clamp(0, xmax);
                        let ys = (pyy >> 4).clamp(0, ymax);
                        acc = bilin(acc, &win, bw, bx0, by0, w, h, xs, ys);
                        pxx = pxx.wrapping_add(dv[2]);
                        pyy = pyy.wrapping_add(dv[3]);
                    }
                }
                let o = (x - rect[0]) as usize * 4;
                mean16(acc, inv, &mut out[o..]);
            }
            from_work(dst_px(dst, y, rect[0], tw as i32), &out, tw, FMT_ARGB, 0);
        }
        sy0 = sy1;
    }
    0
}

// ---------------------------------------------------------------- tables

static P_BOKEH: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 100.0, 25.0),
    FxProp::new(FP_INT, 0, 1, 0, "intensity", 0.0, 100.0, 50.0),
    FxProp::new(FP_INT, 0, 2, 0, "granularity", 0.0, 100.0, 50.0),
];
static P_SKETCH: [FxProp; 4] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 50.0, 10.0),
    FxProp::new(FP_ANGLE, 2, 1, 0, "angle", -180.0, 180.0, 45.0),
    FxProp::new(FP_INT, 0, 2, 0, "spread", 0.0, 180.0, 60.0),
    FxProp::new(FP_SEED, 0, 3, 0, "seed", 0.0, 2147483647.0, 0.0),
];
static P_SURFACE: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 100.0, 6.0),
    FxProp::new(FP_INT, 0, 1, 0, "threshold", 1.0, 100.0, 15.0),
];
static P_FRAGMENT: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "fragments", 2.0, 50.0, 4.0),
    FxProp::new(FP_INT, 0, 1, 0, "distance", 0.0, 100.0, 8.0),
    FxProp::new(FP_ANGLE, 2, 2, 0, "rotation", 0.0, 360.0, 0.0),
];
static P_RADIAL: [FxProp; 3] = [
    FxProp::new(FP_ANGLE, 2, 0, 0, "angle", 0.0, 360.0, 2.0),
    FxProp::pan(1, "offset", 2, -1.0, 1.0, 0.0, 0.0),
    FxProp::new(FP_INT, 0, 3, 0, "quality", 1.0, 5.0, 2.0),
];
static P_ZOOM: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "amount", 0.0, 100.0, 10.0),
    FxProp::pan(1, "offset", 2, -1.0, 1.0, 0.0, 0.0),
];
static P_GAUSS: [FxProp; 1] = [FxProp::new(FP_INT, 0, 0, 0, "radius", 0.0, 200.0, 2.0)];
static P_MEDIAN: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 200.0, 10.0),
    FxProp::new(FP_INT, 0, 1, 0, "percentile", 0.0, 100.0, 50.0),
];
static P_MOTION: [FxProp; 3] = [
    FxProp::new(FP_ANGLE, 2, 0, 0, "angle", -180.0, 180.0, 25.0),
    FxProp::new(FP_INT, 0, 1, 0, "distance", 1.0, 200.0, 10.0),
    FxProp::new(FP_BOOL, 0, 2, 0, "centered", 0.0, 1.0, 1.0),
];
static P_SQUARE: [FxProp; 1] = [FxProp::new(FP_INT, 0, 0, 0, "radius", 0.0, 200.0, 2.0)];
static P_UNFOCUS: [FxProp; 1] = [FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 200.0, 4.0)];

const fn desc(id: u32, flags: u32, name: &'static str, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags, name, prepare: None, render, props, category: FXC_BLUR, user: 0 }
}

/// the group table  [fxtab_blur]
pub static FXTAB: [FxDesc; 11] = [
    desc(FX_BOKEH_BLUR, FXF_SLOW, "bokeh_blur", bokeh_render, &P_BOKEH),
    desc(FX_FRAGMENT_BLUR, 0, "fragment_blur", fragment_render, &P_FRAGMENT),
    desc(FX_GAUSSIAN_BLUR, 0, "gaussian_blur", gauss_render, &P_GAUSS),
    desc(FX_MEDIAN, FXF_SLOW, "median", median_render, &P_MEDIAN),
    desc(FX_MOTION_BLUR, 0, "motion_blur", motion_render, &P_MOTION),
    desc(FX_RADIAL_BLUR, FXF_SLOW, "radial_blur", radial_render, &P_RADIAL),
    desc(FX_SKETCH_BLUR, FXF_SEED | FXF_SLOW, "sketch_blur", sketch_render, &P_SKETCH),
    desc(FX_SQUARE_BLUR, 0, "square_blur", square_render, &P_SQUARE),
    desc(FX_SURFACE_BLUR, FXF_SLOW, "surface_blur", surface_render, &P_SURFACE),
    desc(FX_UNFOCUS, 0, "unfocus", unfocus_render, &P_UNFOCUS),
    desc(FX_ZOOM_BLUR, 0, "zoom_blur", zoom_render, &P_ZOOM),
];
