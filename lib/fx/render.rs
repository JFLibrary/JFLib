// render -- effect group "render" (Rendern): Clouds, Julia Fractal, Mandelbrot Fractal,
// Turbulence                                                                [src/fx/fx_render.s]
//
//   FXTAB            the group table
//
// All four are per-pixel functions of (x, y, image size, parameters).
//   clouds      paint.net 3.x algorithm, own lattice noise: dx = 2x - w, dy = 2y - h; octaves
//               i = 0, 1, ... while i < 12, amp > 0.03 and div > 0 (amp = power^i, div = scale
//               >> i): val += perlin(dx / div + ox_i, dy / div + oy_i, seed, i) amp;
//               t = clamp((val + 1) / 2, 0, 1)
//   turbulence  octaves i < detail while div > 0, amp = roughness^i; val = sum |perlin| amp;
//               t = clamp(2 val / sum amp, 0, 1)
//               Colour = round(c1 + (c2 - c1) t) per channel (f32), blended over the source
//               with the blend mode (gfx::blendmode)
//   julia_fractal / mandelbrot_fractal  paint.net 3.x algorithms, count samples per pixel,
//               escape iteration per sample (the asm runs 6 / 12 packed lanes; every sample
//               does the same IEEE operations), colour schemes classic / grayscale / palette /
//               rainbow, channel sums divided by count (truncating)

use gfx::blendmode::bm_row;
use rt::fx::{cvt, cvtd, cvttd, cvttd64, maxsd, minsd};

use crate::perlin::{perlin_row, PerlinRow};
use alloc::vec;
use simd::{Kernel, Simd, Sse2};
use crate::warp::{log, sincos};
use crate::*;

const NZ_SEG: usize = 256;
const NZ_MAXOCT: usize = 12;
const DEG2RAD: f64 = 0.017453292519943295769;

static OFS: [[f64; 2]; 12] = [
    [0.0, 0.0],
    [0.6180339887498949, 0.75487766624669272],
    [0.23606797749978981, 0.50975533249338545],
    [0.85410196624968471, 0.26463299874007795],
    [0.47213595499957961, 0.019510664986770898],
    [0.090169943749474513, 0.77438833123346384],
    [0.70820393249936942, 0.5292659974801559],
    [0.32623792124926432, 0.28414366372684885],
    [0.94427190999915922, 0.039021329973541796],
    [0.56230589874905412, 0.79389899622023474],
    [0.18033988749894903, 0.54877666246692769],
    [0.79837387624884393, 0.30365432871361975],
];

/// straight colour -> 4 x f32 (b, g, r, a)  [COLVEC]
#[inline]
fn colvec(c: u32) -> [f32; 4] {
    [(c & 255) as f32, (c >> 8 & 255) as f32, (c >> 16 & 255) as f32, (c >> 24) as f32]
}

/// t (f64) -> straight colour c1 + dc t (f32 lanes, rounded, saturated)
#[inline]
fn ramp(t: f64, c1: &[f32; 4], dc: &[f32; 4]) -> [i32; 4] {
    let tf = t as f32;
    core::array::from_fn(|c| cvt(tf * dc[c] + c1[c]))
}

#[inline]
fn pack(v: &[i32; 4]) -> u32 {
    let mut p = 0u32;
    for (c, &x) in v.iter().enumerate() {
        p |= (x.clamp(-32768, 32767).clamp(0, 255) as u32) << (8 * c);
    }
    p
}

// ---------------------------------------------------------------- Clouds / Turbulence

