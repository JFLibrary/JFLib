// eaq -- adaptive quantization, inverse gaborish and the quantizer of the VarDCT writer
// (libjxl 0.11.1 enc_adaptive_quantization.cc InitialQuantField / AdjustQuantField /
// InitialQuantDC, enc_gaborish.cc, quantizer.cc, convolve Symmetric5)       [jxl-plus]
//
//   Img                        three f32 planes of w x h (row stride w)
//   symmetric5(src, w, h, wts, out)       5x5 symmetric convolution, mirrored borders
//   gaborish_inverse(img, threads)        GaborishInverse (weights 1)
//   initial_quant_dc(d) -> f32            InitialQuantDC
//   aq_map(d, img, scale, threads) -> (qf, mask, mask1x1)   InitialQuantField (8x8 blocks)
//   adjust_quant_field(acs, xb, yb, d, qf)  AdjustQuantField
//   Quantizer                  global scale / quant DC, steps (quantizer.cc)

use alloc::vec;
use alloc::vec::Vec;

use simd::{Kernel, Simd};

use crate::emath::{fast_log2f, fast_pow2f, log1p, sqrtf};
use crate::emod::par_map;

/// three planes
#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub p: [Vec<f32>; 3],
}

impl Img {
    pub fn new(w: usize, h: usize) -> Img {
        Img { w, h, p: [vec![0.0; w * h], vec![0.0; w * h], vec![0.0; w * h]] }
    }
}

#[inline(always)]
fn mirror(i: i64, n: i64) -> usize {
    let mut i = i;
    loop {
        if i < 0 {
            i = -i - 1;
        } else if i >= n {
            i = 2 * n - 1 - i;
        } else {
            return i as usize;
        }
    }
}

/// one output sample of Symmetric5 (mirrored borders)
#[inline]
fn sym5_px(src: &[f32], w: usize, h: usize, wts: &[f32; 6], x: usize, y: usize) -> f32 {
    let [c, r, rr, d, dd, l] = *wts;
    let (wi, hi) = (w as i64, h as i64);
    let (xi, yi) = (x as i64, y as i64);
    let inner = xi >= 2 && xi + 2 < wi && yi >= 2 && yi + 2 < hi;
    let at = |dx: i64, dy: i64| {
        if inner { src[((yi + dy) as usize) * w + (xi + dx) as usize] } else { src[mirror(yi + dy, hi) * w + mirror(xi + dx, wi)] }
    };
    let sr = (at(-1, 0) + at(1, 0)) + (at(0, -1) + at(0, 1));
    let srr = (at(-2, 0) + at(2, 0)) + (at(0, -2) + at(0, 2));
    let sd = (at(-1, -1) + at(1, -1)) + (at(-1, 1) + at(1, 1));
    let sdd = (at(-2, -2) + at(2, -2)) + (at(-2, 2) + at(2, 2));
    let sl = ((at(-1, -2) + at(1, -2)) + (at(-2, -1) + at(2, -1))) + ((at(-2, 1) + at(2, 1)) + (at(-1, 2) + at(1, 2)));
    at(0, 0) * c + sr * r + srr * rr + sd * d + sdd * dd + sl * l
}

/// Symmetric5 of rows y0 .. y1: the interior samples L at a time (the scalar formula per
/// lane), the borders with sym5_px  [jxl-speed]
struct Sym5K<'a> {
    src: &'a [f32],
    w: usize,
    h: usize,
    wts: &'a [f32; 6],
    out: &'a mut [f32],
    y0: usize,
    y1: usize,
}

