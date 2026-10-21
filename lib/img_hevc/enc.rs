// enc -- HEVC intra encoder for HEIC (model: tools/hevc_enc_ref.py, byte-exact)
//                                                                [src/img/hevc_enc.s]
//
//   encode(cfg) -> EncOut / ERR_*    EncCfg (HECFG) -> VPS, SPS, PPS and one IDR slice of the
//                                    picture (NAL units with header and emulation
//                                    prevention) + the 12 general PTL bytes (hvcC)
//
// Structure (see the model): CTB 32, coding blocks 8..32 chosen by rate-distortion search,
// transform block = coding block, PART_2Nx2N, chroma mode DM, no SAO, deblocking (unless
// HEF_NODEBLOCK), 4:0:0 / 4:2:0 / 4:4:4, 8 bits; HEF_LOSSLESS: cu_transquant_bypass in every
// CU.  Luma modes: SATD of planar, DC and the even angular modes, then the best angular +-1;
// the two cheapest and MPM 0 are coded for real (prediction / reconstruction through the
// decoder's intra / recon_tb on a decoder record, CABAC bits estimated with exact context
// tracking), J = SSE << 15 + (lambda_q8 * bits_q15 >> 8).  After the search of a CTU its
// decisions (depth and mode maps) are coded again with the real arithmetic coder (9.3.4.3
// PutBit form).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use img_core::ERR_CANCEL;

use crate::ctu::{
    ctx_init_into, sbscan, CTXMAP4, CX_CBFCHROMA, CX_CBFLUMA, CX_CHROMAMODE, CX_CSBF, CX_GT1, CX_GT2, CX_LASTX,
    CX_LASTY, CX_PARTMODE, CX_PREVINTRA, CX_SIG, CX_SPLITCU, CX_TQBYPASS, LPS, SCAN4, SCANS, TLPS, TMPS,
};
use crate::dec::{Hevc, HV_NCTX};
use crate::rec::{DCT32, QPC};

pub const HEF_LOSSLESS: u32 = 1; // cu_transquant_bypass for every CU (QP ignored)
pub const HEF_NODEBLOCK: u32 = 2; // pps_deblocking_filter_disabled (tests)

const LOG2_CTB: u32 = 5;
const NAL_VPS: u32 = 32;
const NAL_SPS: u32 = 33;
const NAL_PPS: u32 = 34;
const NAL_IDR: u32 = 19;
const SK_SA: usize = 1; // snapshot kinds: cu_search before / after the leaf (0, 1),
const SK_L0: usize = 2; // leaf_search before / best candidate
const SK_LB: usize = 3;

/// the input of encode (HECFG)
#[derive(Clone, Copy)]
pub struct EncCfg<'a> {
    pub pl: [&'a [u16]; 3], // source planes (u16 samples, 8-bit values)
    pub st: [u32; 3],       // strides in samples
    pub w: u32,             // picture width (even for 4:2:0)
    pub h: u32,             // picture height (even for 4:2:0)
    pub chroma: u32,        // chroma_format_idc 0 / 1 / 3
    pub qp: u32,            // QP 0..51
    pub flags: u32,         // HEF_*
    pub vui: u32,           // primaries | transfer << 8 | matrix << 16 | full range << 24
    pub cancel: Option<&'a AtomicU32>, // polled per CTU
}

/// the result of encode (HEOUT)
#[derive(Clone, Default)]
pub struct EncOut {
    pub nal: [Vec<u8>; 4], // VPS, SPS, PPS, slice: NAL units (header, emulation prevention)
    pub ptl: [u8; 12],     // general profile / tier / level bytes (hvcC)
}

static QSCALE: [u32; 6] = [26214, 23302, 20560, 18396, 16384, 14564];
static EBL: [u32; 64] = [
    32768, 35232, 37696, 40159, 42623, 45087, 47551, 50015, 52479, 54942, 57406, 59870, 62334, 64798, 67262, 69725,
    72189, 74653, 77117, 79581, 82044, 84508, 86972, 89436, 91900, 94364, 96827, 99291, 101755, 104219, 106683, 109147,
    111610, 114074, 116538, 119002, 121466, 123929, 126393, 128857, 131321, 133785, 136249, 138712, 141176, 143640,
    146104, 148568, 151032, 153495, 155959, 158423, 160887, 163351, 165814, 168278, 170742, 173206, 175670, 178134,
    180597, 183061, 185525, 185525,
];
static EBM: [u32; 64] = [
    32768, 30426, 28306, 26377, 24617, 23005, 21523, 20159, 18899, 17734, 16653, 15650, 14717, 13849, 13038, 12282,
    11575, 10914, 10294, 9714, 9169, 8658, 8178, 7727, 7303, 6903, 6527, 6173, 5840, 5525, 5228, 4948, 4684, 4435, 4199,
    3977, 3767, 3568, 3380, 3202, 3034, 2876, 2725, 2583, 2448, 2321, 2200, 2086, 1978, 1875, 1778, 1686, 1599, 1517,
    1439, 1364, 1294, 1228, 1164, 1105, 1048, 994, 943, 943,
];
static LAM: [u32; 52] = [
    9, 11, 14, 18, 23, 29, 36, 46, 58, 73, 92, 116, 146, 184, 232, 292, 368, 463, 584, 735, 927, 1167, 1471, 1853, 2335,
    2942, 3706, 4669, 5883, 7412, 9339, 11766, 14825, 18678, 23533, 29649, 37356, 47065, 59298, 74711, 94130, 118596,
    149422, 188260, 237193, 298844, 376520, 474386, 597688, 753040, 948771, 1195377,
];
static LAMS: [u32; 52] = [
    48, 54, 61, 68, 77, 86, 97, 108, 122, 137, 153, 172, 193, 217, 244, 273, 307, 344, 387, 434, 487, 547, 614, 689,
    773, 868, 974, 1093, 1227, 1378, 1546, 1736, 1948, 2187, 2454, 2755, 3092, 3471, 3896, 4373, 4909, 5510, 6185,
    6942, 7792, 8747, 9818, 11020, 12370, 13884, 15585, 17493,
];

/// a snapshot of a CU region (SN_*): planes, decoded / mode / depth maps, search contexts
#[derive(Clone)]
struct Snap {
    pl: [[u16; 1024]; 3],
    dec: [i32; 64],
    ipm: [u8; 64],
    dep: [u8; 64],
    ctx: [u8; HV_NCTX],
}

/// the residual coding state (RS_*)
#[derive(Clone, Copy)]
struct Rs {
    c: usize,
    log2: u32,
    n: u32,
    scan: u32,
    nsb: u32,
    lastsb: u32,
    lastpos: u32,
    prevg1: i32, // greater1 state of the previous sub-block (-1 = none)
    i: u32,
    xs: u32,
    ys: u32,
    infer: bool,
    nsig: u32,
    prevcs: u32,
    sbt: usize, // sub-block scan (offset in SCANS)
    pst: usize, // position scan (offset in SCANS)
    csbf: [u8; 64],
    vals: [i32; 16], // levels of the sub-block in scan order
    sig: [u8; 16],   // significant scan positions (reverse order)
    abs: [i32; 16],  // their absolute levels
}