fn noise_render(a: &FxArgs, dst: &mut Dst, turb: bool) -> i32 {
    let p = a.params;
    let mode = p.int(0, 0, 13) as u32 | 0xFF00;
    let mut div = p.int(1, 2, 1000) as u32;
    let power = p.dbl(2, 0.0, 1.0);
    let (maxoct, ampmin, seed, cs) = if turb {
        (p.int(3, 1, NZ_MAXOCT as i32) as usize, -1.0, p.0[4] as u32, 5)
    } else {
        (NZ_MAXOCT, 0.03, p.0[3] as u32, 4)
    };
    let mut inv = [0f64; NZ_MAXOCT];
    let mut ampv = [0f64; NZ_MAXOCT];
    let mut n = 0usize;
    let mut amp = 1.0f64;
    let mut sum = 0.0f64;
    while n < maxoct && amp > ampmin && div != 0 {
        inv[n] = 1.0 / div as f64;
        ampv[n] = amp;
        sum += amp;
        amp *= power;
        div >>= 1;
        n += 1;
    }
    let norm = 2.0 / sum;
    let c1 = colvec(p.0[cs] as u32);
    let c2 = colvec(p.0[cs + 1] as u32);
    let dc: [f32; 4] = core::array::from_fn(|c| c2[c] - c1[c]);
    let (w, h) = (a.src.w, a.src.h);
    let mut acc = [0f64; NZ_SEG];
    let mut buf = [0u8; NZ_SEG * 4];
    for y in a.y0..a.y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let dy = (y + y - h) as f64;
        let srow = a.src_row(y);
        let drow = dst.row(y);
        let mut x = a.x0;
        while a.x1 - x > 0 {
            let ns = ((a.x1 - x) as usize).min(NZ_SEG);
            acc[..ns].fill(0.0);
            for i in 0..n {
                let rec = PerlinRow {
                    n: ns,
                    x0: x + x - w,
                    step: 2,
                    abs: turb,
                    seed,
                    stream: i as u32,
                    inv: inv[i],
                    ofsx: OFS[i][0],
                    y: inv[i] * dy + OFS[i][1],
                    amp: ampv[i],
                };
                perlin_row(&rec, &mut acc);
            }
            for j in 0..ns {
                let v = acc[j];
                let t = if turb { v * norm } else { (v + 1.0) * 0.5 };
                let t = minsd(maxsd(t, 0.0), 1.0);
                put(&mut buf, j, pack(&ramp(t, &c1, &dc)));
            }
            let o = (x - a.x0) as usize * 4;
            bm_row(&mut drow[o..o + ns * 4], &srow[x as usize * 4..x as usize * 4 + ns * 4], &buf[..ns * 4], mode);
            x += ns as i32;
        }
    }
    0
}

fn clouds_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    noise_render(a, dst, false)
}

fn turb_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    noise_render(a, dst, true)
}

// ---------------------------------------------------------------- Julia / Mandelbrot

const FF_CHUNK: usize = 32; // pixels per sample batch
const FF_MAXCNT: usize = 26; // samples per pixel (q = 5: 25 + 1)
const NREG: usize = 3; // registers of lanes per step (the asm: three f64 pairs)
const MAXL: usize = 12;

/// the escape iteration of a batch of samples on packed f64 lanes  [fr_lanes, fr_lanes_avx]
/// Stop at n = kmax or when |z|^2 >= r2 (both checked before each step); record n and
/// |z|^2.  A lane takes the next sample as soon as its current one stops; a global step
/// counter g replaces per-lane counters (a lane started at g0 has n = g - g0, its deadline is
/// g0 + kmax).  Every sample does the same IEEE operations as a scalar loop would: the lane
/// count (6 with SSE2, 12 with AVX2) changes no bit.
struct Lanes<'a> {
    sx: &'a [f64],
    sy: &'a [f64],
    inside: &'a [bool],
    nf: &'a mut [f64],
    sf: &'a mut [f64],
    mand: bool,
    r2: f64,
    kmax: i32,
}

/// f64 per lane, kept as bytes so that a register of lanes is one load / store (perf-fx: the
/// copies through a small array became element inserts at opt-level 2)
struct F64s([u8; 8 * MAXL]);

