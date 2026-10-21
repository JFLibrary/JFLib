// distort -- effect group "distort" (Verzerren): Bulge, Crystalize, Dents, Frosted Glass,
// Morphology, Pixelate, Polar Inversion, Tile Reflection, Twist           [src/fx/fx_distort.s]
//
//   FXTAB            the group table
//
// The warp effects are small mapping functions for the warp framework (warp.rs); Pixelate and
// Morphology have their own loops.  Every pixel depends only on (source, parameters, aux).
//   bulge          a = amount / 100, R = default radius: |p| < R: p (1 - a (1 - |p| / R)^2)
//   twist          R = size * default radius, k = sign(amount) amount^2 / 100: p rotated by
//                  k (1 - |p| / R)^3
//   polar_inversion  p lerp(1, R^2 / |p|^2, amount)
//   tile_reflection  (s, t) = p rotated by -angle, s += k tan(s pi / size) (t likewise), back
//   dents          p + d (-sin a, cos a), a = theta * 4 octaves of perlin noise
//   frosted_glass  samples moved by r ((1 - s) w + s n): w random point of the unit disc, n two
//                  perlin fields
//   crystalize     Voronoi cells of a jittered grid: the colour of the source pixel under the
//                  nearest site
//   pixelate       blocks filled with their premultiplied average (exact integers)
//   morphology     max (dilate) / min (erode) / open / close of the premultiplied channels over
//                  a circle / square / diamond (exact window maxima by doubling)

use alloc::vec;
use alloc::vec::Vec;

use gfx::straight::{premul_row, unpremul_row};
use rt::fx::{cvttd, cvttd64};

use simd::{Kernel, Simd, Sse2};

use crate::blur::acc_px;
use crate::perlin::perlin;
use crate::warp::*;
use crate::*;

const DEG2RAD: f64 = 0.017453292519943295769;

// ---------------------------------------------------------------- Bulge / Twist / Polar / Tile

fn bulge_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    wr.p[0] = p.int(0, -200, 100) as f64 * 0.01;
    wr.center(p.f(1), p.f(2));
    wr.q = p.int(3, 1, 5);
    wr.p[1] = wr.rdef;
    wr.rin = wr.rdef;
    wr.p[2] = wr.rdef * wr.rdef;
    wr.run(dst, |wr, _, pt| {
        let (x, y) = (pt[0], pt[1]);
        let r2 = x * x + y * y;
        if r2 >= wr.p[2] {
            return true;
        }
        let mut t = 1.0 - rt::fx::sqrtd(r2) / wr.p[1];
        t *= t;
        let f = 1.0 - t * wr.p[0];
        pt[0] = x * f;
        pt[1] = y * f;
        false
    })
}

fn twist_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    let am = p.int(0, -200, 200);
    let sq = am.wrapping_mul(am);
    wr.p[2] = (if am < 0 { -sq } else { sq }) as f64 * 0.01;
    let r = p.dbl(1, 0.01, 2.0) * wr.rdef;
    wr.p[0] = r;
    wr.rin = r;
    wr.p[1] = r * r;
    wr.center(p.f(2), p.f(3));
    wr.q = p.int(4, 1, 5);
    wr.run(dst, |wr, _, pt| {
        let (x, y) = (pt[0], pt[1]);
        let r2 = x * x + y * y;
        if r2 >= wr.p[1] {
            return true;
        }
        let t = 1.0 - rt::fx::sqrtd(r2) / wr.p[0];
        let (s, c) = sincos(t * t * t * wr.p[2]);
        pt[0] = x * c - y * s;
        pt[1] = x * s + y * c;
        false
    })
}

fn polar_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    wr.p[0] = p.dbl(0, -4.0, 4.0);
    wr.p[1] = wr.rdef * wr.rdef;
    wr.center(p.f(1), p.f(2));
    wr.edge = p.int(3, 0, FXW_NEDGE - 1);
    wr.q = p.int(4, 1, 5);
    wr.col1 = p.0[5] as u32;
    wr.col2 = p.0[6] as u32;
    wr.run(dst, |wr, _, pt| {
        let (x, y) = (pt[0], pt[1]);
        let r2 = rt::fx::maxsd(x * x + y * y, 1.0e-12);
        let f = (wr.p[1] / r2 - 1.0) * wr.p[0] + 1.0;
        pt[0] = x * f;
        pt[1] = y * f;
        false
    })
}

