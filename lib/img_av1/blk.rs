// blk -- AV1 tile / block syntax of intra frames (specification 5.11, CDF selection 8.3.2)
//                                                                    [src/img/av1_blk.s]
//
//   tile(ts, h, f, data, bounds, cancel) -> 0 / ERR_*   decode_tile( ) of one tile: every
//                                                       superblock, prediction and
//                                                       reconstruction included
//
// T is one tile decode: the tile state Ts (TS of the assembly, kept between tiles), the frame
// header, the frame buffers and the tile data.  The MIB records ([u8; 16] per 4x4 luma block,
// MIB_* offsets) take the per-4x4 arrays of the specification (MiSizes, YModes, UVModes, Skips,
// TxSizes, SegmentIds, PaletteSizes, DeltaLFs, TxTypes, LoopfilterTxSizes); the palette colours
// of the neighbours come from the above / left arrays (the last block written in that column /
// row is the neighbour).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use img_core::{ERR_CANCEL, ERR_NOMEM};

use crate::dec::{qindex, zvec, Frame, Hdr, Lru};
use crate::sym::Sd;
use crate::tab::*;

// ---- constants of the specification  [av1_int.inc]
pub(crate) const BLOCK_4X4: u32 = 0;
pub(crate) const BLOCK_8X8: u32 = 3;
pub(crate) const BLOCK_64X64: u32 = 12;
pub(crate) const BLOCK_128X128: u32 = 15;
pub(crate) const BLOCK_INVALID: u32 = 22;
const PARTITION_NONE: u32 = 0;
const PARTITION_HORZ: u32 = 1;
const PARTITION_VERT: u32 = 2;
const PARTITION_SPLIT: u32 = 3;
const PARTITION_HORZ_A: u32 = 4;
const PARTITION_HORZ_B: u32 = 5;
const PARTITION_VERT_A: u32 = 6;
const PARTITION_VERT_B: u32 = 7;
const PARTITION_HORZ_4: u32 = 8;
pub(crate) const DC_PRED: u32 = 0;
pub(crate) const V_PRED: u32 = 1;
pub(crate) const D67_PRED: u32 = 8;
pub(crate) const UV_CFL_PRED: u32 = 13;
pub(crate) const TX_4X4: u32 = 0;
pub(crate) const TX_32X32: u32 = 3;
pub(crate) const TX_16X32: u32 = 9;
pub(crate) const TX_32X16: u32 = 10;
pub(crate) const TX_16X64: u32 = 17;
pub(crate) const TX_64X16: u32 = 18;
const TX_MODE_SELECT: u32 = 2;
const RESTORE_WIENER: u32 = 1;
const RESTORE_SGRPROJ: u32 = 2;
const SEG_LVL_SKIP: usize = 6;

// ---- MIB: mode info per 4x4 luma block
pub(crate) const MIB_SIZE: usize = 0; // MiSize
pub(crate) const MIB_YMODE: usize = 1;
pub(crate) const MIB_UVMODE: usize = 2;
pub(crate) const MIB_SKIP: usize = 3;
pub(crate) const MIB_TX: usize = 4; // TxSize (= InterTxSizes for intra frames)
pub(crate) const MIB_SEG: usize = 5; // segment_id
pub(crate) const MIB_PALY: usize = 6; // PaletteSizeY / UV
pub(crate) const MIB_DLF: usize = 8; // i8[4] DeltaLF
pub(crate) const MIB_TXT: usize = 12; // TxTypes of this 4x4 (luma)
pub(crate) const MIB_LTX: usize = 13; // u8[3] LoopfilterTxSizes[plane] at (row >> subY, col >> subX)

// CDF offsets of the palette colour index CDFs for n = 2 .. 8
static PAL_CDF_Y: [usize; 7] = [CDF_PALY2, CDF_PALY3, CDF_PALY4, CDF_PALY5, CDF_PALY6, CDF_PALY7, CDF_PALY8];
static PAL_CDF_UV: [usize; 7] =
    [CDF_PALUV2, CDF_PALUV3, CDF_PALUV4, CDF_PALUV5, CDF_PALUV6, CDF_PALUV7, CDF_PALUV8];

/// the tile state (TS)
pub(crate) struct Ts {
    pub sd: Sd,
    pub dcu: bool, // disable_cdf_update
    // tile
    pub mrs: u32, // MiRowStart / End, MiColStart / End
    pub mre: u32,
    pub mcs: u32,
    pub mce: u32,
    pub curq: i32, // CurrentQIndex
    pub rdeltas: u32,
    pub dlf: [i32; 4],
    pub refsgr: [[i32; 2]; 3],
    pub refwi: [[[i32; 3]; 2]; 3],
    // current block
    pub mirow: u32,
    pub micol: u32,
    pub misize: u32,
    pub bw4: u32,
    pub bh4: u32,
    pub hasc: u32,
    pub au: u32, // AvailU / L / UChroma / LChroma
    pub al: u32,
    pub auc: u32,
    pub alc: u32,
    pub skip: u32,
    pub seg: u32,
    pub ll: u32, // Lossless
    pub ymode: u32,
    pub uvmode: u32,
    pub ady: i32, // AngleDeltaY / UV
    pub aduv: i32,
    pub cflu: i32,
    pub cflv: i32,
    pub ufi: u32, // use_filter_intra
    pub fim: u32,
    pub psy: u32, // PaletteSizeY / UV
    pub psuv: u32,
    pub txsz: u32,
    pub maxlw: u32, // MaxLumaW / H
    pub maxlh: u32,
    pub ptxt: u32, // PlaneTxType
    pub eob: u32,
    pub qidx: u32, // qindex of the block
    pub paly: [u16; 8],
    pub palu: [u16; 8],
    pub palv: [u16; 8],
    pub err: i32,
    pub pcache: [u16; 16],
    pub pcn: u32,
    // above arrays (MiCols + 64 entries): AboveLevelContext[3], AboveDcContext[3], AbovePal (Y | U)
    pub acols: u32,
    pub alvl: Vec<u8>,
    pub adc: Vec<u8>,
    pub apal: Vec<[u16; 16]>,
    pub llvl: [[u8; 32]; 3], // LeftLevelContext (index y4 & 31)
    pub ldc: [[u8; 32]; 3],
    pub lpal: [[u16; 16]; 32], // left palette colours (index MiRow & 31)
    pub bdec: [[u8; 34 * 34]; 3], // BlockDecoded ([y + 1][x + 1])
    pub cdf: [u16; CDF_LEN],
    pub cmapy: [u8; 64 * 64], // ColorMapY (stride 64)
    pub cmapuv: [u8; 64 * 64],
    pub quant: [i32; 1024],
    pub dq: [i32; 64 * 64], // Dequant / Residual (stride 64)
    pub tt: [i32; 64],      // 1D transform array T
    pub edge: [i32; 640],   // AboveRow / LeftCol (index + 32)
    pub cfll: [i32; 32 * 32],
    // prediction parameters
    pub ppl: u32,
    pub px: u32, // startX / startY (samples of the plane)
    pub py: u32,
    pub phl: u32, // haveLeft, haveAbove, haveAboveRight, haveBelowLeft
    pub pha: u32,
    pub phar: u32,
    pub phbl: u32,
    pub pmode: u32,
    pub plw: u32,
    pub plh: u32,
    pub ptx: u32,
    pub pbx: u32, // x / y in 4x4 units inside the block
    pub pby: u32,
    pub prow: u32, // row / col (luma 4x4) of the transform block
    pub pcol: u32,
    pub psbr: u32, // subBlockMiRow >> subY / subBlockMiCol >> subX
    pub psbc: u32,
}

