// dctk -- the VarDCT transforms as simd kernels: ComputeScaledDCT / ComputeScaledIDCT of
// rows x cols blocks (4, 8, 16, 32, 64 each)                                 [jxl-speed]
//
//   Fdct { src, stride, rows, cols, out, tmp }   forward, scaled by 1 / (rows cols), the
//                              coefficients with the smaller dimension first (etx::fdct2d)
//   Idct { coef, rows, cols, out, os, tmp }      inverse of that layout into out (row stride
//                              os) (dct::jd_idct2d, coef not destroyed)
//   fdct(..) / idct(..)        the kernels through simd::dispatch
//   sizes_ok(rows, cols)       the sizes the kernels take
//
// Every lane computes the scalar recursion of dct::fd1 / dct::id1 (etx fd8 / id8 ...) for one
// column, in the same operation order (no FMA), so the results are bit-identical for the
// scalar code, SSE2 (4 columns per register) and AVX2 (8).  The transposes between the two
// passes are 4 x 4 shuffles inside each 128-bit lane; with AVX2 the two lanes of a register
// hold rows k and k + 4 (load_f32_lanes / store_f32_lanes).  Blocks with a dimension of 4 run
// on SSE2 registers in either case.  Each pass has one call site of the 1-D transform, so the
// unrolled butterflies exist once per instruction set.

use simd::{Kernel, Simd, Sse2};

use crate::tab;

const SQ2: f32 = f32::from_bits(0x3fb5_04f3);

/// the block sizes of the kernels
#[inline(always)]
pub fn sizes_ok(rows: usize, cols: usize) -> bool {
    matches!(rows, 4 | 8 | 16 | 32 | 64) && matches!(cols, 4 | 8 | 16 | 32 | 64)
}

// ---------------------------------------------------------------- 1-D, unscaled, in place
//
// x[..n]: the vectors in and out, t[..n]: scratch (no arrays of their own: large register
// arrays would be zeroed on every call)

#[inline(always)]
fn fd2<S: Simd>(s: S, x: &mut [S::V], _t: &mut [S::V]) {
    let (a, b) = (x[0], x[1]);
    x[0] = s.addf(a, b);
    x[1] = s.subf(a, b);
}

#[inline(always)]
fn id2<S: Simd>(s: S, x: &mut [S::V], _t: &mut [S::V]) {
    let (a, b) = (x[0], x[1]);
    x[0] = s.addf(a, b);
    x[1] = s.subf(a, b);
}

/// DCT1DImpl of n vectors from its two halves (dct::fd1): the halves in t, transformed with
/// x as their scratch, then interleaved back into x
macro_rules! fdn {
    ($name:ident, $half:ident, $n:expr) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, x: &mut [S::V], t: &mut [S::V]) {
            const H: usize = $n / 2;
            let x = &mut x[..$n];
            let t = &mut t[..$n];
            let w = &tab::WC[H - 2..H - 2 + H];
            for i in 0..H {
                let (p, q) = (x[i], x[$n - 1 - i]);
                t[i] = s.addf(p, q);
                t[H + i] = s.mulf(s.subf(p, q), s.splatf(w[i]));
            }
            {
                let (t0, t1) = t.split_at_mut(H);
                let (x0, x1) = x.split_at_mut(H);
                $half(s, t0, x0);
                $half(s, t1, x1);
            }
            for i in 0..H {
                x[2 * i] = t[i];
                x[2 * i + 1] = if i == 0 {
                    s.addf(s.mulf(t[H], s.splatf(SQ2)), t[H + 1])
                } else if i < H - 1 {
                    s.addf(t[H + i], t[H + i + 1])
                } else {
                    t[$n - 1]
                };
            }
        }
    };
}

