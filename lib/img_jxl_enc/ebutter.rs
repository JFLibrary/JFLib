// ebutter -- Butteraugli, the perceptual distance the writer's effort 8 / 9 quantization loop
// minimizes (libjxl 0.7 lib/jxl/butteraugli/butteraugli.cc: OpsinDynamicsImage,
// SeparateFrequencies, MaltaDiffMap(LF), L2Diff(Asymmetric), MaskPsychoImage,
// CombineChannelsToDiffmap, the half-resolution comparator)            [jxl-tools: no asm]
//
//   Params                      hf_asymmetry, xmul, intensity target (the writer: 80 cd/m2)
//   Comparator::new(rgb, w, h, p)   the reference image (linear sRGB planes, 0..1)
//   cmp.diffmap(rgb) -> Vec<f32>    the per-pixel distance of another image of the same size
//   score(diffmap) -> f32           ButteraugliScoreFromDiffmap: the largest value
//   pnorm(diffmap, p) -> f64        ComputeDistanceP (p = 3: libjxl's "3-norm")
//
// The arithmetic follows libjxl's order where it is cheap to do so (plain float operations
// where libjxl's Highway code fuses multiply-adds); the distances agree with libjxl's
// JxlButteraugliCompute to a small fraction of a percent (jxl_plus_check suite butteraugli).

use alloc::vec;
use alloc::vec::Vec;

use crate::emod::par_map;

/// ButteraugliParams
#[derive(Clone, Copy)]
pub struct Params {
    pub hf_asym: f32,
    pub xmul: f32,
    pub intensity: f32,
    pub dbg: u32, // tests: 1 = no half-size level
}

impl Default for Params {
    fn default() -> Self {
        Params { hf_asym: 1.0, xmul: 1.0, intensity: 80.0, dbg: 0 }
    }
}

type Plane = Vec<f32>;

// ------------------------------------------------------------------ math helpers

use crate::spline::exp_f64;

#[inline]
fn sqrtf(x: f32) -> f32 {
    rt::fx::sqrt(x)
}