impl Ts {
    pub fn new() -> Box<Ts> {
        Box::new(Ts {
            sd: Sd::default(),
            dcu: false,
            mrs: 0,
            mre: 0,
            mcs: 0,
            mce: 0,
            curq: 0,
            rdeltas: 0,
            dlf: [0; 4],
            refsgr: [[0; 2]; 3],
            refwi: [[[0; 3]; 2]; 3],
            mirow: 0,
            micol: 0,
            misize: 0,
            bw4: 0,
            bh4: 0,
            hasc: 0,
            au: 0,
            al: 0,
            auc: 0,
            alc: 0,
            skip: 0,
            seg: 0,
            ll: 0,
            ymode: 0,
            uvmode: 0,
            ady: 0,
            aduv: 0,
            cflu: 0,
            cflv: 0,
            ufi: 0,
            fim: 0,
            psy: 0,
            psuv: 0,
            txsz: 0,
            maxlw: 0,
            maxlh: 0,
            ptxt: 0,
            eob: 0,
            qidx: 0,
            paly: [0; 8],
            palu: [0; 8],
            palv: [0; 8],
            err: 0,
            pcache: [0; 16],
            pcn: 0,
            acols: 0,
            alvl: Vec::new(),
            adc: Vec::new(),
            apal: Vec::new(),
            llvl: [[0; 32]; 3],
            ldc: [[0; 32]; 3],
            lpal: [[0; 16]; 32],
            bdec: [[0; 34 * 34]; 3],
            cdf: [0; CDF_LEN],
            cmapy: [0; 64 * 64],
            cmapuv: [0; 64 * 64],
            quant: [0; 1024],
            dq: [0; 64 * 64],
            tt: [0; 64],
            edge: [0; 640],
            cfll: [0; 32 * 32],
            ppl: 0,
            px: 0,
            py: 0,
            phl: 0,
            pha: 0,
            phar: 0,
            phbl: 0,
            pmode: 0,
            plw: 0,
            plh: 0,
            ptx: 0,
            pbx: 0,
            pby: 0,
            prow: 0,
            pcol: 0,
            psbr: 0,
            psbc: 0,
        })
    }

    /// (framefree) the above arrays
    pub fn above_free(&mut self) {
        self.alvl = Vec::new();
        self.adc = Vec::new();
        self.apal = Vec::new();
        self.acols = 0;
    }

    /// (av_alloc) the above arrays, n entries each
    pub fn above_alloc(&mut self, n: u32) -> Result<(), i32> {
        self.alvl = zvec::<u8>(3 * n as usize)?;
        self.adc = zvec::<u8>(3 * n as usize)?;
        let mut p: Vec<[u16; 16]> = Vec::new();
        p.try_reserve_exact(n as usize).map_err(|_| ERR_NOMEM)?;
        p.resize(n as usize, [0; 16]);
        self.apal = p;
        self.acols = n;
        Ok(())
    }
}

/// one tile decode
pub(crate) struct T<'a> {
    pub s: &'a mut Ts,
    pub h: &'a Hdr,
    pub f: &'a mut Frame,
    pub d: &'a [u8], // tile data
}

/// av1_tile: decode_tile( ) of the tile bounds = [MiRowStart, MiRowEnd, MiColStart, MiColEnd]
pub(crate) fn tile(s: &mut Ts, h: &Hdr, f: &mut Frame, data: &[u8], bounds: [u32; 4], cancel: Option<&AtomicU32>) -> i32 {
    s.mrs = bounds[0];
    s.mre = bounds[1];
    s.mcs = bounds[2];
    s.mce = bounds[3];
    let mut t = T { s, h, f, d: data };
    t.tilesetup();
    t.s.sd.init(data);
    let (sb4, sbsize) = if h.sb128 != 0 { (32, BLOCK_128X128) } else { (16, BLOCK_64X64) };
    let mut r = t.s.mrs;
    'rows: while r < t.s.mre {
        // clear_left_context (and the left palettes)
        t.s.llvl = [[0; 32]; 3];
        t.s.ldc = [[0; 32]; 3];
        t.s.lpal = [[0; 16]; 32];
        let mut c = t.s.mcs;
        while c < t.s.mce {
            if cancel.is_some_and(|x| x.load(Ordering::Relaxed) != 0) {
                t.s.err = ERR_CANCEL;
                break 'rows;
            }
            t.s.rdeltas = h.dqpres;
            // clear_cdef
            let cds = t.f.cds as usize;
            let i = (r >> 4) as usize * cds + (c >> 4) as usize;
            t.f.cdidx[i] = -1;
            if h.sb128 != 0 {
                t.f.cdidx[i + 1] = -1;
                t.f.cdidx[i + cds] = -1;
                t.f.cdidx[i + cds + 1] = -1;
            }
            t.clearbd(r, c, sb4);
            t.readlr(r, c, sbsize);
            t.dpart(r, c, sbsize);
            if t.s.err != 0 {
                break 'rows;
            }
            c += sb4;
        }
        r += sb4;
    }
    t.s.err
}