impl Kernel for Sym5K<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let l = S::BYTES / 4;
        let (w, h, src) = (self.w, self.h, self.src);
        let [c, r, rr, d, dd, ll] = *self.wts;
        let (vc, vr, vrr, vd, vdd, vl) = (s.splatf(c), s.splatf(r), s.splatf(rr), s.splatf(d), s.splatf(dd), s.splatf(ll));
        for y in self.y0..self.y1 {
            let o = (y - self.y0) * w;
            let mut x = 0;
            if y >= 2 && y + 2 < h && w >= 4 {
                while x < 2 {
                    self.out[o + x] = sym5_px(src, w, h, self.wts, x, y);
                    x += 1;
                }
                let rows = [(y - 2) * w, (y - 1) * w, y * w, (y + 1) * w, (y + 2) * w];
                while x + l + 2 <= w {
                    // p(dx, dy) = the samples at (x + dx - 2, y + dy - 2)
                    let b = [rows[0] + x - 2, rows[1] + x - 2, rows[2] + x - 2, rows[3] + x - 2, rows[4] + x - 2];
                    macro_rules! p {
                        ($dx:expr, $dy:expr) => {
                            s.load_f32(&src[b[$dy] + $dx..])
                        };
                    }
                    let sr = s.addf(s.addf(p!(1, 2), p!(3, 2)), s.addf(p!(2, 1), p!(2, 3)));
                    let srr = s.addf(s.addf(p!(0, 2), p!(4, 2)), s.addf(p!(2, 0), p!(2, 4)));
                    let sd = s.addf(s.addf(p!(1, 1), p!(3, 1)), s.addf(p!(1, 3), p!(3, 3)));
                    let sdd = s.addf(s.addf(p!(0, 0), p!(4, 0)), s.addf(p!(0, 4), p!(4, 4)));
                    let sl = s.addf(
                        s.addf(s.addf(p!(1, 0), p!(3, 0)), s.addf(p!(0, 1), p!(4, 1))),
                        s.addf(s.addf(p!(0, 3), p!(4, 3)), s.addf(p!(1, 4), p!(3, 4))),
                    );
                    let mut v = s.mulf(p!(2, 2), vc);
                    v = s.addf(v, s.mulf(sr, vr));
                    v = s.addf(v, s.mulf(srr, vrr));
                    v = s.addf(v, s.mulf(sd, vd));
                    v = s.addf(v, s.mulf(sdd, vdd));
                    v = s.addf(v, s.mulf(sl, vl));
                    s.store_f32(&mut self.out[o + x..], v);
                    x += l;
                }
            }
            while x < w {
                self.out[o + x] = sym5_px(src, w, h, self.wts, x, y);
                x += 1;
            }
        }
    }
}

/// WeightsSymmetric5 {c, r, R, d, D, L}: centre, 4 orthogonal at 1, 4 orthogonal at 2, 4
/// diagonal at (1, 1), 4 diagonal at (2, 2), 8 at (1, 2)
pub fn symmetric5_rows(src: &[f32], w: usize, h: usize, wts: &[f32; 6], out: &mut [f32], y0: usize, y1: usize) {
    simd::dispatch(Sym5K { src, w, h, wts, out, y0, y1 });
}

/// a whole-plane Symmetric5 (rows in parallel bands)
pub fn symmetric5(src: &[f32], w: usize, h: usize, wts: &[f32; 6], threads: bool) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    symmetric5_into(src, w, h, wts, threads, &mut out);
    out
}

/// Symmetric5 of a whole plane into out (w x h), bands of rows straight into out  [jxl-speed]
pub fn symmetric5_into(src: &[f32], w: usize, h: usize, wts: &[f32; 6], threads: bool, out: &mut [f32]) {
    crate::emod::par_rows(vec![(&mut out[..w * h], w)], h, 16, threads, |y0, n, b| {
        symmetric5_rows(src, w, h, wts, &mut b[0][..], y0, y0 + n);
    });
}

/// normalized WeightsSymmetric5 of kernel k (k[0] r, k[1] d, k[2] R, k[3] L, k[4] D)
fn weights5(k: &[f64; 5], mul: f64) -> [f32; 6] {
    let mut sum = 1.0 + mul * 4.0 * (k[0] + k[1] + k[2] + k[4] + 2.0 * k[3]);
    if sum < 1e-5 {
        sum = 1e-5;
    }
    let n = (1.0 / sum) as f32;
    let nm = (mul as f32) * n;
    [n, nm * k[0] as f32, nm * k[2] as f32, nm * k[1] as f32, nm * k[4] as f32, nm * k[3] as f32]
}

/// GaborishInverse with weights 1
pub fn gaborish_inverse(img: &mut Img, threads: bool) {
    static K: [f64; 5] = [
        -0.094_958_156_713_400_26,
        -0.041_031_725_066_768_575,
        0.013_710_004_822_696_948,
        0.006_510_206_083_837_737,
        -0.001_478_906_337_827_224_2,
    ];
    let wts = weights5(&K, 1.0);
    // one spare plane, swapped with each result (jxl-speed)
    let mut spare = vec![0f32; img.w * img.h];
    for c in 0..3 {
        symmetric5_into(&img.p[c], img.w, img.h, &wts, threads, &mut spare);
        core::mem::swap(&mut img.p[c], &mut spare);
    }
}