fn tile_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    let (s, c) = sincos(p.dbl(0, -36000.0, 36000.0) * DEG2RAD);
    wr.p[0] = c;
    wr.p[1] = s;
    wr.p[2] = 3.14159265358979323846 / p.int(1, 2, 200) as f64;
    wr.p[3] = p.int(2, -100, 100) as f64;
    wr.q = p.int(3, 1, 5);
    wr.edge = p.int(4, 0, FXW_NEDGE - 1);
    wr.col1 = p.0[5] as u32;
    wr.col2 = p.0[6] as u32;
    wr.run(dst, |wr, _, pt| {
        let (u, v) = (pt[0], pt[1]);
        let (c, sn) = (wr.p[0], wr.p[1]);
        let s0 = u * c + v * sn;
        let t0 = v * c - u * sn;
        let (ss, cs) = sincos(s0 * wr.p[2]);
        let s1 = ss / cs * wr.p[3] + s0;
        let (st, ct) = sincos(t0 * wr.p[2]);
        let t1 = st / ct * wr.p[3] + t0;
        pt[0] = s1 * c - t1 * sn;
        pt[1] = s1 * sn + t1 * c;
        false
    })
}

// ---------------------------------------------------------------- Dents / Frosted Glass

fn dents_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    let f = 400.0 / (p.dbl(0, 1.0, 200.0) * wr.rdef);
    wr.p[0] = f;
    wr.p[1] = p.dbl(1, 0.0, 200.0) * 0.01 / f;
    wr.p[2] = p.dbl(3, 0.0, 100.0) * 0.62831853071795864769;
    wr.p[3] = p.dbl(2, 0.0, 100.0) * 0.01;
    wr.q = p.int(4, 1, 5);
    wr.seed = p.0[5] as u32;
    wr.run(dst, |wr, _, pt| {
        let mut nx = pt[0] * wr.p[0];
        let mut ny = pt[1] * wr.p[0];
        let mut sum = 0.0f64;
        let mut amp = 1.0f64;
        let mut oct = 0u32;
        loop {
            sum = perlin(nx, ny, wr.seed, oct) * amp + sum;
            oct += 1;
            if oct >= 4 {
                break;
            }
            amp *= wr.p[3];
            if !(amp >= 0.001) {
                break;
            }
            nx += nx;
            ny += ny;
        }
        let (s, c) = sincos(sum * wr.p[2]);
        pt[0] -= s * wr.p[1];
        pt[1] = c * wr.p[1] + pt[1];
        false
    })
}

fn frost_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    let r = p.dbl(0, 0.0, 200.0);
    wr.p[0] = r;
    wr.p[2] = 1.0 / (r * 0.5 + 2.0);
    wr.p[1] = p.dbl(1, 0.0, 100.0) * 0.01;
    wr.q = p.int(2, 1, 5);
    wr.seed = p.0[3] as u32;
    wr.run(dst, |wr, cur, pt| {
        let (mut wx, mut wy) = (0.0f64, 0.0f64);
        let s = wr.p[1];
        if !(s >= 1.0) {
            let h = hash32(cur.x as u32, cur.y as u32, wr.seed, (cur.s as u32).wrapping_add(cur.s as u32));
            let rad = rt::fx::sqrtd((h >> 8) as f64 * 5.9604644775390625e-08);
            let h = hash32(cur.x as u32, cur.y as u32, wr.seed, (cur.s as u32).wrapping_mul(2).wrapping_add(1));
            let (sn, cs) = sincos((h >> 8) as f64 * 3.7450702829238412e-07);
            let k = 1.0 - s;
            wx = cs * rad * k;
            wy = sn * rad * k;
        }
        if s > 0.0 {
            let ax = (pt[0] + wr.cx) * wr.p[2];
            let ay = (pt[1] + wr.cy) * wr.p[2];
            let nx = rt::fx::minsd(rt::fx::maxsd(perlin(ax, ay, wr.seed, 7) * 1.4, -1.0), 1.0);
            let ax = (pt[0] + wr.cx) * wr.p[2];
            let ay = (pt[1] + wr.cy) * wr.p[2];
            let ny = rt::fx::minsd(rt::fx::maxsd(perlin(ax, ay, wr.seed, 8) * 1.4, -1.0), 1.0);
            wy = ny * s + wy;
            wx = nx * s + wx;
        }
        pt[0] = wx * wr.p[0] + pt[0];
        pt[1] = wy * wr.p[0] + pt[1];
        false
    })
}