/// IDCT1DImpl of n vectors from its two halves (dct::id1): evens and odd sums in t,
/// transformed with x as their scratch, then combined into x
macro_rules! idn {
    ($name:ident, $half:ident, $n:expr) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, x: &mut [S::V], t: &mut [S::V]) {
            const H: usize = $n / 2;
            let x = &mut x[..$n];
            let t = &mut t[..$n];
            let w = &tab::WC[H - 2..H - 2 + H];
            for i in 0..H {
                t[i] = x[2 * i];
            }
            t[H] = s.mulf(x[1], s.splatf(SQ2));
            for i in 1..H {
                t[H + i] = s.addf(x[2 * i + 1], x[2 * i - 1]);
            }
            {
                let (t0, t1) = t.split_at_mut(H);
                let (x0, x1) = x.split_at_mut(H);
                $half(s, t0, x0);
                $half(s, t1, x1);
            }
            for i in 0..H {
                let tv = s.mulf(t[H + i], s.splatf(w[i]));
                x[i] = s.addf(tv, t[i]);
                x[$n - 1 - i] = s.subf(t[i], tv);
            }
        }
    };
}

fdn!(fd4, fd2, 4);
fdn!(fd8, fd4, 8);
fdn!(fd16, fd8, 16);
fdn!(fd32, fd16, 32);
fdn!(fd64, fd32, 64);
idn!(id4, id2, 4);
idn!(id8, id4, 8);
idn!(id16, id8, 16);
idn!(id32, id16, 32);
idn!(id64, id32, 64);

/// the forward (scaled by 1 / n) or inverse 1-D transform of v[..n] (n <= M)
#[inline(always)]
fn tr1<S: Simd, const M: usize>(s: S, v: &mut [S::V; M], t: &mut [S::V; M], n: usize, inv: bool) {
    macro_rules! go {
        ($f:ident, $g:ident, $n:expr) => {{
            if inv {
                $g(s, &mut v[..], &mut t[..]);
            } else {
                $f(s, &mut v[..], &mut t[..]);
                let k = s.splatf(1.0f32 / $n as f32);
                for i in 0..$n {
                    v[i] = s.mulf(v[i], k);
                }
            }
        }};
    }
    match n {
        4 => go!(fd4, id4, 4),
        8 => go!(fd8, id8, 8),
        16 if M >= 16 => go!(fd16, id16, 16),
        32 if M >= 32 => go!(fd32, id32, 32),
        64 if M >= 64 => go!(fd64, id64, 64),
        _ => {}
    }
}

// ---------------------------------------------------------------- transposes, lane loads

/// 4 x 4 transpose inside every 128-bit lane
#[inline(always)]
fn tr4<S: Simd>(s: S, r: [S::V; 4]) -> [S::V; 4] {
    let t0 = s.shufps::<0x44>(r[0], r[1]);
    let t2 = s.shufps::<0xEE>(r[0], r[1]);
    let t1 = s.shufps::<0x44>(r[2], r[3]);
    let t3 = s.shufps::<0xEE>(r[2], r[3]);
    [s.shufps::<0x88>(t0, t1), s.shufps::<0xDD>(t0, t1), s.shufps::<0x88>(t2, t3), s.shufps::<0xDD>(t2, t3)]
}

/// 4 floats at a[i0..] in lane 0, at a[i1..] in lane 1 (SSE2: i0 only)
#[inline(always)]
fn load2<S: Simd>(s: S, a: &[f32], i0: usize, i1: usize) -> S::V {
    if S::BYTES == 16 { s.load_f32(&a[i0..]) } else { s.load_f32_lanes(&a[i0..], &a[i1..]) }
}

/// lane 0 to a[i0..], lane 1 to a[i1..] (i1 >= i0 + 4; SSE2: i0 only)
#[inline(always)]
fn store2<S: Simd>(s: S, a: &mut [f32], i0: usize, i1: usize, v: S::V) {
    if S::BYTES == 16 {
        s.store_f32(&mut a[i0..], v);
    } else {
        let (lo, hi) = a.split_at_mut(i1);
        s.store_f32_lanes(&mut lo[i0..], hi, v);
    }
}

