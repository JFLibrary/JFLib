// edots -- dots for the patch dictionary: small Gaussian-like spots (stars, specks) of the XYB
// image go into the reference frame as the image minus a strongly smoothed copy (libjxl 0.11.1
// enc_detect_dots.cc DetectGaussianEllipses, enc_dot_dictionary.cc FindDotDictionary; libjxl
// looks for them at effort 7+ and distance >= 3 when the image has no text-like patches)
//                                                                       [jxl-tools: no asm]
//
//   find(p, s, ph, w, h, threads) -> Vec<Found>   the dots of the XYB planes p (row stride s,
//                                 ph rows, searched in the w x h image), one position each
//
// Energy = 10 (Y - smooth Y)^2 with smooth = two passes of a wide 5-tap Gaussian and the
// Y plane first lightly smoothed; connected components above 0.04 (grown down to 0.02, below
// 5 x 5 pixels, contrast score >= 12 against their surroundings); a Gaussian ellipse fitted to
// each must explain it (small L2 / weighted loss, enough intensity, centred on the peak, no
// pixel overshooting).

use alloc::vec;
use alloc::vec::Vec;

use crate::emod::par_map;
use crate::epatch::Found;
use crate::spline::{exp_f64, fmaf, sqrt_f64};

type Plane = Vec<f32>;

const T_HIGH: f64 = 0.04;
const T_LOW: f64 = 0.02;
const MAX_WIN: usize = 5;
const MAX_L2_LOSS: f64 = 0.005;
const MAX_CUSTOM_LOSS: f64 = 300.0;
const MIN_INTENSITY: f64 = 0.12;
const MAX_DIST_MEAN_MODE: f64 = 1.0;
const MAX_NEG_PIXELS: usize = 0;
const MIN_SCORE: f32 = 12.0;
const MAX_CC: usize = 100;
const MAX_CC_SIZE: usize = 1000;
const EXTRA_RECT: i64 = 4;
const WIN: i64 = 5;
const PI: f64 = core::f64::consts::PI;

/// the distance from which libjxl looks for dots (kMinButteraugliForDots)
pub const MIN_DISTANCE: f32 = 3.0;

// ------------------------------------------------------------------ f64 math (glibc-like)

