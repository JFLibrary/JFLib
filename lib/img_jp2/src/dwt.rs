// dwt -- the inverse discrete wavelet transform (ITU-T T.800 Annex F)
//
// One level: the four bands interleaved into the next resolution by the parity of its absolute
// coordinates (an even column takes a low-pass sample, an odd one a high-pass sample), then the
// one-dimensional synthesis on every row and then on every column.  The reversible 5/3 filter
// lifts integers; the irreversible 9/7 filter lifts f32 (each step written out, so no target fuses
// a multiply and an add and every target gets the same bits).  Both extend a signal symmetrically
// at its ends; a signal of one sample is the sample (a single odd one is half of it).  The columns
// are lifted as whole rows: each step adds two neighbouring rows into one.

use alloc::vec;
use alloc::vec::Vec;

/// the lifting constants of the 9/7 filter (Table F.4) and the scaling of the bands: the low-pass
/// by K, the high-pass by 1/K (the step sizes of Annex E hold the bands' nominal gains; decoders
/// that leave the gains out of the step size scale the high-pass by 2/K instead)
const ALPHA: f32 = -1.586_134_3;
const BETA: f32 = -0.052_980_118;
const GAMMA: f32 = 0.882_911_1;
const DELTA: f32 = 0.443_506_87;
const K: f32 = 1.230_174_1;
const INV_K: f32 = 1.0 / 1.230_174_1;

/// the number of low-pass and of high-pass samples of the span [a0, a1)
pub(crate) fn split(a0: u32, a1: u32) -> (usize, usize) {
    let low = a1.div_ceil(2) - a0.div_ceil(2);
    let high = a1 / 2 - a0 / 2;
    (low as usize, high as usize)
}

/// the next resolution, rx0..rx1 x ry0..ry1, from the one below and the bands HL, LH and HH (each
/// row by row; an empty one is zero)
pub(crate) fn interleave<T: Copy + Default>(ll: &[T], hl: &[T], lh: &[T], hh: &[T], rx0: u32, ry0: u32, rx1: u32, ry1: u32) -> Vec<T> {
    let (w, h) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
    let (lw, hw) = split(rx0, rx1);
    let mut out = vec![T::default(); w * h];
    if w == 0 {
        return out;
    }
    // the columns of the low-pass and of the high-pass samples
    let (lo_x, hi_x) = if rx0 & 1 == 0 { (0, 1) } else { (1, 0) };
    for (v, row) in out.chunks_exact_mut(w).enumerate() {
        let y = ry0 + v as u32;
        let (r, lo, hi) = if y & 1 == 0 { ((y / 2 - ry0.div_ceil(2)) as usize, ll, hl) } else { ((y / 2 - ry0 / 2) as usize, lh, hh) };
        if !lo.is_empty() {
            for (d, &s) in row.iter_mut().skip(lo_x).step_by(2).zip(&lo[r * lw..][..lw]) {
                *d = s;
            }
        }
        if !hi.is_empty() {
            for (d, &s) in row.iter_mut().skip(hi_x).step_by(2).zip(&hi[r * hw..][..hw]) {
                *d = s;
            }
        }
    }
    out
}

/// the left and right neighbours of position k in a signal of n samples, mirrored at the ends
#[inline(always)]
fn nbrs(k: usize, n: usize) -> (usize, usize) {
    let l = if k == 0 { 1 } else { k - 1 };
    let r = if k + 1 < n { k + 1 } else { k - 1 };
    (l, r)
}

/// row k for writing and rows a, b (both other than k) for reading, of a buffer of rows of w
fn rows3<T>(buf: &mut [T], w: usize, k: usize, a: usize, b: usize) -> (&mut [T], &[T], &[T]) {
    let (before, rest) = buf.split_at_mut(k * w);
    let (row, after) = rest.split_at_mut(w);
    let pick = |i: usize| -> &[T] {
        if i < k {
            &before[i * w..i * w + w]
        } else {
            &after[(i - k - 1) * w..(i - k - 1) * w + w]
        }
    };
    let (ra, rb) = (pick(a), pick(b));
    (row, ra, rb)
}

