// ent -- JPEG XR codestream parsing and entropy decoding (ITU-T T.832 clauses 8 and 9.4 - 9.8)
//                                                                    [src/img/jxr_ent.s]
//
//   decode(cs, dec) -> Cs / ERR_*    jxe_decode: IMAGE_HEADER, IMAGE_PLANE_HEADER(s) and the
//                                    index table of the codestream bytes cs, the sample planes,
//                                    DC grids and per-macroblock arrays, then every tile (DC / LP
//                                    / HP / FLEXBITS): coefficient prediction, dequantisation,
//                                    the first level inverse transform of the LP band into the DC
//                                    grids (x2 for scaled chroma) and the dequantised HP
//                                    coefficients into the sample planes (ICT input layout, the
//                                    DC slot of every 4x4 block left 0)
//   je_reset / adapt_all             the entropy context reset / adaptation shared with the
//                                    encoder (jxe_je_reset, jxe_adapt_all)
//
// The bit reader keeps the assembly's register state (buffer, valid bits, byte pointer, end
// pointer) with "addresses" = offset in the codestream + 1: 0 is the absent packet of the
// assembly (zero bits, its pointer counting up from 0), so every comparison of the assembly
// (packet overrun, index entries, byte alignment) gives the same answer.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use img_core::{ERR_CANCEL, ERR_DIMS, ERR_HDR, ERR_TRUNC, ERR_UNSUPPORTED, ImgDec, meta};

use crate::pct::ict4x4;use crate::tab::{JXV_DELTA, JXV_LUT};

// ------------------------------------------------------------------ records (img.inc CX)
pub const JXMAXCH: usize = 16; // colour components per image plane supported

pub const JXF_HARDTILE: u32 = 1; // Cs::flags
pub const JXF_TILING: u32 = 2;
pub const JXF_FREQ: u32 = 4;
pub const JXF_INDEX: u32 = 8;
pub const JXF_SHORT: u32 = 16;
pub const JXF_LONGW: u32 = 32;
pub const JXF_WINDOW: u32 = 64;
pub const JXF_TRIM: u32 = 128;
pub const JXF_RBNS: u32 = 256; // RED_BLUE_NOT_SWAPPED
pub const JXF_PREMUL: u32 = 512;
pub const JXF_ALPHA: u32 = 1024; // interleaved alpha image plane
pub const JXW_TRUNC: u32 = 1; // Cs::warn: a packet ended early (zeros were read)
pub const JXW_START: u32 = 2; // a packet start code was wrong (packet ignored)
pub const JXW_QP: u32 = 4; // a quantisation parameter > 0 was read (lossy)

/// one image plane of a codestream (JXPL): the primary one, or the interleaved alpha plane
#[derive(Default)]
pub struct Plane {
    pub cf: u32, // INTERNAL_CLR_FMT: 0 YONLY 1 YUV420 2 YUV422 3 YUV444 4 YUVK 6 NCOMPONENT
    pub nch: u32,
    pub scaled: u32,
    pub bands: u32, // BANDS_PRESENT (0 all .. 3 DC only)
    pub shift: u32, // SHIFT_BITS
    pub mant: u32,  // LEN_MANTISSA
    pub ebias: i32, // EXP_BIAS
    pub ccx: u32,
    pub ccy: u32,
    pub dcu: u32, // DC / LP / HP image-plane uniform flags
    pub lpu: u32,
    pub hpu: u32,
    pub qp: [[u8; 16]; 3],    // plane-level QP per band (DC, LP, HP) and component
    pub buf: Vec<Vec<i32>>,   // sample planes per component
    pub dcg: Vec<Vec<i32>>,   // DC grids per component
    pub pw: [u32; JXMAXCH],   // sample plane width / height
    pub ph: [u32; JXMAXCH],
    pub gw: [u32; JXMAXCH],   // DC grid width / height (cells)
    pub gh: [u32; JXMAXCH],
    pub lpq: Vec<i32>, // quantized LP per MB: [MBH][MBW][NCH][16]
    pub cbp: Vec<u32>, // HP CBP per MB: [MBH][MBW][NCH + 1] (the last entry: LP QP index)
}

/// one codestream (JXCS): IMAGE_HEADER + planes
#[derive(Default)]
pub struct Cs {
    pub w: u32,
    pub h: u32,
    pub mt: u32, // top / left / bottom / right margin
    pub ml: u32,
    pub mbot: u32,
    pub mr: u32,
    pub mbw: u32, // macroblocks across / down (extended image)
    pub mbh: u32,
    pub flags: u32, // JXF_*
    pub ovl: u32,   // OVERLAP_MODE
    pub ocf: u32,   // OUTPUT_CLR_FMT
    pub obd: u32,   // OUTPUT_BITDEPTH
    pub xform: u32, // SPATIAL_XFRM_SUBORDINATE
    pub ntx: u32,
    pub nty: u32,
    pub npl: u32,     // image planes (2 = interleaved alpha)
    pub tx: Vec<u32>, // [NTX + 1] first MB column per tile column (last = MBW)
    pub ty: Vec<u32>,
    pub idx: Vec<u64>, // packet start addresses (0 = absent)
    pub nb: u32,       // packets per tile in the index (1 in spatial mode)
    pub warn: u32,     // JXW_*
    pub pl: [Plane; 2],
}

// ------------------------------------------------------------------ entropy context (jxr_int.inc)
pub const NAH: usize = 24; // 0..20 T.832 tables, 21 NUM_CBPHP, 22 / 23 NUM_BLKCBPHP (5 / 9)
pub const AHI_NCBP: usize = 21;
pub const AHI_NBLK5: usize = 22;
pub const AHI_NBLK9: usize = 23;
const THR_MEM: i32 = 64; // |discriminant| limit
const MAXTOTAL: u16 = 32767;

/// an adaptive VLC instance (AH)
#[derive(Clone, Copy, Default)]
pub struct Ah {
    pub lut: u32, // current table: first entry in JXV_LUT
    pub dp: u32,  // current delta rows: first entry in JXV_DELTA
    pub dp1: u32,
    pub disc: i32,
    pub disc1: i32,
    pub t: i32, // table index
    pub lo: i32,
    pub hi: i32,
    pub n: u8,  // alphabet size
    pub mt: u8, // tables
    pub sec: u8,
    pub lbase: u32, // first table of the alphabet
    pub dbase: u32, // first delta row of the alphabet
}

/// an adaptive scan (SC)
#[derive(Clone, Copy, Default)]
pub struct Scan {
    pub ord: [u8; 16],
    pub tot: [u16; 16],
}

/// a model (MD)
#[derive(Clone, Copy, Default)]
pub struct Model {
    pub st: [i32; 2],
    pub bits: [i32; 2],
}

/// the entropy context of one plane (JE)
#[derive(Clone)]
pub struct Je {
    pub ah: [Ah; NAH],
    pub scan: [Scan; 3], // LP, HP horizontal, HP vertical
    pub md: [Model; 3],  // DC, LP, HP
    pub cbc0: [i32; 2],  // CBPHP model counts / state
    pub cbc1: [i32; 2],
    pub cbst: [i32; 2],
    pub lpz: i32, // CountZeroCBPLP
    pub lpm: i32, // CountMaxCBPLP
    pub nlpq: u32,
    pub nhpq: u32,
    pub usedc: u32,
    pub uselp: u32,
    pub qdc: [u32; 16],
    pub qlp: [[u32; 16]; 16],
    pub qhp: [[u32; 16]; 16],
    pub idc: [u8; 16],
    pub ilp: [[u8; 16]; 16],
    pub ihp: [[u8; 16]; 16],
}

impl Default for Je {
    fn default() -> Je {
        Je {
            ah: [Ah::default(); NAH],
            scan: [Scan::default(); 3],
            md: [Model::default(); 3],
            cbc0: [0; 2],
            cbc1: [0; 2],
            cbst: [0; 2],
            lpz: 0,
            lpm: 0,
            nlpq: 0,
            nhpq: 0,
            usedc: 0,
            uselp: 0,
            qdc: [0; 16],
            qlp: [[0; 16]; 16],
            qhp: [[0; 16]; 16],
            idc: [0; 16],
            ilp: [[0; 16]; 16],
            ihp: [[0; 16]; 16],
        }
    }
}

// JXV table offsets (u16 entries / i8 rows)
const JXV_L4: u32 = 0;
const JXV_L5: u32 = 256;
const JXV_L6: u32 = 768;
const JXV_L7: u32 = 1792;
const JXV_L8: u32 = 2304;
const JXV_L9: u32 = 2560;
const JXV_L12: u32 = 3072;
const JXV_D5: u32 = 16;
const JXV_D6: u32 = 32;
const JXV_D7: u32 = 80;
const JXV_D9: u32 = 96;
const JXV_D12: u32 = 112;

// AH setup: alphabet, tables, second discriminant, LUT, delta rows  [jxe_ahinfo]
static AHINFO: [(u8, u8, u8, u32, u32); NAH] = [
    (5, 1, 0, JXV_L5, 0), // 0 run value (fixed)
    (4, 1, 0, JXV_L4, 0), // 1 chroma block count (fixed)
    (8, 1, 0, JXV_L8, 0), // 2 VAL_DC_YUV (fixed)
    (7, 2, 0, JXV_L7, JXV_D7), // 3 AbsLevelIndDCLum
    (7, 2, 0, JXV_L7, JXV_D7), // 4 AbsLevelIndDCChr
    (12, 5, 1, JXV_L12, JXV_D12), // 5..7 LP luma
    (6, 4, 1, JXV_L6, JXV_D6),
    (6, 4, 1, JXV_L6, JXV_D6),
    (12, 5, 1, JXV_L12, JXV_D12), // 8..10 LP chroma
    (6, 4, 1, JXV_L6, JXV_D6),
    (6, 4, 1, JXV_L6, JXV_D6),
    (7, 2, 0, JXV_L7, JXV_D7), // 11, 12 AbsLevelIndLP0 / 1
    (7, 2, 0, JXV_L7, JXV_D7),
    (12, 5, 1, JXV_L12, JXV_D12), // 13..15 HP luma
    (6, 4, 1, JXV_L6, JXV_D6),
    (6, 4, 1, JXV_L6, JXV_D6),
    (12, 5, 1, JXV_L12, JXV_D12), // 16..18 HP chroma
    (6, 4, 1, JXV_L6, JXV_D6),
    (6, 4, 1, JXV_L6, JXV_D6),
    (7, 2, 0, JXV_L7, JXV_D7), // 19, 20 AbsLevelIndHP0 / 1
    (7, 2, 0, JXV_L7, JXV_D7),
    (5, 2, 0, JXV_L5, JXV_D5), // 21 NUM_CBPHP
    (5, 2, 0, JXV_L5, JXV_D5), // 22 NUM_BLKCBPHP (5)
    (9, 2, 0, JXV_L9, JXV_D9), // 23 NUM_BLKCBPHP (9)
];

