// etx -- forward transforms of the VarDCT writer (libjxl 0.11.1 enc_transforms-inl.h
// TransformFromPixels, DCFromLowestFrequencies; dct-inl.h ComputeScaledDCT) and the library
// quantization matrices (quant_weights.cc: Matrix / InvMatrix with zero LLF)   [jxl-plus]
//
//   Tx                         per-thread scratch
//   fdct2d(tx, src, stride, rows, cols, out)   ComputeScaledDCT< rows, cols >: the
//                              coefficients with the smaller dimension first (the layout
//                              dct::jd_idct2d inverts)
//   from_pixels(tx, acs, px, stride, out)      TransformFromPixels of one varblock
//   dc_from_llf(tx, acs, block, dc, stride)    DCFromLowestFrequencies
//   to_pixels(tx, acs, coef, out, stride)      TransformToPixels (the decoder's, coef destroyed)
//   Qm::library()              dequantization (1 / w) and quantization (w, LLF 0) tables of
//                              the 17 library encodings
//   size_of(acs) / llf_dims(acs)               coefficients / LLF (rows, cols) of a strategy
//
// jxl-speed: the 2-D DCTs of 8 .. 64 points (forward, and the writer's inverse in
// to_pixels_fast) run as the simd kernels of dctk; the 4-point blocks keep the scalar code here
// (fdct_small / idct_small), which every kernel matches bit for bit.

use alloc::vec;
use alloc::vec::Vec;

use crate::dct::{fdcols, jd_idct2d, jd_transform, transpose, Scr};
use crate::dctk;
use crate::tab;
use crate::vdc::{Qe, ACSCX, ACSCY};

pub const DCT: usize = 0;
pub const IDENTITY: usize = 1;
pub const DCT2X2: usize = 2;
pub const DCT4X4: usize = 3;
pub const DCT16X16: usize = 4;
pub const DCT32X32: usize = 5;
pub const DCT16X8: usize = 6;
pub const DCT8X16: usize = 7;
pub const DCT32X8: usize = 8;
pub const DCT8X32: usize = 9;
pub const DCT32X16: usize = 10;
pub const DCT16X32: usize = 11;
pub const DCT4X8: usize = 12;
pub const DCT8X4: usize = 13;
pub const AFV0: usize = 14;
pub const DCT64X64: usize = 18;
pub const DCT64X32: usize = 19;
pub const DCT32X64: usize = 20;

/// per-thread transform scratch
pub struct Tx {
    a: Vec<f32>,
    b: Vec<f32>,
    c: Vec<f32>,
    tmp: Vec<f32>,
    v1: Vec<V8>,
    v2: Vec<V8>,
    vt: Vec<V8>,
    pub scr: Scr,
}

impl Default for Tx {
    fn default() -> Self {
        Tx::new()
    }
}

impl Tx {
    pub fn new() -> Tx {
        Tx {
            a: vec![0.0; 4096],
            b: vec![0.0; 4096],
            c: vec![0.0; 4096],
            tmp: vec![0.0; 4 * 64 + 64],
            v1: vec![[0.0; 8]; 64],
            v2: vec![[0.0; 8]; 64],
            vt: vec![[0.0; 8]; 4 * 64 + 64],
            scr: Scr::with_size(4096),
        }
    }
}

/// (covered blocks x, y)
#[inline(always)]
pub fn cover(acs: usize) -> (usize, usize) {
    (ACSCX[acs] as usize, ACSCY[acs] as usize)
}

/// coefficients of a varblock
#[inline(always)]
pub fn size_of(acs: usize) -> usize {
    let (cx, cy) = cover(acs);
    64 * cx * cy
}

/// LLF dimensions (smaller, larger) in the coefficient layout
#[inline(always)]
pub fn llf_dims(acs: usize) -> (usize, usize) {
    let (cx, cy) = cover(acs);
    (cx.min(cy), cx.max(cy))
}

/// eight columns at once (auto-vectorized)
type V8 = [f32; 8];

