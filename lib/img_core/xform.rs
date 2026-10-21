// xform -- affine resampling (move / rotate / scale / skew) and 2 x 3 matrix helpers
// [src/img/xform.s]
//
//   affine(src, dst, m, filter, flags, clip) -> 0 / ERR_*                         [rs_affine]
//        dst(x, y) = src sampled at m * (x + 0.5, y + 0.5) for every dst pixel inside clip
//        ([x0, y0, x1, y1] exclusive, or None = all of dst); pixels outside clip are untouched
//   Affine::prepare(src, dst, m, filter, flags, clip) -> Result<Affine, i32>
//        validate; dst is only measured                                 [rs_affine_prepare]
//   xa.rows(band, y0, y1) -> 0           dst rows y0 .. y1-1 (clipped) into a Band of dst;
//                                        disjoint bands may run on several threads
//                                                                          [rs_affine_rows]
//   Matrix, Matrix::IDENTITY, m.apply(x, y), sincos(angle)    (gfx::xform: xf_identity,
//                                        xf_apply, xf_sincos)
//   mul(a, b) -> Matrix                  a * b (apply b first, then a)              [xf_mul]
//   invert(m) -> Option<Matrix>          None: singular or not finite            [xf_invert]
//   translate(&mut m, tx, ty)            m = m * T(tx, ty)                      [xf_translate]
//   scale(&mut m, sx, sy)                m = m * S(sx, sy)                          [xf_scale]
//   rotate(&mut m, angle, cx, cy)        m = m * R(angle about cx, cy)             [xf_rotate]
//   CUBIC_LUT                            Catmull-Rom taps -1, 0, 1, 2 for phase p / 256 (i16,
//                                        sum 16384)                            [xf_cubic_lut]
//
// Matrix {a, b, c, d, e, f}: x' = a x + b y + c, y' = d x + e y + f.  affine maps DESTINATION
// pixel centres to continuous SOURCE coordinates (source pixel (i, j) covers [i, i + 1) x
// [j, j + 1)); to draw an image transformed by a forward matrix F, pass invert(F).  Angles in
// radians; with y pointing down a positive angle turns clockwise on screen: R = {cos, -sin,
// 0, sin, cos, 0}.
//
// Sampling (per destination pixel, xc = x + 0.5, yc = y + 0.5, all f64 in this order):
//   qx = a xc + (b yc + c),  qy = d xc + (e yc + f)
//   RS_NEAREST     pixel (floor(clamp(qx)), floor(clamp(qy))), copied (format converted with
//                  gfx::straight premul / unpremul when src and dst formats differ)
//   RS_BILINEAR    U = floor(clamp((qx - 0.5) * 256 + 0.5)) (24.8 fixed point, same for V);
//                  x0 = U >> 8, phase fx = U & 255.  Phase 0 in both axes: the pixel (x0, y0)
//                  is copied exactly (like nearest).  Else the working values of the 2 x 2
//                  pixels (resample::to_work: premultiplied, linear light with RS_GAMMA) are
//                  mixed: wx = fx * 64: T = (P00 (16384 - wx) + P10 wx + 8192) >> 14, B
//                  likewise, P = (T (16384 - wy) + B wy + 8192) >> 14, then from_work.
//   RS_BICUBIC     same phases, 4 x 4 Catmull-Rom taps x0 - 1 .. x0 + 2 with the weights
//                  CUBIC_LUT[phase] (exact copy at phase 0); rows first:
//                  R = sat16((sum w P + 8192) >> 14), P = sat16((sum w R + 8192) >> 14)
//   RS_MULTISAMPLE 16 bilinear samples at xc = x + o_s, yc = y + o_t, o = 1/8, 3/8, 5/8, 7/8
//                  (no exact-copy shortcut), P = (sum of the 16 sample P + 8) >> 4
//   clamp(v) limits v to +-2^30 (NaN -> -2^30).  An integer translation (a = e = 1, b = d = 0,
//   c and f integers below 2^30) with NEAREST / BILINEAR / BICUBIC gives exactly the copies
//   above and takes a row-copy path.  Edge modes (flags bits 8..9): outside pixels are
//   transparent 0 (XF_TRANSPARENT), the nearest edge pixel (XF_CLAMP), j mod n (XF_WRAP) or
//   mirrored with period 2n, edge pixels repeated (XF_REFLECT).
//
// Threads: no global state; rows() allocates about 17 KB of scratch per call.  The 2 x 2 /
// 4 x 4 mixing keeps the assembly's SSE2 layout (one output per register).

use alloc::vec;
use alloc::vec::Vec;

use gfx::straight::{premul_inplace, unpremul_inplace};
use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use rt::fx::{cvttd, cvttd64, maxsd, minsd};
use simd::{Kernel, Simd, Sse2};

pub use gfx::xform::{Matrix, sincos};

use crate::ops::Band;
use crate::resample::{RS_BICUBIC, RS_BILINEAR, RS_MULTISAMPLE, RS_NEAREST, RS_WONE, from_work, to_work};
use crate::{ERR_DIMS, ERR_UNSUPPORTED};

