// adj -- the 13 adjustments (Korrekturen) of paint.net 5.1 as pure kernels    [src/fx/fx_adj.s]
//
//   FXTAB            the group table (menu order of paint.net)
//
// Pixels are straight ARGB; every adjustment keeps alpha (except Invert Alpha) and changes the
// colour of transparent pixels like any other.  Intensity I = (7471 B + 38470 G + 19595 R) >> 16.
//
//   auto_level          prepare: histogram (alpha > 0, selection > 0) -> paint.net
//                       HistogramRgb.MakeLevelsAuto -> Level lut
//   black_white         B = G = R = I
//   brightness_contrast paint.net 3.x BrightnessAndContrastAdjustment (integer tables)
//   curves              natural cubic spline luts (Luminosity: c + curve[I] - I, or R/G/B)
//   exposure            linear light: out = enc(dec(c) * 2^ev) per channel lut
//   highlights_shadows  blurred alpha-weighted intensity grid (prepare) -> gain
//                       2^(2 s (1-m)^2 - 2 h m^2) on linear light
//   hue_saturation      paint.net HueSaturationLightness (f32 lanes as the asm's SSE code)
//   invert_alpha        A = 255 - A
//   invert_colors       B, G, R = 255 - c
//   levels              paint.net UnaryPixelOps.Level (lut::levels_lut) with a channel mask
//   posterize           paint.net PosterizePixel levels per channel
//   sepia               Desaturate, then Level with gamma B 1.2, G 1.0, R 0.8 (constant tables)
//   temperature_tint    luminance-normalised linear-light channel gains

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use rt::fx::{cvtt, cvttd, maxsd, maxss, minsd, minss};
use simd::{Kernel, Simd};

use crate::hist::{hist_mean, hist_percentile, hist_rgba};
use crate::lut::{exp2, levels_lut, lin_table, log2, luma, lut_ident, lut_row, spline_lut, srgb_dec, srgb_enc, srgb_table, Lvl};
use crate::*;

const HX_GAINN: usize = 1024;
const HX_ENCN: usize = 4096;
const HX_KMAX: i32 = 40; // largest blur kernel radius (grid cells)
const CV_MAXPTS: usize = 19;

// ---------------------------------------------------------------- small helpers

#[inline]
fn copy_rows(a: &FxArgs, dst: &mut Dst) -> i32 {
    rows(a, dst, |d, s, _, _| d.copy_from_slice(s))
}

#[inline]
fn xor_rows(a: &FxArgs, dst: &mut Dst, m: u32) -> i32 {
    rows(a, dst, |d, s, _, _| {
        for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]) ^ m;
            d.copy_from_slice(&v.to_le_bytes());
        }
    })
}

#[inline(always)]
fn clamp255(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

/// c + shift on B, G, R, clamped (alpha kept)
#[inline(always)]
fn shift_px(p: u32, s: i32) -> u32 {
    (p & 0xFF00_0000)
        | clamp255((p & 255) as i32 + s)
        | clamp255((p >> 8 & 255) as i32 + s) << 8
        | clamp255((p >> 16 & 255) as i32 + s) << 16
}

// ---------------------------------------------------------------- invert colours / alpha, b/w, sepia

fn invc_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    xor_rows(a, dst, 0x00FF_FFFF)
}

fn inva_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    xor_rows(a, dst, 0xFF00_0000)
}

fn bw_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            put(d, i, luma(p) * 0x010101 | p & 0xFF00_0000);
        }
    })
}

fn sepia_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let l = luma(p) as usize;
            put(d, i, p & 0xFF00_0000 | (SEP_R[l] as u32) << 16 | (l as u32) << 8 | SEP_B[l] as u32);
        }
    })
}

// ---------------------------------------------------------------- brightness / contrast