// ------------------------------------------------------------------------- quant field

const K_SGMUL: f32 = 226.772_16;
const K_SGMUL2: f32 = 1.0 / 73.377_13;
const K_LOG2: f32 = 0.693_147_2;
const K_SGRETMUL: f32 = K_SGMUL2 * 18.658_093 * K_LOG2;
const K_SGVOFFSET: f32 = 7.782_599;

/// RatioOfDerivativesOfCubicRootToSimpleGamma
#[inline(always)]
fn ratio(v: f32, invert: bool) -> f32 {
    let eps = 1e-2f32;
    let v = if v > 0.0 { v } else { 0.0 };
    let num_mul = K_SGRETMUL * 3.0 * K_SGMUL;
    let voff = K_SGVOFFSET * K_LOG2 + eps;
    let den_mul = K_LOG2 * K_SGMUL;
    let v2 = v * v;
    let num = num_mul * v2 + eps;
    let den = (den_mul * v) * v2 + voff;
    if invert { num / den } else { den / num }
}

#[inline(always)]
fn masking_sqrt(v: f32) -> f32 {
    const LOG_OFFSET: f32 = 27.505_837;
    const MUL: f32 = 211.665_68;
    0.25 * sqrtf(v * sqrtf(MUL * 1e8) + LOG_OFFSET)
}

#[inline(always)]
fn compute_mask(v: f32) -> f32 {
    const BASE: f32 = -0.7647;
    const MUL4: f32 = 9.470_874;
    const MUL2: f32 = 17.350_365;
    const OFF2: f32 = 302.595_88;
    const MUL3: f32 = 6.794_325;
    const OFF3: f32 = 3.717_963_7;
    const OFF4: f32 = 0.25 * OFF3;
    const MUL0: f32 = 0.800_617_6;
    let v1 = (v * MUL0).max(1e-3);
    let v2 = 1.0 / (v1 + OFF2);
    let v3 = 1.0 / (v1 * v1 + OFF3);
    let v4 = 1.0 / (v1 * v1 + OFF4);
    BASE + (MUL4 * v4 + (MUL2 * v2 + MUL3 * v3))
}

fn store_min4(v: f32, m: &mut [f32; 4]) {
    if v < m[3] {
        if v < m[0] {
            m[3] = m[2];
            m[2] = m[1];
            m[1] = m[0];
            m[0] = v;
        } else if v < m[1] {
            m[3] = m[2];
            m[2] = m[1];
            m[1] = v;
        } else if v < m[2] {
            m[3] = m[2];
            m[2] = v;
        } else {
            m[3] = v;
        }
    }
}

/// InitialQuantDC
pub fn initial_quant_dc(d: f32) -> f32 {
    const DC_MUL: f32 = 0.3;
    const DC_QUANT_POW: f32 = 0.83;
    const DC_QUANT: f32 = 1.095_924;
    let p = crate::emath::fast_powf((1.0 / DC_MUL) * d, DC_QUANT_POW);
    let dd = (0.5 * d).max(d.min(DC_MUL * p));
    (DC_QUANT / dd).min(50.0)
}

pub const AC_QUANT: f32 = 0.725;

/// the per-tile part of AdaptiveQuantizationMap: blocks bx0..bx1 x by0..by1 of the image
/// (padded to blocks) -> aq values, masks, and the tile's mask1x1 rows
struct TileOut {
    aq: Vec<f32>,
    mask: Vec<f32>,
}

/// mask1x1 of rows y0 .. y0 + n of the image into out (row stride img.w)  [jxl-speed: a pass
/// of its own, formerly per tile]
fn mask1x1_rows(img: &Img, y0: usize, n: usize, out: &mut [f32]) {
    let (xs, ys) = (img.w, img.h);
    let py = &img.p[1];
    let mg = 0.019f32;
    for y in y0..y0 + n {
        let y2 = if y + 1 < ys { y + 1 } else { y };
        let y1 = if y > 0 { y - 1 } else { y };
        for x in 0..xs {
            let x2 = if x + 1 < xs { x + 1 } else { x };
            let x1 = if x > 0 { x - 1 } else { x };
            let c = py[y * xs + x];
            let base = 0.25 * (py[y2 * xs + x] + py[y1 * xs + x] + py[y * xs + x1] + py[y * xs + x2]);
            let g = ratio(c + mg, false);
            let mut diff = (g * (c - base)).abs();
            diff = log1p(diff);
            out[(y - y0) * xs + x] = 1.0 / (diff + 0.01);
        }
    }
}

