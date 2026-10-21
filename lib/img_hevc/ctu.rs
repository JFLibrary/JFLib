// ctu -- HEVC intra decoder: CABAC and slice segment data (coding tree units)
//                                                                [src/img/hevc_ctu.s]
//
//   slice_data() -> 0 / ERR_*    decode the slice segment data of the NAL in buf (header
//                                parsed by slice(), picture set up by newpic()): SAO syntax,
//                                coding quadtrees, intra prediction and residuals (rec.rs),
//                                per-4x4 maps for the loop filters; WPP / tiles / dependent
//                                slices  [hvc_slice_data]
//
// The CABAC tables (LPS, TLPS, TMPS, INIT) and the scan tables are shared with the encoder
// (enc.rs).

use img_core::{ERR_CANCEL, ERR_HDR, ERR_UNSUPPORTED};

use crate::dec::{Hevc, HV_NCTX, MB_PCM, MB_PCMOFF};

// ---- context offsets (CX_*: index into the context array, value = pStateIdx << 1 | valMps)
pub(crate) const CX_SAOMERGE: usize = 0;
pub(crate) const CX_SAOTYPE: usize = 1;
pub(crate) const CX_SPLITCU: usize = 2; // 3
pub(crate) const CX_TQBYPASS: usize = 5;
pub(crate) const CX_PARTMODE: usize = 6;
pub(crate) const CX_PREVINTRA: usize = 7;
pub(crate) const CX_CHROMAMODE: usize = 8;
pub(crate) const CX_SPLITTF: usize = 9; // 3
pub(crate) const CX_CBFLUMA: usize = 12; // 2
pub(crate) const CX_CBFCHROMA: usize = 14; // 5
pub(crate) const CX_CUQPD: usize = 19; // 2
pub(crate) const CX_TSFLAG: usize = 23; // 2
pub(crate) const CX_LASTX: usize = 25; // 18
pub(crate) const CX_LASTY: usize = 43; // 18
pub(crate) const CX_CSBF: usize = 61; // 4
pub(crate) const CX_SIG: usize = 65; // 44
pub(crate) const CX_GT1: usize = 109; // 24
pub(crate) const CX_GT2: usize = 133; // 6
pub(crate) const CX_END: usize = 149;

/// rangeTabLps[64][4]
pub(crate) static LPS: [[u8; 4]; 64] = [
    [128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205],
    [116, 142, 169, 195], [111, 135, 160, 185], [105, 128, 152, 175], [100, 122, 144, 166],
    [95, 116, 137, 158], [90, 110, 130, 150], [85, 104, 123, 142], [81, 99, 117, 135],
    [77, 94, 111, 128], [73, 89, 105, 122], [69, 85, 100, 116], [66, 80, 95, 110],
    [62, 76, 90, 104], [59, 72, 86, 99], [56, 69, 81, 94], [53, 65, 77, 89],
    [51, 62, 73, 85], [48, 59, 69, 80], [46, 56, 66, 76], [43, 53, 63, 72],
    [41, 50, 59, 69], [39, 48, 56, 65], [37, 45, 54, 62], [35, 43, 51, 59],
    [33, 41, 48, 56], [32, 39, 46, 53], [30, 37, 43, 50], [29, 35, 41, 48],
    [27, 33, 39, 45], [26, 31, 37, 43], [24, 30, 35, 41], [23, 28, 33, 39],
    [22, 27, 32, 37], [21, 26, 30, 35], [20, 24, 29, 33], [19, 23, 27, 31],
    [18, 22, 26, 30], [17, 21, 25, 28], [16, 20, 23, 27], [15, 19, 22, 25],
    [14, 18, 21, 24], [14, 17, 20, 23], [13, 16, 19, 22], [12, 15, 18, 21],
    [12, 14, 17, 20], [11, 14, 16, 19], [11, 13, 15, 18], [10, 12, 15, 17],
    [10, 12, 14, 16], [9, 11, 13, 15], [9, 11, 12, 14], [8, 10, 12, 14],
    [8, 9, 11, 13], [7, 9, 11, 12], [7, 9, 10, 12], [7, 8, 10, 11],
    [6, 8, 9, 11], [6, 7, 9, 10], [6, 7, 8, 9], [2, 2, 2, 2],
];
/// transIdxLps
pub(crate) static TLPS: [u8; 64] = [
    0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, //
    13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24, //
    24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33, //
    33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];
/// transIdxMps
pub(crate) static TMPS: [u8; 64] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, //
    17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, //
    33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, //
    49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];
/// context init values, initType 0 (CX_* order)
pub(crate) static INIT: [u8; 152] = [
    153, 200, 139, 141, 157, 154, 184, 184, 63, 153, 138, 138, 111, 141, 94, 138, //
    182, 154, 154, 154, 154, 154, 154, 139, 139, 110, 110, 124, 125, 140, 153, 125, //
    127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63, 110, 110, 124, 125, 140, //
    153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63, 91, 171, 134, //
    141, 111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, //
    107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125, 140, 139, 182, 182, //
    152, 136, 152, 136, 153, 136, 139, 111, 136, 139, 111, 141, 111, 140, 92, 137, //
    138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152, 140, 179, 166, //
    182, 140, 227, 122, 197, 138, 153, 136, 167, 152, 152, 154, 154, 154, 154, 154, //
    154, 154, 154, 154, 154, 0, 0, 0,
];
/// ctxIdxMap of 4x4 TBs
pub(crate) static CTXMAP4: [u8; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 8];
/// intra_chroma_pred_mode 0..3
static CMODES: [u8; 4] = [0, 26, 10, 1];
/// Table 8-3 (corrected)
static MODE422: [u8; 36] = [
    0, 1, 2, 2, 2, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, //
    25, 25, 26, 27, 27, 28, 28, 29, 29, 30, 31, 0,
];
/// sub-block / position scans x | y << 4: SB1 (1x1, 4 bytes) | SCAN4 [diag, hor, ver][16] |
/// SB2 [diag, hor, ver][4] (2x2 sub-blocks) | SB8 [diag, hor, ver][64] (8x8 sub-blocks), laid
/// out as in the assembly (the table offsets below index this array)
pub(crate) static SCANS: [u8; 4 + 48 + 12 + 192] = [
    // SB1
    0x00, 0x00, 0x00, 0x00, //
    // SCAN4
    0x00, 0x10, 0x01, 0x20, 0x11, 0x02, 0x30, 0x21, 0x12, 0x03, 0x31, 0x22, 0x13, 0x32, 0x23, 0x33, //
    0x00, 0x01, 0x02, 0x03, 0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23, 0x30, 0x31, 0x32, 0x33, //
    0x00, 0x10, 0x20, 0x30, 0x01, 0x11, 0x21, 0x31, 0x02, 0x12, 0x22, 0x32, 0x03, 0x13, 0x23, 0x33, //
    // SB2
    0x00, 0x10, 0x01, 0x11, //
    0x00, 0x01, 0x10, 0x11, //
    0x00, 0x10, 0x01, 0x11, //
    // SB8
    0x00, 0x10, 0x01, 0x20, 0x11, 0x02, 0x30, 0x21, 0x12, 0x03, 0x40, 0x31, 0x22, 0x13, 0x04, 0x50, //
    0x41, 0x32, 0x23, 0x14, 0x05, 0x60, 0x51, 0x42, 0x33, 0x24, 0x15, 0x06, 0x70, 0x61, 0x52, 0x43, //
    0x34, 0x25, 0x16, 0x07, 0x71, 0x62, 0x53, 0x44, 0x35, 0x26, 0x17, 0x72, 0x63, 0x54, 0x45, 0x36, //
    0x27, 0x73, 0x64, 0x55, 0x46, 0x37, 0x74, 0x65, 0x56, 0x47, 0x75, 0x66, 0x57, 0x76, 0x67, 0x77, //
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, //
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, //
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, //
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, //
    0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x01, 0x11, 0x21, 0x31, 0x41, 0x51, 0x61, 0x71, //
    0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x03, 0x13, 0x23, 0x33, 0x43, 0x53, 0x63, 0x73, //
    0x04, 0x14, 0x24, 0x34, 0x44, 0x54, 0x64, 0x74, 0x05, 0x15, 0x25, 0x35, 0x45, 0x55, 0x65, 0x75, //
    0x06, 0x16, 0x26, 0x36, 0x46, 0x56, 0x66, 0x76, 0x07, 0x17, 0x27, 0x37, 0x47, 0x57, 0x67, 0x77,
];
pub(crate) const SB1: usize = 0;
pub(crate) const SCAN4: usize = 4;
pub(crate) const SB2: usize = 52;
pub(crate) const SB8: usize = 64;