pub const XF_TRANSPARENT: u32 = 0; // outside the source = transparent black
pub const XF_CLAMP: u32 = 1; // repeat the edge pixels
pub const XF_WRAP: u32 = 2; // tile
pub const XF_REFLECT: u32 = 3; // mirror (edge pixels duplicated)
pub const XF_EDGE_SHIFT: u32 = 8;
pub const XF_EDGE_MASK: u32 = 0x300;

const CH_BIL: usize = 256; // outputs per chunk
const CH_BIC: usize = 64;
const CH_MS: usize = 16;
const LO: f64 = -1073741824.0; // -2^30
const HI: f64 = 1073741824.0;
const MSOFF: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

// ------------------------------------------------------------------ matrix helpers

/// a * b (b applied first)  [xf_mul]
pub fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    Matrix {
        a: a.a * b.a + a.b * b.d,
        b: a.a * b.b + a.b * b.e,
        c: a.a * b.c + a.b * b.f + a.c,
        d: a.d * b.a + a.e * b.d,
        e: a.d * b.b + a.e * b.e,
        f: a.d * b.c + a.e * b.f + a.f,
    }
}

/// the inverse of m; None when the determinant is 0 or not finite  [xf_invert]
pub fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m.a * m.e - m.b * m.d;
    if !(det - det == 0.0) || det == 0.0 {
        return None;
    }
    let r = 1.0 / det;
    let (a, b, d, e) = (m.e * r, -m.b * r, -m.d * r, m.a * r);
    Some(Matrix { a, b, c: -(a * m.c + b * m.f), d, e, f: -(d * m.c + e * m.f) })
}

/// m = m * T(tx, ty): c += a tx + b ty, f += d tx + e ty  [xf_translate]
pub fn translate(m: &mut Matrix, tx: f64, ty: f64) {
    m.c = m.a * tx + m.b * ty + m.c;
    m.f = m.d * tx + m.e * ty + m.f;
}

/// m = m * S(sx, sy)  [xf_scale]
pub fn scale(m: &mut Matrix, sx: f64, sy: f64) {
    m.a = m.a * sx;
    m.d = m.d * sx;
    m.b = m.b * sy;
    m.e = m.e * sy;
}

/// m = m * T(cx, cy) * R(angle) * T(-cx, -cy)  [xf_rotate]
pub fn rotate(m: &mut Matrix, angle: f64, cx: f64, cy: f64) {
    translate(m, cx, cy);
    let (s, c) = sincos(angle);
    *m = mul(m, &Matrix { a: c, b: -s, c: 0.0, d: s, e: c, f: 0.0 });
    translate(m, -cx, -cy);
}

// ------------------------------------------------------------------ sampling helpers

// clamp to +-2^30 (maxpd / minpd: NaN -> -2^30)
#[inline(always)]
fn clampd(v: f64) -> f64 {
    minsd(maxsd(v, LO), HI)
}

// floor of a clamped value (cvttpd2dq, then one step down where it rounded up)
#[inline(always)]
fn floord(v: f64) -> i32 {
    let t = cvttd(v);
    if v < t as f64 { t - 1 } else { t }
}

// XF_UV: 24.8 fixed-point source coordinate
#[inline(always)]
fn uv(q: f64) -> i32 {
    floord(clampd((q - 0.5) * 256.0 + 0.5))
}

// af_map: source index of index j on an axis of n pixels, -1 = outside (transparent)
#[inline]
fn emap(j: i32, n: i32, mode: u32) -> i32 {
    if (j as u32) < n as u32 {
        return j;
    }
    match mode {
        XF_CLAMP => {
            if j < 0 {
                0
            } else {
                n - 1
            }
        }
        XF_WRAP => {
            let r = j % n;
            if r < 0 { r + n } else { r }
        }
        XF_REFLECT => {
            let n2 = n + n;
            let mut r = j % n2;
            if r < 0 {
                r += n2;
            }
            if (r as u32) < n as u32 { r } else { n2 - r - 1 }
        }
        _ => -1,
    }
}

// the per-call context of rows() (AF_*)
struct Ctx<'a> {
    perspective: Option<[f64; 3]>,
    y: f64,
    a: f64,
    d: f64,
    rowx: f64, // b yc + c of the current row
    rowy: f64, // e yc + f
    rows: [[f64; 2]; 4], // the rows of the multisample offsets
    spx: &'a [u8],
    sstr: usize,
    sw: i32,
    sh: i32,
    edge: u32,
    sfmt: u8,
    dfmt: u8,
    flags: u32,
    tx: i32, // integer translation
    ty: i32,
}

