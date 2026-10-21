// conv -- shared convolution helpers of the fx kernels                     [src/fx/fx_conv.s]
//
//   exp(x) / ln(x) / pow(x, y)       f64 e^x (Cody-Waite + degree 12 Taylor), ln (2 atanh), x^y
//   roi(a) -> Option<[i32; 4]>       the ROI clamped to the image (fxc_roi), None when empty
//   copy_roi(a, dst) -> 0 | ERR_CANCEL      destination ROI = source ROI
//   pint(a, slot, lo, hi) / pdbl(a, slot)   clamped i64 / f64 parameters (pdbl: NaN / inf -> 0,
//                                    clamped to +-1e6)
//   load_row(w16, src, y, x0, x1, mode)     working pixels (4 x i16 premultiplied, full 16320)
//                                    of (x0 .. x1-1, y); outside: edge pixels or 0 (FXL_ZERO);
//                                    FXL_GAMMA: linear light
//   var12(r), Kern::gauss(var12), Kern::boxk(h)   separable kernels (exact FIR for sigma <= 6,
//                                    else a cascade of 3 boxes)
//   sep_run(sep, io) -> 0 | ERR_*    separable convolution of a ROI in strips: H pass of the
//                                    rows (clamped / loaded), V pass, io.emit per row
//   disc_run(disc, io) -> 0 | ERR_*  sums over the paint.net disc of every ROI pixel (prefix
//                                    sums per row, i32 or f32 accumulation)
//   disc_edges(r) -> (E, area)       half widths of the disc rows
//   bac_init(b, c) -> (tab, mode), bac_row, desat_row   paint.net 3 Brightness / Contrast
//   blurbac(a, dst, r, b, c, f)      Gaussian blur then Brightness / Contrast of the ROI;
//                                    f(dst, y, x0, row) finishes each output row
//
// Exactness: every sum is an integer (running box sums, FIR dot products), so the result of
// a pixel never depends on the ROI split; box means are rounded through f32
// (rint(f32(S) * f32(1 / w))), FIR sums as (S + 8192) >> 14 saturated to i16.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{Surface, FMT_ARGB};
use img_core::resample::{from_work, to_work};
use rt::fx::{cvtd64, cvttd, maxsd, minsd};
use simd::{Kernel, Simd, Sse2};

use crate::lut::luma;
use crate::*;

pub const FXL_GAMMA: u32 = 1; // linear light (RS_GAMMA)
pub const FXL_ZERO: u32 = 2; // outside the image = transparent (default: clamp to the edge)
pub const KT_COPY: u32 = 0;
pub const KT_FIR: u32 = 1;
pub const KT_BOX: u32 = 2;
pub const KN_MAXTAPS: usize = 255;
pub const KN_MAXBOX: usize = 8;
const FIR_MAXVAR12: i32 = 432; // 12 sigma^2 of the widest exact FIR (sigma = 6)
const SEP_MAXBYTES: u64 = 16_777_216; // target size of one strip buffer
pub const DSF_F32: u32 = 1; // disc: accumulate the per-row sums in f32

// ---------------------------------------------------------------- maths

static EXPC: [f64; 13] = [
    2.08767569878680989792e-09,
    2.50521083854417187751e-08,
    2.75573192239858906526e-07,
    2.75573192239858906526e-06,
    2.48015873015873015873e-05,
    1.98412698412698412698e-04,
    1.38888888888888888889e-03,
    8.33333333333333333333e-03,
    4.16666666666666666667e-02,
    1.66666666666666666667e-01,
    0.5,
    1.0,
    1.0,
];
static LNC: [f64; 11] = [
    0.0952380952380952380952,
    0.105263157894736842105,
    0.117647058823529411765,
    0.133333333333333333333,
    0.153846153846153846154,
    0.181818181818181818182,
    0.222222222222222222222,
    0.285714285714285714286,
    0.4,
    0.666666666666666666667,
    2.0,
];
const LN2HI: f64 = f64::from_bits(0x3FE6_2E42_FEE0_0000);
const LN2LO: f64 = f64::from_bits(0x3DEA_39EF_3579_3C76);

/// e^x: k = rint(x / ln 2), r = x - k ln2 (Cody-Waite), degree 12 Taylor  [fxc_exp]
pub fn exp(x: f64) -> f64 {
    let x = maxsd(minsd(x, 709.0), -708.0);
    let k = cvtd64(x * 1.4426950408889634074);
    let kd = k as f64;
    let mut r = x - kd * LN2HI;
    r -= kd * LN2LO;
    let mut p = EXPC[0];
    for c in &EXPC[1..] {
        p = p * r + *c;
    }
    p * f64::from_bits(((k + 1023) as u64) << 52)
}

/// ln x: x = m 2^e, ln m = 2 atanh((m - 1) / (m + 1)); x <= 0 or denormal -> -745  [fxc_ln]
pub fn ln(x: f64) -> f64 {
    if !(x > 0.0) {
        return -745.0;
    }
    let b = x.to_bits();
    let ex = (b >> 52) as i32 & 0x7FF;
    if ex == 0 {
        return -745.0;
    }
    let mut e = ex - 1023;
    let mut m = b & 0x000F_FFFF_FFFF_FFFF | 0x3FF0_0000_0000_0000;
    if m >= 0x3FF6_A09E_667F_3BCD {
        m -= 0x0010_0000_0000_0000;
        e += 1;
    }
    let m = f64::from_bits(m);
    let s = (m - 1.0) / (m + 1.0);
    let z = s * s;
    let mut p = LNC[0];
    for c in &LNC[1..] {
        p = p * z + *c;
    }
    let lnm = p * s;
    let ed = e as f64;
    ed * LN2HI + (lnm + ed * LN2LO)
}

