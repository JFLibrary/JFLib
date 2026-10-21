// enc -- AV1 intra encoder: one key frame with a reduced still picture header (AVIF items)
//                                                    [src/img/av1_enc.s]
//
//   encode(cfg) -> EncOut / ERR_*   av1_encode: the planes of cfg (A1E) as OBUs (sequence header
//                                   OBU, frame OBU), the length of the sequence header OBU (the
//                                   av1C configOBUs), seq_profile and seq_level_idx (A1O)
//
// Model: tools/av1_enc_ref.py (byte-exact).  Stream: profile 0 (4:2:0, 4:0:0) or 1 (4:4:4), 8 or
// 10 bits, colour description from cfg.cicp (identity matrix: no color_range, 4:4:4), 64x64
// superblocks, intra edge filter, no filter intra / CDEF / restoration / superres / film grain;
// uniform tiles (sides of at least cfg.tile_sb superblocks, at most 16 tile columns and 64 tiles,
// the level limits of tile width and area), encoded in parallel (plat::parallel_for); base_q_idx
// cfg.qidx (0: lossless), deblocking level from the quantizer, TX_MODE_LARGEST, reduced transform
// set.  The headers are parsed by the decoder (Av1Dec::obus) whose frame state and tile code the
// encoder then uses (eblk, etx).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use img_core::{ERR_DIMS, ERR_HDR, ERR_UNSUPPORTED};
use rt::sync::SpinLock;

use crate::blk::{Ts, T};
use crate::coef::Cf;
use crate::dec::{zvec, Frame, Hdr};
use crate::eblk::Snap;
use crate::eec::Ec;
use crate::tab::{AC_QLOOKUP, DC_QLOOKUP};

const OBU_SEQ: u8 = 1;
const OBU_FHDR: u8 = 3;
const OBU_FRAME: u8 = 6;
const AV_MAXDIM: u32 = 65536; // av1_int.inc

/// av1_encode's input (A1E)
#[derive(Clone, Copy)]
pub struct EncIn<'a> {
    pub w: u32,       // frame size (1 .. 65536)
    pub h: u32,
    pub bd: u32,      // bit depth: 8 / 10
    pub chroma: u32,  // 0 (4:0:0) / 1 (4:2:0) / 3 (4:4:4)
    pub qidx: u32,    // base_q_idx 0 .. 255 (0 = lossless)
    pub cicp: u32,    // primaries | transfer << 8 | matrix << 16 | full range << 24
    pub tile_sb: u32, // smallest tile side in superblocks (0: 4)
    pub flags: u32,   // (reserved, 0)
    pub cancel: Option<&'a AtomicU32>,
    pub planes: [&'a [u16]; 3], // u16 samples; chroma (w + subX) >> subX wide
    pub strides: [u32; 3],      // in samples
}

/// av1_encode's output (A1O)
#[derive(Default)]
pub struct EncOut {
    pub data: Vec<u8>, // OBUs: sequence header OBU, frame OBU
    pub seqlen: u32,   // bytes of the sequence header OBU (the av1C configOBUs)
    pub profile: u32,  // seq_profile
    pub level: u32,    // seq_level_idx
}

// levels (seq_level_idx + 1, MaxPicSize, MaxHSize, MaxVSize)  [es_levels]
static ES_LEVELS: [[u32; 4]; 7] = [
    [1, 147456, 2048, 1152],
    [2, 278784, 2816, 1584],
    [5, 665856, 4352, 2448],
    [6, 1065024, 5504, 3096],
    [9, 2359296, 6144, 3456],
    [13, 8912896, 8192, 4352],
    [17, 35651584, 16384, 8704],
];

/// the frame encoder record (EF)
pub(crate) struct Ef<'a> {
    pub cfg: &'a EncIn<'a>,
    pub w: u32,
    pub h: u32,
    pub bd: u32,
    pub chroma: u32,
    pub qidx: u32,
    pub np: u32, // planes
    pub ssx: u32,
    pub ssy: u32,
    pub micols: u32,
    pub mirows: u32,
    pub lflvl: u32, // loop filter level (0 lossless)
    pub profile: u32,
    pub level: u32, // seq_level_idx
    pub tilesb: u32,
    pub nt: u32,   // tiles
    pub lam: u64,  // lambda (J = SSD * 65536 + lambda * Q8 bits)
    pub lams: u64, // lambda of the SATD pre-selection
    pub spl: [Vec<u16>; 3], // source planes, replicated to the padded size
    pub sst: [u32; 3],      // their strides (samples)
    pub sw: [u32; 3],       // valid samples per plane row / plane rows
    pub sh: [u32; 3],
    pub cicp: u32,
    seq: Vec<u8>, // sequence header OBU
    fh: Vec<u8>,  // frame header (byte aligned)
}