/// v[u0 + j] = lane l: m[(k0 + l) * ms + u0 + j] for j < 4, the L rows k0 .. of the row-major
/// matrix m (row stride ms) transposed into registers
#[inline(always)]
fn gather_t<S: Simd>(s: S, m: &[f32], ms: usize, k0: usize, n: usize, v: &mut [S::V]) {
    let mut u0 = 0;
    while u0 < n {
        let x = [
            load2(s, m, k0 * ms + u0, (k0 + 4) * ms + u0),
            load2(s, m, (k0 + 1) * ms + u0, (k0 + 5) * ms + u0),
            load2(s, m, (k0 + 2) * ms + u0, (k0 + 6) * ms + u0),
            load2(s, m, (k0 + 3) * ms + u0, (k0 + 7) * ms + u0),
        ];
        let t = tr4(s, x);
        v[u0] = t[0];
        v[u0 + 1] = t[1];
        v[u0 + 2] = t[2];
        v[u0 + 3] = t[3];
        u0 += 4;
    }
}

// ---------------------------------------------------------------- 8 x 8, in registers
//
// The most frequent block: straight-line code on register values (fd8v / id8v: the same
// arithmetic as fd8 / id8 above), no loops over sizes.

#[inline(always)]
fn fd4v<S: Simd>(s: S, y: [S::V; 4]) -> [S::V; 4] {
    let (w0, w1) = (s.splatf(tab::WC[0]), s.splatf(tab::WC[1]));
    let a0 = s.addf(y[0], y[3]);
    let a1 = s.addf(y[1], y[2]);
    let b0 = s.mulf(s.subf(y[0], y[3]), w0);
    let b1 = s.mulf(s.subf(y[1], y[2]), w1);
    let t1 = s.subf(b0, b1);
    [s.addf(a0, a1), s.addf(s.mulf(s.addf(b0, b1), s.splatf(SQ2)), t1), s.subf(a0, a1), t1]
}

#[inline(always)]
fn fd8v<S: Simd>(s: S, x: [S::V; 8]) -> [S::V; 8] {
    let w = [s.splatf(tab::WC[2]), s.splatf(tab::WC[3]), s.splatf(tab::WC[4]), s.splatf(tab::WC[5])];
    let a = [s.addf(x[0], x[7]), s.addf(x[1], x[6]), s.addf(x[2], x[5]), s.addf(x[3], x[4])];
    let b = [
        s.mulf(s.subf(x[0], x[7]), w[0]),
        s.mulf(s.subf(x[1], x[6]), w[1]),
        s.mulf(s.subf(x[2], x[5]), w[2]),
        s.mulf(s.subf(x[3], x[4]), w[3]),
    ];
    let t0 = fd4v(s, a);
    let t1 = fd4v(s, b);
    [
        t0[0],
        s.addf(s.mulf(t1[0], s.splatf(SQ2)), t1[1]),
        t0[1],
        s.addf(t1[1], t1[2]),
        t0[2],
        s.addf(t1[2], t1[3]),
        t0[3],
        t1[3],
    ]
}

#[inline(always)]
fn id4v<S: Simd>(s: S, a: [S::V; 4]) -> [S::V; 4] {
    let (w0, w1) = (s.splatf(tab::WC[0]), s.splatf(tab::WC[1]));
    let e0 = s.addf(a[0], a[2]);
    let e1 = s.subf(a[0], a[2]);
    let o0 = s.mulf(a[1], s.splatf(SQ2));
    let o1 = s.addf(a[3], a[1]);
    let t0 = s.mulf(s.addf(o0, o1), w0);
    let t1 = s.mulf(s.subf(o0, o1), w1);
    [s.addf(t0, e0), s.addf(t1, e1), s.subf(e1, t1), s.subf(e0, t0)]
}