/// x^y = exp(y ln x) for x > 0, else 0  [fxc_pow]
pub fn pow(x: f64, y: f64) -> f64 {
    if !(x > 0.0) {
        return 0.0;
    }
    exp(ln(x) * y)
}

// ---------------------------------------------------------------- ROI / parameters

/// the ROI clamped to the image (x0, y0 >= 0, x1 <= w, y1 <= h); None when empty  [fxc_roi]
pub fn roi(a: &FxArgs) -> Option<[i32; 4]> {
    let r = [a.x0.max(0), a.y0.max(0), a.x1.min(a.src.w), a.y1.min(a.src.h)];
    if r[0] < r[2] && r[1] < r[3] { Some(r) } else { None }
}

/// destination ROI = source ROI  [fxc_copy_roi]
pub fn copy_roi(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(r) = roi(a) else {
        return 0;
    };
    for y in r[1]..r[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let s = &a.src_row(y)[r[0] as usize * 4..r[2] as usize * 4];
        dst_px(dst, y, r[0], r[2] - r[0]).copy_from_slice(s);
    }
    0
}

/// the destination pixels x0 .. x0 + n of row y
#[inline]
pub fn dst_px<'d>(dst: &'d mut Dst, y: i32, x0: i32, n: i32) -> &'d mut [u8] {
    let o = (x0 - dst.x0) as usize * 4;
    &mut dst.row(y)[o..o + n as usize * 4]
}

/// i64 parameter slot clamped to [lo, hi]  [fxc_pint]
#[inline]
pub fn pint(a: &FxArgs, slot: usize, lo: i32, hi: i32) -> i32 {
    a.params.int(slot & (FX_NSLOTS - 1), lo, hi)
}

/// f64 parameter slot; NaN / infinite -> 0, clamped to -1e6 .. 1e6  [fxc_pdbl]
#[inline]
pub fn pdbl(a: &FxArgs, slot: usize) -> f64 {
    let v = a.params.f(slot & (FX_NSLOTS - 1));
    if (v.to_bits() >> 52) & 0x7FF == 0x7FF {
        return 0.0;
    }
    minsd(maxsd(v, -1000000.0), 1000000.0)
}

// ---------------------------------------------------------------- row loader

/// w16 = working pixels of (x0 .. x1-1, y); outside columns / rows: the edge pixel (clamp)
/// or 0 (mode & FXL_ZERO); mode & FXL_GAMMA: linear light  [fxc_load_row]
pub fn load_row(dst: &mut [i16], src: &Surface, y: i32, x0: i32, x1: i32, mode: u32) {
    let n = x1 - x0;
    if n <= 0 {
        return;
    }
    let mut y = y;
    if y >= src.h || y < 0 {
        if mode & FXL_ZERO != 0 {
            dst[..n as usize * 4].fill(0);
            return;
        }
        y = if y < 0 { 0 } else { src.h - 1 };
    }
    let row = row(src, y);
    let g = mode & FXL_GAMMA;
    let nl = (-x0).clamp(0, n);
    let nr = (x1 - src.w).clamp(0, n);
    let nm = n - nl - nr;
    if nm > 0 {
        let c = (x0 + nl) as usize * 4;
        to_work(&mut dst[nl as usize * 4..], &row[c..], nm as usize, FMT_ARGB, g);
    }
    if nl > 0 {
        let v: [i16; 4] = if mode & FXL_ZERO != 0 {
            [0; 4]
        } else if nm > 0 {
            dst[nl as usize * 4..nl as usize * 4 + 4].try_into().unwrap()
        } else {
            to_work(&mut dst[..4], row, 1, FMT_ARGB, g);
            dst[..4].try_into().unwrap()
        };
        for p in dst[..nl as usize * 4].chunks_exact_mut(4) {
            p.copy_from_slice(&v);
        }
    }
    if nr > 0 {
        let f = (nl + nm) as usize * 4;
        let v: [i16; 4] = if mode & FXL_ZERO != 0 {
            [0; 4]
        } else if nm > 0 {
            dst[f - 4..f].try_into().unwrap()
        } else {
            to_work(&mut dst[f..f + 4], &row[(src.w as usize - 1) * 4..], 1, FMT_ARGB, g);
            dst[f..f + 4].try_into().unwrap()
        };
        for p in dst[f..f + nr as usize * 4].chunks_exact_mut(4) {
            p.copy_from_slice(&v);
        }
    }
}

// ---------------------------------------------------------------- kernels

/// paint.net-like blur radius -> 12 sigma^2 = 2 r (r + 2) (r <= 0 -> 0)  [fxc_var12]
#[inline]
pub fn var12(r: i32) -> i32 {
    if r <= 0 { 0 } else { (r + 2).wrapping_mul(r) * 2 }
}

/// a separable 1-D kernel (KERN)
#[derive(Clone)]
pub struct Kern {
    pub typ: u32,
    pub halo: i32,
    pub n: i32,           // FIR taps (odd) / number of boxes
    pub w: [i16; 256],    // FIR weights (+ a 0 pad tap)
    pub h: [i32; KN_MAXBOX], // box half widths
}

impl Kern {
    /// the identity
    pub fn copy() -> Kern {
        Kern { typ: KT_COPY, halo: 0, n: 0, w: [0; 256], h: [0; KN_MAXBOX] }
    }

    /// one box of width 2h + 1 (h <= 0: identity)  [fxc_kern_box]
    pub fn boxk(h: i32) -> Kern {
        let mut k = Kern::copy();
        if h > 0 {
            k.typ = KT_BOX;
            k.halo = h;
            k.n = 1;
            k.h[0] = h;
        }
        k
    }