pub static SCAN_H: [u8; 16] = [0, 1, 4, 5, 2, 8, 6, 9, 3, 12, 10, 7, 13, 11, 14, 15];
pub static SCAN_V: [u8; 16] = [0, 4, 8, 5, 1, 12, 9, 6, 2, 13, 3, 15, 7, 10, 14, 11];
static ABSREM: [u8; 6] = [2, 3, 4, 6, 10, 14]; // iRemap
static ABSLEN: [u8; 6] = [0, 0, 1, 2, 2, 2]; // iFixedLen
static RUNBIN5: [u8; 16] = [0, 0, 0, 0, 10, 10, 10, 5, 5, 5, 5, 0, 0, 0, 0, 0]; // 5 * RUNBIN[maxrun]
static RUNREM: [u8; 16] = [1, 2, 3, 5, 7, 1, 2, 3, 5, 7, 1, 2, 3, 4, 5, 0];
static RUNFLC: [u8; 16] = [0, 0, 1, 1, 3, 0, 0, 1, 1, 2, 0, 0, 0, 0, 1, 0];
static W0: [i32; 3] = [240, 12, 1]; // iWeight0
static W2: [i32; 6] = [120, 37, 2, 120, 18, 1]; // iWeight2 (4:2:0, 4:2:2)
static W1: [[u8; 16]; 3] = [
    [0, 240, 120, 80, 60, 48, 40, 34, 30, 27, 24, 22, 20, 18, 17, 16],
    [0, 12, 6, 4, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1],
    [0, 16, 8, 5, 4, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1],
];
/// F order -> 4x4 raster of the ICT input
pub static F2R: [u8; 16] = [0, 2, 1, 7, 8, 15, 12, 11, 4, 3, 5, 6, 13, 14, 9, 10];
static UVPOS420: [u8; 3] = [1, 2, 3];
static UVPOS422: [u8; 7] = [4, 1, 2, 3, 5, 6, 7];
static REF1: [u8; 4] = [6, 9, 10, 12];
static CBPFLC: [u8; 6] = [0, 2, 1, 2, 2, 0];
static CBPOFF: [u8; 6] = [0, 4, 2, 8, 12, 1];
static CBPOUT: [u8; 16] = [0, 15, 3, 12, 1, 2, 4, 8, 5, 6, 9, 10, 7, 11, 13, 14];
static CH422: [u8; 3] = [1, 4, 5];
static CH422SH: [u8; 4] = [0, 1, 4, 5];
static QPBITS: [u8; 17] = [0, 0, 1, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4];
static RAST2: [u8; 16] = [0x00, 0x01, 0x10, 0x11, 0x20, 0x21, 0x30, 0x31, 0, 0, 0, 0, 0, 0, 0, 0];
static QUAD: [u8; 16] = [0x00, 0x01, 0x10, 0x11, 0x02, 0x03, 0x12, 0x13, 0x20, 0x21, 0x30, 0x31, 0x22, 0x23, 0x32, 0x33];
// HP prediction (dst, src) block pairs per geometry (16 blocks, 4:2:0, 4:2:2 chroma) x mode
static HPP0L: [(u8, u8); 12] =
    [(1, 0), (4, 1), (5, 4), (3, 2), (6, 3), (7, 6), (9, 8), (12, 9), (13, 12), (11, 10), (14, 11), (15, 14)];
static HPP0T: [(u8, u8); 12] =
    [(2, 0), (3, 1), (6, 4), (7, 5), (8, 2), (9, 3), (12, 6), (13, 7), (10, 8), (11, 9), (14, 12), (15, 13)];
static HPP1L: [(u8, u8); 2] = [(1, 0), (3, 2)];
static HPP1T: [(u8, u8); 2] = [(2, 0), (3, 1)];
static HPP2L: [(u8, u8); 4] = [(1, 0), (3, 2), (5, 4), (7, 6)];
static HPP2T: [(u8, u8); 6] = [(2, 0), (3, 1), (4, 2), (5, 3), (6, 4), (7, 5)];

// ------------------------------------------------------------------ the entropy context

/// AdaptDiscriminant: switch tables when a discriminant crosses its bound, then select the
/// table / delta rows for the current index  [ah_adapt]
#[inline]
pub fn ah_adapt(a: &mut Ah) {
    let d0 = a.disc;
    let d1 = if a.sec != 0 { a.disc1 } else { d0 };
    let mut t = a.t;
    if d0 < a.lo {
        t -= 1;
        a.disc = 0;
        a.disc1 = 0;
    } else if d1 > a.hi {
        t += 1;
        a.disc = 0;
        a.disc1 = 0;
    }
    let mt1 = a.mt as i32 - 1;
    if t > mt1 {
        t = mt1;
    }
    if t < 0 {
        t = 0;
    }
    a.t = t;
    a.disc = a.disc.clamp(-THR_MEM, THR_MEM);
    a.disc1 = a.disc1.clamp(-THR_MEM, THR_MEM);
    a.lo = if t != 0 { -8 } else { i32::MIN };
    a.hi = if t != mt1 { 8 } else { 0x4000_0000 };
    a.lut = a.lbase + (t as u32) * 256;
    if mt1 == 0 {
        a.dp = 0;
        a.dp1 = 0;
    } else if a.sec == 0 {
        a.dp = a.dbase;
        a.dp1 = 0;
    } else {
        let r0 = if t == 0 { 0 } else { t - 1 };
        let r1 = if t == mt1 { t - 1 } else { t };
        a.dp = a.dbase + r0 as u32 * 16;
        a.dp1 = a.dbase + r1 as u32 * 16;
    }
}

fn scan_tot(s: &mut Scan) {
    s.tot[0] = MAXTOTAL;
    let mut v = 32u16;
    for k in 1..16 {
        s.tot[k] = v;
        v = v.wrapping_sub(2);
    }
}

/// ResetTotalsAdaptiveScanLP  [scan_totals_lp]
pub fn scan_totals_lp(je: &mut Je) {
    scan_tot(&mut je.scan[0]);
}

/// ResetTotalsAdaptiveScanHP (both HP scans)  [scan_totals_hp]
pub fn scan_totals_hp(je: &mut Je) {
    scan_tot(&mut je.scan[1]);
    scan_tot(&mut je.scan[2]);
}

/// tile start: InitializeDC/LP/HP VLC, scans, models, CBPLP / CBPHP counts  [je_reset]
pub fn je_reset(je: &mut Je) {
    for (a, &(n, mt, sec, l, d)) in je.ah.iter_mut().zip(AHINFO.iter()) {
        a.n = n;
        a.mt = mt;
        a.sec = sec;
        a.t = sec as i32;
        a.disc = 0;
        a.disc1 = 0;
        a.lo = i32::MIN;
        a.hi = 0x4000_0000;
        a.lbase = l;
        a.dbase = d;
        ah_adapt(a);
    }
    je.scan[0].ord = SCAN_H;
    je.scan[1].ord = SCAN_H;
    je.scan[2].ord = SCAN_V;
    scan_totals_lp(je);
    scan_totals_hp(je);
    je.md[0] = Model { st: [0, 0], bits: [8, 8] };
    je.md[1] = Model { st: [0, 0], bits: [4, 4] };
    je.md[2] = Model { st: [0, 0], bits: [0, 0] };
    je.cbc0 = [-4, -4];
    je.cbc1 = [4, 4];
    je.cbst = [0, 0];
    je.lpz = 1;
    je.lpm = 1;
}

/// AdaptDC / AdaptLP / AdaptHP after a macroblock  [adapt_all]
pub fn adapt_all(je: &mut Je) {
    for a in je.ah.iter_mut() {
        ah_adapt(a);
    }
}

// ------------------------------------------------------------------ bit reader

const K: u64 = 1; // address of codestream byte 0

/// the MSB-first reader of the assembly (r12 buffer, r13 valid bits, r14 pointer, r15 end)
#[derive(Clone, Copy, Default)]
pub struct Rd {
    pub buf: u64,
    pub cnt: u64,
    pub ptr: u64,
    pub end: u64,
}

impl Rd {
    /// BRM_REFILL
    #[inline(always)]
    fn refill(&mut self, d: &[u8]) {
        if self.cnt >= 56 {
            return;
        }
        if (self.end.wrapping_sub(self.ptr) as i64) >= 8 {
            let o = (self.ptr - K) as usize;
            let v = u64::from_be_bytes(d[o..o + 8].try_into().unwrap());
            self.buf |= v >> self.cnt;
            self.ptr += (63 - self.cnt) >> 3;
            self.cnt |= 56;
        } else {
            self.refill_slow(d);
        }
    }

    #[inline(always)]
    fn refill_slow(&mut self, d: &[u8]) {
        // by value, so that a reader kept in a local never has its address taken
        (self.buf, self.cnt, self.ptr) = refill_bytes(d, self.buf, self.cnt, self.ptr, self.end);
    }

    /// next n bits (n = 0..32)  [getv]
    #[inline(always)]
    fn getv(&mut self, d: &[u8], n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.cnt < n as u64 {
            self.refill(d);
        }
        let v = self.buf >> (64 - n);
        self.buf <<= n;
        self.cnt = self.cnt.wrapping_sub(n as u64);
        v as u32
    }

    /// 64-bit variant of getv for n = 32 (vlwesc)
    fn getv64(&mut self, d: &[u8], n: u32) -> u64 {
        if self.cnt < n as u64 {
            self.refill(d);
        }
        let v = self.buf >> (64 - n);
        self.buf <<= n;
        self.cnt = self.cnt.wrapping_sub(n as u64);
        v
    }

    /// next bit  [bit1]
    #[inline(always)]
    fn bit1(&mut self, d: &[u8]) -> u32 {
        if self.cnt < 1 {
            self.refill(d);
        }
        self.take1()
    }

    /// DB_BIT: a bit without the refill check
    #[inline(always)]
    fn take1(&mut self) -> u32 {
        let v = (self.buf >> 63) as u32;
        self.buf <<= 1;
        self.cnt = self.cnt.wrapping_sub(1);
        v
    }

