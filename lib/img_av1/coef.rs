// coef -- AV1 coefficient syntax (specification 5.11.39 .. 5.11.48, CDF selection 8.3.2)
//                                                                    [src/img/av1_coef.s]
//
//   T::coeffs() -> eob     coeffs( ppl, px, py, ptx ): Quant (scan positions of the (at most
//                          32x32) coded area), PlaneTxType, the above / left level and dc
//                          contexts, TxTypes of luma
//
// also for the encoder (av1_e*.s; cf = the CF record):
//   T::cf_azctx(cf), cf_settxt(cf, txType), cf_txset(txSz), cf_pttype(cf), cf_basectx(cf, pos),
//   cf_brctx(cf, pos), cf_dcctx(cf); SCANS, EOB_CDF

use crate::blk::{T, BLOCK_INVALID, MIB_TXT, TX_16X64, TX_32X32, TX_64X16};
use crate::dec::qindex;
use crate::tab::*;

const TX_SET_DCTONLY: u32 = 0;
const TX_SET_INTRA_1: u32 = 1;
const TX_SET_INTRA_2: u32 = 2;
const TX_16X16: u8 = 2;

/// CF: coefficient context record (the locals of av1_coeffs)
#[derive(Clone, Copy)]
pub(crate) struct Cf {
    pub plane: u32,
    pub x4: u32,
    pub y4: u32,
    pub w4: u32,
    pub h4: u32,
    pub txsz: u32,
    pub txctx: u32, // txSzCtx
    pub ptype: u32,
    pub eob: u32,
    pub cul: u32,   // culLevel
    pub dccat: u32, // dcCategory
    pub class: u32, // tx class of PlaneTxType
    pub bwl: u32,   // Tx_Width_Log2[ Adjusted_Tx_Size[ txSz ] ]
    pub height: u32, // Tx_Height[ Adjusted_Tx_Size[ txSz ] ]
    pub scan: &'static [u16],
    pub base: usize, // CDF offset of coeff_base for (txSzCtx, ptype)
    pub br: usize,   // CDF offset of coeff_br
    pub area: u32,   // height << bwl
    pub eobpt: u32,
    pub width: u32, // 1 << bwl
}

/// CDF offsets of eob_pt_16 .. eob_pt_256
pub(crate) static EOB_CDF: [usize; 5] = [CDF_EOB16, CDF_EOB32, CDF_EOB64, CDF_EOB128, CDF_EOB256];

/// scan tables per txSz: default, mrow, mcol
pub(crate) static SCANS: [[&[u16]; 3]; 19] = [
    [&DEFAULT_SCAN_4X4, &MROW_SCAN_4X4, &MCOL_SCAN_4X4],
    [&DEFAULT_SCAN_8X8, &MROW_SCAN_8X8, &MCOL_SCAN_8X8],
    [&DEFAULT_SCAN_16X16, &MROW_SCAN_16X16, &MCOL_SCAN_16X16],
    [&DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32],
    [&DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32],
    [&DEFAULT_SCAN_4X8, &MROW_SCAN_4X8, &MCOL_SCAN_4X8],
    [&DEFAULT_SCAN_8X4, &MROW_SCAN_8X4, &MCOL_SCAN_8X4],
    [&DEFAULT_SCAN_8X16, &MROW_SCAN_8X16, &MCOL_SCAN_8X16],
    [&DEFAULT_SCAN_16X8, &MROW_SCAN_16X8, &MCOL_SCAN_16X8],
    [&DEFAULT_SCAN_16X32, &DEFAULT_SCAN_16X32, &DEFAULT_SCAN_16X32],
    [&DEFAULT_SCAN_32X16, &DEFAULT_SCAN_32X16, &DEFAULT_SCAN_32X16],
    [&DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32],
    [&DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32, &DEFAULT_SCAN_32X32],
    [&DEFAULT_SCAN_4X16, &MROW_SCAN_4X16, &MCOL_SCAN_4X16],
    [&DEFAULT_SCAN_16X4, &MROW_SCAN_16X4, &MCOL_SCAN_16X4],
    [&DEFAULT_SCAN_8X32, &DEFAULT_SCAN_8X32, &DEFAULT_SCAN_8X32],
    [&DEFAULT_SCAN_32X8, &DEFAULT_SCAN_32X8, &DEFAULT_SCAN_32X8],
    [&DEFAULT_SCAN_16X32, &DEFAULT_SCAN_16X32, &DEFAULT_SCAN_16X32],
    [&DEFAULT_SCAN_32X16, &DEFAULT_SCAN_32X16, &DEFAULT_SCAN_32X16],
];

