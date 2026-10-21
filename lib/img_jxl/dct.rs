// dct -- JPEG XL VarDCT transforms (dct-inl.h IDCT1DImpl / DCT1DImpl, dec_transforms-inl.h
// TransformToPixels / LowestFrequenciesFromDC; model: tools/jxl_ref.py idct1d ..
// transform_to_pixels, llf_from_dc)                                      [src/img/jxl_dct.s]
//
//   Scr                                     transform scratch (JD_SCRSIZE)
//   jd_idct2d(coef, rows, cols, out, os, scr)   ComputeScaledIDCT< rows, cols >: coef
//                                           (destroyed) -> rows x cols f32 at out (row stride
//                                           os floats)
//   jd_transform(acs, coef, out, os, scr)   TransformToPixels of one varblock
//   jd_llf(acs, dc, dcstride, llf, scr)     LowestFrequenciesFromDC of a multi-block strategy
//   jd_fdct8(src, out, scr)                 ComputeScaledDCT< 8, 8 > of an 8 x 8 block (writer)
//
// f32 arithmetic in the model's order (no fused multiply-add); the column transforms work on
// four columns at once ([f32; 4] lanes, the assembly's SSE vectors), which keeps the
// per-element operation order.

use alloc::vec;
use alloc::vec::Vec;

use crate::tab;
use crate::vdc::{ACSCX, ACSCY};

type V4 = [f32; 4];

const SQRT2: f32 = f32::from_bits(0x3fb504f3);

#[inline(always)]
fn add(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}
#[inline(always)]
fn sub(a: V4, b: V4) -> V4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}
#[inline(always)]
fn muls(a: V4, s: f32) -> V4 {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

/// transform scratch: two 65536-float buffers, the 1D recursion scratch and small blocks
pub struct Scr {
    pub a: Vec<f32>,
    pub b: Vec<f32>,
    t: Vec<V4>,  // recursion scratch (< 3 n vectors)
    tf: Vec<f32>, // scalar recursion scratch (fd1)
    col: Vec<V4>,
    col2: Vec<V4>,
    pub blk: [f32; 64],
    pub blk2: [f32; 64],
}

impl Scr {
    pub fn new() -> Scr {
        Scr::with_size(65536)
    }
    /// transforms of up to n coefficients (jxl-plus: the writer's 64 x 64)
    pub fn with_size(n: usize) -> Scr {
        Scr {
            a: vec![0.0; n],
            b: vec![0.0; n],
            t: vec![[0.0; 4]; 3 * 256 + 16],
            tf: vec![0.0; 4 * 256 + 64],
            col: vec![[0.0; 4]; 256],
            col2: vec![[0.0; 4]; 256],
            blk: [0.0; 64],
            blk2: [0.0; 64],
        }
    }
}

impl Default for Scr {
    fn default() -> Self {
        Scr::new()
    }
}

/// jd_id1: IDCT of n 4-float vectors (src stride ss, dst stride ds in vectors)
fn id1(src: &[V4], ss: usize, dst: &mut [V4], ds: usize, n: usize, tmp: &mut [V4]) {
    if n <= 2 {
        let s0 = src[0];
        if n < 2 {
            dst[0] = s0;
        } else {
            let s1 = src[ss];
            dst[0] = add(s0, s1);
            dst[ds] = sub(s0, s1);
        }
        return;
    }
    let h = n >> 1;
    let (e, rest) = tmp.split_at_mut(h);
    let (o, rest) = rest.split_at_mut(h);
    let (t, next) = rest.split_at_mut(h);
    id1(src, 2 * ss, e, 1, h, next);
    o[0] = muls(src[ss], SQRT2);
    for i in 1..h {
        o[i] = add(src[(2 * i + 1) * ss], src[(2 * i - 1) * ss]);
    }
    id1(o, 1, t, 1, h, next);
    let wc = &tab::WC[h - 2..];
    for i in 0..h {
        let tv = muls(t[i], wc[i]);
        dst[i * ds] = add(tv, e[i]);
        dst[(n - 1 - i) * ds] = sub(e[i], tv);
    }
}

/// jd_idcols: in-place IDCT of every column (ncols multiple of 4) of buf (row stride rs)
fn idcols(buf: &mut [f32], n: usize, ncols: usize, rs: usize, scr_t: &mut [V4], col: &mut [V4], col2: &mut [V4]) {
    let mut c = 0;
    while c < ncols {
        for j in 0..n {
            let o = j * rs + c;
            col[j] = [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]];
        }
        id1(col, 1, col2, 1, n, scr_t);
        for j in 0..n {
            let o = j * rs + c;
            buf[o..o + 4].copy_from_slice(&col2[j]);
        }
        c += 4;
    }
}