impl Ctx<'_> {
    /// Inverse homogeneous mapping. Points behind the projection plane are transparent.
    fn mapped(&self, x: f64, y: f64, row: [f64; 2]) -> Option<[f64; 2]> {
        let (mut sx, mut sy) = (self.a * x + row[0], self.d * x + row[1]);
        if let Some([g, h, i]) = self.perspective {
            let z = g * x + h * y + i;
            if !z.is_finite() || z <= 1e-9 { return None; }
            sx /= z;
            sy /= z;
        }
        if !sx.is_finite() || !sy.is_finite() { None } else { Some([sx, sy]) }
    }
    // source pixel (x, y), inside
    #[inline(always)]
    fn px(&self, x: i32, y: i32) -> u32 {
        let o = y as usize * self.sstr + x as usize * 4;
        match self.spx[o..o + 4].try_into() {
            Ok(b) => u32::from_le_bytes(b),
            Err(_) => 0,
        }
    }

    // af_fetch: source pixel (x, y) after the edge mode (0 outside)
    fn fetch(&self, x: i32, y: i32) -> u32 {
        let mx = emap(x, self.sw, self.edge);
        if mx < 0 {
            return 0;
        }
        let my = emap(y, self.sh, self.edge);
        if my < 0 {
            return 0;
        }
        self.px(mx, my)
    }

    // a fetched pixel when (x, y) may be outside
    #[inline(always)]
    fn at(&self, x: i32, y: i32) -> u32 {
        if (x as u32) < self.sw as u32 && (y as u32) < self.sh as u32 { self.px(x, y) } else { self.fetch(x, y) }
    }

    // format conversion of a dst row part in place
    fn conv(&self, b: &mut [u8]) {
        if self.sfmt != self.dfmt {
            if self.sfmt == FMT_ARGB { premul_inplace(b) } else { unpremul_inplace(b) }
        }
    }
}

// scratch of one rows() call
struct Scr {
    raw: Vec<u8>,  // raw source pixels (1024 x u32)
    wk: Vec<i16>,  // their working values
    pout: Vec<i16>, // results (256 x 4 i16, + room for one)
    exr: Vec<u8>,  // exact copies (256 x u32)
    exf: Vec<u8>,  // exact flags
    ph: Vec<u32>,  // phases fx | fy << 16
    nex: usize,    // exact outputs of the chunk
}

#[inline(always)]
fn put(b: &mut [u8], i: usize, v: u32) {
    b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
}

// af_near: nearest, dst[x .. x + n) of the current row, format converted
fn near(c: &Ctx, drow: &mut [u8], x: i32, n: usize) {
    let d = &mut drow[x as usize * 4..(x as usize + n) * 4];
    for (i, o) in d.chunks_exact_mut(4).enumerate() {
        let xc = (x + i as i32) as f64 + 0.5;
        let Some([sx, sy]) = c.mapped(xc, c.y, [c.rowx, c.rowy]) else { o.fill(0); continue };
        let sx = floord(clampd(sx));
        let sy = floord(clampd(sy));
        o.copy_from_slice(&c.at(sx, sy).to_le_bytes());
    }
    c.conv(d);
}

// af_trans_row: integer translation, dst[x0 .. x1) of row y (copies, the edge mode for the
// parts outside the source, then the format conversion)
fn trans_row(c: &Ctx, drow: &mut [u8], x0: i32, x1: i32, y: i32) {
    let sy = y + c.ty;
    let sr = emap(sy, c.sh, c.edge);
    if sr < 0 {
        drow[x0 as usize * 4..x1 as usize * 4].fill(0);
        return;
    }
    let mut xa = x0.max(c.tx.wrapping_neg());
    let mut xb = x1.min(c.sw - c.tx);
    if xa >= xb {
        xa = x0; // nothing inside: every column takes the slow path
        xb = x0;
    }
    if xb > xa {
        let s0 = sr as usize * c.sstr + (xa + c.tx) as usize * 4;
        let n = (xb - xa) as usize * 4;
        drow[xa as usize * 4..xa as usize * 4 + n].copy_from_slice(&c.spx[s0..s0 + n]);
    }
    for x in (x0..xa).chain(xb..x1) {
        put(drow, x as usize, c.fetch(x + c.tx, sy));
    }
    c.conv(&mut drow[x0 as usize * 4..x1 as usize * 4]);
}

// the 2 x 2 source pixels of a sample at (x0, y0) into raw[o .. o + 16]
#[inline(always)]
fn quad(c: &Ctx, raw: &mut [u8], x0: i32, y0: i32) {
    if (x0 as u32) < (c.sw - 1) as u32 && (y0 as u32) < (c.sh - 1) as u32 {
        let o = y0 as usize * c.sstr + x0 as usize * 4;
        raw[0..8].copy_from_slice(&c.spx[o..o + 8]);
        raw[8..16].copy_from_slice(&c.spx[o + c.sstr..o + c.sstr + 8]);
    } else {
        put(raw, 0, c.fetch(x0, y0));
        put(raw, 1, c.fetch(x0 + 1, y0));
        put(raw, 2, c.fetch(x0, y0 + 1));
        put(raw, 3, c.fetch(x0 + 1, y0 + 1));
    }
}