    /// DB_FLC: n = 0..15 bits without the refill check (n = 0 gives 0)
    #[inline(always)]
    fn flc(&mut self, n: u32) -> u32 {
        let v = (self.buf >> 1) >> (63 - n);
        self.buf <<= n;
        self.cnt = self.cnt.wrapping_sub(n as u64);
        v as u32
    }

    /// DB_VLC: adaptive VLC read + discriminant update, no refill check
    #[inline(always)]
    fn vlc_nf(&mut self, a: &mut Ah) -> u32 {
        let e = JXV_LUT[((a.lut + (self.buf >> 56) as u32) & 8191) as usize] as u32;
        let len = e >> 8;
        self.buf <<= len;
        self.cnt = self.cnt.wrapping_sub(len as u64);
        let s = e & 15;
        a.disc = a.disc.wrapping_add(JXV_DELTA[((a.dp + s) & 255) as usize] as i32);
        a.disc1 = a.disc1.wrapping_add(JXV_DELTA[((a.dp1 + s) & 255) as usize] as i32);
        s
    }

    /// adaptive VLC read (refill to 8 bits first)  [vlc]
    #[inline(always)]
    fn vlc(&mut self, d: &[u8], a: &mut Ah) -> u32 {
        if self.cnt < 8 {
            self.refill(d);
        }
        self.vlc_nf(a)
    }

    /// DB_FILL: at least 48 bits
    #[inline(always)]
    fn fill48(&mut self, d: &[u8]) {
        if self.cnt < 48 {
            self.refill(d);
        }
    }

    /// 0 ("1") / 1 ("01") / 2 ("00")  [code3]
    fn code3(&mut self, d: &[u8]) -> u32 {
        if self.cnt < 2 {
            self.refill(d);
        }
        if self.take1() != 0 {
            return 0;
        }
        if self.take1() != 0 { 1 } else { 2 }
    }

    /// drop the bits up to the byte boundary; the address of the next unread byte  [bytepos]
    fn bytepos(&mut self) -> u64 {
        let c = self.cnt & 7;
        self.buf <<= c;
        self.cnt -= c;
        self.ptr.wrapping_sub(self.cnt >> 3)
    }

    /// value of VLW_ESC: (value, present) (0xFD..0xFF: escape)  [vlwesc]
    fn vlwesc(&mut self, d: &[u8]) -> (u64, bool) {
        let v = self.getv(d, 8);
        if v < 0xFB {
            let lo = self.getv(d, 8);
            return (((v << 8) | lo) as u64, true);
        }
        if v == 0xFB {
            return (self.getv64(d, 32), true);
        }
        if v == 0xFC {
            let hi = self.getv64(d, 32);
            let lo = self.getv64(d, 32);
            return ((hi << 32) | lo, true);
        }
        (0, false)
    }
}

/// the byte-wise part of BRM_REFILL (near the end of the data: zero bytes past it)
#[inline(never)]
fn refill_bytes(d: &[u8], mut buf: u64, mut cnt: u64, mut ptr: u64, end: u64) -> (u64, u64, u64) {
    loop {
        let b = if ptr < end { d[(ptr - K) as usize] as u64 } else { 0 };
        ptr = ptr.wrapping_add(1);
        buf |= b << (56 - cnt);
        cnt += 8;
        if cnt > 56 {
            return (buf, cnt, ptr);
        }
    }
}

/// escape of DECODE_ABS_LEVEL (symbol 6)  [abs_esc]
fn abs_esc(r: &mut Rd, d: &[u8]) -> i32 {
    let mut f = r.getv(d, 4) + 4;
    if f == 19 {
        f += r.getv(d, 2);
        if f == 22 {
            f += r.getv(d, 3);
        }
    }
    let v = r.getv(d, f);
    v.wrapping_add(1u32 << f).wrapping_add(2) as i32
}

/// DECODE_ABS_LEVEL -> level >= 2  [abslevel]
fn abslevel(r: &mut Rd, d: &[u8], a: &mut Ah) -> i32 {
    let v = r.vlc(d, a);
    if v < 2 {
        return v as i32 + 2;
    }
    if v >= 6 {
        return abs_esc(r, d);
    }
    let base = ABSREM[v as usize] as u32;
    (r.getv(d, ABSLEN[v as usize] as u32) + base) as i32
}

/// DB_LEVEL: DECODE_ABS_LEVEL inside a block (the escape refills itself, then DB_FILL)
#[inline(always)]
fn db_level(r: &mut Rd, d: &[u8], a: &mut Ah) -> i32 {
    let v = r.vlc_nf(a);
    if v >= 6 {
        let l = abs_esc(r, d);
        r.fill48(d);
        return l;
    }
    let base = ABSREM[v as usize] as u32;
    (r.flc(ABSLEN[v as usize] as u32) + base) as i32
}

/// DB_RUN: DECODE_RUN, maxrun 1..14
#[inline(always)]
fn db_run(r: &mut Rd, maxrun: u32) -> i32 {
    if maxrun < 5 {
        let mut run = 1;
        loop {
            if maxrun == run {
                break;
            }
            if r.take1() != 0 {
                break;
            }
            run += 1;
            if run >= 4 {
                break;
            }
        }
        return run as i32;
    }
    let bin5 = RUNBIN5[maxrun as usize] as u32;
    let e = JXV_LUT[((JXV_L5 + (r.buf >> 56) as u32) & 8191) as usize] as u32;
    let len = e >> 8;
    r.buf <<= len;
    r.cnt = r.cnt.wrapping_sub(len as u64);
    let i = (((e & 15) + bin5) & 15) as usize; // (symbols 0..4: the mask changes nothing)
    let base = RUNREM[i] as u32;
    (r.flc(RUNFLC[i] as u32) + base) as i32
}

// ------------------------------------------------------------------ the decoder work record (JW)

struct Dec<'a> {
    cs: &'a [u8],
    r: Rd,        // the current reader (r12..r15)
    rds: [Rd; 4], // JW_RD
    cur: u32,
    jx: Cs,
    je: [Box<Je>; 2],
    cancel: Option<&'a AtomicU32>,
    pi: usize,
    cf: u32,
    nch: u32,
    scaled: u32,
    bands: u32,
    tx0: u32,
    tx1: u32,
    ty0: u32,
    ty1: u32,
    mx: u32,
    my: u32,
    trim: u32,
    lpqi: u32,
    hpqi: u32,
    acm: u32,
    dcm: u32,
    freq: u32,
    mbw: u32,
    mbh: u32,
    npl: u32,
    lm: [i32; 2], // iLapMean
    nb: u32,
    warn: u32,
    hb: [i32; 2],        // HP model bits of the macroblock (before the update)
    rl: [[i32; 2]; 16],  // run / level pairs of DECODE_BLOCK
    diff: [u32; 16],     // residual CBPHP
    cbp: [u32; 16],      // CBPHP
    lp: [[i32; 16]; 16], // DC + LP (F order) of the macroblock
    hp: Vec<[i32; 16]>,  // HP blocks [component * 16 + block] (F order)
}

/// number of set bits of the low 16 bits
#[inline]
fn pop16(v: u32) -> u32 {
    (v & 0xFFFF).count_ones()
}

/// quantiser of a QP index (QuantMap)  [quantmap]
pub fn quantmap(idx: u32, shift: u32, scaled: bool) -> u32 {
    if idx == 0 {
        return 1;
    }
    if scaled {
        if idx < 16 {
            return idx << shift;
        }
        return ((idx & 15) + 16) << (shift + (idx >> 4) - 1);
    }
    if idx < 32 {
        return (idx + 3) >> 2;
    }
    if idx < 48 {
        return (((idx & 15) + 17) >> 1) << ((idx >> 4) - 2);
    }
    ((idx & 15) + 16) << ((idx >> 4) - 3)
}

