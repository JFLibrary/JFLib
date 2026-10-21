// itx -- AV1 reconstruction: dequantisation and inverse transforms (specification 7.12, 7.13)
//                                                                    [src/img/av1_itx.s]
//
//   T::recon()     reconstruct( ppl, px, py, ptx ) with Quant, PlaneTxType (ptxt) and the
//                  block's qindex (qidx)
//
// The butterflies of the 1D DCT / ADST8 / ADST16 are the generated straight-line functions of
// txfn.rs (from the model, like the assembly's av1_txfn.s); ADST4, identity and WHT are coded
// here.  Residual (i32, stride 64) in Ts::dq.

use crate::blk::T;
use crate::tab::*;
use crate::txfn;

const IT_DCT: u8 = 0;
const IT_ADST: u8 = 1;
const IT_IDTX: u8 = 2;
const IT_WHT2: u8 = 3; // Walsh-Hadamard, pre-shift 2 (rows)
const IT_WHT0: u8 = 4; // pre-shift 0 (columns)
const IDTX: u32 = 9;
const DCT_DCT: u32 = 0;

// dqDenom per txSz
static RC_DEN: [u8; 19] = [1, 1, 1, 2, 4, 1, 1, 1, 1, 2, 2, 4, 4, 1, 1, 1, 1, 2, 2];
// transform kinds per PlaneTxType (rows: second part of the name, columns: first part)
static RC_ROWKIND: [u8; 16] = [0, 0, 1, 1, 0, 1, 1, 1, 1, 2, 2, 0, 2, 1, 2, 1];
static RC_COLKIND: [u8; 16] = [0, 1, 0, 1, 1, 0, 1, 1, 1, 2, 0, 2, 1, 2, 1, 2];
// flipUD (bit 0) / flipLR (bit 1) per PlaneTxType
static RC_FLIPTAB: [u8; 16] = [0, 0, 0, 0, 1, 2, 3, 2, 1, 0, 0, 0, 0, 0, 1, 2];

/// dequantise one coefficient: Abs( v * q ) & 0xFFFFFF / dqDenom with v's sign, clamped
#[inline(always)]
fn dequant(v: i32, q: u32, den: u32, lim: i32) -> i32 {
    let dq = v as i64 * q as i64;
    let a = ((dq.wrapping_abs() as u32) & 0xFF_FFFF) / den;
    let mut a = if dq < 0 { (a as i32).wrapping_neg() } else { a as i32 };
    if a >= lim {
        a = lim - 1;
    }
    if a < -lim {
        a = -lim;
    }
    a
}