// the bit writer (EF_BW / EF_BWPOS): MSB first
#[derive(Default)]
struct Bw {
    buf: Vec<u8>,
    pos: u32,
}

impl Bw {
    // bw_put: append nbits (0..32) of value MSB first
    fn put(&mut self, v: u32, n: u32) {
        let mut k = n;
        while k != 0 {
            k -= 1;
            if (v >> (k & 31)) & 1 != 0 {
                let b = (self.pos >> 3) as usize;
                if self.buf.len() <= b {
                    self.buf.resize(b + 1, 0);
                }
                self.buf[b] |= 0x80 >> (self.pos & 7);
            }
            self.pos += 1;
        }
    }

    // the written bytes (zero padded to whole bytes)
    fn bytes(&self) -> Vec<u8> {
        let n = self.pos.div_ceil(8) as usize;
        let mut v = self.buf.clone();
        v.resize(n, 0);
        v
    }
}

// tlog2: smallest k with blk << k >= target
fn tlog2(blk: u32, target: u32) -> u32 {
    let mut k = 0;
    while (blk << k) < target {
        k += 1;
    }
    k
}

// the bit length of v (at least 1)
fn bitlen1(v: u32) -> u32 {
    if v == 0 { 1 } else { 32 - v.leading_zeros() }
}