/// the sub-block scan table of a TB with nsb x nsb sub-blocks and scanIdx scan (offset into
/// SCANS)
#[inline]
pub(crate) fn sbscan(nsb: u32, scan: u32) -> usize {
    match nsb {
        1 => SB1,
        2 => SB2 + scan as usize * 4,
        4 => SCAN4 + scan as usize * 16,
        _ => SB8 + scan as usize * 64,
    }
}

/// 9.3.2.2 initialisation of the context variables (initType 0) for qp (ctx_init /
/// he_ctxinit: the decoder clamps qp to 0..51 first)
pub(crate) fn ctx_init_into(ctx: &mut [u8; HV_NCTX], qp: i32) {
    for (i, c) in ctx.iter_mut().enumerate().take(CX_END) {
        let v = INIT[i] as i32;
        let m = (v >> 4) * 5 - 45;
        let n = ((v & 15) << 3) - 16;
        let pre = (n + (m.wrapping_mul(qp) >> 4)).max(1).min(126); // preCtxState
        *c = if pre <= 63 { ((63 - pre) << 1) as u8 } else { ((pre - 64) << 1 | 1) as u8 };
    }
}

/// fillrgn: set the 4x4 blocks of the square [x0, x0 + size) x [y0, y0 + size) (clipped
/// to the w x h picture) of a map (mw blocks per row) to v
#[inline]
pub(crate) fn fill<T: Copy>(map: &mut [T], mw: u32, w: u32, h: u32, v: T, x0: u32, y0: u32, size: u32) {
    let xe = (x0.wrapping_add(size).min(w) + 3) >> 2;
    let ye = (y0.wrapping_add(size).min(h) + 3) >> 2;
    let xs = x0 >> 2;
    for by in (y0 >> 2)..ye {
        let row = (by * mw) as usize;
        if xs < xe {
            map[row + xs as usize..row + xe as usize].fill(v);
        }
    }
}

