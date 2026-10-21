// warp -- warp framework of the distort effects: inverse mapping per destination pixel, n x n
// supersampling, bilinear sampling in premultiplied space, edge behaviour; the f64 maths the
// fx kernels share                                                          [src/fx/fx_warp.s]
//
//   prepare(a) -> aux                bounds for centre / radius (the selection's bounding box)
//   Warp::new(a)                     the WARP record of a render call (fxw_init)
//   wr.center(x, y)                  centre from an FP_PAN pair (fxw_center)
//   wr.run(a, dst, map) -> 0 | ERR_CANCEL      render the ROI through map (fxw_run)
//   sincos(x) -> (sin, cos)          f64, |x| <= 1e9 (fdlibm kernels)        [fxm_sincos]
//   log(x)                           f64 ln, x clamped to [DBL_MIN, DBL_MAX]   [fxm_log]
//
// Coordinates: pixel (x, y) is the POINT (x, y) (pixel centres at integers); the image covers
// [-0.5, w - 0.5) x [-0.5, h - 0.5).  For every destination pixel of the ROI run takes n x n
// sample points p = (x - cx + ox_i, y - cy + oy_i) relative to the centre (paint.net's rotated
// grid: oy_i = (i + 1) / (n*n + 1) - 0.5, ox_i = frac(n (i + 1) / (n*n + 1)) - 0.5; n = 1:
// one sample at the centre) and calls map(wr, cur, pt) -> identity?  pt = the point relative to
// the centre on entry, the SOURCE point on return (identity: the sample is the pixel's own
// source pixel, exactly).  A source point is sampled bilinearly from the 2 x 2 pixels around it
// with f32 weights, alpha-weighted (each tap contributes w (c a, a)); taps outside the image
// follow the edge mode (CLAMP / WRAP / REFLECT per tap; PRIMARY / SECONDARY / TRANSPARENT /
// ORIGINAL replace a sample outside the image area by that colour).  The pixel = sum of its
// samples: a = round(S_a / (n n)), c = round(S_c / S_a); a = 0 -> 0.  Pixels farther than
// rin + 0.75 from the centre (rin > 0) are copied unchanged.

use alloc::boxed::Box;

use rt::fx::{cvtd64, cvttd64, maxsd, minsd};
use simd::{Simd, Sse2};

use crate::*;

pub const FXW_CLAMP: i32 = 0; // clamp taps to the image
pub const FXW_WRAP: i32 = 1; // tile the image
pub const FXW_REFLECT: i32 = 2; // mirror (period 2n, edge pixels repeated)
pub const FXW_PRIMARY: i32 = 3; // samples outside the image = col1
pub const FXW_SECONDARY: i32 = 4; // samples outside the image = col2
pub const FXW_TRANSPARENT: i32 = 5; // samples outside the image = transparent
pub const FXW_ORIGINAL: i32 = 6; // samples outside the image = the destination's own pixel
pub const FXW_NEDGE: i32 = 7;

const LO: f64 = -1073741824.0;
const HI: f64 = 1073741824.0;
const SIGN: u64 = 0x8000_0000_0000_0000;

#[inline(always)]
fn neg(v: f64) -> f64 {
    f64::from_bits(v.to_bits() ^ SIGN)
}

/// the aux of prepare: the bounds x0, y0, x1, y1
pub struct Bounds(pub [i32; 4]);

/// the bounding box of the selection (clipped to the image) or the whole image  [fxw_prepare]
pub fn prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let (w, h) = (a.src.w, a.src.h);
    let mut b = [0, 0, w, h];
    if let Some(m) = a.sel {
        if let Some(bb) = m.bbox() {
            let c = [bb[0].max(0), bb[1].max(0), bb[2].min(w), bb[3].min(h)];
            if c[0] < c[2] && c[1] < c[3] {
                b = c;
            }
        }
    }
    Ok(Some(Box::new(Bounds(b))))
}

/// floor of an f64 as i64 (|x| <= 2^30 after clamping)
#[inline(always)]
fn floor64(x: f64) -> i64 {
    let t = cvttd64(x);
    if t as f64 > x { t - 1 } else { t }
}

type V = <Sse2 as Simd>::V;

/// straight pixel -> 4 x f32 (b a, g a, r a, a)  [PX2PM]
#[inline(always)]
fn px2pm(p: u32) -> V {
    let s = Sse2;
    let z = s.zero();
    lanes2pm(s.unpacklo16(s.unpacklo8(s.from_i32(p as i32), z), z))
}