    /// Gaussian with 12 sigma^2 = var12 (<= 0: identity)  [fxc_kern_gauss]
    ///   FIR (var12 <= 432): h = ceil(sqrt(0.75 var12)) (1..18), g_i = exp(-6 i^2 / var12),
    ///   sum = g_0 + 2 g_1 + ..., q_i = trunc(g_i (16384 / sum) + 0.5), the centre absorbs
    ///   16384 - (q_0 + 2 sum q_i).  Boxes: wl = largest odd w with 3 w^2 <= var12 + 3,
    ///   m = (2 num + den) / (2 den) (num = 3 wl^2 + 12 wl + 9 - var12, den = 4 wl + 4,
    ///   m <= 3): m boxes of width wl, then 3 - m of wl + 2.
    pub fn gauss(var12: i32) -> Kern {
        let mut k = Kern::copy();
        if var12 <= 0 {
            return k;
        }
        if var12 <= FIR_MAXVAR12 {
            let x = rt::fx::sqrtd(var12 as f64 * 0.75);
            let mut h = cvttd(x);
            if x > h as f64 {
                h += 1;
            }
            let h = h.clamp(1, 18);
            let c = -6.0 / var12 as f64;
            let mut g = [0f64; 19];
            for (i, gv) in g.iter_mut().enumerate().take(h as usize + 1) {
                *gv = exp(((i * i) as i32) as f64 * c);
            }
            let mut sum = g[0];
            for &gv in g.iter().take(h as usize + 1).skip(1) {
                sum += gv + gv;
            }
            let inv = 16384.0 / sum;
            let hu = h as usize;
            let mut total = 0i32;
            for (i, &gv) in g.iter().enumerate().take(hu + 1) {
                let q = cvttd(gv * inv + 0.5);
                k.w[hu + i] = q as i16;
                k.w[hu - i] = q as i16;
                total += q;
                if i != 0 {
                    total += q;
                }
            }
            k.w[hu] = k.w[hu].wrapping_add((16384 - total) as i16);
            k.w[2 * hu + 1] = 0;
            k.typ = KT_FIR;
            k.halo = h;
            k.n = 2 * h + 1;
            return k;
        }
        let mut wl = 1i32;
        while (wl + 2) * (wl + 2) * 3 <= var12 + 3 {
            wl += 2;
        }
        let num = wl * wl * 3 + wl * 12 + 9 - var12;
        let den = wl * 4 + 4;
        let m = (((2 * num + den) as u32) / ((2 * den) as u32)).min(3) as i32;
        let mut halo = 0;
        for b in 0..3 {
            let w = if b < m { wl } else { wl + 2 };
            k.h[b as usize] = w >> 1;
            halo += w >> 1;
        }
        k.typ = KT_BOX;
        k.halo = halo;
        k.n = 3;
        k
    }

    /// FIR weights as used: taps 0 .. 2 * npairs - 1 (the pad tap is 0)
    #[inline]
    fn fir_taps(&self) -> usize {
        let n = (self.n as usize).min(KN_MAXTAPS);
        n.div_ceil(2) * 2
    }
}

// ---------------------------------------------------------------- row kernels

#[inline(always)]
fn sat16(v: i32) -> i16 {
    v.clamp(-32768, 32767) as i16
}

/// the pmaddwd weight pair (w[2p], w[2p+1]) as one u32
#[inline(always)]
fn pair_w(w: &[i16], p: usize) -> u32 {
    w[2 * p] as u16 as u32 | (w[2 * p + 1] as u16 as u32) << 16
}

/// the two pixels of a register interleaved channel by channel (b b' g g' r r' a a' per lane)
#[inline(always)]
fn pair<S: Simd>(s: S, v: S::V) -> S::V {
    s.shuf16::<0xD8>(s.shuf32::<0xD8>(v))
}

/// one output pixel of fir_h (also the tail of the kernel)
#[inline(always)]
fn fir_h_px(dst: &mut [i16], src: &[i16], i: usize, w: &[i16]) {
    let mut acc = [0i32; 4];
    for (j, &wj) in w.iter().enumerate() {
        if wj == 0 {
            continue;
        }
        let p = &src[(i + j) * 4..(i + j) * 4 + 4];
        for c in 0..4 {
            acc[c] = acc[c].wrapping_add(p[c] as i32 * wj as i32);
        }
    }
    for c in 0..4 {
        dst[i * 4 + c] = sat16(acc[c].wrapping_add(8192) >> 14);
    }
}

// fxc_fir_h: pmaddwd over pixel pairs (the asm's pair rows built on the fly), 2 (SSE2) / 4
// (AVX2) output pixels per step
struct FirHK<'a> {
    dst: &'a mut [i16],
    src: &'a [i16],
    n: usize,
    w: &'a [i16],
}

impl Kernel for FirHK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let FirHK { dst, src, n, w } = self;
        let np = w.len() / 2;
        let step = S::BYTES / 8;
        let r = s.splat32(8192);
        let mut i = 0;
        while i + step <= n && (i + step + 2 * np) * 4 <= src.len() {
            let (mut lo, mut hi) = (s.zero(), s.zero());
            for p in 0..np {
                let wv = s.splat32(pair_w(w, p));
                let a = pair(s, s.load_i16(&src[(i + 2 * p) * 4..]));
                let b = pair(s, s.load_i16(&src[(i + 1 + 2 * p) * 4..]));
                lo = s.add32(lo, s.madd16(a, wv));
                hi = s.add32(hi, s.madd16(b, wv));
            }
            let lo = s.sra32::<14>(s.add32(lo, r));
            let hi = s.sra32::<14>(s.add32(hi, r));
            s.store_i16(&mut dst[i * 4..], s.packs32(lo, hi));
            i += step;
        }
        while i < n {
            fir_h_px(dst, src, i, w);
            i += 1;
        }
    }
}