fn floor_f64(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn ceil_f64(x: f64) -> f64 {
    -floor_f64(-x)
}

/// (sin x, cos x)
fn sin_cos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    // x = k pi/2 + r, |r| <= pi/4 (pi/2 in two parts)
    const PIO2_HI: f64 = 1.570_796_326_734_125_6;
    const PIO2_LO: f64 = 6.077_100_506_506_192e-11;
    let k = floor_f64(x * core::f64::consts::FRAC_2_PI + 0.5);
    let r = (x - k * PIO2_HI) - k * PIO2_LO;
    let r2 = r * r;
    let (mut s, mut c) = (0f64, 0f64);
    let (mut ts, mut tc) = (r, 1.0f64);
    for i in 0..12 {
        s += ts;
        c += tc;
        let n = (2 * i + 2) as f64;
        ts *= -r2 / (n * (n + 1.0));
        tc *= -r2 / ((n - 1.0) * n);
    }
    match (k as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// atan of |t| <= 1 (two argument halvings, then the series)
fn atan_small(t: f64) -> f64 {
    let mut u = t;
    for _ in 0..2 {
        u /= 1.0 + sqrt_f64(1.0 + u * u);
    }
    let u2 = u * u;
    let mut s = 0f64;
    let mut p = u;
    for i in 0..20 {
        s += p / (2 * i + 1) as f64;
        p *= -u2;
    }
    4.0 * s
}

fn atan2_f64(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 && y == 0.0 {
        return if x.is_sign_negative() { if y.is_sign_negative() { -PI } else { PI } } else { y };
    }
    let a = if y.abs() <= x.abs() {
        atan_small(y.abs() / x.abs())
    } else {
        PI / 2.0 - atan_small(x.abs() / y.abs())
    };
    let a = if x < 0.0 { PI - a } else { a };
    if y < 0.0 { -a } else { a }
}

// ------------------------------------------------------------------ energy

/// Separable5 (libjxl, the AVX2 build: multiply-adds fused; mirrored borders; the last
/// w % 8 columns of a row as libjxl's scalar tail); rows on the pool
fn separable5(inp: &[f32], s: usize, w: usize, h: usize, wt: [f32; 3], threads: bool) -> Plane {
    fn mirror(i: i64, n: usize) -> usize {
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
    }
    let tail0 = if w >= 16 { w - w % 8 } else { w };
    // the horizontal pass of every row
    let hz: Vec<Vec<f32>> = par_map(h, threads, |y| {
        let r = &inp[y * s..y * s + w];
        (0..tail0)
            .map(|x| {
                let at = |d: i64| r[mirror(x as i64 + d, w)];
                let m0 = at(0) * wt[0];
                let m1 = fmaf(at(-1) + at(1), wt[1], m0);
                fmaf(at(-2) + at(2), wt[2], m1)
            })
            .collect()
    });
    let rows: Vec<Vec<f32>> = par_map(h, threads, |y| {
        let ys: [usize; 5] = core::array::from_fn(|i| mirror(y as i64 + i as i64 - 2, h));
        let mut o = vec![0f32; s];
        for (x, ov) in o.iter_mut().enumerate().take(w) {
            *ov = if x >= tail0 {
                let mut m = 0f32;
                for dy in -2i64..=2 {
                    let wy = wt[dy.unsigned_abs() as usize];
                    let r = &inp[mirror(y as i64 + dy, h) * s..];
                    for dx in -2i64..=2 {
                        let wx = wt[dx.unsigned_abs() as usize];
                        m += r[mirror(x as i64 + dx, w)] * wx * wy;
                    }
                }
                m
            } else {
                let c0 = hz[ys[2]][x] * wt[0];
                let c1 = fmaf(hz[ys[1]][x] + hz[ys[3]][x], wt[1], c0);
                fmaf(hz[ys[0]][x] + hz[ys[4]][x], wt[2], c1)
            };
        }
        o
    });
    rows.concat()
}

/// ComputeEnergyImage: (energy, smooth planes)
fn energy(p: [&[f32]; 3], s: usize, w: usize, h: usize, threads: bool) -> (Plane, [Plane; 3]) {
    const W1: [f32; 3] = [0.558_311, 0.210_395, 0.010_449]; // keeps dots
    const W3: [f32; 3] = [0.222_338, 0.210_431, 0.178_4]; // removes them (twice)
    let mut sm: [Plane; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut f1 = Vec::new();
    for c in 0..3 {
        let t = separable5(p[c], s, w, h, W3, threads);
        sm[c] = separable5(&t, s, w, h, W3, threads);
        if c == 1 {
            f1 = separable5(p[c], s, w, h, W1, threads);
        }
    }
    // SumOfSquareDifferences: weights 0, 10, 0
    let e = f1.iter().zip(&sm[1]).map(|(&a, &b)| (a - b) * (a - b) * 10.0).collect();
    (e, sm)
}

// ------------------------------------------------------------------ connected components

#[derive(Clone, Copy)]
struct Rect {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
}

struct Cc {
    b: Rect,
    score: f32,
    mode: (i64, i64),
}

/// ExtractComponent (libjxl's neighbour list: (1, 0) twice, no (-1, 0)); false past 1000 pixels
fn extract(img: &mut [f32], s: usize, w: usize, h: usize, pixels: &mut Vec<(i64, i64)>, seed: (i64, i64)) -> bool {
    const NB: [(i64, i64); 8] = [(1, -1), (1, 0), (1, 1), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, 0)];
    let mut q = vec![seed];
    while let Some(cur) = q.pop() {
        pixels.push(cur);
        if pixels.len() > MAX_CC_SIZE {
            return false;
        }
        for d in NB {
            let (cx, cy) = (cur.0 + d.0, cur.1 + d.1);
            if cx >= 0 && (cx as usize) < w && cy >= 0 && (cy as usize) < h {
                let v = &mut img[cy as usize * s + cx as usize];
                if *v as f64 > T_LOW {
                    *v = 0.0;
                    q.push((cx, cy));
                }
            }
        }
    }
    true
}

/// CompStats: the contrast score and the peak of the energy around the component
fn comp_stats(e: &[f32], s: usize, w: usize, h: usize, b: Rect) -> (f32, (i64, i64)) {
    let (mut max_e, mut mean_e, mut mean_bg, mut var_bg) = (0f32, 0f32, 0f32, 0f32);
    let (mut n_in, mut n_out) = (0i32, 0i32);
    let mut mode = (0i64, 0i64);
    for sy in -EXTRA_RECT..b.h as i64 + EXTRA_RECT {
        let y = sy + b.y0 as i64;
        if y < 0 || y as usize >= h {
            continue;
        }
        for sx in -EXTRA_RECT..b.w as i64 + EXTRA_RECT {
            let x = sx + b.x0 as i64;
            if x < 0 || x as usize >= w {
                continue;
            }
            let v = e[y as usize * s + x as usize];
            if v > max_e {
                max_e = v;
                mode = (x, y);
            }
            let inside = x as usize >= b.x0 && (x as usize) < b.x0 + b.w && y as usize >= b.y0 && (y as usize) < b.y0 + b.h;
            if inside {
                mean_e += v;
                n_in += 1;
            } else {
                mean_bg += v;
                var_bg += v * v;
                n_out += 1;
            }
        }
    }
    mean_e /= n_in as f32;
    mean_bg /= n_out as f32;
    var_bg = var_bg / n_out as f32 - mean_bg * mean_bg;
    ((mean_e - mean_bg) / rt::fx::sqrt(var_bg), mode)
}

/// FindCC
fn find_cc(e: &[f32], s: usize, w: usize, h: usize) -> Vec<Cc> {
    let mut img = e.to_vec();
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if img[y * s + x] as f64 <= T_HIGH {
                continue;
            }
            img[y * s + x] = 0.0;
            let mut px = Vec::new();
            if !extract(&mut img, s, w, h, &mut px, (x as i64, y as i64)) {
                continue;
            }
            let (mut lx, mut hx, mut ly, mut hy) = (px[0].0, px[0].0, px[0].1, px[0].1);
            for &(a, b) in &px {
                lx = lx.min(a);
                hx = hx.max(a);
                ly = ly.min(b);
                hy = hy.max(b);
            }
            let b = Rect { x0: lx as usize, y0: ly as usize, w: (hx - lx + 1) as usize, h: (hy - ly + 1) as usize };
            if b.w < MAX_WIN && b.h < MAX_WIN {
                let (score, mode) = comp_stats(e, s, w, h, b);
                if score < MIN_SCORE {
                    continue;
                }
                out.push(Cc { b, score, mode });
            }
        }
    }
    out
}

