// etx -- AV1 encoder: transform blocks of the tile encoder Et  [src/img/av1_etx.s]
//
//   txblocks(plane) -> txSz                the plane's transform blocks of the current block
//                                          inside the frame -> tbl / tbn
//   residual(plane, xy, txSz)              source - CurrFrame -> res (i32, stride 64)
//   satd(w, h) -> u64                      sum of |4x4 Hadamard coefficients| of res
//   ssd(plane, xy, w | h << 16) -> u64     SSD of source and CurrFrame inside the image
//   quant(plane, xy, txSz, txType) -> u64  forward transform of res and quantization with RDOQ ->
//                                          out (signed levels of the coded tw x th area);
//                                          transform-domain distortion
//   txbcode(plane, xy, levels, txSz, txType, update)   coeffs() through Ec::put / lit; update:
//                                          contexts and TxTypes as the decoder sets them
//   txbrate(plane, xy, levels, txSz, txType) -> u64    Q8 bits of txbcode
//   recon(plane, xy, levels, txSz, txType) reconstruction (the decoder's recon)
//   uvtxtype(txSz) -> u32                  compute_tx_type of a chroma transform block
//   tryplane(plane, mode, txsearch) -> u64 SSD   prediction, quantization and reconstruction of
//                                          the plane's transform blocks with mode (txsearch: luma
//                                          transform type among TP_SEARCH by J) -> qb / txb /
//                                          ntxb; contexts restored
//
// Model: tools/av1_enc_ref.py (tx_blocks, residual_of, satd, ssd, quant_txb, rdoq, eob_ctx,
// txb_rate, code_txb, set_ctx, recon_txb, try_plane, fwd2d, fwht4x4).  xy = x | y << 16 (plane
// samples) everywhere.  Levels live in the Et arrays (Lv::*) so that the TS borrows stay apart.

use crate::coef::{Cf, EOB_CDF, SCANS};
use crate::enc::Et;
use crate::ftx;
use crate::tab::*;

const TX_4X4: u32 = 0;
const TX_32X32: u32 = 3;
const DCT_DCT: u32 = 0;
const BLOCK_INVALID: u32 = 22;
const TX_SET_INTRA_1: u32 = 1;
const SINPI_1_9: i64 = 1321;
const SINPI_2_9: i64 = 2482;
const SINPI_3_9: i64 = 3344;
const SINPI_4_9: i64 = 3803;