impl F64s {
    #[inline(always)]
    fn get(&self, c: usize) -> f64 {
        f64::from_le_bytes(self.0[c * 8..c * 8 + 8].try_into().unwrap_or([0; 8]))
    }
    #[inline(always)]
    fn set(&mut self, c: usize, v: f64) {
        self.0[c * 8..c * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    /// the lanes of register r (S::BYTES / 8 of them)
    #[inline(always)]
    fn ld<S: Simd>(&self, s: S, r: usize) -> S::V {
        s.load(&self.0[r * S::BYTES..])
    }
    #[inline(always)]
    fn st<S: Simd>(&mut self, s: S, r: usize, v: S::V) {
        s.store(&mut self.0[r * S::BYTES..], v)
    }
}

/// the samples a lane takes from
struct Src<'a> {
    sx: &'a [f64],
    sy: &'a [f64],
    inside: &'a [bool],
    mand: bool,
    kmax: i32,
}

/// the state of the lanes (LN_* / LA_*)
struct LaneSt {
    zx: F64s,
    zy: F64s,
    cx: F64s,
    cy: F64s,
    idx: [i32; MAXL],
    dl: [i32; MAXL],
    next: usize,
}

impl LaneSt {
    /// lane c takes the next sample that is not inside, or becomes idle (z = c = 0, no
    /// deadline)  [LA_TAKE]
    #[inline(always)]
    fn take(&mut self, c: usize, lb: &Src, g: i32) {
        let n = lb.sx.len();
        while self.next < n && lb.inside[self.next] {
            self.next += 1;
        }
        let j = self.next;
        if j < n {
            self.idx[c] = j as i32;
            self.dl[c] = g + lb.kmax;
            let (zx, zy, cx, cy) = if lb.mand { (0.0, 0.0, lb.sx[j], lb.sy[j]) } else { (lb.sx[j], lb.sy[j], 0.3125, 0.03) };
            self.zx.set(c, zx);
            self.zy.set(c, zy);
            self.cx.set(c, cx);
            self.cy.set(c, cy);
            self.next += 1;
        } else {
            self.zx.set(c, 0.0);
            self.zy.set(c, 0.0);
            self.cx.set(c, 0.0);
            self.cy.set(c, 0.0);
            self.dl[c] = i32::MAX;
            self.idx[c] = -1;
        }
    }

    /// lane c (busy) stops at step g: SF = |z|^2, NF = g - (deadline - kmax)  [LA_RECORD]
    #[inline(always)]
    fn record(&self, c: usize, m: f64, g: i32, kmax: i32, nf: &mut [f64], sf: &mut [f64]) {
        let j = self.idx[c] as usize;
        sf[j] = m;
        nf[j] = (g - (self.dl[c] - kmax)) as f64;
    }
}

impl Kernel for Lanes<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let l = S::BYTES / 8;
        let nl = NREG * l;
        let Lanes { sx, sy, inside, nf, sf, mand, r2, kmax } = self;
        let src = Src { sx, sy, inside, mand, kmax };
        let mut ls = LaneSt {
            zx: F64s([0; 8 * MAXL]),
            zy: F64s([0; 8 * MAXL]),
            cx: F64s([0; 8 * MAXL]),
            cy: F64s([0; 8 * MAXL]),
            idx: [-1; MAXL],
            dl: [i32::MAX; MAXL],
            next: 0,
        };
        let mut g = 0i32;
        let full = if S::BYTES == 32 { u32::MAX } else { 0xFFFF };
        let vr2 = s.splatd(r2);
        // every lane takes a sample (or stays idle)
        for c in 0..nl {
            ls.take(c, &src, g);
        }
        loop {
            let mut mindl = i32::MAX;
            for &d in &ls.dl[..nl] {
                mindl = mindl.min(d);
            }
            if mindl == i32::MAX {
                break;
            }
            let mut x = [s.zero(); NREG];
            let mut y = [s.zero(); NREG];
            let mut vcx = [s.zero(); NREG];
            let mut vcy = [s.zero(); NREG];
            for r in 0..NREG {
                x[r] = ls.zx.ld(s, r);
                y[r] = ls.zy.ld(s, r);
                vcx[r] = ls.cx.ld(s, r);
                vcy[r] = ls.cy.ld(s, r);
            }
            // perf-fx: the byte masks of |z|^2 < r2 per register when a lane escaped
            let mut esc = [full; NREG];
            while g < mindl {
                let mut x2 = [s.zero(); NREG];
                let mut y2 = [s.zero(); NREG];
                let mut all = full;
                for r in 0..NREG {
                    x2[r] = s.muld(x[r], x[r]);
                    y2[r] = s.muld(y[r], y[r]);
                    esc[r] = s.movemask8(s.cmpltd(s.addd(x2[r], y2[r]), vr2));
                    all &= esc[r];
                }
                if all != full {
                    break;
                }
                for r in 0..NREG {
                    let nx = s.addd(s.subd(x2[r], y2[r]), vcx[r]);
                    y[r] = s.addd(s.muld(s.addd(x[r], x[r]), y[r]), vcy[r]);
                    x[r] = nx;
                }
                g += 1;
            }
            for r in 0..NREG {
                ls.zx.st(s, r, x[r]);
                ls.zy.st(s, r, y[r]);
            }
            if g < mindl {
                // perf-fx: escapes (no lane is at its limit): only the escaped lanes stop and
                // take the next sample, as fr_lanes_avx .la_esc (idle lanes have z = 0 and
                // never escape; the scalar |z|^2 equals the lane's)
                for r in 0..NREG {
                    for k in 0..l {
                        let c = r * l + k;
                        if esc[r] >> (8 * k) & 1 == 0 && ls.idx[c] >= 0 {
                            let (zx, zy) = (ls.zx.get(c), ls.zy.get(c));
                            let m = zx * zx + zy * zy;
                            ls.record(c, m, g, kmax, nf, sf);
                            ls.take(c, &src, g);
                        }
                    }
                }
            } else {
                // a lane reached its limit: stop every lane at its limit or escaped
                for c in 0..nl {
                    if ls.idx[c] < 0 {
                        continue;
                    }
                    let (zx, zy) = (ls.zx.get(c), ls.zy.get(c));
                    let m = zx * zx + zy * zy;
                    if g >= ls.dl[c] || !(m < r2) {
                        ls.record(c, m, g, kmax, nf, sf);
                        ls.take(c, &src, g);
                    }
                }
            }
        }
    }
}

