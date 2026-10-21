// ecfl -- chroma from luma of the VarDCT writer: the Y-to-X / Y-to-B factor of every 64x64
// tile (libjxl 0.11.1 enc_chroma_from_luma.cc FindBestMultiplier, ComputeTile)  [jxl-plus]
//
//   tile(tx, img, qm, acs, rawqf, q, xb, yb, ttx, tty, fast) -> (ytox, ytob)
//                              the int8 factors of tile (ttx, tty); acs = None: DCT8
//                              everywhere, unit quantization (the first pass)

use alloc::vec;
use alloc::vec::Vec;

use simd::{Kernel, Simd};

use crate::eaq::{Img, Quantizer};
use crate::emath::{absf, roundf};
use crate::etx::{self, Qm, Tx};

const COLOR_FACTOR: f32 = 84.0;
const INV_CF: f32 = 1.0 / COLOR_FACTOR;
const Y_TO_B: f32 = 1.0;

/// the residual terms of a tile: a = m / 84, b = base m - s, ac = 2/3 a (CFLFunction's
/// per-coefficient values, computed once per FindBestMultiplier: jxl-speed)
struct Terms<'a> {
    a: &'a [f32],
    b: &'a [f32],
    ac: &'a [f32],
}

/// the derivative of one residual: sign(v) * 2/3 a (|v| + 1), 0 from |v| >= 100
#[inline(always)]
fn term(ac: f32, v: f32) -> f32 {
    let av = absf(v);
    let d = ac * (av + 1.0);
    let d = if v < 0.0 { -d } else { d };
    if av < 100.0 { d } else { 0.0 }
}

/// the three derivative sums of cfl_compute by position mod 8 (term of a x_j + b), the
/// whole groups of 8 (SSE2: two registers per sum, AVX2: one; the scalar term lane by lane)
struct DerivK<'a> {
    t: &'a Terms<'a>,
    x: [f32; 3],
}

impl Kernel for DerivK<'_> {
    type Out = [[f32; 8]; 3];
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> [[f32; 8]; 3] {
        let l = S::BYTES / 4;
        let n8 = self.t.a.len() & !7;
        let zero = s.zero();
        let one = s.splatf(1.0);
        let hundred = s.splatf(100.0);
        let absm = s.splat32(0x7FFF_FFFF);
        let sign = s.splat32(0x8000_0000);
        let xs = [s.splatf(self.x[0]), s.splatf(self.x[1]), s.splatf(self.x[2])];
        let mut f = [[zero; 2]; 3];
        let mut i = 0;
        while i < n8 {
            for h in 0..8 / l {
                let k = i + h * l;
                let a = s.load_f32(&self.t.a[k..]);
                let b = s.load_f32(&self.t.b[k..]);
                let ac = s.load_f32(&self.t.ac[k..]);
                for j in 0..3 {
                    let v = s.addf(s.mulf(a, xs[j]), b);
                    let av = s.and(v, absm);
                    let d = s.mulf(ac, s.addf(av, one));
                    let d = s.xor(d, s.and(s.cmpltf(v, zero), sign));
                    f[j][h] = s.addf(f[j][h], s.and(d, s.cmpltf(av, hundred)));
                }
            }
            i += 8;
        }
        let mut o = [[0f32; 8]; 3];
        for j in 0..3 {
            if l == 8 {
                s.store_f32(&mut o[j], f[j][0]);
            } else {
                s.store_f32(&mut o[j][..4], f[j][0]);
                s.store_f32(&mut o[j][4..], f[j][1]);
            }
        }
        o
    }
}

/// the sums of |a x + b| by position mod 8, the whole groups of 8
struct L1K<'a> {
    t: &'a Terms<'a>,
    x: f32,
}

impl Kernel for L1K<'_> {
    type Out = [f32; 8];
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> [f32; 8] {
        let l = S::BYTES / 4;
        let n8 = self.t.a.len() & !7;
        let absm = s.splat32(0x7FFF_FFFF);
        let x = s.splatf(self.x);
        let mut f = [s.zero(); 2];
        let mut i = 0;
        while i < n8 {
            for (h, fh) in f.iter_mut().enumerate().take(8 / l) {
                let k = i + h * l;
                let v = s.addf(s.mulf(s.load_f32(&self.t.a[k..]), x), s.load_f32(&self.t.b[k..]));
                *fh = s.addf(*fh, s.and(v, absm));
            }
            i += 8;
        }
        let mut o = [0f32; 8];
        if l == 8 {
            s.store_f32(&mut o, f[0]);
        } else {
            s.store_f32(&mut o[..4], f[0]);
            s.store_f32(&mut o[4..], f[1]);
        }
        o
    }
}