#[inline(always)]
fn add8(a: V8, b: V8) -> V8 {
    let mut r = a;
    for i in 0..8 {
        r[i] += b[i];
    }
    r
}
#[inline(always)]
fn sub8(a: V8, b: V8) -> V8 {
    let mut r = a;
    for i in 0..8 {
        r[i] -= b[i];
    }
    r
}
#[inline(always)]
fn mul8(a: V8, s: f32) -> V8 {
    let mut r = a;
    for v in r.iter_mut() {
        *v *= s;
    }
    r
}

/// DCT1DImpl of n vectors (strides ss, ds in vectors), unscaled: dct::fd1 on 8 columns
fn fd1v(src: &[V8], ss: usize, dst: &mut [V8], ds: usize, n: usize, tmp: &mut [V8]) {
    if n <= 2 {
        let s0 = src[0];
        if n < 2 {
            dst[0] = s0;
        } else {
            let s1 = src[ss];
            dst[0] = add8(s0, s1);
            dst[ds] = sub8(s0, s1);
        }
        return;
    }
    let h = n >> 1;
    let wc = &tab::WC[h - 2..];
    let (ab, rest) = tmp.split_at_mut(2 * h);
    for i in 0..h {
        let x = src[i * ss];
        let y = src[(n - 1 - i) * ss];
        ab[i] = add8(x, y);
        ab[h + i] = mul8(sub8(x, y), wc[i]);
    }
    let (t01, next) = rest.split_at_mut(2 * h);
    let (t0, t1) = t01.split_at_mut(h);
    fd1v(&ab[..h], 1, t0, 1, h, next);
    fd1v(&ab[h..], 1, t1, 1, h, next);
    for i in 0..h {
        dst[2 * i * ds] = t0[i];
        let mut v = t1[i];
        if i == 0 {
            v = add8(mul8(v, f32::from_bits(0x3fb5_04f3)), t1[1]);
        } else if i < h - 1 {
            v = add8(v, t1[i + 1]);
        }
        dst[(2 * i + 1) * ds] = v;
    }
}

/// fdct1d of every column of an n x ncols matrix (ncols a multiple of 8), scaled by 1 / n
fn fdcols8(src: &[f32], n: usize, ncols: usize, dst: &mut [f32], col: &mut [V8], out: &mut [V8], tmp: &mut [V8]) {
    let inv = 1.0f32 / n as i32 as f32;
    let mut c = 0;
    while c < ncols {
        for j in 0..n {
            let o = j * ncols + c;
            col[j].copy_from_slice(&src[o..o + 8]);
        }
        fd1v(col, 1, out, 1, n, tmp);
        for j in 0..n {
            let o = j * ncols + c;
            dst[o..o + 8].copy_from_slice(&mul8(out[j], inv));
        }
        c += 8;
    }
}

// ---- unrolled 4 / 8 point transforms on L lanes (the arithmetic of dct::fd1 / id1)

const SQ2: f32 = f32::from_bits(0x3fb5_04f3);

#[inline(always)]
fn la<const L: usize>(a: [f32; L], b: [f32; L]) -> [f32; L] {
    let mut r = a;
    for i in 0..L {
        r[i] += b[i];
    }
    r
}
#[inline(always)]
fn ls<const L: usize>(a: [f32; L], b: [f32; L]) -> [f32; L] {
    let mut r = a;
    for i in 0..L {
        r[i] -= b[i];
    }
    r
}
#[inline(always)]
fn lm<const L: usize>(a: [f32; L], s: f32) -> [f32; L] {
    let mut r = a;
    for v in r.iter_mut() {
        *v *= s;
    }
    r
}

/// fd1 (n = 4), unscaled
#[inline(always)]
fn fd4<const L: usize>(y: &[[f32; L]; 4]) -> [[f32; L]; 4] {
    let w = &tab::WC[0..2];
    let a0 = la(y[0], y[3]);
    let a1 = la(y[1], y[2]);
    let b0 = lm(ls(y[0], y[3]), w[0]);
    let b1 = lm(ls(y[1], y[2]), w[1]);
    let t1 = ls(b0, b1);
    [la(a0, a1), la(lm(la(b0, b1), SQ2), t1), ls(a0, a1), t1]
}