// floor(sqrt(v)) exactly (the assembly's sqrtsd estimate and its corrections)
fn isqrt(v: u64) -> u64 {
    let mut r = rt::fx::sqrtd(v as f64) as u64;
    while r * r > v {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= v {
        r += 1;
    }
    r
}

/// av1_encode
pub fn encode(cfg: &EncIn) -> Result<EncOut, i32> {
    if cfg.w.wrapping_sub(1) > AV_MAXDIM - 1 || cfg.h.wrapping_sub(1) > AV_MAXDIM - 1 {
        return Err(ERR_DIMS);
    }
    if !(cfg.bd == 8 || cfg.bd == 10) || !matches!(cfg.chroma, 0 | 1 | 3) || cfg.qidx > 255 {
        return Err(ERR_UNSUPPORTED);
    }
    let mut ef = Ef::setup(cfg);
    ef.seqhdr();
    ef.framehdr();
    let av = ef.state()?;
    ef.source(&av);
    let tiles = ef.tiles(&av)?;
    Ok(ef.output(&tiles))
}

impl<'a> Ef<'a> {
    // ef_setup: frame parameters from the A1E record (model Encoder.__init__, lam_q8, lf_level,
    // the level table)
    fn setup(cfg: &'a EncIn<'a>) -> Ef<'a> {
        let (np, ss) = match cfg.chroma {
            0 => (1, 1), // subsampling 1 / 1 as the decoder
            3 => (3, 0),
            _ => (3, 1),
        };
        let mut ef = Ef {
            cfg,
            w: cfg.w,
            h: cfg.h,
            bd: cfg.bd,
            chroma: cfg.chroma,
            qidx: cfg.qidx,
            np,
            ssx: ss,
            ssy: ss,
            micols: cfg.w.div_ceil(8) * 2,
            mirows: cfg.h.div_ceil(8) * 2,
            lflvl: 0,
            profile: (cfg.chroma == 3) as u32,
            level: 31,
            tilesb: if cfg.tile_sb != 0 { cfg.tile_sb } else { 4 },
            nt: 0,
            lam: 256,
            lams: 0,
            spl: [Vec::new(), Vec::new(), Vec::new()],
            sst: [0; 3],
            sw: [0; 3],
            sh: [0; 3],
            cicp: cfg.cicp,
            seq: Vec::new(),
            fh: Vec::new(),
        };
        // seq_level_idx: the first level whose picture size, width and height limits hold
        let pic = cfg.w as u64 * cfg.h as u64;
        for l in ES_LEVELS.iter() {
            if pic <= l[1] as u64 && cfg.w <= l[2] && cfg.h <= l[3] {
                ef.level = l[0] - 1;
                break;
            }
        }
        // quantizers: lambda = max(1, q^2 * (6600 + 3 * q8) / 16000) (q = dc_q), lossless 256
        let cls = ((cfg.bd - 8) >> 1) as usize * 256;
        if cfg.qidx != 0 {
            let q = DC_QLOOKUP[cls + cfg.qidx as usize] as u64;
            let q8 = (q as u32 >> (cfg.bd - 8)) * 3 + 6600;
            ef.lam = (q * q * q8 as u64 / 16000).max(1);
        }
        // lambda_satd = 16 * isqrt(lambda)
        ef.lams = isqrt(ef.lam) << 4;
        // deblocking level (libaom LPF_PICK_FROM_Q of a key frame), 0 when lossless
        if cfg.qidx != 0 {
            let q = AC_QLOOKUP[cls + cfg.qidx as usize] as i32;
            let lf = if cfg.bd == 8 {
                (q * 17563 - (421574 - (1 << 17))) >> 18
            } else {
                ((q * 20723 + 4060632 + (1 << 19)) >> 20) - 4
            };
            ef.lflvl = lf.clamp(0, 63) as u32;
        }
        ef
    }

    // ef_seqhdr: sequence_header_obu (reduced still picture header) -> seq
    fn seqhdr(&mut self) {
        let mut b = Bw::default();
        b.put(self.profile, 3);
        b.put(1, 1); // still_picture
        b.put(1, 1); // reduced_still_picture_header
        b.put(self.level, 5);
        let nw = bitlen1(self.w - 1);
        let nh = bitlen1(self.h - 1);
        b.put(nw - 1, 4);
        b.put(nh - 1, 4);
        b.put(self.w - 1, nw);
        b.put(self.h - 1, nh);
        b.put(0, 1); // use_128x128_superblock
        b.put(0, 1); // enable_filter_intra
        b.put(1, 1); // enable_intra_edge_filter
        b.put(0, 1); // enable_superres
        b.put(0, 1); // enable_cdef
        b.put(0, 1); // enable_restoration
        // color_config
        b.put((self.bd > 8) as u32, 1); // high_bitdepth
        if self.profile != 1 {
            b.put((self.chroma == 0) as u32, 1); // mono_chrome
        }
        b.put(1, 1); // color_description_present_flag
        b.put(self.cicp & 0xFF, 8);
        b.put((self.cicp >> 8) & 0xFF, 8);
        b.put((self.cicp >> 16) & 0xFF, 8);
        let full = self.cicp >> 24; // full range
        if self.chroma == 0 {
            b.put(full, 1); // color_range
        } else {
            if self.cicp & 0xFFFFFF != 1 | (13 << 8) {
                // not BT.709 / sRGB / identity: color_range
                b.put(full, 1);
                if self.chroma == 1 {
                    b.put(0, 2); // chroma_sample_position (unknown)
                }
            }
            b.put(0, 1); // separate_uv_delta_q
        }
        b.put(0, 1); // film_grain_params_present
        b.put(1, 1); // trailing_bits
        let pay = b.bytes(); // < 128 bytes
        let mut seq = Vec::with_capacity(pay.len() + 2);
        seq.push(OBU_SEQ << 3 | 2);
        seq.push(pay.len() as u8);
        seq.extend_from_slice(&pay);
        self.seq = seq;
    }

    // ef_framehdr: uncompressed_header (key frame of a reduced still picture header) -> fh (byte
    // aligned with zero bits)
    fn framehdr(&mut self) {
        let mut b = Bw::default();
        b.put(0, 1); // disable_cdf_update
        b.put(0, 1); // allow_screen_content_tools
        b.put(0, 1); // render_and_frame_size_different
        // tile_info, uniform spacing
        let sb_cols = (self.micols + 15) >> 4;
        let sb_rows = (self.mirows + 15) >> 4;
        let min_l2c = tlog2(64, sb_cols); // maxTileWidthSb
        let max_l2c = tlog2(1, sb_cols.min(64));
        let max_l2r = tlog2(1, sb_rows.min(64));
        let min_l2t = tlog2(2304, sb_cols * sb_rows).max(min_l2c); // maxTileAreaSb
        b.put(1, 1); // uniform_tile_spacing_flag
        // tcl = Clip3(minLog2TileCols, Min(maxLog2TileCols, 4), FloorLog2(Max(1, sbCols / tile_sb)))
        let fl = 31 - (sb_cols / self.tilesb).max(1).leading_zeros();
        let tcl = fl.min(max_l2c.min(4)).max(min_l2c);
        for _ in min_l2c..tcl {
            b.put(1, 1); // increment_tile_cols_log2
        }
        if tcl < max_l2c {
            b.put(0, 1);
        }
        // minLog2TileRows = Max(minLog2Tiles - tcl, 0); trl = Clip3(it, Max(it, Min(maxLog2TileRows,
        // 6 - tcl)), FloorLog2(Max(1, sbRows / tile_sb)))  (signed as the assembly)
        let min_l2r = (min_l2t as i32 - tcl as i32).max(0);
        let flr = (31 - (sb_rows / self.tilesb).max(1).leading_zeros()) as i32;
        let hi = (6 - tcl as i32).min(max_l2r as i32).max(min_l2r);
        let trl = flr.min(hi).max(min_l2r) as u32;
        for _ in min_l2r as u32..trl {
            b.put(1, 1); // increment_tile_rows_log2
        }
        if trl < max_l2r {
            b.put(0, 1);
        }
        if tcl + trl != 0 {
            b.put(0, tcl + trl); // context_update_tile_id
            b.put(3, 2); // tile_size_bytes_minus_1: 4 bytes
        }
        // quantization_params
        b.put(self.qidx, 8);
        b.put(0, 1); // DeltaQYDc
        if self.chroma != 0 {
            b.put(0, 1); // DeltaQUDc
            b.put(0, 1); // DeltaQUAc
        }
        b.put(0, 1); // using_qmatrix
        b.put(0, 1); // segmentation_enabled
        if self.qidx != 0 {
            b.put(0, 1); // delta_q_present
            // loop_filter_params
            let lf = self.lflvl;
            b.put(lf, 6);
            b.put(lf, 6);
            if self.chroma != 0 && lf != 0 {
                b.put(lf, 6);
                b.put(lf, 6);
            }
            b.put(0, 3); // loop_filter_sharpness
            b.put(0, 1); // loop_filter_delta_enabled
            b.put(0, 1); // tx_mode_select (TX_MODE_LARGEST)
        }
        b.put(1, 1); // reduced_tx_set
        self.fh = b.bytes();
    }

    // ef_state: the decoder parses the headers (sequence header OBU and a frame header OBU) and
    // allocates the frame state
    fn state(&mut self) -> Result<crate::Av1Dec<'static>, i32> {
        let mut obus = Vec::with_capacity(self.seq.len() + self.fh.len() + 2);
        obus.extend_from_slice(&self.seq);
        obus.push(OBU_FHDR << 3 | 2);
        obus.push(self.fh.len() as u8);
        obus.extend_from_slice(&self.fh);
        let mut av = crate::Av1Dec::new();
        av.obus(&obus)?;
        let (state, h, _) = av.enc_view();
        if state != 2 || h.micols != self.micols {
            return Err(ERR_HDR);
        }
        self.nt = h.tcols * h.trows;
        Ok(av)
    }

    // ef_source: source planes with the decoder's plane geometry (AV_PS x AV_PH), the image
    // replicated into the padding (model residual_of clamps to the image)
    fn source(&mut self, av: &crate::Av1Dec) {
        let (_, _, f) = av.enc_view();
        for p in 0..self.np as usize {
            let (sw, sh) = if p == 0 {
                (self.w, self.h)
            } else {
                ((self.w + self.ssx) >> self.ssx, (self.h + self.ssy) >> self.ssy)
            };
            self.sw[p] = sw;
            self.sh[p] = sh;
            let st = f.ps[p];
            let rows = f.ph[p];
            self.sst[p] = st;
            let src = self.cfg.planes[p];
            let sst = self.cfg.strides[p] as usize;
            let mut pl = alloc::vec![0u16; st as usize * rows as usize];
            for (y, row) in pl.chunks_exact_mut(st as usize).enumerate() {
                let sy = (y as u32).min(sh - 1) as usize; // source row (clamped)
                let sr = &src[sy * sst..sy * sst + sw as usize];
                row[..sw as usize].copy_from_slice(sr);
                let last = sr[sw as usize - 1];
                row[sw as usize..].fill(last);
            }
            self.spl[p] = pl;
        }
    }

    // ef_output: sequence header OBU, then OBU_FRAME: frame header, (tile_start_and_end_present_flag
    // = 0 when several tiles, byte aligned), tile sizes (4 bytes, little endian, minus 1) and tile
    // data
    fn output(&self, tiles: &[Vec<u8>]) -> EncOut {
        let nt = tiles.len();
        let mut pay = self.fh.len() as u64 + (nt > 1) as u64;
        for (i, t) in tiles.iter().enumerate() {
            pay += t.len() as u64;
            if i + 1 < nt {
                pay += 4;
            }
        }
        let mut o = Vec::with_capacity(pay as usize + self.seq.len() + 16);
        o.extend_from_slice(&self.seq);
        o.push(OBU_FRAME << 3 | 2);
        let mut v = pay; // leb128(payload length)
        loop {
            let c = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                o.push(c);
                break;
            }
            o.push(c | 0x80);
        }
        o.extend_from_slice(&self.fh);
        if nt > 1 {
            o.push(0); // tile_start_and_end_present_flag, alignment
        }
        for (i, t) in tiles.iter().enumerate() {
            if i + 1 < nt {
                o.extend_from_slice(&((t.len() as u32).wrapping_sub(1)).to_le_bytes());
            }
            o.extend_from_slice(t);
        }
        EncOut { data: o, seqlen: self.seq.len() as u32, profile: self.profile, level: self.level }
    }

    /// the cancel flag (EF_CANCEL) is set
    pub fn cancelled(&self) -> bool {
        self.cfg.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }
}

