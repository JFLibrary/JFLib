// spline -- JPEG XL splines: the bundle of a frame's LfGlobal section and their rendering
// (libjxl 0.7 splines.cc: Splines::Decode, QuantizedSpline::Decode / Dequantize,
// InitializeDrawCache, DrawCentripetalCatmullRomSpline, ForEachEquallySpacedPoint,
// SegmentsFromPoints, ComputeSegments, DrawSegment; fast_math-inl.h FastCosf / FastErff)
//                                                                         [jxl-tools: no asm]
//
//   Quantized / decode(jb, num_pixels) -> Quantized / ERR_*   the coded splines (6 contexts)
//   Splines / Splines::new(q, w, h, ytox, ytob) -> Splines / ERR_*   the draw cache: segments
//                                  per row (Gaussian blobs along the curves, colours and widths
//                                  from the 32-coefficient DCTs)
//   sp.add_to(planes, w, h)        add every segment to the three colour planes (the render
//                                  stage after the patches, before upsampling)
//   fmaf(a, b, c) -> f32           a * b + c with one rounding (exact emulation)
//
// libjxl renders splines with Highway on the best target of the machine; this follows its
// AVX2 path (8 float lanes, MulAdd = FMA, the lane order of ContinuousIDCT's sum), which is
// the target libjxl 0.7 uses on the reference machine, so the pixels match it exactly.  The
// scalar parts (the curve, the arc length, the dequantization) are libjxl's baseline code
// without FMA.

use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_HDR, ERR_LIMIT};
use rt::fx::{maxss, minss, sqrt, sqrtd};

use crate::bits::Jb;
use crate::emath::{ceilf, floorf, roundf};
use crate::ent::{je_read, Jr};
use crate::hdr::unpack_signed;
use crate::rnd::Jrp;

pub const CTX_QADJ: u32 = 0;
pub const CTX_START: u32 = 1;
pub const CTX_NUM: u32 = 2;
pub const CTX_NPOINTS: u32 = 3;
pub const CTX_POINTS: u32 = 4;
pub const CTX_DCT: u32 = 5;
pub const NUM_CTX: u32 = 6;

const MAX_POINTS: u64 = 1 << 20;
const MAX_POINTS_PER_PIXEL: u64 = 2;
const DELTA_LIMIT: i64 = 1 << 30;
const POINT_LIMIT: f32 = (1 << 23) as f32;
const RENDER_DIST: f32 = 1.0;
/// kDistanceExp (JXL_HIGH_PRECISION builds: 5): blobs end where they fall below 10^-5
const DIST_EXP: f64 = 5.0;
const SQRT0_5: f32 = core::f32::consts::FRAC_1_SQRT_2;
const SQRT2: f32 = core::f32::consts::SQRT_2;
const PI: f32 = core::f32::consts::PI;
static CHANNEL_WEIGHT: [f32; 4] = [0.0042, 0.075, 0.07, 0.3333];

/// one coded spline: control point delta-deltas, colour (X, Y, B) and sigma DCTs
#[derive(Clone, Default)]
pub struct QSpline {
    pub points: Vec<(i64, i64)>,
    pub color: [[i32; 32]; 3],
    pub sigma: [i32; 32],
}

/// the splines of a frame as coded
#[derive(Clone, Default)]
pub struct Quantized {
    pub start: Vec<(f32, f32)>,
    pub qadj: i32,
    pub splines: Vec<QSpline>,
}

/// a rendered piece of a spline: a Gaussian blob at a point of the curve
#[derive(Clone, Copy, Default)]
struct Seg {
    cx: f32,
    cy: f32,
    color: [f32; 3],
    inv_sigma: f32,
    s4i: f32, // sigma / 4 * intensity
    maxd: f32,
}

/// the draw cache: segments and, per row, the indices of the segments touching it
#[derive(Clone, Default)]
pub struct Splines {
    segs: Vec<Seg>,
    idx: Vec<u32>,
    ystart: Vec<u32>,
}

// ------------------------------------------------------------------ float helpers