#[inline]
fn maxf(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

// ------------------------------------------------------------------ blur

/// ComputeKernel: a Gaussian of 2 * max(1, int(2.25 sigma)) + 1 taps
fn kernel(sigma: f32) -> Vec<f32> {
    let m = 2.25f32;
    let scaler = -1.0 / (2.0 * sigma as f64 * sigma as f64);
    let diff = ((m * sigma.abs()) as i32).max(1);
    (-diff..=diff).map(|i| exp_f64(scaler * (i * i) as f64) as f32).collect()
}

/// a horizontal convolution of the w x h plane, transposed (the result is h x w): the middle
/// with the normalized symmetric kernel, the borders renormalized over the taps inside
fn conv_transpose(inp: &[f32], w: usize, h: usize, k: &[f32], threads: bool) -> Plane {
    let len = k.len();
    let off = len / 2;
    let mut wsum = 0f32;
    for &v in k {
        wsum += v;
    }
    let sc = 1.0 / wsum;
    let sk: Vec<f32> = (0..=off).map(|i| k[i] * sc).collect();
    let b1 = w.min(off);
    let b2 = if w > off { w - off } else { 0 };
    // rows of the output = columns of the input: compute per input row, then scatter
    let rows: Vec<Vec<f32>> = par_map(h, threads, |y| {
        let r = &inp[y * w..(y + 1) * w];
        let mut o = vec![0f32; w];
        for x in b1..b2.max(b1) {
            let p = &r[x - off..x + off + 1];
            let mut s = p[off] * sk[off];
            for i in 0..off {
                s += (p[i] + p[len - 1 - i]) * sk[i];
            }
            o[x] = s;
        }
        // borders
        let border = |x: usize, o: &mut [f32]| {
            let minx = x.saturating_sub(off);
            let maxx = (w - 1).min(x + off);
            let mut wt = 0f32;
            for j in minx..=maxx {
                wt += k[j + off - x];
            }
            let scale = 1.0 / wt;
            let mut s = 0f32;
            for j in minx..=maxx {
                s += r[j] * k[j + off - x];
            }
            o[x] = s * scale;
        };
        for x in 0..b1 {
            border(x, &mut o);
        }
        for x in b2.max(b1)..w {
            border(x, &mut o);
        }
        o
    });
    let mut out = vec![0f32; w * h];
    for (y, r) in rows.iter().enumerate() {
        for (x, &v) in r.iter().enumerate() {
            out[x * h + y] = v;
        }
    }
    out
}

/// Separable5 with mirrored borders (the 5-tap kernels: sigma 1.2)
fn separable5(inp: &[f32], w: usize, h: usize, k: &[f32], threads: bool) -> Plane {
    let mut s = 0f32;
    for &v in k {
        s += v;
    }
    let sc = 1.0 / s;
    let (w0, w1, w2) = (k[2] * sc, k[1] * sc, k[0] * sc);
    let mirror = |i: i64, n: usize| -> usize {
        let n = n as i64;
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
    };
    let horz = |y: usize, x: usize| -> f32 {
        let r = &inp[y * w..(y + 1) * w];
        let at = |d: i64| r[mirror(x as i64 + d, w)];
        let c = at(0) * w0;
        let m1 = (at(-1) + at(1)) * w1 + c;
        (at(-2) + at(2)) * w2 + m1
    };
    let rows: Vec<Vec<f32>> = par_map(h, threads, |y| {
        let ys: [usize; 5] = core::array::from_fn(|i| mirror(y as i64 + i as i64 - 2, h));
        (0..w)
            .map(|x| {
                let c0 = horz(ys[2], x) * w0;
                let c1 = (horz(ys[1], x) + horz(ys[3], x)) * w1 + c0;
                (horz(ys[0], x) + horz(ys[4], x)) * w2 + c1
            })
            .collect()
    });
    rows.concat()
}

/// Blur
fn blur(inp: &[f32], w: usize, h: usize, sigma: f32, threads: bool) -> Plane {
    let k = kernel(sigma);
    if k.len() == 5 {
        return separable5(inp, w, h, &k, threads);
    }
    let t = conv_transpose(inp, w, h, &k, threads);
    conv_transpose(&t, h, w, &k, threads)
}

// ------------------------------------------------------------------ opsin dynamics

#[inline]
fn opsin_absorbance(r: f32, g: f32, b: f32) -> [f32; 3] {
    const M: [f32; 12] = [
        0.299_565_5,
        0.633_730_9,
        0.077_705_62,
        1.755_748_4,
        0.221_586_9,
        0.693_913_9,
        0.098_731_36,
        1.755_748_4,
        0.02,
        0.02,
        0.204_801_29,
        12.226_455,
    ];
    [
        M[0] * r + (M[1] * g + (M[2] * b + M[3])),
        M[4] * r + (M[5] * g + (M[6] * b + M[7])),
        M[8] * r + (M[9] * g + (M[10] * b + M[11])),
    ]
}

/// Gamma: 19.245 ln(v + 9.971) - 23.16 (FastLog2f)
#[inline]
fn gamma(v: f32) -> f32 {
    let v = if v < 0.0 { 0.0 } else { v };
    let biased = v + 9.971_063_576_929_914_5;
    let l = crate::emath::fast_log2f(biased);
    (19.245_013_259_874_995f32 * 0.693_147_180_559_945f32) * l + (-23.160_462_398_057_55)
}

/// OpsinDynamicsImage: X, Y, B of the linear RGB planes
fn opsin_dynamics(rgb: &[Plane; 3], w: usize, h: usize, p: &Params, threads: bool) -> [Plane; 3] {
    let bl: Vec<Plane> = (0..3).map(|c| blur(&rgb[c], w, h, 1.2, threads)).collect();
    let it = p.intensity;
    let rows: Vec<[Vec<f32>; 3]> = par_map(h, threads, |y| {
        let mut o = [vec![0f32; w], vec![0f32; w], vec![0f32; w]];
        for x in 0..w {
            let i = y * w + x;
            let pre = opsin_absorbance(bl[0][i] * it, bl[1][i] * it, bl[2][i] * it);
            let mut sens = [0f32; 3];
            for c in 0..3 {
                let pm = maxf(pre[c], 1e-4);
                sens[c] = maxf(gamma(pm) / pm, 1e-4);
            }
            let cur = opsin_absorbance(rgb[0][i] * it, rgb[1][i] * it, rgb[2][i] * it);
            let c0 = maxf(cur[0] * sens[0], 1.755_748_4);
            let c1 = maxf(cur[1] * sens[1], 1.755_748_4);
            let c2 = maxf(cur[2] * sens[2], 12.226_455);
            o[0][x] = c0 - c1;
            o[1][x] = c0 + c1;
            o[2][x] = c2;
        }
        o
    });
    let mut out = [vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]];
    for (y, r) in rows.into_iter().enumerate() {
        for c in 0..3 {
            out[c][y * w..(y + 1) * w].copy_from_slice(&r[c]);
        }
    }
    out
}

// ------------------------------------------------------------------ frequencies

#[inline]
fn remove_range(w: f32, x: f32) -> f32 {
    if x > w {
        x - w
    } else if x < -w {
        x + w
    } else {
        0.0
    }
}

#[inline]
fn amplify_range(w: f32, x: f32) -> f32 {
    if x > w {
        x + w
    } else if x < -w {
        x - w
    } else {
        x + x
    }
}

#[inline]
fn maximum_clamp(v: f32, maxval: f32) -> f32 {
    const MUL: f32 = 0.724_216_15;
    if v >= maxval {
        (v - maxval) * MUL + maxval
    } else if v < -maxval {
        (v + maxval) * MUL - maxval
    } else {
        v
    }
}

/// PsychoImage
struct Psycho {
    uhf: [Plane; 2],
    hf: [Plane; 2],
    mf: [Plane; 3],
    lf: [Plane; 3],
}

