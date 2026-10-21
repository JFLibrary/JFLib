// vp8_enc -- WebP lossy (VP8 key frame) encoder                        [src/img/webp_vp8_enc.s]
//
//   encode(sf, opt) -> Ok("VP8 " chunk payload) / Err(ERR_*)      vp8_encode
//   enc_argb(argb, w, h, cfg) -> Ok(VP8 stream) / Err(ERR_*)      vp8_enc_argb
//                                  argb straight BGRA (alpha ignored: the container codes it
//                                  as ALPH), w / h 1..16383, cfg quality 0 = 95, effort 0 = 5,
//                                  preset WPP_*
//
// Modelled on libwebp 1.3's encoder at method 4 (effort 5): RGB -> YUV 4:2:0 with libwebp's
// gamma-corrected, alpha-weighted chroma averaging; analysis of every macroblock's
// susceptibility (DCT histogram "alpha" of the best I16 / chroma prediction), k-means into up
// to 4 segments, quantisers from the quality curve (f64 series pow / log2 / exp2 like the
// assembly), libwebp's quantisation (bias / zero threshold / sharpening), rate-distortion mode
// decisions (I16 4 modes, I4 10 modes, chroma 4 modes: SSE + spectral distortion, token costs
// from level-cost tables, mode costs, flatness penalties), UV DC error diffusion, token
// statistics with the probabilities refreshed every VE_MAXCNT macroblocks and finally updated
// where cheaper, loop filter strength per segment, one token partition, no skip flag.  The
// reconstruction is the decoder's (vp8::itx / vp8::wht).  No threads (the assembly uses none).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::Surface;
use img_core::*;

use crate::container::{cfg_from_opt, load_argb};
use crate::tab::{ACTAB, BMODES, DCTAB, G2L, P4TAB, PROBA0, UPDATE, ZIGZAG};
use crate::vp8::{BPS, itx, wht};
use crate::{Cfg, WEF_NOEXACT, WPP_TEXT};

// ---------------------------------------------------------------- constants
const QFIX: u32 = 17;
const MAX_LEVEL: u32 = 2047;
const MAX_VAR: usize = 67; // MAX_VARIABLE_LEVEL
const FLAT_PEN: i64 = 140; // FLATNESS_PENALTY

// work buffer (stride BPS) offsets: source MB (Y cols 0..15, U 16..23, V 24..31), chosen and
// scratch reconstructions, predictions
const W_IN: usize = 0;
const W_OUT: usize = 512;
const W_OUT2: usize = 1024;
const W_P16: usize = 1536; // 4 x 512 I16 predictions
const W_PUV: usize = 3584; // 4 x 512 chroma predictions (cols 16..31)
const W_P4: usize = 5632; // 10 x 128 I4 predictions (4 rows)
const W_T4A: usize = 6912; // I4 reconstruction scratch
const W_T4B: usize = 7040;
const W_SIZE: usize = 7168;

// libwebp kLinearToGammaTab (33 entries)  [ve_l2g]
static L2G: [u32; 33] = [
    0, 3, 8, 13, 19, 25, 31, 38, 45, 52, 60, 67, 75, 83, 91, 99, 107, 116, 124, 133, 142, 151, 160, 169, 178, 187,
    197, 206, 216, 226, 235, 245, 255,
];
// effort 1..9 -> libwebp method
static EFFORT_METHOD: [u8; 9] = [0, 1, 2, 3, 4, 5, 5, 6, 6];
// presets: sns, filter strength, sharpness, segments (default, picture, photo, drawing, icon, text)
static PRESETS: [[u8; 4]; 6] = [[50, 60, 0, 4], [80, 35, 4, 4], [80, 30, 3, 4], [25, 10, 6, 4], [0, 0, 0, 4], [0, 0, 0, 2]];
// level categories: first value of cat 0 (no extras), 1..6, sentinel
static CATBASE: [u16; 8] = [0, 5, 7, 11, 19, 35, 67, 2048];
// extra-bit probabilities per category and bit counts
static CATP: [[u8; 11]; 7] = [
    [0; 11],
    [159, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [165, 145, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [173, 148, 140, 0, 0, 0, 0, 0, 0, 0, 0],
    [176, 155, 140, 135, 0, 0, 0, 0, 0, 0, 0],
    [180, 157, 141, 134, 130, 0, 0, 0, 0, 0, 0],
    [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129],
];
static CATN: [u8; 7] = [0, 1, 2, 3, 4, 5, 11];
// I4 mode tree bits (node, bit) per mode, 0xff terminated (libwebp PutI4Mode)
static I4TREE: [[u8; 16]; 10] = [
    [0, 0, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 2, 0, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 0, 4, 0, 0xff, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 0, 4, 1, 5, 0, 0xff, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 0, 4, 1, 5, 1, 0xff, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 1, 6, 0, 0xff, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 1, 6, 1, 7, 0, 0xff, 0, 0, 0],
    [0, 1, 1, 1, 2, 1, 3, 1, 6, 1, 7, 1, 8, 0, 0xff, 0],
    [0, 1, 1, 1, 2, 1, 3, 1, 6, 1, 7, 1, 8, 1, 0xff, 0],
];
// VP8DspScan: 16 luma blocks, 4 U, 4 V (byte offsets in the BPS buffers)
static SCAN: [u16; 24] = [
    0, 4, 8, 12, 128, 132, 136, 140, 256, 260, 264, 268, 384, 388, 392, 396, 16, 20, 144, 148, 24, 28, 152, 156,
];
static WY: [i32; 16] = [38, 32, 20, 9, 32, 28, 17, 7, 20, 17, 10, 4, 9, 7, 4, 2];
static BANDS: [u8; 17] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 0];
// I4 boundary position of the top samples of each block
static I4TL: [u8; 16] = [17, 21, 25, 29, 13, 17, 21, 25, 9, 13, 17, 21, 5, 9, 13, 17];
static MDPOS: [u8; 3] = [1, 2, 4];
// level -> tree class (1, 2, 3, 4, 5-6, 7-10, 11-18, 19-34, 35-66, 67)
static LCLASS: [u8; 68] = [
    0, 0, 1, 2, 3, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9,
];
// tree bits (node, bit) after the non-zero bit, 0xff terminated
static LTREE: [[u8; 12]; 10] = [
    [2, 0, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 1, 3, 0, 4, 0, 0xff, 0, 0, 0, 0, 0],
    [2, 1, 3, 0, 4, 1, 5, 0, 0xff, 0, 0, 0],
    [2, 1, 3, 0, 4, 1, 5, 1, 0xff, 0, 0, 0],
    [2, 1, 3, 1, 6, 0, 7, 0, 0xff, 0, 0, 0],
    [2, 1, 3, 1, 6, 0, 7, 1, 0xff, 0, 0, 0],
    [2, 1, 3, 1, 6, 1, 8, 0, 9, 0, 0xff, 0],
    [2, 1, 3, 1, 6, 1, 8, 0, 9, 1, 0xff, 0],
    [2, 1, 3, 1, 6, 1, 8, 1, 10, 0, 0xff, 0],
    [2, 1, 3, 1, 6, 1, 8, 1, 10, 1, 0xff, 0],
];
// kBiasMatrices [luma-ac, luma-dc (y2), chroma][dc, ac]
static BIAS: [u8; 6] = [96, 110, 96, 108, 110, 115];
// kFreqSharpening
static SHARPEN: [u8; 16] = [0, 30, 60, 90, 30, 60, 90, 90, 60, 90, 90, 90, 90, 90, 90, 90];

// f64 constants of the quality curve (bit patterns of the assembly's .rodata)
const D_ONE: u64 = 0x3FF0_0000_0000_0000;
const D_HALF: u64 = 0x3FE0_0000_0000_0000;
const D_SQRT2: u64 = 0x3FF6_A09E_667F_3BCD;
const D_INVLN2: u64 = 0x3FF7_1547_652B_82FE;
const D_LN2: u64 = 0x3FE6_2E42_FEFA_39EF;
const D_100: u64 = 0x4059_0000_0000_0000;
const D_075: u64 = 0x3FE8_0000_0000_0000;
const D_23: u64 = 0x3FE5_5555_5555_5555;
const D_13: u64 = 0x3FD5_5555_5555_5555;
const D_127: u64 = 0x405F_C000_0000_0000;
const D_AMP: u64 = 0x3F12_6E97_8D4F_DF3C; // 0.9 / 100 / 128

#[inline(always)]
fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

// ---------------------------------------------------------------- records

/// quantisation matrix (MX_*)
#[derive(Clone, Copy, Default)]
struct Mtx {
    q: [u32; 16],
    iq: [u32; 16],
    bias: [u32; 16],
    zth: [u32; 16],
    sharp: [u32; 16],
}

/// segment (SG_*)
#[derive(Clone, Copy, Default)]
struct Seg {
    quant: u32,
    fstr: u32,
    alpha: i32,
    beta: i32,
    li4: u32,
    li16: u32,
    luv: u32,
    lmode: u32,
    tlambda: u32,
    mindis: u32,
    maxedge: u32,
    y1: Mtx,
    y2: Mtx,
    uv: Mtx,
}

/// rate-distortion score record (libwebp VP8ModeScore, RD_*); levels: Y2 (DC), 16 Y, 8 UV
#[derive(Clone, Copy)]
struct Rd {
    d: i64,
    sd: i64,
    h: i64,
    r: i64,
    score: i64,
    nz: u32,
    m16: u32,
    muv: u32,
    m4: [u8; 16],
    derr: [[i8; 3]; 2],
    lev: [[i16; 16]; 25],
}

const RD0: Rd = Rd {
    d: 0,
    sd: 0,
    h: 0,
    r: 0,
    score: 0,
    nz: 0,
    m16: 0,
    muv: 0,
    m4: [0; 16],
    derr: [[0; 3]; 2],
    lev: [[0; 16]; 25],
};

/// score of one I4 block (P4_B* / P4_T*)
#[derive(Clone, Copy, Default)]
struct Sc {
    d: i64,
    sd: i64,
    h: i64,
    r: i64,
    sc: i64,
    nz: u64,
}

/// macroblock info (MI_*)
#[derive(Clone, Copy, Default)]
struct Mi {
    ty: u8, // 1 = I16, 0 = I4
    uv: u8,
    seg: u8,
    alpha: u8,
    m16: u8,
    m4: [u8; 16], // I16: its mode 16 times
}

// ---------------------------------------------------------------- bool encoder

/// libwebp VP8BitWriter (VB_*; range kept as range - 1)
struct Vb {
    range: i32,
    value: i32,
    run: i32,
    nb: i32,
    buf: Vec<u8>,
}

impl Vb {
    // vb_init
    fn new(cap: usize) -> Vb {
        Vb { range: 254, value: 0, run: 0, nb: -8, buf: Vec::with_capacity(cap.max(4096)) }
    }

    // vb_put: libwebp VP8PutBit
    #[inline]
    fn put(&mut self, bit: u32, prob: u32) {
        let split = ((self.range as u32).wrapping_mul(prob) >> 8) as i32;
        if bit != 0 {
            self.value = self.value.wrapping_add(split + 1);
            self.range -= split + 1;
        } else {
            self.range = split;
        }
        if (self.range as u32) < 127 {
            // renormalise: shift = 7 - floor(log2(range + 1)), range = ((range + 1) << shift) - 1
            let r1 = (self.range + 1) as u32;
            let shift = 7 - (31 - r1.leading_zeros());
            self.range = ((r1 << shift) - 1) as i32;
            self.value = ((self.value as u32) << shift) as i32;
            self.nb += shift as i32;
            if self.nb > 0 {
                self.flush();
            }
        }
    }

    // vb_putu: probability 128
    #[inline]
    fn putu(&mut self, bit: u32) {
        self.put(bit, 128);
    }

    // vb_bits: n bits MSB first
    fn bits(&mut self, value: u32, n: u32) {
        let mut i = n;
        while i != 0 {
            i -= 1;
            self.putu((value >> i) & 1);
        }
    }

    // vb_signed: flag, magnitude (n bits), sign
    fn signed(&mut self, value: i32, n: u32) {
        self.putu((value != 0) as u32);
        if value != 0 {
            let v = if value < 0 { ((value.wrapping_neg() as u32) << 1) | 1 } else { (value as u32) << 1 };
            self.bits(v, n + 1);
        }
    }

    // vb_flush: libwebp Flush (a byte out, carry over pending 0xff bytes)
    fn flush(&mut self) {
        let s = (self.nb + 8) as u32;
        let bits = self.value >> s;
        self.value = self.value.wrapping_sub(bits.wrapping_shl(s));
        self.nb -= 8;
        if bits & 0xff != 0xff {
            let pos = self.buf.len();
            if bits & 0x100 != 0 && pos > 0 {
                self.buf[pos - 1] = self.buf[pos - 1].wrapping_add(1);
            }
            if self.run > 0 {
                let b = if bits & 0x100 != 0 { 0 } else { 0xff };
                for _ in 0..self.run {
                    self.buf.push(b);
                }
                self.run = 0;
            }
            self.buf.push(bits as u8);
        } else {
            self.run += 1;
        }
    }

    // vb_finish: 9 - nb zero bits, flush
    fn finish(&mut self) {
        self.bits(0, (9 - self.nb) as u32);
        self.nb = 0;
        self.flush();
    }
}

// ---------------------------------------------------------------- public entry points

/// VP8 key frame of sf (payload of a "VP8 " chunk; alpha is not coded); honours quality,
/// effort, preset, cancel  [vp8_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt) -> Result<Vec<u8>, i32> {
    let (px, w, h) = load_argb(sf)?;
    let cfg = cfg_from_opt(opt);
    enc_argb(&px, w, h, &cfg)
}