// af_gather_bil / af_gather_bic: phases, exact copies and the 2 x 2 (taps 2) or 4 x 4 (taps
// 4) source pixels of n outputs
fn gather(c: &Ctx, s: &mut Scr, x: i32, n: usize, taps: usize) {
    s.nex = 0;
    for i in 0..n {
        let xc = (x + i as i32) as f64 + 0.5;
        let Some([sx, sy]) = c.mapped(xc, c.y, [c.rowx, c.rowy]) else {
            s.exf[i] = 1;
            s.nex += 1;
            put(&mut s.exr, i, 0);
            continue;
        };
        let u = uv(sx);
        let v = uv(sy);
        let (fx, fy) = (u & 255, v & 255);
        s.ph[i] = (fy << 16 | fx) as u32;
        let (x0, y0) = (u >> 8, v >> 8);
        if fx | fy == 0 {
            s.exf[i] = 1; // exact copy
            s.nex += 1;
            put(&mut s.exr, i, c.at(x0, y0));
            continue;
        }
        s.exf[i] = 0;
        if taps == 2 {
            quad(c, &mut s.raw[i * 16..i * 16 + 16], x0, y0);
            continue;
        }
        let (sx, sy) = (x0 - 1, y0 - 1);
        let raw = &mut s.raw[i * 64..i * 64 + 64];
        if c.sw - 3 > 0 && (sx as u32) < (c.sw - 3) as u32 && c.sh - 3 > 0 && (sy as u32) < (c.sh - 3) as u32 {
            let o = sy as usize * c.sstr + sx as usize * 4;
            for r in 0..4 {
                raw[r * 16..r * 16 + 16].copy_from_slice(&c.spx[o + r * c.sstr..o + r * c.sstr + 16]);
            }
        } else {
            for t in 0..16 {
                put(raw, t, c.fetch(sx + (t & 3) as i32, sy + (t >> 2) as i32));
            }
        }
    }
}

// af_gather_ms: phases and 2 x 2 source pixels of the 16 samples of n outputs
fn gather_ms(c: &Ctx, s: &mut Scr, x: i32, n: usize) {
    s.nex = 0;
    for q in 0..n * 16 {
        let xc = (x + (q >> 4) as i32) as f64 + MSOFF[q & 3];
        let row = &c.rows[(q >> 2) & 3];
        let y = c.y - 0.5 + MSOFF[(q >> 2) & 3];
        let Some([sx, sy]) = c.mapped(xc, y, *row) else {
            s.ph[q] = 0;
            s.raw[q * 16..q * 16 + 16].fill(0);
            continue;
        };
        let u = uv(sx);
        let v = uv(sy);
        s.ph[q] = ((v & 255) << 16 | (u & 255)) as u32;
        quad(c, &mut s.raw[q * 16..q * 16 + 16], u >> 8, v >> 8);
    }
}

// pshufd / pshuflw / pshufhw 0xD8 (XF_PAIR)
#[inline(always)]
fn pair(x: Sse2, v: <Sse2 as Simd>::V) -> <Sse2 as Simd>::V {
    x.shuf16::<0xD8>(x.shuf32::<0xD8>(v))
}

// (16384 - w, w) word pairs of a phase
#[inline(always)]
fn wpair(x: Sse2, f: u32) -> <Sse2 as Simd>::V {
    let w = f << 6;
    x.splat32(w << 16 | (RS_WONE as u32 - w))
}

// one bilinear sample of the 4 working pixels at wk[o .. o + 16]: i32 lanes, not saturated
#[inline(always)]
fn bil(x: Sse2, wk: &[i16], ph: u32) -> <Sse2 as Simd>::V {
    let r = x.splat32(8192);
    let (wx, wy) = (wpair(x, ph & 255), wpair(x, ph >> 16));
    let t = x.sra32::<14>(x.add32(x.madd16(pair(x, x.load_i16(&wk[0..])), wx), r));
    let b = x.sra32::<14>(x.add32(x.madd16(pair(x, x.load_i16(&wk[8..])), wx), r));
    x.sra32::<14>(x.add32(x.madd16(pair(x, x.packs32(t, b)), wy), r))
}

// one row of 4 working pixels . (w0..w3): (sum + 8192) >> 14 as i32 lanes
#[inline(always)]
fn bicrow(x: Sse2, wk: &[i16], w01: <Sse2 as Simd>::V, w23: <Sse2 as Simd>::V) -> <Sse2 as Simd>::V {
    let a = x.madd16(pair(x, x.load_i16(&wk[0..])), w01);
    let b = x.madd16(pair(x, x.load_i16(&wk[8..])), w23);
    x.sra32::<14>(x.add32(x.add32(a, b), x.splat32(8192)))
}

#[inline(always)]
fn lut2(x: Sse2, w: &[i16; 4], k: usize) -> <Sse2 as Simd>::V {
    x.splat32((w[k] as u16 as u32) | (w[k + 1] as u16 as u32) << 16)
}

const K_BIL: u8 = 0;
const K_BIC: u8 = 1;
const K_MS: u8 = 2;