/// fused multiply-add: a * b + c rounded once (as vfmadd on the AVX2 target); the f64
/// product is exact, the sum's rounding error decides the f32 ties of double rounding
pub fn fmaf(a: f32, b: f32, c: f32) -> f32 {
    let p = a as f64 * b as f64;
    let cd = c as f64;
    let s = p + cd;
    let r = s as f32;
    if !r.is_finite() || s == r as f64 {
        return r;
    }
    // TwoSum: the exact error of s
    let bb = s - p;
    let err = (p - (s - bb)) + (cd - bb);
    if err == 0.0 {
        return r;
    }
    let rd = r as f64;
    let other = if s > rd { next_up(r) } else { next_down(r) };
    let od = other as f64;
    if s * 2.0 == rd + od {
        // s is halfway between two floats: the true sum lies on the side of err
        let (lo, hi) = if rd < od { (r, other) } else { (other, r) };
        return if err > 0.0 { hi } else { lo };
    }
    r
}

fn next_up(x: f32) -> f32 {
    let b = x.to_bits();
    if x == 0.0 {
        return f32::from_bits(1);
    }
    f32::from_bits(if x > 0.0 { b + 1 } else { b - 1 })
}

fn next_down(x: f32) -> f32 {
    -next_up(-x)
}

/// glibc hypotf: sqrt of the f64 sum of squares, rounded to f32
fn hypotf(x: f32, y: f32) -> f32 {
    let (a, b) = (x as f64, y as f64);
    sqrtd(a * a + b * b) as f32
}

/// natural logarithm in f64 (atanh series after the reduction to [sqrt(1/2), sqrt(2)))
pub fn ln_f64(x: f64) -> f64 {
    if !(x > 0.0) || !x.is_finite() {
        return if x == 0.0 { f64::NEG_INFINITY } else { f64::NAN };
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7FF) as i64 - 1023;
    let mut m = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
    if (bits >> 52) & 0x7FF == 0 {
        // subnormal
        let y = x * (1u64 << 54) as f64;
        return ln_f64(y) - 54.0 * core::f64::consts::LN_2;
    }
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let f = (m - 1.0) / (m + 1.0);
    let s = f * f;
    let mut t = 0.0f64;
    let mut k = 41.0f64;
    while k > 1.0 {
        t = t * s + 1.0 / k;
        k -= 2.0;
    }
    let lnm = 2.0 * f * (1.0 + s * t);
    // ln 2 = hi + lo
    const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
    const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;
    let ef = e as f64;
    ef * LN2_HI + (lnm + ef * LN2_LO)
}

/// sqrt in f64 (sqrtsd)
#[inline]
pub fn sqrt_f64(x: f64) -> f64 {
    sqrtd(x)
}

/// e^x in f64 (range reduction by ln 2, Taylor series)
pub fn exp_f64(x: f64) -> f64 {
    if x < -745.0 {
        return 0.0;
    }
    if x > 709.0 {
        return f64::INFINITY;
    }
    let k = (x * core::f64::consts::LOG2_E + if x < 0.0 { -0.5 } else { 0.5 }) as i64;
    let r = x - k as f64 * core::f64::consts::LN_2;
    let mut t = 1.0f64;
    let mut s = 1.0f64;
    for i in 1..24 {
        t *= r / i as f64;
        s += t;
    }
    if k < -1022 {
        return s * f64::from_bits(((k + 1023 + 200) as u64) << 52) * f64::from_bits(((1023 - 200) as u64) << 52);
    }
    s * f64::from_bits(((k + 1023) as u64) << 52)
}

/// x^y in f64 for x >= 0
pub fn pow_f64(x: f64, y: f64) -> f64 {
    if x <= 0.0 {
        return if y == 0.0 { 1.0 } else { 0.0 };
    }
    exp_f64(y * ln_f64(x))
}

/// FastCosf (AVX2: FMA)
#[inline]
fn fast_cosf(x: f32) -> f32 {
    let pi2 = PI * 2.0;
    let pi2_inv = 0.5 / PI;
    let npi2 = floorf(x * pi2_inv) * pi2;
    let xmodpi2 = x - npi2;
    let t = pi2 - xmodpi2;
    let x_pi = if xmodpi2 < t { xmodpi2 } else { t };
    let above = x_pi >= PI / 2.0;
    let x_pihalf = if above { PI - x_pi } else { x_pi };
    let xs = x_pihalf * 0.25;
    let x2 = xs * xs;
    let x4 = x2 * x2;
    let pre = fmaf(x4, 0.069_604_38, fmaf(x2, -0.840_873_73, 1.681_792_68));
    let s1 = fmaf(pre, pre, -1.414_213_56);
    let s2 = fmaf(s1, s1, -1.0);
    if above { f32::from_bits(s2.to_bits() ^ 0x8000_0000) } else { s2 }
}