/// dst[i] = sat((sum_j src[i + j] w[j] + 8192) >> 14), i < n (4 channels per pixel)  [fxc_fir_h]
fn fir_h(dst: &mut [i16], src: &[i16], n: usize, w: &[i16]) {
    simd::dispatch(FirHK { dst, src, n, w });
}

type V = <Sse2 as Simd>::V;

/// rint(f32(v) * inv) of four i32 lanes (cvtdq2ps, mulps, cvtps2dq)
#[inline(always)]
fn box_mean(v: V, inv: V) -> V {
    let x = Sse2;
    x.cvtr_f32_i32(x.mulf(x.cvt_i32_f32(v), inv))
}

/// two results as 8 i16 (2 pixels), or the first pixel only
#[inline(always)]
fn store2(dst: &mut [i16], a: V, b: V, two: bool) {
    let x = Sse2;
    if two {
        x.store_i16(dst, x.packs32(a, b));
    } else {
        let mut t = [0i16; 8];
        x.store_i16(&mut t, x.packs32(a, a));
        dst[..4].copy_from_slice(&t[..4]);
    }
}

/// dst[i] = rint(f32(sum src[i .. i+2h]) * f32(1 / (2h+1))), i < n (running sums of 4 i32
/// lanes, one pixel per register as the asm); src holds 4 slack values  [fxc_box_h]
fn box_h(dst: &mut [i16], src: &[i16], n: usize, h: i32) {
    if n == 0 {
        return;
    }
    let x = Sse2;
    let z = x.zero();
    let w = (2 * h + 1) as usize;
    let inv = x.splatf(1.0f32 / w as i32 as f32);
    let px = |i: usize| x.unpacklo16(x.load_i16(&src[i * 4..]), z);
    let mut sum = z;
    for k in 0..w {
        sum = x.add32(sum, px(k));
    }
    let mut i = 0;
    while i < n {
        let v0 = box_mean(sum, inv);
        if i + 1 == n {
            store2(&mut dst[i * 4..], v0, v0, false);
            break;
        }
        sum = x.sub32(x.add32(sum, px(i + w)), px(i));
        let v1 = box_mean(sum, inv);
        store2(&mut dst[i * 4..], v0, v1, true);
        if i + 2 < n {
            sum = x.sub32(x.add32(sum, px(i + 1 + w)), px(i + 1));
        }
        i += 2;
    }
}

// fxc_fir_v: pmaddwd over row pairs, 2 (SSE2) / 4 (AVX2) pixels per step
struct FirVK<'a> {
    out: &'a mut [i16],
    src: &'a [i16],
    rows: &'a [usize],
    n: usize,
    tw: usize,
    w: &'a [i16],
}

impl Kernel for FirVK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let FirVK { out, src, rows, n, tw, w } = self;
        let np = w.len() / 2;
        let step = S::BYTES / 8;
        let r = s.splat32(8192);
        for k in 0..n {
            let o = &mut out[k * tw * 4..(k + 1) * tw * 4];
            let mut x = 0;
            while x + step <= tw {
                let (mut lo, mut hi) = (s.zero(), s.zero());
                for p in 0..np {
                    let wv = s.splat32(pair_w(w, p));
                    let a = s.load_i16(&src[rows[k + 2 * p] + x * 4..]);
                    let b = s.load_i16(&src[rows[k + 2 * p + 1] + x * 4..]);
                    lo = s.add32(lo, s.madd16(s.unpacklo16(a, b), wv));
                    hi = s.add32(hi, s.madd16(s.unpackhi16(a, b), wv));
                }
                let lo = s.sra32::<14>(s.add32(lo, r));
                let hi = s.sra32::<14>(s.add32(hi, r));
                s.store_i16(&mut o[x * 4..], s.packs32(lo, hi));
                x += step;
            }
            for i in x * 4..tw * 4 {
                let mut a = 0i32;
                for (j, &wj) in w.iter().enumerate() {
                    if wj != 0 {
                        a = a.wrapping_add(src[rows[k + j] + i] as i32 * wj as i32);
                    }
                }
                o[i] = sat16(a.wrapping_add(8192) >> 14);
            }
        }
    }
}

/// output row k = sat((sum_j w_j rows[k + j] + 8192) >> 14); row j at src[rows[j]..]
/// [fxc_fir_v]
fn fir_v(out: &mut [i16], src: &[i16], rows: &[usize], n: usize, tw: usize, w: &[i16]) {
    simd::dispatch(FirVK { out, src, rows, n, tw, w });
}

/// output row k = rint(f32(sum rows[k .. k+2h]) * f32(1 / (2h+1))); row j at src[rows[j]..];
/// acc: tw * 4 scratch values  [fxc_box_v]
fn box_v(out: &mut [i16], src: &[i16], rows: &[usize], n: usize, tw: usize, h: i32, acc: &mut [i32]) {
    let x = Sse2;
    let w = (2 * h + 1) as usize;
    let inv = x.splatf(1.0f32 / w as i32 as f32);
    let acc = &mut acc[..tw * 4];
    acc.fill(0);
    for &r in rows.iter().take(w) {
        for (a, &p) in acc.iter_mut().zip(src[r..r + tw * 4].iter()) {
            *a = a.wrapping_add(p as u16 as i32);
        }
    }
    for k in 0..n {
        let o = &mut out[k * tw * 4..(k + 1) * tw * 4];
        let mut i = 0;
        while i < tw {
            let a = box_mean(x.load_i32(&acc[i * 4..]), inv);
            if i + 1 < tw {
                let b = box_mean(x.load_i32(&acc[(i + 1) * 4..]), inv);
                store2(&mut o[i * 4..], a, b, true);
            } else {
                store2(&mut o[i * 4..], a, a, false);
            }
            i += 2;
        }
        if k + 1 < n {
            let (lv, en) = (&src[rows[k]..rows[k] + tw * 4], &src[rows[k + w]..rows[k + w] + tw * 4]);
            for ((a, &e), &l) in acc.iter_mut().zip(en.iter()).zip(lv.iter()) {
                *a = a.wrapping_add(e as u16 as i32).wrapping_sub(l as u16 as i32);
            }
        }
    }
}