// ------------------------------------------------------------------ the Gaussian fit

struct Ellipse {
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
    angle: f64,
    intensity: [f64; 3],
    l2_loss: f64,
    custom_loss: f64,
    bg: [f64; 3],
    neg_pixels: usize,
}

#[inline]
fn model(dx: f64, dy: f64, ct: f64, st: f64, sx: f64, sy: f64, intensity: f64) -> f64 {
    let rx = ct * dx + st * dy;
    let ry = -st * dx + ct * dy;
    let md = (rx * rx / sx) + (ry * ry / sy);
    intensity * exp_f64(-0.5 * md)
}

/// ConvertToDiagonal of the symmetric 2 x 2 matrix a: (eigenvalues, U)
fn diagonal(a: [[f64; 2]; 2]) -> ([f64; 2], [[f64; 2]; 2]) {
    let b = -(a[0][0] + a[1][1]);
    let c = a[0][0] * a[1][1] - a[0][1] * a[0][1];
    let d = b * b - 4.0 * c;
    if a[0][1].abs() < 1e-10 || d < 0.0 {
        return ([a[0][0], a[1][1]], [[1.0, 0.0], [0.0, 1.0]]);
    }
    let sqd = sqrt_f64(d);
    let l1 = (-b - sqd) * 0.5;
    let l2 = (-b + sqd) * 0.5;
    let mut v1 = [a[0][0] - l1, a[1][0]];
    let n = 1.0 / sqrt_f64(v1[0] * v1[0] + v1[1] * v1[1]);
    v1[0] *= n;
    v1[1] *= n;
    ([l1, l2], [[v1[1], -v1[0]], [v1[0], v1[1]]])
}