/// jd_transpose: dst (cols x rows) = src (rows x cols) transposed
pub fn transpose(src: &[f32], rows: usize, cols: usize, dst: &mut [f32]) {
    for r in 0..rows {
        for c in 0..cols {
            dst[c * rows + r] = src[r * cols + c];
        }
    }
}

/// jd_idct2d: ComputeScaledIDCT< rows, cols > of coef (destroyed) into out (row stride os)
pub fn jd_idct2d(coef: &mut [f32], rows: usize, cols: usize, out: &mut [f32], os: usize, scr: &mut Scr) {
    let Scr { a, b, t, col, col2, .. } = scr;
    let d: &mut [f32] = if rows < cols {
        transpose(coef, rows, cols, a);
        &mut a[..]
    } else {
        coef
    };
    idcols(d, cols, rows, rows, t, col, col2);
    transpose(d, cols, rows, b);
    idcols(b, rows, cols, cols, t, col, col2);
    for r in 0..rows {
        out[r * os..r * os + cols].copy_from_slice(&b[r * cols..r * cols + cols]);
    }
}

/// jd_fd1: DCT1DImpl of n floats (strides ss, ds), unscaled
fn fd1(src: &[f32], ss: usize, dst: &mut [f32], ds: usize, n: usize, tmp: &mut [f32]) {
    if n <= 2 {
        let s0 = src[0];
        if n < 2 {
            dst[0] = s0;
        } else {
            let s1 = src[ss];
            dst[0] = s0 + s1;
            dst[ds] = s0 - s1;
        }
        return;
    }
    let h = n >> 1;
    let wc = &tab::WC[h - 2..];
    let (ab, rest) = tmp.split_at_mut(2 * h);
    for i in 0..h {
        let x = src[i * ss];
        let y = src[(n - 1 - i) * ss];
        ab[i] = x + y;
        ab[h + i] = (x - y) * wc[i];
    }
    let (t01, next) = rest.split_at_mut(2 * h);
    let (t0, t1) = t01.split_at_mut(h);
    fd1(&ab[..h], 1, t0, 1, h, next);
    fd1(&ab[h..], 1, t1, 1, h, next);
    for i in 0..h {
        dst[2 * i * ds] = t0[i];
        let mut v = t1[i];
        if i == 0 {
            v = v * SQRT2 + t1[1];
        } else if i < h - 1 {
            v = v + t1[i + 1];
        }
        dst[(2 * i + 1) * ds] = v;
    }
}

/// jd_fdcols: fdct1d of every column of an n x ncols matrix (scaled by 1 / n)
pub fn fdcols(src: &[f32], n: usize, ncols: usize, dst: &mut [f32], tmp: &mut [f32]) {
    let inv = 1.0f32 / n as i32 as f32;
    for c in 0..ncols {
        fd1(&src[c..], ncols, &mut dst[c..], ncols, n, tmp);
        for k in 0..n {
            let o = k * ncols + c;
            dst[o] = dst[o] * inv;
        }
    }
}