// ---------------------------------------------------------------- separable run

/// a separable run (SEP): source, job, kernels, the output ROI, loader mode
pub struct Sep<'a> {
    pub src: &'a Surface,
    pub job: Option<&'a (dyn Cancel + Sync)>,
    pub kh: &'a Kern,
    pub kv: Option<&'a Kern>,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub mode: u32,
    /// io.load replaces fxc_load_row (rows are then not clamped to the image)
    pub custom_load: bool,
}

impl<'a> Sep<'a> {
    /// the standard run of an effect: source / job / ROI of a (None when the ROI is empty)
    /// [fxc_sep_init]
    pub fn new(a: &FxArgs<'a>, k: &'a Kern) -> Option<Sep<'a>> {
        let r = roi(a)?;
        Some(Sep { src: a.src, job: a.job, kh: k, kv: None, x0: r[0], y0: r[1], x1: r[2], y1: r[3], mode: 0, custom_load: false })
    }
    #[inline]
    fn cancelled(&self) -> bool {
        self.job.is_some_and(|j| j.cancelled())
    }
}

/// the row source and sink of sep_run (SP_LOAD / SP_EMIT)
pub trait SepIo {
    /// a custom loader (when Sep::custom_load): w16 of (x0 .. x1-1, y)
    fn load(&mut self, _sep: &Sep, _w16: &mut [i16], _y: i32, _x0: i32, _x1: i32) {}
    /// output row y (columns sep.x0 .. sep.x1-1) as working pixels
    fn emit(&mut self, sep: &Sep, y: i32, row: &[i16]);
}

/// the standard emitter: working row -> straight ARGB in the destination  [fxc_emit_argb]
pub struct EmitArgb<'d, 'e>(pub &'d mut Dst<'e>);

impl SepIo for EmitArgb<'_, '_> {
    fn emit(&mut self, sep: &Sep, y: i32, row: &[i16]) {
        let n = sep.x1 - sep.x0;
        from_work(dst_px(self.0, y, sep.x0, n), row, n as usize, FMT_ARGB, sep.mode & FXL_GAMMA);
    }
}