// ---------------------------------------------------------------- Crystalize

#[inline(always)]
fn floor_i64(x: f64) -> i64 {
    let t = cvttd64(x);
    if t as f64 > x { t - 1 } else { t }
}

fn cryst_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mut wr = Warp::new(a);
    let p = a.params;
    let s = p.int(0, 2, 250) as f64;
    wr.p[0] = s;
    wr.p[1] = 1.0 / s;
    wr.q = p.int(1, 1, 5);
    wr.seed = p.0[2] as u32;
    wr.run(dst, |wr, _, pt| {
        let gx = rt::fx::minsd(rt::fx::maxsd((pt[0] + wr.cx + 0.5) * wr.p[1], -1.0e9), 1.0e9);
        let gy = rt::fx::minsd(rt::fx::maxsd((pt[1] + wr.cy + 0.5) * wr.p[1], -1.0e9), 1.0e9);
        let ci = floor_i64(gx) as i32;
        let cj = floor_i64(gy) as i32;
        let mut best = 1.0e300f64;
        let (mut bu, mut bv) = (0.0f64, 0.0f64);
        let mut rr = 1;
        loop {
            for dj in -rr..=rr {
                for di in -rr..=rr {
                    let (i, j) = (ci.wrapping_add(di), cj.wrapping_add(dj));
                    let sx = (hash32(i as u32, j as u32, wr.seed, 0) >> 8) as f64 * 5.9604644775390625e-08 + i as f64;
                    let sy = (hash32(i as u32, j as u32, wr.seed, 1) >> 8) as f64 * 5.9604644775390625e-08 + j as f64;
                    let dx = sx - gx;
                    let dy = sy - gy;
                    let d2 = dx * dx + dy * dy;
                    if d2 < best {
                        best = d2;
                        bu = sx;
                        bv = sy;
                    }
                }
            }
            if rr >= 2 {
                break;
            }
            // the nearest site is proven when d^2 <= (1 + m)^2, m = distance to the cell edge
            let fx = gx - ci as f64;
            let mut m = rt::fx::minsd(fx, 1.0 - fx);
            let fy = gy - cj as f64;
            m = rt::fx::minsd(m, fy);
            m = rt::fx::minsd(m, 1.0 - fy);
            let lim = (m + 1.0) * (m + 1.0);
            if !(lim < best) {
                break;
            }
            rr = 2;
        }
        let ix = floor_i64(bu * wr.p[0]).clamp(0, (wr.w - 1) as i64);
        let iy = floor_i64(bv * wr.p[0]).clamp(0, (wr.h - 1) as i64);
        pt[0] = ix as f64 - wr.cx;
        pt[1] = iy as f64 - wr.cy;
        false
    })
}

// ---------------------------------------------------------------- Pixelate

/// floor(x / d) of the block averages (the asm's 32-bit div) through an f64 division
/// (perf-fx: pipelined, a div r32 is not).  Exact: x < 2^31 and d < 2^22 are exact f64 and
/// the quotient is at most 255.5 (x <= 255 d + d / 2), so a non-integer quotient lies at least
/// 1 / d >= 2^-22 below the next integer while the rounding error is below 2^-44.
#[inline(always)]
fn udiv(x: u32, d: u32) -> u32 {
    cvttd(x as f64 / d as f64) as u32
}