struct Img3<'a> {
    p: [&'a [f32]; 3],
    s: usize,
    w: usize,
    h: usize,
}

impl Img3<'_> {
    #[inline]
    fn at(&self, c: usize, x: i64, y: i64) -> f32 {
        self.p[c][y as usize * self.s + x as usize]
    }
}

/// ComputeDotLosses (fills the losses of el)
fn losses(el: &mut Ellipse, cc: &Cc, img: &Img3, bg: &Img3) {
    const BOUNDS: i64 = 2;
    const ZERO_EPS: f64 = 0.1;
    let (st, ct) = sin_cos(el.angle);
    let mut n = 0i32;
    el.l2_loss = 0.0;
    el.neg_pixels = 0;
    let mut neg_value = [0f64; 3];
    let dmm = (cc.mode.0 as f64 - el.x) * (cc.mode.0 as f64 - el.x) + (cc.mode.1 as f64 - el.y) * (cc.mode.1 as f64 - el.y);
    el.custom_loss = 0.0;
    let _ = bg;
    for c in 0..3 {
        for sy in -BOUNDS..cc.b.h as i64 + BOUNDS {
            let y = sy + cc.b.y0 as i64;
            if y < 0 || y as usize >= img.h {
                continue;
            }
            for sx in -BOUNDS..cc.b.w as i64 + BOUNDS {
                let x = sx + cc.b.x0 as i64;
                if x < 0 || x as usize >= img.w {
                    continue;
                }
                let target = img.at(c, x, y) as f64;
                let dot = model(x as f64 - el.x, y as f64 - el.y, ct, st, el.sx, el.sy, el.intensity[c]);
                if dot > target + ZERO_EPS {
                    el.neg_pixels += 1;
                    neg_value[c] += dot - target;
                }
                let pred = el.bg[c] + dot;
                let diff = target - pred;
                let l2 = 1.0 * diff * diff;
                el.l2_loss += l2;
                let wgt = model(x as f64 - cc.mode.0 as f64, y as f64 - cc.mode.1 as f64, 1.0, 0.0, 1.0 + el.sx, 1.0 + el.sy, 1.0);
                el.custom_loss += wgt * l2;
                n += 1;
            }
        }
    }
    el.l2_loss /= n as f64;
    el.custom_loss /= n as f64;
    el.custom_loss += 20.0 * dmm + neg_value[1];
}

/// FitGaussianFast + FitGaussian's normalization
fn fit(cc: &Cc, img: &Img3, bg: &Img3) -> Ellipse {
    const EPS: f64 = 1e-6;
    const RB: i64 = WIN >> 1;
    let (mx, my) = cc.mode;
    let mut sum = 0f64;
    let mut n = 0i32;
    let mut m1 = [0f64; 3];
    let mut m2 = [0f64; 3];
    let mut bgc = [0f64; 3];
    let color: [f64; 3] = core::array::from_fn(|c| (img.at(c, mx, my) - bg.at(c, mx, my)) as f64);
    let sign = if color[1] > 0.0 { 1.0 } else { -1.0 };
    for sy in -RB..=RB {
        let y = sy + my;
        if y < 0 || y as usize >= img.h {
            continue;
        }
        for sx in -RB..=RB {
            let x = sx + mx;
            if x < 0 || x as usize >= img.w {
                continue;
            }
            let d = (img.at(1, x, y) - bg.at(1, x, y)) as f64;
            let w = EPS.max(sign * d);
            sum += w;
            m1[0] += w * x as f64;
            m1[1] += w * y as f64;
            m2[0] += w * x as f64 * x as f64;
            m2[1] += w * x as f64 * y as f64;
            m2[2] += w * y as f64 * y as f64;
            for (c, v) in bgc.iter_mut().enumerate() {
                *v += bg.at(c, x, y) as f64;
            }
            n += 1;
        }
    }
    for i in 0..3 {
        m1[i] /= sum;
        m2[i] /= sum;
        bgc[i] /= n as f64;
    }
    let mut el = Ellipse {
        x: m1[0],
        y: m1[1],
        sx: 0.0,
        sy: 0.0,
        angle: 0.0,
        intensity: core::array::from_fn(|c| 1.1 * color[c]),
        l2_loss: 0.0,
        custom_loss: 0.0,
        bg: bgc,
        neg_pixels: 0,
    };
    let sxy = m2[1] - m1[0] * m1[1];
    let (d, u) = diagonal([[m2[0] - m1[0] * m1[0], sxy], [sxy, m2[2] - m1[1] * m1[1]]]);
    let (p1, p2) = if d[0] < d[1] { (1, 0) } else { (0, 1) };
    el.sx = d[p1];
    el.sy = d[p2];
    el.angle = atan2_f64(u[1][p1], u[1][p2]);
    // the intensity by least squares (fixed background)
    let (st, ct) = sin_cos(el.angle);
    for c in 0..3 {
        let (mut gg, mut gd) = (0f64, 0f64);
        for y in my - RB..=my + RB {
            if y < 0 || y as usize >= img.h {
                continue;
            }
            for x in mx - RB..=mx + RB {
                if x < 0 || x as usize >= img.w {
                    continue;
                }
                let target = (img.at(c, x, y) - bg.at(c, x, y)) as f64;
                let g = model(x as f64 - el.x, y as f64 - el.y, ct, st, el.sx, el.sy, 1.0);
                gg += g * g;
                gd += g * target;
            }
        }
        el.intensity[c] = gd / (gg + 1e-6);
    }
    losses(&mut el, cc, img, bg);
    if el.sx < el.sy {
        core::mem::swap(&mut el.sx, &mut el.sy);
        el.angle += PI / 2.0;
    }
    el.angle -= PI * floor_f64(el.angle / PI);
    if (el.angle - PI).abs() < 1e-6 || el.angle.abs() < 1e-6 {
        el.angle = 0.0;
    }
    el
}