/// a pixel as 4 x i32 lanes (b, g, r, a) -> 4 x f32 (b a, g a, r a, a)  [PX2PM]
#[inline(always)]
fn lanes2pm(p: V) -> V {
    let s = Sse2;
    let v = s.cvt_i32_f32(p);
    let a = s.shuf32::<0xFF>(v);
    let m012 = s.load_i32(&[-1, -1, -1, 0]);
    let one3 = s.load_i32(&[0, 0, 0, 0x3F80_0000]);
    s.mulf(s.or(s.and(v, m012), one3), a)
}

/// the bilinear mix of the 4 premultiplied taps (00, 10, 01, 11) with weights wt  [.ws_mix]
#[inline(always)]
fn mix(t0: V, t1: V, t2: V, t3: V, wt: &[f32; 4]) -> V {
    let s = Sse2;
    let acc = s.addf(s.zero(), s.mulf(t0, s.splatf(wt[0])));
    let acc = s.addf(acc, s.mulf(t1, s.splatf(wt[1])));
    let acc = s.addf(acc, s.mulf(t2, s.splatf(wt[2])));
    s.addf(acc, s.mulf(t3, s.splatf(wt[3])))
}

/// the WARP record of one render call
pub struct Warp<'a> {
    pub src: &'a Surface,
    pub w: i32,
    pub h: i32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub job: Option<&'a (dyn Cancel + Sync)>,
    /// quality 1..5: n x n samples per pixel
    pub q: i32,
    pub edge: i32,
    pub col1: u32,
    pub col2: u32,
    pub cx: f64,
    pub cy: f64,
    /// radius of influence (0 = map every pixel)
    pub rin: f64,
    /// default radius = min(bounds w, h) / 2
    pub rdef: f64,
    pub bx0: i32,
    pub by0: i32,
    pub bx1: i32,
    pub by1: i32,
    /// effect parameters (WR_P)
    pub p: [f64; 16],
    pub seed: u32,
    cv1: [u32; 2],
}

/// the pixel and sample a map function is called for (WR_CURX / CURY / CURS)
#[derive(Clone, Copy)]
pub struct Cur {
    pub x: i32,
    pub y: i32,
    pub s: i32,
}

#[inline(always)]
fn clip0(v: i32, lim: i32) -> i32 {
    if v < 0 {
        0
    } else if v > lim {
        lim
    } else {
        v
    }
}

/// edge index of a tap coordinate (CLAMP / WRAP / REFLECT)  [EDGE_IDX]
#[inline(always)]
fn edge_idx(v: i64, n: i64, mode: i32) -> i64 {
    match mode {
        FXW_WRAP => {
            let r = rem(v, n);
            if r < 0 { r + n } else { r }
        }
        FXW_REFLECT => {
            let n2 = n + n;
            let mut r = rem(v, n2);
            if r < 0 {
                r += n2;
            }
            if r >= n { n2 - 1 - r } else { r }
        }
        _ => {
            if v < 0 {
                0
            } else if v >= n {
                n - 1
            } else {
                v
            }
        }
    }
}

/// the edge indices of the taps v and v + 1 (EDGE_IDX twice; perf-fx: one remainder, none
/// for v within one period below or above)
#[inline(always)]
fn edge_pair(v: i64, n: i64, mode: i32) -> (i64, i64) {
    match mode {
        FXW_WRAP => {
            // r = v mod n in [0, n) (= EDGE_IDX), (v + 1) mod n follows from it
            let r = if (v as u64) < n as u64 {
                v
            } else if v < 0 && v >= -n {
                v + n
            } else {
                let r = rem(v, n);
                if r < 0 { r + n } else { r }
            };
            (r, if r + 1 == n { 0 } else { r + 1 })
        }
        FXW_REFLECT => {
            let n2 = n + n;
            let m = if (v as u64) < n2 as u64 {
                v
            } else if v < 0 && v >= -n2 {
                v + n2
            } else {
                let m = rem(v, n2);
                if m < 0 { m + n2 } else { m }
            };
            let m1 = if m + 1 == n2 { 0 } else { m + 1 };
            (if m >= n { n2 - 1 - m } else { m }, if m1 >= n { n2 - 1 - m1 } else { m1 })
        }
        _ => (edge_idx(v, n, mode), edge_idx(v + 1, n, mode)),
    }
}