fn bc_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let b = a.params.int(0, -100, 100);
    let c = a.params.int(1, -100, 100);
    let (mul, div) = if c == 0 {
        (1, 1)
    } else if c > 0 {
        (100, 100 - c)
    } else {
        (c + 100, 100)
    };
    let mut tab = [0i32; 256];
    if div == 0 {
        for (i, t) in tab.iter_mut().enumerate() {
            *t = if i as i32 + b >= 128 { 255 } else { 0 };
        }
        return rows(a, dst, |d, s, _, _| {
            for i in 0..s.len() / 4 {
                let p = px(s, i);
                put(d, i, (tab[luma(p) as usize] as u32).wrapping_mul(0x010101) | p & 0xFF00_0000);
            }
        });
    }
    for (i, t) in tab.iter_mut().enumerate() {
        let i = i as i32;
        *t = if div == 100 { (i - 127) * mul / div + 127 - i + b } else { (i - 127 + b) * mul / div + 127 - i };
    }
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            put(d, i, shift_px(p, tab[luma(p) as usize]));
        }
    })
}

// ---------------------------------------------------------------- posterize

fn post_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut lut = [0u8; 1024];
    lut_ident(&mut lut);
    let linked = a.params.int(3, 0, 1);
    for ch in 0..3usize {
        let slot = if linked != 0 { 0 } else { ch };
        let l = a.params.int(slot, 2, 64) as u32;
        let off = (2 - ch) * 256; // red -> 512, green -> 256, blue -> 0
        let mut t1 = [0u8; 64];
        for i in 1..l {
            t1[i as usize] = (i * 255 / (l - 1)) as u8;
        }
        let (mut j, mut k) = (0usize, 0u32);
        for i in 0..256 {
            lut[off + i] = t1[j];
            k += l;
            if k > 255 {
                k -= 255;
                j += 1;
            }
        }
    }
    rows(a, dst, |d, s, _, _| lut_row(d, s, &lut))
}

// ---------------------------------------------------------------- hue / saturation

#[derive(Clone, Copy)]
struct Hs {
    flags: u32, // 1 saturation, 2 hue, 4 lightness
    sf: f32,
    h6: f32,
    inva: f32,
    k: f32,
}

/// one output channel of the hue rotation: K = (D + 60 n d) mod 360 d, X = clamp(min(K, 240 d
/// - K), 0, 60 d), mx - trunc((X + 29.5) / 60)
#[inline(always)]
fn hs_ch(n: f32, d0: f32, mx: f32, d360: f32, d60: f32) -> f32 {
    let mut t = d60 * n + d0;
    if t < 0.0 {
        t += d360;
    }
    if !(t < d360) {
        t -= d360;
    }
    if !(t < d360) {
        t -= d360;
    }
    let x = d60 * 4.0 - t;
    let t = maxss(minss(minss(t, x), d60), 0.0);
    let t = cvtt((t + 29.5) * 0.016666668) as f32;
    mx - t
}

#[inline(always)]
fn hs_satc(c: f32, i: f32, i1024: f32, sf: f32) -> f32 {
    let v = maxss((c - i) * sf + i1024, 0.0);
    minss(cvtt(v * 0.0009765625) as f32, 255.0)
}

#[inline(always)]
fn hs_light(c: f32, inva: f32, k: f32) -> f32 {
    cvtt((c * inva + k) * 0.00390625) as f32
}

#[inline(always)]
fn hs_px(p: u32, h: &Hs) -> u32 {
    let mut b = (p & 255) as f32;
    let mut g = (p >> 8 & 255) as f32;
    let mut r = (p >> 16 & 255) as f32;
    if h.flags & 1 != 0 {
        let i = cvtt((b * 7471.0 + g * 38470.0 + r * 19595.0) * 1.52587890625e-05) as f32;
        let i1024 = i * 1024.0;
        b = hs_satc(b, i, i1024, h.sf);
        g = hs_satc(g, i, i1024, h.sf);
        r = hs_satc(r, i, i1024, h.sf);
    }
    if h.flags & 2 != 0 {
        let mx = maxss(maxss(b, g), r);
        let mn = minss(minss(b, g), r);
        let d = mx - mn;
        let mr = r == mx;
        let mg = !mr && g == mx;
        let num = if mr {
            g - b
        } else if mg {
            b - r
        } else {
            r - g
        };
        let base = if mr {
            0.0
        } else if mg {
            2.0
        } else {
            4.0
        };
        let d0 = d * h.h6 + (d * base + num) * 60.0;
        let d360 = d * 360.0;
        let d60 = d * 60.0;
        let nr = hs_ch(5.0, d0, mx, d360, d60);
        let ng = hs_ch(3.0, d0, mx, d360, d60);
        let nb = hs_ch(1.0, d0, mx, d360, d60);
        r = nr;
        g = ng;
        b = nb;
    }
    if h.flags & 4 != 0 {
        b = hs_light(b, h.inva, h.k);
        g = hs_light(g, h.inva, h.k);
        r = hs_light(r, h.inva, h.k);
    }
    (cvtt(b) as u32) | (cvtt(g) as u32) << 8 | (cvtt(r) as u32) << 16 | p & 0xFF00_0000
}