/// SeparateFrequencies
fn separate(xyb: &[Plane; 3], w: usize, h: usize, threads: bool) -> Psycho {
    const SIGMA_LF: f32 = 7.155_933_4;
    const SIGMA_HF: f32 = 3.224_899;
    const SIGMA_UHF: f32 = 1.564_163_3;
    let n = w * h;
    let mut lf: [Plane; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut mf: [Plane; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut hf: [Plane; 2] = [Vec::new(), Vec::new()];
    for i in 0..3 {
        lf[i] = blur(&xyb[i], w, h, SIGMA_LF, threads);
        let m: Plane = (0..n).map(|k| xyb[i][k] - lf[i][k]).collect();
        if i == 2 {
            mf[2] = blur(&m, w, h, SIGMA_HF, threads);
            break;
        }
        let hfi = m.clone();
        let mut mfi = blur(&m, w, h, SIGMA_HF, threads);
        let mut hfo = hfi;
        for k in 0..n {
            let mv = mfi[k];
            hfo[k] -= mv;
            mfi[k] = if i == 0 { remove_range(0.29, mv) } else { amplify_range(0.1, mv) };
        }
        mf[i] = mfi;
        hf[i] = hfo;
    }
    // suppress red-green by intensity change in the high frequencies
    {
        const S: f32 = 0.653_020_56;
        let yw = 46.0f32;
        let (h0, h1) = hf.split_at_mut(1);
        for k in 0..n {
            let vy = h1[0][k];
            let scaler = (yw / (vy * vy + yw)) * (1.0 - S) + S;
            h0[0][k] *= scaler;
        }
    }
    let mut uhf: [Plane; 2] = [Vec::new(), Vec::new()];
    for i in 0..2 {
        let u = hf[i].clone();
        let mut hb = blur(&hf[i], w, h, SIGMA_UHF, threads);
        let mut uo = u;
        for k in 0..n {
            if i == 0 {
                let hv = hb[k];
                let uv = uo[k] - hv;
                hb[k] = remove_range(1.5, hv);
                uo[k] = remove_range(0.04, uv);
            } else {
                let hv = maximum_clamp(hb[k], 28.469_18);
                let uv = maximum_clamp(uo[k] - hv, 5.191_753);
                uo[k] = uv * 2.693_137_6;
                hb[k] = amplify_range(0.132, hv * 2.155);
            }
        }
        hf[i] = hb;
        uhf[i] = uo;
    }
    // XybLowFreqToVals
    for k in 0..n {
        let (x, y, b) = (lf[0][k], lf[1][k], lf[2][k]);
        let bb = (-0.362_267_05f32) * y + b;
        lf[0][k] = x * 32.221_75;
        lf[1][k] = y * 13.769_779;
        lf[2][k] = bb * 47.504_616;
    }
    Psycho { uhf, hf, mf, lf }
}

// ------------------------------------------------------------------ Malta

/// MaltaUnit (libjxl 0.7, the AVX2 build's operation order): the sum of the squared sums of 16
/// lines of samples through (x, y) of the diffs plane (zero outside); the full kernel's last four
/// lines repeat lines 8, 7, 6, 5.  `at(dx, dy)` is the sample at (x + dx, y + dy).
#[inline(always)]
fn malta_full<F: Fn(i32, i32) -> f32>(at: F) -> f32 {
    let d0 = at(0, 0);
    let l1 = ((at(-3, 0) + at(-4, 0)) + (at(-2, 0) + at(-1, 0))) + ((at(3, 0) + at(2, 0)) + (at(1, 0) + d0)) + at(4, 0);
    let l2 = (((at(0, -3) + at(0, -4)) + (at(0, -2) + at(0, -1))) + ((at(0, 3) + at(0, 2)) + (at(0, 1) + d0))) + at(0, 4);
    let l3 = (((at(3, 3) + at(2, 2)) + (at(1, 1) + d0)) + at(-1, -1)) + (at(-2, -2) + at(-3, -3));
    let l4 = (((at(-3, 3) + at(-2, 2)) + (at(-1, 1) + d0)) + at(1, -1)) + (at(3, -3) + at(2, -2));
    let l5 = (((at(1, -3) + at(1, -4)) + (at(0, -1) + at(1, -2))) + ((at(-1, 3) + at(-1, 2)) + (at(0, 1) + d0))) + at(-1, 4);
    let l6 = (((at(-1, -3) + at(-1, -4)) + (at(0, -1) + at(-1, -2))) + ((at(1, 3) + at(1, 2)) + (at(0, 1) + d0))) + at(1, 4);
    let l7 = (((at(-3, -1) + at(-4, -1)) + (at(-1, 0) + at(-2, -1))) + ((at(3, 1) + at(2, 1)) + (at(1, 0) + d0))) + at(4, 1);
    let l8 = (((at(-3, 1) + at(-4, 1)) + (at(-1, 0) + at(-2, 1))) + ((at(3, -1) + at(2, -1)) + (at(1, 0) + d0))) + at(4, -1);
    let l9 = (((at(2, 3) + at(1, 2)) + (at(1, 1) + d0)) + at(-1, -1)) + (at(-2, -3) + at(-1, -2));
    let l10 = (((at(-2, 3) + at(-1, 2)) + (at(-1, 1) + d0)) + at(1, -1)) + (at(2, -3) + at(1, -2));
    let l11 = (((at(3, 2) + at(2, 1)) + (at(1, 1) + d0)) + at(-1, -1)) + (at(-3, -2) + at(-2, -1));
    let l12 = (((at(-3, 2) + at(-2, 1)) + (at(-1, 1) + d0)) + at(1, -1)) + (at(3, -2) + at(2, -1));
    let mut r = l1 * l1;
    for l in [l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l8, l7, l6, l5] {
        r = fma(l, l, r);
    }
    r
}

/// MaltaUnit of the LF kernel (5 samples per line, every second pixel)
#[inline(always)]
fn malta_lf<F: Fn(i32, i32) -> f32>(at: F) -> f32 {
    let d0 = at(0, 0);
    let line = |p1: (i32, i32), p2: (i32, i32), n1: (i32, i32), n2: (i32, i32)| -> f32 {
        ((at(p2.0, p2.1) + at(p1.0, p1.1)) + d0) + (at(n1.0, n1.1) + at(n2.0, n2.1))
    };
    let l = [
        line((2, 0), (4, 0), (-2, 0), (-4, 0)),
        line((0, 2), (0, 4), (0, -2), (0, -4)),
        line((2, 2), (3, 3), (-2, -2), (-3, -3)),
        line((-2, 2), (-3, 3), (3, -3), (2, -2)),
        line((-1, 2), (-1, 4), (1, -4), (1, -2)),
        line((1, 2), (1, 4), (-1, -4), (-1, -2)),
        line((2, 1), (4, 1), (-4, -1), (-2, -1)),
        line((2, -1), (4, -1), (-4, 1), (-2, 1)),
        line((1, 2), (2, 3), (-2, -3), (-1, -2)),
        line((-1, 2), (-2, 3), (2, -3), (1, -2)),
        line((2, 1), (3, 2), (-3, -2), (-2, -1)),
        line((-2, 1), (-3, 2), (3, -2), (2, -1)),
        line((2, -1), (4, -2), (-4, 2), (-2, 1)),
        line((2, 1), (4, 2), (-4, -2), (-2, -1)),
        line((1, 2), (2, 4), (-2, -4), (-1, -2)),
        line((-1, 2), (-2, 4), (2, -4), (1, -2)),
    ];
    let mut r = l[0] * l[0];
    for v in &l[1..] {
        r = fma(*v, *v, r);
    }
    r
}

/// a * b + c rounded once (nearly: through f64; libjxl's MulAdd is an FMA)
#[inline(always)]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64 * b as f64 + c as f64) as f32
}

/// MaltaUnit at (x, y) of the diffs plane (zero outside)
#[inline]
fn malta_unit(d: &[f32], w: usize, h: usize, x: usize, y: usize, lf: bool) -> f32 {
    if x >= 4 && y >= 4 && x + 4 < w && y + 4 < h {
        let c = y * w + x;
        let at = |dx: i32, dy: i32| d[(c as isize + dy as isize * w as isize + dx as isize) as usize];
        return if lf { malta_lf(at) } else { malta_full(at) };
    }
    let at = |dx: i32, dy: i32| -> f32 {
        let xx = x as i64 + dx as i64;
        let yy = y as i64 + dy as i64;
        if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h { d[yy as usize * w + xx as usize] } else { 0.0 }
    };
    if lf { malta_lf(at) } else { malta_full(at) }
}

/// MaltaDiffMap / MaltaDiffMapLF: adds to ac
#[allow(clippy::too_many_arguments)]
fn malta_diff(l0: &[f32], l1: &[f32], w: usize, h: usize, w0gt1: f64, w0lt1: f64, norm1: f64, lf: bool, ac: &mut [f32], threads: bool) {
    const LEN: f64 = 3.75;
    let mulli = if lf { 0.611_612_573_796f64 } else { 0.399_058_176_37f64 };
    let w_pre0gt1 = mulli * crate::spline::sqrt_f64(0.5 * w0gt1) / (LEN * 2.0 + 1.0);
    let w_pre0lt1 = mulli * crate::spline::sqrt_f64(0.33 * w0lt1) / (LEN * 2.0 + 1.0);
    let n2gt = (w_pre0gt1 * norm1) as f32;
    let n2lt = (w_pre0lt1 * norm1) as f32;
    let nf = norm1 as f32;
    let diffs: Vec<f32> = (0..w * h)
        .map(|k| {
            let (a, b) = (l0[k], l1[k]);
            let absval = 0.5 * (a.abs() + b.abs());
            let diff = a - b;
            let scaler = n2gt / (nf + absval);
            let mut r = scaler * diff;
            let scaler2 = n2lt / (nf + absval);
            let fabs0 = a.abs() as f64;
            let too_small = 0.55 * fabs0;
            let too_big = 1.05 * fabs0;
            let (a64, b64) = (a as f64, b as f64);
            if a < 0.0 {
                if b64 > -too_small {
                    r = (r as f64 - scaler2 as f64 * (b64 + too_small)) as f32;
                } else if b64 < -too_big {
                    r = (r as f64 + scaler2 as f64 * (-b64 - too_big)) as f32;
                }
            } else if b64 < too_small {
                r = (r as f64 + scaler2 as f64 * (too_small - b64)) as f32;
            } else if b64 > too_big {
                r = (r as f64 - scaler2 as f64 * (b64 - too_big)) as f32;
            }
            let _ = a64;
            r
        })
        .collect();
    let rows: Vec<Vec<f32>> = par_map(h, threads, |y| (0..w).map(|x| malta_unit(&diffs, w, h, x, y, lf)).collect());
    for (y, r) in rows.iter().enumerate() {
        for (x, &v) in r.iter().enumerate() {
            ac[y * w + x] += v;
        }
    }
}

/// L2DiffAsymmetric: adds to ac
fn l2_asym(i0: &[f32], i1: &[f32], w0gt1: f32, w0lt1: f32, ac: &mut [f32]) {
    if w0gt1 == 0.0 && w0lt1 == 0.0 {
        return;
    }
    let vgt = (w0gt1 as f64 * 0.8) as f32;
    let vlt = (w0lt1 as f64 * 0.8) as f32;
    for k in 0..ac.len() {
        let (v0, v1) = (i0[k], i1[k]);
        let diff = v0 - v1;
        let mut total = diff * diff * vgt + ac[k];
        let fabs0 = v0.abs();
        let too_small = 0.4 * fabs0;
        let too_big = fabs0;
        let v = if v0 < 0.0 {
            if v1 > -too_small {
                v1 + too_small
            } else if v1 < -too_big {
                -v1 - too_big
            } else {
                0.0
            }
        } else if v1 < too_small {
            too_small - v1
        } else if v1 > too_big {
            v1 - too_big
        } else {
            0.0
        };
        total += vlt * (v * v);
        ac[k] = total;
    }
}

/// L2Diff: adds to ac
fn l2_diff(i0: &[f32], i1: &[f32], w: f32, ac: &mut [f32]) {
    if w == 0.0 {
        return;
    }
    for k in 0..ac.len() {
        let d = i0[k] - i1[k];
        ac[k] += d * d * w;
    }
}

// ------------------------------------------------------------------ masking

/// kGlobalScale (as libjxl 0.7 stores it)
const GLOBAL_SCALE: f64 = 0.071_041_770_279_4;

/// CombineChannelsForMasking
fn combine_for_masking(hf: &[Plane; 2], uhf: &[Plane; 2]) -> Plane {
    (0..hf[0].len())
        .map(|k| {
            let xd = (uhf[0][k] + hf[0][k]) * 2.5;
            let yd = uhf[1][k] * 0.4 + hf[1][k] * 0.4;
            sqrtf(xd * xd + yd * yd)
        })
        .collect()
}

/// DiffPrecompute
fn diff_precompute(p: &[f32], mul: f32, bias_arg: f32) -> Plane {
    let bias = mul * bias_arg;
    let sb = sqrtf(bias);
    p.iter().map(|&v| sqrtf(mul * v.abs() + bias) - sb).collect()
}

#[inline]
fn store_min3(v: f32, m: &mut [f32; 3]) {
    if v < m[2] {
        if v < m[0] {
            m[2] = m[1];
            m[1] = m[0];
            m[0] = v;
        } else if v < m[1] {
            m[2] = m[1];
            m[1] = v;
        } else {
            m[2] = v;
        }
    }
}

/// FuzzyErosion
fn fuzzy_erosion(f: &[f32], w: usize, h: usize) -> Plane {
    const S: usize = 3;
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let v0 = f[y * w + x];
            let mut m = [v0, 2.0 * v0, 2.0 * v0];
            if x >= S {
                store_min3(f[y * w + x - S], &mut m);
                if y >= S {
                    store_min3(f[(y - S) * w + x - S], &mut m);
                }
                if y + S < h {
                    store_min3(f[(y + S) * w + x - S], &mut m);
                }
            }
            if x + S < w {
                store_min3(f[y * w + x + S], &mut m);
                if y >= S {
                    store_min3(f[(y - S) * w + x + S], &mut m);
                }
                if y + S < h {
                    store_min3(f[(y + S) * w + x + S], &mut m);
                }
            }
            if y >= S {
                store_min3(f[(y - S) * w + x], &mut m);
            }
            if y + S < h {
                store_min3(f[(y + S) * w + x], &mut m);
            }
            out[y * w + x] = 0.45 * m[0] + 0.3 * m[1] + 0.25 * m[2];
        }
    }
    out
}