fn aq_tile(d: f32, scale: f32, img: &Img, bx0: usize, by0: usize, bx1: usize, by1: usize) -> TileOut {
    let (xs, ys) = (img.w, img.h);
    let py = &img.p[1];
    let mg = 0.019f32;
    let tw = bx1 - bx0;
    let th = by1 - by0;
    // pre-erosion: 4x4 averages of masked squared differences (4 pixels of margin)
    let mut x_start = bx0 * 8;
    let mut x_end = bx1 * 8;
    let mut y_start = by0 * 8;
    let mut y_end = by1 * 8;
    if x_start != 0 {
        x_start -= 4;
    }
    if x_end != xs {
        x_end += 4;
    }
    if y_start != 0 {
        y_start -= 4;
    }
    if y_end != ys {
        y_end += 4;
    }
    let pw = (x_end - x_start) / 4;
    let ph = (y_end - y_start) / 4;
    let mut pre = vec![0f32; pw * ph];
    let mut rowbuf = vec![0f32; x_end - x_start];
    for y in y_start..y_end {
        let y2 = if y + 1 < ys { y + 1 } else { y };
        let y1 = if y > 0 { y - 1 } else { y };
        for x in x_start..x_end {
            let x2 = if x + 1 < xs { x + 1 } else { x };
            let x1 = if x > 0 { x - 1 } else { x };
            let c = py[y * xs + x];
            let base = 0.25 * (py[y2 * xs + x] + py[y1 * xs + x] + py[y * xs + x1] + py[y * xs + x2]);
            let g = ratio(c + mg, false);
            let mut diff = g * (c - base);
            diff *= diff;
            if diff >= 0.2 {
                diff = 0.2;
            }
            diff = masking_sqrt(diff);
            if y % 4 != 0 {
                rowbuf[x - x_start] += diff;
            } else {
                rowbuf[x - x_start] = diff;
            }
        }
        if y % 4 == 3 {
            let r = (y - y_start) / 4;
            for x in 0..pw {
                pre[r * pw + x] = (rowbuf[x * 4] + rowbuf[x * 4 + 1] + rowbuf[x * 4 + 2] + rowbuf[x * 4 + 3]) * 0.25;
            }
        }
    }
    // FuzzyErosion into the tile's blocks
    let fx0 = if x_start % 8 == 0 { 0 } else { 1 };
    let fy0 = if y_start % 8 == 0 { 0 } else { 1 };
    let mut mul = 0.0f32;
    if d < 2.0 {
        mul = (2.0 - d) * 0.5;
    }
    let mut k = [0.125 + mul * 0.0, 0.10 + mul * -0.10, 0.09 + mul * -0.09, 0.06 + mul * -0.06];
    let norm = 0.299_597_06 / (k[0] + k[1] + k[2] + k[3]);
    for v in k.iter_mut() {
        *v *= norm;
    }
    let mut aq = vec![0f32; tw * th];
    for fy in 0..th * 2 {
        let y = fy + fy0;
        let ym1 = if y >= 1 { y - 1 } else { y };
        let yp1 = if y + 1 < ph { y + 1 } else { y };
        for fx in 0..tw * 2 {
            let x = fx + fx0;
            let xm1 = if x >= 1 { x - 1 } else { x };
            let xp1 = if x + 1 < pw { x + 1 } else { x };
            let at = |xx: usize, yy: usize| pre[yy * pw + xx];
            let mut m = [at(x, y), at(xm1, y), at(xp1, y), at(xm1, ym1)];
            if m[0] > m[1] {
                m.swap(0, 1);
            }
            if m[0] > m[2] {
                m.swap(0, 2);
            }
            if m[0] > m[3] {
                m.swap(0, 3);
            }
            if m[1] > m[2] {
                m.swap(1, 2);
            }
            if m[1] > m[3] {
                m.swap(1, 3);
            }
            if m[2] > m[3] {
                m.swap(2, 3);
            }
            store_min4(at(x, ym1), &mut m);
            store_min4(at(xp1, ym1), &mut m);
            store_min4(at(xm1, yp1), &mut m);
            store_min4(at(x, yp1), &mut m);
            store_min4(at(xp1, yp1), &mut m);
            let v = k[0] * m[0] + k[1] * m[1] + k[2] * m[2] + k[3] * m[3];
            let o = (fy / 2) * tw + fx / 2;
            if fx % 2 == 0 && fy % 2 == 0 {
                aq[o] = v;
            } else {
                aq[o] += v;
            }
        }
    }
    let mask: Vec<f32> = aq.iter().map(|&v| 1.0 / (v + 0.001)).collect();
    // PerBlockModulations
    let base_level = 0.48 * scale;
    let mut dampen = 1.0f32;
    if d >= 2.0 {
        dampen = 1.0 - (d - 2.0) / (14.0 - 2.0);
        if dampen < 0.0 {
            dampen = 0.0;
        }
    }
    let bmul = scale * dampen;
    let badd = (1.0 - dampen) * base_level;
    let (px, pb) = (&img.p[0], &img.p[2]);
    for iy in 0..th {
        for ix in 0..tw {
            let x = (bx0 + ix) * 8;
            let y = (by0 + iy) * 8;
            let mut v = compute_mask(aq[iy * tw + ix]);
            // HfModulation
            let mut sum = 0.0f32;
            for dy in 0..8 {
                let r = (y + dy) * xs + x;
                let rn = if dy == 7 { r } else { r + xs };
                for dx in 0..8 {
                    let p = py[r + dx];
                    if dx < 7 {
                        sum += (p - py[r + dx + 1]).abs().min(0.0206);
                    }
                    sum += (p - py[rn + dx]).abs().min(0.0206);
                }
            }
            v += sum * -0.38 + 0.42;
            // GammaModulation
            let mut overall = 0.0f32;
            for dy in 0..8 {
                let r = (y + dy) * xs + x;
                for dx in 0..8 {
                    let iny = py[r + dx] + 0.16;
                    let inx = px[r + dx];
                    overall += ratio(iny - inx, true);
                    overall += ratio(iny + inx, true);
                }
            }
            overall *= 0.5 / 64.0;
            v += 0.100_561_33 * fast_log2f(overall);
            // BlueModulation
            const LIMIT: f32 = 0.027_121_074;
            const OFFSET: f32 = 0.084_381_64;
            let mut s = 0.0f32;
            for dy in 0..8 {
                let r = (y + dy) * xs + x;
                for dx in 0..8 {
                    let pxv = px[r + dx];
                    let pbv = pb[r + dx];
                    let pye = (py[r + dx] + OFFSET) + pxv.abs();
                    if pbv > pye {
                        s += (pbv - pye).min(LIMIT);
                    }
                }
            }
            if s >= 32.0 * LIMIT {
                s = 64.0 * LIMIT - s;
            }
            const MAXL: f32 = 15.398_788;
            if s >= MAXL * LIMIT {
                s = MAXL * LIMIT;
            }
            v += s * 0.142_070_01;
            aq[iy * tw + ix] = fast_pow2f(v * 1.442_695) * bmul + badd;
        }
    }
    TileOut { aq, mask }
}