fn hs_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let hue = a.params.int(0, -180, 180);
    let sat = a.params.int(1, 0, 200);
    let light = a.params.int(2, -100, 100);
    let mut flags = 0;
    let sf = (sat as u32 * 1024 / 100) as i32;
    if sf != 1024 {
        flags |= 1;
    }
    if hue != 0 {
        flags |= 2;
    }
    let (col, al) = if light == 0 {
        (0, 0)
    } else {
        flags |= 4;
        let al = (light.unsigned_abs() * 255 / 100) as i32;
        (if light > 0 { 255 } else { 0 }, al)
    };
    let h = Hs { flags, sf: sf as f32, h6: hue as f32, inva: (255 - al) as f32, k: (al * col) as f32 };
    if flags == 0 {
        return copy_rows(a, dst);
    }
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            put(d, i, hs_px(px(s, i), &h));
        }
    })
}

// ---------------------------------------------------------------- levels / auto level / lut effects

struct LutAux(Box<[u8; 1024]>);

fn lv_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let p = a.params;
    let mut lvl = Lvl::default();
    lvl.inlo = (p.0[0] as u32).to_le_bytes();
    lvl.inhi = (p.0[1] as u32).to_le_bytes();
    lvl.outlo = (p.0[2] as u32).to_le_bytes();
    lvl.outhi = (p.0[3] as u32).to_le_bytes();
    for c in 0..3 {
        let g = p.f(6 - c); // B, G, R <- gamma slots 6, 5, 4
        let g = if g.is_nan() { 1.0 } else { g };
        lvl.gamma[c] = minsd(maxsd(g, 0.1), 10.0) as f32;
    }
    let mut lut = Box::new([0u8; 1024]);
    levels_lut(&mut lut, &lvl);
    // channel mask: red slot 7, green 8, blue 9 -> identity when off
    for c in 0..3usize {
        if p.0[c + 7] == 0 {
            let o = (2 - c) * 256;
            for i in 0..256 {
                lut[o + i] = i as u8;
            }
        }
    }
    Ok(Some(Box::new(LutAux(lut))))
}

/// apply the 1024-byte lut of prepare  [fx_lut_render]
fn lut_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    match a.aux::<LutAux>() {
        Some(l) => rows(a, dst, |d, s, _, _| lut_row(d, s, &l.0)),
        None => copy_rows(a, dst),
    }
}

fn al_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let mut hist = Box::new([[0u32; 256]; 4]);
    let count = hist_rgba(a, &mut hist);
    if count < 0 {
        return Err(count as i32);
    }
    let mut lvl = Lvl { outlo: [0; 4], outhi: [255, 255, 255, 0], ..Lvl::default() };
    for c in 0..3 {
        let lo = hist_percentile(&hist[c], count, 0.004999999888241291);
        let hi = hist_percentile(&hist[c], count, 0.9950000047683716);
        let md = cvtt(hist_mean(&hist[c]) as f32 + 0.5) as u32 & 255;
        lvl.inlo[c] = lo as u8;
        lvl.inhi[c] = hi as u8;
        lvl.gamma[c] = if lo < md && md < hi {
            let v = ((md - lo) as i32 as f32 / (hi - lo) as i32 as f32) as f64;
            minsd(maxsd(-1.0 / log2(v), 0.1), 10.0) as f32
        } else {
            1.0
        };
    }
    let mut lut = Box::new([0u8; 1024]);
    levels_lut(&mut lut, &lvl);
    Ok(Some(Box::new(LutAux(lut))))
}