/// Mask (MaskPsychoImage): the mask; adds the masking difference to diff_ac
fn mask(p0: &Psycho, p1: &Psycho, w: usize, h: usize, diff_ac: &mut [f32], threads: bool) -> Plane {
    const MUL: f32 = 6.194_240_8;
    const BIAS: f32 = 12.610_506;
    const RADIUS: f32 = 2.7;
    let m0 = combine_for_masking(&p0.hf, &p0.uhf);
    let m1 = combine_for_masking(&p1.hf, &p1.uhf);
    let d0 = diff_precompute(&m0, MUL, BIAS);
    let d1 = diff_precompute(&m1, MUL, BIAS);
    let b0 = blur(&d0, w, h, RADIUS, threads);
    let eroded = fuzzy_erosion(&b0, w, h);
    let b1 = blur(&d1, w, h, RADIUS, threads);
    for k in 0..w * h {
        let d = b0[k] - b1[k];
        diff_ac[k] += 10.0 * d * d;
    }
    eroded
}

/// MaskY (in f64, as libjxl 0.7)
#[inline]
fn mask_y(delta: f32) -> f32 {
    let c = 2.548_594_479_3 / (0.451_936_922_203 * delta as f64 + 0.829_591_754_942);
    let r = (c + 1.0) * GLOBAL_SCALE;
    (r * r) as f32
}