impl T<'_> {
    /// av1_recon
    pub fn recon(&mut self) {
        let h = self.h;
        let s = &mut *self.s;
        let tx = s.ptx as usize;
        let den = RC_DEN[tx] as u32;
        let lw = TX_WIDTH_LOG2[tx] as u32;
        let lh = TX_HEIGHT_LOG2[tx] as u32;
        let (w, hh) = (1usize << lw, 1usize << lh);
        let (tw, th) = (w.min(32), hh.min(32));
        let rect = (lw as i32 - lh as i32).abs() == 1;
        // quantizers
        let p = s.ppl as usize;
        let cls = ((h.bd - 8) >> 1) as usize * 256;
        let q = s.qidx as i32;
        let (dcd, acd) = match p {
            0 => (h.dqydc, 0),
            1 => (h.dqudc, h.dquac),
            _ => (h.dqvdc, h.dqvac),
        };
        let dci = q.wrapping_add(dcd).clamp(0, 255) as usize;
        let aci = q.wrapping_add(acd).clamp(0, 255) as usize;
        let dcq = DC_QLOOKUP[cls + dci] as u32;
        let acq = AC_QLOOKUP[cls + aci] as u32;
        // quantizer matrix
        let mut qm: Option<&[u8]> = None;
        if h.useqm != 0 && s.ptxt < IDTX {
            let lvl = h.qmlvl[p][(s.seg & 7) as usize] as usize;
            if lvl < 15 {
                let o = (lvl * 2 + (p != 0) as usize) * 3344 + QM_OFFSET[tx] as usize;
                qm = Some(&QUANTIZER_MATRIX[o..]);
            }
        }
        let lim = 1i32 << (h.bd + 7);
        // shifts, clamps, kinds, flips
        let rcl = h.bd + 8;
        let ccl = (h.bd + 6).max(16);
        let k = (s.ptxt & 15) as usize;
        let mut rk = RC_ROWKIND[k];
        let mut ck = RC_COLKIND[k];
        let fl = RC_FLIPTAB[k];
        let (fud, flr) = (fl & 1 != 0, fl >> 1 != 0);
        let mut rsh = TRANSFORM_ROW_SHIFT[tx] as u32;
        let mut csh = 4u32;
        if s.ll != 0 {
            rsh = 0;
            csh = 0;
            rk = IT_WHT2;
            ck = IT_WHT0;
        } else if s.eob == 1 && s.ptxt == DCT_DCT {
            // DCT_DCT with only the DC coefficient: every residual sample is the same value
            let mut q0 = dcq;
            if let Some(m) = qm {
                q0 = (q0.wrapping_mul(m[0] as u32) + 16) >> 5;
            }
            let a = dequant(s.quant[0], q0, den, lim);
            let mut x = a as i64;
            if rect {
                x = (x * 2896 + 2048) >> 12;
            }
            x = (x * 2896 + 2048) >> 12; // row DCT output
            x = (x + ((1i64 << rsh) >> 1)) >> rsh;
            let c = 1i32 << (ccl - 1);
            let x = (x as i32).max(-c).min(c - 1);
            let x = (x as i64 * 2896 + 2048) >> 12; // column DCT output
            let r = (x as i32).wrapping_add(8) >> 4; // residual
            let st = self.f.ps[p] as usize;
            let dst = &mut self.f.pl[p];
            let mx = (1i32 << h.bd) - 1;
            let mut row = s.py as usize * st + s.px as usize;
            for _ in 0..hh {
                for v in &mut dst[row..row + w] {
                    *v = (*v as i32 + r).max(0).min(mx) as u16;
                }
                row += st;
            }
            return;
        }
        // row transforms
        let chi = (1i32 << (ccl - 1)) - 1;
        let clo = -(1i32 << (ccl - 1));
        let rrnd = (1i32 << rsh) >> 1;
        for i in 0..hh {
            let t = &mut s.tt;
            let mut nz = 0i32;
            for j in 0..w {
                let mut v = 0i32;
                if i < th && j < tw {
                    let pos = (i * tw + j) & 1023;
                    v = s.quant[pos];
                    if v != 0 {
                        let mut qq = if i | j != 0 { acq } else { dcq };
                        if let Some(m) = qm {
                            qq = (qq.wrapping_mul(m[pos] as u32) + 16) >> 5; // Round2( q * qm, 5 )
                        }
                        v = dequant(v, qq, den, lim);
                    }
                }
                t[j] = v;
                nz |= v;
            }
            let res = &mut s.dq[i * 64..i * 64 + w];
            if nz == 0 {
                res.fill(0);
                continue;
            }
            if rect {
                for v in &mut t[..w] {
                    *v = v.wrapping_mul(2896).wrapping_add(2048) >> 12;
                }
            }
            itx1d(t, lw, rk, rcl);
            // Residual[ i ][ j ] = clamp( Round2( T[ j ], rowShift ) )
            for (r, v) in res.iter_mut().zip(t.iter()) {
                *r = (v.wrapping_add(rrnd) >> rsh).max(clo).min(chi);
            }
        }
        // column transforms
        let crnd = (1i32 << csh) >> 1;
        for j in 0..w {
            let t = &mut s.tt;
            for i in 0..hh {
                t[i] = s.dq[i * 64 + j];
            }
            itx1d(t, lh, ck, ccl);
            for i in 0..hh {
                s.dq[i * 64 + j] = t[i].wrapping_add(crnd) >> csh;
            }
        }
        // CurrFrame[ y + yy ][ x + xx ] = Clip1( CurrFrame + Residual[ i ][ j ] )
        let st = self.f.ps[p] as usize;
        let dst = &mut self.f.pl[p];
        let mx = (1i32 << h.bd) - 1;
        let base = s.py as usize * st + s.px as usize;
        for i in 0..hh {
            let yy = if fud { hh - 1 - i } else { i };
            let row = base + yy * st;
            let res = &s.dq[i * 64..i * 64 + w];
            if flr {
                for (j, r) in res.iter().enumerate() {
                    let d = &mut dst[row + w - 1 - j];
                    *d = (*d as i32).wrapping_add(*r).max(0).min(mx) as u16;
                }
            } else {
                for (d, r) in dst[row..row + w].iter_mut().zip(res) {
                    *d = (*d as i32).wrapping_add(*r).max(0).min(mx) as u16;
                }
            }
        }
    }
}