#[inline(always)]
fn sum8(v: &[f32; 8]) -> f32 {
    ((v[0] + v[1]) + (v[2] + v[3])) + ((v[4] + v[5]) + (v[6] + v[7]))
}

/// CFLFunction::Compute: f'(x) and f'(x +- eps)
fn cfl_compute(t: &Terms, dmul: f32, x: f32, eps: f32) -> (f32, f32, f32) {
    let num = t.a.len() as f32;
    let (xp, xm) = (x + eps, x - eps);
    // eight lanes of partial sums, the tail lane by lane
    let mut f = simd::dispatch(DerivK { t, x: [x, xp, xm] });
    let n8 = t.a.len() & !7;
    for k in n8..t.a.len() {
        let (a, b, ac) = (t.a[k], t.b[k], t.ac[k]);
        f[0][k & 7] += term(ac, a * x + b);
        f[1][k & 7] += term(ac, a * xp + b);
        f[2][k & 7] += term(ac, a * xm + b);
    }
    (2.0 * dmul * num * x + sum8(&f[0]), 2.0 * dmul * num * xp + sum8(&f[1]), 2.0 * dmul * num * xm + sum8(&f[2]))
}

/// the sum of |residual| at x (by position mod 8)
fn l1(t: &Terms, x: f32) -> f32 {
    let mut f8 = simd::dispatch(L1K { t, x });
    let n8 = t.a.len() & !7;
    for i in n8..t.a.len() {
        f8[i & 7] += absf(t.a[i] * x + t.b[i]);
    }
    sum8(&f8)
}

/// the least squares solution
fn least_squares(t: &Terms, dmul: f32) -> f32 {
    let (mut ca, mut cb) = (0.0f32, 0.0f32);
    for i in 0..t.a.len() {
        let (a, b) = (t.a[i], t.b[i]);
        ca += a * a;
        cb += a * b;
    }
    -cb / (ca + t.a.len() as f32 * dmul * 0.5)
}

/// FindBestMultiplier (ta / tb / tac: buffers for the terms)
fn best_multiplier(m: &[f32], s: &[f32], base: f32, dmul: f32, fast: bool, ta: &mut Vec<f32>, tb: &mut Vec<f32>, tac: &mut Vec<f32>) -> i8 {
    if m.is_empty() {
        return 0;
    }
    let coeffx2 = (1.0f32 / 3.0) * 2.0;
    ta.clear();
    tb.clear();
    tac.clear();
    for i in 0..m.len() {
        let a = INV_CF * m[i];
        ta.push(a);
        tb.push(base * m[i] - s[i]);
        tac.push(coeffx2 * a);
    }
    let t = Terms { a: ta, b: tb, ac: tac };
    let mut x;
    if fast {
        x = least_squares(&t, dmul);
    } else {
        let eps = 100.0f32;
        x = 0.0;
        for _ in 0..20 {
            let (df, dfpe, dfme) = cfl_compute(&t, dmul, x, eps);
            let ddf = (dfpe - dfme) / (2.0 * eps);
            let step = df / (ddf + 0.85);
            x -= step.clamp(-20.0, 20.0);
            if absf(step) < 3e-3 {
                break;
            }
        }
        // jxl-plus: the Newton iteration oscillates when the residuals are mostly noise (grey
        // areas) and goes astray on sharp edges (text): of it, the least squares solution and
        // no correlation the one with the smallest sum of |residual|
        let xl = least_squares(&t, dmul);
        let (cn, cl, c0) = (l1(&t, x), l1(&t, xl), l1(&t, 0.0));
        if cl < cn && cl <= c0 {
            x = xl;
        } else if c0 <= cn {
            x = 0.0;
        }
    }
    let tz = 2.6f32;
    if x >= tz {
        x -= tz;
    } else if x <= -tz {
        x += tz;
    } else {
        x = 0.0;
    }
    roundf(x).clamp(-128.0, 127.0) as i8
}