/// jd_gather: dst[r * cols + c] = coef[(r * ys + yo) * 8 + c * xs + xo]
#[inline]
fn gather(coef: &[f32], dst: &mut [f32], rows: usize, cols: usize, yo: usize, xo: usize, ys: usize, xs: usize) {
    let mut k = 0;
    for r in 0..rows {
        let mut i = (r * ys + yo) * 8 + xo;
        for _ in 0..cols {
            dst[k] = coef[i];
            k += 1;
            i += xs;
        }
    }
}

/// jd_dcs4: DCT4x4 / identity DCs: ((b00 +- b01) +- b10) +- b11
fn dcs4(c: &[f32]) -> [f32; 4] {
    let (b00, b01, b10, b11) = (c[0], c[1], c[8], c[9]);
    let s = b00 + b01;
    let d = b00 - b01;
    [(s + b10) + b11, (s - b10) - b11, (d + b10) - b11, (d - b10) + b11]
}

/// jd_transform: TransformToPixels of one varblock (coef destroyed) into out (row stride os)
pub fn jd_transform(acs: u32, coef: &mut [f32], out: &mut [f32], os: usize, scr: &mut Scr) {
    let a = acs as usize;
    if a >= 27 {
        return;
    }
    if tab::ACSLOG2[a] != 0 || a == 0 {
        let rows = ACSCY[a] as usize * 8;
        let cols = ACSCX[a] as usize * 8;
        jd_idct2d(coef, rows, cols, out, os, scr);
        return;
    }
    match a {
        1 | 3 => {
            // DCT4x4 and identity: four 4x4 sub-blocks on the even / odd rows and columns
            let dcs = dcs4(coef);
            for k in 0..4usize {
                let mut blk = [0f32; 64];
                gather(coef, &mut blk, 4, 4, k >> 1, k & 1, 2, 2);
                let o = (k >> 1) * 4 * os + (k & 1) * 4;
                if a == 1 {
                    let mut rs = 0.0f32;
                    for p in 1..16 {
                        rs = rs + blk[p];
                    }
                    rs = rs * f32::from_bits(0x3d80_0000);
                    let center = dcs[k] - rs;
                    for p in 0..16usize {
                        let mut v = blk[p] + center;
                        if p == 5 {
                            v = center;
                        }
                        if p == 0 {
                            v = blk[5] + center;
                        }
                        out[o + (p >> 2) * os + (p & 3)] = v;
                    }
                } else {
                    blk[0] = dcs[k];
                    jd_idct2d(&mut blk, 4, 4, &mut out[o..], os, scr);
                }
            }
        }
        2 => {
            // DCT2x2: butterflies of the top-left 2x2, 4x4 and 8x8 squares (in place)
            let mut n = 1usize;
            while n <= 4 {
                let mut t = [0f32; 64];
                for i in 0..n {
                    for j in 0..n {
                        let k = i * 8 + j;
                        let c00 = coef[k];
                        let c01 = coef[k + n];
                        let c10 = coef[k + n * 8];
                        let c11 = coef[k + n * 8 + n];
                        let s = c00 + c01;
                        let d = c00 - c01;
                        let ti = (2 * i) * 8 + 2 * j;
                        t[ti] = (s + c10) + c11;
                        t[ti + 1] = (s - c10) - c11;
                        t[ti + 8] = (d + c10) - c11;
                        t[ti + 9] = (d - c10) + c11;
                    }
                }
                for r in 0..2 * n {
                    for c in 0..2 * n {
                        coef[r * 8 + c] = t[r * 8 + c];
                    }
                }
                n *= 2;
            }
            for r in 0..8 {
                out[r * os..r * os + 8].copy_from_slice(&coef[r * 8..r * 8 + 8]);
            }
        }
        12 | 13 => {
            // DCT4x8 (12) / DCT8x4 (13): two halves on the even / odd coefficient rows
            let dcs = [coef[0] + coef[8], coef[0] - coef[8]];
            for i in 0..2usize {
                let mut blk = [0f32; 64];
                gather(coef, &mut blk, 4, 8, i, 0, 2, 1);
                blk[0] = dcs[i];
                if a == 13 {
                    jd_idct2d(&mut blk, 8, 4, &mut out[i * 4..], os, scr);
                } else {
                    jd_idct2d(&mut blk, 4, 8, &mut out[i * 4 * os..], os, scr);
                }
            }
        }
        _ => {
            // AFV0..3: 4x4 AFV corner, a 4x4 DCT beside it and a 4x8 DCT on the other half
            let afx = (a - 14) & 1;
            let afy = ((a - 14) >> 1) & 1;
            let (b00, b01, b10) = (coef[0], coef[1], coef[8]);
            let s = b00 + b10;
            let dcs = [(s + b01) * 4.0, s - b01, b00 - b10];
            let mut blk = [0f32; 64];
            gather(coef, &mut blk, 4, 4, 0, 0, 2, 2);
            blk[0] = dcs[0];
            let mut px = [0f32; 16];
            for j in 0..16 {
                let cj = blk[j];
                let bas = &tab::AFVBASIS[j * 16..j * 16 + 16];
                for p in 0..16 {
                    px[p] = px[p] + bas[p] * cj;
                }
            }
            for r in 0..4usize {
                let sr = if afy != 0 { 3 - r } else { r };
                let o = (4 * afy + r) * os + 4 * afx;
                for c in 0..4usize {
                    let sc = if afx != 0 { 3 - c } else { c };
                    out[o + c] = px[sr * 4 + sc];
                }
            }
            let mut blk = [0f32; 64];
            gather(coef, &mut blk, 4, 4, 0, 1, 2, 2);
            blk[0] = dcs[1];
            let o = 4 * afy * os + (afx ^ 1) * 4;
            jd_idct2d(&mut blk, 4, 4, &mut out[o..], os, scr);
            let mut blk = [0f32; 64];
            gather(coef, &mut blk, 4, 8, 1, 0, 2, 1);
            blk[0] = dcs[2];
            let o = 4 * (afy ^ 1) * os;
            jd_idct2d(&mut blk, 4, 8, &mut out[o..], os, scr);
        }
    }
}