/// rainbow: c >= 1023 or inside: opaque black; else hue 6 frac(c / 64) at full saturation
/// [fr_rainbow]
fn rainbow(c: f64, inside: bool) -> [i32; 4] {
    if inside || c >= 1023.0 || c.is_nan() {
        return [0, 0, 0, 255];
    }
    let hh = maxsd(c * 0.015625, -1.0e9);
    let mut t = cvttd64(hh);
    if t as f64 > hh {
        t -= 1;
    }
    let mut f = (hh - t as f64) * 6.0;
    let s = cvttd(f);
    f -= s as f64;
    let ff = cvtd(f * 255.0);
    let d = 255 - ff;
    let (r, g, b) = match s {
        0 => (255, ff, 0),
        1 => (d, 255, 0),
        2 => (0, 255, ff),
        3 => (0, d, 255),
        4 => (ff, 0, 255),
        _ => (255, 0, d),
    };
    [b, g, r, 255]
}

fn fractal_render(a: &FxArgs, dst: &mut Dst, mand: bool) -> i32 {
    let p = a.params;
    let (fact, q, invz, ang, scheme, cs, count, kmax, r2, invert, k1, k2);
    if !mand {
        fact = p.dbl(0, 1.0, 10.0);
        q = p.int(1, 1, 5);
        invz = 1.0 / p.dbl(2, 0.01, 50.0);
        ang = p.dbl(3, -36000.0, 36000.0);
        scheme = p.int(4, 0, 3);
        cs = 5usize;
        count = q * q;
        kmax = 256;
        r2 = 10000.0;
        invert = false;
        k1 = [768.0, 512.0];
        k2 = [256.0, 0.0];
    } else {
        let f = p.int(0, 1, 10);
        fact = f as f64;
        kmax = (1023 / f as u32 + 1) as i32;
        r2 = 100000.0;
        q = p.int(1, 1, 5);
        invz = 1.0 / (p.int(2, 0, 50) * 20 + 1) as f64;
        invert = p.int(4, 0, 1) != 0;
        scheme = p.int(5, 0, 3);
        ang = p.dbl(3, -36000.0, 36000.0);
        cs = 6;
        count = q * q + 1;
        k1 = [256.0, 512.0];
        k2 = [768.0, 0.0];
    }
    let invq = 1.0 / q as f64;
    let invcnt = 1.0 / count as f64;
    let invh = 1.0 / a.src.h as f64;
    let (sn, cn) = sincos(ang * DEG2RAD);
    let (c1v, c2v) = if scheme == 1 { (0xFF00_0000, 0xFFFF_FFFF) } else { (p.0[cs] as u32, p.0[cs + 1] as u32) };
    let c1 = colvec(c1v);
    let c2 = colvec(c2v);
    let dc: [f32; 4] = core::array::from_fn(|c| c2[c] - c1[c]);
    let (w, h) = (a.src.w, a.src.h);
    let ns = FF_CHUNK * FF_MAXCNT;
    let (mut sx, mut sy, mut nf, mut sf) = (vec![0f64; ns], vec![0f64; ns], vec![0f64; ns], vec![0f64; ns]);
    let mut ins = vec![false; ns];
    let cnt = count as usize;
    let f64x2 = |p: f64, q: f64| {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&p.to_le_bytes());
        b[8..].copy_from_slice(&q.to_le_bytes());
        Sse2.load(&b)
    };
    let (vk1, vk2) = (f64x2(k1[0], k1[1]), f64x2(k2[0], k2[1]));
    let v255 = Sse2.splatd(255.0);
    let vcf = Sse2.splatd(count as f64);
    let (vdc, vc1) = (Sse2.load_f32(&dc), Sse2.load_f32(&c1));
    for y in a.y0..a.y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let by = (y as f64 + y as f64) - h as f64;
        let dx0 = dst.x0;
        let drow = dst.row(y);
        let mut xa = a.x0;
        while a.x1 - xa > 0 {
            let npx = ((a.x1 - xa) as usize).min(FF_CHUNK);
            // the samples of the chunk
            let mut j = 0usize;
            for k in 0..npx {
                let x = xa + k as i32;
                let bx = (x as f64 + x as f64) - w as f64;
                for i in 0..cnt {
                    let fi = i as i32 as f64;
                    let u = (fi * invcnt + bx) * invh;
                    let t = fi * invq;
                    let v = (t - cvttd(t) as f64 + by) * invh;
                    let ur = u * cn - v * sn;
                    let vr = u * sn + v * cn;
                    let (mut px, mut py) = (ur * invz, vr * invz);
                    ins[j] = false;
                    if mand {
                        px += -0.7453;
                        py += 0.1127;
                        let xq = px - 0.25;
                        let y2 = py * py;
                        let qq = xq * xq + y2;
                        if !((xq + qq) * qq > y2 * 0.25) {
                            ins[j] = true;
                        } else {
                            let x1 = px + 1.0;
                            ins[j] = !(x1 * x1 + py * py > 0.0625);
                        }
                    }
                    sx[j] = px;
                    sy[j] = py;
                    j += 1;
                }
            }
            simd::dispatch(Lanes { sx: &sx[..j], sy: &sy[..j], inside: &ins[..j], nf: &mut nf, sf: &mut sf, mand, r2, kmax });
            // perf-fx: the colours and the per-pixel division on packed lanes, as the asm
            // (.ff_colour: subpd / maxpd / minpd / cvttpd2dq, mulps / addps / cvtps2dq;
            // .ff_px_done: cvtdq2pd / divpd / cvttpd2dq / packssdw / packuswb)
            let x = Sse2;
            let mut j = 0usize;
            for k in 0..npx {
                let mut sum = x.zero();
                for _ in 0..cnt {
                    let inside = ins[j];
                    let val = if !mand {
                        (nf[j] - (2.0 - 18.420680743952367 / log(sf[j]))) * fact
                    } else if inside {
                        0.0
                    } else {
                        (nf[j] - log(sf[j]) * 0.086858896380650365) * fact + 64.0
                    };
                    let col = match scheme {
                        0 => {
                            if inside {
                                x.splat32(255)
                            } else {
                                let v = x.splatd(val);
                                let z = x.zero();
                                let lo = x.cvtt_f64_i32(x.mind(x.maxd(x.subd(v, vk1), z), v255));
                                let hi = x.cvtt_f64_i32(x.mind(x.maxd(x.subd(v, vk2), z), v255));
                                x.shufps::<0x44>(lo, hi)
                            }
                        }
                        3 => x.load_i32(&rainbow(val, inside)),
                        _ => {
                            let c = if inside { 1024.0 } else { val };
                            let t = minsd(maxsd(c * 0.0009765625, 0.0), 1.0);
                            x.cvtr_f32_i32(x.addf(x.mulf(x.splatf(t as f32), vdc), vc1))
                        }
                    };
                    sum = x.add32(sum, col);
                    j += 1;
                }
                let lo = x.cvtt_f64_i32(x.divd(x.cvt_i32_f64(sum), vcf));
                let hi = x.cvtt_f64_i32(x.divd(x.cvt_i32_f64(x.shuf32::<0x0E>(sum)), vcf));
                let q = x.packs32(x.shufps::<0x44>(lo, hi), x.zero());
                let mut pxv = x.to_i32(x.packus16(q, q)) as u32;
                if invert {
                    pxv ^= 0x00FF_FFFF;
                }
                put(drow, (xa + k as i32 - dx0) as usize, pxv);
            }
            xa += npx as i32;
        }
    }
    0
}