impl Default for Rs {
    fn default() -> Self {
        Rs {
            c: 0,
            log2: 0,
            n: 0,
            scan: 0,
            nsb: 0,
            lastsb: 0,
            lastpos: 0,
            prevg1: 0,
            i: 0,
            xs: 0,
            ys: 0,
            infer: false,
            nsig: 0,
            prevcs: 0,
            sbt: 0,
            pst: 0,
            csbf: [0; 64],
            vals: [0; 16],
            sig: [0; 16],
            abs: [0; 16],
        }
    }
}

/// the encoder record (HE)
struct He<'a> {
    hv: Hevc<'a>,           // prediction / reconstruction
    src: [Vec<u16>; 3],     // padded source planes (coded size)
    sst: [u32; 3],          // their strides
    w: u32,                 // coded size (multiple of 8)
    h: u32,
    w0: u32, // picture size
    h0: u32,
    chroma: u32,
    subw: u32,
    subh: u32,
    ncomp: u32,
    qp: u32,
    qpc: u32, // chroma QP
    flags: u32,
    vui: u32,
    real: bool, // real CABAC (ctx) / estimate (sctx)
    lam: u64,   // lambda q8
    lams: u64,  // lambda_sad q8
    est: u64,   // estimated bits (Q15)
    cancel: Option<&'a AtomicU32>,
    out: EncOut,
    bw: Vec<u8>, // bit writer: bytes written
    bwacc: u32,  // pending bits
    bwn: u32,    // number of pending bits (0..7)
    low: u32,    // CABAC low
    range: u32,
    first: bool, // firstBitFlag
    outst: u32,  // bitsOutstanding
    cbf: [u32; 3], // coded block flags of the current CU
    mpm: u32,      // MPM list of the current CU (m0 | m1 << 8 | m2 << 16)
    ptl: [u8; 12],
    ctx: [u8; HV_NCTX],  // real contexts
    sctx: [u8; HV_NCTX], // search contexts
    res: [i32; 1024],    // residual / transform input
    tmp: [i32; 1024],    // transform scratch
    lev: [[i32; 1024]; 3], // levels of the current CU
    snap: Vec<Snap>,     // 12 slots (level * 4 + kind)
    rs: Rs,
    fde: [i32; 32], // forward DCT: even part
    fdo: [i32; 16], // odd part
    fdy: [i32; 32], // outputs
    fdn: u32,       // line length
    fdsh: u32,      // shift
}

/// hevc_encode: encode the picture of cfg (see the file header)
pub fn encode(cfg: &EncCfg) -> Result<EncOut, i32> {
    let mut he = Box::new(He {
        hv: Hevc::new(0, 0),
        src: [Vec::new(), Vec::new(), Vec::new()],
        sst: [0; 3],
        w: (cfg.w + 7) & !7,
        h: (cfg.h + 7) & !7,
        w0: cfg.w,
        h0: cfg.h,
        chroma: cfg.chroma,
        subw: 1,
        subh: 1,
        ncomp: 3,
        qp: 0,
        qpc: 0,
        flags: cfg.flags,
        vui: cfg.vui,
        real: false,
        lam: 0,
        lams: 0,
        est: 0,
        cancel: cfg.cancel,
        out: EncOut::default(),
        bw: Vec::new(),
        bwacc: 0,
        bwn: 0,
        low: 0,
        range: 0,
        first: false,
        outst: 0,
        cbf: [0; 3],
        mpm: 0,
        ptl: [0; 12],
        ctx: [0; HV_NCTX],
        sctx: [0; HV_NCTX],
        res: [0; 1024],
        tmp: [0; 1024],
        lev: [[0; 1024]; 3],
        snap: vec![Snap { pl: [[0; 1024]; 3], dec: [0; 64], ipm: [0; 64], dep: [0; 64], ctx: [0; HV_NCTX] }; 12],
        rs: Rs::default(),
        fde: [0; 32],
        fdo: [0; 16],
        fdy: [0; 32],
        fdn: 0,
        fdsh: 0,
    });
    let mut qp = (cfg.qp as i32).max(0).min(51) as u32;
    if he.flags & HEF_LOSSLESS != 0 {
        qp = 0;
    }
    he.qp = qp;
    he.lam = LAM[qp as usize] as u64;
    he.lams = LAMS[qp as usize] as u64;
    // chroma QP: qPi = QP (no offsets); 4:2:0 table, else min(qPi, 51)
    let mut qpc = qp;
    if cfg.chroma == 1 {
        he.subw = 2;
        he.subh = 2;
        if qp >= 30 {
            qpc = if qp > 43 { qp - 6 } else { QPC[(qp - 30) as usize] as u32 };
        }
    } else if cfg.chroma == 0 {
        he.ncomp = 1;
    }
    he.qpc = qpc;
    // padded source copies
    he.source(cfg);
    // parameter sets
    he.headers();
    // decoder record: parameter sets parsed by the decoder, picture planes and maps
    for i in 1..3 {
        let nal = he.out.nal[i].clone();
        let r = he.hv.nal(&nal);
        if r != 0 {
            return Err(r);
        }
    }
    let r = he.hv.newpic();
    if r != 0 {
        return Err(r);
    }
    he.hv.u.qpy = qp as i32;
    he.hv.u.byp = he.flags & HEF_LOSSLESS;
    he.hv.u.sliceaddr = 0;
    he.hv.u.tile = 0;
    // slice
    let r = he.slice();
    if r != 0 {
        return Err(r);
    }
    he.nal(NAL_IDR, 3);
    Ok(core::mem::take(&mut he.out))
}