/// MaskDcY
#[inline]
fn mask_dc_y(delta: f32) -> f32 {
    let c = 0.505_054_525_019 / (3.874_494_188_04 * delta as f64 + 0.200_255_785_22);
    let r = (c + 1.0) * GLOBAL_SCALE;
    (r * r) as f32
}

// ------------------------------------------------------------------ the comparator

/// ButteraugliComparator: the reference image's frequencies, and the half-size comparator
pub struct Comparator {
    w: usize,
    h: usize,
    p: Params,
    pi0: Option<Psycho>,
    sub: Option<alloc::boxed::Box<Comparator>>,
    threads: bool,
}

/// SubSample2x
fn subsample2x(rgb: &[Plane; 3], w: usize, h: usize) -> ([Plane; 3], usize, usize) {
    let (xs, ys) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = [vec![0f32; xs * ys], vec![0f32; xs * ys], vec![0f32; xs * ys]];
    for c in 0..3 {
        for y in 0..h {
            for x in 0..w {
                out[c][(y / 2) * xs + x / 2] += 0.25 * rgb[c][y * w + x];
            }
        }
        if w & 1 != 0 {
            for y in 0..ys {
                out[c][y * xs + xs - 1] *= 2.0;
            }
        }
        if h & 1 != 0 {
            for x in 0..xs {
                out[c][(ys - 1) * xs + x] *= 2.0;
            }
        }
    }
    (out, xs, ys)
}