/// the buffers of tile (reused from tile to tile: jxl-speed)
pub struct Buf {
    cyx: Vec<f32>,
    cx: Vec<f32>,
    cyb: Vec<f32>,
    cb: Vec<f32>,
    by: Vec<f32>,
    bx: Vec<f32>,
    bb: Vec<f32>,
    ta: Vec<f32>,
    tb: Vec<f32>,
    tac: Vec<f32>,
}

impl Default for Buf {
    fn default() -> Self {
        Buf::new()
    }
}

impl Buf {
    pub fn new() -> Buf {
        Buf {
            cyx: Vec::with_capacity(64 * 64),
            cx: Vec::with_capacity(64 * 64),
            cyb: Vec::with_capacity(64 * 64),
            cb: Vec::with_capacity(64 * 64),
            by: vec![0f32; 4096],
            bx: vec![0f32; 4096],
            bb: vec![0f32; 4096],
            ta: Vec::with_capacity(64 * 64),
            tb: Vec::with_capacity(64 * 64),
            tac: Vec::with_capacity(64 * 64),
        }
    }
}

/// the CfL factors (ytox, ytob) of tile (tx, ty) (8 x 8 blocks)
#[allow(clippy::too_many_arguments)]
pub fn tile(tx: &mut Tx, buf: &mut Buf, img: &Img, qm: &Qm, acs: Option<&[u8]>, rawqf: Option<&[i32]>, q: &Quantizer, xb: usize, yb: usize, ttx: usize, tty: usize, fast: bool) -> (i8, i8) {
    let x0 = ttx * 8;
    let y0 = tty * 8;
    let x1 = (x0 + 8).min(xb);
    let y1 = (y0 + 8).min(yb);
    let stride = img.w;
    let Buf { cyx, cx: cx_, cyb, cb, by: by_, bx: bx_, bb: bb_, ta, tb, tac } = buf;
    cyx.clear();
    cx_.clear();
    cyb.clear();
    cb.clear();
    let use_dct8 = acs.is_none();
    for y in y0..y1 {
        for x in x0..x1 {
            let a = match acs {
                Some(v) => {
                    let a8 = v[y * xb + x];
                    if a8 & 1 == 0 {
                        continue;
                    }
                    (a8 >> 1) as usize
                }
                None => etx::DCT,
            };
            let (cxb, cyb_) = etx::cover(a);
            if x + cxb > x1 || y + cyb_ > y1 {
                continue;
            }
            let o = y * 8 * stride + x * 8;
            let n = etx::size_of(a);
            etx::from_pixels(tx, a, &img.p[1][o..], stride, &mut by_[..n]);
            etx::from_pixels(tx, a, &img.p[0][o..], stride, &mut bx_[..n]);
            etx::from_pixels(tx, a, &img.p[2][o..], stride, &mut bb_[..n]);
            let (mn, mx) = etx::llf_dims(a);
            for iy in 0..mn {
                for ix in 0..mx {
                    by_[mx * 8 * iy + ix] = 0.0;
                    bx_[mx * 8 * iy + ix] = 0.0;
                    bb_[mx * 8 * iy + ix] = 0.0;
                }
            }
            let qq = match rawqf {
                Some(r) => r[y * xb + x] as f32,
                None => 1.0,
            };
            let qv = if use_dct8 { 1.0 } else { q.scale * 128.0 * qq };
            let (_, qmx) = qm.of(a, 0);
            let (_, qmb) = qm.of(a, 2);
            for i in 0..n {
                let qx = qv * qmx[i];
                let qb = qv * qmb[i];
                cyx.push(by_[i] * qx);
                cx_.push(bx_[i] * qx);
                cyb.push(by_[i] * qb);
                cb.push(bb_[i] * qb);
            }
        }
    }
    const DMUL: f32 = 1e-9;
    let fx = best_multiplier(cyx, cx_, 0.0, DMUL, fast, ta, tb, tac);
    let fb = best_multiplier(cyb, cb, Y_TO_B, DMUL, fast, ta, tb, tac);
    (fx, fb)
}