fn pixelate_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let s = a.params.int(0, 1, 100);
    let (w, h) = (a.src.w, a.src.h);
    if a.x1 <= a.x0 {
        return 0;
    }
    let su = s as usize;
    // the blocks of a block row: first column bxs, nb of them (the last clipped to the image)
    let bxs = ((a.x0 as u32) / s as u32 * s as u32) as i32;
    let nb = ((a.x1 - bxs + s - 1) / s) as usize;
    let xe = (bxs + nb as i32 * s).min(w);
    let mut pix = vec![0u32; nb];
    let mut rows: Vec<&[u8]> = Vec::with_capacity(su);
    let mut by = ((a.y0 as u32) / s as u32 * s as u32) as i32;
    while by < a.y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let by1 = (by + s).min(h);
        rows.clear();
        for y in by..by1 {
            rows.push(&a.src_row(y)[bxs as usize * 4..xe as usize * 4]);
        }
        let xn = (xe - bxs) as usize;
        for (i, pp) in pix.iter_mut().enumerate() {
            let (c0, c1) = (i * su, ((i + 1) * su).min(xn));
            let mut v = Sse2.zero();
            // perf-fx: the block's bytes of each row as one slice of a fixed length (the
            // chunks_exact end was recomputed per pixel)
            let nb4 = (c1 - c0) * 4;
            for r in &rows {
                let seg = &r[c0 * 4..][..nb4];
                let mut k = 0;
                while k + 4 <= nb4 {
                    let q: [u8; 4] = seg[k..k + 4].try_into().unwrap_or([0; 4]);
                    v = acc_px(v, u32::from_le_bytes(q));
                    k += 4;
                }
            }
            let mut t = [0i32; 4];
            Sse2.store_i32(&mut t, v);
            let sum = t.map(|c| c as u32);
            let mut v = 0u32;
            let sa = sum[3];
            if sa != 0 {
                let n = ((c1 - c0) as i32 * (by1 - by)) as u32;
                let al = udiv(sa.wrapping_add(n >> 1), n);
                if al != 0 {
                    let hs = sa >> 1;
                    v = al << 24
                        | udiv(sum[2].wrapping_add(hs), sa) << 16
                        | udiv(sum[1].wrapping_add(hs), sa) << 8
                        | udiv(sum[0].wrapping_add(hs), sa);
                }
            }
            *pp = v;
        }
        let dx0 = dst.x0;
        let first = (bxs + s - a.x0) as usize; // ROI pixels in the first block
        let nx = (a.x1 - a.x0) as usize;
        for y in by.max(a.y0)..by1.min(a.y1) {
            let d = &mut dst.row(y)[(a.x0 - dx0) as usize * 4..(a.x1 - dx0) as usize * 4];
            // perf-fx: block spans (first, then su pixels each) instead of a counter per pixel
            let (mut o, mut cnt) = (0usize, first.min(nx));
            for &pv in pix.iter() {
                let b = pv.to_le_bytes();
                for c in d[o * 4..(o + cnt) * 4].chunks_exact_mut(4) {
                    c.copy_from_slice(&b);
                }
                o += cnt;
                cnt = su.min(nx - o);
                if cnt == 0 {
                    break;
                }
            }
        }
        by += s;
    }
    0
}

// ---------------------------------------------------------------- Morphology

const MO_CW: i32 = 512;
const MO_RMAX: i32 = 250;

/// row[i] = max(row[i], row[off + i]) per byte for nb bytes, ascending (off > 0: the reads
/// stay ahead of the writes, as in mo_max2 with dst = a, b = a + off)  [mo_max2]
#[inline(always)]
fn vmax2_in<S: Simd>(s: S, row: &mut [u8], off: usize, nb: usize) {
    let row = &mut row[..off + nb];
    let mut i = 0;
    while i + S::BYTES <= nb {
        // both loads before the store: the bytes at off + i .. are not written yet
        let v = s.maxu8(s.load(&row[i..]), s.load(&row[off + i..]));
        s.store(&mut row[i..], v);
        i += S::BYTES;
    }
    while i < nb {
        row[i] = row[i].max(row[off + i]);
        i += 1;
    }
}