impl Comparator {
    /// the comparator of the reference image (linear sRGB planes w x h, 0..1)
    pub fn new(rgb: &[Plane; 3], w: usize, h: usize, p: Params, threads: bool) -> Comparator {
        let mut c = Comparator::level(rgb, w, h, p, threads);
        if c.pi0.is_some() {
            // the half-size level (libjxl builds its sub levels recursively but uses one)
            let (s, sw, sh) = subsample2x(rgb, w, h);
            c.sub = Some(alloc::boxed::Box::new(Comparator::level(&s, sw, sh, p, threads)));
        }
        c
    }

    /// one level: the reference's frequencies (none below 8 x 8 pixels)
    fn level(rgb: &[Plane; 3], w: usize, h: usize, p: Params, threads: bool) -> Comparator {
        if w < 8 || h < 8 {
            return Comparator { w, h, p, pi0: None, sub: None, threads };
        }
        let xyb = opsin_dynamics(rgb, w, h, &p, threads);
        let pi0 = separate(&xyb, w, h, threads);
        Comparator { w, h, p, pi0: Some(pi0), sub: None, threads }
    }

    /// Diffmap: the per-pixel distance of rgb (same size) from the reference
    pub fn diffmap(&self, rgb: &[Plane; 3]) -> Plane {
        let (w, h) = (self.w, self.h);
        let Some(pi0) = self.pi0.as_ref() else { return vec![0.0; w * h] };
        let xyb = opsin_dynamics(rgb, w, h, &self.p, self.threads);
        let mut out = self.diffmap_psycho(pi0, &separate(&xyb, w, h, self.threads));
        if let Some(sub) = self.sub.as_ref()
            && let Some(spi0) = sub.pi0.as_ref()
            && self.p.dbg & 1 == 0
        {
            // the half-size image: its own level only (libjxl calls the sub comparator's
            // DiffmapOpsinDynamicsImage, not its Diffmap), AddSupersampled2x with weight 0.5
            let (s, sw, sh) = subsample2x(rgb, w, h);
            let sxyb = opsin_dynamics(&s, sw, sh, &self.p, self.threads);
            let sd = sub.diffmap_psycho(spi0, &separate(&sxyb, sw, sh, self.threads));
            for y in 0..h {
                for x in 0..w {
                    let k = y * w + x;
                    let v = (out[k] as f64 * 0.85) as f32;
                    out[k] = v + 0.5 * sd[(y / 2) * sw + x / 2];
                }
            }
        }
        out
    }