#[inline(always)]
fn id8v<S: Simd>(s: S, x: [S::V; 8]) -> [S::V; 8] {
    let e = id4v(s, [x[0], x[2], x[4], x[6]]);
    let t = id4v(s, [s.mulf(x[1], s.splatf(SQ2)), s.addf(x[3], x[1]), s.addf(x[5], x[3]), s.addf(x[7], x[5])]);
    let tv = [
        s.mulf(t[0], s.splatf(tab::WC[2])),
        s.mulf(t[1], s.splatf(tab::WC[3])),
        s.mulf(t[2], s.splatf(tab::WC[4])),
        s.mulf(t[3], s.splatf(tab::WC[5])),
    ];
    [
        s.addf(tv[0], e[0]),
        s.addf(tv[1], e[1]),
        s.addf(tv[2], e[2]),
        s.addf(tv[3], e[3]),
        s.subf(e[3], tv[3]),
        s.subf(e[2], tv[2]),
        s.subf(e[1], tv[1]),
        s.subf(e[0], tv[0]),
    ]
}

/// DCT1DImpl of n register values from its two halves (as fdn, on values)
macro_rules! fdv {
    ($name:ident, $half:ident, $n:expr) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, x: [S::V; $n]) -> [S::V; $n] {
            const H: usize = $n / 2;
            let mut a = [s.zero(); H];
            let mut b = [s.zero(); H];
            for i in 0..H {
                a[i] = s.addf(x[i], x[$n - 1 - i]);
                b[i] = s.mulf(s.subf(x[i], x[$n - 1 - i]), s.splatf(tab::WC[H - 2 + i]));
            }
            let t0 = $half(s, a);
            let t1 = $half(s, b);
            let mut o = [s.zero(); $n];
            for i in 0..H {
                o[2 * i] = t0[i];
                o[2 * i + 1] = if i == 0 {
                    s.addf(s.mulf(t1[0], s.splatf(SQ2)), t1[1])
                } else if i < H - 1 {
                    s.addf(t1[i], t1[i + 1])
                } else {
                    t1[H - 1]
                };
            }
            o
        }
    };
}

/// IDCT1DImpl of n register values from its two halves (as idn, on values)
macro_rules! idv {
    ($name:ident, $half:ident, $n:expr) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, x: [S::V; $n]) -> [S::V; $n] {
            const H: usize = $n / 2;
            let mut ev = [s.zero(); H];
            let mut od = [s.zero(); H];
            for i in 0..H {
                ev[i] = x[2 * i];
            }
            od[0] = s.mulf(x[1], s.splatf(SQ2));
            for i in 1..H {
                od[i] = s.addf(x[2 * i + 1], x[2 * i - 1]);
            }
            let e = $half(s, ev);
            let t = $half(s, od);
            let mut o = [s.zero(); $n];
            for i in 0..H {
                let tv = s.mulf(t[i], s.splatf(tab::WC[H - 2 + i]));
                o[i] = s.addf(tv, e[i]);
                o[$n - 1 - i] = s.subf(e[i], tv);
            }
            o
        }
    };
}

fdv!(fd16v, fd8v, 16);
fdv!(fd32v, fd16v, 32);
idv!(id16v, id8v, 16);
idv!(id32v, id16v, 32);

/// x[u] (u < n): lane l = m[(k0 + l) * ms + u], the L rows k0 .. of the row-major m
/// transposed into register values
#[inline(always)]
fn gather_v<S: Simd, const N: usize>(s: S, m: &[f32], ms: usize, k0: usize) -> [S::V; N] {
    let mut x = [s.zero(); N];
    let mut u0 = 0;
    while u0 < N {
        let t = tr4(
            s,
            [
                load2(s, m, k0 * ms + u0, (k0 + 4) * ms + u0),
                load2(s, m, (k0 + 1) * ms + u0, (k0 + 5) * ms + u0),
                load2(s, m, (k0 + 2) * ms + u0, (k0 + 6) * ms + u0),
                load2(s, m, (k0 + 3) * ms + u0, (k0 + 7) * ms + u0),
            ],
        );
        x[u0] = t[0];
        x[u0 + 1] = t[1];
        x[u0 + 2] = t[2];
        x[u0 + 3] = t[3];
        u0 += 4;
    }
    x
}