/// v % n (truncating, n >= 1): perf-fx: a 32-bit idiv when both fit (the taps are clamped to
/// +-(2^30 + 1), so always for images below 2^30 pixels; the same remainder as the asm's
/// 64-bit idiv)
#[inline(always)]
fn rem(v: i64, n: i64) -> i64 {
    if v as i32 as i64 == v && n as i32 as i64 == n { ((v as i32) % (n as i32)) as i64 } else { v % n }
}

impl<'a> Warp<'a> {
    /// the record from the arguments: ROI clipped to the image, bounds (aux of prepare or the
    /// image), rdef, centre of the bounds, q = 1, edge CLAMP, rin = 0  [fxw_init]
    pub fn new(a: &FxArgs<'a>) -> Warp<'a> {
        let (w, h) = (a.src.w, a.src.h);
        let mut b = [0, 0, w, h];
        if let Some(Bounds(c)) = a.aux::<Bounds>() {
            if c[0] >= 0 && c[1] >= 0 && c[0] < c[2] && c[1] < c[3] && c[2] <= w && c[3] <= h {
                b = *c;
            }
        }
        let rd = (b[2] - b[0]).min(b[3] - b[1]);
        Warp {
            src: a.src,
            w,
            h,
            x0: clip0(a.x0, w),
            y0: clip0(a.y0, h),
            x1: clip0(a.x1, w),
            y1: clip0(a.y1, h),
            job: a.job,
            q: 1,
            edge: FXW_CLAMP,
            col1: 0,
            col2: 0,
            cx: (b[0] + b[2] - 1) as f64 * 0.5,
            cy: (b[1] + b[3] - 1) as f64 * 0.5,
            rin: 0.0,
            rdef: rd as f64 * 0.5,
            bx0: b[0],
            by0: b[1],
            bx1: b[2],
            by1: b[3],
            p: [0.0; 16],
            seed: 0,
            cv1: [0; 2],
        }
    }

    /// centre from an FP_PAN value (-1 = left / top edge of the bounds, 1 = right / bottom;
    /// clamped to -2 .. 2): cx = bx0 - 0.5 + bw (1 + x) / 2  [fxw_center]
    pub fn center(&mut self, x: f64, y: f64) {
        let x = minsd(maxsd(x, -2.0), 2.0);
        let y = minsd(maxsd(y, -2.0), 2.0);
        self.cx = (x + 1.0) * (self.bx1 - self.bx0) as f64 * 0.5 + (self.bx0 as f64 - 0.5);
        self.cy = (y + 1.0) * (self.by1 - self.by0) as f64 * 0.5 + (self.by0 as f64 - 0.5);
    }

    #[inline(always)]
    fn spx(&self, x: i64, y: i64) -> u32 {
        px(&self.src.px[y as usize * self.src.stride as usize..], x as usize)
    }

    /// the 4 taps (x0 y0, x1 y0, x0 y1, x1 y1) of a sample whose 2 x 2 pixels are not all
    /// inside, through the edge mode  [wr_sample .ws_slow]
    #[inline(always)]
    fn edge_taps(&self, ix: i64, iy: i64, mode: i32) -> [u32; 4] {
        let (w, h) = (self.w as i64, self.h as i64);
        let (x0, x1) = edge_pair(ix, w, mode);
        let (y0, y1) = edge_pair(iy, h, mode);
        [self.spx(x0, y0), self.spx(x1, y0), self.spx(x0, y1), self.spx(x1, y1)]
    }

    /// premultiplied bilinear sample at the absolute point (sx, sy) (weights sum 1)  [wr_sample]
    /// (perf-fx: inline in the sample loop; a call cost the Win64 build 8 pushes + 3 xmm saves
    /// and the vector result through memory per sample)
    #[inline(always)]
    fn sample(&self, cur: Cur, sx: f64, sy: f64) -> V {
        let sx = minsd(maxsd(sx, LO), HI);
        let sy = minsd(maxsd(sy, LO), HI);
        let mut mode = self.edge;
        if mode >= FXW_PRIMARY {
            let out = sx < -0.5 || sy < -0.5 || !(sx < self.w as f64 - 0.5) || !(sy < self.h as f64 - 0.5);
            if out {
                return match mode {
                    FXW_PRIMARY => px2pm(self.cv1[0]),
                    FXW_SECONDARY => px2pm(self.cv1[1]),
                    FXW_ORIGINAL => px2pm(self.spx(cur.x as i64, cur.y as i64)),
                    _ => Sse2.zero(),
                };
            }
            mode = FXW_CLAMP;
        }
        let ix = floor64(sx);
        let fx = (sx - ix as f64) as f32;
        let iy = floor64(sy);
        let fy = (sy - iy as f64) as f32;
        let wt = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
        let (w, h) = (self.w as i64, self.h as i64);
        if (ix as u64) < (w - 1) as u64 && (iy as u64) < (h - 1) as u64 {
            // perf-fx: the 2 pixels of each row with one 8-byte load (movq, as the asm), kept
            // in registers
            let s = Sse2;
            let z = s.zero();
            let st = self.src.stride as usize;
            let o = iy as usize * st + ix as usize * 4;
            let r0 = s.widen_px(s.load_raw(&self.src.px[o..]));
            let r1 = s.widen_px(s.load_raw(&self.src.px[o + st..]));
            mix(
                lanes2pm(s.unpacklo16(r0, z)),
                lanes2pm(s.unpackhi16(r0, z)),
                lanes2pm(s.unpacklo16(r1, z)),
                lanes2pm(s.unpackhi16(r1, z)),
                &wt,
            )
        } else {
            let t = self.edge_taps(ix, iy, mode);
            mix(px2pm(t[0]), px2pm(t[1]), px2pm(t[2]), px2pm(t[3]), &wt)
        }
    }

    /// render the ROI through map -> 0 | ERR_CANCEL  [fxw_run]
    pub fn run(&mut self, dst: &mut Dst, mut map: impl FnMut(&Warp, Cur, &mut [f64; 2]) -> bool) -> i32 {
        let n = self.q.clamp(1, 5);
        self.q = n;
        let ns = (n * n) as usize;
        let mut ofs = [[0f64; 2]; 25];
        if ns != 1 {
            let d = ns as i32 as f64 + 1.0;
            for (i, o) in ofs.iter_mut().enumerate().take(ns) {
                let y = (i as i32 + 1) as f64 / d;
                let mut x = y * n as f64;
                x -= rt::fx::cvttd(x) as f64;
                *o = [x - 0.5, y - 0.5];
            }
        }
        self.cv1 = [self.col1, self.col2];
        if self.edge as u32 >= FXW_NEDGE as u32 {
            self.edge = FXW_CLAMP;
        }
        let rlim = if self.rin > 0.0 {
            let r = self.rin + 0.75;
            r * r
        } else {
            -1.0
        };
        let me: &Warp = self;
        for y in me.y0..me.y1 {
            if me.job.is_some_and(|j| j.cancelled()) {
                return ERR_CANCEL;
            }
            let dy = y as f64 - me.cy;
            let dx0 = dst.x0;
            let drow = dst.row(y);
            for x in me.x0..me.x1 {
                let dx = x as f64 - me.cx;
                let o = (x - dx0) as usize;
                if !(rlim < 0.0) && !(dx * dx + dy * dy <= rlim) {
                    put(drow, o, me.spx(x as i64, y as i64));
                    continue;
                }
                // perf-fx: first the source points of every sample (map is a pure function of
                // its arguments), then the samples: the taps of a pixel's samples are fetched
                // close together, so their cache misses overlap; same sums in the same order
                let mut sp = [[0f64; 2]; 25];
                let mut own = [false; 25];
                for (s, of) in ofs.iter().enumerate().take(ns) {
                    let mut pt = [of[0] + dx, of[1] + dy];
                    own[s] = map(me, Cur { x, y, s: s as i32 }, &mut pt);
                    sp[s] = [pt[0] + me.cx, pt[1] + me.cy];
                }
                let mut acc = Sse2.zero();
                for s in 0..ns {
                    let v = if own[s] {
                        px2pm(me.spx(x as i64, y as i64))
                    } else {
                        me.sample(Cur { x, y, s: s as i32 }, sp[s][0], sp[s][1])
                    };
                    acc = Sse2.addf(acc, v);
                }
                put(drow, o, finish(acc, ns as i32));
            }
        }
        0
    }
}

/// sum (S_b, S_g, S_r, S_a) of n samples -> straight pixel: rint(S_c / S_a), rint(S_a / n),
/// saturated to i16, then to u8  [wr_finish]
#[inline]
fn finish(acc: V, n: i32) -> u32 {
    let s = Sse2;
    let mut t = [0f32; 4];
    s.store_f32(&mut t, acc);
    let sa = t[3];
    if !(sa > 0.0) {
        return 0;
    }
    let d = s.load_f32(&[sa, sa, sa, n as f32]);
    let v = s.cvtr_f32_i32(s.divf(acc, d));
    let q = s.packs32(v, v);
    let p = s.to_i32(s.packus16(q, q)) as u32;
    if p < 0x0100_0000 { 0 } else { p }
}

// ---------------------------------------------------------------- f64 maths

const PIO2_1: f64 = 1.57079632673412561417e+00;
const PIO2_1T: f64 = 6.07710050650619224932e-11;
const S1: f64 = -1.66666666666666324348e-01;
const S2: f64 = 8.33333333332248946124e-03;
const S3: f64 = -1.98412698298579493134e-04;
const S4: f64 = 2.75573137070700676789e-06;
const S5: f64 = -2.50507602534068634195e-08;
const S6: f64 = 1.58969099521155010221e-10;
const C1: f64 = 4.16666666666666019037e-02;
const C2: f64 = -1.38888888888741095749e-03;
const C3: f64 = 2.48015872894767294178e-05;
const C4: f64 = -2.75573143513906633035e-07;
const C5: f64 = 2.08757232129817482790e-09;
const C6: f64 = -1.13596475577881948265e-11;

/// (sin x, cos x): k = rint(x 2/pi), Cody-Waite reduction, fdlibm kernels; x clamped to
/// +-1e9 (NaN -> 0)  [fxm_sincos]
pub fn sincos(x: f64) -> (f64, f64) {
    let x = if x.is_nan() { 0.0 } else { minsd(maxsd(x, -1.0e9), 1.0e9) };
    let k = cvtd64(x * 0.63661977236758134308);
    let kd = k as f64;
    let mut r = x - kd * PIO2_1;
    r -= kd * PIO2_1T;
    let z = r * r;
    let ps = ((((S6 * z + S5) * z + S4) * z + S3) * z + S2) * z + S1;
    let s = ps * z * r + r;
    let pc = ((((C6 * z + C5) * z + C4) * z + C3) * z + C2) * z + C1;
    let c = (1.0 - z * 0.5) + pc * z * z;
    match k & 3 {
        1 => (c, neg(s)),
        2 => (neg(s), neg(c)),
        3 => (neg(c), s),
        _ => (s, c),
    }
}

const LN2HI: f64 = 6.93147180369123816490e-01;
const LN2LO: f64 = 1.90821492927058770002e-10;
const LG1: f64 = 6.666666666666735130e-01;
const LG2: f64 = 3.999999999940941908e-01;
const LG3: f64 = 2.857142874366239149e-01;
const LG4: f64 = 2.222219843214978396e-01;
const LG5: f64 = 1.818357216161805012e-01;
const LG6: f64 = 1.531383769920937332e-01;
const LG7: f64 = 1.479819860511658591e-01;

/// ln x (fdlibm e_log.c); x clamped to [DBL_MIN, DBL_MAX] (NaN, 0, negatives -> ln DBL_MIN)
/// [fxm_log]
pub fn log(x: f64) -> f64 {
    let x = minsd(maxsd(x, 2.2250738585072014e-308), 1.7976931348623157e+308);
    let b = x.to_bits();
    let mut k = (b >> 52) as i32 - 1023;
    let mut m = b & 0x000F_FFFF_FFFF_FFFF;
    if m > 0x0006_A09E_667F_3BCD {
        m |= 0x3FE0_0000_0000_0000;
        k += 1;
    } else {
        m |= 0x3FF0_0000_0000_0000;
    }
    let f = f64::from_bits(m) - 1.0;
    let s = f / (f + 2.0);
    let z = s * s;
    let w = z * z;
    let t1 = ((LG6 * w + LG4) * w + LG2) * w;
    let t2 = (((LG7 * w + LG5) * w + LG3) * w + LG1) * z;
    let r = t2 + t1;
    let hfsq = f * f * 0.5;
    let kd = k as f64;
    let a = (hfsq + r) * s + kd * LN2LO;
    let b2 = hfsq - a - f;
    kd * LN2HI - b2
}