// a zeroed fixed-size array on the heap
fn zbox<T: Copy + Default, const N: usize>() -> Box<[T; N]> {
    match alloc::vec![T::default(); N].into_boxed_slice().try_into() {
        Ok(b) => b,
        Err(_) => unreachable!(),
    }
}

/// the tile encoder record (ET): the decoder's tile state TS (prediction, reconstruction,
/// contexts and the CDF context are the decoder's code), the range encoder, the block search
/// state and the scratch arrays
pub(crate) struct Et<'a> {
    pub t: T<'a>,
    pub ef: &'a Ef<'a>,
    pub ec: Ec,
    pub err: i32,       // sticky ERR_* of the job
    pub lam: u64,       // copies of Ef::lam / lams
    pub lams: u64,
    pub dcq: [u32; 3],  // dc_q / ac_q per plane
    pub acq: [u32; 3],
    pub ntxb: [u32; 3], // transform blocks of the last tryplane per plane
    pub dec: u32,       // decisions of the last block: yMode | uvMode << 8
    pub tbn: u32,       // entries of tbl (txblocks; ET_TBTX, written only, is left out)
    pub spl: [&'a [u16]; 3], // the source planes (Ef::spl / sst / sw / sh)
    pub sst: [u32; 3],
    pub sw: [u32; 3],
    pub sh: [u32; 3],
    pub cf: Cf,              // CF record for the decoder's cf_* functions
    pub tbl: Box<[u32; 256]>, // txblocks: x | y << 16 (plane samples)
    pub txb: Box<[[u64; 256]; 3]>, // per plane transform block: x | y << 16 | txType << 32
    pub nodes: Box<[[u8; 4]; 85]>, // decisions per superblock: kind, yMode, uvMode
    pub res: Box<[i32; 4096]>, // residual (stride 64)
    pub c: Box<[i32; 4096]>,   // forward coefficients (stride 64)
    pub a: Box<[u64; 1024]>,   // |C| * K
    pub lv: Box<[i32; 1024]>,  // rounded levels
    pub out: Box<[i32; 1024]>, // levels (quant result, signed)
    pub tmp: Box<[i32; 1024]>, // trial levels (RDOQ)
    pub neg: Box<[u8; 1024]>,  // signs
    pub qb: Box<[[i32; 4096]; 3]>, // levels per plane and transform block
    pub snaps: [[Snap; 2]; 4], // snapshots: 2 per depth
}