/// d[i] = max(a[i], b[i]) per byte for d.len() bytes (separate buffers)  [mo_max2]
#[inline(always)]
fn vmax2_to<S: Simd>(s: S, d: &mut [u8], a: &[u8], b: &[u8]) {
    let nb = d.len();
    let (a, b) = (&a[..nb], &b[..nb]);
    let mut i = 0;
    while i + S::BYTES <= nb {
        s.store(&mut d[i..], s.maxu8(s.load(&a[i..]), s.load(&b[i..])));
        i += S::BYTES;
    }
    while i < nb {
        d[i] = a[i].max(b[i]);
        i += 1;
    }
}

/// square element, horizontal: one input row (inw pixels) -> max over K = 2r + 1 pixels by
/// doubling (in place), then the first cw results  [mo_core .mc_hrow]
struct SqRowK<'x> {
    row: &'x mut [u8],
    inw: usize,
    cw: usize,
    k: usize,
    p: usize,
}

impl Kernel for SqRowK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let SqRowK { row, inw, cw, k, p } = self;
        let mut kk = 1usize;
        while 2 * kk <= k {
            vmax2_in(s, row, kk * 4, (inw - kk) * 4);
            kk *= 2;
        }
        vmax2_in(s, row, (k - p) * 4, cw * 4);
    }
}

/// square element, vertical: doubling over the rows of the horizontal results (in place),
/// then out[y] = max(R[y], R[y + K - P])  [mo_core .mc_vert, .mc_vout]
struct SqColK<'x> {
    inb: &'x mut [u8],
    out: &'x mut [u8],
    inw: usize,
    inh: usize,
    cw: usize,
    coh: usize,
    k: usize,
    p: usize,
}

impl Kernel for SqColK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let SqColK { inb, out, inw, inh, cw, coh, k, p } = self;
        let st = inw * 4;
        let mut kk = 1usize;
        while 2 * kk <= k {
            for y in 0..inh - kk {
                vmax2_in(s, &mut inb[y * st..], kk * st, cw * 4);
            }
            kk *= 2;
        }
        let inb = &*inb;
        for y in 0..coh {
            vmax2_to(s, &mut out[y * cw * 4..(y + 1) * cw * 4], &inb[y * st..], &inb[(y + k - p) * st..]);
        }
    }
}

/// d[i] = max(d[i], a[i], b[i]) per byte  [mo_max3]
#[inline]
fn max3(d: &mut [u8], src: &[u8], a: usize, b: usize, n: usize) {
    for i in 0..n * 4 {
        d[i] = d[i].max(src[a + i].max(src[b + i]));
    }
}

#[inline]
fn not_px(buf: &mut [u8]) {
    for v in buf.iter_mut() {
        *v = !*v;
    }
}

struct Morph<'a, 'b> {
    a: &'b FxArgs<'a>,
    w: i32,
    h: i32,
    r: i32,
    shape: i32,
    wt: Vec<i32>,
    lg: Vec<i32>,
    j: i32,
    cx0: i32,
    cy0: i32,
    cwa: i32,
    cha: i32,
}

impl Morph<'_, '_> {
    /// premultiplied source over the chunk + margin into buf (packed rows), 0 outside the
    /// image, valid pixels complemented when asked  [mo_load]
    fn load(&self, buf: &mut [u8], m: i32, cpl: bool) -> i32 {
        let bw = self.cwa + 2 * m;
        let bh = self.cha + 2 * m;
        for r in 0..bh {
            if self.a.cancelled() {
                return ERR_CANCEL;
            }
            let row = &mut buf[(r * bw) as usize * 4..((r + 1) * bw) as usize * 4];
            let y = self.cy0 - m + r;
            if y < 0 || y >= self.h {
                row.fill(0);
                continue;
            }
            let x0 = self.cx0 - m;
            let xa = x0.max(0);
            let xb = (x0 + bw).min(self.w);
            if xb <= xa {
                row.fill(0);
                continue;
            }
            row[..(xa - x0) as usize * 4].fill(0);
            row[(xb - x0) as usize * 4..].fill(0);
            let v = &mut row[(xa - x0) as usize * 4..(xb - x0) as usize * 4];
            premul_row(v, &self.a.src_row(y)[xa as usize * 4..xb as usize * 4]);
            if cpl {
                not_px(v);
            }
        }
        0
    }