/// lut[c][j] = round(255 enc(dec(j / 255) g_c)), alpha identity (gains B, G, R)
fn gain_lut(g: &[f64; 3]) -> Box<[u8; 1024]> {
    let mut lut = Box::new([0u8; 1024]);
    lut_ident(&mut lut[..]);
    for c in 0..3 {
        for j in 0..256 {
            let v = srgb_enc(srgb_dec(j as f64 / 255.0) * g[c]);
            lut[c * 256 + j] = cvttd(v * 255.0 + 0.5) as u8;
        }
    }
    lut
}

fn ex_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let g = exp2(a.params.dbl(0, -5.0, 5.0));
    Ok(Some(Box::new(LutAux(gain_lut(&[g, g, g])))))
}

fn tt_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let t = a.params.int(0, -100, 100) as f64 / 100.0;
    let m = a.params.int(1, -100, 100) as f64 / 100.0;
    let b = exp2(t * -0.5 + m * 0.15);
    let g = exp2(m * -0.3);
    let r = exp2(t * 0.5 + m * 0.15);
    let lum = r * 0.2126 + g * 0.7152 + b * 0.0722;
    Ok(Some(Box::new(LutAux(gain_lut(&[b / lum, g / lum, r / lum])))))
}

// ---------------------------------------------------------------- curves

struct CvAux {
    lut: Box<[u8; 1024]>,
    mode: i32, // 0 luminosity, 1 rgb
}

/// spline table from a curve blob {u8 n, n x {x, y}} at `slot`
fn cv_blob(lut: &mut [u8], p: &Params, slot: usize) {
    let n = (p.byte(slot, 0) as usize).min(CV_MAXPTS);
    let mut pts = [0u8; 2 * CV_MAXPTS];
    for (i, v) in pts.iter_mut().enumerate() {
        *v = p.byte(slot, 1 + i);
    }
    spline_lut(lut, &pts, n);
}

fn cv_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let mut lut = Box::new([0u8; 1024]);
    lut_ident(&mut lut[..]);
    let mode = a.params.int(0, 0, 1);
    if mode == 0 {
        cv_blob(&mut lut[0..256], a.params, 1);
    } else {
        cv_blob(&mut lut[512..768], a.params, 6);
        cv_blob(&mut lut[256..512], a.params, 11);
        cv_blob(&mut lut[0..256], a.params, 16);
    }
    Ok(Some(Box::new(CvAux { lut, mode })))
}

fn cv_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(cv) = a.aux::<CvAux>() else {
        return copy_rows(a, dst);
    };
    if cv.mode != 0 {
        return rows(a, dst, |d, s, _, _| lut_row(d, s, &cv.lut));
    }
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let l = luma(p);
            put(d, i, shift_px(p, cv.lut[l as usize] as i32 - l as i32));
        }
    })
}

// ---------------------------------------------------------------- highlights / shadows

struct HxAux {
    w: i32,
    h: i32,
    invs: f32,
    tab: Vec<u8>,   // [1025][256] result byte for (gain index, channel byte)
    grid: Vec<f32>, // W * H blurred intensity 0..1
}

/// one blur pass over the W x H grid: dst[p] = sum w[i + R] src[clamp(p + i)] in tap order
/// (f32 multiply, then add; whole vectors of cells where no tap is clamped along x)
fn hx_pass(src: &[f32], dst: &mut [f32], along_x: bool, gw: i32, gh: i32, wts: &[f32]) {
    simd::dispatch(HxPass { src, dst, along_x, gw, gh, wts });
}

struct HxPass<'a> {
    src: &'a [f32],
    dst: &'a mut [f32],
    along_x: bool,
    gw: i32,
    gh: i32,
    wts: &'a [f32],
}