impl T<'_> {
    // ---- symbols
    /// read_symbol of the CDF at element offset off of the context, n symbols
    #[inline(always)]
    pub fn sym(&mut self, off: usize, n: usize) -> u32 {
        let s = &mut *self.s;
        s.sd.sym(self.d, &mut s.cdf[off..], n, s.dcu)
    }
    #[inline]
    pub fn bool(&mut self) -> u32 {
        self.s.sd.bool(self.d)
    }
    #[inline]
    pub fn lit(&mut self, n: u32) -> u32 {
        self.s.sd.lit(self.d, n)
    }
    #[inline]
    pub fn ns(&mut self, n: u32) -> u32 {
        self.s.sd.ns(self.d, n)
    }
    /// index of MIB[ row ][ col ]
    #[inline]
    pub fn mib(&self, row: u32, col: u32) -> usize {
        row.wrapping_mul(self.f.mis).wrapping_add(col) as usize
    }

    /// av1_tilesetup: default CDFs, clear_above_context, DeltaLF, RefSgrXqd, RefLrWiener,
    /// CurrentQIndex
    pub fn tilesetup(&mut self) { // r9-av1enc: pub (the encoder's tile jobs)
        let s = &mut *self.s;
        let h = self.h;
        s.dcu = h.dcu != 0;
        s.err = 0;
        // CDFs: defaults (init_non_coeff_cdfs / init_coeff_cdfs)
        s.cdf[..CDF_COEF].copy_from_slice(&CDF_NC);
        let q = h.baseq;
        let cls = if q <= 20 {
            0
        } else if q <= 60 {
            1
        } else if q <= 120 {
            2
        } else {
            3
        };
        s.cdf[CDF_COEF..CDF_COEF + CDF_COEFSIZE].copy_from_slice(&CDF_COEF_DEF[cls]);
        // clear_above_context, DeltaLF, RefSgrXqd, RefLrWiener
        s.alvl.fill(0);
        s.adc.fill(0);
        s.apal.fill([0; 16]);
        s.dlf = [0; 4];
        for p in 0..3 {
            s.refsgr[p] = [-32, 31];
            s.refwi[p] = [[3, -7, 15], [3, -7, 15]];
        }
        s.curq = h.baseq as i32;
    }

    /// av1_clearbd: clear_block_decoded_flags( r, c, sbSize4 )
    pub fn clearbd(&mut self, r: u32, c: u32, sb4: u32) {
        let h = self.h;
        let s = &mut *self.s;
        for p in 0..h.np as usize {
            let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
            let sbw4 = (s.mce.wrapping_sub(c) >> sx) as i32;
            let r10 = (sb4 >> sx) as usize;
            let sbh4 = (s.mre.wrapping_sub(r) >> sy) as i32;
            let r11 = (sb4 >> sy) as usize;
            let bd = &mut s.bdec[p];
            // row y = -1: entries x = -1 .. sbSize4 >> subX are 1 while x < sbWidth4
            for i in 0..=r10 + 1 {
                bd[i] = ((i as i32) <= sbw4) as u8;
            }
            // rows y = 0 .. sbSize4 >> subY: x = -1 is 1 while y < sbHeight4, the rest 0
            for y in 0..=r11 {
                let row = (y + 1) * 34;
                bd[row] = ((y as i32) < sbh4) as u8;
                bd[row + 1..row + r10 + 2].fill(0);
            }
            bd[(r11 + 1) * 34] = 0;
        }
    }

    // ------------------------------------------------------------------ loop restoration syntax

    /// tl_readlr: read_lr( r, c, bSize )
    fn readlr(&mut self, r: u32, c: u32, bsize: u32) {
        let h = self.h;
        for p in 0..h.np as usize {
            if h.frt[p] == 0 {
                continue;
            }
            let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
            let us = h.lrsize[p];
            let hb = NUM_4X4_BLOCKS_HIGH[bsize as usize] as u32;
            let r8 = 4u32 >> sy;
            let urs = (r.wrapping_mul(r8).wrapping_add(us).wrapping_sub(1)) / us;
            let ure = ((r.wrapping_add(hb)).wrapping_mul(r8).wrapping_add(us).wrapping_sub(1) / us).min(self.f.lrurows[p]);
            let (mut num, mut den) = (4u32 >> sx, us);
            if h.usesr != 0 {
                num = num.wrapping_mul(h.srden);
                den <<= 3;
            }
            let ucs = (c.wrapping_mul(num).wrapping_add(den).wrapping_sub(1)) / den;
            let wb = NUM_4X4_BLOCKS_WIDE[bsize as usize] as u32;
            let uce = ((c.wrapping_add(wb)).wrapping_mul(num).wrapping_add(den).wrapping_sub(1) / den).min(self.f.lrucols[p]);
            for ur in urs..ure {
                for uc in ucs..uce {
                    self.lrunit(p, ur, uc);
                }
            }
        }
    }

    /// tl_lrunit: read_lr_unit
    fn lrunit(&mut self, p: usize, ur: u32, uc: u32) {
        let ui = ur.wrapping_mul(self.f.lrucols[p]).wrapping_add(uc) as usize;
        let frt = self.h.frt[p] as u32;
        let t = if frt == RESTORE_WIENER {
            self.sym(CDF_USEWIENER, 2) // 0 NONE, 1 WIENER
        } else if frt == RESTORE_SGRPROJ {
            self.sym(CDF_USESGR, 2) * 2 // 0 NONE, 2 SGRPROJ
        } else {
            self.sym(CDF_RESTYPE, 3)
        };
        let mut u: Lru = self.f.lru[p][ui];
        u.typ = t as u8;
        if t == RESTORE_WIENER {
            for pass in 0..2 {
                let mut first = 0;
                if p > 0 {
                    first = 1;
                    u.wiener[pass][0] = 0;
                }
                for j in first..3 {
                    let lo = WIENER_TAPS_MIN[j] as i32;
                    let hi = WIENER_TAPS_MAX[j] as i32 + 1;
                    let k = WIENER_TAPS_K[j] as u32;
                    let r = self.s.refwi[p][pass][j];
                    let v = self.subexp(lo, hi, k, r);
                    u.wiener[pass][j] = v as i8;
                    self.s.refwi[p][pass][j] = v;
                }
            }
        } else if t == RESTORE_SGRPROJ {
            let set = self.lit(4) & 15;
            u.set = set as u8;
            for i in 0..2 {
                let radius = SGR_PARAMS[set as usize * 4 + i * 2];
                let lo = SGRPROJ_XQD_MIN[i] as i32;
                let hi = SGRPROJ_XQD_MAX[i] as i32;
                let v = if radius != 0 {
                    let r = self.s.refsgr[p][i];
                    self.subexp(lo, hi + 1, 4, r) // SGRPROJ_PRJ_SUBEXP_K
                } else if i == 1 {
                    (128 - self.s.refsgr[p][0]).max(lo).min(hi)
                } else {
                    0
                };
                u.xqd[i] = v as i16;
                self.s.refsgr[p][i] = v;
            }
        }
        self.f.lru[p][ui] = u;
    }

    /// tl_subexp: decode_signed_subexp_with_ref_bool( low, high, k, r )
    fn subexp(&mut self, low: i32, high: i32, k: u32, r: i32) -> i32 {
        let high = high as i16 as i32;
        let mx = high.wrapping_sub(low);
        let rl = r.wrapping_sub(low);
        // decode_subexp_bool( mx, k )
        let mut i = 0u32;
        let mut mk = 0i32;
        let v = loop {
            let b2 = if i != 0 { k + i - 1 } else { k };
            let a = 1i32.wrapping_shl(b2);
            if mx <= a.wrapping_mul(3).wrapping_add(mk) {
                break (self.ns(mx.wrapping_sub(mk) as u32) as i32).wrapping_add(mk);
            }
            if self.lit(1) != 0 {
                i += 1;
                mk = mk.wrapping_add(a);
            } else {
                break (self.lit(b2) as i32).wrapping_add(mk);
            }
        };
        // inverse recentering around r - low
        let res = if rl.wrapping_add(rl) <= mx {
            recenter(v, rl)
        } else {
            let x = recenter(v, mx.wrapping_sub(1).wrapping_sub(rl));
            mx.wrapping_sub(1).wrapping_sub(x)
        };
        res.wrapping_add(low)
    }

    // ------------------------------------------------------------------ partition

    /// dpart: decode_partition( r, c, bSize )
    fn dpart(&mut self, r: u32, c: u32, bsize: u32) {
        let h = self.h;
        if r >= h.mirows || c >= h.micols {
            return;
        }
        let half = (NUM_4X4_BLOCKS_WIDE[bsize as usize] >> 1) as u32;
        let quarter = half >> 1;
        let has_rows = r + half < h.mirows;
        let has_cols = c + half < h.micols;
        let part = if bsize < BLOCK_8X8 {
            PARTITION_NONE
        } else if !has_rows && !has_cols {
            PARTITION_SPLIT
        } else {
            // partition cdf: ctx from the above / left block sizes
            let bsl = MI_WIDTH_LOG2[bsize as usize];
            let mut ctx = 0usize;
            if r > self.s.mrs {
                let a = self.f.mi[self.mib(r - 1, c)][MIB_SIZE] as usize;
                if MI_WIDTH_LOG2[a] < bsl {
                    ctx = 1;
                }
            }
            if c > self.s.mcs {
                let l = self.f.mi[self.mib(r, c - 1)][MIB_SIZE] as usize;
                if MI_HEIGHT_LOG2[l] < bsl {
                    ctx += 2;
                }
            }
            let (off, n) = match bsl {
                1 => (CDF_PART8 + ctx * 5, 4),
                2 => (CDF_PART16 + ctx * 11, 10),
                3 => (CDF_PART32 + ctx * 11, 10),
                4 => (CDF_PART64 + ctx * 11, 10),
                _ => (CDF_PART128 + ctx * 9, 8),
            };
            if has_rows && has_cols {
                self.sym(off, n)
            } else {
                let c_ = &self.s.cdf[off..];
                let g = |k: usize| c_[k] as i32;
                let mut psum;
                if !has_rows {
                    // split_or_horz: psum of VERT, SPLIT, HORZ_A, VERT_A, VERT_B (, VERT_4)
                    psum = g(2) - g(1) + g(3) - g(2) + g(4) - g(3) + g(6) - g(5) + g(7) - g(6);
                    if bsize != BLOCK_128X128 {
                        psum += g(9) - g(8);
                    }
                } else {
                    // split_or_vert: psum of HORZ, SPLIT, HORZ_A, HORZ_B, VERT_A (, HORZ_4)
                    psum = g(1) - g(0) + g(3) - g(2) + g(4) - g(3) + g(5) - g(4) + g(6) - g(5);
                    if bsize != BLOCK_128X128 {
                        psum += g(8) - g(7);
                    }
                }
                let mut tmp = [(32768 - psum) as u16, 32768, 0];
                let s = &mut *self.s;
                let b = s.sd.sym(self.d, &mut tmp, 2, s.dcu);
                if b != 0 {
                    PARTITION_SPLIT
                } else if !has_rows {
                    PARTITION_HORZ // hasCols only
                } else {
                    PARTITION_VERT
                }
            }
        };
        let sub = PARTITION_SUBSIZE[(part * 22 + bsize) as usize] as u32;
        let split = PARTITION_SUBSIZE[(PARTITION_SPLIT * 22 + bsize) as usize] as u32;
        match part {
            PARTITION_NONE => self.dblock(r, c, sub),
            PARTITION_HORZ => {
                self.dblock(r, c, sub);
                if has_rows {
                    self.dblock(r + half, c, sub);
                }
            }
            PARTITION_VERT => {
                self.dblock(r, c, sub);
                if has_cols {
                    self.dblock(r, c + half, sub);
                }
            }
            PARTITION_SPLIT => {
                self.dpart(r, c, sub);
                self.dpart(r, c + half, sub);
                self.dpart(r + half, c, sub);
                self.dpart(r + half, c + half, sub);
            }
            PARTITION_HORZ_A => {
                self.dblock(r, c, split);
                self.dblock(r, c + half, split);
                self.dblock(r + half, c, sub);
            }
            PARTITION_HORZ_B => {
                self.dblock(r, c, sub);
                self.dblock(r + half, c, split);
                self.dblock(r + half, c + half, split);
            }
            PARTITION_VERT_A => {
                self.dblock(r, c, split);
                self.dblock(r + half, c, split);
                self.dblock(r, c + half, sub);
            }
            PARTITION_VERT_B => {
                self.dblock(r, c, sub);
                self.dblock(r, c + half, split);
                self.dblock(r + half, c + half, split);
            }
            PARTITION_HORZ_4 => {
                for i in 0..4 {
                    if i == 3 && r + quarter * 3 >= h.mirows {
                        break;
                    }
                    self.dblock(r + quarter * i, c, sub);
                }
            }
            _ => {
                // VERT_4
                for i in 0..4 {
                    if i == 3 && c + quarter * 3 >= h.micols {
                        break;
                    }
                    self.dblock(r, c + quarter * i, sub);
                }
            }
        }
    }

    // ------------------------------------------------------------------ block

    /// dblock: decode_block( r, c, subSize )
    fn dblock(&mut self, r: u32, c: u32, sub: u32) {
        let h = self.h;
        let bw4 = NUM_4X4_BLOCKS_WIDE[sub as usize] as u32;
        let bh4 = NUM_4X4_BLOCKS_HIGH[sub as usize] as u32;
        {
            let s = &mut *self.s;
            s.mirow = r;
            s.micol = c;
            s.misize = sub;
            s.bw4 = bw4;
            s.bh4 = bh4;
            // HasChroma
            let hasc = if bh4 == 1 && h.ssy != 0 && r & 1 == 0 {
                0
            } else if bw4 == 1 && h.ssx != 0 && c & 1 == 0 {
                0
            } else {
                (h.np > 1) as u32
            };
            s.hasc = hasc;
            s.au = (r > s.mrs) as u32;
            s.auc = s.au;
            s.al = (c > s.mcs) as u32;
            s.alc = s.al;
            if hasc == 0 {
                s.auc = 0;
                s.alc = 0;
            } else {
                if h.ssy != 0 && bh4 == 1 {
                    s.auc = (r >= s.mrs + 2) as u32;
                }
                if h.ssx != 0 && bw4 == 1 {
                    s.alc = (c >= s.mcs + 2) as u32;
                }
            }
        }
        self.modeinfo();
        self.paltokens();
        self.txsize();
        if self.s.skip != 0 {
            self.blkreset();
        }
        // YModes / UVModes of the block
        let (ym, uvm, hasc) = (self.s.ymode as u8, self.s.uvmode as u8, self.s.hasc != 0);
        for y in 0..bh4 {
            let i = self.mib(r + y, c);
            for m in &mut self.f.mi[i..i + bw4 as usize] {
                m[MIB_YMODE] = ym;
                if hasc {
                    m[MIB_UVMODE] = uvm;
                }
            }
        }
        // qindex of the block for dequantisation
        self.s.qidx = qindex(h, self.s.seg, self.s.curq);
        self.residual();
        // mode info of the block
        let s = &*self.s;
        let rec = [s.skip as u8, s.txsz as u8, sub as u8, s.seg as u8, s.psy as u8, s.psuv as u8];
        let dlf = [s.dlf[0] as u8, s.dlf[1] as u8, s.dlf[2] as u8, s.dlf[3] as u8];
        for y in 0..bh4 {
            let i = self.mib(r + y, c);
            for m in &mut self.f.mi[i..i + bw4 as usize] {
                m[MIB_SKIP] = rec[0];
                m[MIB_TX] = rec[1];
                m[MIB_SIZE] = rec[2];
                m[MIB_SEG] = rec[3];
                m[MIB_PALY] = rec[4];
                m[MIB_PALY + 1] = rec[5];
                m[MIB_DLF..MIB_DLF + 4].copy_from_slice(&dlf);
            }
        }
        // palette colours for the neighbours: above (per column), left (per row & 31)
        let s = &mut *self.s;
        if s.psy | s.psuv != 0 {
            let mut pal = [0u16; 16];
            pal[..8].copy_from_slice(&s.paly);
            pal[8..].copy_from_slice(&s.palu);
            for x in 0..bw4 {
                s.apal[(c + x) as usize] = pal;
            }
            for y in 0..bh4 {
                s.lpal[((r + y) & 31) as usize] = pal;
            }
        }
    }

    /// av1_blkreset: reset_block_context( bw4, bh4 )
    pub fn blkreset(&mut self) { // r9-av1enc: pub
        let h = self.h;
        let s = &mut *self.s;
        let n = s.acols as usize;
        for p in 0..(s.hasc * 2 + 1) as usize {
            let sx = if p > 0 { h.ssx } else { 0 };
            let x0 = (s.micol >> sx) as usize;
            let x1 = ((s.micol + s.bw4) >> sx) as usize;
            for x in x0..x1 {
                s.alvl[p * n + x] = 0;
                s.adc[p * n + x] = 0;
            }
            let sy = if p > 0 { h.ssy } else { 0 };
            let y0 = s.mirow >> sy;
            let y1 = (s.mirow + s.bh4) >> sy;
            for y in y0..y1 {
                s.llvl[p][(y & 31) as usize] = 0;
                s.ldc[p][(y & 31) as usize] = 0;
            }
        }
    }

    // ------------------------------------------------------------------ mode info

    /// db_modeinfo: intra_frame_mode_info( )
    fn modeinfo(&mut self) {
        let h = self.h;
        let (r, c) = (self.s.mirow, self.s.micol);
        self.s.skip = 0;
        if h.segpreskip != 0 {
            self.segid();
        }
        self.skip();
        if h.segpreskip == 0 {
            self.segid();
        }
        self.readcdef();
        self.readdq();
        self.readdlf();
        self.s.rdeltas = 0;
        // intra_frame_y_mode
        let a = if self.s.au != 0 { self.f.mi[self.mib(r - 1, c)][MIB_YMODE] as usize } else { 0 };
        let am = INTRA_MODE_CONTEXT[a] as usize;
        let l = if self.s.al != 0 { self.f.mi[self.mib(r, c - 1)][MIB_YMODE] as usize } else { 0 };
        let lm = INTRA_MODE_CONTEXT[l] as usize;
        let ym = self.sym(CDF_IFYMODE + (am * 5 + lm) * 14, 13);
        self.s.ymode = ym;
        // intra_angle_info_y
        self.s.ady = 0;
        if self.s.misize >= BLOCK_8X8 && ym.wrapping_sub(V_PRED) <= D67_PRED - V_PRED {
            self.s.ady = self.sym(CDF_ANGLE + (ym - V_PRED) as usize * 8, 7) as i32 - 3;
        }
        self.s.uvmode = DC_PRED;
        self.s.aduv = 0;
        self.s.cflu = 0;
        self.s.cflv = 0;
        if self.s.hasc != 0 {
            // CFL allowed?
            let cfl = if self.s.ll != 0 {
                let i = self.s.misize * 4 + h.ssx * 2 + h.ssy;
                SUBSAMPLED_SIZE[i as usize] as u32 == BLOCK_4X4
            } else {
                self.s.bw4 <= 8 && self.s.bh4 <= 8
            };
            let uvm = if cfl {
                self.sym(CDF_UVMODE_C + self.s.ymode as usize * 15, 14)
            } else {
                self.sym(CDF_UVMODE_NC + self.s.ymode as usize * 14, 13)
            };
            self.s.uvmode = uvm;
            if uvm == UV_CFL_PRED {
                self.readcfl();
            } else if self.s.misize >= BLOCK_8X8 && uvm.wrapping_sub(V_PRED) <= D67_PRED - V_PRED {
                self.s.aduv = self.sym(CDF_ANGLE + (uvm - V_PRED) as usize * 8, 7) as i32 - 3;
            }
        }
        self.s.psy = 0;
        self.s.psuv = 0;
        if self.s.misize >= BLOCK_8X8 && self.s.bw4 <= 16 && self.s.bh4 <= 16 && h.sct != 0 {
            self.palmode();
        }
        // filter_intra_mode_info
        self.s.ufi = 0;
        if h.efi != 0 && self.s.ymode == DC_PRED && self.s.psy == 0 && self.s.bw4 <= 8 && self.s.bh4 <= 8 {
            let u = self.sym(CDF_FILTERINTRA + self.s.misize as usize * 3, 2);
            self.s.ufi = u;
            if u != 0 {
                self.s.fim = self.sym(CDF_FIMODE, 5);
            }
        }
    }

    /// db_segid: intra_segment_id( ) (read_segment_id, Lossless)
    fn segid(&mut self) {
        let h = self.h;
        let mut seg = 0u32;
        if h.segen != 0 {
            let (r, c) = (self.s.mirow, self.s.micol);
            let (mut pul, mut pu, mut pl) = (-1i32, -1i32, -1i32);
            if self.s.au != 0 {
                let i = self.mib(r - 1, c);
                pu = self.f.mi[i][MIB_SEG] as i32;
                if self.s.al != 0 {
                    pul = self.f.mi[i - 1][MIB_SEG] as i32;
                }
            }
            if self.s.al != 0 {
                pl = self.f.mi[self.mib(r, c - 1)][MIB_SEG] as i32;
            }
            let pred = if pu == -1 {
                if pl == -1 { 0 } else { pl }
            } else if pl == -1 || pul == pu {
                pu
            } else {
                pl
            };
            let mut v = pred;
            if self.s.skip == 0 {
                let ctx = if pul < 0 {
                    0
                } else if pul == pu && pul == pl {
                    2
                } else if pul == pu || pul == pl || pu == pl {
                    1
                } else {
                    0
                };
                let diff = self.sym(CDF_SEGID + ctx * 9, 8) as i32;
                // neg_deinterleave( diff, ref = pred, max = LastActiveSegId + 1 )
                let rf = pred;
                let mx = h.lastseg as i32 + 1;
                let alt = |diff: i32| {
                    if diff & 1 != 0 {
                        rf + ((diff as u32 + 1) >> 1) as i32
                    } else {
                        rf - ((diff as u32) >> 1) as i32
                    }
                };
                v = if rf == 0 {
                    diff
                } else if rf >= mx - 1 {
                    mx - diff - 1
                } else if 2 * rf < mx {
                    if diff <= 2 * rf { alt(diff) } else { diff }
                } else if diff <= (mx - rf - 1) * 2 {
                    alt(diff)
                } else {
                    mx - (diff + 1)
                };
            }
            seg = v as u32;
        }
        let seg = seg & 7;
        self.s.seg = seg;
        self.s.ll = h.llarr[seg as usize] as u32;
    }

    /// db_skip: read_skip( )
    fn skip(&mut self) {
        let h = self.h;
        if h.segpreskip != 0 && h.segen != 0 && h.fen[self.s.seg as usize][SEG_LVL_SKIP] != 0 {
            self.s.skip = 1;
            return;
        }
        let (r, c) = (self.s.mirow, self.s.micol);
        let mut ctx = 0usize;
        if self.s.au != 0 {
            ctx = self.f.mi[self.mib(r - 1, c)][MIB_SKIP] as usize;
        }
        if self.s.al != 0 {
            ctx += self.f.mi[self.mib(r, c - 1)][MIB_SKIP] as usize;
        }
        self.s.skip = self.sym(CDF_SKIP + ctx * 3, 2);
    }

    /// db_cdef: read_cdef( )
    fn readcdef(&mut self) {
        let h = self.h;
        if self.s.skip != 0 || h.codedll != 0 || h.ecdef == 0 {
            return;
        }
        let r64 = (self.s.mirow >> 4) as usize;
        let c64 = (self.s.micol >> 4) as usize;
        let cds = self.f.cds as usize;
        let i = r64 * cds + c64;
        if self.f.cdidx[i] != -1 {
            return;
        }
        let v = self.lit(h.cdefbits) as i8;
        self.f.cdidx[i] = v;
        // spread over the 64x64 blocks of a larger block
        let ny = ((self.s.bh4 + 15) >> 4) as usize;
        let nx = ((self.s.bw4 + 15) >> 4) as usize;
        for y in 0..ny {
            for x in 0..nx {
                self.f.cdidx[(r64 + y) * cds + c64 + x] = v;
            }
        }
    }

    /// db_dq: read_delta_qindex( )
    fn readdq(&mut self) {
        let h = self.h;
        let sbsize = if h.sb128 != 0 { BLOCK_128X128 } else { BLOCK_64X64 };
        if sbsize == self.s.misize && self.s.skip != 0 {
            return;
        }
        if self.s.rdeltas == 0 {
            return;
        }
        let mut a = self.sym(CDF_DELTAQ, 4) as i32;
        if a == 3 {
            // DELTA_Q_SMALL
            let rb = self.lit(3) + 1;
            let v = self.lit(rb) as i32;
            a = 1i32.wrapping_shl(rb).wrapping_add(v).wrapping_add(1);
        }
        if a != 0 {
            if self.lit(1) != 0 {
                a = a.wrapping_neg();
            }
            let q = a.wrapping_shl(h.dqres).wrapping_add(self.s.curq);
            self.s.curq = q.clamp(1, 255);
        }
    }

    /// db_dlf: read_delta_lf( )
    fn readdlf(&mut self) {
        let h = self.h;
        let sbsize = if h.sb128 != 0 { BLOCK_128X128 } else { BLOCK_64X64 };
        if sbsize == self.s.misize && self.s.skip != 0 {
            return;
        }
        if self.s.rdeltas == 0 || h.dlfpres == 0 {
            return;
        }
        let cnt = if h.dlfmulti == 0 {
            1
        } else if h.np == 1 {
            2
        } else {
            4
        };
        for i in 0..cnt {
            let off = if h.dlfmulti != 0 { CDF_DELTALFM + i * 5 } else { CDF_DELTALF };
            let mut a = self.sym(off, 4) as i32;
            if a == 3 {
                // DELTA_LF_SMALL
                let rb = self.lit(3) + 1;
                let v = self.lit(rb) as i32;
                a = 1i32.wrapping_shl(rb).wrapping_add(v).wrapping_add(1);
            }
            if a != 0 {
                if self.lit(1) != 0 {
                    a = a.wrapping_neg();
                }
                let v = a.wrapping_shl(h.dlfres).wrapping_add(self.s.dlf[i]);
                self.s.dlf[i] = v.clamp(-63, 63);
            }
        }
    }

    /// db_cfl: read_cfl_alphas( )
    fn readcfl(&mut self) {
        let s = self.sym(CDF_CFLSIGN, 8) + 1;
        let su = s / 3;
        let sv = s % 3;
        let mut a = 0i32;
        if su != 0 {
            a = self.sym(CDF_CFLALPHA + ((su - 1) * 3 + sv) as usize * 17, 16) as i32 + 1;
            if su == 1 {
                a = -a; // CFL_SIGN_NEG
            }
        }
        self.s.cflu = a;
        let mut a = 0i32;
        if sv != 0 {
            a = self.sym(CDF_CFLALPHA + ((sv - 1) * 3 + su) as usize * 17, 16) as i32 + 1;
            if sv == 1 {
                a = -a;
            }
        }
        self.s.cflv = a;
    }

    // ------------------------------------------------------------------ palette

    /// db_palmode: palette_mode_info( )
    fn palmode(&mut self) {
        let (r, c) = (self.s.mirow, self.s.micol);
        let ms = self.s.misize as usize;
        let bctx = (MI_WIDTH_LOG2[ms] + MI_HEIGHT_LOG2[ms]) as usize - 2;
        if self.s.ymode == DC_PRED {
            let mut ctx = 0usize;
            if self.s.au != 0 && self.f.mi[self.mib(r - 1, c)][MIB_PALY] != 0 {
                ctx += 1;
            }
            if self.s.al != 0 && self.f.mi[self.mib(r, c - 1)][MIB_PALY] != 0 {
                ctx += 1;
            }
            if self.sym(CDF_PALYMODE + (bctx * 3 + ctx) * 3, 2) != 0 {
                self.s.psy = self.sym(CDF_PALYSIZE + bctx * 8, 7) + 2;
                self.palcolors(0);
            }
        }
        if self.s.hasc != 0 && self.s.uvmode == DC_PRED {
            let ctx = if self.s.psy != 0 { 3 } else { 0 };
            if self.sym(CDF_PALUVMODE + ctx, 2) != 0 {
                self.s.psuv = self.sym(CDF_PALUVSIZE + bctx * 8, 7) + 2;
                self.palcolors(1);
            }
        }
    }

    /// db_palcache: get_palette_cache( plane ) into pcache / pcn
    fn palcache(&mut self, p: usize) {
        let (r, c) = (self.s.mirow, self.s.micol);
        let mut an = 0usize;
        if r & 15 != 0 {
            an = self.f.mi[self.mib(r - 1, c)][MIB_PALY + p] as usize;
        }
        let mut ln = 0usize;
        if self.s.al != 0 {
            ln = self.f.mi[self.mib(r, c - 1)][MIB_PALY + p] as usize;
        }
        an &= 15; // (sizes are 0 or 2 .. 8)
        ln &= 15;
        let s = &mut *self.s;
        let above = s.apal[c as usize];
        let left = s.lpal[(r & 31) as usize];
        let col = |v: &[u16; 16], i: usize| v[(p * 8 + i) & 15];
        let (mut ai, mut li, mut n) = (0usize, 0usize, 0usize);
        while ai < an && li < ln {
            let a = col(&above, ai);
            let l = col(&left, li);
            if l < a {
                if n == 0 || l != s.pcache[n - 1] {
                    s.pcache[n & 15] = l;
                    n += 1;
                }
                li += 1;
            } else {
                if n == 0 || a != s.pcache[n - 1] {
                    s.pcache[n & 15] = a;
                    n += 1;
                }
                ai += 1;
                if a == l {
                    li += 1;
                }
            }
        }
        while ai < an {
            let a = col(&above, ai);
            ai += 1;
            if n == 0 || a != s.pcache[n - 1] {
                s.pcache[n & 15] = a;
                n += 1;
            }
        }
        while li < ln {
            let l = col(&left, li);
            li += 1;
            if n == 0 || l != s.pcache[n - 1] {
                s.pcache[n & 15] = l;
                n += 1;
            }
        }
        s.pcn = n as u32;
    }

    /// db_palcolors: the palette_colors_y (plane 0) or _u and _v (plane 1) of palette_mode_info
    fn palcolors(&mut self, p: usize) {
        self.palcache(p);
        let n = if p == 0 { self.s.psy } else { self.s.psuv } as usize;
        let bd = self.h.bd;
        let mut dst = if p == 0 { self.s.paly } else { self.s.palu };
        let mut idx = 0usize;
        let mut i = 0usize;
        while i < self.s.pcn as usize && idx < n {
            if self.lit(1) != 0 {
                dst[idx] = self.s.pcache[i];
                idx += 1;
            }
            i += 1;
        }
        if idx < n {
            dst[idx] = self.lit(bd) as u16;
            idx += 1;
            if idx < n {
                let mut pb = bd + self.lit(2) - 3; // paletteBits
                while idx < n {
                    let mut d = self.lit(pb);
                    if p == 0 {
                        d += 1; // palette_delta_y++
                    }
                    let mx = (1u32 << bd) - 1;
                    let v = (d.wrapping_add(dst[idx - 1] as u32) as i32).min(mx as i32) as u32;
                    dst[idx] = v as u16;
                    idx += 1;
                    let mut range = mx.wrapping_sub(v); // Y: ( 1 << bd ) - v - 1
                    if p != 0 {
                        range = range.wrapping_add(1); // U: ( 1 << bd ) - v
                    }
                    // CeilLog2( range )
                    let cl = if range < 2 { 0 } else { 32 - (range - 1).leading_zeros() };
                    if cl < pb {
                        pb = cl;
                    }
                }
            }
        }
        // insertion sort of n colours
        for k in 1..n {
            let v = dst[k];
            let mut j = k;
            while j > 0 && dst[j - 1] > v {
                dst[j] = dst[j - 1];
                j -= 1;
            }
            dst[j] = v;
        }
        if p == 0 {
            self.s.paly = dst;
            return;
        }
        self.s.palu = dst;
        // palette_colors_v
        if self.lit(1) != 0 {
            // delta_encode_palette_colors_v
            let bits = bd + self.lit(2) - 4;
            self.s.palv[0] = self.lit(bd) as u16;
            for k in 1..n {
                let mut d = self.lit(bits) as i32;
                if d != 0 && self.lit(1) != 0 {
                    d = -d;
                }
                let mut v = (self.s.palv[k - 1] as i32).wrapping_add(d);
                let mx = 1i32 << bd;
                if v < 0 {
                    v += mx;
                }
                if v >= mx {
                    v -= mx;
                }
                self.s.palv[k] = v.clamp(0, mx - 1) as u16; // Clip1
            }
        } else {
            for k in 0..n {
                self.s.palv[k] = self.lit(bd) as u16;
            }
        }
    }

    /// db_paltokens: palette_tokens( )
    fn paltokens(&mut self) {
        let h = self.h;
        let mut bw = self.s.bw4 * 4;
        let mut bh = self.s.bh4 * 4;
        let mut ow = ((h.micols - self.s.micol) * 4).min(bw);
        let mut oh = ((h.mirows - self.s.mirow) * 4).min(bh);
        if self.s.psy != 0 {
            self.cmap(self.s.psy & 15, false, bw, bh, ow, oh);
        }
        if self.s.psuv != 0 {
            bw >>= h.ssx;
            ow >>= h.ssx;
            bh >>= h.ssy;
            oh >>= h.ssy;
            if bw < 4 {
                bw += 2;
                ow += 2;
            }
            if bh < 4 {
                bh += 2;
                oh += 2;
            }
            self.cmap(self.s.psuv & 15, true, bw, bh, ow, oh);
        }
    }

    /// db_cmap: read one colour index map (ColorMapY / ColorMapUV, stride 64)
    fn cmap(&mut self, n: u32, uv: bool, bw: u32, bh: u32, ow: u32, oh: u32) {
        let base = if uv { PAL_CDF_UV[(n as usize).wrapping_sub(2) % 7] } else { PAL_CDF_Y[(n as usize).wrapping_sub(2) % 7] };
        // (bounds of the map: 64 x 64)
        if bw > 64 || bh > 64 || ow == 0 || oh == 0 || ow > bw || oh > bh {
            return;
        }
        let v = self.ns(n) as u8;
        self.map_mut(uv)[0] = v;
        let (ow, oh, bw, bh) = (ow as i32, oh as i32, bw as usize, bh as usize);
        let nn = n as usize;
        for i in 1..(ow + oh - 1) {
            let mut j = i.min(ow - 1);
            let jmin = (i - oh + 1).max(0);
            while j >= jmin {
                // get_palette_color_context( map, i - j, j, n )
                let r = (i - j) as usize;
                let c = j as usize;
                let mut sc = [0u8; 8];
                let mut ord = [0u8, 1, 2, 3, 4, 5, 6, 7];
                {
                    let m = self.map_mut(uv);
                    if c > 0 {
                        sc[(m[r * 64 + c - 1] & 7) as usize] += 2;
                        if r > 0 {
                            sc[(m[(r - 1) * 64 + c - 1] & 7) as usize] += 1;
                        }
                    }
                    if r > 0 {
                        sc[(m[(r - 1) * 64 + c] & 7) as usize] += 2;
                    }
                }
                for k in 0..3 {
                    let mut ms = sc[k];
                    let mut mi = k;
                    for q in k + 1..nn {
                        if sc[q & 7] > ms {
                            ms = sc[q & 7];
                            mi = q & 7;
                        }
                    }
                    if mi != k {
                        let mo = ord[mi];
                        let mut q = mi;
                        while q > k {
                            sc[q] = sc[q - 1];
                            ord[q] = ord[q - 1];
                            q -= 1;
                        }
                        sc[k] = ms;
                        ord[k] = mo;
                    }
                }
                let mut hash = sc[0] as usize + (sc[1] as usize + sc[2] as usize) * 2;
                if hash > 8 {
                    hash = 0;
                }
                let ctx = PALETTE_COLOR_CONTEXT[hash].max(0) as usize;
                let s = self.sym(base + ctx * (nn + 1), nn);
                let col = ord[(s & 7) as usize];
                self.map_mut(uv)[r * 64 + c] = col;
                j -= 1;
            }
        }
        // extend to the block size
        let (ow, oh) = (ow as usize, oh as usize);
        let m = self.map_mut(uv);
        for i in 0..oh {
            let v = m[i * 64 + ow - 1];
            m[i * 64 + ow..i * 64 + bw].fill(v);
        }
        for i in oh..bh {
            m.copy_within((oh - 1) * 64..(oh - 1) * 64 + bw, i * 64);
        }
    }

    #[inline]
    fn map_mut(&mut self, uv: bool) -> &mut [u8; 4096] {
        if uv { &mut self.s.cmapuv } else { &mut self.s.cmapy }
    }

    // ------------------------------------------------------------------ transform size

    /// db_txsize: read_block_tx_size( ) of an intra block (read_tx_size( 1 ))
    fn txsize(&mut self) {
        let h = self.h;
        self.s.txsz = TX_4X4;
        if self.s.ll != 0 {
            return;
        }
        let ms = self.s.misize as usize;
        let mut tx = MAX_TX_SIZE_RECT[ms] as u32; // maxRectTxSize
        self.s.txsz = tx;
        if ms == 0 || h.txmode != TX_MODE_SELECT {
            return;
        }
        let depth = MAX_TX_DEPTH[ms];
        let (r, c) = (self.s.mirow, self.s.micol);
        let mut ctx = 0usize;
        if self.s.au != 0 {
            let a = self.f.mi[self.mib(r - 1, c)][MIB_TX] as usize;
            if TX_WIDTH[a] >= TX_WIDTH[tx as usize] {
                ctx += 1;
            }
        }
        if self.s.al != 0 {
            let l = self.f.mi[self.mib(r, c - 1)][MIB_TX] as usize;
            if TX_HEIGHT[l] >= TX_HEIGHT[tx as usize] {
                ctx += 1;
            }
        }
        let mut d = match depth {
            4 => self.sym(CDF_TX64 + ctx * 4, 3),
            3 => self.sym(CDF_TX32 + ctx * 4, 3),
            2 => self.sym(CDF_TX16 + ctx * 4, 3),
            _ => self.sym(CDF_TX8 + ctx * 3, 2),
        };
        while d != 0 {
            tx = SPLIT_TX_SIZE[tx as usize] as u32;
            d -= 1;
        }
        self.s.txsz = tx;
    }

    // ------------------------------------------------------------------ residual

    /// av1_uvtx: get_tx_size( 1, TxSize )
    pub fn uvtx(&self) -> u32 {
        let h = self.h;
        let mut ps = SUBSAMPLED_SIZE[(self.s.misize * 4 + h.ssx * 2 + h.ssy) as usize] as u32;
        if ps >= BLOCK_INVALID {
            ps = 0;
        }
        let uv = MAX_TX_SIZE_RECT[ps as usize] as u32;
        let (w, hh) = (TX_WIDTH[uv as usize], TX_HEIGHT[uv as usize]);
        if w == 64 || hh == 64 {
            if w == 16 {
                return TX_16X32;
            }
            if hh == 16 {
                return TX_32X16;
            }
            return TX_32X32;
        }
        uv
    }

    /// db_residual: residual( ) of an intra block
    fn residual(&mut self) {
        let h = self.h;
        let wc = (self.s.bw4 >> 4).max(1); // widthChunks
        let hc = (self.s.bh4 >> 4).max(1);
        let msc = if wc > 1 || hc > 1 { BLOCK_64X64 } else { self.s.misize }; // miSizeChunk
        for cy in 0..hc {
            for cx in 0..wc {
                for p in 0..(self.s.hasc * 2 + 1) {
                    let tx = if self.s.ll != 0 {
                        TX_4X4
                    } else if p == 0 {
                        self.s.txsz
                    } else {
                        self.uvtx()
                    };
                    let stepx = (TX_WIDTH[tx as usize] >> 2) as u32;
                    let stepy = (TX_HEIGHT[tx as usize] >> 2) as u32;
                    let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
                    let mut psz = SUBSAMPLED_SIZE[(msc * 4 + sx * 2 + sy) as usize] as u32;
                    if psz >= BLOCK_INVALID {
                        psz = 0;
                    }
                    let n4w = NUM_4X4_BLOCKS_WIDE[psz as usize] as u32;
                    let n4h = NUM_4X4_BLOCKS_HIGH[psz as usize] as u32;
                    let bx = (self.s.micol >> sx) << 2; // baseXBlock
                    let by = (self.s.mirow >> sy) << 2;
                    let mut y = 0;
                    while y < n4h {
                        let mut x = 0;
                        while x < n4w {
                            // transform_block( plane, baseXBlock, baseYBlock, txSz, x + ..., y + ... )
                            self.s.ppl = p;
                            self.s.ptx = tx;
                            self.s.pbx = ((cx << 4) >> sx) + x;
                            self.s.pby = ((cy << 4) >> sy) + y;
                            self.tblock(bx, by);
                            x += stepx;
                        }
                        y += stepy;
                    }
                }
            }
        }
    }

    /// db_tblock: transform_block( ppl, baseX, baseY, ptx, pbx, pby )
    fn tblock(&mut self, bx: u32, by: u32) {
        if !self.tbpred(bx, by) {
            return;
        }
        if self.s.skip == 0 && self.coeffs() != 0 {
            self.recon();
        }
        self.tbflags();
    }

    /// av1_tbpred: transform_block up to the prediction; false (nothing done) when the
    /// transform block lies off screen
    pub fn tbpred(&mut self, bx: u32, by: u32) -> bool {
        let h = self.h;
        let p = self.s.ppl;
        let tx = self.s.ptx as usize;
        let x = bx + self.s.pbx * 4; // startX
        let y = by + self.s.pby * 4;
        self.s.px = x;
        self.s.py = y;
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        // off-screen transform blocks
        if x >= (h.micols << 2) >> sx || y >= (h.mirows << 2) >> sy {
            return false;
        }
        // row / col (luma 4x4), position in the superblock
        let s = &mut *self.s;
        s.prow = (y << sy) >> 2;
        s.pcol = (x << sx) >> 2;
        let mask = if h.sb128 != 0 { 31 } else { 15 };
        s.psbr = (s.prow & mask) >> sy;
        s.psbc = (s.pcol & mask) >> sx;
        // prediction
        let pal = if p == 0 { s.psy } else { s.psuv };
        if pal != 0 {
            self.palpred();
        } else {
            let (mut mode, mut hl, mut ha) = (s.ymode, s.al, s.au);
            if p != 0 {
                mode = s.uvmode;
                if mode == UV_CFL_PRED {
                    mode = DC_PRED; // CFL: DC prediction first
                }
                hl = s.alc;
                ha = s.auc;
            }
            s.pmode = mode;
            if s.pbx != 0 {
                hl = 1;
            }
            s.phl = hl;
            if s.pby != 0 {
                ha = 1;
            }
            s.pha = ha;
            s.plw = TX_WIDTH_LOG2[tx] as u32;
            s.plh = TX_HEIGHT_LOG2[tx] as u32;
            // haveAboveRight = BlockDecoded[ plane ][ sbr - 1 ][ sbc + stepX ]
            let bd = &s.bdec[p as usize];
            let stepx = (TX_WIDTH[tx] >> 2) as u32;
            s.phar = bd[(s.psbr * 34 + s.psbc + stepx + 1) as usize] as u32;
            // haveBelowLeft = BlockDecoded[ plane ][ sbr + stepY ][ sbc - 1 ]
            let stepy = (TX_HEIGHT[tx] >> 2) as u32;
            s.phbl = bd[((s.psbr + stepy + 1) * 34 + s.psbc) as usize] as u32;
            self.predict();
            if p != 0 && self.s.uvmode == UV_CFL_PRED {
                self.cfl();
            }
        }
        if p == 0 {
            self.s.maxlw = x + TX_WIDTH[tx] as u32;
            self.s.maxlh = y + TX_HEIGHT[tx] as u32;
        }
        true
    }

    /// av1_tbflags: end of transform_block (after tbpred returned true): LoopfilterTxSizes and
    /// BlockDecoded of the ptx block of plane ppl
    pub fn tbflags(&mut self) {
        let h = self.h;
        let p = self.s.ppl as usize;
        let tx = self.s.ptx;
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let stepx = (TX_WIDTH[tx as usize] >> 2) as usize;
        let stepy = (TX_HEIGHT[tx as usize] >> 2) as u32;
        let r10 = self.s.prow >> sy;
        let r11 = self.s.pcol >> sx;
        for i in 0..stepy {
            let m = self.mib(r10 + i, r11);
            for e in &mut self.f.mi[m..m + stepx] {
                e[MIB_LTX + p] = tx as u8;
            }
            let b = ((self.s.psbr + i + 1) * 34 + self.s.psbc + 1) as usize;
            self.s.bdec[p][b..b + stepx].fill(1);
        }
    }
}

/// se_recenter: inverse_recenter( r, v )
fn recenter(v: i32, r: i32) -> i32 {
    if v > r.wrapping_add(r) {
        return v;
    }
    if v & 1 != 0 {
        r.wrapping_sub(((v as u32).wrapping_add(1) >> 1) as i32)
    } else {
        r.wrapping_add(((v as u32) >> 1) as i32)
    }
}