/// FastErff (AVX2: FMA)
#[inline]
fn fast_erff(x: f32) -> f32 {
    let le0 = x <= 0.0;
    let ax = f32::from_bits(x.to_bits() & 0x7FFF_FFFF);
    let d1 = fmaf(ax, 7.773_943_69e-02, 2.052_600_15e-04);
    let d2 = fmaf(d1, ax, 2.321_202_16e-01);
    let d3 = fmaf(d2, ax, 2.778_208_01e-01);
    let d4 = fmaf(d3, ax, 1.0);
    let d5 = d4 * d4;
    let inv = 1.0 / d5;
    let r = fmaf(-inv, inv, 1.0);
    if le0 { f32::from_bits(r.to_bits() ^ 0x8000_0000) } else { r }
}

/// ContinuousIDCT: the DCT-III of the 32 coefficients at t (8 lanes, 4 FMA steps each, then
/// the AVX2 lane sum)
fn continuous_idct(dct: &[f32; 32], t: f32) -> f32 {
    let th = t + 0.5;
    let mut lane = [0f32; 8];
    for i in (0..32).step_by(8) {
        for (j, l) in lane.iter_mut().enumerate() {
            let k = i + j;
            let mult = PI / 32.0 * k as f32;
            let c = fast_cosf(mult * th);
            let local = dct[k] * c;
            *l = fmaf(SQRT2, local, *l);
        }
    }
    // SumOfLanes (AVX2): halves, then within 128 bits
    let s: [f32; 4] = [lane[0] + lane[4], lane[1] + lane[5], lane[2] + lane[6], lane[3] + lane[7]];
    let a = [s[0] + s[2], s[1] + s[3]];
    a[1] + a[0]
}

// ------------------------------------------------------------------ decoding

/// Splines::Decode: the bundle at jb (num_pixels = frame width * height)
pub fn decode(jb: &mut Jb, num_pixels: u64) -> Result<Quantized, i32> {
    let jc = je_read(jb, NUM_CTX, false)?;
    let mut jr = Jr::new(&jc, jb, 0)?;
    let n = jr.read(jb, CTX_NUM) as u64 + 1;
    let maxp = MAX_POINTS.min(num_pixels / MAX_POINTS_PER_PIXEL);
    if n > maxp {
        return Err(ERR_LIMIT);
    }
    let mut q = Quantized { start: Vec::with_capacity(n as usize), ..Default::default() };
    let (mut lx, mut ly) = (0i64, 0i64);
    for i in 0..n {
        let mut x = jr.read(jb, CTX_START) as i64;
        let mut y = jr.read(jb, CTX_START) as i64;
        if i != 0 {
            x = unpack_signed(x as u32) as i64 + lx;
            y = unpack_signed(y as u32) as i64 + ly;
        }
        q.start.push((x as f32, y as f32));
        lx = x;
        ly = y;
    }
    q.qadj = unpack_signed(jr.read(jb, CTX_QADJ));
    let mut total = n;
    for _ in 0..n {
        let np = jr.read(jb, CTX_NPOINTS) as u64;
        if np > maxp {
            return Err(ERR_LIMIT);
        }
        total += np;
        if total > maxp {
            return Err(ERR_LIMIT);
        }
        let mut s = QSpline { points: Vec::with_capacity(np as usize), ..Default::default() };
        for _ in 0..np {
            let x = unpack_signed(jr.read(jb, CTX_POINTS)) as i64;
            let y = unpack_signed(jr.read(jb, CTX_POINTS)) as i64;
            if x >= DELTA_LIMIT || -x >= DELTA_LIMIT || y >= DELTA_LIMIT || -y >= DELTA_LIMIT {
                return Err(ERR_HDR);
            }
            s.points.push((x, y));
        }
        for c in 0..3 {
            for i in 0..32 {
                s.color[c][i] = unpack_signed(jr.read(jb, CTX_DCT));
                if s.color[c][i] == i32::MIN {
                    return Err(ERR_HDR);
                }
            }
        }
        for i in 0..32 {
            s.sigma[i] = unpack_signed(jr.read(jb, CTX_DCT));
            if s.sigma[i] == i32::MIN {
                return Err(ERR_HDR);
            }
        }
        q.splines.push(s);
        if jb.err != 0 {
            return Err(jb.err);
        }
    }
    if !jr.fin() {
        return Err(ERR_CODE);
    }
    if q.splines.is_empty() {
        return Err(ERR_HDR);
    }
    Ok(q)
}