impl Kernel for HxPass<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let HxPass { src, dst, along_x, gw, gh, wts } = self;
        let r = (wts.len() / 2) as i32;
        let n = S::BYTES / 4;
        let scalar = |dst: &mut [f32], x: i32, y: i32| {
            let mut acc = 0f32;
            for i in -r..=r {
                let idx = if along_x {
                    (x + i).clamp(0, gw - 1) + y * gw
                } else {
                    (y + i).clamp(0, gh - 1) * gw + x
                };
                acc += src[idx as usize] * wts[(i + r) as usize];
            }
            dst[(y * gw + x) as usize] = acc;
        };
        let g = gw as usize;
        for y in 0..gh {
            let mut x = 0i32;
            while x < gw {
                // a vector of n cells: along x only where no tap is clamped
                let vec_ok = x as usize + n <= g && (!along_x || (x >= r && x + n as i32 + r <= gw));
                if !vec_ok {
                    scalar(dst, x, y);
                    x += 1;
                    continue;
                }
                let mut acc = s.zero();
                for i in -r..=r {
                    let o = if along_x {
                        (y * gw + x + i) as usize
                    } else {
                        ((y + i).clamp(0, gh - 1) * gw + x) as usize
                    };
                    acc = s.addf(acc, s.mulf(s.load_f32(&src[o..]), s.splatf(wts[(i + r) as usize])));
                }
                let o = (y * gw + x) as usize;
                s.store_f32(&mut dst[o..], acc);
                x += n as i32;
            }
        }
    }
}

fn hx_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let (w, h) = (a.src.w, a.src.h);
    let sh = a.params.int(0, -100, 100);
    let hl = a.params.int(1, -100, 100);
    let radius = a.params.int(2, 1, 200);
    let s = (radius >> 2).max(1);
    let gw = ((w + s - 1) as u32 / s as u32) as i32;
    let gh = ((h + s - 1) as u32 / s as u32) as i32;
    let sigma = radius as f64 / (s as f64 + s as f64);
    let s3 = sigma * 3.0;
    let mut kr = cvttd(s3);
    if s3 > kr as f64 {
        kr += 1;
    }
    let kr = kr.min(HX_KMAX);
    let mut wts = vec![0f32; (2 * kr + 1) as usize];
    let mut sum = 0f64;
    for i in -kr..=kr {
        let mut den = sigma * sigma;
        den += den;
        den *= 0.6931471805599453;
        let e = exp2(-((i * i) as f64 / den));
        sum += e;
        wts[(i + kr) as usize] = e as f32;
    }
    for wv in wts.iter_mut() {
        *wv = (*wv as f64 / sum) as f32;
    }
    let cells = (gw as usize) * (gh as usize);
    let mut tmp = vec![0f32; cells];
    let mut sums = vec![(0i64, 0i64); gw as usize];
    for cy in 0..gh {
        if a.cancelled() {
            return Err(ERR_CANCEL);
        }
        sums.iter_mut().for_each(|v| *v = (0, 0));
        let ys = cy * s;
        let ye = (ys + s).min(h);
        for y in ys..ye {
            let row = &a.src_row(y)[..w as usize * 4];
            for (sm, c) in sums.iter_mut().zip(row.chunks(s as usize * 4)) {
                // integer sums: the order does not matter (s <= 50 pixels fit a u32)
                let (mut ai, mut sa) = (0u32, 0u32);
                for q in c.chunks_exact(4) {
                    let p = u32::from_le_bytes([q[0], q[1], q[2], q[3]]);
                    let al = p >> 24;
                    ai += luma(p) * al;
                    sa += al;
                }
                sm.0 += ai as i64;
                sm.1 += sa as i64;
            }
        }
        let trow = &mut tmp[(cy * gw) as usize..((cy + 1) * gw) as usize];
        for (t, &(ai, sa)) in trow.iter_mut().zip(sums.iter()) {
            *t = if sa == 0 { 0.5 } else { ai as f32 / (sa * 255) as f32 };
        }
    }
    let mut grid = vec![0f32; cells];
    hx_pass(&tmp, &mut grid, true, gw, gh, &wts);
    hx_pass(&grid, &mut tmp, false, gw, gh, &wts);
    let grid = tmp;
    // tables
    let mut lin = [0f32; 256];
    lin_table(&mut lin);
    let mut enc = vec![0u8; HX_ENCN + 1];
    srgb_table(&mut enc, HX_ENCN + 1);
    let s2 = sh as f64 / 50.0;
    let h2 = hl as f64 / 50.0;
    let mut gain = [0f32; HX_GAINN + 1];
    for (i, g) in gain.iter_mut().enumerate() {
        let m = i as f64 / 1024.0;
        let mut v = 1.0 - m;
        v *= v;
        v *= s2;
        v -= m * m * h2;
        *g = exp2(v) as f32;
    }
    let mut tab = vec![0u8; (HX_GAINN + 1) * 256];
    for (gi, &g) in gain.iter().enumerate() {
        for c in 0..256 {
            let v = minss(lin[c] * g, 1.0) * 4096.0 + 0.5;
            tab[gi * 256 + c] = enc[cvtt(v) as usize];
        }
    }
    Ok(Some(Box::new(HxAux { w: gw, h: gh, invs: 1.0 / s as f32, tab, grid })))
}