/// InitialQuantField: (quant field, mask, mask1x1) of the (block padded) XYB image
pub fn aq_map(d: f32, img: &Img, scale: f32, threads: bool) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let xb = img.w / 8;
    let yb = img.h / 8;
    let tx = xb.div_ceil(8);
    let ty = yb.div_ceil(8);
    let tiles: Vec<TileOut> = par_map(tx * ty, threads, |t| {
        let bx0 = (t % tx) * 8;
        let by0 = (t / tx) * 8;
        aq_tile(d, scale, img, bx0, by0, (bx0 + 8).min(xb), (by0 + 8).min(yb))
    });
    let mut qf = vec![0f32; xb * yb];
    let mut mask = vec![0f32; xb * yb];
    let mut m1 = vec![0f32; img.w * img.h];
    crate::emod::par_rows(vec![(&mut m1[..], img.w)], img.h, 16, threads, |y0, n, b| mask1x1_rows(img, y0, n, &mut b[0][..]));
    for (t, o) in tiles.iter().enumerate() {
        let bx0 = (t % tx) * 8;
        let by0 = (t / tx) * 8;
        let tw = (bx0 + 8).min(xb) - bx0;
        let th = (by0 + 8).min(yb) - by0;
        for iy in 0..th {
            for ix in 0..tw {
                qf[(by0 + iy) * xb + bx0 + ix] = o.aq[iy * tw + ix];
                mask[(by0 + iy) * xb + bx0 + ix] = o.mask[iy * tw + ix];
            }
        }
    }
    // Blur1x1Masking
    static K: [f64; 5] = [0.256_470_676_337_372_27, 0.205_005_691_235_439_9, 0.154_082_048_668_497_3, 0.081_495_765_913_620_04, 0.051_275_010_481_230_85];
    let wts = weights5(&K, 1.0);
    let m1 = symmetric5(&m1, img.w, img.h, &wts, threads);
    (qf, mask, m1)
}