/// DetectGaussianEllipses with FindDotDictionary's parameters (see the module header)
pub fn find(p: [&[f32]; 3], s: usize, ph: usize, w: usize, h: usize, threads: bool) -> Vec<Found> {
    if w < 16 || h < 16 || s < w || ph < h {
        return Vec::new();
    }
    let (e, sm) = energy(p, s, s, ph, threads);
    let mut ccs = find_cc(&e, s, w, h);
    let keep = MAX_CC.min(ccs.len());
    if ccs.len() > keep {
        let mut perm: Vec<u32> = (0..ccs.len() as u32).collect();
        crate::epatch::cxx_sort(&mut perm, |a, b| ccs[*a as usize].score > ccs[*b as usize].score);
        let mut slots: Vec<Option<Cc>> = ccs.into_iter().map(Some).collect();
        ccs = perm[..keep].iter().filter_map(|&i| slots[i as usize].take()).collect();
    }
    let img = Img3 { p, s, w, h };
    let bg = Img3 { p: [&sm[0], &sm[1], &sm[2]], s, w, h };
    let mut out = Vec::new();
    for cc in &ccs {
        let el = fit(cc, &img, &bg);
        if el.x < 0.0 || ceil_f64(el.x) >= w as f64 || el.y < 0.0 || ceil_f64(el.y) >= h as f64 {
            continue;
        }
        if el.neg_pixels > MAX_NEG_PIXELS {
            continue;
        }
        let it = 0.21 * el.intensity[0] + 0.72 * el.intensity[1] + 0.07 * el.intensity[2];
        let dmm = (el.x - cc.mode.0 as f64) * (el.x - cc.mode.0 as f64) + (el.y - cc.mode.1 as f64) * (el.y - cc.mode.1 as f64);
        if el.l2_loss < MAX_L2_LOSS
            && el.custom_loss < MAX_CUSTOM_LOSS
            && it * it > MIN_INTENSITY * MIN_INTENSITY
            && dmm < MAX_DIST_MEAN_MODE * MAX_DIST_MEAN_MODE
        {
            let b = cc.b;
            let f: [Vec<f32>; 3] = core::array::from_fn(|c| {
                let mut v = Vec::with_capacity(b.w * b.h);
                for y in b.y0..b.y0 + b.h {
                    for x in b.x0..b.x0 + b.w {
                        v.push(p[c][y * s + x] - sm[c][y * s + x]);
                    }
                }
                v
            });
            out.push(Found::single(b.w, b.h, f, (b.x0 as u32, b.y0 as u32)));
        }
    }
    out
}