/// fd1 (n = 8), unscaled
#[inline(always)]
fn fd8<const L: usize>(x: &[[f32; L]; 8]) -> [[f32; L]; 8] {
    let w = &tab::WC[2..6];
    let a = [la(x[0], x[7]), la(x[1], x[6]), la(x[2], x[5]), la(x[3], x[4])];
    let b = [lm(ls(x[0], x[7]), w[0]), lm(ls(x[1], x[6]), w[1]), lm(ls(x[2], x[5]), w[2]), lm(ls(x[3], x[4]), w[3])];
    let t0 = fd4(&a);
    let t1 = fd4(&b);
    [t0[0], la(lm(t1[0], SQ2), t1[1]), t0[1], la(t1[1], t1[2]), t0[2], la(t1[2], t1[3]), t0[3], t1[3]]
}

/// id1 (n = 4)
#[inline(always)]
fn id4<const L: usize>(a: &[[f32; L]; 4]) -> [[f32; L]; 4] {
    let w = &tab::WC[0..2];
    let e0 = la(a[0], a[2]);
    let e1 = ls(a[0], a[2]);
    let o0 = lm(a[1], SQ2);
    let o1 = la(a[3], a[1]);
    let t0 = lm(la(o0, o1), w[0]);
    let t1 = lm(ls(o0, o1), w[1]);
    [la(t0, e0), la(t1, e1), ls(e1, t1), ls(e0, t0)]
}

/// id1 (n = 8)
#[inline(always)]
fn id8<const L: usize>(s: &[[f32; L]; 8]) -> [[f32; L]; 8] {
    let w = &tab::WC[2..6];
    let e = id4(&[s[0], s[2], s[4], s[6]]);
    let o = [lm(s[1], SQ2), la(s[3], s[1]), la(s[5], s[3]), la(s[7], s[5])];
    let t = id4(&o);
    let tv = [lm(t[0], w[0]), lm(t[1], w[1]), lm(t[2], w[2]), lm(t[3], w[3])];
    [
        la(tv[0], e[0]),
        la(tv[1], e[1]),
        la(tv[2], e[2]),
        la(tv[3], e[3]),
        ls(e[3], tv[3]),
        ls(e[2], tv[2]),
        ls(e[1], tv[1]),
        ls(e[0], tv[0]),
    ]
}

/// 1-D forward transform of n (4 / 8) lane vectors, scaled by 1 / n
#[inline(always)]
fn fdn<const L: usize>(v: &mut [[f32; L]], n: usize) {
    if n == 8 {
        let r = fd8(&[v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]);
        for i in 0..8 {
            v[i] = lm(r[i], 0.125);
        }
    } else {
        let r = fd4(&[v[0], v[1], v[2], v[3]]);
        for i in 0..4 {
            v[i] = lm(r[i], 0.25);
        }
    }
}

/// 1-D inverse transform of n (4 / 8) lane vectors
#[inline(always)]
fn idn<const L: usize>(v: &mut [[f32; L]], n: usize) {
    if n == 8 {
        let r = id8(&[v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]);
        v[..8].copy_from_slice(&r);
    } else {
        let r = id4(&[v[0], v[1], v[2], v[3]]);
        v[..4].copy_from_slice(&r);
    }
}

/// ComputeScaledDCT< R, C > for R, C in {4, 8} (fdct2d's layout): columns on C lanes, then
/// the transposed rows on R lanes
fn fdct_small<const R: usize, const C: usize>(src: &[f32], stride: usize, out: &mut [f32]) {
    let mut a = [[0f32; C]; 8];
    for r in 0..R {
        a[r].copy_from_slice(&src[r * stride..r * stride + C]);
    }
    fdn(&mut a[..R], R);
    let mut t = [[0f32; R]; 8];
    for (c, tc) in t.iter_mut().enumerate().take(C) {
        for k in 0..R {
            tc[k] = a[k][c];
        }
    }
    fdn(&mut t[..C], C);
    if R < C {
        for k in 0..R {
            for u in 0..C {
                out[k * C + u] = t[u][k];
            }
        }
    } else {
        for u in 0..C {
            out[u * R..u * R + R].copy_from_slice(&t[u]);
        }
    }
}