impl T<'_> {
    /// av1_coeffs: coeffs( ppl, px, py, ptx ) -> eob
    pub fn coeffs(&mut self) -> u32 {
        let plane = self.s.ppl;
        let tx = self.s.ptx;
        let txu = tx as usize;
        let w = TX_WIDTH[txu] as u32;
        let hh = TX_HEIGHT[txu] as u32;
        let mut cf = Cf {
            plane,
            x4: self.s.px >> 2,
            y4: self.s.py >> 2,
            w4: w >> 2,
            h4: hh >> 2,
            txsz: tx,
            txctx: (TX_SIZE_SQR_UP[txu] as u32 + TX_SIZE_SQR[txu] as u32 + 1) >> 1,
            ptype: (plane != 0) as u32,
            eob: 0,
            cul: 0,
            dccat: 0,
            class: 0,
            bwl: 0,
            height: 0,
            scan: &[],
            base: 0,
            br: 0,
            area: 0,
            eobpt: 0,
            width: 0,
        };
        // Quant = 0 (segEob entries)
        let mut n = (w * hh).min(1024);
        if tx == TX_16X64 || tx == TX_64X16 {
            n = 512;
        }
        self.s.quant[..n as usize].fill(0);
        // all_zero
        let ctx = self.cf_azctx(&cf);
        let az = self.sym(CDF_TXBSKIP + (ctx + cf.txctx * 13) as usize * 3, 2);
        if az != 0 {
            if plane == 0 {
                self.cf_settxt(&cf, 0); // DCT_DCT
            }
        } else {
            if plane == 0 {
                self.txtype(&cf);
            }
            let ptt = self.cf_pttype(&cf);
            self.s.ptxt = ptt;
            // tx class
            cf.class = match ptt {
                10 | 12 | 14 => 2, // V_DCT, V_ADST, V_FLIPADST: TX_CLASS_VERT
                11 | 13 | 15 => 1, // H_*: TX_CLASS_HORIZ
                _ => 0,
            };
            // scan: 0 default, 1 mrow (V_*), 2 mcol (H_*); IDTX and 2D: default
            let sk = match cf.class {
                2 => 1,
                1 => 2,
                _ => 0,
            };
            cf.scan = SCANS[txu][sk];
            // adjusted size for the contexts
            let adj = ADJUSTED_TX_SIZE[txu] as usize;
            cf.bwl = TX_WIDTH_LOG2[adj] as u32;
            cf.width = 1 << cf.bwl;
            cf.height = TX_HEIGHT[adj] as u32;
            cf.area = cf.height << cf.bwl;
            // eobPt
            let ems = (TX_WIDTH_LOG2[txu] as u32).min(5) + (TX_HEIGHT_LOG2[txu] as u32).min(5) - 4; // eobMultisize
            let c2 = (cf.class != 0) as u32;
            let nsym = (ems + 5) as usize;
            let per = ems + 6;
            let off = if ems < 5 {
                EOB_CDF[ems as usize] + ((cf.ptype * 2 + c2) * per) as usize
            } else if ems == 5 {
                CDF_EOB512 + (cf.ptype * per) as usize
            } else {
                CDF_EOB1024 + (cf.ptype * per) as usize
            };
            let ept = self.sym(off, nsym) + 1; // eobPt
            cf.eobpt = ept;
            let mut eob = ept;
            if ept >= 2 {
                eob = (1u32 << (ept - 2)) + 1;
                if ept >= 3 {
                    // eob_extra
                    let o = CDF_EOBEXTRA + (((cf.txctx * 2 + cf.ptype) * 9 + ept - 3) * 3) as usize;
                    if self.sym(o, 2) != 0 {
                        eob += 1u32.wrapping_shl(ept - 3);
                    }
                    let mut i = 1;
                    while i < ept - 2 {
                        if self.bool() != 0 {
                            eob = eob.wrapping_add(1u32.wrapping_shl(ept - 3 - i));
                        }
                        i += 1;
                    }
                }
            }
            cf.eob = eob;
            // CDF offsets of coeff_base / coeff_br for (txSzCtx, ptype)
            cf.base = CDF_CBASE + ((cf.txctx * 2 + cf.ptype) * 42 * 5) as usize;
            cf.br = CDF_CBR + ((cf.txctx.min(3) * 2 + cf.ptype) * 21 * 5) as usize;
            // levels in reverse scan order
            let scan = cf.scan;
            let mut c = eob as i64 - 1;
            while c >= 0 {
                let ci = c as u32;
                let pos = (scan[ci as usize] & 1023) as u32;
                let mut level;
                if ci == eob - 1 {
                    // coeff_base_eob
                    let mut cx = 0u32;
                    if ci != 0 {
                        cx = 1;
                        if ci > cf.area >> 3 {
                            cx = 2;
                            if ci > cf.area >> 2 {
                                cx = 3;
                            }
                        }
                    }
                    let o = CDF_CBASEEOB + ((cx + (cf.txctx * 2 + cf.ptype) * 4) * 4) as usize;
                    level = self.sym(o, 3) + 1;
                } else {
                    let bc = self.cf_basectx(&cf, pos);
                    level = self.sym(cf.base + bc as usize * 5, 4);
                }
                if level > 2 {
                    // NUM_BASE_LEVELS
                    let o = cf.br + self.cf_brctx(&cf, pos) as usize * 5;
                    for _ in 0..4 {
                        // COEFF_BASE_RANGE / ( BR_CDF_SIZE - 1 )
                        let k = self.sym(o, 4);
                        level += k;
                        if k < 3 {
                            break;
                        }
                    }
                }
                self.s.quant[pos as usize] = level as i32;
                c -= 1;
            }
            // signs, Golomb
            let mut bad = false;
            for ci in 0..eob {
                let pos = (scan[ci as usize] & 1023) as usize;
                let mut lv = self.s.quant[pos] as u32;
                let mut sign = 0;
                if lv != 0 {
                    if ci == 0 {
                        let dc = self.cf_dcctx(&cf);
                        sign = self.sym(CDF_DCSIGN + ((dc + cf.ptype * 3) * 3) as usize, 2);
                    } else {
                        sign = self.bool();
                    }
                }
                if lv > 14 {
                    // NUM_BASE_LEVELS + COEFF_BASE_RANGE
                    let mut len = 0u32;
                    loop {
                        len += 1;
                        if len > 32 {
                            bad = true;
                            break;
                        }
                        if self.bool() != 0 {
                            break;
                        }
                    }
                    if bad {
                        break;
                    }
                    let mut x = 1u32;
                    let mut i = len as i32 - 2;
                    while i >= 0 {
                        let b = self.bool();
                        x = x.wrapping_add(x) | b;
                        i -= 1;
                    }
                    lv = x.wrapping_add(14);
                }
                if pos == 0 && lv != 0 {
                    cf.dccat = if sign != 0 { 1 } else { 2 };
                }
                lv &= 0xFFFFF;
                cf.cul = cf.cul.wrapping_add(lv);
                self.s.quant[pos] = if sign != 0 { (lv as i32).wrapping_neg() } else { lv as i32 };
            }
            if bad {
                self.s.err = img_core::ERR_HDR;
                cf.eob = 0;
            }
            cf.cul = cf.cul.min(63);
        }
        // above / left level and dc contexts
        let s = &mut *self.s;
        let n = s.acols as usize;
        let pl = plane as usize;
        for x in cf.x4..cf.x4 + cf.w4 {
            s.alvl[pl * n + x as usize] = cf.cul as u8;
            s.adc[pl * n + x as usize] = cf.dccat as u8;
        }
        for y in cf.y4..cf.y4 + cf.h4 {
            s.llvl[pl][(y & 31) as usize] = cf.cul as u8;
            s.ldc[pl][(y & 31) as usize] = cf.dccat as u8;
        }
        s.eob = cf.eob;
        cf.eob
    }

    /// av1_cf_azctx: ctx of all_zero
    pub fn cf_azctx(&self, cf: &Cf) -> u32 {
        let h = self.h;
        let s = &*self.s;
        let (mut mx4, mut my4) = (h.micols, h.mirows);
        let (mut sx, mut sy) = (0, 0);
        if cf.plane != 0 {
            sx = h.ssx;
            sy = h.ssy;
            mx4 >>= sx;
            my4 >>= sy;
        }
        // bw, bh of get_plane_residual_size( MiSize, plane )
        let mut ps = SUBSAMPLED_SIZE[(s.misize * 4 + sx * 2 + sy) as usize] as u32;
        if ps >= BLOCK_INVALID {
            ps = 0;
        }
        let bw = NUM_4X4_BLOCKS_WIDE[ps as usize] as u32; // bw / 4
        let bh = NUM_4X4_BLOCKS_HIGH[ps as usize] as u32;
        let n = s.acols as usize;
        let pl = cf.plane as usize;
        let alvl = &s.alvl[pl * n..];
        let adc = &s.adc[pl * n..];
        if cf.plane == 0 {
            // luma: top / left = max of the level contexts
            let mut top = 0u32;
            for x in cf.x4..cf.x4 + cf.w4 {
                if x < mx4 {
                    top = top.max(alvl[x as usize] as u32);
                }
            }
            let mut left = 0u32;
            for y in cf.y4..cf.y4 + cf.h4 {
                if y < my4 {
                    left = left.max(s.llvl[0][(y & 31) as usize] as u32);
                }
            }
            if bw == cf.w4 && bh == cf.h4 {
                return 0;
            }
            if top | left == 0 {
                return 1;
            }
            let mx = top.max(left);
            let mn = top.min(left);
            if mn == 0 {
                return if mx <= 3 { 2 } else { 3 };
            }
            if mx <= 3 {
                return 4;
            }
            if mn <= 3 {
                return 5;
            }
            return 6;
        }
        let mut above = 0u8;
        for x in cf.x4..cf.x4 + cf.w4 {
            if x < mx4 {
                above |= alvl[x as usize];
                above |= adc[x as usize];
            }
        }
        let mut left = 0u8;
        for y in cf.y4..cf.y4 + cf.h4 {
            if y < my4 {
                left |= s.llvl[pl][(y & 31) as usize];
                left |= s.ldc[pl][(y & 31) as usize];
            }
        }
        let mut c = (above != 0) as u32 + (left != 0) as u32 + 7;
        if bw * bh > cf.w4 * cf.h4 {
            c += 3;
        }
        c
    }

    /// av1_cf_settxt: TxTypes of the w4 x h4 luma 4x4 blocks of the transform block
    pub fn cf_settxt(&mut self, cf: &Cf, txt: u32) {
        for j in 0..cf.h4 {
            let i = self.mib(cf.y4 + j, cf.x4);
            for m in &mut self.f.mi[i..i + cf.w4 as usize] {
                m[MIB_TXT] = txt as u8;
            }
        }
    }

    /// av1_cf_txset: get_tx_set( txSz ) of an intra block
    pub fn cf_txset(&self, tx: u32) -> u32 {
        if TX_SIZE_SQR_UP[tx as usize] as u32 >= TX_32X32 {
            return TX_SET_DCTONLY;
        }
        if self.h.redtx != 0 {
            return TX_SET_INTRA_2;
        }
        if TX_SIZE_SQR[tx as usize] == TX_16X16 {
            return TX_SET_INTRA_2;
        }
        TX_SET_INTRA_1
    }

    /// cf_txtype: transform_type( x4, y4, txSz ) (intra)
    fn txtype(&mut self, cf: &Cf) {
        let set = self.cf_txset(cf.txsz);
        let mut tt = 0u32; // DCT_DCT
        if set != TX_SET_DCTONLY {
            let h = self.h;
            let q = if h.segen != 0 { qindex(h, self.s.seg, -1) } else { h.baseq };
            if q != 0 {
                let mut dir = self.s.ymode; // intraDir
                if self.s.ufi != 0 {
                    dir = FILTER_INTRA_MODE_TO_INTRA_DIR[self.s.fim as usize] as u32;
                }
                let k = (dir + TX_SIZE_SQR[cf.txsz as usize] as u32 * 13) as usize;
                if set == TX_SET_INTRA_1 {
                    let v = self.sym(CDF_ITX1 + k * 8, 7);
                    tt = TX_TYPE_INTRA_INV_SET1[v as usize] as u32;
                } else {
                    let v = self.sym(CDF_ITX2 + k * 6, 5);
                    tt = TX_TYPE_INTRA_INV_SET2[v as usize] as u32;
                }
            }
        }
        self.cf_settxt(cf, tt);
    }

    /// av1_cf_pttype: compute_tx_type( plane, txSz, x4, y4 ) (intra)
    pub fn cf_pttype(&self, cf: &Cf) -> u32 {
        if self.s.ll != 0 {
            return 0;
        }
        if TX_SIZE_SQR_UP[cf.txsz as usize] as u32 > TX_32X32 {
            return 0;
        }
        let set = self.cf_txset(cf.txsz);
        if cf.plane == 0 {
            return self.f.mi[self.mib(cf.y4, cf.x4)][MIB_TXT] as u32;
        }
        let t = MODE_TO_TXFM[self.s.uvmode as usize] as u32;
        if TX_TYPE_IN_SET_INTRA[(set * 16 + t) as usize] != 0 {
            t
        } else {
            0
        }
    }

    /// av1_cf_basectx: ctx of coeff_base (get_coeff_base_ctx, isEob = 0)
    #[inline]
    pub fn cf_basectx(&self, cf: &Cf, pos: u32) -> u32 {
        let row = pos >> cf.bwl;
        let col = pos - (row << cf.bwl);
        let offs = &SIG_REF_DIFF_OFFSET[(cf.class * 10) as usize..];
        let q = &self.s.quant;
        let mut mag = 0u32;
        for k in 0..5 {
            let rr = (row as i32 + offs[k * 2] as i32) as u32;
            let rc = (col as i32 + offs[k * 2 + 1] as i32) as u32;
            if rr < cf.height && rc < cf.width {
                let v = q[(((rr << cf.bwl) + rc) & 1023) as usize];
                mag += v.unsigned_abs().min(3);
            }
        }
        let mag = ((mag + 1) >> 1).min(4);
        if cf.class == 0 {
            if row | col == 0 {
                return 0;
            }
            let i = cf.txsz * 25 + row.min(4) * 5 + col.min(4);
            return mag + COEFF_BASE_CTX_OFFSET[i as usize] as u32;
        }
        let idx = if cf.class == 2 { row } else { col }; // VERT: row, HORIZ: col
        mag + COEFF_BASE_POS_CTX_OFFSET[idx.min(2) as usize] as u32
    }

    /// av1_cf_brctx: ctx of coeff_br
    #[inline]
    pub fn cf_brctx(&self, cf: &Cf, pos: u32) -> u32 {
        let row = pos >> cf.bwl;
        let col = pos - (row << cf.bwl);
        let offs = &MAG_REF_OFFSET_WITH_TX_CLASS[(cf.class * 6) as usize..];
        let q = &self.s.quant;
        let mut mag = 0u32;
        for k in 0..3 {
            let rr = (row as i32 + offs[k * 2] as i32) as u32;
            let rc = (col as i32 + offs[k * 2 + 1] as i32) as u32;
            if rr < cf.height && rc < cf.width {
                let v = q[(((rr << cf.bwl) + rc) & 1023) as usize] as u32;
                mag = mag.wrapping_add(v.min(15)); // COEFF_BASE_RANGE + NUM_BASE_LEVELS + 1
            }
        }
        let mag = ((mag.wrapping_add(1)) >> 1).min(6);
        if pos == 0 {
            return mag;
        }
        let near = match cf.class {
            0 => row < 2 && col < 2,
            1 => col == 0, // HORIZ
            _ => row == 0, // VERT
        };
        if near { mag + 7 } else { mag + 14 }
    }

    /// av1_cf_dcctx: ctx of dc_sign
    pub fn cf_dcctx(&self, cf: &Cf) -> u32 {
        let h = self.h;
        let s = &*self.s;
        let (mut mx4, mut my4) = (h.micols, h.mirows);
        if cf.plane != 0 {
            mx4 >>= h.ssx;
            my4 >>= h.ssy;
        }
        let n = s.acols as usize;
        let pl = cf.plane as usize;
        let adc = &s.adc[pl * n..];
        let mut dc = 0i32;
        for x in cf.x4..cf.x4 + cf.w4 {
            if x < mx4 {
                match adc[x as usize] {
                    1 => dc -= 1,
                    2 => dc += 1,
                    _ => {}
                }
            }
        }
        for y in cf.y4..cf.y4 + cf.h4 {
            if y < my4 {
                match s.ldc[pl][(y & 31) as usize] {
                    1 => dc -= 1,
                    2 => dc += 1,
                    _ => {}
                }
            }
        }
        if dc == 0 {
            0
        } else if dc < 0 {
            1
        } else {
            2
        }
    }
}