/// ComputeScaledDCT< R, C > for R, C in {8, 16} on register values (a: R * C floats scratch)
macro_rules! fdct_rc {
    ($name:ident, $r:expr, $c:expr, $fr:ident, $fc:ident) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, src: &[f32], stride: usize, out: &mut [f32], a: &mut [f32]) {
            const R: usize = $r;
            const C: usize = $c;
            let l = S::BYTES / 4;
            let a = &mut a[..R * C];
            let out = &mut out[..R * C];
            let kr = s.splatf(1.0f32 / R as f32);
            let kc = s.splatf(1.0f32 / C as f32);
            let mut g = 0;
            while g < C {
                let mut x = [s.zero(); R];
                for (r, xr) in x.iter_mut().enumerate() {
                    *xr = s.load_f32(&src[r * stride + g..]);
                }
                let y = $fr(s, x);
                for (r, yr) in y.iter().enumerate() {
                    s.store_f32(&mut a[r * C + g..], s.mulf(*yr, kr));
                }
                g += l;
            }
            let mut g = 0;
            while g < R {
                let y = $fc(s, gather_v::<S, C>(s, a, C, g));
                if R < C {
                    // out[k][u] (R x C): transposed back
                    let mut u0 = 0;
                    while u0 < C {
                        let t = tr4(s, [s.mulf(y[u0], kc), s.mulf(y[u0 + 1], kc), s.mulf(y[u0 + 2], kc), s.mulf(y[u0 + 3], kc)]);
                        for (j, tj) in t.iter().enumerate() {
                            store2(s, out, (g + j) * C + u0, (g + 4 + j) * C + u0, *tj);
                        }
                        u0 += 4;
                    }
                } else {
                    for (u, yu) in y.iter().enumerate() {
                        s.store_f32(&mut out[u * R + g..], s.mulf(*yu, kc));
                    }
                }
                g += l;
            }
        }
    };
}

/// ComputeScaledIDCT< R, C > for R, C in {8, 16} on register values (d: R * C floats scratch)
macro_rules! idct_rc {
    ($name:ident, $r:expr, $c:expr, $ir:ident, $ic:ident) => {
        #[inline(always)]
        fn $name<S: Simd>(s: S, coef: &[f32], out: &mut [f32], os: usize, d: &mut [f32]) {
            const R: usize = $r;
            const C: usize = $c;
            let l = S::BYTES / 4;
            let coef = &coef[..R * C];
            let d = &mut d[..R * C];
            let mut g = 0;
            while g < R {
                let x = if R < C {
                    gather_v::<S, C>(s, coef, C, g)
                } else {
                    let mut x = [s.zero(); C];
                    for (u, xu) in x.iter_mut().enumerate() {
                        *xu = s.load_f32(&coef[u * R + g..]);
                    }
                    x
                };
                let y = $ic(s, x);
                for (u, yu) in y.iter().enumerate() {
                    s.store_f32(&mut d[u * R + g..], *yu);
                }
                g += l;
            }
            let mut g = 0;
            while g < C {
                let y = $ir(s, gather_v::<S, R>(s, d, R, g));
                for (r, yr) in y.iter().enumerate() {
                    s.store_f32(&mut out[r * os + g..], *yr);
                }
                g += l;
            }
        }
    };
}