impl Hevc<'_> {
    // ================================================================== CABAC engine
    /// cb_bits: the next n (1..32) bits at the CABAC position (0 past the end)
    #[inline(always)]
    pub(crate) fn cb_bits(&mut self, n: u32) -> u32 {
        let p = self.c.pos;
        let mut v = 0;
        if p >> 3 < self.blen as u64 {
            let i = (p >> 3) as usize;
            let b: [u8; 8] = self.buf[i..i + 8].try_into().unwrap_or([0; 8]);
            v = ((u64::from_be_bytes(b) << (p & 7)) >> (64 - n)) as u32;
        }
        self.c.pos = p.wrapping_add(n as u64);
        v
    }

    /// cb_start: InitializeArithmeticDecoder at the current position
    fn cb_start(&mut self) {
        self.c.range = 510;
        self.c.off = self.cb_bits(9);
    }

    /// cb_restart: after a terminating bin 1 / PCM: continue at the next byte boundary
    fn cb_restart(&mut self) {
        self.c.pos = (self.c.pos + 7) & !7;
        self.cb_start();
    }

    /// cb_renorm: RenormD (range < 256)
    #[inline(always)]
    fn renorm(&mut self, range: u32) {
        let sh = 8 - (31 - range.leading_zeros()); // so that range >= 256
        self.c.range = range << sh;
        let b = self.cb_bits(sh);
        self.c.off = self.c.off << sh | b;
    }

    /// cb_dec: DecodeDecision with context ctx
    #[inline]
    pub(crate) fn cb_dec(&mut self, ctx: usize) -> u32 {
        let ctx = if ctx < HV_NCTX { ctx } else { 0 };
        let s = self.c.ctx[ctx] as usize;
        let st = s >> 1; // pStateIdx
        let mps = (s & 1) as u32; // valMps
        let range = self.c.range;
        let lps = LPS[st & 63][((range >> 6) & 3) as usize] as u32;
        let r = range.wrapping_sub(lps);
        if self.c.off < r {
            self.c.ctx[ctx] = (TMPS[st & 63] << 1) | mps as u8;
            self.c.range = r;
            if r < 256 {
                self.renorm(r);
            }
            mps
        } else {
            self.c.off -= r;
            let bin = mps ^ 1;
            let nm = if st == 0 { bin } else { mps };
            self.c.ctx[ctx] = (TLPS[st & 63] << 1) | nm as u8;
            self.c.range = lps;
            self.renorm(lps);
            bin
        }
    }

    /// cb_byp: DecodeBypass
    #[inline]
    pub(crate) fn cb_byp(&mut self) -> u32 {
        let b = self.cb_bits(1);
        let off = self.c.off.wrapping_add(self.c.off) | b;
        if off >= self.c.range {
            self.c.off = off - self.c.range;
            1
        } else {
            self.c.off = off;
            0
        }
    }

    /// cb_bypn: n bypass bins MSB first
    pub(crate) fn cb_bypn(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = v.wrapping_add(v) | self.cb_byp();
        }
        v
    }

    /// cb_term: DecodeTerminate
    fn cb_term(&mut self) -> u32 {
        let r = self.c.range.wrapping_sub(2);
        self.c.range = r;
        if self.c.off >= r {
            return 1;
        }
        if r < 256 {
            self.renorm(r);
        }
        0
    }

    /// ctx_init: all context variables for SliceQpY qp
    fn ctx_init(&mut self, qp: i32) {
        ctx_init_into(&mut self.c.ctx, qp.max(0).min(51));
    }

    // ================================================================== maps
    /// mapidx: index of the 4x4 block of (x, y)
    #[inline(always)]
    pub(crate) fn mapidx(&self, x: u32, y: u32) -> usize {
        ((y >> 2).wrapping_mul(self.mw).wrapping_add(x >> 2)) as usize
    }

    /// avail: z-scan availability (6.4.1): inside the picture, decoded, same slice, same tile
    #[inline]
    pub(crate) fn avail(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.s.w || y as u32 >= self.s.h {
            return false;
        }
        let i = self.mapidx(x as u32, y as u32);
        self.mdec[i] == self.u.sliceaddr && self.mtid[i] as u32 == self.u.tile
    }

    /// mark_dec: the 4x4 blocks of the square are decoded (slice address, tile id)
    fn mark_dec(&mut self, x0: u32, y0: u32, size: u32) {
        let (mw, w, h) = (self.mw, self.s.w, self.s.h);
        let (sa, t) = (self.u.sliceaddr, self.u.tile as u16);
        fill(&mut self.mdec, mw, w, h, sa, x0, y0, size);
        fill(&mut self.mtid, mw, w, h, t, x0, y0, size);
    }

    /// edges: TU / CU / PU edge flags: left column (bit 0), top row (bit 1)
    fn edges(&mut self, x0: u32, y0: u32, size: u32) {
        let ye = y0.wrapping_add(size).min(self.s.h);
        let mut y = y0;
        while y < ye {
            let i = self.mapidx(x0, y);
            self.medge[i] |= 1;
            y += 4;
        }
        let xe = x0.wrapping_add(size).min(self.s.w);
        let mut x = x0;
        while x < xe {
            let i = self.mapidx(x, y0);
            self.medge[i] |= 2;
            x += 4;
        }
    }

    // ================================================================== slice data
    /// hvc_slice_data -> 0 / ERR_* (see the file header)
    pub(crate) fn slice_data(&mut self) -> i32 {
        self.u.sliceaddr = self.l.saddr as i32;
        self.c.pos = (self.l.data as u64) << 3;
        self.cb_start();
        let mut ts = self.rs2ts[self.l.addr as usize];
        if self.l.dep != 0 && self.c.dvalid != 0 {
            self.c.ctx = self.c.dctx;
            self.u.qpprev = self.c.dqp;
        } else {
            let q = self.l.qp;
            self.ctx_init(q);
            self.u.qpprev = q;
        }
        let mut first = true;
        loop {
            if ts >= self.nctb {
                return ERR_HDR;
            }
            let rs = self.ts2rs[ts as usize];
            let pw = self.s.pwctb;
            let rx = rs % pw;
            let ry = rs / pw;
            self.u.rx = rx;
            self.u.ry = ry;
            self.u.ts = ts;
            self.u.tile = self.tid[ts as usize] as u32;
            // first CTB of a CTB row in the tile?
            let col0 = self.p.colbd[self.tilecol(rx)];
            if !first {
                if self.p.tiles != 0 && self.tid[ts as usize] != self.tid[ts as usize - 1] {
                    // tile change: fresh contexts
                    let q = self.l.qp;
                    self.ctx_init(q);
                    self.u.qpprev = q;
                } else if self.p.entsync != 0 && rx == col0 {
                    self.wpp_sync(false);
                }
            } else if self.p.entsync != 0 && rx == col0 {
                self.wpp_sync(true);
            }
            if self.p.entsync != 0 && rx == col0 {
                self.u.qpprev = self.l.qp;
            }
            self.cslice[rs as usize] = self.u.sliceaddr;
            self.cdb[rs as usize] = (self.l.lfx.wrapping_add(self.l.lfx) | self.l.dbdis) as u8;
            self.cbeta[rs as usize] = [self.l.beta as i8, self.l.tc as i8];
            // coding_tree_unit
            let r = self.ctu(rx, ry, rs);
            if r != 0 {
                return r;
            }
            first = false;
            self.nctu += 1;
            // WPP storage after the 2nd CTB of a row in the tile
            if self.p.entsync != 0 && rx == col0.wrapping_add(1) {
                self.c.wctx = self.c.ctx;
                self.c.wvalid = 1;
                self.c.wrow = ry;
                self.c.wtc = self.tilecol(rx) as u32;
            }
            let end = self.cb_term(); // end_of_slice_segment_flag
            ts += 1;
            if end != 0 {
                self.c.dctx = self.c.ctx;
                self.c.dvalid = 1;
                self.c.dqp = self.u.qpprev;
                return self.err;
            }
            // test hook: stop after maxctu CTUs
            if self.maxctu != 0 && self.nctu >= self.maxctu {
                return self.err;
            }
            if self.cancelled() {
                return ERR_CANCEL;
            }
            if ts >= self.nctb {
                return ERR_HDR;
            }
            // end of a tile or a WPP row: end_of_subset_one_bit, byte alignment, new decoder
            let mut sub = self.p.tiles != 0 && self.tid[ts as usize] != self.tid[ts as usize - 1];
            if self.p.entsync != 0 {
                let nrx = self.ts2rs[ts as usize] % pw;
                if nrx == self.p.colbd[self.tilecol(nrx)] {
                    sub = true;
                }
            }
            if sub {
                if self.cb_term() == 0 {
                    return ERR_HDR;
                }
                self.cb_restart();
            }
        }
    }

    /// tilecol: the tile column of CTB column x
    fn tilecol(&self, x: u32) -> usize {
        let mut t = 0usize;
        while t + 1 < self.p.ncols as usize && x >= self.p.colbd[t + 1] {
            t += 1;
        }
        t
    }

    /// wpp_sync: 9.3.1 synchronisation at the start of a CTB row of a tile: contexts from
    /// the storage when the CTB above-right (same tile, same slice, decoded) exists, else
    /// initialised (not at a slice start: the contexts are fresh already)
    fn wpp_sync(&mut self, first: bool) {
        let (rx, ry) = (self.u.rx, self.u.ry);
        let mut ok = false;
        if ry != 0 && self.c.wvalid != 0 && self.c.wrow == ry - 1 {
            let tc = self.tilecol(rx);
            if self.c.wtc == tc as u32 && rx + 1 < self.p.colbd[tc + 1] && rx + 1 < self.s.pwctb {
                let l = self.s.log2ctb;
                ok = self.avail(((rx + 1) << l) as i32, ((ry - 1) << l) as i32);
            }
        }
        if ok {
            self.c.ctx = self.c.wctx;
        } else if !first {
            let q = self.l.qp;
            self.ctx_init(q);
        }
    }

    // ================================================================== coding tree unit
    /// ctu: SAO syntax and the coding quadtree of one CTB -> 0 / ERR_*
    fn ctu(&mut self, rx: u32, ry: u32, rs: u32) -> i32 {
        if self.l.saoy | self.l.saoc != 0 {
            self.sao_syntax(rx, ry, rs);
        } else {
            let r = &mut self.csao[rs as usize];
            r[0].ty = 0;
            r[1].ty = 0;
            r[2].ty = 0;
        }
        let l = self.s.log2ctb;
        let r = self.cqt(rx << l, ry << l, l, 0);
        if r != 0 {
            return r;
        }
        self.err
    }

    /// sao_syntax: sao( ) into csao[rs] (merges copy the left / upper record)
    fn sao_syntax(&mut self, rx: u32, ry: u32, rs: u32) {
        let rs = rs as usize;
        if rx != 0 && self.ctbsame(rs as u32 - 1) && self.cb_dec(CX_SAOMERGE) != 0 {
            self.csao[rs] = self.csao[rs - 1];
            return;
        }
        let pw = self.s.pwctb as usize;
        if ry != 0 && self.ctbsame((rs - pw) as u32) && self.cb_dec(CX_SAOMERGE) != 0 {
            self.csao[rs] = self.csao[rs - pw];
            return;
        }
        let mut ty = 0u32; // SaoTypeIdx (shared by Cb / Cr)
        let mut eo = 0u32; // EO class (shared)
        for c in 0..self.u.ncomp as usize {
            self.csao[rs][c].ty = 0;
            let on = if c == 0 { self.l.saoy } else { self.l.saoc };
            if on == 0 {
                continue;
            }
            if c != 2 {
                let b = self.cb_dec(CX_SAOTYPE);
                ty = 0;
                if b != 0 {
                    ty = self.cb_byp() + 1; // 1 band, 2 edge
                }
            }
            if ty == 0 {
                continue;
            }
            let mut rec = self.csao[rs][c];
            rec.ty = ty as u8;
            // offsets: TR cMax = (1 << (Min(bitDepth, 10) - 5)) - 1
            let bd = if c == 0 { self.s.bdy } else { self.s.bdc }.min(10);
            let cmax = (1u32 << (bd.wrapping_sub(5) & 31)).wrapping_sub(1);
            for i in 0..4 {
                let mut v = 0u32;
                while v < cmax && self.cb_byp() != 0 {
                    v += 1;
                }
                rec.off[i] = v as i16;
            }
            let scale = if c == 0 { self.p.saoscy } else { self.p.saoscc }; // log2 offset scale
            if ty == 1 {
                // band: signs of the nonzero offsets, band position
                for i in 0..4 {
                    if rec.off[i] != 0 && self.cb_byp() != 0 {
                        rec.off[i] = rec.off[i].wrapping_neg();
                    }
                }
                rec.pos = self.cb_bypn(5) as u8;
            } else {
                if c != 2 {
                    eo = self.cb_bypn(2);
                }
                rec.pos = eo as u8;
                rec.off[2] = rec.off[2].wrapping_neg(); // offsets 3, 4 negative
                rec.off[3] = rec.off[3].wrapping_neg();
            }
            for i in 0..4 {
                rec.off[i] = (rec.off[i] as i32).wrapping_shl(scale) as i16;
            }
            self.csao[rs][c] = rec;
        }
    }

    /// ctbsame: CTB rs is decoded, in the current slice and tile
    fn ctbsame(&self, rs: u32) -> bool {
        if rs >= self.nctb {
            return false;
        }
        if self.cslice[rs as usize] != self.u.sliceaddr {
            return false;
        }
        if self.tid[self.rs2ts[rs as usize] as usize] as u32 != self.u.tile {
            return false;
        }
        // decoded: its first 4x4 block
        let pw = self.s.pwctb;
        let l = self.s.log2ctb;
        self.avail(((rs % pw) << l) as i32, ((rs / pw) << l) as i32)
    }

    // ================================================================== coding quadtree
    /// cqt: coding_quadtree( ) (recursive, depth <= 3) -> 0 / ERR_*
    fn cqt(&mut self, x0: u32, y0: u32, log2: u32, depth: u32) -> i32 {
        let size = 1u32 << log2;
        let split;
        if x0 + size <= self.s.w && y0 + size <= self.s.h && log2 > self.s.log2mincb {
            let mut inc = 0usize; // ctxInc
            if self.avail(x0 as i32 - 1, y0 as i32) && self.mdepth[self.mapidx(x0 - 1, y0)] as u32 > depth {
                inc += 1;
            }
            if self.avail(x0 as i32, y0 as i32 - 1) && self.mdepth[self.mapidx(x0, y0 - 1)] as u32 > depth {
                inc += 1;
            }
            split = self.cb_dec(CX_SPLITCU + inc);
        } else {
            split = (log2 > self.s.log2mincb) as u32;
        }
        // quantization group start
        if self.p.cuqpd != 0 && log2 >= self.s.log2ctb.wrapping_sub(self.p.cuqpddepth) {
            self.u.qpdc = 0;
            self.u.qpdval = 0;
            self.u.qgx = x0;
            self.u.qgy = y0;
            self.u.qgvalid = 0;
        }
        if split != 0 {
            let half = size >> 1;
            for ch in 0..4u32 {
                let x = x0 + if ch & 1 != 0 { half } else { 0 };
                let y = y0 + if ch & 2 != 0 { half } else { 0 };
                if x < self.s.w && y < self.s.h {
                    let r = self.cqt(x, y, log2 - 1, depth + 1);
                    if r != 0 {
                        return r;
                    }
                }
            }
            return 0;
        }
        self.cu(x0, y0, log2, depth)
    }

    // ================================================================== coding unit
    /// qppred: qPY_PRED of the quantization group (computed at its first CU)
    fn qppred(&mut self) -> i32 {
        if self.u.qgvalid == 0 {
            let (gx, gy, prev) = (self.u.qgx, self.u.qgy, self.u.qpprev);
            let a = self.qpnb(gx as i32 - 1, gy as i32, gx, gy, prev);
            let b = self.qpnb(gx as i32, gy as i32 - 1, gx, gy, prev);
            self.u.qgpred = a.wrapping_add(b).wrapping_add(1) >> 1;
            self.u.qgvalid = 1;
        }
        self.u.qgpred
    }

    /// qpnb: QpY of the neighbour, or qPY_PREV when it is unavailable or in another CTB
    /// than the quantization group (gx, gy)
    fn qpnb(&self, xn: i32, yn: i32, gx: u32, gy: u32, prev: i32) -> i32 {
        if !self.avail(xn, yn) {
            return prev;
        }
        let l = self.s.log2ctb;
        if (xn as u32) >> l != gx >> l || (yn as u32) >> l != gy >> l {
            return prev;
        }
        self.mqp[self.mapidx(xn as u32, yn as u32)] as i32
    }

    /// qpcalc: QpY from qPY_PRED and CuQpDeltaVal (8.6.1)
    pub(crate) fn qpcalc(&self, pred: i32, delta: i32) -> i32 {
        let off = (self.s.bdy as i32 - 8).wrapping_mul(6); // QpBdOffsetY
        let d = off + 52;
        let n = pred.wrapping_add(delta).wrapping_add(52).wrapping_add(off.wrapping_mul(2));
        let mut r = n.wrapping_rem(d);
        if r < 0 {
            r += d;
        }
        r - off
    }

    /// cu: coding_unit( ) of an I slice -> 0 / ERR_*
    fn cu(&mut self, x0: u32, y0: u32, log2: u32, depth: u32) -> i32 {
        self.u.cux = x0;
        self.u.cuy = y0;
        self.u.culog2 = log2;
        self.u.byp = 0;
        if self.p.tqbypass != 0 {
            self.u.byp = self.cb_dec(CX_TQBYPASS);
        }
        self.u.nxn = 0;
        if log2 == self.s.log2mincb {
            self.u.nxn = self.cb_dec(CX_PARTMODE) ^ 1;
        }
        self.u.pcm = 0;
        if self.u.nxn == 0 && self.s.pcm != 0 && log2 >= self.s.log2minpcm && log2 <= self.s.log2maxpcm {
            self.u.pcm = self.cb_term();
        }
        let pred = if self.p.cuqpd != 0 { self.qppred() } else { self.l.qp };
        self.u.qppred = pred;
        let delta = if self.p.cuqpd != 0 { self.u.qpdval } else { 0 };
        self.u.qpy = self.qpcalc(pred, delta);
        let size = 1u32 << log2;
        if self.u.pcm != 0 {
            // ---- PCM
            let fl = MB_PCM | if self.s.pcmlfdis != 0 { MB_PCMOFF } else { 0 };
            self.cuinfo(x0, y0, size, depth, fl);
            self.u.modes[0] = 1;
            self.pcm(x0, y0, log2);
        } else {
            // ---- prev_intra_luma_pred_flag for every PU, then mpm_idx / rem_intra_luma_pred_mode
            let parts = if self.u.nxn != 0 { 4 } else { 1 };
            let mut flags = [0u8; 4];
            for f in flags.iter_mut().take(parts) {
                *f = self.cb_dec(CX_PREVINTRA) as u8;
            }
            let pb = if self.u.nxn != 0 { size >> 1 } else { size };
            for part in 0..parts {
                let xp = x0 + if part & 1 != 0 { pb } else { 0 };
                let yp = y0 + if part & 2 != 0 { pb } else { 0 };
                let mode = if flags[part] != 0 {
                    let mut idx = 0usize; // mpm_idx (TR, cMax 2, bypass)
                    if self.cb_byp() != 0 {
                        idx += 1;
                        if self.cb_byp() != 0 {
                            idx += 1;
                        }
                    }
                    self.mpm(xp, yp)[idx]
                } else {
                    let rem = self.cb_bypn(5);
                    let c = self.mpm(xp, yp);
                    // sort the 3 candidates ascending
                    let (mut a, mut b, mut d) = (c[0], c[1], c[2]);
                    if a > b {
                        core::mem::swap(&mut a, &mut b);
                    }
                    if a > d {
                        core::mem::swap(&mut a, &mut d);
                    }
                    if b > d {
                        core::mem::swap(&mut b, &mut d);
                    }
                    let mut m = rem;
                    if m >= a {
                        m += 1;
                    }
                    if m >= b {
                        m += 1;
                    }
                    if m >= d {
                        m += 1;
                    }
                    m
                };
                self.u.modes[part & 3] = mode as u8;
                // IntraPredModeY of the PU's 4x4 blocks
                let (mw, w, h) = (self.mw, self.s.w, self.s.h);
                fill(&mut self.mipm, mw, w, h, mode as u8, xp, yp, pb);
            }
            if self.s.cat != 0 {
                let n = if self.s.cat == 3 && self.u.nxn != 0 { 4 } else { 1 };
                for i in 0..n {
                    let b = self.cb_dec(CX_CHROMAMODE);
                    let v = if b != 0 { self.cb_bypn(2) } else { 4 };
                    let lm = self.u.modes[i & 3] as u32; // luma mode of the PU
                    let mut m = lm;
                    if v != 4 {
                        m = CMODES[v as usize] as u32;
                        if m == lm {
                            m = 34;
                        }
                    }
                    if self.s.cat == 2 {
                        m = MODE422[m as usize] as u32;
                    }
                    self.u.cmodes[i & 3] = m as u8;
                }
            }
            let byp = self.u.byp as u8; // MB_BYPASS = 1
            self.cuinfo(x0, y0, size, depth, byp);
            // transform tree: MaxTrafoDepth = max_transform_hierarchy_depth_intra + IntraSplitFlag
            self.u.sdepth = self.s.maxthintra + self.u.nxn;
            let r = self.ttree(x0, y0, x0, y0, log2, 0, 0, 0x0F);
            if r != 0 {
                return r;
            }
        }
        // QpY of the CU's 4x4 blocks, qPY_PREV
        let (mw, w, h) = (self.mw, self.s.w, self.s.h);
        let q = self.u.qpy;
        fill(&mut self.mqp, mw, w, h, q as i8, x0, y0, size);
        self.u.qpprev = q;
        self.err
    }

    /// cuinfo: CtDepth, filter flags (MB_*) and the CU's left / top edges
    fn cuinfo(&mut self, x0: u32, y0: u32, size: u32, depth: u32, flags: u8) {
        let (mw, w, h) = (self.mw, self.s.w, self.s.h);
        fill(&mut self.mdepth, mw, w, h, depth as u8, x0, y0, size);
        fill(&mut self.mbyp, mw, w, h, flags, x0, y0, size);
        self.edges(x0, y0, size);
    }

    /// mpm: candModeList (8.4.2); PUs of the current CU before this one count as available
    fn mpm(&self, xp: u32, yp: u32) -> [u32; 3] {
        let a = self.mpmcand(xp as i32 - 1, yp as i32, false);
        let b = self.mpmcand(xp as i32, yp as i32 - 1, true);
        if a == b {
            if a < 2 {
                [0, 1, 26]
            } else {
                [a, (a + 29) % 32 + 2, (a - 1) % 32 + 2]
            }
        } else {
            // planar unless A or B is planar, then DC unless A or B is DC, then 26
            let c = if a != 0 && b != 0 {
                0
            } else if a != 1 && b != 1 {
                1
            } else {
                26
            };
            [a, b, c]
        }
    }

    /// mpmcand: candIntraPredModeX of (xn, yn) (isb: B, the neighbour above)
    fn mpmcand(&self, xn: i32, yn: i32, isb: bool) -> u32 {
        let cx = self.u.cux as i32;
        let cy = self.u.cuy as i32;
        let n = 1i32 << self.u.culog2;
        let inside = xn >= cx && xn < cx + n && yn >= cy && yn < cy + n;
        if !inside {
            if !self.avail(xn, yn) {
                return 1;
            }
            if self.mbyp[self.mapidx(xn as u32, yn as u32)] & MB_PCM != 0 {
                return 1;
            }
            if isb {
                // B outside the current CTB row -> DC
                let l = self.s.log2ctb;
                let top = (self.u.cuy >> l << l) as i32;
                if yn < top {
                    return 1;
                }
            }
        }
        self.mipm[self.mapidx(xn as u32, yn as u32)] as u32
    }

    /// pcm: pcm_sample( ): raw samples from the next byte boundary, then a new arithmetic
    /// decoder
    fn pcm(&mut self, x0: u32, y0: u32, log2: u32) {
        self.c.pos = (self.c.pos + 7) & !7;
        let size = 1u32 << log2;
        // luma
        let (bd, pbd) = (self.s.bdy, self.s.pcmbdy);
        for i in 0..size * size {
            let v = self.cb_bits(pbd) << ((bd - pbd) & 31);
            self.putsample(0, x0 + i % size, y0 + i / size, v);
        }
        if self.s.cat != 0 {
            let cw = size / self.s.subw;
            let chh = size / self.s.subh;
            let (bd, pbd) = (self.s.bdc, self.s.pcmbdc);
            for c in 1..3 {
                for i in 0..cw * chh {
                    let v = self.cb_bits(pbd) << ((bd - pbd) & 31);
                    let x = x0 / self.s.subw + i % cw;
                    let y = y0 / self.s.subh + i / cw;
                    self.putsample(c, x, y, v);
                }
            }
        }
        self.cb_restart();
        self.mark_dec(x0, y0, size);
    }

    /// putsample: store into the plane (clipped to its size)
    fn putsample(&mut self, c: usize, x: u32, y: u32, v: u32) {
        if x < self.pw[c] && y < self.ph[c] {
            let i = (y * self.pw[c] + x) as usize;
            self.pl[c][i] = v as u16;
        }
    }

    // ================================================================== transform tree
    /// ttree: transform_tree( ) -> 0 / ERR_*; pcbf: cbf_cb[0] 1, cb[1] 2, cr[0] 4, cr[1] 8 of
    /// the parent node (the 4x4 luma TBs of 4:2:0 / 4:2:2 inherit them; depth 0 passes 0x0F)
    #[allow(clippy::too_many_arguments)]
    fn ttree(&mut self, x0: u32, y0: u32, xb: u32, yb: u32, log2: u32, depth: u32, blk: u32, pcbf: u32) -> i32 {
        let nxn = self.u.nxn != 0;
        let split = if log2 <= self.s.log2maxtb && log2 > self.s.log2mintb && depth < self.u.sdepth && !(depth == 0 && nxn) {
            self.cb_dec(CX_SPLITTF + 5 - log2 as usize)
        } else {
            (log2 > self.s.log2maxtb || (depth == 0 && nxn)) as u32
        };
        // cbf_cb / cbf_cr
        let mut cbf = 0u32;
        let cat = self.s.cat;
        if cat != 0 {
            if cat == 3 || log2 > 2 {
                for comp in 0..2u32 {
                    let par = (pcbf >> (comp * 2)) & 3; // parent cb / cr bits
                    if depth == 0 || par != 0 {
                        let b = self.cb_dec(CX_CBFCHROMA + depth as usize);
                        cbf |= b << (comp * 2);
                        if cat == 2 && (split == 0 || log2 == 3) {
                            let b = self.cb_dec(CX_CBFCHROMA + depth as usize);
                            cbf |= b << (comp * 2 + 1);
                        }
                    }
                }
            } else if depth != 0 {
                cbf = pcbf; // 4x4 luma of 4:2:0 / 4:2:2: inherited
            }
        }
        if split != 0 {
            // children: parent bits = this node's
            let half = (1u32 << log2) >> 1;
            for ch in 0..4u32 {
                let x = x0 + if ch & 1 != 0 { half } else { 0 };
                let y = y0 + if ch & 2 != 0 { half } else { 0 };
                let r = self.ttree(x, y, x0, y0, log2 - 1, depth + 1, ch, cbf);
                if r != 0 {
                    return r;
                }
            }
            return 0;
        }
        let cl = self.cb_dec(CX_CBFLUMA + (depth == 0) as usize);
        self.tu(x0, y0, xb, yb, log2, blk, cl << 4 | cbf)
    }

    /// tu: transform_unit( ): cu_qp_delta, luma prediction + residual, chroma prediction +
    /// residuals -> 0 / ERR_*; cbf: bits 0..3 chroma (as ttree), bit 4 luma
    #[allow(clippy::too_many_arguments)]
    fn tu(&mut self, x0: u32, y0: u32, xb: u32, yb: u32, log2: u32, blk: u32, cbf: u32) -> i32 {
        let size = 1u32 << log2;
        self.edges(x0, y0, size);
        if cbf & 0x1F != 0 {
            if self.p.cuqpd != 0 && self.u.qpdc == 0 {
                let mut v = 0u32; // prefix (TR cMax 5)
                while v < 5 {
                    let ctx = CX_CUQPD + (v != 0) as usize;
                    if self.cb_dec(ctx) == 0 {
                        break;
                    }
                    v += 1;
                }
                if v == 5 {
                    // EG0 suffix
                    let mut k = 0u32;
                    while self.cb_byp() != 0 {
                        k += 1;
                        if k >= 16 {
                            self.err = ERR_HDR;
                            return ERR_HDR;
                        }
                    }
                    v = v.wrapping_add((1 << k) - 1);
                    v = v.wrapping_add(self.cb_bypn(k));
                }
                let mut d = v as i32;
                if v != 0 && self.cb_byp() != 0 {
                    d = d.wrapping_neg();
                }
                self.u.qpdc = 1;
                self.u.qpdval = d;
                self.u.qpy = self.qpcalc(self.u.qppred, d);
            }
            if self.l.cqpo != 0 {
                self.err = ERR_UNSUPPORTED;
                return ERR_UNSUPPORTED;
            }
        }
        // luma: prediction, residual
        let mode = self.mipm[self.mapidx(x0, y0)] as u32; // IntraPredModeY
        self.intra(0, x0, y0, log2, mode);
        if cbf & 16 != 0 {
            let r = self.resid(0, x0, y0, log2, mode);
            if r != 0 {
                return r;
            }
        }
        self.mark_dec(x0, y0, size);
        let cat = self.s.cat;
        if cat == 0 {
            return self.err;
        }
        // chroma
        let (lx, ly, cl) = if cat == 3 {
            (x0, y0, log2)
        } else if log2 > 2 {
            (x0, y0, log2 - 1)
        } else {
            if blk != 3 {
                return self.err;
            }
            (xb, yb, 2) // 4x4 luma TBs: chroma at the parent, log2 2
        };
        let cmode = self.cmodeat(lx, ly);
        let xc = lx / self.s.subw;
        let yc = ly / self.s.subh;
        let nt = if cat == 2 { 2 } else { 1 };
        for comp in 1..3u32 {
            for t in 0..nt {
                let y = yc + (t << cl);
                self.intra(comp as usize, xc, y, cl, cmode);
                // cbf bit: (component - 1) * 2 + t
                if cbf >> ((comp - 1) * 2 + t) & 1 != 0 {
                    let r = self.resid(comp, xc, y, cl, cmode);
                    if r != 0 {
                        return r;
                    }
                }
            }
        }
        self.err
    }

    /// cmodeat: IntraPredModeC of the PU containing the luma point
    fn cmodeat(&self, x: u32, y: u32) -> u32 {
        if self.s.cat != 3 || self.u.nxn == 0 {
            return self.u.cmodes[0] as u32;
        }
        let half = 1u32 << (self.u.culog2.wrapping_sub(1) & 31);
        let mut i = 0usize;
        if x >= self.u.cux + half {
            i |= 1;
        }
        if y >= self.u.cuy + half {
            i |= 2;
        }
        self.u.cmodes[i] as u32
    }

    // ================================================================== residual coding
    /// resid: residual_coding( ) into coef (raster, i32), then recon_tb -> 0 / ERR_HDR
    fn resid(&mut self, cidx: u32, x0: u32, y0: u32, log2: u32, mode: u32) -> i32 {
        self.rw.cidx = cidx;
        self.rw.log2 = log2;
        self.rw.mode = mode;
        let n = 1u32 << log2;
        self.rw.n = n;
        self.coef[..(n * n) as usize].fill(0);
        // transform_skip_flag
        self.rw.ts = 0;
        if self.p.tskip != 0 && self.u.byp == 0 && log2 <= self.p.log2maxts {
            self.rw.ts = self.cb_dec(CX_TSFLAG + (cidx != 0) as usize);
        }
        // last_sig_coeff prefixes
        let (coff, cshift) = if cidx == 0 { ((log2 - 2) * 3 + ((log2 - 1) >> 2), (log2 + 1) >> 2) } else { (15, log2 - 2) };
        let cmax = log2 * 2 - 1;
        let mut px = 0u32;
        while px < cmax {
            if self.cb_dec(CX_LASTX + ((px >> cshift) + coff) as usize) == 0 {
                break;
            }
            px += 1;
        }
        let mut py = 0u32;
        while py < cmax {
            if self.cb_dec(CX_LASTY + ((py >> cshift) + coff) as usize) == 0 {
                break;
            }
            py += 1;
        }
        self.rw.lastx = self.lastsuf(px);
        self.rw.lasty = self.lastsuf(py);
        // scanIdx
        let mut scan = 0u32;
        if log2 == 2 || (log2 == 3 && (cidx == 0 || self.s.cat == 3)) {
            if (6..=14).contains(&mode) {
                scan = 2;
            } else if (22..=30).contains(&mode) {
                scan = 1;
            }
        }
        self.rw.scan = scan;
        if scan == 2 {
            core::mem::swap(&mut self.rw.lastx, &mut self.rw.lasty);
        }
        if self.rw.lastx >= n || self.rw.lasty >= n {
            self.err = ERR_HDR;
            return ERR_HDR;
        }
        // sub-block scan: nsb = n / 4
        let nsb = n >> 2;
        self.rw.nsb = nsb;
        let sbt = sbscan(nsb, scan); // sub-block scan table (x | y << 4)
        let pst = SCAN4 + scan as usize * 16; // position scan table
        self.rw.ptab = pst as u32;
        // last sub-block / position
        let key = ((self.rw.lastx >> 2) | (self.rw.lasty >> 2) << 4) as u8;
        let Some(lastsb) = SCANS[sbt..sbt + (nsb * nsb) as usize].iter().position(|&v| v == key) else {
            self.err = ERR_HDR;
            return ERR_HDR;
        };
        let key = ((self.rw.lastx & 3) | (self.rw.lasty & 3) << 4) as u8;
        let Some(lastpos) = SCANS[pst..pst + 16].iter().position(|&v| v == key) else {
            self.err = ERR_HDR;
            return ERR_HDR;
        };
        // coded_sub_block_flag array
        self.rw.csbf = [0; 64];
        self.rw.c1 = -1;
        let mut i = lastsb as i32;
        while i >= 0 {
            let v = SCANS[sbt + i as usize] as u32;
            let xs = v & 15;
            let ys = v >> 4;
            self.rw.xs = xs;
            self.rw.ys = ys;
            // csbf right / below
            let prev = self.csbfnb();
            self.rw.prev = prev;
            let mut infer = false; // inferSbDcSigCoeffFlag
            let ci = (ys * 8 + xs) as usize; // csbf index
            if (i as usize) < lastsb && i != 0 {
                let c = ((prev & 1) + (prev >> 1)).min(1) as usize;
                let ctx = CX_CSBF + c + if cidx != 0 { 2 } else { 0 };
                self.rw.csbf[ci & 63] = self.cb_dec(ctx) as u8;
                infer = true;
            } else {
                self.rw.csbf[ci & 63] = 1;
            }
            // significant coefficients of the sub-block (scan positions, decoding order)
            let mut nsig = 0u32;
            let mut k = 15i32;
            if i as usize == lastsb {
                self.rw.sig[0] = lastpos as u8;
                nsig = 1;
                k = lastpos as i32 - 1;
            }
            if self.rw.csbf[ci & 63] != 0 {
                while k >= 0 {
                    if k == 0 && infer {
                        self.rw.sig[(nsig & 15) as usize] = 0; // DC inferred
                        nsig += 1;
                    } else {
                        let c = self.sigctx(k as u32);
                        if self.cb_dec(CX_SIG + c as usize) != 0 {
                            self.rw.sig[(nsig & 15) as usize] = k as u8;
                            nsig += 1;
                            infer = false;
                        }
                    }
                    k -= 1;
                }
            }
            if nsig != 0 {
                self.rw.nsig = nsig;
                let r = self.levels(i as u32);
                if r != 0 {
                    return r;
                }
            }
            i -= 1;
        }
        // reconstruction
        let ts = self.rw.ts;
        self.recon_tb(cidx as usize, x0, y0, log2, ts);
        0
    }

    /// lastsuf: LastSignificantCoeff (suffix bins when prefix > 3)
    fn lastsuf(&mut self, p: u32) -> u32 {
        if p <= 3 {
            return p;
        }
        let bits = (p >> 1) - 1; // suffix bits
        let v = self.cb_bypn(bits);
        v.wrapping_add(((p & 1) + 2) << bits)
    }

    /// csbfnb: coded_sub_block_flag right | below << 1 of the sub-block (xs, ys)
    #[inline]
    fn csbfnb(&self) -> u32 {
        let (xs, ys, nsb) = (self.rw.xs, self.rw.ys, self.rw.nsb);
        let mut v = 0u32;
        if xs + 1 < nsb {
            v = self.rw.csbf[((ys << 3) + xs + 1) as usize & 63] as u32;
        }
        if ys + 1 < nsb {
            v += (self.rw.csbf[(((ys + 1) << 3) + xs) as usize & 63] as u32) * 2;
        }
        v
    }

    /// sigctx: ctxIdxInc of sig_coeff_flag (9.3.4.2.5) at scan position k of the sub-block
    #[inline]
    fn sigctx(&self, k: u32) -> u32 {
        let v = SCANS[self.rw.ptab as usize + (k & 15) as usize] as u32; // xP | yP << 4
        let xp = v & 15;
        let yp = v >> 4;
        let xc = (self.rw.xs << 2) + xp;
        let yc = (self.rw.ys << 2) + yp;
        let log2 = self.rw.log2;
        let mut c;
        if log2 == 2 {
            c = CTXMAP4[((yc << 2) + xc) as usize & 15] as u32;
        } else if xc + yc == 0 {
            c = 0;
        } else {
            c = match self.rw.prev {
                0 => {
                    let s = xp + yp;
                    if s == 0 {
                        2
                    } else if s < 3 {
                        1
                    } else {
                        0
                    }
                }
                1 => match yp {
                    0 => 2,
                    1 => 1,
                    _ => 0,
                },
                2 => match xp {
                    0 => 2,
                    1 => 1,
                    _ => 0,
                },
                _ => 2,
            };
            if self.rw.cidx == 0 {
                if self.rw.xs | self.rw.ys != 0 {
                    c += 3;
                }
                if log2 == 3 {
                    c += 9;
                    if self.rw.scan != 0 {
                        c += 6;
                    }
                } else {
                    c += 21;
                }
            } else {
                c += 9;
                if log2 != 3 {
                    c += 3;
                }
            }
        }
        if self.rw.cidx != 0 {
            c += 27;
        }
        c
    }

    /// levels: greater1 / greater2 flags, signs, remaining levels of the sub-block i into
    /// coef -> 0 / ERR_HDR
    fn levels(&mut self, i: u32) -> i32 {
        let cidx = self.rw.cidx;
        let nsig = self.rw.nsig;
        // ctxSet
        let mut set = if i != 0 && cidx == 0 { 2usize } else { 0 };
        if self.rw.c1 == 0 {
            set += 1;
        }
        self.rw.abs = [1; 16];
        let mut g1 = 1usize; // greater1Ctx
        let mut first: i32 = -1; // first greater1 index
        let mut j = 0u32;
        while j < nsig && j < 8 {
            let ctx = CX_GT1 + set * 4 + g1 + if cidx != 0 { 16 } else { 0 };
            let b = self.cb_dec(ctx);
            self.rw.abs[j as usize] += b as i32;
            if b != 0 {
                g1 = 0;
                if first < 0 {
                    first = j as i32;
                }
            } else if g1 != 0 && g1 < 3 {
                g1 += 1;
            }
            j += 1;
        }
        self.rw.c1 = g1 as i32;
        if first >= 0 {
            let ctx = CX_GT2 + set + if cidx != 0 { 4 } else { 0 };
            let b = self.cb_dec(ctx);
            self.rw.abs[(first & 15) as usize] += b as i32;
        }
        // sign data hiding
        let mut hidden = false;
        if self.p.sdh != 0 && self.u.byp == 0 {
            let a = self.rw.sig[0] as i32;
            let b = self.rw.sig[((nsig - 1) & 15) as usize] as i32;
            if a - b > 3 {
                hidden = true;
            }
        }
        for j in 0..nsig as usize {
            self.rw.sign[j & 15] = 2; // 2 = hidden
            if !(hidden && j as u32 == nsig - 1) {
                self.rw.sign[j & 15] = self.cb_byp() as u8;
            }
        }
        // coeff_abs_level_remaining
        let mut rice = 0u32; // cRiceParam
        let mut fc2 = 1i32; // firstCoeff2
        for j in 0..nsig as usize {
            let base = if j < 8 { fc2 + 2 } else { 1 };
            if self.rw.abs[j & 15] == base {
                let mut prefix = 0u32;
                while self.cb_byp() != 0 {
                    prefix += 1;
                    if prefix > 28 {
                        self.err = ERR_HDR;
                        return ERR_HDR;
                    }
                }
                let v = if prefix <= 3 {
                    let s = self.cb_bypn(rice);
                    s.wrapping_add(prefix << rice)
                } else {
                    let s = self.cb_bypn(prefix - 3 + rice);
                    s.wrapping_add(((1u32 << (prefix - 3)) + 2) << rice)
                };
                let v = (v as i32).wrapping_add(base);
                self.rw.abs[j & 15] = v;
                if v > (3 << rice) && rice < 4 {
                    rice += 1;
                }
            }
            if self.rw.abs[j & 15] >= 2 {
                fc2 = 0;
            }
        }
        // TransCoeffLevel into coef (raster y * n + x)
        let mut sum = 0i32; // sumAbsLevel
        let pt = self.rw.ptab as usize;
        let n = self.rw.n;
        let (xs, ys) = (self.rw.xs, self.rw.ys);
        for j in 0..nsig as usize {
            let mut v = self.rw.abs[j & 15];
            sum = sum.wrapping_add(v);
            let s = self.rw.sign[j & 15];
            if (s == 2 && sum & 1 != 0) || s == 1 {
                v = v.wrapping_neg();
            }
            let p = SCANS[pt + (self.rw.sig[j & 15] & 15) as usize] as u32; // xP | yP << 4
            let xc = (p & 15) + xs * 4;
            let yc = (p >> 4) + ys * 4;
            let idx = (yc.wrapping_mul(n).wrapping_add(xc) & 1023) as usize;
            self.coef[idx] = v;
        }
        0
    }
}