/// separable convolution of a ROI -> 0 | ERR_CANCEL  [fxc_sep_run]
/// Strips of S rows: H pass for the rows sy0 - HY .. sy1 + HY (clamped rows computed once),
/// then the V pass over a row table, then io.emit per row.
pub fn sep_run(sep: &Sep, io: &mut impl SepIo) -> i32 {
    let kh = sep.kh;
    let kv = sep.kv.unwrap_or(kh);
    let (hx, hy) = (kh.halo, kv.halo);
    let tw = sep.x1 - sep.x0;
    if tw <= 0 || sep.y1 <= sep.y0 {
        return 0;
    }
    let l = (tw + 2 * hx) as usize;
    let tw = tw as usize;
    let rb = tw as u64 * 8;
    let mut s = 256i32;
    while s > 16 && (s + 2 * hy) as u64 * rb > SEP_MAXBYTES {
        s >>= 1;
    }
    let nrmax = (s + 2 * hy) as usize;
    let imgh = sep.src.h;
    let clamp = !sep.custom_load && sep.mode & FXL_ZERO == 0;
    // one block for the buffers (as the asm: one allocation per run)
    let vbox = kv.typ == KT_BOX;
    let (l4, big) = ((l + 4) * 4, nrmax * tw * 4);
    let nb = if vbox { big } else { 0 };
    let mut buf = vec![0i16; 3 * l4 + big + 2 * nb + s as usize * tw * 4];
    let (inb, rest) = buf.split_at_mut(l4);
    let (ha, rest) = rest.split_at_mut(l4);
    let (hb, rest) = rest.split_at_mut(l4);
    let (t, rest) = rest.split_at_mut(big);
    let (b1, rest) = rest.split_at_mut(nb);
    let (b2, out) = rest.split_at_mut(nb);
    // row tables: pt (offsets of the T rows of a strip, + 2 slack) and the plain rows of B1 / B2
    let mut tabs = vec![0usize; 2 * nrmax + 2];
    let (pt, ident) = tabs.split_at_mut(nrmax + 2);
    for (k, v) in ident.iter_mut().enumerate() {
        *v = k * tw * 4;
    }
    let mut acc = vec![0i32; if vbox { tw * 4 } else { 0 }];
    let wh: &[i16] = &kh.w[..kh.fir_taps()];
    let wv: &[i16] = &kv.w[..kv.fir_taps()];
    let mut sy0 = sep.y0;
    while sy0 < sep.y1 {
        let sy1 = (sy0 + s).min(sep.y1);
        let ra = sy0 - hy;
        let rbnd = sy1 + hy;
        let (ta, tb) = if clamp { (ra.max(0), rbnd.min(imgh)) } else { (ra, rbnd) };
        // ---- H pass: rows ta .. tb-1 -> t
        for (k, y) in (ta..tb).enumerate() {
            if sep.cancelled() {
                return ERR_CANCEL;
            }
            let (x0, x1) = (sep.x0 - hx, sep.x1 + hx);
            if sep.custom_load {
                io.load(sep, inb, y, x0, x1);
            } else {
                load_row(inb, sep.src, y, x0, x1, sep.mode);
            }
            let trow = &mut t[k * tw * 4..(k + 1) * tw * 4];
            match kh.typ {
                KT_FIR => fir_h(trow, inb, tw, wh),
                KT_BOX => {
                    let mut len = l;
                    let nb = (kh.n as usize).min(KN_MAXBOX);
                    for b in 0..nb {
                        let h = kh.h[b];
                        let nlen = len - 2 * h as usize;
                        let last = b + 1 >= kh.n as usize;
                        // source: inb (box 0), ha (after an even box), hb (after an odd box)
                        match (b, last) {
                            (0, true) => box_h(trow, inb, nlen, h),
                            (0, false) => box_h(ha, inb, nlen, h),
                            (_, true) if b % 2 == 1 => box_h(trow, ha, nlen, h),
                            (_, true) => box_h(trow, hb, nlen, h),
                            (_, false) if b % 2 == 1 => box_h(hb, ha, nlen, h),
                            (_, false) => box_h(ha, hb, nlen, h),
                        }
                        len = nlen;
                    }
                }
                _ => trow.copy_from_slice(&inb[..tw * 4]),
            }
        }
        // ---- row table: PT[k] = t row of clamp(ra + k, ta, tb - 1)
        let nv = (rbnd - ra) as usize;
        for (k, v) in pt[..nv].iter_mut().enumerate() {
            let y = (ra + k as i32).clamp(ta, tb - 1) - ta;
            *v = y as usize * tw * 4;
        }
        pt[nv] = pt[nv - 1];
        pt[nv + 1] = pt[nv - 1];
        let ptr: &[usize] = &pt[..nv + 2];
        let nrows = (sy1 - sy0) as usize;
        // ---- V pass
        let res: Option<&[i16]> = match kv.typ {
            KT_FIR => {
                fir_v(out, t, ptr, nrows, tw, wv);
                Some(&*out)
            }
            KT_BOX => {
                // box b reads the T rows (b = 0), B1 (odd b) or B2 (even b > 0) and writes OUT
                // (the last box), B1 (even b) or B2 (odd b)
                let nb = (kv.n as usize).min(KN_MAXBOX);
                let mut nin = nv;
                for b in 0..nb {
                    let h = kv.h[b];
                    let nprev = nin;
                    nin -= 2 * h as usize;
                    let last = b + 1 >= kv.n as usize;
                    if b == 0 {
                        box_v(if last { &mut *out } else { &mut *b1 }, t, ptr, nin, tw, h, &mut acc);
                    } else if b % 2 == 1 {
                        box_v(if last { &mut *out } else { &mut *b2 }, b1, &ident[..nprev], nin, tw, h, &mut acc);
                    } else {
                        box_v(if last { &mut *out } else { &mut *b1 }, b2, &ident[..nprev], nin, tw, h, &mut acc);
                    }
                }
                Some(&*out)
            }
            _ => None,
        };
        // ---- emit
        for k in 0..nrows {
            if sep.cancelled() {
                return ERR_CANCEL;
            }
            let row: &[i16] = match res {
                Some(o) => &o[k * tw * 4..(k + 1) * tw * 4],
                None => &t[ptr[k]..ptr[k] + tw * 4],
            };
            io.emit(sep, sy0 + k as i32, row);
        }
        sy0 = sy1;
    }
    0
}

// ---------------------------------------------------------------- disc run

/// half widths of the disc rows E[0..r] (paint.net LocalHistogramEffect: largest u with
/// u^2 + v^2 <= ((2r+1)^2 + 2) / 4) and the area  [fxc_disc_edges]
pub fn disc_edges(r: i32) -> (Vec<i32>, i32) {
    let cutoff = ((2 * r + 1) * (2 * r + 1) + 2) >> 2;
    let mut e = vec![0i32; r.max(0) as usize + 1];
    let mut area = 0;
    for v in 0..=r.max(0) {
        let mut u = 0;
        while u < r && (u + 1) * (u + 1) + v * v <= cutoff {
            u += 1;
        }
        e[v as usize] = u;
        area += if v == 0 { 2 * u + 1 } else { 2 * (2 * u + 1) };
    }
    (e, area)
}

/// a disc run (DSC)
pub struct Disc<'a> {
    pub src: &'a Surface,
    pub job: Option<&'a (dyn Cancel + Sync)>,
    pub r: i32,
    pub flags: u32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub mode: u32,
    /// pixels in the disc (set by disc_run)
    pub area: i32,
    /// io.prep replaces the default row preparation
    pub custom_prep: bool,
}