fdct_rc!(fdct8x8, 8, 8, fd8v, fd8v);
fdct_rc!(fdct8x16, 8, 16, fd8v, fd16v);
fdct_rc!(fdct16x8, 16, 8, fd16v, fd8v);
fdct_rc!(fdct16x16, 16, 16, fd16v, fd16v);
fdct_rc!(fdct16x32, 16, 32, fd16v, fd32v);
fdct_rc!(fdct32x16, 32, 16, fd32v, fd16v);
fdct_rc!(fdct32x32, 32, 32, fd32v, fd32v);
idct_rc!(idct8x8, 8, 8, id8v, id8v);
idct_rc!(idct8x16, 8, 16, id8v, id16v);
idct_rc!(idct16x8, 16, 8, id16v, id8v);
idct_rc!(idct16x16, 16, 16, id16v, id16v);
idct_rc!(idct16x32, 16, 32, id16v, id32v);
idct_rc!(idct32x16, 32, 16, id32v, id16v);
idct_rc!(idct32x32, 32, 32, id32v, id32v);

// ---------------------------------------------------------------- 2-D kernels

/// ComputeScaledDCT< rows, cols > of src (row stride) into out (rows < cols: rows x cols,
/// else cols x rows); tmp: rows * cols floats
pub struct Fdct<'a> {
    pub src: &'a [f32],
    pub stride: usize,
    pub rows: usize,
    pub cols: usize,
    pub out: &'a mut [f32],
    pub tmp: &'a mut [f32],
}

#[inline(always)]
fn fdct2<S: Simd, const M: usize>(s: S, k: Fdct) {
    let l = S::BYTES / 4;
    let (rows, cols) = (k.rows, k.cols);
    let a = &mut k.tmp[..rows * cols];
    let out = k.out;
    // pass 0: the columns, L at a time, into a (rows x cols); pass 1: the rows, L at a time
    // (transposed into registers), into out
    let mut v = [s.zero(); M];
    let mut t = [s.zero(); M];
    for pass in 0..2 {
        let (groups, n) = if pass == 0 { (cols, rows) } else { (rows, cols) };
        let mut g0 = 0;
        while g0 < groups {
            if pass == 0 {
                for r in 0..rows {
                    v[r] = s.load_f32(&k.src[r * k.stride + g0..]);
                }
            } else {
                gather_t(s, a, cols, g0, cols, &mut v);
            }
            tr1(s, &mut v, &mut t, n, false);
            if pass == 0 {
                for r in 0..rows {
                    s.store_f32(&mut a[r * cols + g0..], v[r]);
                }
            } else if rows < cols {
                // out[k][u] (rows x cols): transposed back
                let mut u0 = 0;
                while u0 < cols {
                    let t = tr4(s, [v[u0], v[u0 + 1], v[u0 + 2], v[u0 + 3]]);
                    for (j, tj) in t.iter().enumerate() {
                        store2(s, out, (g0 + j) * cols + u0, (g0 + 4 + j) * cols + u0, *tj);
                    }
                    u0 += 4;
                }
            } else {
                // out[u][k] (cols x rows)
                for u in 0..cols {
                    s.store_f32(&mut out[u * rows + g0..], v[u]);
                }
            }
            g0 += l;
        }
    }
}

impl Kernel for Fdct<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let m = self.rows.max(self.cols);
        if self.rows == 4 || self.cols == 4 {
            if m <= 8 { fdct2::<Sse2, 8>(Sse2, self) } else { fdct2::<Sse2, 64>(Sse2, self) }
        } else if m <= 16 {
            match (self.rows, self.cols) {
                (8, 8) => fdct8x8(s, self.src, self.stride, self.out, self.tmp),
                (8, 16) => fdct8x16(s, self.src, self.stride, self.out, self.tmp),
                (16, 8) => fdct16x8(s, self.src, self.stride, self.out, self.tmp),
                _ => fdct16x16(s, self.src, self.stride, self.out, self.tmp),
            }
        } else {
            match (self.rows, self.cols) {
                (16, 32) => fdct16x32(s, self.src, self.stride, self.out, self.tmp),
                (32, 16) => fdct32x16(s, self.src, self.stride, self.out, self.tmp),
                (32, 32) => fdct32x32(s, self.src, self.stride, self.out, self.tmp),
                _ => fdct2::<S, 64>(s, self),
            }
        }
    }
}