/// jd_idct2d< R, C > for R, C in {4, 8}: coef (fdct2d's layout) to pixels (row stride os)
fn idct_small<const R: usize, const C: usize>(coef: &[f32], out: &mut [f32], os: usize) {
    // d: C vectors of R lanes (d[u][k])
    let mut d = [[0f32; R]; 8];
    if R < C {
        for u in 0..C {
            for k in 0..R {
                d[u][k] = coef[k * C + u];
            }
        }
    } else {
        for u in 0..C {
            d[u].copy_from_slice(&coef[u * R..u * R + R]);
        }
    }
    idn(&mut d[..C], C);
    let mut b = [[0f32; C]; 8];
    for k in 0..R {
        for c in 0..C {
            b[k][c] = d[c][k];
        }
    }
    idn(&mut b[..R], R);
    for r in 0..R {
        out[r * os..r * os + C].copy_from_slice(&b[r]);
    }
}





/// jd_idct2d with the writer's kernels (dctk)
fn idct2d(tx: &mut Tx, coef: &mut [f32], rows: usize, cols: usize, out: &mut [f32], os: usize) {
    match (rows, cols) {
        (4, 4) => return idct_small::<4, 4>(coef, out, os),
        (4, 8) => return idct_small::<4, 8>(coef, out, os),
        (8, 4) => return idct_small::<8, 4>(coef, out, os),
        _ => {}
    }
    if dctk::sizes_ok(rows, cols) {
        dctk::idct(coef, rows, cols, out, os, &mut tx.a);
    } else {
        jd_idct2d(coef, rows, cols, out, os, &mut tx.scr);
    }
}