impl<'a> Disc<'a> {
    /// the standard run of an effect (None when the ROI is empty)  [fxc_disc_init]
    pub fn new(a: &FxArgs<'a>, r: i32) -> Option<Disc<'a>> {
        let rc = roi(a)?;
        Some(Disc { src: a.src, job: a.job, r, flags: 0, x0: rc[0], y0: rc[1], x1: rc[2], y1: rc[3], mode: 0, area: 0, custom_prep: false })
    }
    #[inline]
    fn cancelled(&self) -> bool {
        self.job.is_some_and(|j| j.cancelled())
    }
}

/// the summed row of a disc run: i32 x 4 per pixel, or f32 x 4 with DSF_F32
pub enum DiscRow<'r> {
    I(&'r mut [i32]),
    F(&'r mut [f32]),
}

/// the row source and sink of disc_run (DS_PREP / DS_EMIT)
pub trait DiscIo {
    /// custom preparation (Disc::custom_prep): i32 x 4 values of (x0 .. x1-1, y)
    fn prep(&mut self, _d: &Disc, _v: &mut [i32], _y: i32, _x0: i32, _x1: i32) {}
    fn emit(&mut self, d: &Disc, y: i32, row: DiscRow);
}

/// the standard emitter (i32 sums): W16 = rint(f32(sum) * f32(1 / area)), then straight ARGB
/// [fxc_disc_emit_argb]
pub struct DiscArgb<'d, 'e>(pub &'d mut Dst<'e>);

impl DiscIo for DiscArgb<'_, '_> {
    fn emit(&mut self, d: &Disc, y: i32, row: DiscRow) {
        let n = d.x1 - d.x0;
        if n <= 0 {
            return;
        }
        let inv = Sse2.splatf(1.0f32 / d.area as f32);
        let DiscRow::I(r) = row else {
            return;
        };
        // chunks of DE_CHUNK pixels through a stack buffer
        let mut w = [0i16; DE_CHUNK * 4];
        let mut x = 0usize;
        for c in r[..n as usize * 4].chunks(DE_CHUNK * 4) {
            let m = c.len() / 4;
            for (o, q) in w.chunks_exact_mut(8).zip(c.chunks(8)) {
                let sv = Sse2;
                let a = sv.cvtr_f32_i32(sv.mulf(sv.cvt_i32_f32(sv.load_i32(&q[..4])), inv));
                let b = if q.len() == 8 { sv.cvtr_f32_i32(sv.mulf(sv.cvt_i32_f32(sv.load_i32(&q[4..])), inv)) } else { a };
                sv.store_i16(o, sv.packs32(a, b));
            }
            from_work(dst_px(self.0, y, d.x0 + x as i32, m as i32), &w[..m * 4], m, FMT_ARGB, d.mode & FXL_GAMMA);
            x += m;
        }
    }
}

/// pixels per from_work call of the disc emitters
pub(crate) const DE_CHUNK: usize = 256;

/// disc sums of a ROI -> 0 | ERR_CANCEL  [fxc_disc_run]
/// Per source row a prefix sum P (i32, wrapping) makes each disc row one difference
/// P[x + e + 1] - P[x - e]; rows are clamped to the image; the rows are summed in i32 or (DSF_F32)
/// in f32 in the order v = -r .. r.
pub fn disc_run(d: &mut Disc, io: &mut impl DiscIo) -> i32 {
    let r = d.r.clamp(0, 1000);
    let tw = d.x1 - d.x0;
    if tw <= 0 || d.y1 <= d.y0 {
        return 0;
    }
    let l = (tw + 2 * r) as usize;
    let tw = tw as usize;
    let prb = (l as u64 + 1) * 16;
    let imgh = d.src.h;
    let mut s = 128i32;
    while s > 8 && (s + 2 * r) as u64 * prb > SEP_MAXBYTES {
        s >>= 1;
    }
    let nrmax = (s + 2 * r) as usize;
    let (e, area) = disc_edges(r);
    d.area = area;
    let pw = (l + 1) * 4; // prefix row length (i32)
    // the i32 buffers in one block (v: custom preparation only; acc_i: i32 sums only)
    let f32m = d.flags & DSF_F32 != 0;
    let nv = if d.custom_prep { (l + 4) * 4 } else { 0 };
    let na = if f32m { 0 } else { tw * 4 };
    let mut ibuf = vec![0i32; nv + nrmax * pw + na];
    let (v, rest) = ibuf.split_at_mut(nv);
    let (p, acc_i) = rest.split_at_mut(nrmax * pw);
    let mut w16 = vec![0i16; if d.custom_prep { 0 } else { (l + 4) * 4 }];
    let mut acc_f = vec![0f32; if f32m { tw * 4 } else { 0 }];
    let mut sy0 = d.y0;
    while sy0 < d.y1 {
        let sy1 = (sy0 + s).min(d.y1);
        let ra = sy0 - r;
        let ta = ra.max(0);
        let tb = (sy1 + r).min(imgh);
        // prefix rows ta .. tb-1
        for (k, y) in (ta..tb).enumerate() {
            if d.cancelled() {
                return ERR_CANCEL;
            }
            let (x0, x1) = (d.x0 - r, d.x1 + r);
            let pr = &mut p[k * pw..(k + 1) * pw];
            pr[..4].fill(0);
            let x = Sse2;
            let mut sum = x.zero();
            if d.custom_prep {
                io.prep(d, v, y, x0, x1);
                for (o, q) in pr[4..].chunks_exact_mut(4).zip(v[..l * 4].chunks_exact(4)) {
                    sum = x.add32(sum, x.load_i32(q));
                    x.store_i32(o, sum);
                }
            } else {
                load_row(&mut w16, d.src, y, x0, x1, d.mode);
                // w16 holds 4 values of slack: an 8-value load per pixel stays inside
                let z = x.zero();
                for (i, o) in pr[4..].chunks_exact_mut(4).enumerate() {
                    sum = x.add32(sum, x.unpacklo16(x.load_i16(&w16[i * 4..]), z));
                    x.store_i32(o, sum);
                }
            }
        }
        let nrows = (sy1 - sy0) as usize;
        for kk in 0..nrows {
            if d.cancelled() {
                return ERR_CANCEL;
            }
            acc_i.fill(0);
            acc_f.fill(0.0);
            for j in 0..=(2 * r) as usize {
                let vv = j as i32 - r;
                let ev = e[vv.unsigned_abs() as usize] as usize;
                let y = (ra + (kk + j) as i32).clamp(ta, tb - 1) - ta;
                let pr = &p[y as usize * pw..(y as usize + 1) * pw];
                let ho = (r as usize + ev + 1) * 4;
                let lo = (r as usize - ev) * 4;
                let (hi, lo) = (&pr[ho..ho + tw * 4], &pr[lo..lo + tw * 4]);
                if f32m {
                    for ((a, &h), &l) in acc_f.iter_mut().zip(hi).zip(lo) {
                        *a += h.wrapping_sub(l) as f32;
                    }
                } else {
                    for ((a, &h), &l) in acc_i.iter_mut().zip(hi).zip(lo) {
                        *a = a.wrapping_add(h.wrapping_sub(l));
                    }
                }
            }
            let row = if f32m { DiscRow::F(&mut acc_f) } else { DiscRow::I(&mut *acc_i) };
            io.emit(d, sy0 + kk as i32, row);
        }
        sy0 = sy1;
    }
    0
}

// ---------------------------------------------------------------- brightness / contrast

/// paint.net 3 BrightnessAndContrast as used inside Glow / Soften Portrait / Pencil / Ink:
/// (tab, mode) -- mode 0 (div = 100) / 1: shift tables, mode 2 (contrast 100): threshold
/// [fxc_bac_init]
pub fn bac_init(b: i32, c: i32) -> ([i32; 256], u32) {
    let (mul, div) = if c < 0 {
        (c + 100, 100)
    } else if c == 0 {
        (1, 1)
    } else {
        (100, 100 - c)
    };
    let mut tab = [0i32; 256];
    if div == 0 {
        for (i, t) in tab.iter_mut().enumerate() {
            *t = if i as i32 + b < 128 { 0 } else { 255 };
        }
        return (tab, 2);
    }
    let mode = if div == 100 { 0 } else { 1 };
    for (i, t) in tab.iter_mut().enumerate() {
        let i = i as i32;
        let mut v = i - 127;
        if mode != 0 {
            v += b;
        }
        let mut r = v.wrapping_mul(mul) / div + 127 - i;
        if mode == 0 {
            r += b;
        }
        *t = r;
    }
    (tab, mode)
}

/// apply bac_init's table to a straight ARGB row (dst may be src: pass the same slice twice
/// through bac_row_inplace)  [fxc_bac_row]
pub fn bac_row_inplace(row: &mut [u8], tab: &[i32; 256], mode: u32) {
    for i in 0..row.len() / 4 {
        let p = px(row, i);
        let t = tab[luma(p) as usize];
        let o = if mode == 2 {
            (t as u32).wrapping_mul(0x010101) | p & 0xFF00_0000
        } else {
            let mut o = p & 0xFF00_0000;
            for sh in [0, 8, 16] {
                let v = ((p >> sh & 255) as i32 + t).clamp(0, 255) as u32;
                o |= v << sh;
            }
            o
        };
        put(row, i, o);
    }
}

/// B = G = R = intensity, alpha kept  [fxc_desat_row]
pub fn desat_row(dst: &mut [u8], src: &[u8]) {
    for i in 0..src.len() / 4 {
        let p = px(src, i);
        put(dst, i, luma(p) * 0x010101 | p & 0xFF00_0000);
    }
}

/// desat_row in place
pub fn desat_inplace(row: &mut [u8]) {
    for i in 0..row.len() / 4 {
        let p = px(row, i);
        put(row, i, luma(p) * 0x010101 | p & 0xFF00_0000);
    }
}

struct BlurBac<'d, 'e, 'f, F: FnMut(&mut Dst, i32, i32, &mut [u8])> {
    dst: &'d mut Dst<'e>,
    tab: [i32; 256],
    mode: u32,
    tmp: Vec<u8>,
    f: &'f mut F,
}

impl<F: FnMut(&mut Dst, i32, i32, &mut [u8])> SepIo for BlurBac<'_, '_, '_, F> {
    fn emit(&mut self, sep: &Sep, y: i32, row: &[i16]) {
        let n = (sep.x1 - sep.x0) as usize;
        from_work(&mut self.tmp[..n * 4], row, n, FMT_ARGB, 0);
        bac_row_inplace(&mut self.tmp[..n * 4], &self.tab, self.mode);
        (self.f)(self.dst, y, sep.x0, &mut self.tmp[..n * 4]);
    }
}

/// the paint.net 3 chain "Gaussian blur (radius r) of the source, then Brightness /
/// Contrast" for the ROI; f(dst, y, x0, row) receives the straight ARGB result of each row
/// (columns x0 ..) and writes the destination itself -> 0 | ERR_*  [fxc_blurbac]
pub fn blurbac(a: &FxArgs, dst: &mut Dst, r: i32, b: i32, c: i32, mut f: impl FnMut(&mut Dst, i32, i32, &mut [u8])) -> i32 {
    let (tab, mode) = bac_init(b, c);
    let k = Kern::gauss(var12(r));
    let Some(sep) = Sep::new(a, &k) else {
        return 0;
    };
    let n = (sep.x1 - sep.x0) as usize;
    let mut io = BlurBac { dst, tab, mode, tmp: vec![0u8; n * 4 + 64], f: &mut f };
    sep_run(&sep, &mut io)
}

/// working values of one straight pixel  (rs_to_work, no gamma)
#[inline]
pub fn w16_of(p: u32) -> [i16; 4] {
    let mut w = [0i16; 4];
    to_work(&mut w, &p.to_le_bytes(), 1, FMT_ARGB, 0);
    w
}