fn inv_adjusted_quant(a: i32) -> f32 {
    if a >= 0 { 1.0 / (1.0 + 0.125 * a as f32) } else { 1.0 - 0.125 * a as f32 }
}

fn point_ok(x: f32, y: f32) -> bool {
    !(x >= POINT_LIMIT || x <= -POINT_LIMIT || y >= POINT_LIMIT || y <= -POINT_LIMIT)
}

/// a dequantized spline
struct Spline {
    points: Vec<(f32, f32)>,
    color: [[f32; 32]; 3],
    sigma: [f32; 32],
}

/// QuantizedSpline::Dequantize (with the area estimate of libjxl's later versions)
fn dequantize(q: &QSpline, start: (f32, f32), qadj: i32, ytox: f32, ytob: f32, image_size: u64, total_area: &mut u64) -> Result<Spline, i32> {
    let area_limit = (1024 * image_size + (1u64 << 32)).min(1u64 << 42);
    let px = roundf(start.0);
    let py = roundf(start.1);
    if !point_ok(px, py) {
        return Err(ERR_HDR);
    }
    let (mut cx, mut cy) = (px as i64, py as i64);
    let mut sp = Spline { points: Vec::with_capacity(q.points.len() + 1), color: [[0.0; 32]; 3], sigma: [0.0; 32] };
    sp.points.push((cx as f32, cy as f32));
    let (mut dx, mut dy) = (0i64, 0i64);
    let mut manhattan = 0u64;
    for &(ddx, ddy) in &q.points {
        dx += ddx;
        dy += ddy;
        manhattan += dx.unsigned_abs() + dy.unsigned_abs();
        if manhattan > area_limit {
            return Err(ERR_LIMIT);
        }
        if !point_ok(dx as f32, dy as f32) {
            return Err(ERR_HDR);
        }
        cx += dx;
        cy += dy;
        if !point_ok(cx as f32, cy as f32) {
            return Err(ERR_HDR);
        }
        sp.points.push((cx as f32, cy as f32));
    }
    let iq = inv_adjusted_quant(qadj);
    for c in 0..3 {
        for i in 0..32 {
            let f = if i == 0 { SQRT0_5 } else { 1.0 };
            sp.color[c][i] = q.color[c][i] as f32 * f * CHANNEL_WEIGHT[c] * iq;
        }
    }
    for i in 0..32 {
        sp.color[0][i] += ytox * sp.color[1][i];
        sp.color[2][i] += ytob * sp.color[1][i];
    }
    let mut colsum = [0u64; 3];
    for c in 0..3 {
        for i in 0..32 {
            colsum[c] += ceilf(iq * (q.color[c][i] as f32).abs()) as u64;
        }
    }
    colsum[0] += ceilf(ytox.abs()) as u64 * colsum[1];
    colsum[2] += ceilf(ytob.abs()) as u64 * colsum[1];
    let maxc = colsum[0].max(colsum[1]).max(colsum[2]);
    let logcolor = 1u64.max(ceil_log2_u64(1 + maxc));
    let wl = ceilf(sqrt((area_limit as f32 / logcolor as f32) / manhattan.max(1) as f32));
    let mut width = 0u64;
    for i in 0..32 {
        let f = if i == 0 { SQRT0_5 } else { 1.0 };
        sp.sigma[i] = q.sigma[i] as f32 * f * CHANNEL_WEIGHT[3] * iq;
        let wf = ceilf(iq * (q.sigma[i] as f32).abs());
        let w = minss(wl, maxss(1.0, wf)) as u64;
        width += w * w * logcolor;
    }
    *total_area += width * manhattan;
    if *total_area > area_limit {
        return Err(ERR_LIMIT);
    }
    Ok(sp)
}

fn ceil_log2_u64(x: u64) -> u64 {
    if x <= 1 { 0 } else { 64 - (x - 1).leading_zeros() as u64 }
}