// 1D kinds (0 DCT, 1 ADST, 2 identity) of the transform types 0..15 (model ROW_KIND / COL_KIND)
static F2_ROWKIND: [u8; 16] = [0, 0, 1, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
static F2_COLKIND: [u8; 16] = [0, 1, 0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
// symbol of a transform type in Tx_Type_Intra_Inv_Set1 / Set2 (types of the sets only)
static TC_SET1IDX: [u8; 16] = [1, 5, 6, 4, 0, 0, 0, 0, 0, 0, 2, 3, 0, 0, 0, 0];
static TC_SET2IDX: [u8; 16] = [1, 3, 4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
// luma transform types searched (model TX_SEARCH): DCT_DCT, ADST_DCT, DCT_ADST, ADST_ADST, IDTX
static TP_SEARCH: [u32; 5] = [0, 1, 2, 3, 9];

/// where a level array lives (the assembly passes pointers)
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lv {
    Out,             // ET_OUT
    Tmp,             // ET_TMP
    Qb(usize, usize), // ET_QB[plane] + offset
}

// n = Min(tw, 32) * Min(th, 32) of a transform size
#[inline]
pub(crate) fn tx_n(txsz: u32) -> usize {
    (TX_WIDTH[txsz as usize] as usize).min(32) * (TX_HEIGHT[txsz as usize] as usize).min(32)
}

// eobctx: context of the last coefficient (coeff_base_eob)
#[inline]
fn eobctx(c: u32, area: u32) -> u32 {
    if c == 0 {
        0
    } else if c <= area >> 3 {
        1
    } else if c <= area >> 2 {
        2
    } else {
        3
    }
}

// brcost: Q8 bits of the coeff_br symbols of level (> 2)
fn brcost(cdf: &[u16], level: u32) -> u32 {
    let mut rem = level - 3;
    let mut r = 0;
    for _ in 0..4 {
        let k = rem.min(3);
        rem -= k;
        r += crate::eec::cost(cdf, k);
        if k < 3 {
            break;
        }
    }
    r
}

// the bit length of v - 1 (bsr)
#[inline]
fn bsr(v: u32) -> u32 {
    31 - v.leading_zeros()
}

// fwd1d: 1D forward transform of t[0 .. 2^n) into o (kind 0 DCT, 1 ADST, 2 identity; model fdct
// / fadst / fidentity)
fn fwd1d(t: &mut [i32; 64], o: &mut [i32; 64], n: u32, kind: u8) {
    let nn = 1usize << n;
    match kind {
        0 => {
            // DCT: butterflies in place, result i -> coefficient brev(n, i)
            let brev: &[u8] = match n {
                2 => {
                    ftx::dct4((&mut t[..4]).try_into().unwrap());
                    &BREV2
                }
                3 => {
                    ftx::dct8((&mut t[..8]).try_into().unwrap());
                    &BREV3
                }
                4 => {
                    ftx::dct16((&mut t[..16]).try_into().unwrap());
                    &BREV4
                }
                5 => {
                    ftx::dct32((&mut t[..32]).try_into().unwrap());
                    &BREV5
                }
                _ => {
                    ftx::dct64(t);
                    &BREV6
                }
            };
            for i in 0..nn {
                o[brev[i] as usize] = t[i];
            }
        }
        1 if n == 2 => {
            // ADST4: the transpose of the inverse's matrix
            let (x0, x1, x2, x3) = (t[0] as i64, t[1] as i64, t[2] as i64, t[3] as i64);
            o[0] = ((x0 * SINPI_1_9 + x1 * SINPI_2_9 + x2 * SINPI_3_9 + x3 * SINPI_4_9 + 2048) >> 12) as i32;
            o[1] = (((x0 + x1 - x3) * SINPI_3_9 + 2048) >> 12) as i32;
            o[2] = ((x0 * SINPI_4_9 - x1 * SINPI_1_9 - x2 * SINPI_3_9 + x3 * SINPI_2_9 + 2048) >> 12) as i32;
            o[3] = ((x0 * SINPI_2_9 - x1 * SINPI_4_9 + x2 * SINPI_3_9 - x3 * SINPI_1_9 + 2048) >> 12) as i32;
        }
        1 => {
            // ADST8 / 16: input outp(i) (odd i negated), butterflies, output permutation
            let mut u = [0i32; 16];
            let (outp, inp): (&[u8], &[u8]) = if n == 3 { (&ADST_OUT8, &ADST_IN8) } else { (&ADST_OUT16, &ADST_IN16) };
            for i in 0..nn {
                let p = outp[i];
                let v = t[i];
                u[(p & 0x7F) as usize] = if p & 0x80 != 0 { v.wrapping_neg() } else { v };
            }
            if n == 3 {
                ftx::adst8((&mut u[..8]).try_into().unwrap());
            } else {
                ftx::adst16(&mut u);
            }
            for i in 0..nn {
                o[inp[i] as usize] = u[i];
            }
        }
        _ => {
            for i in 0..nn {
                let x = t[i] as i64;
                o[i] = match n {
                    2 => (x * 5793 + 2048) >> 12,
                    3 => x * 2,
                    4 => (x * 11586 + 2048) >> 12,
                    _ => x << 2,
                } as i32;
            }
        }
    }
}

impl Et<'_> {
    // ------------------------------------------------------------------ transform block list

    /// ae_txblocks
    pub(crate) fn txblocks(&mut self, plane: u32) -> u32 {
        let (sx, sy) = if plane != 0 { (self.t.h.ssx, self.t.h.ssy) } else { (0, 0) };
        let txsz = if self.t.s.ll != 0 {
            TX_4X4
        } else if plane == 0 {
            self.t.s.txsz
        } else {
            self.t.uvtx()
        };
        let tw = TX_WIDTH[txsz as usize] as u32;
        let th = TX_HEIGHT[txsz as usize] as u32;
        let mut bs = SUBSAMPLED_SIZE[(self.t.s.misize * 4 + sx * 2 + sy) as usize] as u32;
        if bs >= BLOCK_INVALID {
            bs = 0;
        }
        let bw = (NUM_4X4_BLOCKS_WIDE[bs as usize] as u32) << 2;
        let bh = (NUM_4X4_BLOCKS_HIGH[bs as usize] as u32) << 2;
        let x0 = (self.t.s.micol >> sx) << 2;
        let max_x = (self.t.h.micols << 2) >> sx;
        let y0 = (self.t.s.mirow >> sy) << 2;
        let max_y = (self.t.h.mirows << 2) >> sy;
        let mut n = 0usize;
        let mut y = 0;
        while y < bh {
            let mut x = 0;
            while x < bw {
                if x0 + x < max_x && y0 + y < max_y && n < 256 {
                    self.tbl[n] = (x0 + x) | (y0 + y) << 16;
                    n += 1;
                }
                x += tw;
            }
            y += th;
        }
        self.tbn = n as u32;
        txsz
    }

    // ------------------------------------------------------------------ residual, SATD, SSD

    /// ae_residual: res[i][j] = source - CurrFrame
    pub(crate) fn residual(&mut self, plane: u32, xy: u32, txsz: u32) {
        let p = plane as usize;
        let (x, y) = ((xy & 0xFFFF) as usize, (xy >> 16) as usize);
        let w = TX_WIDTH[txsz as usize] as usize;
        let h = TX_HEIGHT[txsz as usize] as usize;
        let sst = self.sst[p] as usize;
        let src = self.spl[p];
        let (cur, ps) = (&self.t.f.pl[p], self.t.f.ps[p] as usize);
        for i in 0..h {
            let s = &src[(y + i) * sst + x..(y + i) * sst + x + w];
            let c = &cur[(y + i) * ps + x..(y + i) * ps + x + w];
            let r = &mut self.res[i * 64..i * 64 + w];
            for ((r, &s), &c) in r.iter_mut().zip(s).zip(c) {
                *r = s as i32 - c as i32;
            }
        }
    }

    /// ae_satd: sum over the 4x4 blocks of res (w x h) of the absolute values of the Hadamard
    /// transform (rows, then columns)
    pub(crate) fn satd(&self, w: u32, h: u32) -> u64 {
        let mut sum = 0u64;
        let mut y = 0;
        while y < h as usize {
            let mut x = 0;
            while x < w as usize {
                let mut t = [0i32; 16];
                for k in 0..4 {
                    let r = &self.res[(y + k) * 64 + x..(y + k) * 64 + x + 4];
                    let s0 = r[0].wrapping_add(r[1]);
                    let d0 = r[0].wrapping_sub(r[1]);
                    let s1 = r[2].wrapping_add(r[3]);
                    let d1 = r[2].wrapping_sub(r[3]);
                    t[k * 4] = s0.wrapping_add(s1);
                    t[k * 4 + 1] = s0.wrapping_sub(s1);
                    t[k * 4 + 2] = d0.wrapping_add(d1);
                    t[k * 4 + 3] = d0.wrapping_sub(d1);
                }
                for i in 0..4 {
                    let s0 = t[i].wrapping_add(t[4 + i]);
                    let d0 = t[i].wrapping_sub(t[4 + i]);
                    let s1 = t[8 + i].wrapping_add(t[12 + i]);
                    let d1 = t[8 + i].wrapping_sub(t[12 + i]);
                    sum += s0.wrapping_add(s1).wrapping_abs() as u32 as u64;
                    sum += s0.wrapping_sub(s1).wrapping_abs() as u32 as u64;
                    sum += d0.wrapping_add(d1).wrapping_abs() as u32 as u64;
                    sum += d0.wrapping_sub(d1).wrapping_abs() as u32 as u64;
                }
                x += 4;
            }
            y += 4;
        }
        sum
    }

    /// ae_ssd: SSD of source and CurrFrame over the part of the w x h block at xy inside the
    /// image (sw / sh)
    pub(crate) fn ssd(&self, plane: u32, xy: u32, wh: u32) -> u64 {
        let p = plane as usize;
        let (x, y) = (xy & 0xFFFF, xy >> 16);
        let (mut w, mut h) = (wh & 0xFFFF, wh >> 16);
        let cx = self.sw[p] as i32 - x as i32;
        if cx <= 0 {
            return 0;
        }
        w = w.min(cx as u32);
        let cy = self.sh[p] as i32 - y as i32;
        if cy <= 0 {
            return 0;
        }
        h = h.min(cy as u32);
        let sst = self.sst[p] as usize;
        let src = self.spl[p];
        let (cur, ps) = (&self.t.f.pl[p], self.t.f.ps[p] as usize);
        let (x, y, w) = (x as usize, y as usize, w as usize);
        let mut sum = 0u64;
        for i in 0..h as usize {
            let s = &src[(y + i) * sst + x..(y + i) * sst + x + w];
            let c = &cur[(y + i) * ps + x..(y + i) * ps + x + w];
            for (&s, &c) in s.iter().zip(c) {
                let d = s as i32 - c as i32;
                sum += d.wrapping_mul(d) as u32 as u64;
            }
        }
        sum
    }

    // ------------------------------------------------------------------ forward transforms

    // fwd2d: res (h x w) -> c: rows, then columns (model fwd2d)
    fn fwd2d(&mut self, txsz: u32, txtype: u32) {
        let lw = TX_WIDTH_LOG2[txsz as usize] as u32;
        let lh = TX_HEIGHT_LOG2[txsz as usize] as u32;
        let (w, h) = (1usize << lw, 1usize << lh);
        let rk = F2_ROWKIND[(txtype & 15) as usize];
        let ck = F2_COLKIND[(txtype & 15) as usize];
        let mut t = [0i32; 64];
        let mut o = [0i32; 64];
        for i in 0..h {
            t[..w].copy_from_slice(&self.res[i * 64..i * 64 + w]);
            fwd1d(&mut t, &mut o, lw, rk);
            self.c[i * 64..i * 64 + w].copy_from_slice(&o[..w]);
        }
        for j in 0..w {
            for i in 0..h {
                t[i] = self.c[i * 64 + j];
            }
            fwd1d(&mut t, &mut o, lh, ck);
            for i in 0..h {
                self.c[i * 64 + j] = o[i];
            }
        }
    }

    // fwht: lossless: libaom's av1_fwht4x4 of res (columns, then rows, times 4), the levels
    // C / 4 (sign and magnitude) -> out[16]
    fn fwht(&mut self) {
        #[inline(always)]
        fn bf(a: i32, b: i32, c: i32, d: i32) -> (i32, i32, i32, i32) {
            let a1 = a.wrapping_add(b);
            let d1 = d.wrapping_sub(c);
            let e1 = a1.wrapping_sub(d1) >> 1;
            let b1 = e1.wrapping_sub(b);
            let c1 = e1.wrapping_sub(c);
            (a1.wrapping_sub(c1), b1, c1, d1.wrapping_add(b1))
        }
        let mut t = [0i32; 16];
        for i in 0..4 {
            let r = &self.res;
            let (a1, b1, c1, d1) = bf(r[i], r[64 + i], r[128 + i], r[192 + i]);
            t[i] = a1;
            t[4 + i] = c1;
            t[8 + i] = d1;
            t[12 + i] = b1;
        }
        for k in 0..4 {
            let (a1, b1, c1, d1) = bf(t[4 * k], t[4 * k + 1], t[4 * k + 2], t[4 * k + 3]);
            // outputs a1, c1, d1, b1 (times 4, then / 4 as sign-magnitude = the values themselves)
            self.out[4 * k] = a1;
            self.out[4 * k + 1] = c1;
            self.out[4 * k + 2] = d1;
            self.out[4 * k + 3] = b1;
        }
    }

    // ------------------------------------------------------------------ quantization

    /// ae_quant -> transform-domain distortion
    pub(crate) fn quant(&mut self, plane: u32, xy: u32, txsz: u32, txtype: u32) -> u64 {
        if self.t.s.ll != 0 {
            self.fwht();
            return 0;
        }
        self.fwd2d(txsz, txtype);
        let k = ftx::KQ[txsz as usize] as u64; // (ET_KQ: written only)
        let w = TX_WIDTH[txsz as usize] as usize;
        let h = TX_HEIGHT[txsz as usize] as usize;
        let tw = w.min(32);
        let n = tw * h.min(32);
        let p = plane as usize;
        let (dcq, acq) = (self.dcq[p] as u64, self.acq[p] as u64);
        let mut dz = 0u64;
        for i in 0..h {
            for j in 0..w {
                let v = self.c[i * 64 + j] as i64;
                let neg = v < 0;
                let a = (v.wrapping_abs() as u64).wrapping_mul(k); // |C| * K
                if i < 32 && j < 32 {
                    let pos = i * tw + j;
                    self.a[pos] = a;
                    self.neg[pos] = neg as u8;
                    let q = if pos == 0 { dcq } else { acq };
                    self.lv[pos] = ((a + (q << 15)) / (q << 16)) as u32 as i32;
                } else {
                    let a8 = a >> 8;
                    dz = dz.wrapping_add(a8.wrapping_mul(a8) >> 6);
                }
            }
        }
        self.rdoq(plane, xy, txsz, txtype);
        // distortion, signs
        let mut dist = dz;
        for s in 0..n {
            let q = if s == 0 { dcq } else { acq };
            let l = self.out[s] as u32 as u64;
            let d = ((self.a[s] >> 8) as i64).wrapping_sub((l.wrapping_mul(q) << 8) as i64);
            dist = dist.wrapping_add((d.wrapping_mul(d) as u64) >> 6);
            if l != 0 && self.neg[s] != 0 {
                self.out[s] = (l as u32 as i32).wrapping_neg();
            }
        }
        dist
    }

    /// ae_cfsetup: the CF record of a transform block: plane, x4 / y4 / w4 / h4, txSz, txSzCtx,
    /// ptype, class 2D, bwl / width / height / area (adjusted size)
    fn cfsetup(&mut self, plane: u32, xy: u32, txsz: u32) {
        let t = txsz as usize;
        let adj = ADJUSTED_TX_SIZE[t] as usize;
        let bwl = TX_WIDTH_LOG2[adj] as u32;
        let height = TX_HEIGHT[adj] as u32;
        self.cf = Cf {
            eob: 0,
            cul: 0,
            dccat: 0,
            scan: &[],
            base: 0,
            br: 0,
            eobpt: 0,
            plane,
            ptype: (plane != 0) as u32,
            x4: (xy & 0xFFFF) >> 2,
            y4: xy >> 18,
            txsz,
            w4: TX_WIDTH[t] as u32 >> 2,
            h4: TX_HEIGHT[t] as u32 >> 2,
            txctx: (TX_SIZE_SQR[t] as u32 + TX_SIZE_SQR_UP[t] as u32 + 1) >> 1,
            class: 0,
            height,
            bwl,
            width: 1 << bwl,
            area: height << bwl,
        };
    }

    // the levels at l
    #[inline]
    fn lvl(&self, l: Lv) -> &[i32] {
        match l {
            Lv::Out => &self.out[..],
            Lv::Tmp => &self.tmp[..],
            Lv::Qb(p, o) => &self.qb[p][o..],
        }
    }

    // ae_rdoq: lv (rounded levels), a -> out (unsigned levels): model rdoq
    fn rdoq(&mut self, plane: u32, xy: u32, txsz: u32, txtype: u32) {
        let n = tx_n(txsz);
        let scan = SCANS[txsz as usize][0];
        let p = plane as usize;
        self.out[..n].fill(0);
        self.t.s.quant[..n].fill(0);
        // eob of the rounded levels
        let mut eob = n;
        while eob > 0 && self.lv[scan[eob - 1] as usize] == 0 {
            eob -= 1;
        }
        if eob == 0 {
            return;
        }
        self.cfsetup(plane, xy, txsz);
        // CDF offsets (u16 entries)
        let cf = self.cf;
        let g = cf.txctx * 2 + cf.ptype;
        let cbase = CDF_CBASE + (g * 42 * 5) as usize;
        let cbaseeob = CDF_CBASEEOB + (g * 4 * 4) as usize;
        let cbr = CDF_CBR + ((cf.txctx.min(3) * 2 + cf.ptype) * 21 * 5) as usize;
        let dcsign = CDF_DCSIGN + ((self.t.cf_dcctx(&cf) + cf.ptype * 3) * 3) as usize;
        let lam = self.lam;
        // greedy pass in reverse scan order
        for c in (0..eob).rev() {
            let pos = scan[c] as usize;
            let l0 = self.lv[pos] as u32;
            if l0 == 0 {
                continue;
            }
            let q = if pos != 0 { self.acq[p] } else { self.dcq[p] } as u64;
            let a8 = self.a[pos] >> 8;
            let last = c == eob - 1;
            let cx = if last { eobctx(c as u32, cf.area) } else { self.t.cf_basectx(&cf, pos as u32) };
            let brx = self.t.cf_brctx(&cf, pos as u32);
            let mut best_j = u64::MAX;
            let mut best = 0u32;
            for cand in 0..2 {
                let l = l0 - cand;
                if l == 0 && last {
                    continue; // the last coefficient stays nonzero
                }
                // rate
                let cdf = &self.t.s.cdf;
                let mut r = if last {
                    crate::eec::cost(&cdf[cbaseeob + cx as usize * 4..], l.min(3) - 1)
                } else {
                    crate::eec::cost(&cdf[cbase + cx as usize * 5..], l.min(3))
                };
                if l > 2 {
                    r += brcost(&cdf[cbr + brx as usize * 5..], l);
                }
                if l != 0 {
                    // sign (DC: the cheaper sign of dc_sign), Golomb remainder of levels > 14
                    r += if c != 0 {
                        256
                    } else {
                        crate::eec::cost(&cdf[dcsign..], 0).min(crate::eec::cost(&cdf[dcsign..], 1))
                    };
                    if l > 14 {
                        r += (bsr(l - 14) * 2 + 1) << 8;
                    }
                }
                // J = ((a8 - l * q * 256)^2 >> 6) + lambda * r
                let d = (a8 as i64).wrapping_sub(((l as u64).wrapping_mul(q) << 8) as i64);
                let j = ((d.wrapping_mul(d) as u64) >> 6).wrapping_add((r as u64).wrapping_mul(lam));
                if j < best_j {
                    best_j = j;
                    best = l;
                }
            }
            self.out[pos] = best as i32;
            self.t.s.quant[pos] = best.min(15) as i32;
        }
        // trailing ones while J decreases
        let mut jcur = self.jtot(Lv::Out, plane, xy, txsz, txtype);
        let mut trim = 8;
        'trim: loop {
            let mut c = eob;
            while c > 0 && self.out[scan[c - 1] as usize] == 0 {
                c -= 1;
            }
            if c == 0 {
                break 'trim;
            }
            let pos = scan[c - 1] as usize;
            if self.out[pos] != 1 {
                break 'trim;
            }
            self.tmp[..n].copy_from_slice(&self.out[..n]);
            self.tmp[pos] = 0;
            let j = self.jtot(Lv::Tmp, plane, xy, txsz, txtype);
            if j >= jcur {
                break 'trim;
            }
            jcur = j;
            self.out[pos] = 0;
            trim -= 1;
            if trim == 0 {
                break 'trim;
            }
        }
        // all zero when that is cheaper
        if self.out[..n].iter().all(|&v| v == 0) {
            return;
        }
        self.tmp[..n].fill(0);
        let j = self.jtot(Lv::Tmp, plane, xy, txsz, txtype);
        if j < jcur {
            self.out[..n].fill(0);
        }
    }

    // rq_jtot: J of levels (distortion of the n coefficients + lambda * rate)
    fn jtot(&mut self, l: Lv, plane: u32, xy: u32, txsz: u32, txtype: u32) -> u64 {
        let p = plane as usize;
        let n = tx_n(txsz);
        let mut dist = 0u64;
        {
            let lv = self.lvl(l);
            for i in 0..n {
                let q = if i != 0 { self.acq[p] } else { self.dcq[p] } as u64;
                let d = ((self.a[i] >> 8) as i64).wrapping_sub(((lv[i] as u32 as u64).wrapping_mul(q) << 8) as i64);
                dist = dist.wrapping_add((d.wrapping_mul(d) as u64) >> 6);
            }
        }
        let r = self.txbrate(plane, xy, l, txsz, txtype);
        r.wrapping_mul(self.lam).wrapping_add(dist)
    }

    // ------------------------------------------------------------------ coefficient syntax

    /// ae_txbrate -> Q8 bits
    pub(crate) fn txbrate(&mut self, plane: u32, xy: u32, l: Lv, txsz: u32, txtype: u32) -> u64 {
        let (est, bits) = (self.ec.est, self.ec.bits);
        self.ec.est = true;
        self.ec.bits = 0;
        self.txbcode(plane, xy, l, txsz, txtype, false);
        let r = self.ec.bits;
        self.ec.est = est;
        self.ec.bits = bits;
        r
    }

    /// ae_txbcode
    pub(crate) fn txbcode(&mut self, plane: u32, xy: u32, l: Lv, txsz: u32, txtype: u32, update: bool) {
        let t = txsz as usize;
        let lw = TX_WIDTH_LOG2[t] as u32;
        let lh = TX_HEIGHT_LOG2[t] as u32;
        let n = tx_n(txsz);
        let scan = SCANS[t][0];
        let mut cul = 0u32; // culLevel
        let mut dcc = 0u32; // dcCategory
        self.cfsetup(plane, xy, txsz);
        let cf = self.cf;
        // eob (last nonzero in scan order + 1)
        let mut eob = n;
        {
            let lv = self.lvl(l);
            while eob > 0 && lv[scan[eob - 1] as usize] == 0 {
                eob -= 1;
            }
        }
        // all_zero
        let az = self.t.cf_azctx(&cf);
        let off = CDF_TXBSKIP + ((cf.txctx * 13 + az) * 3) as usize;
        self.ec.put(&mut self.t.s.cdf[off..], (eob == 0) as u32, 2);
        if eob == 0 {
            if plane == 0 && update {
                self.t.cf_settxt(&cf, DCT_DCT);
            }
        } else {
            if plane == 0 {
                // transform_type (luma)
                let set = self.t.cf_txset(txsz);
                if set != 0 && self.t.h.baseq != 0 {
                    let idx = (TX_SIZE_SQR[t] as u32 * 13 + self.t.s.ymode) as usize; // intraDir
                    if set == TX_SET_INTRA_1 {
                        let off = CDF_ITX1 + idx * 8;
                        self.ec.put(&mut self.t.s.cdf[off..], TC_SET1IDX[txtype as usize] as u32, 7);
                    } else {
                        let off = CDF_ITX2 + idx * 6;
                        self.ec.put(&mut self.t.s.cdf[off..], TC_SET2IDX[txtype as usize] as u32, 5);
                    }
                }
                if update {
                    self.t.cf_settxt(&cf, txtype);
                }
            }
            // eob_pt: eob in (2^(eobPt - 2), 2^(eobPt - 1)]
            let eob32 = eob as u32;
            let eob_pt = if eob32 == 1 { 1 } else { bsr(eob32 - 1) + 2 };
            let ems = lw.min(5) + lh.min(5) - 4; // eobMultisize
            let ent = (ems + 6) as usize; // entries per cdf
            let off = if ems < 5 {
                EOB_CDF[ems as usize] + (cf.ptype * 2) as usize * ent
            } else {
                (if ems == 5 { CDF_EOB512 } else { CDF_EOB1024 }) + cf.ptype as usize * ent
            };
            self.ec.put(&mut self.t.s.cdf[off..], eob_pt - 1, ems + 5);
            if eob_pt >= 3 {
                // eob_extra and the remaining bits of eob - (2^(eobPt - 2) + 1)
                let extra = eob32 - ((1 << (eob_pt - 2)) + 1);
                let off = CDF_EOBEXTRA + (((cf.txctx * 2 + cf.ptype) * 9 + eob_pt - 3) * 3) as usize;
                self.ec.put(&mut self.t.s.cdf[off..], (extra >> (eob_pt - 3)) & 1, 2);
                let nb = eob_pt - 3; // bits below the top one
                if nb != 0 {
                    self.ec.lit(extra & ((1 << nb) - 1), nb);
                }
            }
            // CDF offsets of coeff_base / coeff_br
            let g = cf.txctx * 2 + cf.ptype;
            let cbase = CDF_CBASE + (g * 42 * 5) as usize;
            let cbr = CDF_CBR + ((cf.txctx.min(3) * 2 + cf.ptype) * 21 * 5) as usize;
            self.t.s.quant[..n].fill(0);
            for c in (0..eob).rev() {
                let pos = scan[c] as usize;
                let level = self.lvl(l)[pos].unsigned_abs();
                if c == eob - 1 {
                    let ctx = eobctx(c as u32, cf.area);
                    let off = CDF_CBASEEOB + ((ctx + g * 4) * 4) as usize;
                    self.ec.put(&mut self.t.s.cdf[off..], level.min(3) - 1, 3);
                } else {
                    let ctx = self.t.cf_basectx(&cf, pos as u32);
                    let off = cbase + ctx as usize * 5;
                    self.ec.put(&mut self.t.s.cdf[off..], level.min(3), 4);
                }
                if level > 2 {
                    let ctx = self.t.cf_brctx(&cf, pos as u32);
                    let off = cbr + ctx as usize * 5;
                    let mut rem = level - 3;
                    for _ in 0..4 {
                        let k = rem.min(3);
                        rem -= k;
                        self.ec.put(&mut self.t.s.cdf[off..], k, 4);
                        if k < 3 {
                            break;
                        }
                    }
                }
                self.t.s.quant[pos] = level.min(15) as i32;
            }
            // signs
            let dcsign = CDF_DCSIGN + ((self.t.cf_dcctx(&cf) + cf.ptype * 3) * 3) as usize;
            for c in 0..eob {
                let pos = scan[c] as usize;
                let v = self.lvl(l)[pos];
                if v == 0 {
                    continue;
                }
                let sign = (v < 0) as u32;
                if c == 0 {
                    self.ec.put(&mut self.t.s.cdf[dcsign..], sign, 2);
                } else {
                    self.ec.lit(sign, 1);
                }
                let a = v.unsigned_abs();
                if a > 14 {
                    // Golomb: length - 1 zeros, a one, the length - 1 low bits of x = |v| - 14
                    let x = a - 14;
                    let len1 = bsr(x);
                    if len1 != 0 {
                        self.ec.lit(0, len1);
                    }
                    self.ec.lit(1, 1);
                    if len1 != 0 {
                        self.ec.lit(x & ((1 << len1) - 1), len1);
                    }
                }
                if pos == 0 {
                    dcc = 2 - sign; // 1 negative, 2 positive
                }
                cul = cul.wrapping_add(a);
            }
            cul = cul.min(63);
        }
        if update {
            self.setctx(plane, cul, dcc);
        }
    }

    // ae_setctx: above / left level and dc contexts of the transform block in cf (x4 .. x4 + w4,
    // y4 .. y4 + h4)
    fn setctx(&mut self, plane: u32, cul: u32, dcc: u32) {
        let p = plane as usize;
        let cf = self.cf;
        let a = p * self.t.s.acols as usize + cf.x4 as usize; // Above*Context[plane] + x4
        let w4 = cf.w4 as usize;
        self.t.s.alvl[a..a + w4].fill(cul as u8);
        self.t.s.adc[a..a + w4].fill(dcc as u8);
        for k in 0..cf.h4 {
            let y = ((cf.y4 + k) & 31) as usize;
            self.t.s.llvl[p][y] = cul as u8;
            self.t.s.ldc[p][y] = dcc as u8;
        }
    }

    // ------------------------------------------------------------------ reconstruction

    /// ae_uvtxtype: compute_tx_type(plane > 0, txSz)
    pub(crate) fn uvtxtype(&self, txsz: u32) -> u32 {
        if self.t.s.ll != 0 || TX_SIZE_SQR_UP[txsz as usize] as u32 > TX_32X32 {
            return 0;
        }
        let set = self.t.cf_txset(txsz);
        let t = MODE_TO_TXFM[self.t.s.uvmode as usize] as u32;
        if TX_TYPE_IN_SET_INTRA[(set * 16 + t) as usize] != 0 { t } else { 0 }
    }

    /// ae_recon: reconstruct() of a transform block with these levels (nothing when all are
    /// zero)
    pub(crate) fn recon(&mut self, plane: u32, xy: u32, l: Lv, txsz: u32, txtype: u32) {
        let n = tx_n(txsz);
        // copy into Quant; eob = 1 when only the DC level is nonzero (the decoder's DC path)
        let mut beyond = false; // nonzero beyond position 0
        let mut any = false;
        for i in 0..n {
            let v = self.lvl(l)[i];
            self.t.s.quant[i] = v;
            if v != 0 {
                any = true;
                if i != 0 {
                    beyond = true;
                }
            }
        }
        if !any {
            return;
        }
        self.t.s.eob = beyond as u32 + 1;
        let t = if self.t.s.ll != 0 {
            DCT_DCT
        } else if plane != 0 {
            self.uvtxtype(txsz)
        } else {
            txtype
        };
        self.t.s.ptxt = t;
        self.t.s.ppl = plane;
        self.t.s.px = xy & 0xFFFF;
        self.t.s.py = xy >> 16;
        self.t.s.ptx = txsz;
        self.t.recon();
    }

    // ------------------------------------------------------------------ one plane of a block

    /// ae_tryplane -> SSD
    pub(crate) fn tryplane(&mut self, plane: u32, mode: u32, txsearch: bool) -> u64 {
        let p = plane as usize;
        let txsz = self.txblocks(plane);
        let n = tx_n(txsz);
        if plane == 0 {
            self.t.s.ymode = mode;
        } else {
            self.t.s.uvmode = mode;
        }
        // block origin in the plane, context spans
        let (sx, sy) = if plane != 0 { (self.t.h.ssx, self.t.h.ssy) } else { (0, 0) };
        let ax = self.t.s.micol >> sx; // above start (x4)
        let x0 = ax << 2;
        let an = (self.t.s.bw4 >> sx).max(1);
        let ry = self.t.s.mirow >> sy;
        let y0 = ry << 2;
        let ls = ry & 31; // left start (y4 & 31)
        let ln = (self.t.s.bh4 >> sy).max(1);
        let spans = [ax, an, ls, ln];
        // save the contexts of the spans
        let save = self.ctxcopy(p, spans);
        let mut dist = 0u64;
        let tw = TX_WIDTH[txsz as usize] as u32;
        let th = TX_HEIGHT[txsz as usize] as u32;
        for k in 0..self.tbn as usize {
            let xy = self.tbl[k];
            // prediction (transform_block's first part)
            self.t.s.ppl = plane;
            self.t.s.ptx = txsz;
            self.t.s.pbx = ((xy & 0xFFFF) - x0) >> 2;
            self.t.s.pby = ((xy >> 16) - y0) >> 2;
            self.t.tbpred(x0, y0);
            self.residual(plane, xy, txsz);
            // destination of the levels: qb[plane] + k * n
            let dst = Lv::Qb(p, k * n);
            let mut txtype;
            if plane == 0 && txsearch && self.t.s.ll == 0 && self.t.cf_txset(txsz) != 0 {
                // transform type search: J = transform-domain distortion + lambda * rate
                let mut best_j = u64::MAX;
                let mut best_t = 0;
                for &t in TP_SEARCH.iter() {
                    let d = self.quant(plane, xy, txsz, t);
                    let r = self.txbrate(plane, xy, Lv::Out, txsz, t);
                    let j = r.wrapping_mul(self.lam).wrapping_add(d);
                    if j < best_j {
                        best_j = j;
                        best_t = t;
                        self.qb[p][k * n..k * n + n].copy_from_slice(&self.out[..n]);
                    }
                }
                txtype = best_t;
            } else {
                // DCT_DCT (luma, lossless)
                txtype = if self.t.s.ll == 0 && plane != 0 { self.uvtxtype(txsz) } else { DCT_DCT };
                self.quant(plane, xy, txsz, txtype);
                self.qb[p][k * n..k * n + n].copy_from_slice(&self.out[..n]);
            }
            // all zero: DCT_DCT; culLevel / dcCategory
            let lv = &self.qb[p][k * n..k * n + n];
            let sum = lv.iter().fold(0u32, |s, &v| s.wrapping_add(v.unsigned_abs()));
            let dc = lv[0];
            if sum == 0 {
                txtype = DCT_DCT;
            }
            self.recon(plane, xy, dst, txsz, txtype);
            self.txb[p][k] = xy as u64 | (txtype as u64) << 32;
            // SSD
            dist += self.ssd(plane, xy, tw | th << 16);
            // transform_block's end, contexts of this block
            self.t.tbflags();
            self.cfsetup(plane, xy, txsz);
            let dcc = if dc == 0 {
                0
            } else if dc > 0 {
                2
            } else {
                1
            };
            self.setctx(plane, sum.min(63), dcc);
        }
        self.ntxb[p] = self.tbn;
        // restore the contexts
        self.ctxrest(p, spans, &save);
        dist
    }

    // tp_ctxcopy: save the span contexts (16 bytes per array: above lvl / dc, left lvl / dc)
    fn ctxcopy(&mut self, p: usize, sp: [u32; 4]) -> [[u8; 16]; 4] {
        let mut s = [[0u8; 16]; 4];
        let (a, an, l, ln) = (sp[0] as usize, sp[1] as usize, sp[2] as usize, sp[3] as usize);
        let a = p * self.t.s.acols as usize + a;
        s[0][..an].copy_from_slice(&self.t.s.alvl[a..a + an]);
        s[1][..an].copy_from_slice(&self.t.s.adc[a..a + an]);
        s[2][..ln].copy_from_slice(&self.t.s.llvl[p][l..l + ln]);
        s[3][..ln].copy_from_slice(&self.t.s.ldc[p][l..l + ln]);
        s
    }

    // tp_ctxrest: restore them
    fn ctxrest(&mut self, p: usize, sp: [u32; 4], s: &[[u8; 16]; 4]) {
        let (a, an, l, ln) = (sp[0] as usize, sp[1] as usize, sp[2] as usize, sp[3] as usize);
        let a = p * self.t.s.acols as usize + a;
        self.t.s.alvl[a..a + an].copy_from_slice(&s[0][..an]);
        self.t.s.adc[a..a + an].copy_from_slice(&s[1][..an]);
        self.t.s.llvl[p][l..l + ln].copy_from_slice(&s[2][..ln]);
        self.t.s.ldc[p][l..l + ln].copy_from_slice(&s[3][..ln]);
    }
}