impl Dec<'_> {
    // ---------------------------------------------------------------- helpers
    fn getv(&mut self, n: u32) -> u32 {
        self.r.getv(self.cs, n)
    }

    fn bit1(&mut self) -> u32 {
        self.r.bit1(self.cs)
    }

    /// make that band's reader current (spatial mode: always reader 0)  [sel_rd]
    fn sel_rd(&mut self, band: u32) {
        let b = if self.freq == 0 { 0 } else { band };
        if self.cur == b {
            return;
        }
        self.rds[self.cur as usize] = self.r;
        self.cur = b;
        self.r = self.rds[b as usize];
    }

    /// JW_PL / JW_JE / JW_CF / JW_NCH / JW_SCALED / JW_BANDS  [setplane]
    fn setplane(&mut self, p: usize) {
        self.pi = p;
        let pl = &self.jx.pl[p];
        self.cf = pl.cf;
        self.nch = pl.nch;
        self.scaled = pl.scaled;
        self.bands = pl.bands;
    }

    /// LPQ record of a macroblock: first i32 in Plane::lpq  [mbrec]
    #[inline]
    fn mbrec(&self, mx: u32, my: u32) -> usize {
        (((my * self.mbw + mx) * self.nch) as usize) << 4
    }

    /// CBP record of a macroblock: first u32 in Plane::cbp  [cbprec]
    #[inline]
    fn cbprec(&self, mx: u32, my: u32) -> usize {
        ((my * self.mbw + mx) * (self.nch + 1)) as usize
    }

    // ---------------------------------------------------------------- models, scans
    /// UpdateModelMB with iLapMean = lm  [updmodel]
    fn updmodel(&mut self, band: usize) {
        let cf = self.cf;
        let nch = self.nch;
        self.lm[0] = self.lm[0].wrapping_mul(W0[band]);
        let mut lm1 = self.lm[1];
        if cf == 1 {
            lm1 = lm1.wrapping_mul(W2[band]);
        } else if cf == 2 {
            lm1 = lm1.wrapping_mul(W2[3 + band]);
        } else {
            lm1 = lm1.wrapping_mul(W1[band][(nch.wrapping_sub(1) & 15) as usize] as i32);
            if band == 2 {
                lm1 >>= 4;
            }
        }
        self.lm[1] = lm1;
        let m = &mut self.je[self.pi].md[band];
        for j in 0..2 {
            let delta = self.lm[j].wrapping_sub(70) >> 2;
            let mut ms = m.st[j];
            let mut bits = m.bits[j];
            if delta <= -8 {
                let dl = (delta + 4).max(-16);
                ms = ms.wrapping_add(dl);
                if ms < -8 {
                    if bits == 0 {
                        ms = -8;
                    } else {
                        ms = 0;
                        bits -= 1;
                    }
                }
            } else if delta >= 8 {
                let dl = (delta - 4).min(15);
                ms = ms.wrapping_add(dl);
                if ms > 8 {
                    if bits >= 15 {
                        bits = 15;
                        ms = 8;
                    } else {
                        ms = 0;
                        bits += 1;
                    }
                }
            }
            m.st[j] = ms;
            m.bits[j] = bits;
            if cf == 0 {
                break;
            }
        }
    }

    /// AdaptiveLPScan / AdaptiveHPScan: coefficient k of the pairs -> coef[order[k]], totals
    /// updated and neighbouring order entries swapped; an index past 15 ends the block
    /// (JXW_TRUNC)  [place]
    #[inline(always)]
    fn place(rl: &[[i32; 2]; 16], s: &mut Scan, coef: &mut [i32; 16], np: usize, first: u32, warn: &mut u32) {
        let mut k = first;
        for p in rl.iter().take(np) {
            k = k.wrapping_add(p[0] as u32);
            if k > 15 {
                *warn |= JXW_TRUNC;
                return;
            }
            let ku = k as usize;
            coef[(s.ord[ku] & 15) as usize] = p[1];
            let t = s.tot[ku] as u32 + 1;
            s.tot[ku] = t as u16;
            if ku > 1 {
                let prev = s.tot[ku - 1] as u32;
                if t > prev {
                    s.tot[ku - 1] = t as u16;
                    s.tot[ku] = prev as u16;
                    s.ord.swap(ku - 1, ku);
                }
            }
            k += 1;
        }
    }

    // ---------------------------------------------------------------- DECODE_BLOCK
    /// DECODE_BLOCK -> number of (run, level) pairs in rl; a run past the block end stops the
    /// block (JXW_TRUNC)  [decblock]
    #[inline(always)]
    fn decblock(&mut self, off: usize, chroma: usize, loc0: u32) -> usize {
        let d = self.cs;
        let mut r = self.r;
        let je = &mut *self.je[self.pi];
        let a1 = off + 3 * chroma;
        let alev = off + 6;
        let mut loc = loc0;
        let mut np = 0usize;
        let mut bad = false;
        r.fill48(d);
        // FIRST_INDEX
        let idx = r.vlc_nf(&mut je.ah[a1]);
        let sign = r.take1().wrapping_neg() as i32;
        let srn0 = idx >> 2;
        let mut cont = (idx & 1 & srn0) as usize;
        let mut level = 1;
        if idx & 2 != 0 {
            level = db_level(&mut r, d, &mut je.ah[alev + cont]);
        }
        self.rl[0][1] = (level ^ sign).wrapping_sub(sign);
        let mut run = 0;
        'blk: {
            if idx & 1 == 0 {
                let mr = 15i32 - loc as i32;
                if mr <= 0 {
                    bad = true;
                    break 'blk;
                }
                run = db_run(&mut r, mr as u32);
            }
            self.rl[0][0] = run;
            loc = loc.wrapping_add(run as u32 + 1);
            np = 1;
            let mut srn = srn0;
            loop {
                if srn == 0 {
                    break;
                }
                if np >= 16 {
                    bad = true;
                    break;
                }
                r.fill48(d);
                let mut run = 0;
                if srn & 1 == 0 {
                    let mr = 15i32 - loc as i32;
                    if mr <= 0 {
                        bad = true;
                        break;
                    }
                    run = db_run(&mut r, mr as u32);
                }
                self.rl[np][0] = run;
                loc = loc.wrapping_add(run as u32 + 1);
                let idx = if loc < 15 {
                    r.vlc_nf(&mut je.ah[a1 + 1 + cont]) // INDEX_A
                } else if loc == 15 {
                    // INDEX_B: 0 / 10 / 110 / 111
                    if r.take1() == 0 {
                        0
                    } else if r.take1() == 0 {
                        2
                    } else {
                        r.take1() * 2 + 1
                    }
                } else if loc == 16 {
                    r.take1() // INDEX_C_FLAG
                } else {
                    bad = true;
                    break;
                };
                let sign = r.take1().wrapping_neg() as i32;
                cont &= (idx >> 1) as usize;
                let mut level = 1;
                if idx & 1 != 0 {
                    level = db_level(&mut r, d, &mut je.ah[alev + cont]);
                }
                self.rl[np][1] = (level ^ sign).wrapping_sub(sign);
                np += 1;
                srn = idx >> 1;
            }
        }
        if bad {
            self.warn |= JXW_TRUNC;
        }
        self.r = r;
        np
    }

    // ---------------------------------------------------------------- refinement
    /// REFINE_LP / DECODE_FLEX of one coefficient  [refine1]
    fn refine1(&mut self, c: usize, k: usize, n: u32, trim: u32, mbits: u32) {
        let rf = self.getv(n).wrapping_shl(trim) as i32;
        let v = self.lp[c][k];
        let nv = if v > 0 {
            (v.wrapping_shl(mbits)).wrapping_add(rf)
        } else if v < 0 {
            (v.wrapping_neg().wrapping_shl(mbits)).wrapping_add(rf).wrapping_neg()
        } else if rf == 0 {
            0
        } else if self.bit1() != 0 {
            rf.wrapping_neg()
        } else {
            rf
        };
        self.lp[c][k] = nv;
    }

    /// refine coefficients 1..15: v = (v << mbits) +- (ref << trim), a new coefficient
    /// reads a sign; model bits 0: nothing  [refblk]
    #[inline(always)]
    fn refblk(rp: &mut Rd, d: &[u8], coef: &mut [i32; 16], mbits: u32, trim: u32) {
        if mbits == 0 {
            return;
        }
        let mut rl = *rp;
        let r = &mut rl;
        let n = (mbits as i32 - trim as i32).max(0) as u32;
        for v in coef.iter_mut().skip(1) {
            if r.cnt < 16 {
                r.refill(d);
            }
            let rf = (((r.buf >> 1) >> (63 - n)) as u32).wrapping_shl(trim);
            r.buf <<= n;
            r.cnt = r.cnt.wrapping_sub(n as u64);
            let x = *v;
            let newc = ((x == 0) & (rf != 0)) as u32;
            let sbit = ((r.buf >> 63) as u32) & newc;
            r.buf <<= newc;
            r.cnt = r.cnt.wrapping_sub(newc as u64);
            let s = (sbit.wrapping_neg() as i32) | (x >> 31);
            let a = (x ^ (x >> 31)).wrapping_sub(x >> 31);
            let y = (a.wrapping_shl(mbits)).wrapping_add(rf as i32);
            *v = (y ^ s).wrapping_sub(s);
        }
        *rp = rl;
    }

    /// no FLEXBITS band: coefficients << model bits  [shlblk]
    fn shlblk(coef: &mut [i32; 16], mbits: u32) {
        for v in coef.iter_mut().skip(1) {
            *v = v.wrapping_shl(mbits);
        }
    }

    // ---------------------------------------------------------------- DC band
    /// MB_DC of the current plane into lp[c][0]  [mb_dc]
    fn mb_dc(&mut self) {
        let d = self.cs;
        let pi = self.pi;
        self.lm = [0, 0];
        let cf = self.cf;
        let joint = cf != 0 && cf < 4;
        let mut flags = 7;
        if joint {
            flags = self.r.vlc(d, &mut self.je[pi].ah[2]); // VAL_DC_YUV
        }
        for c in 0..self.nch as usize {
            let have = if joint { (flags >> (2 - c as u32)) & 1 } else { self.bit1() };
            let mut idc: i32 = 0;
            let c1 = (c != 0) as usize;
            if have != 0 {
                self.lm[c1] += 1;
                let chroma = if cf.wrapping_sub(1) < 3 { c1 } else { 0 };
                idc = abslevel(&mut self.r, d, &mut self.je[pi].ah[3 + chroma]) - 1;
            }
            let bits = self.je[pi].md[0].bits[c1] as u32;
            idc = idc.wrapping_shl(bits) | self.getv(bits) as i32;
            if idc != 0 && self.bit1() != 0 {
                idc = idc.wrapping_neg();
            }
            self.lp[c][0] = idc;
        }
        self.updmodel(0);
    }

    // ---------------------------------------------------------------- LP band
    /// CBPLP_YUV1 / CBPLP_YUV2 and UpdateCountCBPLP  [cbplp]
    fn cbplp(&mut self, full: u32) -> u32 {
        let pi = self.pi;
        let imax = full * 4 - 5;
        let (lpz, lpm) = (self.je[pi].lpz, self.je[pi].lpm);
        let v = if lpz > 0 && lpm >= 0 {
            self.getv(full)
        } else {
            let mut v = if self.bit1() == 0 {
                0
            } else {
                let x = self.getv(full - 1);
                if x == 0 { 1 } else { x * 2 + self.bit1() }
            };
            if lpm < lpz {
                v = imax.wrapping_sub(v);
            }
            v
        };
        let je = &mut self.je[pi];
        je.lpz = (if v != 0 { je.lpz + 1 } else { je.lpz - 3 }).clamp(-8, 7);
        je.lpm = (if v != imax { je.lpm + 1 } else { je.lpm - 3 }).clamp(-8, 7);
        v
    }

    /// one component's LP block: DECODE_BLOCK (location 1) + AdaptiveLPScan  [lpblock]
    fn lpblock(&mut self, c: usize) {
        let c1 = (c != 0) as usize;
        let n = self.decblock(5, c1, 1);
        self.lm[c1] += n as i32;
        let je = &mut *self.je[self.pi];
        Self::place(&self.rl, &mut je.scan[0], &mut self.lp[c], n, 1, &mut self.warn);
    }

    /// MB_LP of the current plane: CBPLP, blocks, refinement into lp[c][1..]  [mb_lp]
    fn mb_lp(&mut self) {
        let nch = self.nch as usize;
        for c in 0..nch {
            for v in self.lp[c][1..].iter_mut() {
                *v = 0;
            }
        }
        if self.bands >= 3 {
            return;
        }
        self.lm = [0, 0];
        let pi = self.pi;
        let cf = self.cf;
        if cf == 1 || cf == 2 {
            // luma block, then U and V interleaved
            let cbp = self.cbplp(2);
            if cbp & 1 != 0 {
                self.lpblock(0);
            }
            let b = self.je[pi].md[1].bits[0] as u32;
            Self::refblk(&mut self.r, self.cs, &mut self.lp[0], b, 0);
            if cbp & 2 != 0 {
                let loc = if cf == 1 { 10 } else { 2 };
                let n = self.decblock(5, 1, loc);
                self.lm[1] += n as i32;
                let cnt = 16 - loc;
                let pos: &[u8] = if cf == 1 { &UVPOS420 } else { &UVPOS422 };
                let mut i = 0u32;
                for p in 0..n {
                    i = i.wrapping_add(self.rl[p][0] as u32);
                    if i >= cnt {
                        self.warn |= JXW_TRUNC;
                        break;
                    }
                    let position = pos[(i >> 1) as usize] as usize;
                    let comp = (i & 1) as usize + 1;
                    self.lp[comp][position] = self.rl[p][1];
                    i += 1;
                }
            }
            let mb = self.je[pi].md[1].bits[1] as u32;
            if mb != 0 {
                let end = if cf == 1 { 4 } else { 8 };
                for p in 1..end {
                    self.refine1(1, p, mb, 0, mb);
                    self.refine1(2, p, mb, 0, mb);
                }
            }
        } else {
            let cbp = if cf == 3 {
                self.cbplp(3)
            } else {
                // YONLY / YUVK / NCOMPONENT: one CBPLP_CH_BIT per component
                let mut b = 0u32;
                for c in 0..nch {
                    b |= self.bit1() << c;
                }
                b
            };
            for c in 0..nch {
                if (cbp >> (c & 31)) & 1 != 0 {
                    self.lpblock(c);
                }
                let b = self.je[pi].md[1].bits[(c != 0) as usize] as u32;
                Self::refblk(&mut self.r, self.cs, &mut self.lp[c], b, 0);
            }
        }
        self.updmodel(1);
    }

    // ---------------------------------------------------------------- DC / LP prediction
    /// DC / LP prediction (9.6.1 / 9.6.2) of lp, stored into the LPQ record; sets dcm  [pred]
    fn pred(&mut self) {
        let (mx, my) = (self.mx, self.my);
        let nch = self.nch as usize;
        let cf = self.cf;
        let left = mx > self.tx0;
        let top = my > self.ty0;
        let lrec = if left { self.mbrec(mx - 1, my) } else { 0 };
        let trec = if top { self.mbrec(mx, my - 1) } else { 0 };
        let tl = if left && top { self.mbrec(mx - 1, my - 1) } else { 0 };
        let rec = self.mbrec(mx, my);
        let crec = self.cbprec(mx, my);
        let lcr = if left { self.cbprec(mx - 1, my) } else { 0 };
        let tcr = if top { self.cbprec(mx, my - 1) } else { 0 };
        let pl = &mut self.jx.pl[self.pi];
        let q = &pl.lpq;
        let dcm = if !left {
            if !top { 3 } else { 1 }
        } else if !top {
            0
        } else {
            let mut sh = q[tl].wrapping_sub(q[lrec]).wrapping_abs();
            let mut sv = q[tl].wrapping_sub(q[trec]).wrapping_abs();
            if cf != 0 && cf != 6 {
                let f = if cf == 1 {
                    8
                } else if cf == 2 {
                    4
                } else {
                    2
                };
                sh = sh.wrapping_mul(f);
                sv = sv.wrapping_mul(f);
                for c in 1..3 {
                    let o = c * 16;
                    sh = sh.wrapping_add(q[tl + o].wrapping_sub(q[lrec + o]).wrapping_abs());
                    sv = sv.wrapping_add(q[tl + o].wrapping_sub(q[trec + o]).wrapping_abs());
                }
            }
            if sh.wrapping_mul(4) < sv {
                1
            } else if sv.wrapping_mul(4) < sh {
                0
            } else {
                2
            }
        };
        self.dcm = dcm;
        // MBLPMode: DC mode 0 / 1 with the same LP QP index as the neighbour
        let mut lpm = 2;
        if dcm <= 1 {
            let cr = if dcm == 0 { lcr } else { tcr };
            if pl.cbp[cr + nch] == self.lpqi {
                lpm = dcm;
            }
        }
        for c in 0..nch {
            let o = c * 16;
            let cur = &mut self.lp[c];
            match dcm {
                1 => cur[0] = cur[0].wrapping_add(q[trec + o]),
                0 => cur[0] = cur[0].wrapping_add(q[lrec + o]),
                2 => {
                    let mut s = q[lrec + o].wrapping_add(q[trec + o]);
                    if c != 0 && (cf == 1 || cf == 2) {
                        s = s.wrapping_add(1);
                    }
                    cur[0] = cur[0].wrapping_add(s >> 1);
                }
                _ => {}
            }
            if c != 0 && cf == 1 {
                match lpm {
                    0 => cur[1] = cur[1].wrapping_add(q[lrec + o + 1]),
                    1 => cur[2] = cur[2].wrapping_add(q[trec + o + 2]),
                    _ => {}
                }
            } else if c != 0 && cf == 2 {
                match lpm {
                    0 => {
                        for k in [1, 4, 5] {
                            cur[k] = cur[k].wrapping_add(q[lrec + o + k]);
                        }
                    }
                    1 => {
                        cur[4] = cur[4].wrapping_add(q[trec + o + 4]);
                        cur[2] = cur[2].wrapping_add(q[trec + o + 6]);
                        cur[6] = cur[6].wrapping_add(cur[2]);
                    }
                    _ => {
                        if dcm == 1 {
                            cur[6] = cur[6].wrapping_add(cur[2]);
                        }
                    }
                }
            } else {
                match lpm {
                    0 => {
                        for k in 1..4 {
                            cur[k] = cur[k].wrapping_add(q[lrec + o + k]);
                        }
                    }
                    1 => {
                        for k in [4, 8, 12] {
                            cur[k] = cur[k].wrapping_add(q[trec + o + k]);
                        }
                    }
                    _ => {}
                }
            }
        }
        // store the macroblock record and its LP QP index
        for c in 0..nch {
            pl.lpq[rec + c * 16..rec + c * 16 + 16].copy_from_slice(&self.lp[c]);
        }
        pl.cbp[crec + nch] = self.lpqi;
    }

    // ---------------------------------------------------------------- LP dequantisation
    /// dequantise DC / LP of the macroblock and run the first level inverse transform into
    /// the DC grids (scaled chroma x2)  [lpfin]
    fn lpfin(&mut self) {
        let je = &*self.je[self.pi];
        let pl = &mut self.jx.pl[self.pi];
        let cf = self.cf;
        for c in 0..self.nch as usize {
            let qdc = je.qdc[c] as i32;
            let qlp = je.qlp[(self.lpqi & 15) as usize][c] as i32;
            let co = &self.lp[c];
            let mut tv = [0i32; 16];
            let (cw, ch);
            if c != 0 && cf == 1 {
                tv[0] = co[0].wrapping_mul(qdc);
                for k in 1..4 {
                    tv[k] = co[k].wrapping_mul(qlp);
                }
                t2x2h(&mut tv, 0, 0);
                cw = 2;
                ch = 2;
            } else if c != 0 && cf == 2 {
                let a = co[0].wrapping_mul(qdc);
                let b = co[4].wrapping_mul(qlp);
                let topv = a.wrapping_sub(b.wrapping_add(1) >> 1);
                tv[0] = topv;
                tv[4] = b.wrapping_add(topv);
                for k in 1..4 {
                    tv[k] = co[k].wrapping_mul(qlp);
                    tv[k + 4] = co[k + 4].wrapping_mul(qlp);
                }
                t2x2h(&mut tv, 0, 0);
                t2x2h(&mut tv, 4, 0);
                cw = 2;
                ch = 4;
            } else {
                tv[0] = co[0].wrapping_mul(qdc);
                for k in 1..16 {
                    tv[F2R[k] as usize] = co[k].wrapping_mul(qlp);
                }
                ict4x4(&mut tv);
                cw = 4;
                ch = 4;
            }
            if self.scaled != 0 && c != 0 {
                for v in tv[..cw * ch].iter_mut() {
                    *v = v.wrapping_shl(1);
                }
            }
            let gw = pl.gw[c] as usize;
            let first = (self.my as usize * ch) * gw + self.mx as usize * cw;
            let g = &mut pl.dcg[c];
            for rr in 0..ch {
                let o = first + rr * gw;
                g[o..o + cw].copy_from_slice(&tv[rr * cw..rr * cw + cw]);
            }
        }
    }

    // ---------------------------------------------------------------- CBPHP
    /// REFINE_CBPHP(n)  [refcbp]
    fn refcbp(&mut self, n: u32) -> u32 {
        match n {
            2 => {
                let v = self.getv(2);
                if v < 2 {
                    v * 2 + 3
                } else {
                    let b = self.bit1();
                    REF1[(b + v * 2 - 4) as usize] as u32
                }
            }
            1 => 1 << self.getv(2),
            3 => (1 << self.getv(2)) ^ 15,
            4 => 15,
            _ => 0,
        }
    }

    /// NUM_CH_BLK (1 -> 0, 01 -> 1, 000 -> 2, 001 -> 3)  [code4]
    fn code4(&mut self) -> u32 {
        if self.bit1() != 0 {
            return 0;
        }
        if self.bit1() != 0 {
            return 1;
        }
        self.bit1() + 2
    }

    /// UpdateCBPHPModel  [cbpupd]
    fn cbpupd(&mut self, c1: usize, ones: u32) {
        let je = &mut self.je[self.pi];
        let c0 = (je.cbc0[c1] + ones as i32 - 3).clamp(-16, 15);
        je.cbc0[c1] = c0;
        let c1v = (je.cbc1[c1] - ones as i32 + 13).clamp(-16, 15);
        je.cbc1[c1] = c1v;
        je.cbst[c1] = if c0 < 0 {
            if c0 >= c1v { 2 } else { 1 }
        } else if c1v < 0 {
            2
        } else {
            0
        };
    }

    /// the prediction bit for block 0: 1 at the tile's top-left MB, else the adjacent block
    /// of the top / left MB  [cbpedge]
    fn cbpedge(&self, i: usize, shift: u32) -> u32 {
        let pl = &self.jx.pl[self.pi];
        if self.mx == self.tx0 {
            if self.my == self.ty0 {
                return 1;
            }
            let r = self.cbprec(self.mx, self.my - 1);
            return (pl.cbp[r + i] >> shift) & 1;
        }
        let r = self.cbprec(self.mx - 1, self.my);
        let s = if shift == 10 { 5 } else { 1 };
        (pl.cbp[r + i] >> s) & 1
    }

    /// MB_CBPHP + PredCBPHP of the current plane into cbp and the CBP record  [mb_cbphp]
    fn mb_cbphp(&mut self) {
        let d = self.cs;
        let pi = self.pi;
        let nch = self.nch as usize;
        let cf = self.cf;
        for v in self.diff[..nch].iter_mut() {
            *v = 0;
        }
        let ncomp = if cf == 4 || cf == 6 { nch } else { 1 };
        for i in 0..ncomp {
            let n = self.r.vlc(d, &mut self.je[pi].ah[AHI_NCBP]); // NUM_CBPHP
            let groups = self.refcbp(n);
            for g in 0..4u32 {
                if (groups >> g) & 1 == 0 {
                    continue;
                }
                let ai = if cf == 0 || cf >= 4 { AHI_NBLK5 } else { AHI_NBLK9 };
                let mut val = self.r.vlc(d, &mut self.je[pi].ah[ai]) + 1; // NUM_BLKCBPHP
                let mut blk = 0u32;
                if val >= 6 {
                    blk = (self.r.code3(d) + 1) << 4; // CHR_CBPHP
                    if val >= 9 {
                        val += self.r.code3(d); // VAL_INC
                    }
                    val -= 6;
                }
                let flc = CBPFLC[val as usize] as u32;
                let off = CBPOFF[val as usize] as u32;
                let x = self.getv(flc) + off; // CODE_INC
                blk += CBPOUT[x as usize] as u32;
                if (1..4).contains(&cf) {
                    self.diff[0] |= (blk & 15) << (g * 4); // luma of YUV formats
                    for k in 0..2 {
                        if (blk >> (4 + k)) & 1 == 0 {
                            continue;
                        }
                        let v = match cf {
                            1 => 1 << g,
                            2 => {
                                let s = self.r.code3(d); // CBPHP_CH_BLK
                                (CH422[s as usize] as u32) << CH422SH[g as usize]
                            }
                            _ => {
                                let nb = self.code4() + 1; // NUM_CH_BLK (444)
                                self.refcbp(nb) << (g * 4)
                            }
                        };
                        self.diff[1 + k] |= v;
                    }
                } else {
                    self.diff[i] |= blk.wrapping_shl(g * 4);
                }
            }
        }
        // ---- prediction
        let np = if cf == 1 || cf == 2 { 1 } else { nch };
        for i in 0..np {
            let mut dv = self.diff[i];
            let c1 = (i != 0) as usize;
            let st = self.je[pi].cbst[c1];
            if st == 0 {
                dv ^= self.cbpedge(i, 10);
                dv ^= (dv << 1) & 0x02;
                dv ^= (dv << 3) & 0x10;
                dv ^= (dv << 1) & 0x20;
                dv ^= (dv & 0x33) << 2;
                dv ^= (dv & 0xCC) << 6;
                dv ^= (dv & 0x3300) << 2;
            } else if st >= 2 {
                dv ^= 0xFFFF;
            }
            self.cbpupd(c1, pop16(dv));
            self.cbp[i] = dv;
        }
        if cf == 1 || cf == 2 {
            for i in 1..3 {
                let mut dv = self.diff[i];
                let st = self.je[pi].cbst[1];
                let ones;
                if cf == 1 {
                    // 4:2:0 -- 2 x 2 blocks
                    if st == 0 {
                        dv ^= self.cbpedge(i, 2);
                        dv ^= (dv << 1) & 0x02;
                        dv ^= (dv & 3) << 2;
                    } else if st >= 2 {
                        dv ^= 0x0F;
                    }
                    ones = pop16(dv) << 2;
                } else {
                    // 4:2:2 -- 2 wide x 4 high
                    if st == 0 {
                        dv ^= self.cbpedge(i, 6);
                        dv ^= (dv & 1) << 1;
                        dv ^= (dv & 3) << 2;
                        dv ^= (dv & 0x0C) << 2;
                        dv ^= (dv & 0x30) << 2;
                    } else if st >= 2 {
                        dv ^= 0xFF;
                    }
                    ones = pop16(dv) * 2;
                }
                self.cbpupd(1, ones);
                self.cbp[i] = dv;
            }
        }
        let r = self.cbprec(self.mx, self.my);
        self.jx.pl[pi].cbp[r..r + nch].copy_from_slice(&self.cbp[..nch]);
    }

    // ---------------------------------------------------------------- HP band
    /// blocks of a component (16, chroma 4 / 8 for 420 / 422)  [nblk]
    fn nblk(&self, c: usize) -> usize {
        if c == 0 {
            16
        } else if self.cf == 1 {
            4
        } else if self.cf == 2 {
            8
        } else {
            16
        }
    }

    /// FLEXBITS of one block (model bits of the macroblock, TRIM_FLEXBITS)  [hpflex1]
    #[inline]
    fn hpflex1(&mut self, c: usize, b: usize) {
        let mb = self.hb[(c != 0) as usize] as u32;
        Self::refblk(&mut self.r, self.cs, &mut self.hp[c * 16 + b], mb, self.trim);
    }

    /// HP prediction mode, MB_CBPHP, HP blocks with FLEXBITS, HP prediction, dequantisation
    /// into the sample planes  [mb_hp]
    fn mb_hp(&mut self) {
        let nch = self.nch as usize;
        let cf = self.cf;
        let pi = self.pi;
        // CalcHPPredMode from the predicted LP coefficients
        let l = &self.lp;
        let mut sh = 0i32;
        let mut sv = 0i32;
        for k in 1..4 {
            sh = sh.wrapping_add(l[0][k].wrapping_abs());
            sv = sv.wrapping_add(l[0][k * 4].wrapping_abs());
        }
        if cf != 0 && cf != 6 {
            let ti = if cf >= 3 { 4 } else { 2 };
            for c in 1..3 {
                sh = sh.wrapping_add(l[c][1].wrapping_abs());
                sv = sv.wrapping_add(l[c][ti].wrapping_abs());
                if cf == 2 {
                    sh = sh.wrapping_add(l[c][5].wrapping_abs());
                    sv = sv.wrapping_add(l[c][6].wrapping_abs());
                }
            }
        }
        self.acm = if sh.wrapping_mul(4) < sv {
            1
        } else if sv.wrapping_mul(4) < sh {
            0
        } else {
            2
        };
        self.hb = self.je[pi].md[2].bits;
        self.mb_cbphp();
        for b in self.hp[..nch * 16].iter_mut() {
            *b = [0; 16];
        }
        self.lm = [0, 0];
        let flexi = self.bands == 0 && self.freq == 0;
        let si = if self.acm == 1 { 2 } else { 1 };
        for c in 0..nch {
            let nb = self.nblk(c);
            let c1 = (c != 0) as usize;
            for b in 0..nb {
                if (self.cbp[c] >> b) & 1 != 0 {
                    let n = self.decblock(13, c1, 1);
                    self.lm[c1] += n as i32;
                    let je = &mut *self.je[pi];
                    Self::place(&self.rl, &mut je.scan[si], &mut self.hp[c * 16 + b], n, 1, &mut self.warn);
                }
                if flexi {
                    self.hpflex1(c, b);
                }
            }
        }
        self.updmodel(2);
        if self.bands == 0 {
            if self.freq != 0 {
                self.sel_rd(3);
                for c in 0..nch {
                    for b in 0..self.nblk(c) {
                        self.hpflex1(c, b);
                    }
                }
            }
        } else {
            for c in 0..nch {
                let mb = self.hb[(c != 0) as usize] as u32;
                for b in 0..self.nblk(c) {
                    Self::shlblk(&mut self.hp[c * 16 + b], mb);
                }
            }
        }
        self.hppred();
        self.hpfin();
    }

    /// HP prediction inside the macroblock (left: coefficients 1..3 from the block to the left,
    /// top: 4 / 8 / 12 from the block above)  [hppred]
    fn hppred(&mut self) {
        let acm = self.acm;
        if acm == 2 {
            return;
        }
        for c in 0..self.nch as usize {
            let geo = if c == 0 {
                0
            } else if self.cf == 1 {
                1
            } else if self.cf == 2 {
                2
            } else {
                0
            };
            let pairs: &[(u8, u8)] = match (geo, acm) {
                (0, 0) => &HPP0L,
                (0, _) => &HPP0T,
                (1, 0) => &HPP1L,
                (1, _) => &HPP1T,
                (_, 0) => &HPP2L,
                _ => &HPP2T,
            };
            let hp = &mut self.hp[c * 16..c * 16 + 16];
            for &(dst, src) in pairs {
                let s = hp[src as usize];
                let t = &mut hp[dst as usize];
                if acm == 0 {
                    for k in 1..4 {
                        t[k] = t[k].wrapping_add(s[k]);
                    }
                } else {
                    for k in [4, 8, 12] {
                        t[k] = t[k].wrapping_add(s[k]);
                    }
                }
            }
        }
    }

    /// dequantise the HP blocks and store them into the sample planes (4x4 raster of the ICT
    /// input, DC slot untouched)  [hpfin]
    fn hpfin(&mut self) {
        let je = &*self.je[self.pi];
        let pl = &mut self.jx.pl[self.pi];
        for c in 0..self.nch as usize {
            let q = je.qhp[(self.hpqi & 15) as usize][c] as i32;
            let pw = pl.pw[c] as usize;
            let (bw, bh, nb, tab): (usize, usize, usize, &[u8; 16]) = if c == 0 {
                (4, 4, 16, &QUAD)
            } else if self.cf == 1 {
                (2, 2, 4, &RAST2)
            } else if self.cf == 2 {
                (2, 4, 8, &RAST2)
            } else {
                (4, 4, 16, &QUAD)
            };
            let origin = (self.my as usize * bh * 4) * pw + self.mx as usize * bw * 4;
            let buf = &mut pl.buf[c];
            for b in 0..nb {
                let rc = tab[b] as usize;
                let bo = origin + ((rc >> 4) * 4) * pw + (rc & 15) * 4;
                let blk = &self.hp[c * 16 + b];
                // the block in raster order; slot 0 (the DC, from the grid later) stays 0
                let mut ras = [0i32; 16];
                for k in 1..16 {
                    ras[F2R[k] as usize] = blk[k].wrapping_mul(q);
                }
                for r in 0..4 {
                    let o = bo + r * pw;
                    buf[o..o + 4].copy_from_slice(&ras[r * 4..r * 4 + 4]);
                }
            }
        }
    }

    // ---------------------------------------------------------------- quantisation parameters
    /// QP indices of one set for the components (COMPONENT_MODE uniform / separate /
    /// independent)  [qpset]
    fn qpset(&mut self, dst: &mut [u8; 16]) -> Result<(), i32> {
        let mode = if self.nch == 1 { 0 } else { self.getv(2) };
        if mode == 3 {
            return Err(ERR_HDR);
        }
        let v = self.getv(8);
        self.qpmark(v);
        dst[0] = v as u8;
        match mode {
            0 => {
                for x in dst[1..].iter_mut() {
                    *x = v as u8;
                }
            }
            1 => {
                let v2 = self.getv(8);
                self.qpmark(v2);
                for x in dst[1..].iter_mut() {
                    *x = v2 as u8;
                }
            }
            _ => {
                for c in 1..self.nch as usize {
                    let v = self.getv(8);
                    self.qpmark(v);
                    dst[c & 15] = v as u8;
                }
            }
        }
        Ok(())
    }

    /// JXW_QP when QP > 0  [qpmark]
    fn qpmark(&mut self, v: u32) {
        if v != 0 {
            self.warn |= JXW_QP;
        }
    }

    /// DECODE_QP_INDEX, clamped to the sets  [qpindex]
    fn qpindex(&mut self, n: u32) -> u32 {
        if self.bit1() == 0 {
            return 0;
        }
        let v = self.getv(QPBITS[(n & 31).min(16) as usize] as u32) + 1;
        if v < n { v } else { 0 }
    }

    /// TILE_HEADER_DC / _LOWPASS / _HIGHPASS of the current plane and the quantisers  [tileqp]
    fn tileqp(&mut self) -> Result<(), i32> {
        let pi = self.pi;
        self.sel_rd(0);
        if self.jx.pl[pi].dcu == 0 {
            let mut t = self.je[pi].idc;
            self.qpset(&mut t)?;
            self.je[pi].idc = t;
        } else {
            self.je[pi].idc = self.jx.pl[pi].qp[0];
        }
        {
            let je = &mut self.je[pi];
            je.nlpq = 1;
            je.nhpq = 1;
            je.usedc = 0;
            je.uselp = 0;
        }
        if self.bands < 3 {
            self.sel_rd(1);
            if self.jx.pl[pi].lpu == 0 {
                let u = self.bit1();
                self.je[pi].usedc = u;
                if u != 0 {
                    self.je[pi].ilp[0] = self.je[pi].idc;
                } else {
                    let n = self.getv(4) + 1;
                    self.je[pi].nlpq = n;
                    for s in 0..n as usize {
                        let mut t = self.je[pi].ilp[s];
                        self.qpset(&mut t)?;
                        self.je[pi].ilp[s] = t;
                    }
                }
            } else {
                self.je[pi].ilp[0] = self.jx.pl[pi].qp[1];
            }
            if self.bands < 2 {
                self.sel_rd(2);
                if self.jx.pl[pi].hpu == 0 {
                    let u = self.bit1();
                    self.je[pi].uselp = u;
                    if u != 0 {
                        let je = &mut self.je[pi];
                        je.nhpq = je.nlpq;
                        je.ihp = je.ilp;
                    } else {
                        let n = self.getv(4) + 1;
                        self.je[pi].nhpq = n;
                        for s in 0..n as usize {
                            let mut t = self.je[pi].ihp[s];
                            self.qpset(&mut t)?;
                            self.je[pi].ihp[s] = t;
                        }
                    }
                } else {
                    self.je[pi].ihp[0] = self.jx.pl[pi].qp[2];
                }
            }
        }
        // quantisers: DC / LP shift 1 for component 0, 0 otherwise; HP shift 1
        let sc = self.scaled != 0;
        let je = &mut self.je[pi];
        for c in 0..16 {
            je.qdc[c] = quantmap(je.idc[c] as u32, (c == 0) as u32, sc);
        }
        for s in 0..16 {
            for c in 0..16 {
                je.qlp[s][c] = quantmap(je.ilp[s][c] as u32, (c == 0) as u32, sc);
                je.qhp[s][c] = quantmap(je.ihp[s][c] as u32, 1, sc);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- headers
    /// IMAGE_HEADER into jx (tile arrays allocated)  [hdr_image]
    fn hdr_image(&mut self) -> Result<(), i32> {
        if self.getv(32) != 0x574D_5048 || self.getv(32) != 0x4F54_4F00 {
            return Err(ERR_HDR); // "WMPHOTO\0"
        }
        self.getv(4); // VERSION
        let mut fl = 0;
        if self.bit1() != 0 {
            fl |= JXF_HARDTILE;
        }
        self.getv(3); // sub-version
        if self.bit1() != 0 {
            fl |= JXF_TILING;
        }
        if self.bit1() != 0 {
            fl |= JXF_FREQ;
        }
        self.jx.xform = self.getv(3);
        if self.bit1() != 0 {
            fl |= JXF_INDEX;
        }
        let ovl = self.getv(2);
        if ovl == 3 {
            return Err(ERR_HDR);
        }
        self.jx.ovl = ovl;
        for f in [JXF_SHORT, JXF_LONGW, JXF_WINDOW, JXF_TRIM] {
            if self.bit1() != 0 {
                fl |= f;
            }
        }
        self.bit1(); // RESERVED_D
        for f in [JXF_RBNS, JXF_PREMUL, JXF_ALPHA] {
            if self.bit1() != 0 {
                fl |= f;
            }
        }
        self.jx.flags = fl;
        self.jx.ocf = self.getv(4);
        self.jx.obd = self.getv(4);
        let wb = if fl & JXF_SHORT != 0 { 16 } else { 32 };
        let w1 = self.getv(wb);
        let h1 = self.getv(wb);
        if w1 > 65534 || h1 > 65534 {
            return Err(ERR_DIMS);
        }
        self.jx.w = w1 + 1;
        self.jx.h = h1 + 1;
        self.jx.ntx = 1;
        self.jx.nty = 1;
        if fl & JXF_TILING != 0 {
            self.jx.ntx = self.getv(12) + 1;
            self.jx.nty = self.getv(12) + 1;
        }
        // tile sizes are read into the arrays first, turned into start columns after the margins
        let mut tx = vec![0u32; self.jx.ntx as usize + 1];
        let mut ty = vec![0u32; self.jx.nty as usize + 1];
        let tb = if fl & JXF_SHORT != 0 { 8 } else { 16 };
        for v in tx[1..self.jx.ntx as usize].iter_mut() {
            *v = self.getv(tb);
        }
        for v in ty[1..self.jx.nty as usize].iter_mut() {
            *v = self.getv(tb);
        }
        if fl & JXF_WINDOW != 0 {
            self.jx.mt = self.getv(6);
            self.jx.ml = self.getv(6);
            self.jx.mbot = self.getv(6);
            self.jx.mr = self.getv(6);
        } else {
            self.jx.mbot = self.jx.h.wrapping_neg() & 15;
            self.jx.mr = self.jx.w.wrapping_neg() & 15;
        }
        let ew = self.jx.w + self.jx.ml + self.jx.mr;
        if ew & 15 != 0 {
            return Err(ERR_HDR);
        }
        self.jx.mbw = ew >> 4;
        let eh = self.jx.h + self.jx.mt + self.jx.mbot;
        if eh & 15 != 0 {
            return Err(ERR_HDR);
        }
        self.jx.mbh = eh >> 4;
        tilestarts(&mut tx, self.jx.mbw)?;
        tilestarts(&mut ty, self.jx.mbh)?;
        self.jx.tx = tx;
        self.jx.ty = ty;
        Ok(())
    }

    /// IMAGE_PLANE_HEADER  [hdr_plane]
    fn hdr_plane(&mut self, p: usize, alpha: bool) -> Result<(), i32> {
        let cf = self.getv(3);
        self.jx.pl[p].cf = cf;
        if (alpha && cf != 0) || cf == 5 || cf == 7 {
            return Err(ERR_HDR);
        }
        self.jx.pl[p].scaled = self.bit1();
        let bands = self.getv(4);
        if bands > 3 {
            return Err(ERR_HDR);
        }
        self.jx.pl[p].bands = bands;
        let nch;
        if cf == 0 {
            nch = 1;
        } else if cf <= 3 {
            nch = 3;
            if cf == 3 {
                self.getv(8); // RESERVED_F + RESERVED_H
            } else {
                self.bit1(); // RESERVED_E_BIT
                self.jx.pl[p].ccx = self.getv(3);
                if cf == 1 {
                    self.bit1(); // RESERVED_G_BIT
                    self.jx.pl[p].ccy = self.getv(3);
                } else {
                    self.getv(4);
                }
            }
        } else if cf == 4 {
            nch = 4;
        } else {
            // NCOMPONENT
            let n = self.getv(4);
            if n == 15 {
                return Err(ERR_UNSUPPORTED); // more than 16 components
            }
            self.getv(4); // RESERVED_H
            nch = n + 1;
        }
        self.jx.pl[p].nch = nch;
        match self.jx.obd {
            2 | 3 | 6 => self.jx.pl[p].shift = self.getv(8), // BD16 / BD16S / BD32S
            7 => {
                // BD32F
                self.jx.pl[p].mant = self.getv(8);
                self.jx.pl[p].ebias = self.getv(8) as u8 as i8 as i32;
            }
            _ => {}
        }
        self.nch = nch; // for qpset
        self.jx.pl[p].lpu = 1;
        self.jx.pl[p].hpu = 1;
        let dcu = self.bit1();
        self.jx.pl[p].dcu = dcu;
        if dcu != 0 {
            let mut t = self.jx.pl[p].qp[0];
            self.qpset(&mut t)?;
            self.jx.pl[p].qp[0] = t;
        }
        if bands < 3 {
            self.bit1(); // RESERVED_I_BIT
            let lpu = self.bit1();
            self.jx.pl[p].lpu = lpu;
            if lpu != 0 {
                let mut t = self.jx.pl[p].qp[1];
                self.qpset(&mut t)?;
                self.jx.pl[p].qp[1] = t;
            }
            if bands < 2 {
                self.bit1(); // RESERVED_J_BIT
                let hpu = self.bit1();
                self.jx.pl[p].hpu = hpu;
                if hpu != 0 {
                    let mut t = self.jx.pl[p].qp[2];
                    self.qpset(&mut t)?;
                    self.jx.pl[p].qp[2] = t;
                }
            }
        }
        self.r.bytepos();
        Ok(())
    }

    // ---------------------------------------------------------------- tiles
    /// make the reader current, check TILE_STARTCODE + ARBITRARY_BYTE of a present packet (a
    /// bad start code empties the packet: JXW_START)  [pkstart]
    fn pkstart(&mut self, i: u32) {
        self.sel_rd(i);
        if self.r.ptr == 0 {
            return; // absent packet: zero bits
        }
        let v = self.getv(24);
        self.getv(8);
        if v != 1 {
            self.warn |= JXW_START;
            self.r = Rd::default();
        }
    }

    /// decode one tile of all planes  [dec_tile]
    fn dec_tile(&mut self, tx: usize, ty: usize) -> Result<(), i32> {
        self.tx0 = self.jx.tx[tx];
        self.tx1 = self.jx.tx[tx + 1];
        self.ty0 = self.jx.ty[ty];
        self.ty1 = self.jx.ty[ty + 1];
        self.trim = 0;
        for i in 0..self.nb {
            self.pkstart(i);
        }
        if self.jx.flags & JXF_TRIM != 0 && (self.freq == 0 || self.nb >= 4) {
            self.sel_rd(3);
            self.trim = self.getv(4);
        }
        for p in 0..self.npl as usize {
            self.setplane(p);
            je_reset(&mut self.je[p]);
            self.tileqp()?;
        }
        self.my = self.ty0;
        while self.my < self.ty1 {
            if let Some(c) = self.cancel
                && c.load(Ordering::Relaxed) != 0
            {
                return Err(ERR_CANCEL);
            }
            self.mx = self.tx0;
            while self.mx < self.tx1 {
                for p in 0..self.npl as usize {
                    self.setplane(p);
                    if (self.mx - self.tx0) & 15 == 0 {
                        scan_totals_lp(&mut self.je[p]);
                        scan_totals_hp(&mut self.je[p]);
                    }
                    // QP indices (spatial mode: before MB_DC; frequency mode: in the LP / HP packets)
                    self.lpqi = 0;
                    self.hpqi = 0;
                    if self.bands < 3 {
                        self.sel_rd(1);
                        if self.je[p].nlpq > 1 && self.je[p].usedc == 0 {
                            let n = self.je[p].nlpq;
                            self.lpqi = self.qpindex(n);
                        }
                        if self.bands < 2 {
                            if self.je[p].uselp != 0 {
                                self.hpqi = self.lpqi;
                            } else if self.freq == 0 && self.je[p].nhpq > 1 {
                                let n = self.je[p].nhpq;
                                self.hpqi = self.qpindex(n);
                            }
                        }
                    }
                    self.sel_rd(0);
                    self.mb_dc();
                    self.sel_rd(1);
                    self.mb_lp();
                    self.pred();
                    self.lpfin();
                    if self.bands < 2 {
                        self.sel_rd(2);
                        if self.freq != 0 && self.je[p].uselp == 0 && self.je[p].nhpq > 1 {
                            let n = self.je[p].nhpq;
                            self.hpqi = self.qpindex(n);
                        }
                        self.mb_hp();
                    }
                }
                // adaptation after the last MB of the tile row / every 16th MB
                if self.mx + 1 == self.tx1 || (self.mx - self.tx0) & 15 == 0 {
                    for p in 0..self.npl as usize {
                        adapt_all(&mut self.je[p]);
                    }
                }
                self.mx += 1;
            }
            self.my += 1;
        }
        self.sel_rd(0); // the tile's reader 0 back in r
        Ok(())
    }

    /// JXW_TRUNC if a present packet was read past its end  [rd_overrun]
    fn rd_overrun(&mut self) {
        self.rds[self.cur as usize] = self.r;
        for rd in self.rds.iter() {
            if rd.ptr == 0 {
                continue;
            }
            let d = rd.ptr.wrapping_sub(rd.end) as i64;
            if d <= 0 {
                continue;
            }
            if (d as u64) << 3 > rd.cnt {
                self.warn |= JXW_TRUNC;
            }
        }
    }
}

/// converts the tile sizes (entries 1..n-1) into first-MB indices, [n] = total  [tilestarts]
fn tilestarts(a: &mut [u32], total: u32) -> Result<(), i32> {
    let n = a.len() - 1;
    a[0] = 0;
    let mut run = 0u32;
    for v in a[1..n].iter_mut() {
        if *v == 0 {
            return Err(ERR_HDR);
        }
        run = run.wrapping_add(*v);
        if run >= total {
            return Err(ERR_HDR);
        }
        *v = run;
    }
    a[n] = total;
    Ok(())
}

/// T2x2h in place on v[o..o + 4]
#[inline]
pub fn t2x2h(v: &mut [i32; 16], o: usize, rnd: i32) {
    let (mut a, mut b, c, d) = (v[o], v[o + 1], v[o + 2], v[o + 3]);
    a = a.wrapping_add(d);
    b = b.wrapping_sub(c);
    let t = a.wrapping_sub(b).wrapping_add(rnd) >> 1;
    let c2 = t.wrapping_sub(d);
    let d2 = t.wrapping_sub(c);
    a = a.wrapping_sub(d2);
    b = b.wrapping_add(c2);
    v[o] = a;
    v[o + 1] = b;
    v[o + 2] = c2;
    v[o + 3] = d2;
}

/// decode the codestream cs (dec: maximum pixels, cancel flag)  [jxe_decode]
pub fn decode(cs: &[u8], dec: Option<&ImgDec>) -> Result<Cs, i32> {
    let end = cs.len() as u64 + K;
    let mut d = Dec {
        cs,
        r: Rd { buf: 0, cnt: 0, ptr: K, end },
        rds: [Rd::default(); 4],
        cur: 0,
        jx: Cs::default(),
        je: [Box::default(), Box::default()],
        cancel: dec.and_then(|d| d.cancel),
        pi: 0,
        cf: 0,
        nch: 0,
        scaled: 0,
        bands: 0,
        tx0: 0,
        tx1: 0,
        ty0: 0,
        ty1: 0,
        mx: 0,
        my: 0,
        trim: 0,
        lpqi: 0,
        hpqi: 0,
        acm: 0,
        dcm: 0,
        freq: 0,
        mbw: 0,
        mbh: 0,
        npl: 0,
        lm: [0; 2],
        nb: 0,
        warn: 0,
        hb: [0; 2],
        rl: [[0; 2]; 16],
        diff: [0; 16],
        cbp: [0; 16],
        lp: [[0; 16]; 16],
        hp: vec![[0i32; 16]; 256],
    };
    d.hdr_image()?;
    let dd = ImgDec::default();
    let r = meta::dims_check(d.jx.w, d.jx.h, dec.unwrap_or(&dd));
    if r != 0 {
        return Err(r);
    }
    // hard tiling (overlap filters stop at tile edges) is not implemented: unsupported when it
    // changes the result (several tiles and overlap filtering)
    if d.jx.flags & JXF_HARDTILE != 0 && d.jx.ovl != 0 && (d.jx.ntx > 1 || d.jx.nty > 1) {
        return Err(ERR_UNSUPPORTED);
    }
    d.jx.npl = 1;
    d.hdr_plane(0, false)?;
    if d.jx.flags & JXF_ALPHA != 0 {
        d.jx.npl = 2;
        d.hdr_plane(1, true)?;
    }
    d.npl = d.jx.npl;
    d.mbw = d.jx.mbw;
    d.mbh = d.jx.mbh;
    let mut nb = 1;
    if d.jx.flags & JXF_FREQ != 0 {
        d.freq = 1;
        nb = 4u32.wrapping_sub(d.jx.pl[0].bands);
    }
    d.nb = nb;
    d.jx.nb = nb;
    // ---- index table
    let entries = (d.jx.ntx * d.jx.nty) as u64 * nb as u64;
    let mut have_idx = false;
    if d.jx.flags & JXF_INDEX != 0 {
        if d.getv(16) != 1 {
            return Err(ERR_HDR);
        }
        let p = d.r.bytepos();
        if entries > end.wrapping_sub(p) {
            return Err(ERR_TRUNC); // every entry takes at least one byte
        }
        let mut idx = vec![0u64; entries as usize];
        for v in idx.iter_mut() {
            let (x, ok) = d.r.vlwesc(cs);
            *v = if ok { x } else { u64::MAX };
        }
        d.jx.idx = idx;
        have_idx = true;
    } else if d.freq != 0 {
        return Err(ERR_UNSUPPORTED); // frequency mode needs the index table
    }
    let (n, ok) = d.r.vlwesc(cs); // SubsequentBytes
    let n = if ok { n } else { 0 };
    let p = d.r.bytepos();
    if n > end.wrapping_sub(p) {
        return Err(ERR_TRUNC);
    }
    let base = p.wrapping_add(n); // first tile packet
    d.r = Rd { buf: 0, cnt: 0, ptr: base, end };
    for v in d.jx.idx.iter_mut() {
        *v = if *v < end.wrapping_sub(base) { base + *v } else { 0 };
    }
    // ---- buffers
    for p in 0..d.npl as usize {
        d.setplane(p);
        let nch = d.nch as usize;
        let mbs = (d.mbw * d.mbh) as usize;
        let cf = d.cf;
        let pl = &mut d.jx.pl[p];
        pl.lpq = vec![0; mbs * nch * 16];
        pl.cbp = vec![0; mbs * (nch + 1)];
        for c in 0..nch {
            let mut w = d.mbw << 4;
            let mut h = d.mbh << 4;
            if c != 0 && cf == 1 {
                w >>= 1;
                h >>= 1;
            } else if c != 0 && cf == 2 {
                w >>= 1;
            }
            pl.pw[c] = w;
            pl.ph[c] = h;
            pl.gw[c] = w >> 2;
            pl.gh[c] = h >> 2;
            let n = w as usize * h as usize;
            pl.buf.push(vec![0; n]);
            pl.dcg.push(vec![0; n >> 4]);
        }
    }
    // ---- tiles
    for ty in 0..d.jx.nty as usize {
        for tx in 0..d.jx.ntx as usize {
            if have_idx {
                let bi = (ty * d.jx.ntx as usize + tx) * d.nb as usize;
                for i in 0..4 {
                    let mut rd = Rd::default();
                    if i < d.nb as usize {
                        let a = d.jx.idx[bi + i];
                        if a != 0 {
                            rd.ptr = a;
                            rd.end = end;
                        }
                    }
                    d.rds[i] = rd;
                }
                d.cur = 0;
                d.r = d.rds[0];
            } else {
                d.cur = 0;
            }
            d.dec_tile(tx, ty)?;
            d.rd_overrun();
            if !have_idx {
                d.r.bytepos(); // sequential tiles are byte aligned
            }
        }
    }
    d.jx.warn = d.warn;
    if d.warn & JXW_TRUNC != 0 {
        return Err(ERR_TRUNC);
    }
    if d.warn & JXW_START != 0 {
        return Err(ERR_HDR);
    }
    Ok(d.jx)
}