impl He<'_> {
    /// he_source: copy the source planes into coded-size planes, replicating the last column
    /// / row
    fn source(&mut self, cfg: &EncCfg) {
        for c in 0..self.ncomp as usize {
            let (sw, sh) = if c == 0 { (1, 1) } else { (self.subw, self.subh) };
            let pw = self.w / sw; // plane width
            let ph = self.h / sh;
            self.sst[c] = pw;
            let srcw = self.w0 / sw; // source width
            let srch = self.h0 / sh;
            let mut p = vec![0u16; pw as usize * ph as usize];
            let st = cfg.st[c] as usize;
            let sp = cfg.pl[c];
            for r in 0..ph {
                let sr = r.min(srch.wrapping_sub(1)) as usize; // source row (clamped)
                let drow = &mut p[(r * pw) as usize..((r + 1) * pw) as usize];
                for (x, d) in drow.iter_mut().enumerate() {
                    let sx = (x as u32).min(srcw.wrapping_sub(1)) as usize;
                    // (a plane of size 0 -- 1-pixel 4:2:0 sizes -- reads beyond it in the
                    // assembly; 0 here)
                    *d = sp.get(sr * st + sx).copied().unwrap_or(0) & 255;
                }
            }
            self.src[c] = p;
        }
    }

    // ================================================================== bit writer
    /// BWBIT: append one bit
    #[inline]
    fn bwbit(&mut self, b: u32) {
        let acc = self.bwacc.wrapping_add(self.bwacc) | b;
        let n = self.bwn + 1;
        if n >= 8 {
            self.bw.push(acc as u8);
            self.bwacc = 0;
            self.bwn = 0;
        } else {
            self.bwacc = acc;
            self.bwn = n;
        }
    }

    /// bw_bits: count (0..64) bits of v, MSB first
    fn bits(&mut self, v: u64, count: u32) {
        let mut k = count;
        while k != 0 {
            k -= 1;
            self.bwbit((v >> k) as u32 & 1);
        }
    }

    /// bw_ue: ue(v)
    fn ue(&mut self, v: u32) {
        let x = v as u64 + 1;
        let b = 63 - x.leading_zeros();
        self.bits(x, b * 2 + 1);
    }

    /// bw_se: se(v)
    fn se(&mut self, v: i32) {
        if v > 0 {
            self.ue((v as u32).wrapping_mul(2).wrapping_sub(1));
        } else {
            self.ue((v as u32).wrapping_neg().wrapping_mul(2));
        }
    }

    /// bw_trail: rbsp_trailing_bits / byte_alignment: a one, then zeros to the byte boundary
    fn trail(&mut self) {
        let n = 8 - self.bwn; // 1..8 bits
        self.bits(1 << (n - 1), n);
    }

    /// he_nal: the bytes written (an RBSP) as a NAL unit (2-byte header, emulation
    /// prevention) into out.nal[index]; the writer is emptied
    fn nal(&mut self, ty: u32, index: usize) {
        let mut d = Vec::with_capacity(self.bw.len() + self.bw.len() / 2 + 16);
        d.push((ty * 2) as u8);
        d.push(1);
        let mut z = 0u32; // zero run
        for &b in &self.bw {
            if z >= 2 && b <= 3 {
                d.push(3);
                z = 0;
            }
            d.push(b);
            z = if b == 0 { z + 1 } else { 0 };
        }
        self.out.nal[index] = d;
        self.bw.clear();
        self.bwacc = 0;
        self.bwn = 0;
    }

    // ================================================================== parameter sets
    /// he_ptlbytes: the 12 general profile_tier_level bytes: Main for 4:2:0, format range
    /// extensions (profile 4) Main 4:4:4 / Monochrome otherwise, level 6.2
    fn ptlbytes(&mut self) {
        let mut p = [0u8; 12];
        if self.chroma == 1 {
            p[0] = 1;
            p[1] = 0x60;
            p[5] = 0x90;
        } else {
            p[0] = 4;
            p[1] = 0x08;
            p[5] = 0x9E;
            p[6] = 0x08;
            if self.chroma != 3 {
                p[5] = 0x9F;
                p[6] = 0xC8;
            }
        }
        p[11] = 186;
        self.ptl = p;
        self.out.ptl = p;
    }

    /// he_ptl: write the 12 PTL bytes
    fn ptlw(&mut self) {
        for i in 0..12 {
            let b = self.ptl[i];
            self.bits(b as u64, 8);
        }
    }

    /// he_headers: VPS, SPS and PPS into out.nal[0..2]
    fn headers(&mut self) {
        self.ptlbytes();
        // ---- VPS
        self.bits(0, 4);
        self.bits(1, 1);
        self.bits(1, 1);
        self.bits(0, 6);
        self.bits(0, 3);
        self.bits(1, 1);
        self.bits(0xFFFF, 16);
        self.ptlw();
        self.bits(1, 1); // sub_layer_ordering_info_present
        self.ue(0);
        self.ue(0);
        self.ue(0);
        self.bits(0, 6);
        self.ue(0);
        self.bits(0, 1);
        self.bits(0, 1);
        self.trail();
        self.nal(NAL_VPS, 0);
        // ---- SPS
        self.bits(0, 4);
        self.bits(0, 3);
        self.bits(1, 1);
        self.ptlw();
        self.ue(0);
        self.ue(self.chroma);
        if self.chroma == 3 {
            self.bits(0, 1);
        }
        self.ue(self.w);
        self.ue(self.h);
        // conformance window (right / bottom, in chroma units)
        let cr = (self.w - self.w0) / self.subw;
        let cb = (self.h - self.h0) / self.subh;
        if cr | cb != 0 {
            self.bits(1, 1);
            self.ue(0);
            self.ue(cr);
            self.ue(0);
            self.ue(cb);
        } else {
            self.bits(0, 1);
        }
        self.ue(0);
        self.ue(0);
        self.ue(4);
        self.bits(1, 1);
        self.ue(0);
        self.ue(0);
        self.ue(0);
        self.ue(0); // log2_min_luma_coding_block_size_minus3
        self.ue(LOG2_CTB - 3);
        self.ue(0);
        self.ue(3);
        self.ue(0);
        self.ue(0);
        self.bits(0, 1); // scaling lists
        self.bits(0, 1); // amp
        self.bits(0, 1); // sao
        self.bits(0, 1); // pcm
        self.ue(0);
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(1, 1); // strong intra smoothing
        self.bits(1, 1); // vui
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(1, 1); // video_signal_type_present
        self.bits(5, 3);
        self.bits((self.vui >> 24 & 1) as u64, 1);
        self.bits(1, 1);
        self.bits((self.vui & 255) as u64, 8);
        self.bits((self.vui >> 8 & 255) as u64, 8);
        self.bits((self.vui >> 16 & 255) as u64, 8);
        self.bits(0, 1); // chroma_loc
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(0, 1); // default display window
        self.bits(0, 1); // timing
        self.bits(0, 1); // bitstream restriction
        self.bits(0, 1); // sps_extension
        self.trail();
        self.nal(NAL_SPS, 1);
        // ---- PPS
        self.ue(0);
        self.ue(0);
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(0, 3);
        self.bits(0, 1); // sign data hiding
        self.bits(0, 1);
        self.ue(0);
        self.ue(0);
        self.se(self.qp as i32 - 26);
        self.bits(0, 1);
        self.bits(0, 1); // transform skip
        self.bits(0, 1); // cu_qp_delta
        self.se(0);
        self.se(0);
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits(0, 1);
        self.bits((self.flags & HEF_LOSSLESS) as u64, 1); // transquant_bypass_enabled
        self.bits(0, 1); // tiles
        self.bits(0, 1); // entropy coding sync
        self.bits(0, 1); // loop filter across slices
        if self.flags & HEF_NODEBLOCK != 0 {
            self.bits(1, 1);
            self.bits(0, 1); // override enabled
            self.bits(1, 1); // pps_deblocking_filter_disabled
        } else {
            self.bits(0, 1); // deblocking_filter_control_present
        }
        self.bits(0, 1);
        self.bits(0, 1);
        self.ue(0);
        self.bits(0, 1);
        self.bits(0, 1);
        self.trail();
        self.nal(NAL_PPS, 2);
    }

    // ================================================================== CABAC
    /// PutBit (9.3.4.3)
    #[inline]
    fn putbit(&mut self, b: u32) {
        if self.first {
            self.first = false;
        } else {
            self.bwbit(b);
        }
        while self.outst != 0 {
            self.bwbit(b ^ 1);
            self.outst -= 1;
        }
    }

    /// cab_renorm: RenormE
    fn renorm(&mut self) {
        while self.range < 256 {
            if self.low < 256 {
                self.putbit(0);
            } else if self.low >= 512 {
                self.low -= 512;
                self.putbit(1);
            } else {
                self.low -= 256;
                self.outst += 1;
            }
            self.range <<= 1;
            self.low <<= 1;
        }
    }

    /// he_bin: one context-coded bin: estimate (sctx, est) or encode (ctx)
    fn bin(&mut self, ctx: usize, bin: u32) {
        let ctx = if ctx < HV_NCTX { ctx } else { 0 };
        if !self.real {
            let s = self.sctx[ctx] as usize;
            let st = (s >> 1) & 63; // pStateIdx
            let mut mps = (s & 1) as u8; // valMps
            if bin == mps as u32 {
                self.est = self.est.wrapping_add(EBM[st] as u64);
                self.sctx[ctx] = TMPS[st] * 2 + mps;
            } else {
                self.est = self.est.wrapping_add(EBL[st] as u64);
                if st == 0 {
                    mps ^= 1;
                }
                self.sctx[ctx] = TLPS[st] * 2 + mps;
            }
            return;
        }
        let s = self.ctx[ctx] as usize;
        let st = (s >> 1) & 63;
        let mut mps = (s & 1) as u8;
        let lps = LPS[st][((self.range >> 6) & 3) as usize] as u32;
        let r = self.range - lps;
        if bin == mps as u32 {
            self.ctx[ctx] = TMPS[st] * 2 + mps;
            self.range = r;
        } else {
            self.low = self.low.wrapping_add(r);
            self.range = lps;
            if st == 0 {
                mps ^= 1;
            }
            self.ctx[ctx] = TLPS[st] * 2 + mps;
        }
        self.renorm();
    }

    /// BYPASS: EncodeBypass of bin b
    #[inline]
    fn bypass(&mut self, b: u32) {
        let mut v = self.low.wrapping_add(self.low);
        if b != 0 {
            v = v.wrapping_add(self.range);
        }
        if v >= 1024 {
            self.low = v - 1024;
            self.putbit(1);
        } else if v < 512 {
            self.low = v;
            self.putbit(0);
        } else {
            self.low = v - 512;
            self.outst += 1;
        }
    }

    /// he_byp: one bypass bin
    fn byp(&mut self, b: u32) {
        if !self.real {
            self.est = self.est.wrapping_add(32768);
        } else {
            self.bypass(b);
        }
    }

    /// he_bypn: count (0..32) bypass bins of v, MSB first
    fn bypn(&mut self, v: u32, count: u32) {
        if !self.real {
            self.est = self.est.wrapping_add((count as u64) << 15);
            return;
        }
        let mut k = count;
        while k != 0 {
            k -= 1;
            self.bypass(v.wrapping_shr(k) & 1);
        }
    }

    /// he_term: EncodeTerminate (real coder only), with EncodeFlush for bin 1 (its last bit
    /// is the rbsp_stop_one_bit)
    fn term(&mut self, b: u32) {
        self.range -= 2;
        if b == 0 {
            self.renorm();
            return;
        }
        self.low = self.low.wrapping_add(self.range);
        self.range = 2;
        self.renorm();
        self.putbit((self.low >> 9) & 1);
        self.bwbit((self.low >> 8) & 1);
        self.bwbit(1);
    }

    // ================================================================== maps
    /// he_avail: the luma position is inside the coded picture and decoded already (one
    /// slice, one tile)
    #[inline]
    fn avail(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return false;
        }
        self.hv.mdec[self.hv.mapidx(x as u32, y as u32)] == 0
    }

    /// he_mpm: candModeList (m0 | m1 << 8 | m2 << 16) of a 2Nx2N CU (8.4.2)
    fn mpm(&self, x: u32, y: u32) -> u32 {
        // A = left
        let a = if self.avail(x as i32 - 1, y as i32) { self.hv.mipm[self.hv.mapidx(x - 1, y)] as u32 } else { 1 };
        // B = above (DC when outside the CTB row)
        let mut b = 1u32;
        if y & ((1 << LOG2_CTB) - 1) != 0 && self.avail(x as i32, y as i32 - 1) {
            b = self.hv.mipm[self.hv.mapidx(x, y - 1)] as u32;
        }
        if a == b {
            if a < 2 {
                return 1 << 8 | 26 << 16;
            }
            let m1 = ((a + 29) & 31) + 2;
            let m2 = ((a - 1) & 31) + 2;
            return a | m1 << 8 | m2 << 16;
        }
        let mut c = if a == 1 || b == 1 { 26 } else { 1 };
        if a != 0 && b != 0 {
            c = 0;
        }
        a | b << 8 | c << 16
    }

    /// he_splitctx: ctxInc of split_cu_flag (9.3.4.2.2)
    fn splitctx(&self, x: u32, y: u32, depth: u32) -> usize {
        let mut c = 0;
        if self.avail(x as i32 - 1, y as i32) && self.hv.mdepth[self.hv.mapidx(x - 1, y)] as u32 > depth {
            c += 1;
        }
        if self.avail(x as i32, y as i32 - 1) && self.hv.mdepth[self.hv.mapidx(x, y - 1)] as u32 > depth {
            c += 1;
        }
        c
    }

    /// he_fillmap: set the 4x4 entries of an n x n luma region in map which: 0 mdec (i32),
    /// 1 mipm (u8), 2 mdepth (u8)
    fn fillmap(&mut self, x: u32, y: u32, n: u32, v: u32, which: u32) {
        let k = (n >> 2) as usize; // blocks per side
        let first = self.hv.mapidx(x, y);
        let mw = self.hv.mw as usize;
        for r in 0..k {
            let o = first + r * mw;
            match which {
                0 => self.hv.mdec[o..o + k].fill(v as i32),
                1 => self.hv.mipm[o..o + k].fill(v as u8),
                _ => self.hv.mdepth[o..o + k].fill(v as u8),
            }
        }
    }

    // ================================================================== transforms
    /// he_satd: SATD of the n x n residual in res (n >= 8): per 8 x 8 block the sum of
    /// |Hadamard coefficients|, (s + 2) >> 2
    fn satd(&self, n: u32) -> u64 {
        #[inline(always)]
        fn bfly(b: &mut [i32; 64], i: usize, j: usize) {
            let (x, y) = (b[i], b[j]);
            b[i] = x.wrapping_add(y);
            b[j] = x.wrapping_sub(y);
        }
        #[inline(always)]
        fn had8(b: &mut [i32; 64], o: usize, s: usize) {
            bfly(b, o, o + s);
            bfly(b, o + 2 * s, o + 3 * s);
            bfly(b, o + 4 * s, o + 5 * s);
            bfly(b, o + 6 * s, o + 7 * s);
            bfly(b, o, o + 2 * s);
            bfly(b, o + s, o + 3 * s);
            bfly(b, o + 4 * s, o + 6 * s);
            bfly(b, o + 5 * s, o + 7 * s);
            bfly(b, o, o + 4 * s);
            bfly(b, o + s, o + 5 * s);
            bfly(b, o + 2 * s, o + 6 * s);
            bfly(b, o + 3 * s, o + 7 * s);
        }
        let n = n as usize;
        let mut total = 0u64;
        let mut by = 0;
        while by < n {
            let mut bx = 0;
            while bx < n {
                let mut b = [0i32; 64];
                for (i, v) in b.iter_mut().enumerate() {
                    *v = self.res[((i >> 3) + by) * n + (i & 7) + bx];
                }
                for r in 0..8 {
                    had8(&mut b, r * 8, 1);
                }
                for c in 0..8 {
                    had8(&mut b, c, 8);
                }
                let mut s = 0u32;
                for &v in &b {
                    s = s.wrapping_add(v.unsigned_abs());
                }
                total += (s.wrapping_add(2) >> 2) as u64;
                bx += 8;
            }
            by += 8;
        }
        total
    }

    /// he_fdct: forward DCT of the n x n residual in res into lev[c] (raster): rows with
    /// shift log2 - 1, columns with log2 + 6 (fd_line: partial butterflies, the same integer
    /// sums as the matrix product of the model)
    fn fdct(&mut self, log2: u32, c: usize) {
        let n = 1usize << log2;
        self.fdn = n as u32;
        self.fdsh = log2 - 1;
        for r in 0..n {
            let mut line = [0i32; 32];
            line[..n].copy_from_slice(&self.res[r * n..r * n + n]);
            let out = self.fd_line(&line);
            self.tmp[r * n..r * n + n].copy_from_slice(&out[..n]);
        }
        self.fdsh = log2 + 6;
        for col in 0..n {
            let mut line = [0i32; 32];
            for (k, v) in line.iter_mut().enumerate().take(n) {
                *v = self.tmp[k * n + col];
            }
            let out = self.fd_line(&line);
            for k in 0..n {
                self.lev[c][k * n + col] = out[k];
            }
        }
    }

    /// fd_line: forward DCT of one line of fdn samples: at size m the odd outputs from the
    /// differences x[j] - x[m-1-j] (matrix rows k * 32 / m of DCT32), the even part continues
    /// with the sums; rounded and shifted by fdsh
    fn fd_line(&mut self, src: &[i32; 32]) -> [i32; 32] {
        let n = (self.fdn & 63) as usize;
        self.fde[..n].copy_from_slice(&src[..n]);
        let mut m = n;
        while m > 1 {
            let h = m >> 1;
            for j in 0..h {
                let e = self.fde[j];
                let o = self.fde[m - 1 - j];
                self.fde[j] = e.wrapping_add(o);
                self.fdo[j] = e.wrapping_sub(o);
            }
            let lm = 31 - (m as u32).leading_zeros(); // log2 m
            let rstep = 32usize >> lm; // matrix row step
            let ostep = n >> lm; // output index step n / m
            let mut k = 1usize;
            while k < m {
                let roff = (k * rstep) << 5; // matrix row offset
                let mut sum = 0i32;
                for j in 0..h {
                    sum = sum.wrapping_add((DCT32[(roff + j) & 1023] as i32).wrapping_mul(self.fdo[j]));
                }
                self.fdy[k * ostep] = sum;
                k += 2;
            }
            m = h;
        }
        self.fdy[0] = self.fde[0].wrapping_shl(6);
        let sh = self.fdsh;
        let rnd = 1i32 << (sh - 1);
        let mut out = [0i32; 32];
        for i in 0..n {
            out[i] = self.fdy[i].wrapping_add(rnd) >> sh;
        }
        out
    }

    // ================================================================== residual coding
    /// rs_loadsb: sub-block rs.i -> rs.xs / rs.ys and its 16 levels in scan order (rs.vals)
    fn loadsb(&mut self) {
        let v = SCANS[self.rs.sbt + (self.rs.i & 63) as usize] as u32;
        self.rs.xs = v & 15;
        self.rs.ys = v >> 4;
        let n = self.rs.n;
        let c = self.rs.c;
        for k in 0..16 {
            let p = SCANS[self.rs.pst + k] as u32;
            let xp = p & 15;
            let yp = p >> 4;
            let i = (yp + self.rs.ys * 4) * n + xp + self.rs.xs * 4;
            self.rs.vals[k] = self.lev[c][i as usize & 1023];
        }
    }

    /// rs_prevcs: csbf of the right | below << 1 sub-block of (rs.xs, rs.ys)
    fn prevcs(&self) -> u32 {
        let (xs, ys) = (self.rs.xs, self.rs.ys);
        let mut v = 0u32;
        if xs + 1 < self.rs.nsb {
            v = self.rs.csbf[((xs + 1 + ys * 8) & 63) as usize] as u32;
        }
        if ys + 1 < self.rs.nsb {
            v |= (self.rs.csbf[((xs + (ys + 1) * 8) & 63) as usize] as u32) * 2;
        }
        v
    }

    /// rs_sigctx: context index of sig_coeff_flag at scan position k
    fn sigctx(&self, k: u32) -> usize {
        let p = SCANS[self.rs.pst + (k & 15) as usize] as u32;
        let xp = p & 15;
        let yp = p >> 4;
        let mut c;
        if self.rs.log2 == 2 {
            c = CTXMAP4[((yp << 2) + xp) as usize & 15] as u32; // 4x4 TB: xc = xp, yc = yp
        } else if self.rs.xs | self.rs.ys == 0 && xp | yp == 0 {
            c = 0; // DC
        } else {
            c = match self.rs.prevcs {
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
                3 => 2,
                _ => {
                    let s = xp + yp; // prevCsbf 0
                    if s == 0 {
                        2
                    } else if s < 3 {
                        1
                    } else {
                        0
                    }
                }
            };
            if self.rs.c == 0 {
                if self.rs.xs | self.rs.ys != 0 {
                    c += 3;
                }
                if self.rs.log2 == 3 {
                    c += 9;
                    if self.rs.scan != 0 {
                        c += 6;
                    }
                } else {
                    c += 21;
                }
            } else {
                c += 12;
                if self.rs.log2 == 3 {
                    c -= 3;
                }
            }
        }
        if self.rs.c != 0 {
            c += 27;
        }
        CX_SIG + c as usize
    }

    /// he_rem: coeff_abs_level_remaining (bypass bins)
    fn rem(&mut self, rem: u32, rice: u32) {
        if rem < (4 << rice) {
            let prefix = rem >> rice;
            self.bypn((1 << (prefix + 1)) - 2, prefix + 1);
            self.bypn(((1 << rice) - 1) & rem, rice);
            return;
        }
        let mut prefix = 4u32;
        while prefix < 30 && rem >= ((1 << (prefix - 2)) + 2) << rice {
            prefix += 1;
        }
        self.bypn((1u32 << (prefix + 1)).wrapping_sub(2), prefix + 1);
        let base = ((1u32 << (prefix - 3)) + 2) << rice;
        self.bypn(rem.wrapping_sub(base), prefix - 3 + rice);
    }

    /// he_resid: residual_coding() of the levels lev[c] (mirror of the decoder's parsing,
    /// see tools/hevc_enc_ref.py Encoder.residual)
    fn resid(&mut self, c: usize, log2: u32, mode: u32) {
        self.rs.c = c;
        self.rs.log2 = log2;
        self.rs.n = 1 << log2;
        // scanIdx
        let mut scan = 0u32;
        if log2 == 2 || (log2 == 3 && (c == 0 || self.chroma == 3)) {
            if mode.wrapping_sub(6) <= 8 {
                scan = 2;
            } else if mode.wrapping_sub(22) <= 8 {
                scan = 1;
            }
        }
        self.rs.scan = scan;
        // scan tables
        let nsb = 1u32 << (log2 - 2);
        self.rs.nsb = nsb;
        self.rs.sbt = sbscan(nsb, scan);
        self.rs.pst = SCAN4 + scan as usize * 16;
        // last significant coefficient
        let mut i = nsb * nsb;
        let mut found = false;
        while i > 0 && !found {
            i -= 1;
            self.rs.i = i;
            self.loadsb();
            let mut k = 16;
            while k > 0 {
                k -= 1;
                if self.rs.vals[k] != 0 {
                    self.rs.lastsb = i;
                    self.rs.lastpos = k as u32;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            return; // (never: the caller has cbf 1)
        }
        let p = SCANS[self.rs.pst + self.rs.lastpos as usize] as u32;
        let mut lx = (p & 15) + self.rs.xs * 4;
        let mut ly = (p >> 4) + self.rs.ys * 4;
        if scan == 2 {
            core::mem::swap(&mut lx, &mut ly);
        }
        // contexts of the prefixes
        let (off, shift) = if c == 0 { ((log2 - 2) * 3 + ((log2 - 1) >> 2), (log2 + 1) >> 2) } else { (15, log2 - 2) };
        let pfx = [lastpfx(lx), lastpfx(ly)];
        let lv = [lx, ly];
        // prefixes: truncated unary, cMax = 2 * log2 - 1
        for t in 0..2 {
            let base = if t == 0 { CX_LASTX } else { CX_LASTY };
            for b in 0..pfx[t] {
                self.bin(base + ((b >> shift) + off) as usize, 1);
            }
            if pfx[t] < log2 * 2 - 1 {
                self.bin(base + ((pfx[t] >> shift) + off) as usize, 0);
            }
        }
        // suffixes
        for t in 0..2 {
            let p = pfx[t];
            if p > 3 {
                let bits = (p >> 1) - 1;
                let base = ((p & 1) + 2) << bits;
                self.bypn(lv[t].wrapping_sub(base), bits);
            }
        }
        // sub-blocks
        self.rs.csbf = [0; 64];
        self.rs.prevg1 = -1;
        let mut i = self.rs.lastsb as i32;
        while i >= 0 {
            self.rs.i = i as u32;
            self.loadsb();
            self.rs.infer = false;
            let ci = ((self.rs.ys << 3) + self.rs.xs) as usize & 63; // csbf index
            if i != 0 && (i as u32) < self.rs.lastsb {
                let pc = self.prevcs();
                let ctx = CX_CSBF + (pc != 0) as usize + if c != 0 { 2 } else { 0 };
                let any = self.rs.vals.iter().any(|&v| v != 0) as u32;
                self.rs.csbf[ci] = any as u8;
                self.bin(ctx, any);
                self.rs.infer = true;
            } else {
                self.rs.csbf[ci] = 1;
            }
            self.rs.nsig = 0;
            let mut k = 15i32;
            if i as u32 == self.rs.lastsb {
                self.rs.sig[0] = self.rs.lastpos as u8;
                self.rs.nsig = 1;
                k = self.rs.lastpos as i32 - 1;
            }
            self.rs.prevcs = self.prevcs();
            if self.rs.csbf[ci] != 0 {
                while k >= 0 {
                    if k == 0 && self.rs.infer {
                        let n = (self.rs.nsig & 15) as usize;
                        self.rs.sig[n] = 0; // inferred DC
                        self.rs.nsig += 1;
                    } else {
                        let ctx = self.sigctx(k as u32);
                        let b = (self.rs.vals[k as usize] != 0) as u32;
                        self.bin(ctx, b);
                        if b != 0 {
                            let n = (self.rs.nsig & 15) as usize;
                            self.rs.sig[n] = k as u8;
                            self.rs.nsig += 1;
                            self.rs.infer = false;
                        }
                    }
                    k -= 1;
                }
            }
            if self.rs.nsig != 0 {
                self.levels(i as u32);
            }
            i -= 1;
        }
    }

    /// the levels part of he_resid for sub-block i
    fn levels(&mut self, i: u32) {
        let c = self.rs.c;
        let nsig = self.rs.nsig as usize;
        for j in 0..nsig.min(16) {
            self.rs.abs[j] = self.rs.vals[(self.rs.sig[j] & 15) as usize].wrapping_abs();
        }
        let mut set = if i != 0 && c == 0 { 2usize } else { 0 };
        if self.rs.prevg1 == 0 {
            set += 1;
        }
        let mut g1 = 1usize;
        let mut first: i32 = -1;
        let mut j = 0usize;
        while j < 8 && j < nsig {
            let ctx = CX_GT1 + set * 4 + g1 + if c != 0 { 16 } else { 0 };
            let b = (self.rs.abs[j] > 1) as u32;
            self.bin(ctx, b);
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
        self.rs.prevg1 = g1 as i32;
        if first >= 0 {
            let ctx = CX_GT2 + set + if c != 0 { 4 } else { 0 };
            let b = (self.rs.abs[(first & 15) as usize] > 2) as u32;
            self.bin(ctx, b);
        }
        // signs
        for j in 0..nsig.min(16) {
            let v = self.rs.vals[(self.rs.sig[j] & 15) as usize];
            self.byp((v as u32) >> 31);
        }
        // remaining levels
        let mut rice = 0u32;
        let mut fc2 = 1u32;
        for j in 0..nsig.min(16) {
            let level = self.rs.abs[j] as u32;
            let mut partial = 1u32; // 1 + gt1 + gt2
            let mut base = 1u32;
            if j < 8 {
                if level > 1 {
                    partial += 1;
                    if j as i32 == first && level > 2 {
                        partial += 1;
                    }
                }
                base = 2 + fc2;
            }
            if partial == base {
                self.rem(level.wrapping_sub(base), rice);
                if level > 3 << rice && rice < 4 {
                    rice += 1;
                }
            }
            if level >= 2 {
                fc2 = 0;
            }
        }
    }

    // ================================================================== coding units
    /// he_codecu: CU syntax after split_cu_flag: transquant bypass, part_mode, luma mode
    /// (MPM mpm), chroma DM, cbfs, residuals of lev
    fn codecu(&mut self, log2: u32, mode: u32) {
        if self.flags & HEF_LOSSLESS != 0 {
            self.bin(CX_TQBYPASS, 1);
        }
        if log2 == 3 {
            self.bin(CX_PARTMODE, 1);
        }
        let m = self.mpm;
        let idx = (0..3).find(|&i| (m >> (i * 8)) & 255 == mode);
        match idx {
            Some(i) => {
                self.bin(CX_PREVINTRA, 1);
                self.byp((i != 0) as u32);
                if i != 0 {
                    self.byp((i > 1) as u32);
                }
            }
            None => {
                self.bin(CX_PREVINTRA, 0);
                // rem_intra_luma_pred_mode = mode - #(candidates below it)
                let mut r = mode;
                for i in 0..3 {
                    if (m >> (i * 8)) & 255 < mode {
                        r -= 1;
                    }
                }
                self.bypn(r, 5);
            }
        }
        if self.ncomp != 1 {
            self.bin(CX_CHROMAMODE, 0);
            let (c1, c2) = (self.cbf[1], self.cbf[2]);
            self.bin(CX_CBFCHROMA, c1);
            self.bin(CX_CBFCHROMA, c2);
        }
        let c0 = self.cbf[0];
        self.bin(CX_CBFLUMA + 1, c0);
        for c in 0..self.ncomp as usize {
            if self.cbf[c] != 0 {
                let l = if c != 0 && self.subw != 1 { log2 - 1 } else { log2 };
                self.resid(c, l, mode);
            }
        }
    }

    /// he_leaf -> SSE: predict, transform, quantise and reconstruct the CU with luma mode
    /// mode (chroma DM), update the maps, code its syntax (real decides: estimate or real);
    /// mpm must hold the CU's MPM list
    fn leaf(&mut self, x: u32, y: u32, log2: u32, depth: u32, mode: u32) -> u64 {
        let mut sse = 0u64;
        let n = 1u32 << log2;
        self.fillmap(x, y, n, depth, 2);
        self.fillmap(x, y, n, mode, 1);
        for c in 0..self.ncomp as usize {
            let (cx, cy, tl) = if c != 0 && self.subw != 1 { (x >> 1, y >> 1, log2 - 1) } else { (x, y, log2) };
            let tn = 1u32 << tl;
            // prediction into the plane
            self.hv.intra(c, cx, cy, tl, mode);
            // residual = source - prediction
            self.residual(c, cx, cy, tn);
            // levels
            let nn = (tn * tn) as usize;
            let cbf;
            if self.flags & HEF_LOSSLESS != 0 {
                let mut any = 0i32;
                for k in 0..nn {
                    let v = self.res[k];
                    self.lev[c][k] = v;
                    any |= v;
                }
                cbf = (any != 0) as u32;
            } else {
                self.fdct(tl, c);
                let q = if c == 0 { self.qp } else { self.qpc };
                cbf = quant(tl, q, &mut self.lev[c]);
            }
            self.cbf[c] = cbf;
            if cbf != 0 {
                // reconstruction: levels -> coef -> recon_tb
                self.hv.coef[..nn].copy_from_slice(&self.lev[c][..nn]);
                self.hv.recon_tb(c, cx, cy, tl, 0);
            }
            if c == 0 {
                self.fillmap(x, y, n, 0, 0); // value 0 (slice address)
            }
            sse += self.ssd(c, cx, cy, tn);
        }
        self.codecu(log2, mode);
        sse
    }

    /// he_residual: res = source - plane over the n x n block of component c
    fn residual(&mut self, c: usize, cx: u32, cy: u32, n: u32) {
        let st = self.sst[c] as usize;
        let pw = self.hv.pw[c] as usize;
        let (cx, cy, n) = (cx as usize, cy as usize, n as usize);
        let src = &self.src[c];
        let pl = &self.hv.pl[c];
        let mut o = 0usize;
        for r in 0..n {
            let s = &src[(cy + r) * st + cx..(cy + r) * st + cx + n];
            let p = &pl[(cy + r) * pw + cx..(cy + r) * pw + cx + n];
            for k in 0..n {
                self.res[o] = s[k] as i32 - p[k] as i32;
                o = (o + 1) & 1023;
            }
        }
    }

    /// he_ssd: sum of squared differences source - plane of the block
    fn ssd(&self, c: usize, cx: u32, cy: u32, n: u32) -> u64 {
        let st = self.sst[c] as usize;
        let pw = self.hv.pw[c] as usize;
        let (cx, cy, n) = (cx as usize, cy as usize, n as usize);
        let mut sum = 0u64;
        for r in 0..n {
            let s = &self.src[c][(cy + r) * st + cx..(cy + r) * st + cx + n];
            let p = &self.hv.pl[c][(cy + r) * pw + cx..(cy + r) * pw + cx + n];
            for k in 0..n {
                let d = s[k] as i32 - p[k] as i32;
                sum += (d * d) as u32 as u64;
            }
        }
        sum
    }

    // ================================================================== snapshots
    /// he_snap: copy the state of the CU region (planes, decoded / mode / depth maps,
    /// search contexts) into snapshot slot, or back (restore)
    fn snap(&mut self, slot: usize, x: u32, y: u32, log2: u32, restore: bool) {
        let n = 1u32 << log2;
        let ncomp = self.ncomp as usize;
        let half = self.subw != 1;
        let first = self.hv.mapidx(x, y);
        let mw = self.hv.mw as usize;
        let sn = &mut self.snap[slot];
        let hv = &mut self.hv;
        // planes
        for c in 0..ncomp {
            let (bx, by, bn) = if c != 0 && half { (x >> 1, y >> 1, n >> 1) } else { (x, y, n) };
            let pw = hv.pw[c] as usize;
            let bn = bn as usize;
            for r in 0..bn {
                let o = (by as usize + r) * pw + bx as usize;
                let s = &mut sn.pl[c][r * bn..r * bn + bn];
                let p = &mut hv.pl[c][o..o + bn];
                if restore {
                    p.copy_from_slice(s);
                } else {
                    s.copy_from_slice(p);
                }
            }
        }
        // maps: (n / 4)^2 entries
        let k = (n >> 2) as usize;
        for r in 0..k {
            for col in 0..k {
                let m = first + r * mw + col; // map index
                let si = (r * 8 + col) & 63; // snapshot index
                if restore {
                    hv.mdec[m] = sn.dec[si];
                    hv.mipm[m] = sn.ipm[si];
                    hv.mdepth[m] = sn.dep[si];
                } else {
                    sn.dec[si] = hv.mdec[m];
                    sn.ipm[si] = hv.mipm[m];
                    sn.dep[si] = hv.mdepth[m];
                }
            }
        }
        if restore {
            self.sctx = sn.ctx;
        } else {
            sn.ctx = self.sctx;
        }
    }

    // ================================================================== search
    /// he_sadcost: SATD-based cost of a luma mode (prediction into the plane): SATD << 15 +
    /// (lambda_sad * mode bits >> 8), mode bits 2 / 3 / 6 (MPM 0 / 1, 2 / other)
    fn sadcost(&mut self, x: u32, y: u32, log2: u32, mode: u32) -> u64 {
        self.hv.intra(0, x, y, log2, mode);
        let n = 1u32 << log2;
        self.residual(0, x, y, n);
        let s = self.satd(n) << 15;
        let m = self.mpm;
        let bits: u64 = if m & 255 == mode {
            2 << 15
        } else if (m >> 8) & 255 == mode || (m >> 16) & 255 == mode {
            3 << 15
        } else {
            6 << 15
        };
        (bits.wrapping_mul(self.lams) >> 8).wrapping_add(s)
    }

    /// he_leafsearch -> J of the best luma mode of the CU (its reconstruction, maps and
    /// contexts are left in place)
    fn leafsearch(&mut self, x: u32, y: u32, log2: u32, depth: u32) -> u64 {
        let level = ((5 - log2) * 4) as usize; // snapshot slot base
        self.mpm = self.mpm(x, y);
        // queue: planar, DC, 2, 4, .. 34
        let mut modes = [0u8; 24];
        for (i, m) in modes.iter_mut().enumerate().take(19) {
            *m = if i < 2 { i as u8 } else { (i * 2 - 2) as u8 };
        }
        let mut cost = [0u64; 21];
        let mut nq = 19usize;
        let mut ext = false;
        let mut i = 0usize;
        while i < 21 {
            if i >= nq {
                if ext {
                    break;
                }
                ext = true;
                // best angular mode of the first 19 (first minimum)
                let mut best = u64::MAX;
                let mut bm = 0u32;
                for k in 0..19 {
                    if modes[k] >= 2 && cost[k] < best {
                        best = cost[k];
                        bm = modes[k] as u32;
                    }
                }
                if bm.wrapping_sub(1) >= 2 && nq < 21 {
                    modes[nq] = (bm - 1) as u8;
                    nq += 1;
                }
                if bm + 1 <= 34 && nq < 21 {
                    modes[nq] = (bm + 1) as u8;
                    nq += 1;
                }
                continue;
            }
            let c = self.sadcost(x, y, log2, modes[i] as u32);
            cost[i] = c;
            i += 1;
        }
        // the two cheapest (ties: the earlier) and MPM 0
        let mut m0 = 0usize;
        let mut b = u64::MAX;
        let mut k = 0usize;
        while k < 21 {
            if cost[k] < b {
                b = cost[k];
                m0 = k;
            }
            k += 1;
            if k >= nq {
                break;
            }
        }
        let mut m1 = 0usize;
        let mut b = u64::MAX;
        let mut k = 0usize;
        while k < 21 {
            if k != m0 && cost[k] < b {
                b = cost[k];
                m1 = k;
            }
            k += 1;
            if k >= nq {
                break;
            }
        }
        let mut cand = [modes[m0] as u32, modes[m1] as u32, 0];
        let mut nc = 2;
        let mp0 = self.mpm & 255;
        if mp0 != cand[0] && mp0 != cand[1] {
            cand[2] = mp0;
            nc = 3;
        }
        // real coding of the candidates
        self.snap(level + SK_L0, x, y, log2, false);
        let mut best = u64::MAX;
        for &m in cand.iter().take(nc) {
            self.snap(level + SK_L0, x, y, log2, true);
            self.est = 0;
            let sse = self.leaf(x, y, log2, depth, m);
            let j = (sse << 15).wrapping_add(self.est.wrapping_mul(self.lam) >> 8);
            if j < best {
                best = j;
                self.snap(level + SK_LB, x, y, log2, false);
            }
        }
        self.snap(level + SK_LB, x, y, log2, true);
        best
    }

    /// he_cusearch -> J of the best coding of the block (split or not, recursively); the
    /// winner's state stays in place
    fn cusearch(&mut self, x: u32, y: u32, log2: u32, depth: u32) -> u64 {
        let level = ((5 - log2) * 4) as usize;
        let n = 1u32 << log2;
        if x + n > self.w || y + n > self.h {
            // crossing the picture boundary: split without a flag, sub-blocks inside only
            let mut j = 0u64;
            let half = n >> 1;
            for s in 0..4u32 {
                let sx = x + (s & 1) * half;
                let sy = y + (s >> 1) * half;
                if sx < self.w && sy < self.h {
                    j = j.wrapping_add(self.cusearch(sx, sy, log2 - 1, depth + 1));
                }
            }
            return j;
        }
        if log2 <= 3 {
            return self.leafsearch(x, y, log2, depth);
        }
        // no split
        self.snap(level, x, y, log2, false);
        self.est = 0;
        let c = self.splitctx(x, y, depth);
        self.bin(CX_SPLITCU + c, 0);
        let sbits = self.est;
        let j0 = self.leafsearch(x, y, log2, depth).wrapping_add(sbits.wrapping_mul(self.lam) >> 8);
        self.snap(level + SK_SA, x, y, log2, false);
        self.snap(level, x, y, log2, true);
        // split
        self.est = 0;
        let c = self.splitctx(x, y, depth);
        self.bin(CX_SPLITCU + c, 1);
        let mut j1 = self.est.wrapping_mul(self.lam) >> 8;
        let half = n >> 1;
        for s in 0..4u32 {
            let sx = x + (s & 1) * half;
            let sy = y + (s >> 1) * half;
            j1 = j1.wrapping_add(self.cusearch(sx, sy, log2 - 1, depth + 1));
        }
        if j1 < j0 {
            return j1;
        }
        self.snap(level + SK_SA, x, y, log2, true);
        j0
    }

    /// he_emit: code the decisions of the search (depth / mode maps) with the real coder,
    /// reconstructing every CU again
    fn emit(&mut self, x: u32, y: u32, log2: u32, depth: u32) {
        let n = 1u32 << log2;
        let mut split = true;
        if !(x + n > self.w || y + n > self.h) {
            if log2 <= 3 {
                split = false;
            } else {
                split = self.hv.mdepth[self.hv.mapidx(x, y)] as u32 > depth;
                let c = self.splitctx(x, y, depth);
                self.bin(CX_SPLITCU + c, split as u32);
            }
        }
        if split {
            let half = n >> 1;
            for s in 0..4u32 {
                let sx = x + (s & 1) * half;
                let sy = y + (s >> 1) * half;
                if sx < self.w && sy < self.h {
                    self.emit(sx, sy, log2 - 1, depth + 1);
                }
            }
            return;
        }
        self.mpm = self.mpm(x, y);
        let mode = self.hv.mipm[self.hv.mapidx(x, y)] as u32;
        self.leaf(x, y, log2, depth, mode);
    }

    // ================================================================== slice
    /// he_slice -> 0 / ERR_CANCEL: slice header and slice data of the picture into the
    /// writer
    fn slice(&mut self) -> i32 {
        self.bits(1, 1); // first_slice_segment_in_pic
        self.bits(0, 1); // no_output_of_prior_pics
        self.ue(0);
        self.ue(2); // I
        self.se(0); // slice_qp_delta
        self.trail(); // byte_alignment()
        ctx_init_into(&mut self.ctx, self.qp as i32);
        self.low = 0;
        self.range = 510;
        self.first = true;
        self.outst = 0;
        let cols = (self.w + 31) >> 5;
        let rows = (self.h + 31) >> 5;
        for cy in 0..rows {
            for cx in 0..cols {
                if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
                    return ERR_CANCEL;
                }
                // search with the current contexts
                self.sctx = self.ctx;
                self.real = false;
                self.cusearch(cx << LOG2_CTB, cy << LOG2_CTB, LOG2_CTB, 0);
                // the CTB counts as not decoded again; code its decisions for real
                self.undecode(cx, cy);
                self.real = true;
                self.emit(cx << LOG2_CTB, cy << LOG2_CTB, LOG2_CTB, 0);
                // end_of_slice_segment_flag
                let last = cx + 1 == cols && cy + 1 == rows;
                self.term(last as u32);
            }
        }
        if self.bwn != 0 {
            let n = 8 - self.bwn;
            self.bits(0, n);
        }
        0
    }

    /// he_undecode: the decoded flags of the CTB's 4x4 blocks (inside the coded picture) back
    /// to -1
    fn undecode(&mut self, cx: u32, cy: u32) {
        let c0 = cx << (LOG2_CTB - 2); // first 4x4 column
        let r0 = cy << (LOG2_CTB - 2); // first 4x4 row
        let mcols = self.w >> 2;
        let mrows = self.h >> 2;
        let mw = self.hv.mw;
        for r in 0..1u32 << (LOG2_CTB - 2) {
            if r0 + r >= mrows {
                break;
            }
            let base = (r0 + r) * mw;
            for c in 0..1u32 << (LOG2_CTB - 2) {
                if c0 + c >= mcols {
                    break;
                }
                self.hv.mdec[(base + c0 + c) as usize] = -1;
            }
        }
    }
}

/// he_lastpfx: prefix of last_sig_coeff_x / y of a position
fn lastpfx(p: u32) -> u32 {
    if p < 4 {
        return p;
    }
    let k = 31 - p.leading_zeros();
    if p < 3 << (k - 1) { k * 2 } else { k * 2 + 1 }
}

/// he_quant: HM quantisation in place: (|c| scale[qp % 6] + (171 << (qbits - 9))) >> qbits,
/// qbits = 21 + qp / 6 - log2 -> 1 when a level is not zero
fn quant(log2: u32, qp: u32, lev: &mut [i32; 1024]) -> u32 {
    let scale = QSCALE[(qp % 6) as usize] as u64;
    let qbits = 21 + qp / 6 - log2;
    let add = 171u64 << (qbits - 9);
    let nn = 1usize << (2 * log2);
    let mut nz = 0u32;
    for v in lev[..nn].iter_mut() {
        let c = *v as i64;
        let s = c >> 63;
        let a = ((c ^ s) - s) as u64; // |c|
        let q = (a.wrapping_mul(scale).wrapping_add(add)) >> qbits;
        nz |= q as u32;
        *v = ((q as i64 ^ s) - s) as i32; // sign back
    }
    (nz != 0) as u32
}