/// jd_llf: LowestFrequenciesFromDC: cy x cx DC patch (row stride ds) -> min x max values at
/// out with a row stride of 8 max floats
pub fn jd_llf(acs: u32, dc: &[f32], ds: usize, out: &mut [f32], scr: &mut Scr) {
    let a = acs as usize;
    if a >= 27 {
        return;
    }
    let cy = ACSCY[a] as usize;
    let cx = ACSCX[a] as usize;
    let Scr { a: ba, b: bb, tf, .. } = scr;
    for i in 0..cy {
        for j in 0..cx {
            ba[i * cx + j] = dc[i * ds + j];
        }
    }
    fdcols(ba, cy, cx, bb, tf);
    transpose(bb, cy, cx, ba);
    fdcols(ba, cx, cy, bb, tf);
    let m: &[f32] = if cy < cx {
        transpose(bb, cx, cy, ba);
        &ba[..]
    } else {
        &bb[..]
    };
    let (mn, mx) = if cy <= cx { (cy, cx) } else { (cx, cy) };
    let sa = &tab::RS[mn - 1..];
    let sb = &tab::RS[mx - 1..];
    for i in 0..mn {
        for j in 0..mx {
            out[i * 8 * mx + j] = m[i * mx + j] * sa[i] * sb[j];
        }
    }
}

/// jd_fdct8: dct2d( p, 8, 8 ) = fdct1d( fdct1d( p ).T ): columns, transpose, columns
pub fn jd_fdct8(src: &[f32], out: &mut [f32], scr: &mut Scr) {
    let Scr { a, b, tf, .. } = scr;
    fdcols(src, 8, 8, b, tf);
    transpose(b, 8, 8, a);
    fdcols(a, 8, 8, out, tf);
}