fn julia_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    fractal_render(a, dst, false)
}

fn mandel_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    fractal_render(a, dst, true)
}

// ---------------------------------------------------------------- tables

static BLENDS: [&str; 14] = [
    "normal", "multiply", "additive", "color_burn", "color_dodge", "reflect", "glow", "overlay", "difference", "negation", "lighten",
    "darken", "screen", "xor",
];
static SCHEMES: [&str; 4] = ["classic", "grayscale", "palette", "rainbow"];

static P_CLOUDS: [FxProp; 6] = [
    FxProp::choice(0, "blend_mode", 13.0, 0.0, &BLENDS),
    FxProp::new(FP_INT, 0, 1, 0, "scale", 2.0, 1000.0, 250.0),
    FxProp::new(FP_DEC, 2, 2, 0, "power", 0.0, 1.0, 0.5),
    FxProp::new(FP_SEED, 0, 3, 0, "seed", 0.0, 0.0, 0.0),
    FxProp::color(4, "primary_color", 0xFF00_0000),
    FxProp::color(5, "secondary_color", 0xFFFF_FFFF),
];
static P_TURB: [FxProp; 7] = [
    FxProp::choice(0, "blend_mode", 13.0, 0.0, &BLENDS),
    FxProp::new(FP_INT, 0, 1, 0, "scale", 2.0, 1000.0, 250.0),
    FxProp::new(FP_DEC, 2, 2, 0, "roughness", 0.0, 1.0, 0.5),
    FxProp::new(FP_INT, 0, 3, 0, "detail", 1.0, 12.0, 8.0),
    FxProp::new(FP_SEED, 0, 4, 0, "seed", 0.0, 0.0, 0.0),
    FxProp::color(5, "primary_color", 0xFF00_0000),
    FxProp::color(6, "secondary_color", 0xFFFF_FFFF),
];
static P_JULIA: [FxProp; 7] = [
    FxProp::new(FP_DEC, 2, 0, 0, "factor", 1.0, 10.0, 4.0),
    FxProp::new(FP_INT, 0, 1, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::new(FP_DEC, 2, 2, 0, "zoom", 0.0, 50.0, 1.0),
    FxProp::new(FP_ANGLE, 2, 3, 0, "angle", -180.0, 180.0, 0.0),
    FxProp::choice(4, "color_scheme", 3.0, 0.0, &SCHEMES),
    FxProp::color(5, "primary_color", 0xFF00_0000),
    FxProp::color(6, "secondary_color", 0xFFFF_FFFF),
];
static P_MANDEL: [FxProp; 8] = [
    FxProp::new(FP_INT, 0, 0, 0, "factor", 1.0, 10.0, 1.0),
    FxProp::new(FP_INT, 0, 1, 0, "quality", 1.0, 5.0, 2.0),
    FxProp::new(FP_INT, 0, 2, 0, "zoom", 0.0, 50.0, 10.0),
    FxProp::new(FP_ANGLE, 2, 3, 0, "angle", -180.0, 180.0, 0.0),
    FxProp::new(FP_BOOL, 0, 4, 0, "invert_colors", 0.0, 1.0, 0.0),
    FxProp::choice(5, "color_scheme", 3.0, 0.0, &SCHEMES),
    FxProp::color(6, "primary_color", 0xFF00_0000),
    FxProp::color(7, "secondary_color", 0xFFFF_FFFF),
];

const fn desc(id: u32, flags: u32, name: &'static str, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags, name, prepare: None, render, props, category: FXC_RENDER, user: 0 }
}

/// the group table  [fxtab_render]
pub static FXTAB: [FxDesc; 4] = [
    desc(FX_CLOUDS, FXF_SEED, "clouds", clouds_render, &P_CLOUDS),
    desc(FX_JULIA, FXF_SLOW, "julia_fractal", julia_render, &P_JULIA),
    desc(FX_MANDELBROT, FXF_SLOW, "mandelbrot_fractal", mandel_render, &P_MANDEL),
    desc(FX_TURBULENCE, FXF_SEED, "turbulence", turb_render, &P_TURB),
];