/// VP8 stream of the w x h straight BGRA pixels  [vp8_enc_argb]
pub fn enc_argb(px: &[u32], w: u32, h: u32, cfg: &Cfg) -> Result<Vec<u8>, i32> {
    if w.wrapping_sub(1) >= 16383 || h.wrapping_sub(1) >= 16383 {
        return Err(ERR_DIMS);
    }
    let mut ve = Ve::new(w, h, cfg);
    ve.rgb2yuv(px);
    ve.analyze()?;
    ve.segparams();
    ve.run()?;
    ve.assemble()
}

// ---------------------------------------------------------------- encoder context

struct Ve<'a> {
    w: usize,
    h: usize,
    mbw: usize,
    mbh: usize,
    q: u32,
    method: u32,
    sns: u32,
    fstr: u32,
    fsharp: u32,
    fsimple: u32,
    nseg: u32,
    flags: u32,
    cancel: Option<&'a AtomicU32>,
    yp: Vec<u8>, // planes padded to macroblocks
    up: Vec<u8>,
    vp: Vec<u8>,
    ys: usize,
    uvs: usize,
    mb: Vec<Mi>,
    ytop: Vec<u8>,     // reconstructed row above (mbw * 16)
    uvtop: Vec<u8>,    // u 8 | v 8 per column
    topnz: Vec<u8>,    // non-zero flags of the MB above (16 per MB, 9 used)
    topm: Vec<u8>,     // I4 modes of the row above
    topderr: Vec<i8>,  // UV DC diffusion errors of the row above
    alpha: u32,
    uvalpha: u32,
    baseq: u32,
    dquvdc: i32,
    dquvac: i32,
    maxi4hb: u32,
    level: u32,
    segupd: bool,
    segp: [u8; 4],
    maxcnt: i32,
    cnt: i32,
    sizep0: u64,
    x: usize,
    y: usize,
    doerr: bool,
    nsegf: u32,
    tok: Vec<u8>, // level store
    bw1: Vb,      // token partition
    leftnz: [u8; 16],
    leftm: [u8; 4],
    leftderr: [i8; 4],
    yleft: [u8; 17], // [0] = top-left, [1..16] = left column
    uleft: [u8; 9],
    vleft: [u8; 9],
    i4b: [u8; 48], // libwebp i4_boundary_
    tmp32: [u8; 32], // analysis top samples (Y 16, U 8, V 8)
    seg: [Seg; 4],
    proba: [u8; 1056],
    stats: Vec<[u32; 2]>, // (ones, total) per node
    lc: Vec<u16>,         // [4][8][3][68] level costs
    fixc: Vec<u16>,       // [2048] sign + extra-bit costs
    ec: [u16; 256],       // entropy cost of a 0 bit at probability p
    mc16: [u16; 4],
    mcuv: [u16; 4],
    mc4: [u16; 1000],
    wb: [u8; W_SIZE],
    rdm: Rd,         // macroblock result
    rdab: [Rd; 2],   // scratch A / B
    rdi4: Rd,        // I4 accumulation
    coef: [i16; 256], // luma coefficients
    dct: [i16; 16],   // WHT coefficients
    uvc: [i16; 128],
    ttsrc: [i32; 16], // TTransform of the source luma blocks
}

// ve_bitcost: cost (1/256 bit) of the bit with probability p (of a 0)
#[inline(always)]
fn bcost(ec: &[u16; 256], bit: u32, p: u32) -> u32 {
    if bit != 0 { ec[(255 - p) as usize & 255] as u32 } else { ec[p as usize & 255] as u32 }
}

// ve_log2fix: log2(v) * 2^24 (v >= 1; integer squaring)
fn log2fix(v: u32) -> u64 {
    let e = 31 - v.leading_zeros();
    let mut x = (v as u64) << (30 - e);
    let mut f = 0u32;
    for _ in 0..24 {
        x = (x * x) >> 30;
        f += f;
        if x & (1 << 31) != 0 {
            x >>= 1;
            f |= 1;
        }
    }
    ((e as u64) << 24) + f as u64
}