/// ComputeScaledIDCT< rows, cols > of coef (fdct's layout) into out (row stride os); tmp:
/// rows * cols floats
pub struct Idct<'a> {
    pub coef: &'a [f32],
    pub rows: usize,
    pub cols: usize,
    pub out: &'a mut [f32],
    pub os: usize,
    pub tmp: &'a mut [f32],
}

#[inline(always)]
fn idct2<S: Simd, const M: usize>(s: S, k: Idct) {
    let l = S::BYTES / 4;
    let (rows, cols) = (k.rows, k.cols);
    let d = &mut k.tmp[..rows * cols];
    let (coef, out, os) = (k.coef, k.out, k.os);
    // pass 0: along the coefficient columns u for L rows k at a time (D[u][k] = rows < cols ?
    // coef[k][u] : coef[u][k]) into d (cols x rows); pass 1: along k for L pixel columns c at
    // a time (d transposed into registers) into out
    let mut v = [s.zero(); M];
    let mut t = [s.zero(); M];
    for pass in 0..2 {
        let (groups, n) = if pass == 0 { (rows, cols) } else { (cols, rows) };
        let mut g0 = 0;
        while g0 < groups {
            if pass == 0 {
                if rows < cols {
                    gather_t(s, coef, cols, g0, cols, &mut v);
                } else {
                    for u in 0..cols {
                        v[u] = s.load_f32(&coef[u * rows + g0..]);
                    }
                }
            } else {
                gather_t(s, d, rows, g0, rows, &mut v);
            }
            tr1(s, &mut v, &mut t, n, true);
            if pass == 0 {
                for u in 0..cols {
                    s.store_f32(&mut d[u * rows + g0..], v[u]);
                }
            } else {
                for r in 0..rows {
                    s.store_f32(&mut out[r * os + g0..], v[r]);
                }
            }
            g0 += l;
        }
    }
}

impl Kernel for Idct<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let m = self.rows.max(self.cols);
        if self.rows == 4 || self.cols == 4 {
            if m <= 8 { idct2::<Sse2, 8>(Sse2, self) } else { idct2::<Sse2, 64>(Sse2, self) }
        } else if m <= 16 {
            match (self.rows, self.cols) {
                (8, 8) => idct8x8(s, self.coef, self.out, self.os, self.tmp),
                (8, 16) => idct8x16(s, self.coef, self.out, self.os, self.tmp),
                (16, 8) => idct16x8(s, self.coef, self.out, self.os, self.tmp),
                _ => idct16x16(s, self.coef, self.out, self.os, self.tmp),
            }
        } else {
            match (self.rows, self.cols) {
                (16, 32) => idct16x32(s, self.coef, self.out, self.os, self.tmp),
                (32, 16) => idct32x16(s, self.coef, self.out, self.os, self.tmp),
                (32, 32) => idct32x32(s, self.coef, self.out, self.os, self.tmp),
                _ => idct2::<S, 64>(s, self),
            }
        }
    }
}

/// Fdct through simd::dispatch (sizes_ok(rows, cols))
#[inline]
pub fn fdct(src: &[f32], stride: usize, rows: usize, cols: usize, out: &mut [f32], tmp: &mut [f32]) {
    simd::dispatch(Fdct { src, stride, rows, cols, out, tmp });
}

/// Idct through simd::dispatch (sizes_ok(rows, cols))
#[inline]
pub fn idct(coef: &[f32], rows: usize, cols: usize, out: &mut [f32], os: usize, tmp: &mut [f32]) {
    simd::dispatch(Idct { coef, rows, cols, out, os, tmp });
}