/// gy / gx of a pixel coordinate: ((c + 0.5) / s - 0.5) clamped to 0 .. n - 1
#[inline(always)]
fn hx_g(c: i32, invs: f32, top: f32) -> f32 {
    minss(maxss((c as f32 + 0.5) * invs - 0.5, 0.0), top)
}

fn hx_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(hx) = a.aux::<HxAux>() else {
        return copy_rows(a, dst);
    };
    rows(a, dst, |d, s, x0, y| {
        let gy = hx_g(y, hx.invs, (hx.h - 1) as f32);
        let iy = cvtt(gy);
        let fy = gy - iy as f32;
        let iy1 = (iy + 1).min(hx.h - 1);
        let r0 = &hx.grid[(iy * hx.w) as usize..];
        let r1 = &hx.grid[(iy1 * hx.w) as usize..];
        let wm1 = hx.w - 1;
        let fw = wm1 as f32;
        let n = (s.len() / 4) as i32;
        let mut v = [0f32; 260];
        let mut done = 0i32;
        while n - done > 0 {
            let x = x0 + done;
            let cn = (n - done).min(256);
            let c0 = cvtt(hx_g(x, hx.invs, fw));
            let c1 = (cvtt(hx_g(x - 1 + cn, hx.invs, fw)) + 1).min(wm1);
            let mut k = 0i32;
            while c0 + k <= c1 && k <= 259 {
                let col = (c0 + k) as usize;
                v[k as usize] = r0[col] + (r1[col] - r0[col]) * fy;
                k += 1;
            }
            let (b0, b1) = (done as usize * 4, (done + cn) as usize * 4);
            for (p, (dq, sq)) in d[b0..b1].chunks_exact_mut(4).zip(s[b0..b1].chunks_exact(4)).enumerate() {
                let gx = hx_g(x + p as i32, hx.invs, fw);
                let ix = cvtt(gx);
                let fx = gx - ix as f32;
                let ix1 = (ix + 1).min(wm1);
                let va = v[(ix - c0) as usize];
                let vb = v[(ix1 - c0) as usize];
                let m = va + (vb - va) * fx;
                let gi = cvtt(m * 1024.0 + 0.5).clamp(0, HX_GAINN as i32) as usize;
                let t: &[u8; 256] = hx.tab[gi * 256..gi * 256 + 256].try_into().unwrap();
                let q = u32::from_le_bytes([sq[0], sq[1], sq[2], sq[3]]);
                let o = q & 0xFF00_0000 | t[(q & 255) as usize] as u32 | (t[(q >> 8 & 255) as usize] as u32) << 8 | (t[(q >> 16 & 255) as usize] as u32) << 16;
                dq.copy_from_slice(&o.to_le_bytes());
            }
            done += cn;
        }
    })
}

// ---------------------------------------------------------------- descriptors