// af_comp_bil / af_comp_bic / af_comp_ms: the results of n outputs (exact ones skipped)
struct CompK<'a> {
    s: &'a mut Scr,
    n: usize,
    kind: u8,
}

impl Kernel for CompK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, _: S) {
        let x = Sse2; // one output per register, as in the assembly
        let CompK { s, n, kind } = self;
        let Scr { wk, pout, exf, ph, .. } = s;
        for i in 0..n {
            let v = match kind {
                K_BIL => {
                    if exf[i] != 0 {
                        continue;
                    }
                    bil(x, &wk[i * 16..i * 16 + 16], ph[i])
                }
                K_BIC => {
                    if exf[i] != 0 {
                        continue;
                    }
                    let (wx, wy) = (&CUBIC_LUT[(ph[i] & 255) as usize], &CUBIC_LUT[((ph[i] >> 16) & 255) as usize]);
                    let (w01, w23) = (lut2(x, wx, 0), lut2(x, wx, 2));
                    let k = &wk[i * 64..i * 64 + 64];
                    let r0 = bicrow(x, &k[0..], w01, w23);
                    let r1 = bicrow(x, &k[16..], w01, w23);
                    let r2 = bicrow(x, &k[32..], w01, w23);
                    let r3 = bicrow(x, &k[48..], w01, w23);
                    let top = x.madd16(pair(x, x.packs32(r0, r1)), lut2(x, wy, 0));
                    let bot = x.madd16(pair(x, x.packs32(r2, r3)), lut2(x, wy, 2));
                    x.sra32::<14>(x.add32(x.add32(top, bot), x.splat32(8192)))
                }
                _ => {
                    let mut sum = x.zero();
                    for q in i * 16..i * 16 + 16 {
                        sum = x.add32(sum, bil(x, &wk[q * 16..q * 16 + 16], ph[q]));
                    }
                    x.sra32::<4>(x.add32(sum, x.splat32(8)))
                }
            };
            x.store_i16(&mut pout[i * 4..], x.packs32(v, v));
        }
    }
}

// af_patch: store the exact copies of the chunk (format converted)
fn patch(c: &Ctx, s: &mut Scr, dst: &mut [u8], n: usize) {
    if s.nex == 0 {
        return;
    }
    c.conv(&mut s.exr[..n * 4]);
    for i in 0..n {
        if s.exf[i] != 0 {
            dst[i * 4..i * 4 + 4].copy_from_slice(&s.exr[i * 4..i * 4 + 4]);
        }
    }
}

// ------------------------------------------------------------------ public entry points

/// a prepared affine resampling of src into a destination of the measured size (the XFORM
/// record); read-only while rows() runs
pub struct Affine<'a> {
    perspective: Option<[f64; 3]>,
    src: &'a Surface,
    m: Matrix,
    filter: u32,
    flags: u32,
    dfmt: u8,
    clip: [i32; 4], // x0, y0, x1, y1 in dst pixels, intersected with dst
}

#[inline]
fn fmt_ok(f: u8) -> bool {
    f == FMT_PARGB || f == FMT_ARGB
}

// c is an integer with |c| < 2^30
fn int30(v: f64) -> Option<i32> {
    let t = cvttd64(v);
    if !(v == t as f64) || t > (1 << 30) - 1 || t < -((1 << 30) - 1) { None } else { Some(t as i32) }
}