/// AdjustQuantField: the quant field of a multi-block transform = max (or a max / mean mix
/// at larger distances) over its blocks
pub fn adjust_quant_field(acs: &[u8], xb: usize, yb: usize, d: f32, qf: &mut [f32]) {
    let mut mixer = 1.0f32;
    if d > 1.541_38 {
        mixer -= (d - 1.541_38) * 0.563_91;
        if mixer < 0.0 {
            mixer = 0.0;
        }
    }
    for y in 0..yb {
        for x in 0..xb {
            let a8 = acs[y * xb + x];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            let (cx, cy) = crate::etx::cover(a);
            let mut mx = qf[y * xb + x];
            let mut mean = 0.0f32;
            for iy in 0..cy {
                for ix in 0..cx {
                    let v = qf[(y + iy) * xb + x + ix];
                    mean += v;
                    mx = mx.max(v);
                }
            }
            mean /= (cx * cy) as f32;
            if cx * cy >= 4 {
                mx *= mixer;
                mx += (1.0 - mixer) * mean;
            }
            for iy in 0..cy {
                for ix in 0..cx {
                    qf[(y + iy) * xb + x + ix] = mx;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------- quantizer

pub const DC_WEIGHTS: [f32; 3] = [4096.0, 512.0, 256.0]; // 1 / default DC quant

/// the quantizer (global scale, quant DC)
#[derive(Clone, Copy)]
pub struct Quantizer {
    pub global_scale: i32,
    pub quant_dc: i32,
    pub inv_global_scale: f32,
    pub scale: f32,
    pub dcw: [f32; 3], // 1 / DC quantization of X, Y, B (DequantMatrices DC)
}

impl Quantizer {
    pub fn new() -> Quantizer {
        let mut q = Quantizer { global_scale: 65536 / 64, quant_dc: 64, inv_global_scale: 0.0, scale: 0.0, dcw: DC_WEIGHTS };
        q.recompute();
        q
    }
    fn recompute(&mut self) {
        self.scale = (self.global_scale as f64 * (1.0 / 65536.0)) as f32;
        self.inv_global_scale = (65536.0f64 / self.global_scale as f64) as f32;
    }
    /// ComputeGlobalScaleAndQuant
    pub fn compute(&mut self, quant_dc: f32, median: f32, absd: f32) {
        let mut sc = 65536.0f32 * (median - absd) / 5.0;
        if sc < 1.0 {
            sc = 1.0;
        }
        if sc > 32768.0 {
            sc = 32768.0;
        }
        let mut gs = sc as i32;
        let sq = (quant_dc * 4096.0 * 1.6) as i32;
        if gs > sq {
            gs = sq;
            if gs <= 0 {
                gs = 1;
            }
        }
        self.global_scale = gs;
        self.recompute();
        let fval = (quant_dc * self.inv_global_scale + 0.5).min(65536.0);
        self.quant_dc = fval as i32;
        self.recompute();
    }
    /// SetQuantFieldRect value
    #[inline]
    pub fn raw(&self, qf: f32) -> i32 {
        ((qf * self.inv_global_scale + 0.5) as i32).clamp(1, 256)
    }
    /// GetDcStep(c): the DC dequantization multiplier
    pub fn dc_step(&self, c: usize) -> f32 {
        (self.inv_global_scale / self.quant_dc as f32) / self.dcw[c]
    }
    /// GetInvDcStep(c)
    pub fn inv_dc_step(&self, c: usize) -> f32 {
        self.dcw[c] * (self.scale * self.quant_dc as f32)
    }
    /// inv_quant_ac(quant)
    pub fn inv_quant_ac(&self, quant: i32) -> f32 {
        self.inv_global_scale / quant as f32
    }
}

impl Default for Quantizer {
    fn default() -> Self {
        Quantizer::new()
    }
}
