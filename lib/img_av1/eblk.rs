// eblk -- AV1 encoder: superblocks of a tile (partition search, intra mode decision, block
// coding) of the tile encoder Et  [src/img/av1_eblk.s]
//
//   Et::tile() -> 0 / ERR_*    every superblock of the tile mrs .. mce (TS set up by the decoder's
//                              tilesetup, range encoder started): searched with estimated rates
//                              (est), state restored, then coded
//
// Model: tools/av1_enc_ref.py (encode_tile, snap / restore, part_cost / code_partition, search,
// emit, set_block, mode_cost, best_mode, choose_modes, block, mode_info).  Snapshots: two per
// depth (slot 0 = the block's entry state, 1 = after PARTITION_NONE, or after the luma decision
// inside choose); decisions: nodes (ND_* kind, yMode, uvMode per node; node of depth d at
// ((r & 15) >> (4 - d), (c & 15) >> (4 - d))).

use alloc::vec::Vec;

use img_core::ERR_CANCEL;

use crate::blk::{T, MIB_DLF, MIB_PALY, MIB_SEG, MIB_SIZE, MIB_SKIP, MIB_TX, MIB_UVMODE, MIB_YMODE};
use crate::eec::cost;
use crate::enc::Et;
use crate::etx::{tx_n, Lv};
use crate::tab::*;

const NCAND: usize = 3; // modes kept by the SATD pre-selection
const MAX_ANGLE_DELTA: u32 = 3;
const BLOCK_4X4: u32 = 0;
const BLOCK_8X8: u32 = 3;
const BLOCK_64X64: u32 = 12;
const PARTITION_NONE: u32 = 0;
const PARTITION_SPLIT: u32 = 3;
const V_PRED: u32 = 1;
const D67_PRED: u32 = 8;
const TX_4X4: u32 = 0;
const MIB_PALUV: usize = 7;
const ND_NONE: u8 = 0;
const ND_SPLIT: u8 = 1;
const ND_OUT: u8 = 2;
// first node of each depth
static ND_BASE: [u32; 4] = [0, 1, 5, 21];

/// a snapshot of the state a block can change (ae_snap / ae_restore)
#[derive(Default)]
pub(crate) struct Snap {
    r: u32,
    c: u32,
    bsize: u32,
    bdec: Vec<[u8; 34 * 34]>, // BlockDecoded
    ctx: [[[u8; 16]; 4]; 3],  // per plane: above lvl / dc, left lvl / dc
    mib: Vec<[u8; 16]>,       // MIB rows r .. r + bh4 - 1, bw4 records each
    px: [Vec<u16>; 3],        // rows of samples per plane
}