impl<'a> Ef<'a> {
    // the tiles of the frame, encoded in parallel (plat::parallel_for over ae_tilejob); the first
    // error in tile order
    fn tiles(&self, av: &crate::Av1Dec) -> Result<Vec<Vec<u8>>, i32> {
        let (_, h, f) = av.enc_view();
        let nt = self.nt as usize;
        let outs: Vec<SpinLock<Result<Vec<u8>, i32>>> = (0..nt).map(|_| SpinLock::new(Err(ERR_UNSUPPORTED))).collect();
        let geo = (f.ps, f.ph, f.mis, f.mir);
        let item = |idx: usize, _job: &plat::JobCtl| {
            let r = self.tilejob(h, geo, idx);
            *outs[idx].lock() = r;
        };
        let _ = plat::parallel_for(nt, &item);
        let mut v = Vec::with_capacity(nt);
        for o in outs {
            v.push(core::mem::replace(&mut *o.lock(), Ok(Vec::new()))?);
        }
        Ok(v)
    }

    // ae_tilejob: encode tile idx.  The tile works on frame buffers of its own (the decoder's
    // Frame layout, zero pages from the OS that only its region touches) and a tile state of its
    // own: AV1 tiles never read across their bounds, so the bytes equal those of the assembly's
    // shared frame.
    fn tilejob(&self, h: &Hdr, geo: ([u32; 3], [u32; 3], u32, u32), idx: usize) -> Result<Vec<u8>, i32> {
        let (ps, ph, mis, mir) = geo;
        let tr = idx / h.tcols as usize;
        let tc = idx % h.tcols as usize;
        let (mrs, mre) = (h.mirs[tr] as u32, h.mirs[tr + 1] as u32);
        let (mcs, mce) = (h.mics[tc] as u32, h.mics[tc + 1] as u32);
        // the rows the tile touches: its superblocks (the last row padded) and the MIB padding
        let sbe = ((mre + 15) & !15).min(mir);
        let mut f = Frame::new();
        for p in 0..h.np as usize {
            let sy = if p != 0 { h.ssy } else { 0 };
            let rows = ((sbe << 2) >> sy).min(ph[p]);
            f.ps[p] = ps[p];
            f.ph[p] = ph[p];
            f.pl[p] = zvec::<u16>(rows as usize * ps[p] as usize)?;
        }
        f.mis = mis;
        f.mir = mir;
        f.mi = zvec::<[u8; 16]>(sbe as usize * mis as usize)?;
        let mut s = Ts::new();
        s.above_alloc(h.micols + 64)?;
        s.mrs = mrs;
        s.mre = mre;
        s.mcs = mcs;
        s.mce = mce;
        let mut t = T { s: &mut s, h, f: &mut f, d: &[] };
        t.tilesetup();
        // encoder parameters
        let cls = ((self.bd - 8) >> 1) as usize * 256;
        let q = self.qidx as usize;
        let (dq, aq) = (DC_QLOOKUP[cls + q] as u32, AC_QLOOKUP[cls + q] as u32);
        let mut et = Et {
            t,
            ef: self,
            ec: Ec::new(),
            err: 0,
            lam: self.lam,
            lams: self.lams,
            dcq: [dq; 3],
            acq: [aq; 3],
            ntxb: [0; 3],
            dec: 0,
            tbn: 0,
            spl: [&self.spl[0], &self.spl[1], &self.spl[2]],
            sst: self.sst,
            sw: self.sw,
            sh: self.sh,
            cf: Cf {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: 0,
                h4: 0,
                txsz: 0,
                txctx: 0,
                ptype: 0,
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
            },
            tbl: zbox(),
            txb: Box::new([[0; 256]; 3]),
            nodes: zbox(),
            res: zbox(),
            c: zbox(),
            a: zbox(),
            lv: zbox(),
            out: zbox(),
            tmp: zbox(),
            neg: zbox(),
            qb: Box::new([[0; 4096]; 3]),
            snaps: Default::default(),
        };
        let e = et.tile();
        if e != 0 {
            return Err(e);
        }
        Ok(et.ec.done())
    }
}