    /// tests: the parts of the distance at pixel k of the full-size level: dc X, Y, B, ac X,
    /// Y, B (with the masking term), the masking term, the mask, MaskY, MaskDcY, the lf vals
    /// of both images (3 + 3), xyb of both (3 + 3)
    pub fn parts(&self, rgb: &[Plane; 3], k: usize) -> Vec<f32> {
        let (w, h) = (self.w, self.h);
        let Some(pi0) = self.pi0.as_ref() else { return Vec::new() };
        let xyb = opsin_dynamics(rgb, w, h, &self.p, self.threads);
        let pi1 = separate(&xyb, w, h, self.threads);
        let mut o = Vec::new();
        let (ac, dc, mk, madd) = self.parts_of(pi0, &pi1);
        for c in 0..3 {
            o.push(dc[c][k]);
        }
        for c in 0..3 {
            o.push(ac[c][k]);
        }
        o.push(madd[k]);
        o.push(mk[k]);
        o.push(mask_y(mk[k]));
        o.push(mask_dc_y(mk[k]));
        for c in 0..3 {
            o.push(pi0.lf[c][k]);
        }
        for c in 0..3 {
            o.push(pi1.lf[c][k]);
        }
        for c in 0..3 {
            o.push(xyb[c][k]);
        }
        // each term of ac at k: Malta uhf Y, uhf X, LF hf Y, hf X, mf Y, mf X; L2 asym hf X, Y;
        // L2 mf X, Y, B
        let n = w * h;
        let t = self.threads;
        let one = |f: &dyn Fn(&mut [f32])| -> f32 {
            let mut p = vec![0f32; n];
            f(&mut p);
            p[k]
        };
        o.push(one(&|p| malta_diff(&pi0.uhf[1], &pi1.uhf[1], w, h, 1.100_390_325_55, 1.100_390_325_55, 71.780_027_516_9, false, p, t)));
        o.push(one(&|p| malta_diff(&pi0.uhf[0], &pi1.uhf[0], w, h, 173.5, 173.5, 5.0, false, p, t)));
        o.push(one(&|p| malta_diff(&pi0.hf[1], &pi1.hf[1], w, h, 18.723_741_438_7, 18.723_741_438_7, 4_498_534.452_32, true, p, t)));
        o.push(one(&|p| malta_diff(&pi0.hf[0], &pi1.hf[0], w, h, 6_923.994_761_09, 6_923.994_761_09, 8_051.158_332_47, true, p, t)));
        o.push(one(&|p| malta_diff(&pi0.mf[1], &pi1.mf[1], w, h, 37.081_987_039_9, 37.081_987_039_9, 130_262_059.556, true, p, t)));
        o.push(one(&|p| malta_diff(&pi0.mf[0], &pi1.mf[0], w, h, 8_246.753_213_53, 8_246.753_213_53, 1_009_002.705_82, true, p, t)));
        o.push(one(&|p| l2_asym(&pi0.hf[0], &pi1.hf[0], 400.0, 400.0, p)));
        o.push(one(&|p| l2_asym(&pi0.hf[1], &pi1.hf[1], 1.508_157_031_18, 1.508_157_031_18, p)));
        o.push(one(&|p| l2_diff(&pi0.mf[0], &pi1.mf[0], 2150.0, p)));
        o.push(one(&|p| l2_diff(&pi0.mf[1], &pi1.mf[1], 10.619_543_323_9, p)));
        o.push(one(&|p| l2_diff(&pi0.mf[2], &pi1.mf[2], 16.217_604_315_2, p)));
        // the frequency planes at k: uhf X, Y; hf X, Y; mf X, Y, B of both images
        for pi in [pi0, &pi1] {
            o.extend_from_slice(&[pi.uhf[0][k], pi.uhf[1][k], pi.hf[0][k], pi.hf[1][k], pi.mf[0][k], pi.mf[1][k], pi.mf[2][k]]);
        }
        o
    }

    /// the planes of DiffmapPsychoImage: ac, dc, mask, the masking term of ac Y
    #[allow(clippy::type_complexity)]
    fn parts_of(&self, pi0: &Psycho, pi1: &Psycho) -> ([Plane; 3], [Plane; 3], Plane, Plane) {
        let (w, h) = (self.w, self.h);
        let n = w * h;
        let t = self.threads;
        let ha = self.p.hf_asym as f64;
        let sha = crate::spline::sqrt_f64(ha);
        let mut ac: [Plane; 3] = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        malta_diff(&pi0.uhf[1], &pi1.uhf[1], w, h, 1.100_390_325_55 * ha, 1.100_390_325_55 / ha, 71.780_027_516_9, false, &mut ac[1], t);
        malta_diff(&pi0.uhf[0], &pi1.uhf[0], w, h, 173.5 * ha, 173.5 / ha, 5.0, false, &mut ac[0], t);
        malta_diff(&pi0.hf[1], &pi1.hf[1], w, h, 18.723_741_438_7 * sha, 18.723_741_438_7 / sha, 4_498_534.452_32, true, &mut ac[1], t);
        malta_diff(&pi0.hf[0], &pi1.hf[0], w, h, 6_923.994_761_09 * sha, 6_923.994_761_09 / sha, 8_051.158_332_47, true, &mut ac[0], t);
        malta_diff(&pi0.mf[1], &pi1.mf[1], w, h, 37.081_987_039_9, 37.081_987_039_9, 130_262_059.556, true, &mut ac[1], t);
        malta_diff(&pi0.mf[0], &pi1.mf[0], w, h, 8_246.753_213_53, 8_246.753_213_53, 1_009_002.705_82, true, &mut ac[0], t);
        const WMUL: [f64; 9] = [400.0, 1.508_157_031_18, 0.0, 2150.0, 10.619_543_323_9, 16.217_604_315_2, 29.235_379_799_4, 0.844_626_970_982, 0.703_646_627_719];
        let mut dc: [Plane; 3] = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        for c in 0..3 {
            if c < 2 {
                l2_asym(&pi0.hf[c], &pi1.hf[c], (WMUL[c] * ha) as f32, (WMUL[c] / ha) as f32, &mut ac[c]);
            }
            l2_diff(&pi0.mf[c], &pi1.mf[c], WMUL[3 + c] as f32, &mut ac[c]);
            let wd = WMUL[6 + c] as f32;
            for k in 0..n {
                let d = pi0.lf[c][k] - pi1.lf[c][k];
                dc[c][k] = d * d * wd;
            }
        }
        let before = ac[1].clone();
        let mk = mask(pi0, pi1, w, h, &mut ac[1], t);
        let madd: Plane = (0..n).map(|k| ac[1][k] - before[k]).collect();
        (ac, dc, mk, madd)
    }