impl<'a> Ve<'a> {
    // vp8_enc_argb's setup: ve_config, ve_tables, ve_alloc
    fn new(w: u32, h: u32, cfg: &Cfg<'a>) -> Ve<'a> {
        let (w, h) = (w as usize, h as usize);
        let mbw = (w + 15) >> 4;
        let mbh = (h + 15) >> 4;
        let ys = mbw * 16;
        let luma = ys * mbh * 16;
        let row = mbw * 16 + 64;
        let mut ve = Ve {
            w,
            h,
            mbw,
            mbh,
            q: 0,
            method: 0,
            sns: 0,
            fstr: 0,
            fsharp: 0,
            fsimple: 0,
            nseg: 0,
            flags: 0,
            cancel: None,
            yp: vec![0; luma + 64],
            up: vec![0; luma / 4 + 64],
            vp: vec![0; luma / 4 + 64],
            ys,
            uvs: ys / 2,
            mb: vec![Mi::default(); mbw * mbh],
            ytop: vec![0; row],
            uvtop: vec![0; row],
            topnz: vec![0; row],
            topm: vec![0; row],
            topderr: vec![0; row],
            alpha: 0,
            uvalpha: 0,
            baseq: 0,
            dquvdc: 0,
            dquvac: 0,
            maxi4hb: 0,
            level: 0,
            segupd: false,
            segp: [255; 4],
            maxcnt: 0,
            cnt: 0,
            sizep0: 0,
            x: 0,
            y: 0,
            doerr: false,
            nsegf: 0,
            tok: Vec::new(),
            bw1: Vb::new(0),
            leftnz: [0; 16],
            leftm: [0; 4],
            leftderr: [0; 4],
            yleft: [0; 17],
            uleft: [0; 9],
            vleft: [0; 9],
            i4b: [0; 48],
            tmp32: [0; 32],
            seg: [Seg::default(); 4],
            proba: [0; 1056],
            stats: vec![[0; 2]; 1056],
            lc: vec![0; 96 * 68],
            fixc: vec![0; 2048],
            ec: [0; 256],
            mc16: [0; 4],
            mcuv: [0; 4],
            mc4: [0; 1000],
            wb: [0; W_SIZE],
            rdm: RD0,
            rdab: [RD0; 2],
            rdi4: RD0,
            coef: [0; 256],
            dct: [0; 16],
            uvc: [0; 128],
            ttsrc: [0; 16],
        };
        ve.config(cfg);
        ve.tables();
        ve
    }

    // ve_config: quality, method (effort 1..9 -> 0, 1, 2, 3, 4, 5, 5, 6, 6), preset parameters
    fn config(&mut self, cfg: &Cfg<'a>) {
        self.q = if cfg.quality == 0 { 95 } else { cfg.quality.min(100) };
        let e = if cfg.effort == 0 { 5 } else { cfg.effort.min(9) };
        self.method = EFFORT_METHOD[e as usize - 1] as u32;
        self.flags = cfg.flags;
        self.cancel = cfg.cancel;
        let p = if cfg.preset > WPP_TEXT { 0 } else { cfg.preset as usize };
        let pr = PRESETS[p];
        self.sns = pr[0] as u32;
        self.fstr = pr[1] as u32;
        self.fsharp = pr[2] as u32;
        self.nseg = pr[3] as u32;
        self.fsimple = 0;
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }

    // ve_tables: entropy costs, sign / extra-bit costs of the levels, mode costs
    fn tables(&mut self) {
        // EC[p] = round((8 - log2 p) * 256), EC[0] = EC[1]
        for p in 1..256u32 {
            let v = ((2048u64 << 24) - (log2fix(p) << 8) + (1 << 23)) >> 24;
            self.ec[p as usize] = v as u16;
        }
        self.ec[0] = self.ec[1];
        let ec = &self.ec;
        // fixed level costs
        self.fixc[0] = 0;
        for v in 1..2048u32 {
            let mut c = 256u32; // sign
            if v >= 5 {
                let mut cat = 0usize;
                while cat < 6 && v >= CATBASE[cat + 1] as u32 {
                    cat += 1;
                }
                let extra = v - CATBASE[cat] as u32;
                let nb = CATN[cat] as u32;
                for i in 0..nb {
                    let bit = (extra >> (nb - i - 1)) & 1;
                    c += bcost(ec, bit, CATP[cat][i as usize] as u32);
                }
            }
            self.fixc[v as usize] = c as u16;
        }
        // I16 mode costs: is-I16 bit (145) + tree 156 / 128 / 163
        for m in 0..4u32 {
            let mut c = bcost(ec, 1, 145);
            let tmhe = (m == 1 || m == 3) as u32;
            c += bcost(ec, tmhe, 156);
            c += if tmhe != 0 { bcost(ec, (m == 1) as u32, 128) } else { bcost(ec, (m == 2) as u32, 163) };
            self.mc16[m as usize] = c as u16;
        }
        // chroma mode costs: DC 142 / VE 114 / HE 183
        for m in 0..4u32 {
            let mut c = bcost(ec, (m != 0) as u32, 142);
            if m != 0 {
                c += bcost(ec, (m != 2) as u32, 114);
                if m != 2 {
                    c += bcost(ec, (m != 3) as u32, 183);
                }
            }
            self.mcuv[m as usize] = c as u16;
        }
        // I4 mode costs [top][left][mode] with kBModesProba
        for tl in 0..100usize {
            let pr = &BMODES[tl * 9..tl * 9 + 9];
            for m in 0..10usize {
                let tr = &I4TREE[m];
                let mut c = 0u32;
                let mut k = 0;
                while tr[k] != 0xff {
                    c += bcost(ec, tr[k + 1] as u32, pr[tr[k] as usize] as u32);
                    k += 2;
                }
                self.mc4[tl * 10 + m] = c as u16;
            }
        }
    }

    #[inline(always)]
    fn bc(&self, bit: u32, p: u32) -> u32 {
        bcost(&self.ec, bit, p)
    }

    // ------------------------------------------------------------ RGB -> YUV 4:2:0

    // ve_rgb2yuv: libwebp ImportYUVAFromRGBA (non-sharp); planes padded to whole
    // macroblocks by edge replication
    fn rgb2yuv(&mut self, px: &[u32]) {
        let (w, h, ys, uvs) = (self.w, self.h, self.ys, self.uvs);
        let alpha = px[..w * h].iter().any(|&p| p >> 24 != 255);
        // luma
        for y in 0..h {
            let src = &px[y * w..y * w + w];
            let row = &mut self.yp[y * ys..y * ys + ys];
            for (d, &p) in row[..w].iter_mut().zip(src) {
                let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
                *d = ((r * 16839 + g * 33059 + b * 6420 + (1 << 15) + (16 << 16)) >> 16) as u8;
            }
            let last = row[w - 1];
            row[w..].fill(last);
        }
        // chroma: two rows at a time
        let (cw, ch) = ((w + 1) >> 1, (h + 1) >> 1);
        for uy in 0..ch {
            let r0 = 2 * uy * w;
            let st = if 2 * uy + 1 < h { w } else { 0 };
            let o = uy * uvs;
            for ux in 0..cw {
                let step = if 2 * ux + 1 < w { 1 } else { 0 };
                let (u, v) = uvpix(px, r0 + 2 * ux, step, st, alpha);
                self.up[o + ux] = u;
                self.vp[o + ux] = v;
            }
            let (lu, lv) = (self.up[o + cw - 1], self.vp[o + cw - 1]);
            self.up[o + cw..o + uvs].fill(lu);
            self.vp[o + cw..o + uvs].fill(lv);
        }
        // NOEXACT: flatten / smooth fully or partly transparent blocks, then pad again
        if self.flags & WEF_NOEXACT != 0 && alpha {
            self.cleanup(px);
            hpad(&mut self.yp, ys, w, h);
            hpad(&mut self.up, uvs, cw, ch);
            hpad(&mut self.vp, uvs, cw, ch);
        }
        padrows(&mut self.yp, ys, h, self.mbh * 16);
        padrows(&mut self.up, uvs, ch, self.mbh * 8);
        padrows(&mut self.vp, uvs, ch, self.mbh * 8);
    }

    // ve_cleanup: libwebp WebPCleanupTransparentArea (lossy, WEF_NOEXACT): 8x8 luma blocks
    // without any visible pixel are flattened (Y / U / V) to the values of the first block of
    // their run, partly transparent blocks get the mean luma of their visible pixels.  The
    // bottom strip walks the columns like libwebp (the assembly's tail loop does not stop
    // when the width is not a multiple of 8)
    fn cleanup(&mut self, px: &[u32]) {
        let (w, h, ys, uvs) = (self.w, self.h, self.ys, self.uvs);
        let mut v = [0u8; 3];
        let mut y = 0;
        while y + 8 <= h {
            let mut reset = true;
            let mut x = 0;
            while x + 8 <= w {
                if self.smooth(px, x, y, 8, 8) {
                    let yo = y * ys + x;
                    let co = (y >> 1) * uvs + (x >> 1);
                    if reset {
                        v = [self.yp[yo], self.up[co], self.vp[co]];
                        reset = false;
                    }
                    for r in 0..8 {
                        self.yp[yo + r * ys..yo + r * ys + 8].fill(v[0]);
                    }
                    for r in 0..4 {
                        self.up[co + r * uvs..co + r * uvs + 4].fill(v[1]);
                        self.vp[co + r * uvs..co + r * uvs + 4].fill(v[2]);
                    }
                } else {
                    reset = true;
                }
                x += 8;
            }
            if w - x != 0 {
                self.smooth(px, x, y, w - x, 8);
            }
            y += 8;
        }
        let rows = h - y;
        if rows != 0 {
            let mut x = 0;
            while x < w {
                let bw = (w - x).min(8);
                self.smooth(px, x, y, bw, rows);
                x += 8;
            }
        }
    }

    // ve_smooth: libwebp SmoothenBlock on the luma plane -> true when no pixel is visible
    fn smooth(&mut self, px: &[u32], x: usize, y: usize, bw: usize, bh: usize) -> bool {
        let (w, ys) = (self.w, self.ys);
        let mut cnt = 0u32;
        let mut sum = 0u32;
        for r in 0..bh {
            let a = (y + r) * w + x;
            let l = (y + r) * ys + x;
            for c in 0..bw {
                if px[a + c] >> 24 != 0 {
                    cnt += 1;
                    sum += self.yp[l + c] as u32;
                }
            }
        }
        if cnt == 0 {
            return true;
        }
        if cnt >= (bw * bh) as u32 {
            return false;
        }
        let mean = (sum / cnt) as u8;
        for r in 0..bh {
            let a = (y + r) * w + x;
            let l = (y + r) * ys + x;
            for c in 0..bw {
                if px[a + c] >> 24 == 0 {
                    self.yp[l + c] = mean;
                }
            }
        }
        false
    }

    // ------------------------------------------------------------ helpers

    // ve_import: source macroblock (x, y) into W_IN
    fn import(&mut self) {
        let (ys, uvs) = (self.ys, self.uvs);
        let o = self.y * 16 * ys + self.x * 16;
        for r in 0..16 {
            self.wb[W_IN + r * BPS..W_IN + r * BPS + 16].copy_from_slice(&self.yp[o + r * ys..o + r * ys + 16]);
        }
        let o = self.y * 8 * uvs + self.x * 8;
        for r in 0..8 {
            let d = W_IN + r * BPS;
            self.wb[d + 16..d + 24].copy_from_slice(&self.up[o + r * uvs..o + r * uvs + 8]);
            self.wb[d + 24..d + 32].copy_from_slice(&self.vp[o + r * uvs..o + r * uvs + 8]);
        }
    }

    // ve_collect: libwebp GetAlpha of the DCT histogram of the blocks first..last-1
    // (VP8DspScan order) of W_IN - pred
    fn collect(&self, pred: usize, first: usize, last: usize) -> i32 {
        let mut dist = [0u32; 32];
        for b in first..last {
            let o = SCAN[b] as usize;
            let out = ftx(&self.wb, W_IN + o, pred + o);
            for &v in &out {
                let a = ((v as i32).unsigned_abs() >> 3).min(31);
                dist[a as usize] += 1;
            }
        }
        let mut maxv = 0u32;
        let mut lastnz = 1u32;
        for (k, &c) in dist.iter().enumerate() {
            if c != 0 {
                if c > maxv {
                    maxv = c;
                }
                lastnz = k as u32;
            }
        }
        if maxv > 1 { (lastnz * 510 / maxv) as i32 } else { 0 }
    }

    // ve_ttsrc: TTransform of the source luma blocks
    fn ttsrc(&mut self) {
        for i in 0..16 {
            self.ttsrc[i] = tt(&self.wb, W_IN + SCAN[i] as usize);
        }
    }

    // ve_disto2: libwebp Disto4x4 (|TT(b) - TT(source)| >> 5 with kWeightY) summed against the
    // cached source transforms: blocks 16 (b = 16x16 block) or 1 (b = the 4x4 block of
    // source block first)
    fn disto2(&self, b: usize, blocks: usize, first: usize) -> u32 {
        let mut sum = 0u32;
        for k in 0..blocks {
            let o = if blocks == 1 { 0 } else { SCAN[k] as usize };
            let t = tt(&self.wb, b + o);
            sum = sum.wrapping_add(t.wrapping_sub(self.ttsrc[first + k]).unsigned_abs() >> 5);
        }
        sum
    }

    // ve_rcost: libwebp GetResidualCost with the level cost tables
    fn rcost(&self, ctx0: usize, ty: usize, first: usize, lev: &[i16; 16]) -> u32 {
        let last = last(lev, first);
        let p0 = self.proba[(ty * 24 + first * 3 + ctx0) * 11] as u32;
        if last < 0 {
            return self.bc(0, p0); // EOB right away
        }
        let last = last as usize;
        let mut cost = if ctx0 == 0 { self.bc(1, p0) } else { 0 };
        let mut ctx = ctx0;
        let mut n = first;
        loop {
            let t = (ty * 24 + BANDS[n] as usize * 3 + ctx) * 68;
            let v = (lev[n] as i32).unsigned_abs() as usize & 2047;
            cost += self.fixc[v] as u32;
            cost += self.lc[t + v.min(MAX_VAR)] as u32;
            if n >= last {
                if last < 15 {
                    // EOB after the last level
                    let c = if v == 1 { 1 } else { 2 };
                    let b = BANDS[last + 1] as usize;
                    cost += self.bc(0, self.proba[(ty * 24 + b * 3 + c) * 11] as u32);
                }
                return cost;
            }
            ctx = v.min(2);
            n += 1;
        }
    }

    // ve_cost16: libwebp VP8GetCostLuma16 (contexts from the neighbours)
    fn cost16(&self, lev: &[[i16; 16]; 25]) -> u32 {
        let t0 = self.x * 16;
        let mut ct = [0u8; 16];
        ct.copy_from_slice(&self.topnz[t0..t0 + 16]);
        let mut cl = self.leftnz;
        let mut c = self.rcost((ct[8] + cl[8]) as usize, 1, 0, &lev[0]);
        for b in 0..16 {
            let ctx = (ct[b & 3] + cl[b >> 2]) as usize;
            c += self.rcost(ctx, 0, 1, &lev[1 + b]);
            let nz = (last(&lev[1 + b], 1) >= 0) as u8;
            ct[b & 3] = nz;
            cl[b >> 2] = nz;
        }
        c
    }

    // ve_costuv: libwebp VP8GetCostUV
    fn costuv(&self, lev: &[[i16; 16]; 25]) -> u32 {
        let t0 = self.x * 16;
        let mut ct = [0u8; 16];
        ct.copy_from_slice(&self.topnz[t0..t0 + 16]);
        let mut cl = self.leftnz;
        let mut c = 0;
        for b in 0..8 {
            let base = 4 + 2 * (b >> 2);
            let ti = base + (b & 1);
            let li = base + ((b >> 1) & 1);
            c += self.rcost((ct[ti] + cl[li]) as usize, 2, 0, &lev[17 + b]);
            let nz = (last(&lev[17 + b], 0) >= 0) as u8;
            ct[ti] = nz;
            cl[li] = nz;
        }
        c
    }

    // ve_seg: segment of the current macroblock
    #[inline]
    fn segi(&self) -> usize {
        self.mb[self.y * self.mbw + self.x].seg as usize & 3
    }

    // ------------------------------------------------------------ mode decisions

    // ve_recon16: libwebp ReconstructIntra16 into dst: 16 DCTs, WHT of the DCs quantised
    // with y2, ACs with y1, inverse transforms -> nz
    fn recon16(&mut self, ri: usize, mode: usize, dst: usize) -> u32 {
        let s = self.segi();
        let pred = W_P16 + mode * 512;
        for b in 0..16 {
            let o = SCAN[b] as usize;
            let c = ftx(&self.wb, W_IN + o, pred + o);
            self.coef[b * 16..b * 16 + 16].copy_from_slice(&c);
        }
        self.dct = fwht(&self.coef);
        let sg = &self.seg[s];
        let rd = &mut self.rdab[ri];
        let mut nz = quant(&mut self.dct, &mut rd.lev[0], &sg.y2) << 24;
        for b in 0..16 {
            self.coef[b * 16] = 0;
            nz |= quant(&mut self.coef[b * 16..b * 16 + 16], &mut rd.lev[1 + b], &sg.y1) << b;
        }
        wht(&self.dct, &mut self.coef);
        for b in 0..16 {
            let o = SCAN[b] as usize;
            itx_ref(&mut self.wb, pred + o, &self.coef[b * 16..b * 16 + 16], dst + o);
        }
        nz
    }

    // ve_reconuv: libwebp ReconstructUV (UV DC error diffusion when doerr) -> nz << 16
    fn reconuv(&mut self, ri: usize, mode: usize, dst: usize) -> u32 {
        let s = self.segi();
        let pred = W_PUV + mode * 512;
        for b in 0..8 {
            let o = SCAN[16 + b] as usize;
            let c = ftx(&self.wb, W_IN + o, pred + o);
            self.uvc[b * 16..b * 16 + 16].copy_from_slice(&c);
        }
        if self.doerr {
            self.correctdc(ri, s);
        }
        let sg = &self.seg[s];
        let rd = &mut self.rdab[ri];
        let mut nz = 0;
        for b in 0..8 {
            nz |= quant(&mut self.uvc[b * 16..b * 16 + 16], &mut rd.lev[17 + b], &sg.uv) << b;
        }
        for b in 0..8 {
            let o = SCAN[16 + b] as usize;
            itx_ref(&mut self.wb, pred + o, &self.uvc[b * 16..b * 16 + 16], dst + o);
        }
        nz << 16
    }

    // ve_correctdc: libwebp CorrectDCValues: diffuse the DC quantisation error of the U / V
    // blocks (7 below, 8 right, >> 3); errors to rd.derr
    fn correctdc(&mut self, ri: usize, s: usize) {
        let m = &self.seg[s].uv;
        for ch in 0..2 {
            let t = self.x * 4 + ch * 2;
            let (t0, t1) = (self.topderr[t] as i32, self.topderr[t + 1] as i32);
            let (l0, l1) = (self.leftderr[ch * 2] as i32, self.leftderr[ch * 2 + 1] as i32);
            let c = &mut self.uvc[ch * 64..ch * 64 + 64];
            c[0] = c[0].wrapping_add(((t0 * 7 + l0 * 8) >> 3) as i16);
            let e0 = qsingle(&mut c[0], m);
            c[16] = c[16].wrapping_add(((t1 * 7 + e0 * 8) >> 3) as i16);
            let e1 = qsingle(&mut c[16], m);
            c[32] = c[32].wrapping_add(((e0 * 7 + l1 * 8) >> 3) as i16);
            let e2 = qsingle(&mut c[32], m);
            c[48] = c[48].wrapping_add(((e1 * 7 + e2 * 8) >> 3) as i16);
            let e3 = qsingle(&mut c[48], m);
            self.rdab[ri].derr[ch] = [e1 as i8, e2 as i8, e3 as i8];
        }
    }

    // ve_pick16: libwebp PickBestIntra16 into rdm (reconstruction in W_OUT)
    fn pick16(&mut self) {
        let s = self.segi();
        let (tl, li16, lmode, mindis) = {
            let sg = &self.seg[s];
            (sg.tlambda, sg.li16, sg.lmode, sg.mindis)
        };
        let mut flat = isflatsrc(&self.wb);
        let (mut cur, mut best) = (0usize, 1usize);
        for mode in 0..4 {
            self.rdab[cur].m16 = mode as u32;
            let nz = self.recon16(cur, mode, W_OUT2);
            let dd = sse(&self.wb, W_IN, W_OUT2, 16, 16);
            let sd = if tl != 0 { self.disto2(W_OUT2, 16, 0).wrapping_mul(tl).wrapping_add(128) >> 8 } else { 0 };
            let r = self.cost16(&self.rdab[cur].lev);
            let rd = &mut self.rdab[cur];
            rd.nz = nz;
            rd.d = dd as i64;
            rd.sd = sd as i64;
            rd.h = self.mc16[mode] as i64;
            rd.r = r as i64;
            if flat {
                flat = isflat(&rd.lev[1..17], 0);
                if flat {
                    rd.d *= 2;
                    rd.sd *= 2;
                }
            }
            setscore(rd, li16);
            if mode == 0 || rd.score < self.rdab[best].score {
                core::mem::swap(&mut cur, &mut best);
                self.swapout();
            }
        }
        // the best into rdm, final score with lambda_mode
        self.rdm = self.rdab[best];
        setscore(&mut self.rdm, lmode);
        // blocky macroblock (DC only) with high distortion: record the max DC delta
        if self.rdm.nz & 0x100_ffff == 0x100_0000 && self.rdm.d > mindis as i64 {
            let dc = &self.rdm.lev[0];
            let mut m = 0u32;
            for &p in &MDPOS {
                m = m.max((dc[p as usize] as i32).unsigned_abs());
            }
            let sg = &mut self.seg[s];
            if m > sg.maxedge {
                sg.maxedge = m;
            }
        }
    }

    // ve_swapout: exchange W_OUT and W_OUT2
    fn swapout(&mut self) {
        let (a, b) = self.wb[W_OUT..W_OUT2 + 512].split_at_mut(512);
        a.swap_with_slice(b);
    }

    // ve_pickuv: libwebp PickBestUV: chroma mode into rdm (score added), the reconstruction
    // into W_OUT columns 16..31
    fn pickuv(&mut self) {
        let s = self.segi();
        let luv = self.seg[s].luv;
        let (mut cur, mut best) = (0usize, 1usize);
        for mode in 0..4 {
            let nz = self.reconuv(cur, mode, W_OUT2);
            let dd = sse(&self.wb, W_IN + 16, W_OUT2 + 16, 16, 8);
            let r = self.costuv(&self.rdab[cur].lev);
            let rd = &mut self.rdab[cur];
            rd.nz = nz;
            rd.d = dd as i64;
            rd.sd = 0;
            rd.h = self.mcuv[mode] as i64;
            rd.r = r as i64;
            if mode > 0 && isflat(&rd.lev[17..25], 2) {
                rd.r += FLAT_PEN * 8;
            }
            setscore(rd, luv);
            if mode == 0 || rd.score < self.rdab[best].score {
                core::mem::swap(&mut cur, &mut best);
                self.rdab[best].muv = mode as u32;
                // keep the chroma reconstruction in W_OUT
                for r in 0..8 {
                    let o = r * BPS + 16;
                    self.wb.copy_within(W_OUT2 + o..W_OUT2 + o + 16, W_OUT + o);
                }
            }
        }
        // add the chroma score and levels to rdm
        let b = &self.rdab[best];
        let m = &mut self.rdm;
        m.muv = b.muv;
        m.lev[17..25].copy_from_slice(&b.lev[17..25]);
        m.derr = b.derr;
        m.d = m.d.wrapping_add(b.d);
        m.sd = m.sd.wrapping_add(b.sd);
        m.h = m.h.wrapping_add(b.h);
        m.r = m.r.wrapping_add(b.r);
        m.score = m.score.wrapping_add(b.score);
        m.nz |= b.nz;
    }

    // ve_pick4: libwebp PickBestIntra4 -> true when I4 beats the I16 choice in rdm (then
    // rdm / W_OUT hold the I4 result)
    fn pick4(&mut self) -> bool {
        if self.maxi4hb == 0 {
            return false;
        }
        let s = self.segi();
        let (tl, li4, lmode) = {
            let sg = &self.seg[s];
            (sg.tlambda, sg.li4, sg.lmode)
        };
        // rd_best: H = cost of the I4 flag, score with lambda_mode
        {
            let h = self.bc(0, 145) as i64;
            let r = &mut self.rdi4;
            r.d = 0;
            r.sd = 0;
            r.r = 0;
            r.nz = 0;
            r.m16 = 0;
            r.muv = 0;
            r.m4 = [0; 16];
            r.derr = [[0; 3]; 2];
            r.lev[0] = [0; 16];
            r.h = h;
            setscore(r, lmode);
        }
        let mut hdr = 0u32;
        // ve_starti4: boundary from the left column (reversed, then the top-left) and the top
        // row (+ top-right), first block, working contexts
        let x = self.x;
        for i in 0..17 {
            self.i4b[i] = self.yleft[16 - i];
        }
        self.i4b[17..33].copy_from_slice(&self.ytop[x * 16..x * 16 + 16]);
        if x + 1 < self.mbw {
            self.i4b[33..37].copy_from_slice(&self.ytop[x * 16 + 16..x * 16 + 20]);
        } else {
            let v = self.ytop[x * 16 + 15];
            self.i4b[33..37].fill(v);
        }
        let mut top = 17usize;
        let mut ct = [0u8; 16];
        ct.copy_from_slice(&self.topnz[x * 16..x * 16 + 16]);
        let mut cl = self.leftnz;
        for i4 in 0..16 {
            let o = SCAN[i4] as usize;
            // mode costs [top][left]
            let tm = if i4 < 4 { self.topm[x * 4 + i4] } else { self.rdi4.m4[i4 - 4] } as usize;
            let lm = if i4 & 3 == 0 { self.leftm[i4 >> 2] } else { self.rdi4.m4[i4 - 1] } as usize;
            let mc = (tm * 10 + lm) * 10;
            pred4(&mut self.wb, &self.i4b, top);
            let mut bestm = -1i32;
            let mut b = Sc::default();
            for mode in 0..10 {
                // reconstruct into T4A
                let pred = W_P4 + mode * 128;
                let mut c = ftx(&self.wb, W_IN + o, pred);
                let mut lev = [0i16; 16];
                let q = quant(&mut c, &mut lev, &self.seg[s].y1);
                let mut t = Sc { nz: (q << i4) as u64, ..Sc::default() };
                itx_ref(&mut self.wb, pred, &c, W_T4A);
                t.d = sse(&self.wb, W_IN + o, W_T4A, 4, 4) as i64;
                t.sd = if tl != 0 { (self.disto2(W_T4A, 1, i4).wrapping_mul(tl).wrapping_add(128) >> 8) as i64 } else { 0 };
                t.h = self.mc4[mc + mode] as i64;
                t.r = if mode != 0 { isflat(core::slice::from_ref(&lev), 3) as i64 * FLAT_PEN } else { 0 };
                setscore_sc(&mut t, li4);
                if bestm >= 0 && t.sc >= b.sc {
                    continue;
                }
                // + token cost
                t.r += self.rcost((ct[i4 & 3] + cl[i4 >> 2]) as usize, 3, 0, &lev) as i64;
                setscore_sc(&mut t, li4);
                if bestm >= 0 && t.sc >= b.sc {
                    continue;
                }
                // new best mode of the block
                b = t;
                bestm = mode as i32;
                self.rdi4.lev[1 + i4] = lev;
                for r in 0..4 {
                    self.wb.copy_within(W_T4A + r * BPS..W_T4A + r * BPS + 4, W_T4B + r * BPS);
                }
            }
            // block score with lambda_mode, add to rd_best
            setscore_sc(&mut b, lmode);
            let r = &mut self.rdi4;
            r.d = r.d.wrapping_add(b.d);
            r.sd = r.sd.wrapping_add(b.sd);
            r.h = r.h.wrapping_add(b.h);
            r.r = r.r.wrapping_add(b.r);
            r.score = r.score.wrapping_add(b.sc);
            r.nz |= b.nz as u32;
            if r.score >= self.rdm.score {
                return false;
            }
            hdr = hdr.wrapping_add(b.h as u32);
            if hdr > self.maxi4hb {
                return false;
            }
            // chosen block into OUT2, mode, contexts, boundary rotation
            for r in 0..4 {
                self.wb.copy_within(W_T4B + r * BPS..W_T4B + r * BPS + 4, W_OUT2 + o + r * BPS);
            }
            self.rdi4.m4[i4] = bestm as u8;
            let nz = (b.nz != 0) as u8;
            ct[i4 & 3] = nz;
            cl[i4 >> 2] = nz;
            // ve_rotatei4: libwebp VP8IteratorRotateI4 with the chosen block in OUT2
            let blk = W_OUT2 + o;
            self.i4b[top - 4..top].copy_from_slice(&self.wb[blk + 3 * BPS..blk + 3 * BPS + 4]);
            if i4 & 3 != 3 {
                self.i4b[top] = self.wb[blk + 3 + 2 * BPS];
                self.i4b[top + 1] = self.wb[blk + 3 + BPS];
                self.i4b[top + 2] = self.wb[blk + 3];
            } else {
                self.i4b.copy_within(top + 4..top + 8, top); // replicate the top-right samples
            }
            if i4 + 1 < 16 {
                top = I4TL[i4 + 1] as usize;
            }
        }
        // I4 wins: rdm gets the I4 score and levels (the DC levels are unused)
        let r = &self.rdi4;
        let m = &mut self.rdm;
        m.d = r.d;
        m.sd = r.sd;
        m.h = r.h;
        m.r = r.r;
        m.score = r.score;
        m.nz = r.nz;
        m.m4 = r.m4;
        m.lev[1..17].copy_from_slice(&r.lev[1..17]);
        self.swapout();
        true
    }

    // ------------------------------------------------------------ analysis / segments

    // ve_analyze: libwebp VP8EncAnalyze: per macroblock the susceptibility alpha (DCT histogram
    // of the best I16 and chroma predictions, source samples as context), k-means of the alpha
    // histogram into the segments, the average alphas; one segment: everything 0
    fn analyze(&mut self) -> Result<(), i32> {
        let mut hist = [0u32; 256];
        let mut sum = 0u64;
        let mut uvsum = 0u64;
        self.alpha = 0;
        self.uvalpha = 0;
        if self.nseg <= 1 {
            return Ok(());
        }
        for y in 0..self.mbh {
            self.y = y;
            if self.cancelled() {
                return Err(ERR_CANCEL);
            }
            for x in 0..self.mbw {
                self.x = x;
                self.import();
                self.srcctx();
                // I16: best alpha of the four predictions
                pred16(
                    &mut self.wb,
                    if x > 0 { Some(&self.yleft[..]) } else { None },
                    if y > 0 { Some(&self.tmp32[..16]) } else { None },
                );
                let mut best = -1i32;
                for m in 0..4 {
                    let a = self.collect(W_P16 + m * 512, 0, 16);
                    if a > best {
                        best = a;
                    }
                }
                // chroma: best (largest) alpha of the four predictions
                preduv(
                    &mut self.wb,
                    if x > 0 { Some(&self.uleft[..]) } else { None },
                    if x > 0 { Some(&self.vleft[..]) } else { None },
                    if y > 0 { Some(&self.tmp32[16..24]) } else { None },
                    if y > 0 { Some(&self.tmp32[24..32]) } else { None },
                );
                let mut buv = -1i32;
                for m in 0..4 {
                    let a = self.collect(W_PUV + m * 512, 16, 24);
                    if a > buv {
                        buv = a;
                    }
                }
                // mix: alpha = 255 - ((3 best + uv + 2) >> 2), clipped
                let a = (255 - ((best * 3 + buv + 2) >> 2)).clamp(0, 255) as usize;
                hist[a] += 1;
                sum += a as u64;
                uvsum += buv as u32 as u64;
                self.mb[y * self.mbw + x].alpha = a as u8;
            }
        }
        self.kmeans(&hist);
        let n = (self.mbw * self.mbh) as u64;
        self.alpha = (sum / n) as u32;
        self.uvalpha = (uvsum / n) as u32;
        Ok(())
    }

    // ve_srcctx: analysis context from the source planes: yleft / uleft / vleft ([0] =
    // top-left: 127 in row 0) for x > 0, tmp32 (Y 16, U 8, V 8) for y > 0
    fn srcctx(&mut self) {
        let (x, y, ys, uvs) = (self.x, self.y, self.ys, self.uvs);
        if x > 0 {
            let base = y * 16 * ys + x * 16 - 1;
            for i in 0..16 {
                self.yleft[1 + i] = self.yp[base + i * ys];
            }
            self.yleft[0] = if y > 0 { self.yp[(y * 16 - 1) * ys + x * 16 - 1] } else { 127 };
            let base = y * 8 * uvs + x * 8 - 1;
            for i in 0..8 {
                self.uleft[1 + i] = self.up[base + i * uvs];
                self.vleft[1 + i] = self.vp[base + i * uvs];
            }
            if y > 0 {
                let tl = (y * 8 - 1) * uvs + x * 8 - 1;
                self.uleft[0] = self.up[tl];
                self.vleft[0] = self.vp[tl];
            } else {
                self.uleft[0] = 127;
                self.vleft[0] = 127;
            }
        }
        if y > 0 {
            let o = (y * 16 - 1) * ys + x * 16;
            self.tmp32[..16].copy_from_slice(&self.yp[o..o + 16]);
            let o = (y * 8 - 1) * uvs + x * 8;
            self.tmp32[16..24].copy_from_slice(&self.up[o..o + 8]);
            self.tmp32[24..32].copy_from_slice(&self.vp[o..o + 8]);
        }
    }

    // ve_kmeans: libwebp AssignSegments: k-means (6 rounds at most) of the alpha histogram
    // into nseg centres, map of every macroblock, SetSegmentAlphas
    fn kmeans(&mut self, hist: &[u32; 256]) {
        let nb = self.nseg as usize; // 2..4
        // bracket
        let mut mn = 0usize;
        while mn < 255 && hist[mn] == 0 {
            mn += 1;
        }
        let mut mx = 255usize;
        while mx > mn && hist[mx] == 0 {
            mx -= 1;
        }
        let range = (mx - mn) as i32;
        // initial centres min + (2k + 1) range / (2 nb)
        let mut cent = [0i32; 4];
        for k in 0..nb {
            cent[k] = (2 * k as i32 + 1) * range / (2 * nb as i32) + mn as i32;
        }
        let mut map = [0u8; 256];
        let mut wavg = 0u32;
        for _ in 0..6 {
            let mut acc = [0u32; 4];
            let mut dist = [0u32; 4];
            let mut n = 0usize; // nearest centre
            for a in mn..=mx {
                let c = hist[a];
                if c == 0 {
                    continue;
                }
                while n + 1 < nb && (a as i32 - cent[n + 1]).unsigned_abs() < (a as i32 - cent[n]).unsigned_abs() {
                    n += 1;
                }
                map[a] = n as u8;
                dist[n] = dist[n].wrapping_add((a as u32).wrapping_mul(c));
                acc[n] = acc[n].wrapping_add(c);
            }
            let mut displaced = 0u32;
            let mut wsum = 0u32;
            let mut wtot = 0u32;
            for k in 0..nb {
                let c = acc[k];
                if c != 0 {
                    let nc = ((c >> 1) + dist[k]) / c;
                    displaced = displaced.wrapping_add(cent[k].wrapping_sub(nc as i32).unsigned_abs());
                    cent[k] = nc as i32;
                    wsum = wsum.wrapping_add(nc.wrapping_mul(c));
                    wtot = wtot.wrapping_add(c);
                }
            }
            wavg = ((wtot >> 1) + wsum) / wtot;
            if displaced < 5 {
                break;
            }
        }
        // every macroblock: segment = map[alpha], alpha = its centre
        for m in self.mb.iter_mut() {
            let s = map[m.alpha as usize];
            m.seg = s;
            m.alpha = cent[s as usize & 3] as u8;
        }
        // SetSegmentAlphas
        let mut lo = cent[0];
        let mut hi = cent[0];
        for &c in &cent[..nb] {
            lo = lo.min(c);
            hi = hi.max(c);
        }
        if lo == hi {
            hi = lo + 1;
        }
        let span = hi - lo;
        for k in 0..nb {
            self.seg[k].alpha = ((cent[k] - wavg as i32) * 255 / span).clamp(-127, 127);
            self.seg[k].beta = ((cent[k] - lo) * 255 / span).clamp(0, 255);
        }
    }

    // ve_segparams: libwebp VP8SetSegmentParams: quantisers from the quality curve and the
    // segment alphas, dq_uv from the chroma alpha, filter strengths, SimplifySegments,
    // SetupMatrices, segment probabilities
    fn segparams(&mut self) {
        // c_base = (Q < 0.75 ? 2Q / 3 : 2Q - 1)^(1/3)
        let mut qd = self.q as i32 as f64 / d(D_100);
        qd = if qd >= d(D_075) { qd + qd - d(D_ONE) } else { qd * d(D_23) };
        let cbase = pow(qd, d(D_13));
        let amp = self.sns as i32 as f64 * d(D_AMP);
        let nseg = self.nseg as usize;
        for s in 0..nseg {
            let x = self.seg[s].alpha as f64 * amp;
            let expn = d(D_ONE) - x;
            let c = pow(cbase, expn);
            let v = ((d(D_ONE) - c) * d(D_127)) as i32;
            self.seg[s].quant = v.clamp(0, 127) as u32;
        }
        let q0 = self.seg[0].quant;
        self.baseq = q0;
        for s in nseg..4 {
            self.seg[s].quant = q0;
        }
        // dq_uv_ac = clip((uv_alpha - 64) * 10 / 70 * sns / 100, -4, 6), dq_uv_dc = -4 sns / 100
        let sns = self.sns as i32;
        let v = (self.uvalpha as i32).wrapping_sub(64).wrapping_mul(10) / 70;
        self.dquvac = (v.wrapping_mul(sns) / 100).clamp(-4, 6);
        self.dquvdc = (sns * -4 / 100).clamp(-15, 15);
        // filter strengths: f = FromDelta(sharpness, kAcTable[q] >> 2) * 5 fstr / (256 + beta)
        for s in 0..4 {
            let sg = &mut self.seg[s];
            let dl = ACTAB[sg.quant as usize & 127] as u32 >> 2;
            let f = fstrength(self.fsharp, dl) as i32;
            let f = f * self.fstr as i32 * 5 / (sg.beta + 256);
            sg.fstr = if f < 2 { 0 } else { f.min(63) as u32 };
        }
        self.level = self.seg[0].fstr;
        self.nsegf = self.nseg;
        if self.nseg > 1 {
            self.simplify();
        }
        for s in 0..4 {
            self.setupseg(s);
        }
        self.segprobas();
    }

    // ve_simplify: libwebp SimplifySegments: merge segments with equal quantiser and filter
    // strength, remap the macroblocks, replicate the last record
    fn simplify(&mut self) {
        let mut map = [0u8, 1, 2, 3];
        let mut nf = 1usize;
        let nseg = self.nseg as usize;
        for s1 in 1..nseg {
            let mut found = None;
            for s2 in 0..nf {
                if self.seg[s1].quant == self.seg[s2].quant && self.seg[s1].fstr == self.seg[s2].fstr {
                    found = Some(s2);
                    break;
                }
            }
            match found {
                Some(s2) => map[s1] = s2 as u8,
                None => {
                    map[s1] = nf as u8;
                    if nf != s1 {
                        self.seg[nf] = self.seg[s1];
                    }
                    nf += 1;
                }
            }
        }
        if nf < nseg {
            for m in self.mb.iter_mut() {
                m.seg = map[m.seg as usize & 3];
            }
            self.nsegf = nf as u32;
            for s in nf..4 {
                self.seg[s] = self.seg[nf - 1];
            }
        }
    }

    // ve_setupseg: libwebp SetupMatrices for segment s
    fn setupseg(&mut self, s: usize) {
        let (method, sns, dqdc, dqac) = (self.method, self.sns, self.dquvdc, self.dquvac);
        let sg = &mut self.seg[s];
        let q = sg.quant as usize & 127;
        // y1: dc = kDcTable[q], ac = kAcTable[q]
        let qi4 = expand(&mut sg.y1, DCTAB[q] as u32, ACTAB[q] as u32, 0);
        // y2: dc = 2 kDcTable[q], ac = kAcTable[q] * 101581 >> 16 (>= 8)
        let ac2 = ((ACTAB[q] as u32 * 101581) >> 16).max(8);
        let qi16 = expand(&mut sg.y2, DCTAB[q] as u32 * 2, ac2, 1);
        // uv: dc = kDcTable[clip(q + dq_uv_dc, 0, 117)], ac = kAcTable[clip(q + dq_uv_ac)]
        let qdc = (q as i32 + dqdc).clamp(0, 117) as usize;
        let qac = (q as i32 + dqac).clamp(0, 127) as usize;
        let quv = expand(&mut sg.uv, DCTAB[qdc] as u32, ACTAB[qac] as u32, 2);
        // lambdas
        sg.li4 = ((qi4 * qi4 * 3) >> 7).max(1);
        sg.lmode = ((qi4 * qi4) >> 7).max(1);
        sg.li16 = (qi16 * qi16 * 3).max(1);
        sg.luv = ((quv * quv * 3) >> 6).max(1);
        // tlambda = (method >= 4 ? sns : 0) * q_i4 >> 5
        sg.tlambda = if method >= 4 { (sns * qi4) >> 5 } else { 0 };
        sg.mindis = sg.y1.q[0] * 20;
        sg.maxedge = 0;
    }

    // ve_segprobas: libwebp SetSegmentProbas: tree probabilities of the segment map
    // (update_map only when one of them is not 255); no map: every macroblock segment 0
    fn segprobas(&mut self) {
        self.segupd = false;
        self.segp = [255; 4];
        if self.nsegf > 1 {
            let mut c = [0u32; 4];
            for m in &self.mb {
                c[(m.seg & 3) as usize] += 1;
            }
            self.segp[0] = getp(c[0] + c[1], c[2] + c[3]);
            self.segp[1] = getp(c[0], c[1]);
            self.segp[2] = getp(c[2], c[3]);
            self.segupd = self.segp[..3] != [255; 3];
        }
        if !self.segupd {
            for m in self.mb.iter_mut() {
                m.seg = 0;
            }
        }
    }

    // ------------------------------------------------------------ main loop

    // ve_loop: the token pass (libwebp VP8EncTokenLoop, one pass): every macroblock:
    // predictions, rate-distortion decisions, token statistics, levels stored for the
    // emission; probabilities refreshed every maxcnt macroblocks; the pass is repeated with a
    // halved I4 header budget while partition 0 would overflow
    fn run(&mut self) -> Result<(), i32> {
        self.maxcnt = ((self.mbw * self.mbh) as u32 >> 3).max(96) as i32;
        self.doerr = self.q <= 98;
        self.maxi4hb = if self.method >= 2 { 65536 } else { 0 };
        loop {
            self.passinit();
            let mut mbi = 0;
            for y in 0..self.mbh {
                self.y = y;
                if self.cancelled() {
                    return Err(ERR_CANCEL);
                }
                self.initleft();
                for x in 0..self.mbw {
                    self.x = x;
                    self.cnt -= 1;
                    if self.cnt < 0 {
                        self.finalprobas();
                        self.levelcosts();
                        self.cnt = self.maxcnt;
                    }
                    self.import();
                    self.ttsrc();
                    self.mbpreds();
                    self.pick16();
                    let i4 = self.method >= 2 && self.pick4();
                    self.pickuv();
                    // macroblock info
                    let rm = &self.rdm;
                    let mi = &mut self.mb[mbi];
                    mi.ty = (!i4) as u8;
                    mi.uv = rm.muv as u8;
                    if !i4 {
                        mi.m16 = rm.m16 as u8;
                        mi.m4 = [rm.m16 as u8; 16];
                    } else {
                        mi.m4 = rm.m4;
                    }
                    self.sizep0 = self.sizep0.wrapping_add(rm.h as u64);
                    // UV DC diffusion errors for the next blocks
                    if self.doerr {
                        self.storederr();
                    }
                    // tokens: statistics + contexts, levels kept for the emission
                    let lev = self.rdm.lev;
                    self.tokens(!i4, &lev, false);
                    self.storelev(!i4, &lev);
                    self.savectx(mbi);
                    mbi += 1;
                }
            }
            // partition 0 limit (libwebp PARTITION0_SIZE_LIMIT): halve the I4 budget, again
            if self.sizep0 > (524288 - 2048) << 11 && self.maxi4hb != 0 {
                self.maxi4hb >>= 1;
                continue;
            }
            break;
        }
        self.finalprobas();
        self.adjustfilter();
        Ok(())
    }

    // ve_passinit: default probabilities, empty statistics, level costs, row contexts (top
    // 127, no coefficients, DC modes), empty level store
    fn passinit(&mut self) {
        self.proba.copy_from_slice(&PROBA0);
        self.stats.fill([0; 2]);
        self.levelcosts();
        self.cnt = self.maxcnt;
        self.sizep0 = 0;
        self.tok.clear();
        let n = self.mbw * 16 + 32;
        self.ytop[..n].fill(127);
        self.uvtop[..n].fill(127);
        self.topnz[..n].fill(0);
        self.topm[..n].fill(0);
        self.topderr[..n].fill(0);
        for s in self.seg.iter_mut() {
            s.maxedge = 0;
        }
    }

    // ve_initleft: libwebp InitLeft: left samples 129 (top-left 127 in row 0, else 129), no
    // coefficients / DC modes / diffusion errors on the left
    fn initleft(&mut self) {
        let v = if self.y == 0 { 127 } else { 129 };
        self.yleft[0] = v;
        self.uleft[0] = v;
        self.vleft[0] = v;
        self.yleft[1..].fill(129);
        self.uleft[1..].fill(129);
        self.vleft[1..].fill(129);
        self.leftnz = [0; 16];
        self.leftm = [0; 4];
        self.leftderr = [0; 4];
    }

    // ve_mbpreds: the I16 and chroma predictions (no left in column 0, no top in row 0)
    fn mbpreds(&mut self) {
        let (x, y) = (self.x, self.y);
        let t = x * 16;
        pred16(
            &mut self.wb,
            if x > 0 { Some(&self.yleft[..]) } else { None },
            if y > 0 { Some(&self.ytop[t..t + 16]) } else { None },
        );
        preduv(
            &mut self.wb,
            if x > 0 { Some(&self.uleft[..]) } else { None },
            if x > 0 { Some(&self.vleft[..]) } else { None },
            if y > 0 { Some(&self.uvtop[t..t + 8]) } else { None },
            if y > 0 { Some(&self.uvtop[t + 8..t + 16]) } else { None },
        );
    }

    // ve_storederr: libwebp StoreDiffusionErrors from rdm
    fn storederr(&mut self) {
        let t = self.x * 4;
        for ch in 0..2 {
            let e = self.rdm.derr[ch];
            let l1 = (e[2] as i32 * 3) >> 2;
            self.leftderr[ch * 2] = e[0];
            self.leftderr[ch * 2 + 1] = l1 as i8;
            self.topderr[t + ch * 2] = e[1];
            self.topderr[t + ch * 2 + 1] = (e[2] as i32 - l1) as i8;
        }
    }

    // ve_savectx: after a macroblock: I4 mode contexts, reconstructed boundary samples
    // (libwebp VP8IteratorSaveBoundary; unfiltered)
    fn savectx(&mut self, mbi: usize) {
        let m4 = self.mb[mbi].m4;
        let x = self.x;
        self.topm[x * 4..x * 4 + 4].copy_from_slice(&m4[12..16]);
        self.leftm = [m4[3], m4[7], m4[11], m4[15]];
        let t = x * 16;
        if x + 1 < self.mbw {
            for i in 0..16 {
                self.yleft[1 + i] = self.wb[W_OUT + i * BPS + 15];
                if i < 8 {
                    self.uleft[1 + i] = self.wb[W_OUT + i * BPS + 23];
                    self.vleft[1 + i] = self.wb[W_OUT + i * BPS + 31];
                }
            }
            // top-left before the top row changes
            self.yleft[0] = self.ytop[t + 15];
            self.uleft[0] = self.uvtop[t + 7];
            self.vleft[0] = self.uvtop[t + 15];
        }
        if self.y + 1 < self.mbh {
            self.ytop[t..t + 16].copy_from_slice(&self.wb[W_OUT + 15 * BPS..W_OUT + 15 * BPS + 16]);
            self.uvtop[t..t + 8].copy_from_slice(&self.wb[W_OUT + 7 * BPS + 16..W_OUT + 7 * BPS + 24]);
            self.uvtop[t + 8..t + 16].copy_from_slice(&self.wb[W_OUT + 7 * BPS + 24..W_OUT + 7 * BPS + 32]);
        }
    }

    // ------------------------------------------------------------ tokens

    // ve_tokens: walk the residuals of a macroblock (libwebp RecordTokens / the emission):
    // statistics or bool-encode into bw1; updates the non-zero contexts (topnz[x], leftnz)
    fn tokens(&mut self, i16: bool, lev: &[[i16; 16]; 25], emit: bool) {
        let t0 = self.x * 16;
        let (mut first, mut ty) = (0, 3);
        if i16 {
            let ctx = (self.topnz[t0 + 8] + self.leftnz[8]) as usize;
            let nz = self.coeffs(ctx, 1, 0, &lev[0], emit);
            self.topnz[t0 + 8] = nz;
            self.leftnz[8] = nz;
            first = 1;
            ty = 0;
        }
        for b in 0..16 {
            let ctx = (self.topnz[t0 + (b & 3)] + self.leftnz[b >> 2]) as usize;
            let nz = self.coeffs(ctx, ty, first, &lev[1 + b], emit);
            self.topnz[t0 + (b & 3)] = nz;
            self.leftnz[b >> 2] = nz;
        }
        for b in 0..8 {
            let base = 4 + 2 * (b >> 2);
            let ti = base + (b & 1);
            let li = base + ((b >> 1) & 1);
            let ctx = (self.topnz[t0 + ti] + self.leftnz[li]) as usize;
            let nz = self.coeffs(ctx, 2, 0, &lev[17 + b], emit);
            self.topnz[t0 + ti] = nz;
            self.leftnz[li] = nz;
        }
    }

    // .cf_bit: bit at node p (statistics or bool encoder)
    #[inline(always)]
    fn tbit(&mut self, p: usize, bit: u32, emit: bool) {
        if emit {
            let pr = self.proba[p] as u32;
            self.bw1.put(bit, pr);
        } else {
            let s = &mut self.stats[p];
            s[1] = s[1].wrapping_add(1);
            s[0] = s[0].wrapping_add(bit);
        }
    }

    // ve_coeffs: libwebp PutCoeffs / VP8RecordCoeffs on one block (zigzag levels) -> 1 when
    // a level is non-zero; .cf_fix bits (fixed probabilities) only when emitting
    fn coeffs(&mut self, ctx: usize, ty: usize, first: usize, lev: &[i16; 16], emit: bool) -> u8 {
        let next = |n: usize, c: usize| (ty * 24 + BANDS[n] as usize * 3 + c) * 11;
        let mut n = first;
        let mut p = (ty * 24 + n * 3 + ctx) * 11;
        let last = last(lev, n);
        self.tbit(p, (last >= 0) as u32, emit); // not EOB
        if last < 0 {
            return 0;
        }
        loop {
            if n >= 16 {
                return 1;
            }
            let v = lev[n] as i32;
            n += 1;
            let sign = (v as u32) >> 31;
            let a = v.unsigned_abs();
            self.tbit(p + 1, (a != 0) as u32, emit);
            if a == 0 {
                p = next(n, 0); // zero: context 0
                continue;
            }
            self.tbit(p + 2, (a > 1) as u32, emit);
            if a <= 1 {
                p = next(n, 1);
            } else {
                self.tbit(p + 3, (a > 4) as u32, emit);
                if a <= 4 {
                    self.tbit(p + 4, (a != 2) as u32, emit);
                    if a != 2 {
                        self.tbit(p + 5, (a == 4) as u32, emit);
                    }
                } else {
                    self.tbit(p + 6, (a > 10) as u32, emit);
                    if a <= 10 {
                        self.tbit(p + 7, (a > 6) as u32, emit);
                        if emit {
                            if a <= 6 {
                                self.bw1.put((a == 6) as u32, 159);
                            } else {
                                self.bw1.put((a >= 9) as u32, 165);
                                self.bw1.put(!a & 1, 145);
                            }
                        }
                    } else {
                        // categories 3..6
                        let cat;
                        if a < 19 {
                            self.tbit(p + 8, 0, emit);
                            self.tbit(p + 9, 0, emit);
                            cat = 3;
                        } else if a < 35 {
                            self.tbit(p + 8, 0, emit);
                            self.tbit(p + 9, 1, emit);
                            cat = 4;
                        } else {
                            self.tbit(p + 8, 1, emit);
                            self.tbit(p + 10, (a >= 67) as u32, emit);
                            cat = if a >= 67 { 6 } else { 5 };
                        }
                        // extra bits, MSB first, with the category probabilities
                        if emit {
                            let e = a.wrapping_sub(CATBASE[cat] as u32);
                            let nb = CATN[cat] as u32;
                            for i in 0..nb {
                                self.bw1.put((e >> (nb - i - 1)) & 1, CATP[cat][i as usize] as u32);
                            }
                        }
                    }
                }
                p = next(n, 2);
            }
            // sign
            if emit {
                self.bw1.put(sign, 128);
            }
            if n >= 16 {
                return 1;
            }
            let more = n as i32 <= last;
            self.tbit(p, more as u32, emit);
            if !more {
                return 1;
            }
        }
    }

    // ve_storelev: append the levels of a macroblock to the level store: per block (Y2 only
    // for I16, 16 Y, 8 UV) a count n = last + 1 and n int16
    fn storelev(&mut self, i16: bool, lev: &[[i16; 16]; 25]) {
        for b in (if i16 { 0 } else { 1 })..25 {
            let n = (last(&lev[b], 0) + 1) as usize;
            self.tok.push(n as u8);
            for &v in &lev[b][..n] {
                self.tok.extend_from_slice(&v.to_le_bytes());
            }
        }
    }

    // ------------------------------------------------------------ probabilities

    // ve_finalprobas: libwebp FinalizeTokenProbas: per node the probability from the
    // statistics when the update (8 bits + update flag) is cheaper than the default
    fn finalprobas(&mut self) {
        let ec = &self.ec;
        for i in 0..1056 {
            let [ones, total] = self.stats[i];
            let old = PROBA0[i] as u32;
            let upd = UPDATE[i] as u32;
            // new = ones ? 255 - ones * 255 / total : 255
            let new = if ones != 0 { 255 - ones.wrapping_mul(255) / total } else { 255 };
            let branch = |p: u32| -> u64 {
                bcost(ec, 1, p) as u64 * ones as u64 + bcost(ec, 0, p) as u64 * total.wrapping_sub(ones) as u64
            };
            let oc = branch(old) + bcost(ec, 0, upd) as u64;
            let nc = branch(new) + 8 * 256 + bcost(ec, 1, upd) as u64;
            self.proba[i] = if oc > nc { new } else { old } as u8;
        }
    }

    // ve_levelcosts: libwebp VP8CalculateLevelCosts: per type / band / context the cost of
    // levels 0..67 (tree bits; the sign and extra bits are fixc)
    fn levelcosts(&mut self) {
        let ec = &self.ec;
        for i in 0..96 {
            let p = &self.proba[i * 11..i * 11 + 11];
            let t = &mut self.lc[i * 68..i * 68 + 68];
            let c0 = if i % 3 != 0 { bcost(ec, 1, p[0] as u32) } else { 0 };
            t[0] = (bcost(ec, 0, p[1] as u32) + c0) as u16;
            let base = c0 + bcost(ec, 1, p[1] as u32);
            for v in 1..=MAX_VAR {
                let tr = &LTREE[LCLASS[v] as usize];
                let mut c = base;
                let mut k = 0;
                while tr[k] != 0xff {
                    c += bcost(ec, tr[k + 1] as u32, p[tr[k] as usize] as u32);
                    k += 2;
                }
                t[v] = c.min(0xffff) as u16;
            }
        }
    }

    // ve_adjustfilter: libwebp VP8AdjustFilterStrength (filter strength > 0): raise a
    // segment's level to the one needed for its largest blocky DC step, frame level = max
    fn adjustfilter(&mut self) {
        if self.fstr == 0 {
            return;
        }
        let mut mx = 0u32;
        for s in 0..4 {
            let sg = &mut self.seg[s];
            let dl = sg.maxedge.wrapping_mul(sg.y2.q[1]) >> 3;
            let lv = fstrength(self.fsharp, dl);
            if lv > sg.fstr {
                sg.fstr = lv;
            }
            mx = mx.max(sg.fstr);
        }
        self.level = mx;
    }

    // ------------------------------------------------------------ bitstream

    // ve_assemble: partition 0 (frame header fields, probabilities, modes), the token
    // partition from the level store, the 10-byte frame header
    fn assemble(&mut self) -> Result<Vec<u8>, i32> {
        let mut bw = Vb::new(self.mbw * self.mbh * 4 + 4096);
        // colour space, clamping type
        bw.putu(0);
        bw.putu(0);
        // segment header
        if self.nsegf > 1 {
            bw.putu(1);
            bw.putu(self.segupd as u32);
            bw.putu(1); // update data
            bw.putu(1); // absolute values
            for s in 0..4 {
                bw.signed(self.seg[s].quant as i32, 7);
            }
            for s in 0..4 {
                bw.signed(self.seg[s].fstr as i32, 6);
            }
            if self.segupd {
                for s in 0..3 {
                    let p = self.segp[s] as u32;
                    bw.putu((p != 255) as u32);
                    if p != 255 {
                        bw.bits(p, 8);
                    }
                }
            }
        } else {
            bw.putu(0);
        }
        // filter
        bw.putu(self.fsimple);
        bw.bits(self.level, 6);
        bw.bits(self.fsharp, 3);
        bw.putu(0); // no loop filter deltas
        bw.bits(0, 2); // one token partition
        // quantiser
        bw.bits(self.baseq, 7);
        bw.signed(0, 4); // y1 dc delta
        bw.signed(0, 4); // y2 dc
        bw.signed(0, 4); // y2 ac
        bw.signed(self.dquvdc, 4);
        bw.signed(self.dquvac, 4);
        bw.putu(0); // refresh entropy probabilities: no
        // coefficient probability updates
        for i in 0..1056 {
            let p = self.proba[i] as u32;
            let ch = (p != PROBA0[i] as u32) as u32;
            bw.put(ch, UPDATE[i] as u32);
            if ch != 0 {
                bw.bits(p, 8);
            }
        }
        bw.putu(0); // no skip probability
        // modes of every macroblock
        self.writemodes(&mut bw);
        bw.finish();
        if bw.buf.len() >= 524288 {
            return Err(ERR_LIMIT);
        }
        // tokens
        self.bw1 = Vb::new(self.tok.len() + 4096);
        self.emit()?;
        self.bw1.finish();
        // frame: 10-byte header + partition 0 + tokens
        let (p0, p1) = (&bw.buf, &self.bw1.buf);
        let mut out = Vec::with_capacity(p0.len() + p1.len() + 10);
        // key frame, profile 0 (1: simple filter), shown
        let b = ((p0.len() as u32) << 5) | (1 << 4) | (self.fsimple << 1);
        out.extend_from_slice(&[b as u8, (b >> 8) as u8, (b >> 16) as u8, 0x9d, 0x01, 0x2a]);
        out.extend_from_slice(&(self.w as u16).to_le_bytes());
        out.extend_from_slice(&(self.h as u16).to_le_bytes());
        out.extend_from_slice(p0);
        out.extend_from_slice(p1);
        Ok(out)
    }

    // ve_writemodes: libwebp VP8CodeIntraModes: segment (tree), I16 flag (145) and mode, or
    // 16 I4 modes with the kBModesProba contexts, chroma mode
    fn writemodes(&mut self, bw: &mut Vb) {
        let mbw = self.mbw;
        self.topm[..mbw * 4].fill(0);
        let mut mbi = 0;
        for _ in 0..self.mbh {
            self.leftm = [0; 4];
            for x in 0..mbw {
                let mi = self.mb[mbi];
                if self.segupd {
                    let s = mi.seg as u32;
                    bw.put((s >= 2) as u32, self.segp[0] as u32);
                    bw.put(s & 1, if s >= 2 { self.segp[2] } else { self.segp[1] } as u32);
                }
                bw.put(mi.ty as u32, 145);
                if mi.ty != 0 {
                    // I16: 156 (TM / HE vs DC / VE), then 128 (TM?) or 163 (VE?)
                    let m = mi.m16;
                    let b = (m == 1 || m == 3) as u32;
                    bw.put(b, 156);
                    if b != 0 {
                        bw.put((m == 1) as u32, 128);
                    } else {
                        bw.put((m == 2) as u32, 163);
                    }
                } else {
                    for i in 0..16 {
                        let tm = if i < 4 { self.topm[x * 4 + i] } else { mi.m4[i - 4] } as usize;
                        let lm = if i & 3 == 0 { self.leftm[i >> 2] } else { mi.m4[i - 1] } as usize;
                        let pr = &BMODES[(tm * 10 + lm) * 9..(tm * 10 + lm) * 9 + 9];
                        let tr = &I4TREE[mi.m4[i] as usize % 10];
                        let mut k = 0;
                        while tr[k] != 0xff {
                            bw.put(tr[k + 1] as u32, pr[tr[k] as usize] as u32);
                            k += 2;
                        }
                    }
                }
                // chroma: 142 (DC?), 114 (VE?), 183 (HE?)
                let u = mi.uv as u32;
                bw.put((u != 0) as u32, 142);
                if u != 0 {
                    bw.put((u != 2) as u32, 114);
                    if u != 2 {
                        bw.put((u != 3) as u32, 183);
                    }
                }
                // mode contexts
                self.topm[x * 4..x * 4 + 4].copy_from_slice(&mi.m4[12..16]);
                self.leftm = [mi.m4[3], mi.m4[7], mi.m4[11], mi.m4[15]];
                mbi += 1;
            }
        }
    }

    // ve_emit: all tokens with the final probabilities (contexts rebuilt in raster order
    // from the level store)
    fn emit(&mut self) -> Result<(), i32> {
        self.topnz[..self.mbw * 16].fill(0);
        let mut pos = 0usize;
        let mut mbi = 0;
        let tok = core::mem::take(&mut self.tok);
        for y in 0..self.mbh {
            self.y = y;
            self.leftnz = [0; 16];
            for x in 0..self.mbw {
                self.x = x;
                // levels of the macroblock
                let mut lev = [[0i16; 16]; 25];
                let i16 = self.mb[mbi].ty != 0;
                for l in lev[if i16 { 0 } else { 1 }..].iter_mut() {
                    let n = tok[pos] as usize;
                    pos += 1;
                    for v in l[..n.min(16)].iter_mut() {
                        *v = i16::from_le_bytes([tok[pos], tok[pos + 1]]);
                        pos += 2;
                    }
                }
                self.tokens(i16, &lev, true);
                mbi += 1;
            }
            if self.cancelled() {
                self.tok = tok;
                return Err(ERR_CANCEL);
            }
        }
        self.tok = tok;
        Ok(())
    }
}

// ---------------------------------------------------------------- RGB -> YUV helpers

// ve_uvpix: one chroma sample (U, V) from the 2x2 block at pixel i (step 0: odd last column,
// st 0: odd last row); gamma-compressed (0.80) averages, alpha weighted where the block's
// alpha is neither all 0 nor all 255
fn uvpix(px: &[u32], i: usize, step: usize, st: usize, alpha: bool) -> (u8, u8) {
    let p = [px[i], px[i + step], px[i + st], px[i + st + step]];
    let mut wt = 0u32;
    if alpha {
        let t = (p[0] >> 24) + (p[1] >> 24) + (p[2] >> 24) + (p[3] >> 24);
        if t != 0 && t != 4 * 255 {
            wt = t;
        }
    }
    let mut c = [0i32; 3]; // b, g, r
    for (ch, cv) in c.iter_mut().enumerate() {
        let sh = ch * 8;
        let l = [
            G2L[((p[0] >> sh) & 255) as usize] as u32,
            G2L[((p[1] >> sh) & 255) as usize] as u32,
            G2L[((p[2] >> sh) & 255) as usize] as u32,
            G2L[((p[3] >> sh) & 255) as usize] as u32,
        ];
        let s = if wt == 0 {
            l[0] + l[1] + l[2] + l[3]
        } else {
            let ws = l[0] * (p[0] >> 24) + l[1] * (p[1] >> 24) + l[2] * (p[2] >> 24) + l[3] * (p[3] >> 24);
            ws.wrapping_mul(4) / wt
        };
        // LinearToGamma(v, 0): interpolate the 33-entry table, descale
        let pos = (s >> 9) as usize & 31;
        let f = s & 511;
        *cv = ((L2G[pos + 1] * f + (512 - f) * L2G[pos] + 64) >> 7) as i32;
    }
    let (r, g, b) = (c[2], c[1], c[0]);
    let u = (r * -9719 + g * -19081 + b * 28800 + (1 << 17) + (128 << 18)) >> 18;
    let v = (r * 28800 + g * -24116 + b * -4684 + (1 << 17) + (128 << 18)) >> 18;
    (u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

// ve_hpad: replicate column valid - 1 to the stride
fn hpad(plane: &mut [u8], stride: usize, valid: usize, rows: usize) {
    for r in 0..rows {
        let row = &mut plane[r * stride..r * stride + stride];
        let v = row[valid - 1];
        row[valid..].fill(v);
    }
}

// ve_padrows: copy row rows - 1 into rows .. total - 1
fn padrows(plane: &mut [u8], stride: usize, rows: usize, total: usize) {
    let src = (rows - 1) * stride;
    for r in rows..total {
        plane.copy_within(src..src + stride, r * stride);
    }
}

// ---------------------------------------------------------------- transforms

// ve_ftx: libwebp FTransform: forward DCT of the 4x4 block src - rf (stride BPS), natural order
#[inline]
fn ftx(wb: &[u8], src: usize, rf: usize) -> [i16; 16] {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let s = &wb[src + i * BPS..src + i * BPS + 4];
        let r = &wb[rf + i * BPS..rf + i * BPS + 4];
        let d0 = s[0] as i32 - r[0] as i32;
        let d1 = s[1] as i32 - r[1] as i32;
        let d2 = s[2] as i32 - r[2] as i32;
        let d3 = s[3] as i32 - r[3] as i32;
        let a0 = d0 + d3;
        let a1 = d1 + d2;
        let a2 = d1 - d2;
        let a3 = d0 - d3;
        tmp[4 * i] = (a0 + a1) * 8;
        tmp[4 * i + 2] = (a0 - a1) * 8;
        tmp[4 * i + 1] = (a2 * 2217 + a3 * 5352 + 1812) >> 9;
        tmp[4 * i + 3] = (a3 * 2217 - a2 * 5352 + 937) >> 9;
    }
    let mut out = [0i16; 16];
    for i in 0..4 {
        let a0 = tmp[i] + tmp[12 + i];
        let a1 = tmp[4 + i] + tmp[8 + i];
        let a2 = tmp[4 + i] - tmp[8 + i];
        let a3 = tmp[i] - tmp[12 + i];
        out[i] = ((a0 + a1 + 7) >> 4) as i16;
        out[8 + i] = ((a0 - a1 + 7) >> 4) as i16;
        out[4 + i] = (((a2 * 2217 + a3 * 5352 + 12000) >> 16) + (a3 != 0) as i32) as i16;
        out[12 + i] = ((a3 * 2217 - a2 * 5352 + 51000) >> 16) as i16;
    }
    out
}

// ve_fwht: libwebp FTransformWHT: the DCs of the 16 blocks (c[16 * n]) -> out[16]
fn fwht(c: &[i16; 256]) -> [i16; 16] {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let b = i * 64;
        let i0 = c[b] as i32;
        let i1 = c[b + 16] as i32;
        let i2 = c[b + 32] as i32;
        let i3 = c[b + 48] as i32;
        let a0 = i0 + i2;
        let a1 = i1 + i3;
        let a2 = i1 - i3;
        let a3 = i0 - i2;
        tmp[4 * i] = a0 + a1;
        tmp[4 * i + 1] = a3 + a2;
        tmp[4 * i + 2] = a3 - a2;
        tmp[4 * i + 3] = a0 - a1;
    }
    let mut out = [0i16; 16];
    for i in 0..4 {
        let a0 = tmp[i] + tmp[8 + i];
        let a1 = tmp[4 + i] + tmp[12 + i];
        let a2 = tmp[4 + i] - tmp[12 + i];
        let a3 = tmp[i] - tmp[8 + i];
        out[i] = ((a0 + a1) >> 1) as i16;
        out[4 + i] = ((a3 + a2) >> 1) as i16;
        out[8 + i] = ((a3 - a2) >> 1) as i16;
        out[12 + i] = ((a0 - a1) >> 1) as i16;
    }
    out
}

// ve_quant: libwebp QuantizeBlock: inp (natural order) becomes the dequantised block, out =
// levels in zigzag order -> 1 if a level is non-zero
#[inline]
fn quant(inp: &mut [i16], out: &mut [i16; 16], m: &Mtx) -> u32 {
    let inp: &mut [i16; 16] = (&mut inp[..16]).try_into().unwrap();
    let mut nz = 0i32;
    for n in 0..16 {
        let j = ZIGZAG[n] as usize & 15;
        let v = inp[j] as i32;
        let sign = v >> 31;
        let c = (((v ^ sign) - sign) as u32).wrapping_add(m.sharp[j]);
        if c > m.zth[j] {
            let l = (c.wrapping_mul(m.iq[j]).wrapping_add(m.bias[j]) >> QFIX).min(MAX_LEVEL);
            let sl = (l as i32 ^ sign) - sign;
            out[n] = sl as i16;
            inp[j] = sl.wrapping_mul(m.q[j] as i32) as i16;
            nz |= sl;
        } else {
            out[n] = 0;
            inp[j] = 0;
        }
    }
    (nz != 0) as u32
}

// ve_qsingle: libwebp QuantizeSingle on the DC coefficient *v (replaced by the dequantised
// value) -> quantisation error >> 1
fn qsingle(v: &mut i16, m: &Mtx) -> i32 {
    let x = *v as i32;
    let sign = x >> 31;
    let a = ((x ^ sign) - sign) as u32;
    if a > m.zth[0] {
        let qv = (a.wrapping_mul(m.iq[0]).wrapping_add(m.bias[0]) >> QFIX).wrapping_mul(m.q[0]);
        let err = a.wrapping_sub(qv) as i32;
        *v = ((qv as i32 ^ sign) - sign) as i16;
        ((err ^ sign) - sign) >> 1
    } else {
        *v = 0;
        x >> 1
    }
}

// ve_itx: dst = rf + inverse DCT of the coefficients (4x4, stride BPS): the decoder's
// reconstruction
#[inline]
fn itx_ref(wb: &mut [u8], rf: usize, c: &[i16], dst: usize) {
    for r in 0..4 {
        wb.copy_within(rf + r * BPS..rf + r * BPS + 4, dst + r * BPS);
    }
    itx(c, wb, dst);
}

// ve_sse: sum of squared differences of w x h samples (stride BPS)
#[inline]
fn sse(wb: &[u8], a: usize, b: usize, w: usize, h: usize) -> u32 {
    let mut s = 0u32;
    for r in 0..h {
        let x = &wb[a + r * BPS..a + r * BPS + w];
        let y = &wb[b + r * BPS..b + r * BPS + w];
        for (&p, &q) in x.iter().zip(y) {
            let d = p as i32 - q as i32;
            s = s.wrapping_add((d * d) as u32);
        }
    }
    s
}

// ve_tt: libwebp TTransform: weighted sum of the absolute Hadamard coefficients of the 4x4
// block at `at` (stride BPS), weights kWeightY
#[inline]
fn tt(wb: &[u8], at: usize) -> i32 {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let r = &wb[at + i * BPS..at + i * BPS + 4];
        let (p0, p1, p2, p3) = (r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
        let a0 = p0 + p2;
        let a1 = p1 + p3;
        let a2 = p1 - p3;
        let a3 = p0 - p2;
        tmp[4 * i] = a0 + a1;
        tmp[4 * i + 1] = a3 + a2;
        tmp[4 * i + 2] = a3 - a2;
        tmp[4 * i + 3] = a0 - a1;
    }
    let mut sum = 0i32;
    for i in 0..4 {
        let a0 = tmp[i] + tmp[8 + i];
        let a1 = tmp[4 + i] + tmp[12 + i];
        let a2 = tmp[4 + i] - tmp[12 + i];
        let a3 = tmp[i] - tmp[8 + i];
        let b0 = a0 + a1;
        let b3 = a0 - a1;
        let b1 = a3 + a2;
        let b2 = a3 - a2;
        sum = sum.wrapping_add(b0.abs() * WY[i]);
        sum = sum.wrapping_add(b1.abs() * WY[4 + i]);
        sum = sum.wrapping_add(b2.abs() * WY[8 + i]);
        sum = sum.wrapping_add(b3.abs() * WY[12 + i]);
    }
    sum
}

// ---------------------------------------------------------------- costs / scores

// ve_last: index of the last non-zero level (>= first) or -1
#[inline]
fn last(lev: &[i16; 16], first: usize) -> i32 {
    let mut i = 15i32;
    while i >= first as i32 {
        if lev[i as usize] != 0 {
            return i;
        }
        i -= 1;
    }
    -1
}

// ve_isflat: true when at most thresh AC levels (positions 1..15 of each block) are non-zero
fn isflat(levs: &[[i16; 16]], thresh: u32) -> bool {
    let mut cnt = 0u32;
    for l in levs {
        for &v in &l[1..] {
            if v != 0 {
                cnt += 1;
                if cnt > thresh {
                    return false;
                }
            }
        }
    }
    true
}

// ve_isflatsrc: true when the 16x16 source block is one value
fn isflatsrc(wb: &[u8]) -> bool {
    let v = wb[W_IN];
    (0..16).all(|r| wb[W_IN + r * BPS..W_IN + r * BPS + 16].iter().all(|&p| p == v))
}

// ve_setscore: score = (R + H) * lambda + 256 * (D + SD)
#[inline]
fn setscore(rd: &mut Rd, lambda: u32) {
    rd.score = rd.r.wrapping_add(rd.h).wrapping_mul(lambda as i64).wrapping_add(rd.d.wrapping_add(rd.sd) << 8);
}

// ve_setscore_loc: the same on a block record
#[inline]
fn setscore_sc(s: &mut Sc, lambda: u32) {
    s.sc = s.r.wrapping_add(s.h).wrapping_mul(lambda as i64).wrapping_add(s.d.wrapping_add(s.sd) << 8);
}

// ---------------------------------------------------------------- predictions
// left = [top-left, left column...] or None, top = the row above or None

// ve_fill: size x size bytes of value
fn fill(wb: &mut [u8], dst: usize, v: u8, size: usize) {
    for r in 0..size {
        wb[dst + r * BPS..dst + r * BPS + size].fill(v);
    }
}

// ve_pdc: DC prediction (libwebp DCMode: both edges, one edge doubled, none -> 128)
fn pdc(wb: &mut [u8], dst: usize, left: Option<&[u8]>, top: Option<&[u8]>, size: usize) {
    let sum = |e: &[u8]| e[..size].iter().map(|&v| v as u32).sum::<u32>();
    let s = match (top, left) {
        (None, None) => return fill(wb, dst, 128, size),
        (Some(t), None) => 2 * sum(t),
        (None, Some(l)) => 2 * sum(&l[1..]),
        (Some(t), Some(l)) => sum(t) + sum(&l[1..]),
    };
    let sh = if size == 8 { 4 } else { 5 };
    fill(wb, dst, ((s + size as u32) >> sh) as u8, size);
}

// ve_pve: vertical (top missing -> 127)
fn pve(wb: &mut [u8], dst: usize, top: Option<&[u8]>, size: usize) {
    match top {
        None => fill(wb, dst, 127, size),
        Some(t) => {
            for r in 0..size {
                wb[dst + r * BPS..dst + r * BPS + size].copy_from_slice(&t[..size]);
            }
        }
    }
}

// ve_phe: horizontal (left missing -> 129)
fn phe(wb: &mut [u8], dst: usize, left: Option<&[u8]>, size: usize) {
    match left {
        None => fill(wb, dst, 129, size),
        Some(l) => {
            for r in 0..size {
                wb[dst + r * BPS..dst + r * BPS + size].fill(l[1 + r]);
            }
        }
    }
}

// ve_ptm: TrueMotion (libwebp: no left -> VE or 129, no top -> HE)
fn ptm(wb: &mut [u8], dst: usize, left: Option<&[u8]>, top: Option<&[u8]>, size: usize) {
    match (left, top) {
        (None, None) => fill(wb, dst, 129, size),
        (None, t) => pve(wb, dst, t, size),
        (l, None) => phe(wb, dst, l, size),
        (Some(l), Some(t)) => {
            let tl = l[0] as i32;
            for r in 0..size {
                let dl = l[1 + r] as i32 - tl;
                let row = &mut wb[dst + r * BPS..dst + r * BPS + size];
                for (d, &tv) in row.iter_mut().zip(&t[..size]) {
                    *d = (tv as i32 + dl).clamp(0, 255) as u8;
                }
            }
        }
    }
}

// ve_pred16: the four 16x16 predictions (DC, TM, VE, HE) into W_P16 + mode * 512
fn pred16(wb: &mut [u8], left: Option<&[u8]>, top: Option<&[u8]>) {
    pdc(wb, W_P16, left, top, 16);
    ptm(wb, W_P16 + 512, left, top, 16);
    pve(wb, W_P16 + 1024, top, 16);
    phe(wb, W_P16 + 1536, left, 16);
}

// ve_preduv: chroma predictions into W_PUV + mode * 512 (U at columns 16..23, V at 24..31)
fn preduv(wb: &mut [u8], uleft: Option<&[u8]>, vleft: Option<&[u8]>, utop: Option<&[u8]>, vtop: Option<&[u8]>) {
    for ch in 0..2 {
        let base = W_PUV + 16 + ch * 8;
        let (l, t) = if ch == 0 { (uleft, utop) } else { (vleft, vtop) };
        pdc(wb, base, l, t, 8);
        ptm(wb, base + 512, l, t, 8);
        pve(wb, base + 1024, t, 8);
        phe(wb, base + 1536, l, 8);
    }
}

// ve_pred4: the ten 4x4 predictions into W_P4 + mode * 128; top = the I4 boundary position:
// [top - 5 .. top - 2] = L K J I, [top - 1] = X, [top .. top + 7] = A..H
fn pred4(wb: &mut [u8], e: &[u8; 48], top: usize) {
    // DC: (A + B + C + D + I + J + K + L + 4) >> 3
    let mut s = 4u32;
    for k in 0..4 {
        s += e[top + k] as u32 + e[top + k - 5] as u32;
    }
    for r in 0..4 {
        wb[W_P4 + r * BPS..W_P4 + r * BPS + 4].fill((s >> 3) as u8);
    }
    // TM: clip(top[x] + left[y] - X), left[y] = e[top - 2 - y]
    let xv = e[top - 1] as i32;
    for y in 0..4 {
        let dl = e[top - 2 - y] as i32 - xv;
        for x in 0..4 {
            wb[W_P4 + 128 + y * BPS + x] = (e[top + x] as i32 + dl).clamp(0, 255) as u8;
        }
    }
    // VE .. HU from the decoder's recipes (op: 0 AVG3, 1 AVG2, 2 copy; edge e[13] at top - 5)
    let eb = &e[top - 5..top + 8];
    for mode in 2..10 {
        let rc = &P4TAB[(mode - 2) * 64..(mode - 2) * 64 + 64];
        let d = W_P4 + mode * 128;
        for px in 0..16 {
            let r = &rc[px * 4..px * 4 + 4];
            let a = eb[r[1] as usize % 13] as u32;
            let v = match r[0] {
                2 => a,
                1 => (a + eb[r[2] as usize % 13] as u32 + 1) >> 1,
                _ => (a + 2 * eb[r[2] as usize % 13] as u32 + eb[r[3] as usize % 13] as u32 + 2) >> 2,
            };
            wb[d + (px >> 2) * BPS + (px & 3)] = v as u8;
        }
    }
}

// ---------------------------------------------------------------- quality curve (f64)

// ve_log2d: log2(x) (x > 0): exponent + 2 atanh series of the mantissa in [0.707, 1.414)
fn log2d(x: f64) -> f64 {
    let bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > d(D_SQRT2) {
        m *= d(D_HALF);
        e += 1;
    }
    let t = (m - d(D_ONE)) / (m + d(D_ONE));
    let t2 = t * t;
    let mut sum = t;
    let mut term = t;
    let mut k = 3;
    while k < 41 {
        term *= t2;
        sum += term / k as f64;
        k += 2;
    }
    let ln = sum + sum;
    e as f64 + ln * d(D_INVLN2)
}

// ve_exp2d: 2^x (|x| < 1000)
fn exp2d(x: f64) -> f64 {
    let mut i = x as i32;
    if i as f64 > x {
        i -= 1; // floor for negative fractions
    }
    let r = (x - i as f64) * d(D_LN2);
    let mut sum = d(D_ONE);
    let mut term = d(D_ONE);
    for k in 1..24 {
        term = term * r / k as f64;
        sum += term;
    }
    f64::from_bits(((i + 1023) as i64 as u64) << 52) * sum
}

// ve_pow: x^y (x >= 0; 0 -> 0)
fn pow(x: f64, y: f64) -> f64 {
    if !(x > 0.0) {
        return 0.0;
    }
    exp2d(log2d(x) * y)
}

// ve_fstrength: smallest filter level whose edge limit 2 level + interior level reaches
// 3 delta (stands in for libwebp's kLevelsFromDelta table)
fn fstrength(sharp: u32, delta: u32) -> u32 {
    let delta = delta.min(63);
    let d3 = delta * 3;
    let mut level = 0u32;
    loop {
        let mut il = level;
        if sharp != 0 {
            if sharp > 4 {
                il >>= 1;
            }
            il >>= 1;
            let lim = 9u32.wrapping_sub(sharp);
            if il > lim {
                il = lim;
            }
        }
        if il < 1 {
            il = 1;
        }
        if level * 2 + il >= d3 {
            break;
        }
        level += 1;
        if level >= 63 {
            break;
        }
    }
    if delta == 0 { 0 } else { level }
}

// ve_expand: libwebp ExpandMatrix: iq = 2^17 / q, bias = kBiasMatrices[type] << 9,
// zthresh = (2^17 - 1 - bias) / iq, sharpening of the luma AC coefficients (type 0)
// -> (sum q + 8) >> 4
fn expand(m: &mut Mtx, dc: u32, ac: u32, ty: usize) -> u32 {
    m.q[0] = dc;
    m.q[1] = ac;
    for i in 0..2 {
        m.iq[i] = (1 << QFIX) / m.q[i];
        m.bias[i] = (BIAS[ty * 2 + (i > 0) as usize] as u32) << (QFIX - 8);
        m.zth[i] = ((1 << QFIX) - 1 - m.bias[i]) / m.iq[i];
    }
    for i in 2..16 {
        m.q[i] = m.q[1];
        m.iq[i] = m.iq[1];
        m.bias[i] = m.bias[1];
        m.zth[i] = m.zth[1];
    }
    let mut sum = 0u32;
    for i in 0..16 {
        sum += m.q[i];
        m.sharp[i] = if ty == 0 { (SHARPEN[i] as u32 * m.q[i]) >> 11 } else { 0 };
    }
    (sum + 8) >> 4
}

// .spp_p: GetProba(a, b) = total ? (255 a + total / 2) / total : 255
fn getp(a: u32, b: u32) -> u8 {
    let t = a + b;
    if t == 0 { 255 } else { ((a * 255 + (t >> 1)) / t) as u8 }
}