static PROPS_BC: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "brightness", -100.0, 100.0, 0.0),
    FxProp::new(FP_INT, 0, 1, 0, "contrast", -100.0, 100.0, 0.0),
];
static PROPS_CURVES: [FxProp; 5] = [
    FxProp::choice(0, "mode", 1.0, 0.0, &["luminosity", "rgb"]),
    FxProp::new(FP_BLOB, 0, 1, 0, "luminosity", 0.0, 5.0, 0.0),
    FxProp::new(FP_BLOB, 0, 6, 0, "red", 0.0, 5.0, 0.0),
    FxProp::new(FP_BLOB, 0, 11, 0, "green", 0.0, 5.0, 0.0),
    FxProp::new(FP_BLOB, 0, 16, 0, "blue", 0.0, 5.0, 0.0),
];
static PROPS_EXPOSURE: [FxProp; 1] = [FxProp::new(FP_DEC, 2, 0, 0, "exposure", -5.0, 5.0, 0.0)];
static PROPS_HX: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "shadows", -100.0, 100.0, 0.0),
    FxProp::new(FP_INT, 0, 1, 0, "highlights", -100.0, 100.0, 0.0),
    FxProp::new(FP_INT, 0, 2, 0, "radius", 1.0, 200.0, 30.0),
];
static PROPS_HS: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "hue", -180.0, 180.0, 0.0),
    FxProp::new(FP_INT, 0, 1, 0, "saturation", 0.0, 200.0, 100.0),
    FxProp::new(FP_INT, 0, 2, 0, "lightness", -100.0, 100.0, 0.0),
];
static PROPS_LEVELS: [FxProp; 10] = [
    FxProp::color(0, "input_black", 0xFF00_0000),
    FxProp::color(1, "input_white", 0xFFFF_FFFF),
    FxProp::color(2, "output_black", 0xFF00_0000),
    FxProp::color(3, "output_white", 0xFFFF_FFFF),
    FxProp::new(FP_DEC, 2, 4, 0, "gamma_red", 0.1, 10.0, 1.0),
    FxProp::new(FP_DEC, 2, 5, 0, "gamma_green", 0.1, 10.0, 1.0),
    FxProp::new(FP_DEC, 2, 6, 0, "gamma_blue", 0.1, 10.0, 1.0),
    FxProp::new(FP_BOOL, 0, 7, 0, "red", 0.0, 1.0, 1.0),
    FxProp::new(FP_BOOL, 0, 8, 0, "green", 0.0, 1.0, 1.0),
    FxProp::new(FP_BOOL, 0, 9, 0, "blue", 0.0, 1.0, 1.0),
];
static PROPS_POST: [FxProp; 4] = [
    FxProp::new(FP_INT, 0, 0, 0, "red", 2.0, 64.0, 16.0),
    FxProp::new(FP_INT, 0, 1, 0, "green", 2.0, 64.0, 16.0),
    FxProp::new(FP_INT, 0, 2, 0, "blue", 2.0, 64.0, 16.0),
    FxProp::new(FP_BOOL, 0, 3, 0, "linked", 0.0, 1.0, 1.0),
];
static PROPS_TT: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "temperature", -100.0, 100.0, 0.0),
    FxProp::new(FP_INT, 0, 1, 0, "tint", -100.0, 100.0, 0.0),
];

const fn desc(id: u32, flags: u32, name: &'static str, prepare: Option<PrepareFn>, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags, name, prepare, render, props, category: FXC_ADJUST, user: 0 }
}