impl<'a> Affine<'a> {
    /// validate: ERR_UNSUPPORTED (filter, formats), ERR_DIMS (sizes outside 1 .. 65535)
    /// [rs_affine_prepare]
    pub fn prepare(src: &'a Surface, dst: &Surface, m: &Matrix, filter: u32, flags: u32, clip: Option<[i32; 4]>) -> Result<Affine<'a>, i32> {
        if !matches!(filter, RS_NEAREST | RS_BILINEAR | RS_BICUBIC | RS_MULTISAMPLE) || !fmt_ok(src.fmt) || !fmt_ok(dst.fmt) {
            return Err(ERR_UNSUPPORTED);
        }
        for v in [src.w, src.h, dst.w, dst.h] {
            if (v as u32).wrapping_sub(1) > 65534 {
                return Err(ERR_DIMS);
            }
        }
        let mut cl = [0, 0, dst.w, dst.h];
        if let Some(c) = clip {
            cl = [c[0].max(0), c[1].max(0), c[2].min(dst.w), c[3].min(dst.h)];
        }
        Ok(Affine { perspective: None, src, m: *m, filter, flags, dfmt: dst.fmt, clip: cl })
    }

    /// Prepare an inverse 3x3 homography, in row-major order. Uses the same alpha-aware
    /// filters and edge modes as affine resampling; no intermediate image is allocated.
    pub fn prepare_projective(src: &'a Surface, dst: &Surface, m: &[f64; 9], filter: u32, flags: u32, clip: Option<[i32; 4]>) -> Result<Affine<'a>, i32> {
        if m.iter().any(|v| !v.is_finite()) { return Err(ERR_DIMS); }
        let n = Matrix { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5] };
        let mut out = Self::prepare(src, dst, &n, filter, flags, clip)?;
        out.perspective = Some([m[6], m[7], m[8]]);
        Ok(out)
    }

    /// dst rows y0 .. y1-1 inside the clip into band, which must hold them  [rs_affine_rows]
    pub fn rows(&self, band: &mut Band, y0: i32, y1: i32) -> i32 {
        let [cx0, cy0, cx1, cy1] = self.clip;
        let y0 = y0.max(cy0);
        let y1 = y1.min(cy1);
        if y0 >= y1 || cx0 >= cx1 {
            return 0;
        }
        let (m, filter) = (&self.m, self.filter);
        let mut c = Ctx {
            perspective: self.perspective,
            y: 0.0,
            a: m.a,
            d: m.d,
            rowx: 0.0,
            rowy: 0.0,
            rows: [[0.0; 2]; 4],
            spx: &self.src.px,
            sstr: self.src.stride as usize,
            sw: self.src.w,
            sh: self.src.h,
            edge: (self.flags & XF_EDGE_MASK) >> XF_EDGE_SHIFT,
            sfmt: self.src.fmt,
            dfmt: self.dfmt,
            flags: self.flags,
            tx: 0,
            ty: 0,
        };
        // integer translation?
        let mut itr = false;
        if self.perspective.is_none() && filter != RS_MULTISAMPLE && m.a == 1.0 && m.e == 1.0 && m.b == 0.0 && m.d == 0.0 {
            if let (Some(tx), Some(ty)) = (int30(m.c), int30(m.f)) {
                (c.tx, c.ty, itr) = (tx, ty, true);
            }
        }
        let (chunk, per, kind) = match filter {
            RS_BICUBIC => (CH_BIC, 16, K_BIC),
            RS_MULTISAMPLE => (CH_MS, 64, K_MS),
            _ => (CH_BIL, 4, K_BIL),
        };
        let mut s = Scr { raw: Vec::new(), wk: Vec::new(), pout: Vec::new(), exr: Vec::new(), exf: Vec::new(), ph: Vec::new(), nex: 0 };
        if filter != RS_NEAREST && !itr {
            s = Scr {
                raw: vec![0; 4096],
                wk: vec![0; 4096],
                pout: vec![0; 256 * 4 + 8],
                exr: vec![0; 1024],
                exf: vec![0; 256],
                ph: vec![0; 256],
                nex: 0,
            };
        }
        for y in y0..y1 {
            // row constants [b yc + c, e yc + f]
            let yc = y as f64 + 0.5;
            c.y = yc;
            c.rowx = m.b * yc + m.c;
            c.rowy = m.e * yc + m.f;
            if filter == RS_MULTISAMPLE {
                for (t, r) in c.rows.iter_mut().enumerate() {
                    let yt = y as f64 + MSOFF[t];
                    *r = [m.b * yt + m.c, m.e * yt + m.f];
                }
            }
            let drow = band.row(y);
            if itr {
                trans_row(&c, drow, cx0, cx1, y);
                continue;
            }
            let mut x = cx0;
            while x < cx1 {
                if filter == RS_NEAREST {
                    near(&c, drow, x, (cx1 - x) as usize);
                    break;
                }
                let n = ((cx1 - x) as usize).min(chunk);
                if kind == K_MS {
                    gather_ms(&c, &mut s, x, n);
                } else {
                    gather(&c, &mut s, x, n, if kind == K_BIC { 4 } else { 2 });
                }
                let d = &mut drow[x as usize * 4..(x as usize + n) * 4];
                if s.nex < n {
                    to_work(&mut s.wk, &s.raw, n * per, c.sfmt, c.flags);
                    simd::dispatch(CompK { s: &mut s, n, kind });
                    from_work(d, &s.pout, n, c.dfmt, c.flags);
                }
                patch(&c, &mut s, d, n);
                x += n as i32;
            }
        }
        0
    }
}

/// dst = src resampled through m inside clip (None = all of dst)  [rs_affine]
pub fn affine(src: &Surface, dst: &mut Surface, m: &Matrix, filter: u32, flags: u32, clip: Option<[i32; 4]>) -> i32 {
    let xa = match Affine::prepare(src, dst, m, filter, flags, clip) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let [_, y0, _, y1] = xa.clip;
    xa.rows(&mut Band::whole(dst), y0, y1)
}

// ------------------------------------------------------------------ the bicubic phase table

/// Catmull-Rom taps -1, 0, 1, 2 for phase p / 256 (i16, sum 16384)  [xf_cubic_lut]
pub static CUBIC_LUT: [[i16; 4]; 256] = [
    [0, 16384, 0, 0], [-32, 16384, 32, 0], [-63, 16381, 66, 0], [-94, 16379, 100, -1],
    [-124, 16374, 136, -2], [-154, 16369, 172, -3], [-183, 16361, 210, -4], [-212, 16354, 248, -6],
    [-240, 16345, 287, -8], [-268, 16335, 327, -10], [-295, 16322, 369, -12], [-322, 16309, 411, -14],
    [-349, 16297, 453, -17], [-375, 16282, 497, -20], [-400, 16265, 542, -23], [-425, 16247, 588, -26],
    [-450, 16230, 634, -30], [-474, 16211, 681, -34], [-498, 16191, 729, -38], [-521, 16169, 778, -42],
    [-544, 16146, 828, -46], [-566, 16122, 879, -51], [-588, 16097, 930, -55], [-610, 16071, 983, -60],
    [-631, 16044, 1036, -65], [-651, 16015, 1090, -70], [-672, 15988, 1144, -76], [-691, 15957, 1200, -82],
    [-711, 15926, 1256, -87], [-730, 15894, 1313, -93], [-748, 15861, 1370, -99], [-766, 15827, 1429, -106],
    [-784, 15792, 1488, -112], [-801, 15756, 1548, -119], [-818, 15719, 1608, -125], [-835, 15681, 1670, -132],
    [-851, 15642, 1732, -139], [-866, 15602, 1794, -146], [-882, 15562, 1858, -154], [-897, 15520, 1922, -161],
    [-911, 15478, 1986, -169], [-925, 15433, 2052, -176], [-939, 15390, 2117, -184], [-953, 15345, 2184, -192],
    [-966, 15299, 2251, -200], [-978, 15252, 2319, -209], [-991, 15205, 2387, -217], [-1002, 15155, 2456, -225],
    [-1014, 15106, 2526, -234], [-1025, 15056, 2596, -243], [-1036, 15004, 2667, -251], [-1047, 14953, 2738, -260],
    [-1057, 14900, 2810, -269], [-1066, 14846, 2882, -278], [-1076, 14793, 2955, -288], [-1085, 14737, 3029, -297],
    [-1094, 14681, 3103, -306], [-1102, 14625, 3177, -316], [-1110, 14567, 3252, -325], [-1118, 14509, 3328, -335],
    [-1125, 14450, 3404, -345], [-1133, 14391, 3480, -354], [-1139, 14330, 3557, -364], [-1146, 14270, 3634, -374],
    [-1152, 14208, 3712, -384], [-1158, 14146, 3790, -394], [-1163, 14082, 3869, -404], [-1169, 14019, 3948, -414],
    [-1174, 13955, 4027, -424], [-1178, 13890, 4107, -435], [-1182, 13823, 4188, -445], [-1187, 13758, 4268, -455],
    [-1190, 13691, 4349, -466], [-1194, 13623, 4431, -476], [-1197, 13556, 4512, -487], [-1200, 13486, 4595, -497],
    [-1202, 13417, 4677, -508], [-1205, 13347, 4760, -518], [-1207, 13277, 4843, -529], [-1208, 13205, 4926, -539],
    [-1210, 13134, 5010, -550], [-1211, 13062, 5094, -561], [-1212, 12989, 5178, -571], [-1213, 12916, 5263, -582],
    [-1213, 12842, 5348, -593], [-1214, 12768, 5433, -603], [-1214, 12694, 5518, -614], [-1213, 12618, 5604, -625],
    [-1213, 12542, 5690, -635], [-1212, 12466, 5776, -646], [-1211, 12390, 5862, -657], [-1210, 12312, 5949, -667],
    [-1208, 12235, 6035, -678], [-1207, 12157, 6122, -688], [-1205, 12079, 6209, -699], [-1202, 11998, 6297, -709],
    [-1200, 11920, 6384, -720], [-1197, 11839, 6472, -730], [-1195, 11761, 6559, -741], [-1192, 11680, 6647, -751],
    [-1188, 11599, 6735, -762], [-1185, 11518, 6823, -772], [-1181, 11436, 6911, -782], [-1177, 11354, 7000, -793],
    [-1173, 11272, 7088, -803], [-1169, 11189, 7177, -813], [-1165, 11107, 7265, -823], [-1160, 11023, 7354, -833],
    [-1155, 10939, 7443, -843], [-1150, 10856, 7531, -853], [-1145, 10772, 7620, -863], [-1140, 10687, 7709, -872],
    [-1134, 10602, 7798, -882], [-1128, 10517, 7887, -892], [-1122, 10431, 7976, -901], [-1116, 10346, 8065, -911],
    [-1110, 10260, 8154, -920], [-1104, 10175, 8242, -929], [-1097, 10088, 8331, -938], [-1091, 10002, 8420, -947],
    [-1084, 9915, 8509, -956], [-1077, 9829, 8597, -965], [-1070, 9742, 8686, -974], [-1062, 9653, 8775, -982],
    [-1055, 9567, 8863, -991], [-1047, 9479, 8951, -999], [-1040, 9392, 9040, -1008], [-1032, 9304, 9128, -1016],
    [-1024, 9216, 9216, -1024], [-1016, 9128, 9304, -1032], [-1008, 9040, 9392, -1040], [-999, 8951, 9479, -1047],
    [-991, 8863, 9567, -1055], [-982, 8775, 9653, -1062], [-974, 8686, 9742, -1070], [-965, 8597, 9829, -1077],
    [-956, 8509, 9915, -1084], [-947, 8420, 10002, -1091], [-938, 8331, 10088, -1097], [-929, 8242, 10175, -1104],
    [-920, 8154, 10260, -1110], [-911, 8065, 10346, -1116], [-901, 7976, 10431, -1122], [-892, 7887, 10517, -1128],
    [-882, 7798, 10602, -1134], [-872, 7709, 10687, -1140], [-863, 7620, 10772, -1145], [-853, 7531, 10856, -1150],
    [-843, 7443, 10939, -1155], [-833, 7354, 11023, -1160], [-823, 7265, 11107, -1165], [-813, 7177, 11189, -1169],
    [-803, 7088, 11272, -1173], [-793, 7000, 11354, -1177], [-782, 6911, 11436, -1181], [-772, 6823, 11518, -1185],
    [-762, 6735, 11599, -1188], [-751, 6647, 11680, -1192], [-741, 6559, 11761, -1195], [-730, 6472, 11839, -1197],
    [-720, 6384, 11920, -1200], [-709, 6297, 11998, -1202], [-699, 6209, 12079, -1205], [-688, 6122, 12157, -1207],
    [-678, 6035, 12235, -1208], [-667, 5949, 12312, -1210], [-657, 5862, 12390, -1211], [-646, 5776, 12466, -1212],
    [-635, 5690, 12542, -1213], [-625, 5604, 12618, -1213], [-614, 5518, 12694, -1214], [-603, 5433, 12768, -1214],
    [-593, 5348, 12842, -1213], [-582, 5263, 12916, -1213], [-571, 5178, 12989, -1212], [-561, 5094, 13062, -1211],
    [-550, 5010, 13134, -1210], [-539, 4926, 13205, -1208], [-529, 4843, 13277, -1207], [-518, 4760, 13347, -1205],
    [-508, 4677, 13417, -1202], [-497, 4595, 13486, -1200], [-487, 4512, 13556, -1197], [-476, 4431, 13623, -1194],
    [-466, 4349, 13691, -1190], [-455, 4268, 13758, -1187], [-445, 4188, 13823, -1182], [-435, 4107, 13890, -1178],
    [-424, 4027, 13955, -1174], [-414, 3948, 14019, -1169], [-404, 3869, 14082, -1163], [-394, 3790, 14146, -1158],
    [-384, 3712, 14208, -1152], [-374, 3634, 14270, -1146], [-364, 3557, 14330, -1139], [-354, 3480, 14391, -1133],
    [-345, 3404, 14450, -1125], [-335, 3328, 14509, -1118], [-325, 3252, 14567, -1110], [-316, 3177, 14625, -1102],
    [-306, 3103, 14681, -1094], [-297, 3029, 14737, -1085], [-288, 2955, 14793, -1076], [-278, 2882, 14846, -1066],
    [-269, 2810, 14900, -1057], [-260, 2738, 14953, -1047], [-251, 2667, 15004, -1036], [-243, 2596, 15056, -1025],
    [-234, 2526, 15106, -1014], [-225, 2456, 15155, -1002], [-217, 2387, 15205, -991], [-209, 2319, 15252, -978],
    [-200, 2251, 15299, -966], [-192, 2184, 15345, -953], [-184, 2117, 15390, -939], [-176, 2052, 15433, -925],
    [-169, 1986, 15478, -911], [-161, 1922, 15520, -897], [-154, 1858, 15562, -882], [-146, 1794, 15602, -866],
    [-139, 1732, 15642, -851], [-132, 1670, 15681, -835], [-125, 1608, 15719, -818], [-119, 1548, 15756, -801],
    [-112, 1488, 15792, -784], [-106, 1429, 15827, -766], [-99, 1370, 15861, -748], [-93, 1313, 15894, -730],
    [-87, 1256, 15926, -711], [-82, 1200, 15957, -691], [-76, 1144, 15988, -672], [-70, 1090, 16015, -651],
    [-65, 1036, 16044, -631], [-60, 983, 16071, -610], [-55, 930, 16097, -588], [-51, 879, 16122, -566],
    [-46, 828, 16146, -544], [-42, 778, 16169, -521], [-38, 729, 16191, -498], [-34, 681, 16211, -474],
    [-30, 634, 16230, -450], [-26, 588, 16247, -425], [-23, 542, 16265, -400], [-20, 497, 16282, -375],
    [-17, 453, 16297, -349], [-14, 411, 16309, -322], [-12, 369, 16322, -295], [-10, 327, 16335, -268],
    [-8, 287, 16345, -240], [-6, 248, 16354, -212], [-4, 210, 16361, -183], [-3, 172, 16369, -154],
    [-2, 136, 16374, -124], [-1, 100, 16379, -94], [0, 66, 16381, -63], [0, 32, 16384, -32],
];