/// DrawCentripetalCatmullRomSpline
fn catmull_rom(pts: &[(f32, f32)], out: &mut Vec<(f32, f32)>) {
    if pts.is_empty() {
        return;
    }
    if pts.len() == 1 {
        out.push(pts[0]);
        return;
    }
    const N: usize = 16;
    let n = pts.len();
    let mut p: Vec<(f32, f32)> = Vec::with_capacity(n + 2);
    p.push((pts[0].0 + (pts[0].0 - pts[1].0), pts[0].1 + (pts[0].1 - pts[1].1)));
    p.extend_from_slice(pts);
    p.push((pts[n - 1].0 + (pts[n - 1].0 - pts[n - 2].0), pts[n - 1].1 + (pts[n - 1].1 - pts[n - 2].1)));
    let add = |a: (f32, f32), f: f32, b: (f32, f32)| (a.0 + f * b.0, a.1 + f * b.1);
    let sub = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0, a.1 - b.1);
    for start in 0..p.len() - 3 {
        let q = &p[start..start + 4];
        out.push(q[1]);
        let mut d = [0f32; 3];
        let mut t = [0f32; 4];
        for k in 0..3 {
            d[k] = sqrt(hypotf(q[k + 1].0 - q[k].0, q[k + 1].1 - q[k].1));
            t[k + 1] = t[k] + d[k];
        }
        for i in 1..N {
            let tt = d[0] + (i as f32 / N as f32) * d[1];
            let mut a = [(0f32, 0f32); 3];
            for k in 0..3 {
                a[k] = add(q[k], (tt - t[k]) / d[k], sub(q[k + 1], q[k]));
            }
            let mut b = [(0f32, 0f32); 2];
            for k in 0..2 {
                b[k] = add(a[k], (tt - t[k]) / (d[k] + d[k + 1]), sub(a[k + 1], a[k]));
            }
            out.push(add(b[0], (tt - t[1]) / d[1], sub(b[1], b[0])));
        }
    }
    out.push(p[p.len() - 2]);
}

/// ForEachEquallySpacedPoint: points one unit apart along the polyline (the last with the
/// remaining length as its multiplier)
fn equally_spaced(pts: &[(f32, f32)], out: &mut Vec<((f32, f32), f32)>) {
    let mut cur = pts[0];
    out.push((cur, RENDER_DIST));
    let mut next = 0usize;
    while next < pts.len() {
        let mut prev = cur;
        let mut from_prev = 0f32;
        loop {
            if next == pts.len() {
                out.push((prev, from_prev));
                return;
            }
            let dx = pts[next].0 - prev.0;
            let dy = pts[next].1 - prev.1;
            let to_next = sqrt(dx * dx + dy * dy);
            if from_prev + to_next >= RENDER_DIST {
                let f = (RENDER_DIST - from_prev) / to_next;
                cur = (prev.0 + f * dx, prev.1 + f * dy);
                out.push((cur, RENDER_DIST));
                break;
            }
            from_prev += to_next;
            prev = pts[next];
            next += 1;
        }
    }
}

/// ComputeSegments: the blob of one point (skipped for a degenerate sigma)
fn compute_segment(center: (f32, f32), intensity: f32, color: [f32; 3], sigma: f32, segs: &mut Vec<Seg>, by_y: &mut Vec<(u64, u32)>) {
    if !(sigma.is_finite() && sigma != 0.0 && (1.0 / sigma).is_finite() && intensity.is_finite()) {
        return;
    }
    let mut maxc = 0.01f32;
    for c in color {
        maxc = maxc.max((c * intensity).abs());
    }
    const LN_0_1: f64 = -2.302_585_092_994_045_684;
    let lnc = ln_f64(maxc as f64) as f32 as f64;
    let md = sqrtd((-2.0 * sigma * sigma) as f64 * (LN_0_1 * DIST_EXP - lnc)) as f32;
    let seg = Seg {
        cx: center.0,
        cy: center.1,
        color,
        inv_sigma: 1.0 / sigma,
        s4i: 0.25 * sigma * intensity,
        maxd: md,
    };
    let y0 = (center.1 - md + 0.5) as i64;
    let y1 = (center.1 + md + 1.5) as i64;
    let i = segs.len() as u32;
    for y in y0.max(0)..y1 {
        by_y.push((y as u64, i));
    }
    segs.push(seg);
}