    /// stage 1 result (chunk + r margins) -> input of stage 2: complement, then 0 outside the
    /// image  [mo_between]
    fn between(&self, buf: &mut [u8]) {
        let r = self.r;
        let bw = self.cwa + 2 * r;
        let bh = self.cha + 2 * r;
        not_px(&mut buf[..(bw * bh) as usize * 4]);
        for k in 0..bh {
            let row = &mut buf[(k * bw) as usize * 4..((k + 1) * bw) as usize * 4];
            let y = self.cy0 - r + k;
            if y < 0 || y >= self.h {
                row.fill(0);
                continue;
            }
            let x0 = self.cx0 - r;
            if x0 < 0 {
                row[..(-x0).min(bw) as usize * 4].fill(0);
            }
            let e = self.w - x0;
            if e < bw {
                row[e.max(0) as usize * 4..].fill(0);
            }
        }
    }

    /// maximum over the structuring element: input (cow + 2r) x (coh + 2r) (destroyed) ->
    /// output cow x coh  [mo_core]
    fn core(&self, inb: &mut [u8], out: &mut [u8], lev: &mut [u8], cow: i32, coh: i32) -> i32 {
        let r = self.r;
        let inw = (cow + 2 * r) as usize;
        let inh = coh + 2 * r;
        let cw = cow as usize;
        if self.shape == 1 {
            // square: separable (pmaxub kernels; byte maxima are exact for any register width)
            let k = (2 * r + 1) as usize;
            let p = 1usize << (31 - (k as u32).leading_zeros());
            for yi in 0..inh as usize {
                if self.a.cancelled() {
                    return ERR_CANCEL;
                }
                simd::dispatch(SqRowK { row: &mut inb[yi * inw * 4..(yi + 1) * inw * 4], inw, cw, k, p });
            }
            simd::dispatch(SqColK { inb, out, inw, inh: inh as usize, cw, coh: coh as usize, k, p });
            return 0;
        }
        // circle / diamond: row spans from doubling tables of every input row
        out[..cw * coh as usize * 4].fill(0);
        for yi in 0..inh {
            if self.a.cancelled() {
                return ERR_CANCEL;
            }
            let l0 = yi as usize * inw * 4;
            for jl in 1..=self.j {
                let h = 1usize << (jl - 1);
                let cnt = inw + 1 - 2 * h;
                let dsto = (jl - 1) as usize * inw * 4;
                if jl == 1 {
                    for i in 0..cnt * 4 {
                        lev[dsto + i] = inb[l0 + i].max(inb[l0 + h * 4 + i]);
                    }
                } else {
                    let so = (jl - 2) as usize * inw * 4;
                    let (lo, hi) = lev.split_at_mut(dsto);
                    for i in 0..cnt * 4 {
                        hi[i] = lo[so + i].max(lo[so + h * 4 + i]);
                    }
                }
            }
            let oy0 = (yi - 2 * r).max(0);
            let oy1 = (coh - 1).min(yi);
            for oy in oy0..=oy1 {
                let d = (yi - r - oy).unsigned_abs() as usize;
                let wv = self.wt[d];
                let jl = self.lg[d];
                let p = 1i32 << jl;
                let a = (r - wv) as usize * 4;
                let b = (r + wv + 1 - p) as usize * 4;
                let dst = &mut out[oy as usize * cw * 4..(oy as usize + 1) * cw * 4];
                if jl == 0 {
                    max3(dst, &inb[l0..], a, b, cw);
                } else {
                    max3(dst, &lev[(jl - 1) as usize * inw * 4..], a, b, cw);
                }
            }
        }
        0
    }
}