/// the group table (menu order of paint.net)  [fxtab_adj]
pub static FXTAB: [FxDesc; 13] = [
    desc(FX_AUTO_LEVEL, FXF_ADJ | FXF_PREPARE | FXF_NO_DIALOG, "auto_level", Some(al_prepare), lut_render, &[]),
    desc(FX_BLACK_WHITE, FXF_ADJ | FXF_NO_DIALOG, "black_white", None, bw_render, &[]),
    desc(FX_BRIGHT_CONTRAST, FXF_ADJ, "brightness_contrast", None, bc_render, &PROPS_BC),
    desc(FX_CURVES, FXF_ADJ | FXF_PREPARE, "curves", Some(cv_prepare), cv_render, &PROPS_CURVES),
    desc(FX_EXPOSURE, FXF_ADJ | FXF_PREPARE, "exposure", Some(ex_prepare), lut_render, &PROPS_EXPOSURE),
    desc(FX_HIGHLIGHTS_SHADOWS, FXF_ADJ | FXF_PREPARE, "highlights_shadows", Some(hx_prepare), hx_render, &PROPS_HX),
    desc(FX_HUE_SATURATION, FXF_ADJ, "hue_saturation", None, hs_render, &PROPS_HS),
    desc(FX_INVERT_ALPHA, FXF_ADJ | FXF_NO_DIALOG, "invert_alpha", None, inva_render, &[]),
    desc(FX_INVERT_COLORS, FXF_ADJ | FXF_NO_DIALOG, "invert_colors", None, invc_render, &[]),
    desc(FX_LEVELS, FXF_ADJ | FXF_PREPARE, "levels", Some(lv_prepare), lut_render, &PROPS_LEVELS),
    desc(FX_POSTERIZE, FXF_ADJ, "posterize", None, post_render, &PROPS_POST),
    desc(FX_SEPIA, FXF_ADJ | FXF_NO_DIALOG, "sepia", None, sepia_render, &[]),
    desc(FX_TEMP_TINT, FXF_ADJ | FXF_PREPARE, "temperature_tint", Some(tt_prepare), lut_render, &PROPS_TT),
];

// sepia tables: Level(gamma B 1.2, R 0.8) of the intensity (G = identity)
static SEP_B: [u8; 256] = [
    0, 0, 0, 1, 1, 2, 2, 3, 4, 4, 5, 5, 6, 7, 7, 8, 9, 9, 10, 11, 12, 12, 13, 14, 14, 15, 16, 17, 18, 18, 19, 20, //
    21, 21, 22, 23, 24, 25, 25, 26, 27, 28, 29, 30, 30, 31, 32, 33, 34, 35, 36, 36, 37, 38, 39, 40, 41, 42, 43, 44, 44, 45, 46, 47, //
    48, 49, 50, 51, 52, 53, 54, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 76, 77, //
    78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, //
    111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 137, 138, 139, 140, 141, 142, 143, 144, //
    145, 146, 147, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 160, 161, 162, 163, 164, 165, 166, 167, 169, 170, 171, 172, 173, 174, 175, 176, 178, 179, 180, //
    181, 182, 183, 184, 185, 187, 188, 189, 190, 191, 192, 193, 195, 196, 197, 198, 199, 200, 202, 203, 204, 205, 206, 207, 208, 210, 211, 212, 213, 214, 215, 217, //
    218, 219, 220, 221, 222, 224, 225, 226, 227, 228, 230, 231, 232, 233, 234, 235, 237, 238, 239, 240, 241, 243, 244, 245, 246, 247, 249, 250, 251, 252, 253, 255,
];
static SEP_R: [u8; 256] = [
    0, 3, 5, 7, 9, 10, 12, 14, 15, 17, 19, 20, 22, 23, 25, 26, 27, 29, 30, 31, 33, 34, 35, 37, 38, 39, 41, 42, 43, 44, 46, 47, //
    48, 49, 50, 52, 53, 54, 55, 56, 57, 59, 60, 61, 62, 63, 64, 65, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 79, 80, 81, 82, 83, //
    84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, //
    116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 145, //
    146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 165, 166, 167, 168, 169, 170, 171, 172, 172, 173, 174, //
    175, 176, 177, 178, 179, 180, 180, 181, 182, 183, 184, 185, 186, 186, 187, 188, 189, 190, 191, 192, 192, 193, 194, 195, 196, 197, 198, 198, 199, 200, 201, 202, //
    203, 204, 204, 205, 206, 207, 208, 209, 209, 210, 211, 212, 213, 214, 214, 215, 216, 217, 218, 219, 219, 220, 221, 222, 223, 224, 224, 225, 226, 227, 228, 229, //
    229, 230, 231, 232, 233, 233, 234, 235, 236, 237, 238, 238, 239, 240, 241, 242, 242, 243, 244, 245, 246, 246, 247, 248, 249, 250, 250, 251, 252, 253, 254, 255,
];