/// the 5/3 synthesis of one row; `odd` when its first sample is at an odd position (high-pass)
fn row53(r: &mut [i32], odd: bool) {
    let n = r.len();
    if n == 1 {
        if odd {
            r[0] /= 2;
        }
        return;
    }
    let (lo, hi) = if odd { (1, 0) } else { (0, 1) };
    let mut k = lo;
    while k < n {
        let (a, b) = nbrs(k, n);
        r[k] = r[k].wrapping_sub(r[a].wrapping_add(r[b]).wrapping_add(2) >> 2);
        k += 2;
    }
    let mut k = hi;
    while k < n {
        let (a, b) = nbrs(k, n);
        r[k] = r[k].wrapping_add(r[a].wrapping_add(r[b]) >> 1);
        k += 2;
    }
}

/// the 5/3 synthesis of every column of a w x h buffer
fn col53(buf: &mut [i32], w: usize, h: usize, odd: bool) {
    if h == 1 {
        if odd {
            for v in buf.iter_mut() {
                *v /= 2;
            }
        }
        return;
    }
    let (lo, hi) = if odd { (1, 0) } else { (0, 1) };
    let mut k = lo;
    while k < h {
        let (a, b) = nbrs(k, h);
        let (row, ra, rb) = rows3(buf, w, k, a, b);
        for ((x, &p), &q) in row.iter_mut().zip(ra).zip(rb) {
            *x = x.wrapping_sub(p.wrapping_add(q).wrapping_add(2) >> 2);
        }
        k += 2;
    }
    let mut k = hi;
    while k < h {
        let (a, b) = nbrs(k, h);
        let (row, ra, rb) = rows3(buf, w, k, a, b);
        for ((x, &p), &q) in row.iter_mut().zip(ra).zip(rb) {
            *x = x.wrapping_add(p.wrapping_add(q) >> 1);
        }
        k += 2;
    }
}

fn lift_row(r: &mut [f32], start: usize, c: f32) {
    let n = r.len();
    let mut k = start;
    while k < n {
        let (a, b) = nbrs(k, n);
        let s = r[a] + r[b];
        r[k] -= c * s;
        k += 2;
    }
}

/// the 9/7 synthesis of one row
fn row97(r: &mut [f32], odd: bool) {
    let n = r.len();
    if n == 1 {
        if odd {
            r[0] *= 0.5;
        }
        return;
    }
    let (lo, hi) = if odd { (1, 0) } else { (0, 1) };
    for (k, v) in r.iter_mut().enumerate() {
        *v *= if k & 1 == lo { K } else { INV_K };
    }
    lift_row(r, lo, DELTA);
    lift_row(r, hi, GAMMA);
    lift_row(r, lo, BETA);
    lift_row(r, hi, ALPHA);
}

fn lift_cols(buf: &mut [f32], w: usize, h: usize, start: usize, c: f32) {
    let mut k = start;
    while k < h {
        let (a, b) = nbrs(k, h);
        let (row, ra, rb) = rows3(buf, w, k, a, b);
        for ((x, &p), &q) in row.iter_mut().zip(ra).zip(rb) {
            let s = p + q;
            *x -= c * s;
        }
        k += 2;
    }
}

/// the 9/7 synthesis of every column
fn col97(buf: &mut [f32], w: usize, h: usize, odd: bool) {
    if h == 1 {
        if odd {
            for v in buf.iter_mut() {
                *v *= 0.5;
            }
        }
        return;
    }
    let (lo, hi) = if odd { (1, 0) } else { (0, 1) };
    for (k, row) in buf.chunks_exact_mut(w).enumerate() {
        let s = if k & 1 == lo { K } else { INV_K };
        for v in row.iter_mut() {
            *v *= s;
        }
    }
    lift_cols(buf, w, h, lo, DELTA);
    lift_cols(buf, w, h, hi, GAMMA);
    lift_cols(buf, w, h, lo, BETA);
    lift_cols(buf, w, h, hi, ALPHA);
}

/// one level of the reversible transform on the interleaved buffer of rx0..rx1 x ry0..ry1
pub(crate) fn level53(buf: &mut [i32], rx0: u32, ry0: u32, rx1: u32, ry1: u32) {
    let (w, h) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
    if w == 0 || h == 0 {
        return;
    }
    for row in buf.chunks_exact_mut(w) {
        row53(row, rx0 & 1 == 1);
    }
    col53(buf, w, h, ry0 & 1 == 1);
}

/// one level of the irreversible transform
pub(crate) fn level97(buf: &mut [f32], rx0: u32, ry0: u32, rx1: u32, ry1: u32) {
    let (w, h) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
    if w == 0 || h == 0 {
        return;
    }
    for row in buf.chunks_exact_mut(w) {
        row97(row, rx0 & 1 == 1);
    }
    col97(buf, w, h, ry0 & 1 == 1);
}