fn morph_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let p = a.params;
    let (w, h) = (a.src.w, a.src.h);
    let r = p.int(0, 1, MO_RMAX);
    let mode = p.int(1, 0, 3);
    let shape = p.int(2, 0, 2);
    let (nst, op1, op2) = match mode {
        1 => (1, true, false),
        2 => (2, true, false),
        3 => (2, false, true),
        _ => (1, false, false),
    };
    let m = r * nst;
    let mut wt = vec![0i32; r as usize + 1];
    let mut lg = vec![0i32; r as usize + 1];
    for d in 0..=r {
        let v = match shape {
            1 => r,
            2 => r - d,
            _ => cvttd(rt::fx::sqrtd((r * r + r - d * d) as f64)),
        };
        wt[d as usize] = v;
        lg[d as usize] = 31 - ((2 * v + 1) as u32).leading_zeros() as i32;
    }
    let j = lg[0];
    let cwmax = a.x1 - a.x0;
    if cwmax <= 0 {
        return 0;
    }
    let cwmax = cwmax.min(MO_CW);
    let roih = a.y1 - a.y0;
    if roih <= 0 {
        return 0;
    }
    let chmax = (2 * r).max(64).min(roih);
    let inw = cwmax + 2 * m;
    let inh = chmax + 2 * m;
    let mut bin = vec![0u8; (inw * inh) as usize * 4];
    let mut bmid = vec![0u8; ((cwmax + 2 * r) * (chmax + 2 * r)) as usize * 4];
    let mut bout = vec![0u8; (cwmax * chmax) as usize * 4];
    let mut blev = vec![0u8; (j.max(1) * inw) as usize * 4];
    let mut mo = Morph { a, w, h, r, shape, wt, lg, j, cx0: 0, cy0: 0, cwa: 0, cha: 0 };
    let mut cy0 = a.y0;
    while a.y1 - cy0 > 0 {
        let cha = (a.y1 - cy0).min(chmax);
        let mut cx0 = a.x0;
        while a.x1 - cx0 > 0 {
            let cwa = (a.x1 - cx0).min(cwmax);
            mo.cx0 = cx0;
            mo.cy0 = cy0;
            mo.cwa = cwa;
            mo.cha = cha;
            let e = mo.load(&mut bin, m, op1);
            if e != 0 {
                return e;
            }
            let ext = 2 * (m - r);
            let e = if nst == 1 {
                mo.core(&mut bin, &mut bout, &mut blev, cwa + ext, cha + ext)
            } else {
                mo.core(&mut bin, &mut bmid, &mut blev, cwa + ext, cha + ext)
            };
            if e != 0 {
                return e;
            }
            if nst == 2 {
                mo.between(&mut bmid);
                let e = mo.core(&mut bmid, &mut bout, &mut blev, cwa, cha);
                if e != 0 {
                    return e;
                }
            }
            let last_min = if nst == 1 { op1 } else { op2 };
            if last_min {
                not_px(&mut bout[..(cwa * cha) as usize * 4]);
            }
            for k in 0..cha {
                let y = cy0 + k;
                let o = (k * cwa) as usize * 4;
                let d = &mut dst.row(y)[(cx0 - a.x0) as usize * 4..(cx0 - a.x0 + cwa) as usize * 4];
                unpremul_row(d, &bout[o..o + cwa as usize * 4]);
            }
            cx0 += cwa;
        }
        cy0 += cha;
    }
    0
}

// ---------------------------------------------------------------- tables

static EDGES: [&str; 7] = ["clamp", "wrap", "reflect", "primary", "secondary", "transparent", "original"];