impl Splines {
    /// InitializeDrawCache for an image of w x h (the frame's upsampled size); ytox / ytob:
    /// the colour correlation's base ratios
    pub fn new(q: &Quantized, w: u32, h: u32, ytox: f32, ytob: f32) -> Result<Splines, i32> {
        let mut segs: Vec<Seg> = Vec::new();
        let mut by_y: Vec<(u64, u32)> = Vec::new();
        let mut total = 0u64;
        let mut sps = Vec::with_capacity(q.splines.len());
        for (i, s) in q.splines.iter().enumerate() {
            let sp = dequantize(s, q.start[i], q.qadj, ytox, ytob, w as u64 * h as u64, &mut total)?;
            if sp.points.windows(2).any(|p| p[0] == p[1]) {
                return Err(ERR_HDR); // identical successive control points
            }
            sps.push(sp);
        }
        let mut inter: Vec<(f32, f32)> = Vec::new();
        let mut draw: Vec<((f32, f32), f32)> = Vec::new();
        for sp in &sps {
            inter.clear();
            draw.clear();
            catmull_rom(&sp.points, &mut inter);
            equally_spaced(&inter, &mut draw);
            let arc = ((draw.len() - 2) as f32 * RENDER_DIST + draw[draw.len() - 1].1) as f64;
            if arc <= 0.0 {
                continue;
            }
            let inv_arc = 1.0f32 / arc as f32;
            for (k, &(pt, mult)) in draw.iter().enumerate() {
                let prog = minss(1.0, (k as f32 * RENDER_DIST) * inv_arc);
                let t = 31.0 * prog;
                let color = [continuous_idct(&sp.color[0], t), continuous_idct(&sp.color[1], t), continuous_idct(&sp.color[2], t)];
                let sigma = continuous_idct(&sp.sigma, t);
                compute_segment(pt, mult, color, sigma, &mut segs, &mut by_y);
            }
        }
        by_y.sort_unstable();
        let mut ystart = vec![0u32; h as usize + 1];
        let mut idx = Vec::with_capacity(by_y.len());
        for &(y, i) in &by_y {
            idx.push(i);
            if y < h as u64 {
                ystart[y as usize + 1] += 1;
            }
        }
        for y in 0..h as usize {
            ystart[y + 1] += ystart[y];
        }
        Ok(Splines { segs, idx, ystart })
    }

    /// add the segments to the first three planes (their size: the frame before upsampling)
    pub fn add_to(&self, p: &mut [Jrp]) {
        let (w, h, s) = (p[0].w, p[0].h, p[0].s as usize);
        if p.len() < 3 || p[1].s as usize != s || p[2].s as usize != s {
            return;
        }
        let (a, rest) = p.split_at_mut(1);
        let (b, c) = rest.split_at_mut(1);
        let rows: [&mut [f32]; 3] = [&mut a[0].d, &mut b[0].d, &mut c[0].d];
        let h = (h as usize).min(self.ystart.len() - 1);
        for y in 0..h {
            let (a, b) = (self.ystart[y] as usize, self.ystart[y + 1] as usize);
            for &si in &self.idx[a..b] {
                let seg = &self.segs[si as usize];
                let x0 = (seg.cx - seg.maxd + 0.5) as i64;
                let x1 = ((seg.cx + seg.maxd + 1.5) as i64).min(w as i64);
                let dy = y as f32 - seg.cy;
                let dy2 = dy * dy;
                for x in x0.max(0)..x1 {
                    let dx = x as f32 - seg.cx;
                    let sqd = fmaf(dx, dx, dy2);
                    let dist = sqrt(sqd);
                    let a1 = fmaf(dist, 0.5, 0.353_553_391) * seg.inv_sigma;
                    let a2 = fmaf(dist, 0.5, -0.353_553_391) * seg.inv_sigma;
                    let f = fast_erff(a1) - fast_erff(a2);
                    let li = seg.s4i * (f * f);
                    let o = y * s + x as usize;
                    for c in 0..3 {
                        rows[c][o] = fmaf(seg.color[c], li, rows[c][o]);
                    }
                }
            }
        }
    }

    /// whether there is anything to draw
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// the number of segments (blobs) of all splines
    pub fn segments(&self) -> usize {
        self.segs.len()
    }
}