/// itx1d: in-place 1D inverse transform of length 1 << n (IT_*), clamp range r
fn itx1d(t: &mut [i32; 64], n: u32, kind: u8, r: u32) {
    let hi = (1i32 << (r - 1)) - 1;
    let len = 1usize << n;
    match kind {
        IT_DCT => {
            // T[ i ] = copy[ brev( n, i ) ] (in place: the bit reversal is its own inverse), then
            // the butterflies
            let br: &[u8] = match n {
                2 => &BREV2,
                3 => &BREV3,
                4 => &BREV4,
                5 => &BREV5,
                _ => &BREV6,
            };
            for i in 0..len {
                let j = (br[i] & 63) as usize;
                if j > i {
                    t.swap(i, j);
                }
            }
            match n {
                2 => txfn::dct4(sub(t), hi),
                3 => txfn::dct8(sub(t), hi),
                4 => txfn::dct16(sub(t), hi),
                5 => txfn::dct32(sub(t), hi),
                _ => txfn::dct64(t, hi),
            }
        }
        IT_ADST => {
            if n == 2 {
                adst4(t);
                return;
            }
            // input permutation, butterflies, output permutation (with negation)
            let c: [i32; 16] = *sub::<16>(t);
            let (pin, pout): (&[u8], &[u8]) = if n == 3 { (&ADST_IN8, &ADST_OUT8) } else { (&ADST_IN16, &ADST_OUT16) };
            for i in 0..len {
                t[i] = c[(pin[i] & 15) as usize];
            }
            if n == 3 {
                txfn::adst8(sub(t), hi);
            } else {
                txfn::adst16(sub(t), hi);
            }
            let c: [i32; 16] = *sub::<16>(t);
            for i in 0..len {
                let o = pout[i];
                let v = c[(o & 15) as usize];
                t[i] = if o & 0x80 != 0 { v.wrapping_neg() } else { v };
            }
        }
        IT_IDTX => {
            for v in &mut t[..len] {
                *v = match n {
                    2 => ((*v as i64 * 5793 + 2048) >> 12) as i32,
                    3 => v.wrapping_add(*v),
                    4 => ((*v as i64 * 11586 + 2048) >> 12) as i32,
                    _ => v.wrapping_shl(2), // identity 32: * 4
                };
            }
        }
        _ => {
            // Walsh-Hadamard (n = 2)
            let sh = if kind == IT_WHT2 { 2 } else { 0 };
            let mut a = t[0] >> sh;
            let mut c = t[1] >> sh;
            let mut d = t[2] >> sh;
            let mut b = t[3] >> sh;
            a = a.wrapping_add(c);
            d = d.wrapping_sub(b);
            let e = a.wrapping_sub(d) >> 1;
            b = e.wrapping_sub(b);
            c = e.wrapping_sub(c);
            a = a.wrapping_sub(b);
            d = d.wrapping_add(c);
            t[0] = a;
            t[1] = b;
            t[2] = c;
            t[3] = d;
        }
    }
}

/// the first N values of t as an array
#[inline(always)]
fn sub<const N: usize>(t: &mut [i32; 64]) -> &mut [i32; N] {
    (&mut t[..N]).try_into().unwrap()
}

/// inverse ADST4 (sinpi)
fn adst4(t: &mut [i32; 64]) {
    let (t0, t1, t2, t3) = (t[0] as i64, t[1] as i64, t[2] as i64, t[3] as i64);
    let mut s0 = t0 * 1321; // SINPI_1_9
    let mut s1 = t0 * 2482; // SINPI_2_9
    let s2 = t1 * 3344; // SINPI_3_9
    let mut s3 = t2 * 3803; // SINPI_4_9
    let s4 = t2 * 1321;
    let s5 = t3 * 2482;
    let s6 = t3 * 3803;
    let b7 = t0 - t2 + t3;
    s0 += s3;
    s1 -= s4;
    s3 = s2;
    let s2 = b7 * 3344;
    s0 += s5;
    s1 -= s6;
    let x0 = s0 + s3;
    let x1 = s1 + s3;
    let x3 = s0 + s1 - s3;
    t[0] = ((x0 + 2048) >> 12) as i32;
    t[1] = ((x1 + 2048) >> 12) as i32;
    t[2] = ((s2 + 2048) >> 12) as i32;
    t[3] = ((x3 + 2048) >> 12) as i32;
}