static P_BULGE: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "amount", -200.0, 100.0, 45.0),
    FxProp::pan(1, "offset", 2, -1.0, 1.0, 0.0, 0.0),
    FxProp::new(FP_INT, 0, 3, 0, "quality", 1.0, 5.0, 2.0),
];
static P_TWIST: [FxProp; 4] = [
    FxProp::new(FP_INT, 0, 0, 0, "amount", -200.0, 200.0, 45.0),
    FxProp::new(FP_DEC, 2, 1, 0, "size", 0.01, 2.0, 1.0),
    FxProp::pan(2, "offset", 2, -1.0, 1.0, 0.0, 0.0),
    FxProp::new(FP_INT, 0, 4, 0, "quality", 1.0, 5.0, 2.0),
];
static P_POLAR: [FxProp; 6] = [
    FxProp::new(FP_DEC, 2, 0, 0, "amount", -4.0, 4.0, 4.0),
    FxProp::pan(1, "offset", 2, -1.0, 1.0, 0.0, 0.0),
    FxProp::choice(3, "edge_behavior", 6.0, 2.0, &EDGES),
    FxProp::new(FP_INT, 0, 4, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::color(5, "primary_color", 0xFF00_0000),
    FxProp::color(6, "secondary_color", 0xFFFF_FFFF),
];
static P_TILE: [FxProp; 7] = [
    FxProp::new(FP_ANGLE, 2, 0, 0, "angle", -180.0, 180.0, 30.0),
    FxProp::new(FP_INT, 0, 1, 0, "tile_size", 2.0, 200.0, 40.0),
    FxProp::new(FP_INT, 0, 2, 0, "curvature", -100.0, 100.0, 8.0),
    FxProp::new(FP_INT, 0, 3, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::choice(4, "edge_behavior", 6.0, 1.0, &EDGES),
    FxProp::color(5, "primary_color", 0xFF00_0000),
    FxProp::color(6, "secondary_color", 0xFFFF_FFFF),
];
static P_DENTS: [FxProp; 6] = [
    FxProp::new(FP_DEC, 2, 0, 0, "scale", 1.0, 200.0, 25.0),
    FxProp::new(FP_DEC, 2, 1, 0, "refraction", 0.0, 200.0, 50.0),
    FxProp::new(FP_DEC, 2, 2, 0, "roughness", 0.0, 100.0, 10.0),
    FxProp::new(FP_DEC, 2, 3, 0, "tension", 0.0, 100.0, 10.0),
    FxProp::new(FP_INT, 0, 4, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::new(FP_SEED, 0, 5, 0, "seed", 0.0, 0.0, 0.0),
];
static P_FROST: [FxProp; 4] = [
    FxProp::new(FP_DEC, 2, 0, 0, "scatter_radius", 0.0, 200.0, 3.0),
    FxProp::new(FP_DEC, 2, 1, 0, "smoothness", 0.0, 100.0, 25.0),
    FxProp::new(FP_INT, 0, 2, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::new(FP_SEED, 0, 3, 0, "seed", 0.0, 0.0, 0.0),
];
static P_CRYST: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "cell_size", 2.0, 250.0, 8.0),
    FxProp::new(FP_INT, 0, 1, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::new(FP_SEED, 0, 2, 0, "seed", 0.0, 0.0, 0.0),
];
static P_MORPH: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 250.0, 5.0),
    FxProp::choice(1, "mode", 3.0, 0.0, &["dilate", "erode", "open", "close"]),
    FxProp::choice(2, "shape", 2.0, 0.0, &["circle", "square", "diamond"]),
];
static P_PIXELATE: [FxProp; 1] = [FxProp::new(FP_INT, 0, 0, 0, "cell_size", 1.0, 100.0, 2.0)];

const fn desc(id: u32, flags: u32, name: &'static str, prepare: Option<PrepareFn>, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags, name, prepare, render, props, category: FXC_DISTORT, user: 0 }
}

/// the group table  [fxtab_distort]
pub static FXTAB: [FxDesc; 9] = [
    desc(FX_BULGE, FXF_PREPARE, "bulge", Some(prepare), bulge_render, &P_BULGE),
    desc(FX_CRYSTALIZE, FXF_SEED | FXF_SLOW, "crystalize", None, cryst_render, &P_CRYST),
    desc(FX_DENTS, FXF_PREPARE | FXF_SEED | FXF_SLOW, "dents", Some(prepare), dents_render, &P_DENTS),
    desc(FX_FROSTED_GLASS, FXF_SEED, "frosted_glass", None, frost_render, &P_FROST),
    desc(FX_MORPHOLOGY, 0, "morphology", None, morph_render, &P_MORPH),
    desc(FX_PIXELATE, 0, "pixelate", None, pixelate_render, &P_PIXELATE),
    desc(FX_POLAR_INVERSION, FXF_PREPARE, "polar_inversion", Some(prepare), polar_render, &P_POLAR),
    desc(FX_TILE_REFLECTION, FXF_PREPARE, "tile_reflection", Some(prepare), tile_render, &P_TILE),
    desc(FX_TWIST, FXF_PREPARE, "twist", Some(prepare), twist_render, &P_TWIST),
];