    /// DiffmapPsychoImage
    fn diffmap_psycho(&self, pi0: &Psycho, pi1: &Psycho) -> Plane {
        let (w, h) = (self.w, self.h);
        let n = w * h;
        let t = self.threads;
        let ha = self.p.hf_asym as f64;
        let sha = crate::spline::sqrt_f64(ha);
        let mut ac: [Plane; 3] = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        malta_diff(&pi0.uhf[1], &pi1.uhf[1], w, h, 1.100_390_325_55 * ha, 1.100_390_325_55 / ha, 71.780_027_516_9, false, &mut ac[1], t);
        malta_diff(&pi0.uhf[0], &pi1.uhf[0], w, h, 173.5 * ha, 173.5 / ha, 5.0, false, &mut ac[0], t);
        malta_diff(&pi0.hf[1], &pi1.hf[1], w, h, 18.723_741_438_7 * sha, 18.723_741_438_7 / sha, 4_498_534.452_32, true, &mut ac[1], t);
        malta_diff(&pi0.hf[0], &pi1.hf[0], w, h, 6_923.994_761_09 * sha, 6_923.994_761_09 / sha, 8_051.158_332_47, true, &mut ac[0], t);
        malta_diff(&pi0.mf[1], &pi1.mf[1], w, h, 37.081_987_039_9, 37.081_987_039_9, 130_262_059.556, true, &mut ac[1], t);
        malta_diff(&pi0.mf[0], &pi1.mf[0], w, h, 8_246.753_213_53, 8_246.753_213_53, 1_009_002.705_82, true, &mut ac[0], t);
        const WMUL: [f64; 9] = [400.0, 1.508_157_031_18, 0.0, 2150.0, 10.619_543_323_9, 16.217_604_315_2, 29.235_379_799_4, 0.844_626_970_982, 0.703_646_627_719];
        let mut dc: [Plane; 3] = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        for c in 0..3 {
            if c < 2 {
                l2_asym(&pi0.hf[c], &pi1.hf[c], (WMUL[c] * ha) as f32, (WMUL[c] / ha) as f32, &mut ac[c]);
            }
            l2_diff(&pi0.mf[c], &pi1.mf[c], WMUL[3 + c] as f32, &mut ac[c]);
            let wd = WMUL[6 + c] as f32;
            for k in 0..n {
                let d = pi0.lf[c][k] - pi1.lf[c][k];
                dc[c][k] = d * d * wd;
            }
        }
        let mk = mask(pi0, pi1, w, h, &mut ac[1], t);
        let xmul = self.p.xmul;
        (0..n)
            .map(|k| {
                let v = mk[k];
                let mv = mask_y(v);
                let dmv = mask_dc_y(v);
                let dac = ac[0][k] * xmul + ac[1][k] + ac[2][k];
                let ddc = dc[0][k] * xmul + dc[1][k] + dc[2][k];
                sqrtf(mv * dac + dmv * ddc)
            })
            .collect()
    }
}

/// ButteraugliScoreFromDiffmap: the largest value
pub fn score(diffmap: &[f32]) -> f32 {
    let mut r = 0f32;
    for &v in diffmap {
        r = maxf(r, v);
    }
    r
}

/// ComputeDistanceP (no border skipped)
pub fn pnorm(diffmap: &[f32], p: f64) -> f64 {
    let n = diffmap.len().max(1) as f64;
    if (p - 3.0).abs() < 1e-6 {
        let mut s = [0f64; 3];
        for &v in diffmap {
            let d1 = v as f64;
            let d3 = d1 * d1 * d1;
            let d6 = d3 * d3;
            s[0] += d3;
            s[1] += d6;
            s[2] += d6 * d6;
        }
        let mut v = 0f64;
        for (i, &e) in [1.0f64, 2.0, 4.0].iter().enumerate() {
            v += crate::spline::pow_f64(s[i] / n, 1.0 / (p * e));
        }
        return v / 3.0;
    }
    let mut s = 0f64;
    for &v in diffmap {
        s += crate::spline::pow_f64(v as f64, p);
    }
    crate::spline::pow_f64(s / n, 1.0 / p)
}