/// TransformToPixels with the small kernels (the decoder's transforms in another float
/// order; the writer's error estimates)
pub fn to_pixels_fast(tx: &mut Tx, acs: usize, coef: &mut [f32], out: &mut [f32], os: usize) {
    match acs {
        DCT => dctk::idct(coef, 8, 8, out, os, &mut tx.a),
        DCT4X4 => {
            let (b00, b01, b10, b11) = (coef[0], coef[1], coef[8], coef[9]);
            let s = b00 + b01;
            let d = b00 - b01;
            let dcs = [(s + b10) + b11, (s - b10) - b11, (d + b10) - b11, (d - b10) + b11];
            for k in 0..4usize {
                let (yo, xo) = (k >> 1, k & 1);
                let mut blk = [0f32; 16];
                for r in 0..4 {
                    for c in 0..4 {
                        blk[r * 4 + c] = coef[(r * 2 + yo) * 8 + c * 2 + xo];
                    }
                }
                blk[0] = dcs[k];
                idct_small::<4, 4>(&blk, &mut out[yo * 4 * os + xo * 4..], os);
            }
        }
        DCT4X8 | DCT8X4 => {
            let dcs = [coef[0] + coef[8], coef[0] - coef[8]];
            for i in 0..2usize {
                let mut blk = [0f32; 32];
                for r in 0..4 {
                    blk[r * 8..r * 8 + 8].copy_from_slice(&coef[(r * 2 + i) * 8..(r * 2 + i) * 8 + 8]);
                }
                blk[0] = dcs[i];
                if acs == DCT8X4 {
                    idct_small::<8, 4>(&blk, &mut out[i * 4..], os);
                } else {
                    idct_small::<4, 8>(&blk, &mut out[i * 4 * os..], os);
                }
            }
        }
        AFV0..=17 => {
            // the decoder's AFV with the small kernels
            let afx = (acs - AFV0) & 1;
            let afy = ((acs - AFV0) >> 1) & 1;
            let (b00, b01, b10) = (coef[0], coef[1], coef[8]);
            let s = b00 + b10;
            let dcs = [(s + b01) * 4.0, s - b01, b00 - b10];
            let mut blk = [0f32; 16];
            for r in 0..4 {
                for c in 0..4 {
                    blk[r * 4 + c] = coef[(r * 2) * 8 + c * 2];
                }
            }
            blk[0] = dcs[0];
            let mut px = [0f32; 16];
            for (j, &cj) in blk.iter().enumerate() {
                let bas = &tab::AFVBASIS[j * 16..j * 16 + 16];
                for p in 0..16 {
                    px[p] += bas[p] * cj;
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
            let mut b4 = [0f32; 16];
            for r in 0..4 {
                for c in 0..4 {
                    b4[r * 4 + c] = coef[(r * 2) * 8 + c * 2 + 1];
                }
            }
            b4[0] = dcs[1];
            idct_small::<4, 4>(&b4, &mut out[4 * afy * os + (afx ^ 1) * 4..], os);
            let mut b8 = [0f32; 32];
            for r in 0..4 {
                b8[r * 8..r * 8 + 8].copy_from_slice(&coef[(r * 2 + 1) * 8..(r * 2 + 1) * 8 + 8]);
            }
            b8[0] = dcs[2];
            idct_small::<4, 8>(&b8, &mut out[4 * (afy ^ 1) * os..], os);
        }
        DCT2X2 => {
            // jd_transform's butterflies of the 2x2, 4x4 and 8x8 squares (jxl-speed: one
            // temporary, the last level straight into out)
            let mut t = [0f32; 64];
            let mut n = 1usize;
            while n <= 4 {
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
                if n == 4 {
                    for r in 0..8 {
                        out[r * os..r * os + 8].copy_from_slice(&t[r * 8..r * 8 + 8]);
                    }
                } else {
                    for r in 0..2 * n {
                        coef[r * 8..r * 8 + 2 * n].copy_from_slice(&t[r * 8..r * 8 + 2 * n]);
                    }
                }
                n *= 2;
            }
        }
        IDENTITY => {
            // jd_transform's identity: four 4x4 sub-blocks on the even / odd rows and columns
            let (b00, b01, b10, b11) = (coef[0], coef[1], coef[8], coef[9]);
            let sm = b00 + b01;
            let df = b00 - b01;
            let dcs = [(sm + b10) + b11, (sm - b10) - b11, (df + b10) - b11, (df - b10) + b11];
            for k in 0..4usize {
                let (yo, xo) = (k >> 1, k & 1);
                let mut blk = [0f32; 16];
                for r in 0..4 {
                    for c in 0..4 {
                        blk[r * 4 + c] = coef[(r * 2 + yo) * 8 + c * 2 + xo];
                    }
                }
                let o = yo * 4 * os + xo * 4;
                let mut rs = 0.0f32;
                for v in blk.iter().skip(1) {
                    rs = rs + *v;
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
            }
        }
        _ => {
            if tab::ACSLOG2[acs] != 0 {
                let (cx, cy) = cover(acs);
                idct2d(tx, coef, cy * 8, cx * 8, out, os);
            } else {
                jd_transform(acs as u32, coef, out, os, &mut tx.scr);
            }
        }
    }
}

/// ComputeScaledDCT< rows, cols > of src (row stride) into out
pub fn fdct2d(tx: &mut Tx, src: &[f32], stride: usize, rows: usize, cols: usize, out: &mut [f32]) {
    match (rows, cols) {
        (4, 4) => return fdct_small::<4, 4>(src, stride, out),
        (4, 8) => return fdct_small::<4, 8>(src, stride, out),
        (8, 4) => return fdct_small::<8, 4>(src, stride, out),
        _ => {}
    }
    if dctk::sizes_ok(rows, cols) {
        dctk::fdct(src, stride, rows, cols, out, &mut tx.a);
        return;
    }
    fdct2d_general(tx, src, stride, rows, cols, out);
}


/// the scalar column / row passes (dct::fdcols, the decoder model's order)
fn fdct2d_general(tx: &mut Tx, src: &[f32], stride: usize, rows: usize, cols: usize, out: &mut [f32]) {
    let n = rows * cols;
    for r in 0..rows {
        tx.a[r * cols..r * cols + cols].copy_from_slice(&src[r * stride..r * stride + cols]);
    }
    if rows % 8 == 0 && cols % 8 == 0 {
        fdcols8(&tx.a[..n], rows, cols, &mut tx.b[..n], &mut tx.v1, &mut tx.v2, &mut tx.vt);
        transpose(&tx.b[..n], rows, cols, &mut tx.a[..n]);
        fdcols8(&tx.a[..n], cols, rows, &mut tx.c[..n], &mut tx.v1, &mut tx.v2, &mut tx.vt);
    } else {
        fdcols(&tx.a[..n], rows, cols, &mut tx.b[..n], &mut tx.tmp);
        transpose(&tx.b[..n], rows, cols, &mut tx.a[..n]);
        fdcols(&tx.a[..n], cols, rows, &mut tx.c[..n], &mut tx.tmp);
    }
    if rows < cols {
        transpose(&tx.c[..n], cols, rows, &mut out[..n]);
    } else {
        out[..n].copy_from_slice(&tx.c[..n]);
    }
}

/// AFVTransformFromPixels
fn afv_from_pixels(tx: &mut Tx, kind: usize, px: &[f32], stride: usize, c: &mut [f32]) {
    let afx = kind & 1;
    let afy = kind / 2;
    let mut blk = [0f32; 16];
    for iy in 0..4 {
        for ix in 0..4 {
            let r = if afy == 1 { 3 - iy } else { iy };
            let cc = if afx == 1 { 3 - ix } else { ix };
            blk[r * 4 + cc] = px[(iy + 4 * afy) * stride + ix + 4 * afx];
        }
    }
    // (every output summed over j in order, the outputs side by side: vectorizes)
    let mut coeff = [0f32; 16];
    for (j, &b) in blk.iter().enumerate() {
        for (i, co) in coeff.iter_mut().enumerate() {
            *co += b * tab::AFVBASIS[i * 16 + j];
        }
    }
    for iy in 0..4 {
        for ix in 0..4 {
            c[iy * 16 + ix * 2] = coeff[iy * 4 + ix];
        }
    }
    let mut b4 = [0f32; 16];
    let o = afy * 4 * stride + if afx == 1 { 0 } else { 4 };
    fdct2d(tx, &px[o..], stride, 4, 4, &mut b4);
    for iy in 0..4 {
        for ix in 0..4 {
            c[iy * 16 + ix * 2 + 1] = b4[iy * 4 + ix];
        }
    }
    let mut b8 = [0f32; 32];
    let o = if afy == 1 { 0 } else { 4 * stride };
    fdct2d(tx, &px[o..], stride, 4, 8, &mut b8);
    for iy in 0..4 {
        for ix in 0..8 {
            c[(1 + iy * 2) * 8 + ix] = b8[iy * 8 + ix];
        }
    }
    let b00 = c[0] * 0.25;
    let b01 = c[1];
    let b10 = c[8];
    c[0] = (b00 + b01 + 2.0 * b10) * 0.25;
    c[1] = (b00 - b01) * 0.5;
    c[8] = (b00 + b01 - 2.0 * b10) * 0.25;
}

/// TransformFromPixels of one varblock (px: its top-left pixel, row stride)
pub fn from_pixels(tx: &mut Tx, acs: usize, px: &[f32], stride: usize, c: &mut [f32]) {
    match acs {
        IDENTITY => {
            for y in 0..2 {
                for x in 0..2 {
                    let mut dc = 0.0f32;
                    for iy in 0..4 {
                        for ix in 0..4 {
                            dc += px[(y * 4 + iy) * stride + x * 4 + ix];
                        }
                    }
                    dc *= 1.0 / 16.0;
                    let center = px[(y * 4 + 1) * stride + x * 4 + 1];
                    for iy in 0..4 {
                        for ix in 0..4 {
                            if ix == 1 && iy == 1 {
                                continue;
                            }
                            c[(y + iy * 2) * 8 + x + ix * 2] = px[(y * 4 + iy) * stride + x * 4 + ix] - center;
                        }
                    }
                    c[(y + 2) * 8 + x + 2] = c[y * 8 + x];
                    c[y * 8 + x] = dc;
                }
            }
            let (b00, b01, b10, b11) = (c[0], c[1], c[8], c[9]);
            c[0] = (b00 + b01 + b10 + b11) * 0.25;
            c[1] = (b00 + b01 - b10 - b11) * 0.25;
            c[8] = (b00 - b01 + b10 - b11) * 0.25;
            c[9] = (b00 - b01 - b10 + b11) * 0.25;
        }
        DCT8X4 => {
            for x in 0..2 {
                let mut b = [0f32; 32];
                fdct2d(tx, &px[x * 4..], stride, 8, 4, &mut b);
                for iy in 0..4 {
                    for ix in 0..8 {
                        c[(x + iy * 2) * 8 + ix] = b[iy * 8 + ix];
                    }
                }
            }
            let (b0, b1) = (c[0], c[8]);
            c[0] = (b0 + b1) * 0.5;
            c[8] = (b0 - b1) * 0.5;
        }
        DCT4X8 => {
            for y in 0..2 {
                let mut b = [0f32; 32];
                fdct2d(tx, &px[y * 4 * stride..], stride, 4, 8, &mut b);
                for iy in 0..4 {
                    for ix in 0..8 {
                        c[(y + iy * 2) * 8 + ix] = b[iy * 8 + ix];
                    }
                }
            }
            let (b0, b1) = (c[0], c[8]);
            c[0] = (b0 + b1) * 0.5;
            c[8] = (b0 - b1) * 0.5;
        }
        DCT4X4 => {
            for y in 0..2 {
                for x in 0..2 {
                    let mut b = [0f32; 16];
                    fdct2d(tx, &px[y * 4 * stride + x * 4..], stride, 4, 4, &mut b);
                    for iy in 0..4 {
                        for ix in 0..4 {
                            c[(y + iy * 2) * 8 + x + ix * 2] = b[iy * 4 + ix];
                        }
                    }
                }
            }
            let (b00, b01, b10, b11) = (c[0], c[1], c[8], c[9]);
            c[0] = (b00 + b01 + b10 + b11) * 0.25;
            c[1] = (b00 + b01 - b10 - b11) * 0.25;
            c[8] = (b00 - b01 + b10 - b11) * 0.25;
            c[9] = (b00 - b01 - b10 + b11) * 0.25;
        }
        DCT2X2 => {
            // DCT2TopBlock< 8 > from the pixels into c, then < 4 > and < 2 > on its top left
            // in place (jxl-speed: no 64-value copies)
            for y in 0..4 {
                for x in 0..4 {
                    let c00 = px[y * 2 * stride + x * 2];
                    let c01 = px[y * 2 * stride + x * 2 + 1];
                    let c10 = px[(y * 2 + 1) * stride + x * 2];
                    let c11 = px[(y * 2 + 1) * stride + x * 2 + 1];
                    c[y * 8 + x] = (c00 + c01 + c10 + c11) * 0.25;
                    c[y * 8 + 4 + x] = (c00 + c01 - c10 - c11) * 0.25;
                    c[(y + 4) * 8 + x] = (c00 - c01 + c10 - c11) * 0.25;
                    c[(y + 4) * 8 + 4 + x] = (c00 - c01 - c10 + c11) * 0.25;
                }
            }
            for sz in [4usize, 2] {
                let n = sz / 2;
                let mut t = [0f32; 16];
                for y in 0..n {
                    for x in 0..n {
                        let c00 = c[y * 2 * 8 + x * 2];
                        let c01 = c[y * 2 * 8 + x * 2 + 1];
                        let c10 = c[(y * 2 + 1) * 8 + x * 2];
                        let c11 = c[(y * 2 + 1) * 8 + x * 2 + 1];
                        t[y * 4 + x] = (c00 + c01 + c10 + c11) * 0.25;
                        t[y * 4 + n + x] = (c00 + c01 - c10 - c11) * 0.25;
                        t[(y + n) * 4 + x] = (c00 - c01 + c10 - c11) * 0.25;
                        t[(y + n) * 4 + n + x] = (c00 - c01 - c10 + c11) * 0.25;
                    }
                }
                for y in 0..sz {
                    for x in 0..sz {
                        c[y * 8 + x] = t[y * 4 + x];
                    }
                }
            }
        }
        14..=17 => afv_from_pixels(tx, acs - 14, px, stride, c),
        _ => {
            let (cx, cy) = cover(acs);
            fdct2d(tx, px, stride, cy * 8, cx * 8, c);
        }
    }
}

/// DCFromLowestFrequencies: the DC (cy x cx, row stride ds) of a varblock's coefficients
pub fn dc_from_llf(tx: &mut Tx, acs: usize, block: &[f32], dc: &mut [f32], ds: usize) {
    let (cx, cy) = cover(acs);
    if cx * cy == 1 {
        dc[0] = block[0];
        return;
    }
    let (mn, mx) = (cx.min(cy), cx.max(cy));
    let sa = &tab::RSD[mn - 1..];
    let sb = &tab::RSD[mx - 1..];
    let mut t = [0f32; 64];
    for i in 0..mn {
        for j in 0..mx {
            t[i * mx + j] = block[i * 8 * mx + j] * sa[i] * sb[j];
        }
    }
    let mut o = [0f32; 64];
    jd_idct2d(&mut t, cy, cx, &mut o, cx, &mut tx.scr);
    for y in 0..cy {
        dc[y * ds..y * ds + cx].copy_from_slice(&o[y * cx..y * cx + cx]);
    }
}

/// TransformToPixels (coef destroyed) into out (row stride)
pub fn to_pixels(tx: &mut Tx, acs: usize, coef: &mut [f32], out: &mut [f32], os: usize) {
    jd_transform(acs as u32, coef, out, os, &mut tx.scr);
}

/// quantization matrices of the library encodings
pub struct Qm {
    pub m: Vec<Vec<f32>>,  // per table: dequantization 1 / w [3][size]
    pub iw: Vec<Vec<f32>>, // per table: quantization w [3][size] with the LLF zeroed
}

impl Qm {
    pub fn library() -> Qm {
        let qe: Vec<Qe> = (0..17).map(Qe::library).collect();
        Qm::with(&qe)
    }
    /// the tables of the 17 encodings qe
    pub fn with(qe: &[Qe]) -> Qm {
        let mut m = Vec::with_capacity(17);
        let mut iw = Vec::with_capacity(17);
        for (t, q) in qe.iter().enumerate().take(17) {
            let d = crate::vqt::qweights(q, t).unwrap_or_default();
            let rows = tab::QTSX[t] as usize;
            let cols = tab::QTSX[t + 17] as usize;
            let num = rows * cols * 64;
            let (mn, mx) = (rows.min(cols), rows.max(cols));
            let mut w = vec![0f32; num * 3];
            for c in 0..3 {
                for i in 0..num {
                    let v = d.get(c * num + i).copied().unwrap_or(1.0);
                    w[c * num + i] = 1.0 / v;
                }
                for y in 0..mn {
                    for x in 0..mx {
                        w[c * num + y * 8 * mx + x] = 0.0;
                    }
                }
            }
            m.push(d);
            iw.push(w);
        }
        Qm { m, iw }
    }
    /// (dequantization, quantization) of strategy acs, channel c
    #[inline]
    pub fn of(&self, acs: usize, c: usize) -> (&[f32], &[f32]) {
        let t = tab::ACSQT[acs] as usize;
        let n = size_of(acs);
        (&self.m[t][c * n..(c + 1) * n], &self.iw[t][c * n..(c + 1) * n])
    }
}