// sn_copy: save (restore false: state -> slot) or restore (true: slot -> state) a snapshot
fn sn_copy(t: &mut T, s: &mut Snap, restore: bool) {
    let (r, c) = (s.r, s.c);
    let bw4 = NUM_4X4_BLOCKS_WIDE[s.bsize as usize] as u32;
    let bh4 = NUM_4X4_BLOCKS_HIGH[s.bsize as usize] as u32;
    let h = t.h;
    let ts = &mut *t.s;
    // BlockDecoded
    if restore {
        ts.bdec.copy_from_slice(&s.bdec);
    } else {
        s.bdec.clear();
        s.bdec.extend_from_slice(&ts.bdec);
    }
    // contexts per plane
    let n = ts.acols as usize;
    for p in 0..h.np as usize {
        let (sx, sy) = if p != 0 { (h.ssx, h.ssy) } else { (0, 0) };
        // above: start c >> subX, n = Max(1, bw4 >> subX)
        let an = (bw4 >> sx).max(1) as usize;
        let a0 = p * n + (c >> sx) as usize;
        // left: start (r >> subY) & 31, n = Max(1, bh4 >> subY)
        let ln = (bh4 >> sy).max(1) as usize;
        let l0 = ((r >> sy) & 31) as usize;
        let k = &mut s.ctx[p];
        if restore {
            ts.alvl[a0..a0 + an].copy_from_slice(&k[0][..an]);
            ts.adc[a0..a0 + an].copy_from_slice(&k[1][..an]);
            ts.llvl[p][l0..l0 + ln].copy_from_slice(&k[2][..ln]);
            ts.ldc[p][l0..l0 + ln].copy_from_slice(&k[3][..ln]);
        } else {
            k[0][..an].copy_from_slice(&ts.alvl[a0..a0 + an]);
            k[1][..an].copy_from_slice(&ts.adc[a0..a0 + an]);
            k[2][..ln].copy_from_slice(&ts.llvl[p][l0..l0 + ln]);
            k[3][..ln].copy_from_slice(&ts.ldc[p][l0..l0 + ln]);
        }
    }
    // MIB rows r .. r + bh4 - 1, bw4 records each
    let f = &mut *t.f;
    let (mis, w) = (f.mis as usize, bw4 as usize);
    if !restore {
        s.mib.clear();
    }
    for i in 0..bh4 as usize {
        let m = (r as usize + i) * mis + c as usize;
        if restore {
            f.mi[m..m + w].copy_from_slice(&s.mib[i * w..i * w + w]);
        } else {
            s.mib.extend_from_slice(&f.mi[m..m + w]);
        }
    }
    // samples: plane rows y0 .. y0 + ph - 1, pw samples (Max(4, size >> sub))
    for p in 0..h.np as usize {
        let (sx, sy) = if p != 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let pw = ((bw4 << 2) >> sx).max(4) as usize;
        let x0 = ((c << 2) >> sx) as usize;
        let ph = ((bh4 << 2) >> sy).max(4) as usize;
        let y0 = ((r << 2) >> sy) as usize;
        let v = &mut s.px[p];
        let st = f.ps[p] as usize;
        let pl = &mut f.pl[p];
        if restore {
            for i in 0..ph {
                let o = (y0 + i) * st + x0;
                pl[o..o + pw].copy_from_slice(&v[i * pw..i * pw + pw]);
            }
        } else {
            v.clear();
            for i in 0..ph {
                let o = (y0 + i) * st + x0;
                v.extend_from_slice(&pl[o..o + pw]);
            }
        }
    }
}

impl Et<'_> {
    // ------------------------------------------------------------------ tile

    /// ae_tile -> 0 / ERR_*
    pub(crate) fn tile(&mut self) -> i32 {
        let mut r = self.t.s.mrs;
        while r < self.t.s.mre {
            // clear_left_context (and the left palettes)
            self.t.s.llvl = [[0; 32]; 3];
            self.t.s.ldc = [[0; 32]; 3];
            self.t.s.lpal = [[0; 16]; 32];
            let mut c = self.t.s.mcs;
            while c < self.t.s.mce {
                if self.ef.cancelled() {
                    self.err = ERR_CANCEL;
                    return self.err;
                }
                self.t.clearbd(r, c, 16);
                // search (its entry state lands in slot (0, 0)), restore, code
                self.ec.est = true;
                self.ec.bits = 0;
                self.search(r, c, BLOCK_64X64, 0);
                self.restore(0, 0);
                self.ec.est = false;
                self.emit(r, c, BLOCK_64X64, 0);
                if self.err != 0 {
                    return self.err;
                }
                c += 16;
            }
            r += 16;
        }
        self.err
    }

    // ------------------------------------------------------------------ snapshots

    /// ae_snap: save the state a block can change into slot (d, k)
    pub(crate) fn snap(&mut self, d: usize, k: usize, r: u32, c: u32, bsize: u32) {
        let s = &mut self.snaps[d][k];
        s.r = r;
        s.c = c;
        s.bsize = bsize;
        sn_copy(&mut self.t, s, false);
    }

    /// ae_restore: back to the state of slot (d, k)
    pub(crate) fn restore(&mut self, d: usize, k: usize) {
        sn_copy(&mut self.t, &mut self.snaps[d][k], true);
    }

    // ------------------------------------------------------------------ block parameters

    /// ae_setblock: set_block: TS fields of the current block (no palette, filter intra,
    /// segment, angle delta, CFL; TxSize = the largest rectangular one or TX_4X4 when lossless)
    pub(crate) fn setblock(&mut self, r: u32, c: u32, bsize: u32) {
        let h = self.t.h;
        let ts = &mut *self.t.s;
        ts.mirow = r;
        ts.micol = c;
        ts.misize = bsize;
        let bw4 = NUM_4X4_BLOCKS_WIDE[bsize as usize] as u32;
        let bh4 = NUM_4X4_BLOCKS_HIGH[bsize as usize] as u32;
        ts.bw4 = bw4;
        ts.bh4 = bh4;
        let hasc = if (bh4 == 1 && h.ssy != 0 && r & 1 == 0) || (bw4 == 1 && h.ssx != 0 && c & 1 == 0) {
            0
        } else {
            (h.np > 1) as u32
        };
        ts.hasc = hasc;
        ts.au = (r > ts.mrs) as u32;
        ts.auc = ts.au;
        ts.al = (c > ts.mcs) as u32;
        ts.alc = ts.al;
        if hasc == 0 {
            ts.auc = 0;
            ts.alc = 0;
        } else {
            if h.ssy != 0 && bh4 == 1 {
                ts.auc = (r >= ts.mrs + 2) as u32;
            }
            if h.ssx != 0 && bw4 == 1 {
                ts.alc = (c >= ts.mcs + 2) as u32;
            }
        }
        ts.seg = 0;
        ts.ufi = 0;
        ts.psy = 0;
        ts.psuv = 0;
        ts.ady = 0;
        ts.aduv = 0;
        ts.cflu = 0;
        ts.cflv = 0;
        ts.ll = h.llarr[0] as u32;
        ts.qidx = h.baseq;
        ts.txsz = if ts.ll != 0 { TX_4X4 } else { MAX_TX_SIZE_RECT[bsize as usize] as u32 };
    }

    // MIB[r][c][k]
    #[inline]
    fn mibv(&self, r: u32, c: u32, k: usize) -> u8 {
        self.t.f.mi[self.t.mib(r, c)][k]
    }

    // ------------------------------------------------------------------ mode syntax

    // ymodecdf: CDF offset (u16 entries) of intra_frame_y_mode (above / left mode contexts)
    fn ymodecdf(&self) -> usize {
        let ts = &*self.t.s;
        let a = if ts.au != 0 { INTRA_MODE_CONTEXT[self.mibv(ts.mirow - 1, ts.micol, MIB_YMODE) as usize] as usize } else { 0 };
        let l = if ts.al != 0 { INTRA_MODE_CONTEXT[self.mibv(ts.mirow, ts.micol - 1, MIB_YMODE) as usize] as usize } else { 0 };
        CDF_IFYMODE + (a * 5 + l) * 14
    }

    // uvmodecdf: CDF offset of uv_mode and its symbol count (uv_mode_cdf: CFL allowed or not)
    fn uvmodecdf(&self) -> (usize, u32) {
        let ts = &*self.t.s;
        let h = self.t.h;
        let m = ts.ymode as usize;
        let cfl = if ts.ll != 0 {
            // lossless: CFL allowed when the chroma residual size is BLOCK_4X4
            SUBSAMPLED_SIZE[(ts.misize * 4 + h.ssx * 2 + h.ssy) as usize] as u32 == BLOCK_4X4
        } else {
            NUM_4X4_BLOCKS_WIDE[ts.misize as usize] <= 8 && NUM_4X4_BLOCKS_HIGH[ts.misize as usize] <= 8
        };
        if cfl { (CDF_UVMODE_C + m * 15, 14) } else { (CDF_UVMODE_NC + m * 14, 13) }
    }

    /// ae_modecost: Q8 bits of the luma (pl 0) or chroma mode symbol (+ angle delta 0)
    pub(crate) fn modecost(&self, pl: u32, mode: u32) -> u32 {
        let off = if pl == 0 { self.ymodecdf() } else { self.uvmodecdf().0 };
        let mut r = cost(&self.t.s.cdf[off..], mode);
        if self.t.s.misize >= BLOCK_8X8 && (V_PRED..=D67_PRED).contains(&mode) {
            let off = CDF_ANGLE + ((mode - V_PRED) * 8) as usize;
            r += cost(&self.t.s.cdf[off..], MAX_ANGLE_DELTA);
        }
        r
    }

    /// ae_modeinfo: intra_frame_mode_info written: skip, y mode (+ angle delta 0), uv mode
    /// (+ angle delta 0)
    pub(crate) fn modeinfo(&mut self, skip: u32) {
        let ts = &*self.t.s;
        let mut ctx = 0usize; // skip context
        if ts.au != 0 {
            ctx += self.mibv(ts.mirow - 1, ts.micol, MIB_SKIP) as usize;
        }
        if ts.al != 0 {
            ctx += self.mibv(ts.mirow, ts.micol - 1, MIB_SKIP) as usize;
        }
        let off = CDF_SKIP + ctx * 3;
        self.ec.put(&mut self.t.s.cdf[off..], skip, 2);
        let off = self.ymodecdf();
        let ym = self.t.s.ymode;
        self.ec.put(&mut self.t.s.cdf[off..], ym, 13);
        self.mi_angle(ym);
        if self.t.s.hasc != 0 {
            let (off, n) = self.uvmodecdf();
            let uvm = self.t.s.uvmode;
            self.ec.put(&mut self.t.s.cdf[off..], uvm, n);
            self.mi_angle(uvm);
        }
    }

    // mi_angle: angle_delta (0) of a directional mode of a block of 8x8 and more
    fn mi_angle(&mut self, mode: u32) {
        if self.t.s.misize >= BLOCK_8X8 && (V_PRED..=D67_PRED).contains(&mode) {
            let off = CDF_ANGLE + ((mode - V_PRED) * 8) as usize;
            self.ec.put(&mut self.t.s.cdf[off..], MAX_ANGLE_DELTA, 7);
        }
    }

    /// ae_partcode: partition symbol (or the split / horz / vert bool at the frame edges;
    /// nothing when neither half is inside)
    pub(crate) fn partcode(&mut self, r: u32, c: u32, bsize: u32, partition: u32) {
        let half = NUM_4X4_BLOCKS_WIDE[bsize as usize] as u32 >> 1;
        let has_rows = r + half < self.t.h.mirows;
        let has_cols = c + half < self.t.h.micols;
        if !has_rows && !has_cols {
            return;
        }
        // partition CDF: ctx = left * 2 + above (neighbour narrower / shorter than this block)
        let bsl = MI_WIDTH_LOG2[bsize as usize] as u32;
        let mut ctx = 0u32;
        if r > self.t.s.mrs && (MI_WIDTH_LOG2[self.mibv(r - 1, c, MIB_SIZE) as usize] as u32) < bsl {
            ctx |= 1;
        }
        if c > self.t.s.mcs && (MI_HEIGHT_LOG2[self.mibv(r, c - 1, MIB_SIZE) as usize] as u32) < bsl {
            ctx |= 2;
        }
        let (off, n) = if bsl == 1 {
            (CDF_PART8 + (ctx * 5) as usize, 4)
        } else {
            (CDF_PART16 + ((bsl - 2) * 4 * 11 + ctx * 11) as usize, 10)
        };
        if has_rows && has_cols {
            self.ec.put(&mut self.t.s.cdf[off..], partition, n);
            return;
        }
        // psum of the partitions that split the inside half; bool split_or_horz / split_or_vert
        let cdf = &self.t.s.cdf;
        let f = |i: usize| cdf[off + i] as u32;
        let psum = if has_cols {
            // hasCols: VERT, SPLIT, HORZ_A, VERT_A, VERT_B, VERT_4
            f(2).wrapping_sub(f(1))
                .wrapping_add(f(3))
                .wrapping_sub(f(2))
                .wrapping_add(f(4))
                .wrapping_sub(f(3))
                .wrapping_add(f(6))
                .wrapping_sub(f(5))
                .wrapping_add(f(7))
                .wrapping_sub(f(6))
                .wrapping_add(f(9))
                .wrapping_sub(f(8))
        } else {
            // hasRows: HORZ, SPLIT, HORZ_A, HORZ_B, VERT_A, HORZ_4
            f(1).wrapping_sub(f(0))
                .wrapping_add(f(3))
                .wrapping_sub(f(2))
                .wrapping_add(f(4))
                .wrapping_sub(f(3))
                .wrapping_add(f(5))
                .wrapping_sub(f(4))
                .wrapping_add(f(6))
                .wrapping_sub(f(5))
                .wrapping_add(f(8))
                .wrapping_sub(f(7))
        };
        let mut b = [32768u32.wrapping_sub(psum) as u16, 32768, 0];
        self.ec.put(&mut b, (partition == PARTITION_SPLIT) as u32, 2);
    }

    /// ae_partcost: Q8 bits of partcode (0 below 8x8)
    pub(crate) fn partcost(&mut self, r: u32, c: u32, bsize: u32, partition: u32) -> u64 {
        if bsize < BLOCK_8X8 {
            return 0;
        }
        let (est, bits) = (self.ec.est, self.ec.bits);
        self.ec.est = true;
        self.ec.bits = 0;
        self.partcode(r, c, bsize, partition);
        let b = self.ec.bits;
        self.ec.est = est;
        self.ec.bits = bits;
        b
    }

    // ------------------------------------------------------------------ partition search

    // ndaddr: the node of (r, c) at depth d
    #[inline]
    fn ndaddr(r: u32, c: u32, d: u32) -> usize {
        let s = 4 - d;
        (ND_BASE[d as usize] + ((((r & 15) >> s) << d) + ((c & 15) >> s))) as usize
    }

    /// ae_search -> J of the best partitioning of the block (model search); the decisions in
    /// nodes, the state that of the best choice
    pub(crate) fn search(&mut self, r: u32, c: u32, bsize: u32, depth: u32) -> u64 {
        let node = Self::ndaddr(r, c, depth);
        if r >= self.t.h.mirows || c >= self.t.h.micols {
            self.nodes[node][0] = ND_OUT;
            return 0;
        }
        let d = depth as usize;
        let half = NUM_4X4_BLOCKS_WIDE[bsize as usize] as u32 >> 1;
        let has_rows = r + half < self.t.h.mirows;
        let has_cols = c + half < self.t.h.micols;
        let can_none = (has_rows && has_cols) || bsize == BLOCK_8X8;
        let can_split = bsize > BLOCK_8X8;
        self.snap(d, 0, r, c, bsize);
        let mut best = u64::MAX;
        if can_none {
            // PARTITION_NONE
            let j = self.block(r, c, bsize, depth, None);
            best = self.partcost(r, c, bsize, PARTITION_NONE).wrapping_mul(self.lam).wrapping_add(j);
            let dec = self.dec;
            self.nodes[node][0] = ND_NONE;
            self.nodes[node][1] = dec as u8;
            self.nodes[node][2] = (dec >> 8) as u8;
            if can_split {
                self.snap(d, 1, r, c, bsize);
                self.restore(d, 0);
            }
        }
        if can_split {
            let mut js = self.partcost(r, c, bsize, PARTITION_SPLIT).wrapping_mul(self.lam);
            let sub = PARTITION_SUBSIZE[(PARTITION_SPLIT * 22 + bsize) as usize] as u32;
            for q in 0..4 {
                let rr = if q & 2 != 0 { r + half } else { r };
                let cc = if q & 1 != 0 { c + half } else { c };
                js = js.wrapping_add(self.search(rr, cc, sub, depth + 1));
            }
            if !can_none || js < best {
                best = js;
                self.nodes[node][0] = ND_SPLIT;
            } else {
                self.restore(d, 1);
            }
        }
        best
    }

    /// ae_emit: code the decided partitioning (model emit)
    pub(crate) fn emit(&mut self, r: u32, c: u32, bsize: u32, depth: u32) {
        if r >= self.t.h.mirows || c >= self.t.h.micols {
            return;
        }
        let nd = self.nodes[Self::ndaddr(r, c, depth)];
        let half = NUM_4X4_BLOCKS_WIDE[bsize as usize] as u32 >> 1;
        let partition = if nd[0] == ND_NONE { PARTITION_NONE } else { PARTITION_SPLIT };
        if bsize >= BLOCK_8X8 {
            self.partcode(r, c, bsize, partition);
        }
        if partition == PARTITION_NONE {
            self.block(r, c, bsize, depth, Some(nd[1] as u32 | (nd[2] as u32) << 8));
            return;
        }
        let sub = PARTITION_SUBSIZE[(PARTITION_SPLIT * 22 + bsize) as usize] as u32;
        for q in 0..4 {
            let rr = if q & 2 != 0 { r + half } else { r };
            let cc = if q & 1 != 0 { c + half } else { c };
            self.emit(rr, cc, sub, depth + 1);
        }
    }

    // ------------------------------------------------------------------ blocks

    /// ae_block -> J: one block (model block): dec None searches the modes (choose), else dec =
    /// yMode | uvMode << 8; prediction, quantization and reconstruction of every plane, then mode
    /// info and coefficients through Ec::put (estimated or written); J = SSD * 65536 + lambda *
    /// bits; self.dec = the decisions
    pub(crate) fn block(&mut self, r: u32, c: u32, bsize: u32, depth: u32, dec: Option<u32>) -> u64 {
        self.setblock(r, c, bsize);
        let dec = match dec {
            Some(d) => d,
            None => self.choose(bsize, depth),
        };
        self.dec = dec;
        self.t.s.ymode = dec & 0xFF;
        self.t.s.uvmode = (dec >> 8) & 0xFF;
        let mut dist = self.tryplane(0, self.t.s.ymode, true);
        self.ntxb[1] = 0;
        self.ntxb[2] = 0;
        if self.t.s.hasc != 0 {
            dist += self.tryplane(1, self.t.s.uvmode, false);
            dist += self.tryplane(2, self.t.s.uvmode, false);
        }
        // skip = every level of every plane zero
        let mut skip = 1;
        for p in 0..3 {
            let cnt = (self.planen(p).0).wrapping_mul(self.ntxb[p as usize]) as usize;
            if self.qb[p as usize][..cnt].iter().any(|&v| v != 0) {
                skip = 0;
                break;
            }
        }
        let bits0 = self.ec.bits;
        self.t.s.skip = skip; // (ET_SKIP: written only)
        self.modeinfo(skip);
        if skip != 0 {
            self.t.blkreset();
        }
        // YModes, UVModes, InterTxSizes
        let (bw4, bh4) = (self.t.s.bw4, self.t.s.bh4);
        let (ym, uvm, tx, hasc) = (self.t.s.ymode as u8, self.t.s.uvmode as u8, self.t.s.txsz as u8, self.t.s.hasc != 0);
        for i in 0..bh4 {
            let m = self.t.mib(r + i, c);
            for e in &mut self.t.f.mi[m..m + bw4 as usize] {
                e[MIB_YMODE] = ym;
                if hasc {
                    e[MIB_UVMODE] = uvm;
                }
                e[MIB_TX] = tx;
            }
        }
        if skip == 0 {
            for p in 0..3u32 {
                let (n, txsz) = self.planen(p);
                for k in 0..self.ntxb[p as usize] as usize {
                    let e = self.txb[p as usize][k];
                    let lv = Lv::Qb(p as usize, k * n as usize);
                    self.txbcode(p, e as u32, lv, txsz, (e >> 32) as u32, true);
                }
            }
        }
        // Skips, TxSizes, MiSizes, SegmentIds, PaletteSizes, DeltaLFs
        for i in 0..bh4 {
            let m = self.t.mib(r + i, c);
            for e in &mut self.t.f.mi[m..m + bw4 as usize] {
                e[MIB_SKIP] = skip as u8;
                e[MIB_TX] = tx;
                e[MIB_SIZE] = bsize as u8;
                e[MIB_SEG] = 0;
                e[MIB_PALY] = 0;
                e[MIB_PALUV] = 0;
                e[MIB_DLF..MIB_DLF + 4].fill(0);
            }
        }
        (self.ec.bits.wrapping_sub(bits0)).wrapping_mul(self.lam).wrapping_add(dist << 16)
    }

    // bk_planen: coefficients per transform block of the plane (tw * th) and its txSz (0 when the
    // plane does not exist)
    fn planen(&self, plane: u32) -> (u32, u32) {
        if plane >= self.t.h.np {
            return (0, 0);
        }
        let txsz = if self.t.s.ll != 0 {
            TX_4X4
        } else if plane == 0 {
            self.t.s.txsz
        } else {
            self.t.uvtx()
        };
        (tx_n(txsz) as u32, txsz)
    }

    /// ae_choose -> yMode | uvMode << 8 (model choose_modes; the entry state is in slot
    /// (depth, 0), taken by search right before)
    pub(crate) fn choose(&mut self, bsize: u32, depth: u32) -> u32 {
        let d = depth as usize;
        let (r0, c0) = (self.snaps[d][0].r, self.snaps[d][0].c);
        let y = self.bestmode(0, d, 0, None);
        let mut uv = 0;
        if self.t.s.hasc != 0 {
            self.restore(d, 0);
            self.setblock(r0, c0, bsize);
            self.tryplane(0, y, false);
            self.snap(d, 1, r0, c0, bsize);
            uv = self.bestmode(1, d, 1, Some(y));
        }
        self.restore(d, 0);
        self.setblock(r0, c0, bsize);
        y | uv << 8
    }

    /// ae_bestmode -> the luma (pl 0) or chroma (pl 1: planes 1 and 2) intra mode with the
    /// smallest J (model best_mode): a single transform block pre-selects NCAND modes by
    /// SATD * 32768 + lambda_satd * mode bits
    pub(crate) fn bestmode(&mut self, pl: u32, d: usize, k: usize, ymode: Option<u32>) -> u32 {
        let txsz = self.txblocks(pl);
        let tw = TX_WIDTH[txsz as usize] as u32;
        let th = TX_HEIGHT[txsz as usize] as u32;
        let mut cand: [u8; 13] = core::array::from_fn(|i| i as u8);
        let mut ncand = 13;
        if self.tbn == 1 {
            let xy = self.tbl[0]; // x | y << 16 of the single block
            let mut costs = [0u64; 13];
            for (m, cm) in costs.iter_mut().enumerate() {
                let mut satd = 0u64;
                let planes: &[u32] = if pl == 0 { &[0] } else { &[1, 2] };
                for &plane in planes {
                    if plane == 0 {
                        self.t.s.ymode = m as u32;
                    } else {
                        self.t.s.uvmode = m as u32;
                    }
                    self.t.s.ppl = plane;
                    self.t.s.ptx = txsz;
                    self.t.s.pbx = 0;
                    self.t.s.pby = 0;
                    self.t.tbpred(xy & 0xFFFF, xy >> 16);
                    self.residual(plane, xy, txsz);
                    satd += self.satd(tw, th);
                }
                *cm = (self.modecost(pl, m as u32) as u64).wrapping_mul(self.lams).wrapping_add(satd << 15);
            }
            // the NCAND smallest (cost, mode) in order
            let mut used = [false; 13];
            for slot in cand.iter_mut().take(NCAND) {
                let mut min = u64::MAX;
                let mut mm = usize::MAX;
                for (i, &cv) in costs.iter().enumerate() {
                    if !used[i] && cv < min {
                        min = cv;
                        mm = i;
                    }
                }
                used[mm] = true;
                *slot = mm as u8;
            }
            ncand = NCAND;
        }
        // rate-distortion of the candidates
        let mut best_j = u64::MAX;
        let mut best = 0;
        for &m in cand.iter().take(ncand) {
            let m = m as u32;
            self.restore(d, k);
            let (sr, sc, sb) = (self.snaps[d][k].r, self.snaps[d][k].c, self.snaps[d][k].bsize);
            self.setblock(sr, sc, sb);
            if let Some(y) = ymode {
                self.t.s.ymode = y;
            }
            let bits0 = self.ec.bits;
            let mut dist = 0u64;
            let mut plane = pl;
            loop {
                dist += self.tryplane(plane, m, false);
                let (n, ptx) = self.planen(plane);
                for kk in 0..self.ntxb[plane as usize] as usize {
                    let e = self.txb[plane as usize][kk];
                    let lv = Lv::Qb(plane as usize, kk * n as usize);
                    self.txbcode(plane, e as u32, lv, ptx, (e >> 32) as u32, true); // update (estimate mode)
                }
                if pl == 0 || plane >= 2 {
                    break;
                }
                plane += 1;
            }
            let mc = self.modecost(pl, m) as u64;
            let j = (self.ec.bits.wrapping_sub(bits0).wrapping_add(mc))
                .wrapping_mul(self.lam)
                .wrapping_add(dist << 16);
            if j < best_j {
                best_j = j;
                best = m;
            }
        }
        best
    }
}
