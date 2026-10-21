// enc -- JPEG XR encoder (ITU-T T.832); the bit-exact model is tools/jxr_enc_ref.py
//                                                                    [src/img/jxr_enc.s]
//
//   encode(sf, opt, out) -> 0 / ERR_DIMS / ERR_CANCEL -- JPEG XR file of sf (any get_row format,
//       1..65535 pixels per side): Bgr24, or Bgra32 with the alpha channel as a second, planar
//       YONLY codestream (always lossless) when opt.bitdepth is 32, or 0 and some alpha < 255.
//       IOF_LOSSLESS: YUV444, unscaled arithmetic, OVERLAP 0, every QP 0.  Otherwise
//       opt.quality 1..100 (0 = 90) gives the QP indices like jxrlib (DPK_QPS tables, DC = LP,
//       U / V / HP separate), scaled arithmetic, OVERLAP 1, opt.chroma 422 / 420 (anything else
//       444).  ICC (IOF_EMBED_ICC) and XMP (IOF_EMBED_XMP) from opt.meta, resolution tags from
//       meta.dpix / dpiy; the cancel flag is polled per MB row.
//
// Codestream: spatial mode, one tile, no index table, soft tiling, short header, BD8, all bands,
// QPs uniform over the image plane.  Per codestream (Jc::cs): the planes (i32: the image extended
// to MB multiples by edge replication, bias removed, << 3 when scaled, FwdColorFmtConvert1,
// chroma decimated with [1 3 3 1] / 8, centred) get the first level overlap pre-filter (OVERLAP
// 1), the FCT of every 4x4 block in place (HP coefficients quantised and stored back in the
// block in F order, slot 0 unused), then per macroblock the block DCs (chroma halved when
// scaled -- the decoder doubles them), the second level transform and the DC / LP quantisation
// (Jc::lpq).  The entropy pass codes the macroblocks in raster order: DC / LP prediction, DC, LP
// (CBPLP), CBPHP, HP blocks with interleaved FLEXBITS; adaptive VLC tables, scans and models are
// those of the decoder (ent::je_reset, ent::adapt_all).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use gfx::FMT_ARGB;
use img_core::meta::get_row;
use img_core::{ERR_CANCEL, ERR_DIMS, IMG_MAXDIM, IOF_EMBED_ICC, IOF_EMBED_XMP, IOF_LOSSLESS, ImgOpt, Surface};

use crate::ent::{AHI_NBLK5, AHI_NBLK9, AHI_NCBP, Ah, F2R, Je, Scan, adapt_all, je_reset, quantmap, scan_totals_hp, scan_totals_lp};
use crate::tab::{ENC_VLC, JXV_DELTA, QPS8, QPS420};

const YONLY: u32 = 0;
const YUV420: u32 = 1;
const YUV422: u32 = 2;
const YUV444: u32 = 3;
const JXR_QDEF: u32 = 90;

static GUID: [u8; 16] =
    [0x24, 0xc3, 0xdd, 0x6f, 0x03, 0x4e, 0xfe, 0x4b, 0xb1, 0x85, 0x3d, 0x77, 0x76, 0x8d, 0xc9, 0x0c]; // GUID_PKPixelFormat24bppBGR
static QUAD: [u8; 16] = [0x00, 0x01, 0x10, 0x11, 0x02, 0x03, 0x12, 0x13, 0x20, 0x21, 0x30, 0x31, 0x22, 0x23, 0x32, 0x33];
static RAST2: [u8; 16] = [0x00, 0x01, 0x10, 0x11, 0x20, 0x21, 0x30, 0x31, 0, 0, 0, 0, 0, 0, 0, 0];
static UVPOS: [[u8; 8]; 2] = [[1, 2, 3, 0, 0, 0, 0, 0], [4, 1, 2, 3, 5, 6, 7, 0]]; // 4:2:0, 4:2:2
static LEVIDX: [u8; 16] = [0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5]; // absolute level 2..17 -> VLC index
static LEVREM: [u8; 8] = [2, 3, 4, 6, 10, 14, 0, 0];
static LEVLEN: [u8; 8] = [0, 0, 1, 2, 2, 2, 0, 0];
static RUNBIN: [u8; 16] = [0, 0, 0, 0, 0, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0];
static RUNREM: [u8; 16] = [1, 2, 3, 5, 7, 1, 2, 3, 5, 7, 1, 2, 3, 4, 5, 0];
static RUNFLC: [u8; 16] = [0, 0, 1, 1, 3, 0, 0, 1, 1, 2, 0, 0, 0, 0, 1, 0];
static CODE3: [(u32, u32); 4] = [(0, 1), (6, 3), (2, 2), (7, 3)]; // index at location 15
static CODE3B: [(u32, u32); 4] = [(1, 1), (1, 2), (0, 2), (0, 0)]; // "1" / "01" / "00"
static MASK4: [(u32, u32); 16] = [
    (0, 0),
    (0, 2),
    (1, 2),
    (0, 2),
    (2, 2),
    (1, 2),
    (4, 3),
    (3, 2),
    (3, 2),
    (5, 3),
    (6, 3),
    (2, 2),
    (7, 3),
    (1, 2),
    (0, 2),
    (0, 0),
]; // 4-bit mask -> position code (value, bits)
static QCLASS: [u32; 16] = [
    0x000, 0x001, 0x101, 0x002, 0x201, 0x003, 0x103, 0x004, 0x301, 0x203, 0x303, 0x104, 0x102, 0x204, 0x304, 0x005,
]; // 4-bit pattern -> class | code bits << 8
static GFLC: [u32; 8] = [0, 2, 1, 2, 2, 0, 0, 0]; // code bits per class
static Q9N: [u32; 8] = [0, 0, 1, 2, 3, 4, 0, 0]; // NUM_BLKCBPHP symbol without chroma
static Q9C: [u32; 8] = [5, 6, 7, 8, 8, 8, 0, 0]; // with chroma
static W0: [i32; 3] = [240, 12, 1];
static W1: [i32; 3] = [120, 6, 8];
static W2: [i32; 6] = [120, 37, 2, 120, 18, 1];
static VBASE: [u32; 16] = [0, 0, 0, 0, 0, 1, 3, 7, 9, 10, 0, 0, 12, 0, 0, 0]; // alphabet size -> first table

// ================================================================== transforms (16 x i32)

#[inline(always)]
fn ad(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

#[inline(always)]
fn sb(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}

/// (3 x + r) >> s
#[inline(always)]
fn m3(x: i32, r: i32, s: u32) -> i32 {
    x.wrapping_mul(3).wrapping_add(r) >> s
}

/// forward T2x2h (exact inverse of the decoder's T2x2h with rounding r)  [FT2]
#[inline(always)]
fn ft2(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize, r: i32) {
    let (mut x0, mut x1, x2, x3) = (v[a], v[b], v[c], v[d]);
    x0 = ad(x0, x3);
    x1 = sb(x1, x2);
    let t = ad(sb(x0, x1), r) >> 1;
    let nd = sb(t, x2); // D = t - C
    let nc = sb(t, x3); // C = t - D (old)
    x0 = sb(x0, nd);
    x1 = ad(x1, nc);
    v[a] = x0;
    v[b] = x1;
    v[c] = nc;
    v[d] = nd;
}

/// forward odd 2x2 rotation  [FODD]
#[inline(always)]
fn fodd(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut x0, mut x1, mut x2, mut x3) = (v[a], v[b], v[c], v[d]);
    x0 = ad(x0, x3);
    x1 = sb(x1, x2);
    x3 = sb(ad(x0, 1) >> 1, x3);
    x2 = ad(x2, ad(x1, 1) >> 1);
    x3 = sb(x3, m3(x2, 4, 3));
    x2 = ad(x2, m3(x3, 4, 3));
    x1 = sb(x1, m3(x0, 4, 3));
    x0 = ad(x0, m3(x1, 4, 3));
    x2 = sb(x2, ad(x0, 1) >> 1);
    x3 = ad(x3, x1 >> 1);
    x0 = ad(x0, x2);
    x1 = sb(x1, x3);
    v[a] = x0;
    v[b] = x1;
    v[c] = x2;
    v[d] = x3;
}

/// forward odd-odd 2x2 rotation  [FOO]
#[inline(always)]
fn foo(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let mut x0 = v[a];
    let mut x1 = v[b].wrapping_neg();
    let mut x2 = v[c].wrapping_neg();
    let mut x3 = v[d];
    x3 = ad(x3, x0);
    x2 = sb(x2, x1);
    let t1 = x3 >> 1;
    let t2 = x2 >> 1;
    x0 = sb(x0, t1);
    x1 = ad(x1, t2);
    x0 = ad(x0, m3(x1, 4, 3));
    x1 = sb(x1, m3(x0, 3, 2));
    x0 = ad(x0, m3(x1, 3, 3));
    x1 = sb(x1, t2);
    x0 = ad(x0, t1);
    x2 = ad(x2, x1);
    x3 = sb(x3, x0);
    v[a] = x0;
    v[b] = x1;
    v[c] = x2;
    v[d] = x3;
}

/// T2x2hEnc (Table D.14)  [T2E]
#[inline(always)]
fn t2e(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut x0, mut x1, x2, x3) = (v[a], v[b], v[c], v[d]);
    x0 = ad(x0, x3);
    x1 = sb(x1, x2);
    let nc = sb(sb(x0, x1) >> 1, x3);
    let nd = ad(x1 >> 1, x2);
    x1 = ad(x1, nc);
    x0 = sb(x0, m3(nd, 4, 3));
    v[a] = x0;
    v[b] = x1;
    v[c] = nc;
    v[d] = nd;
}

/// FwdRotate (Table D.15)  [FROT]
#[inline(always)]
fn frot(v: &mut [i32], a: usize, b: usize) {
    let (mut x, mut y) = (v[a], v[b]);
    y = sb(y, ad(x, 1) >> 1);
    x = ad(x, ad(y, 1) >> 1);
    v[a] = x;
    v[b] = y;
}

/// FwdScale (Table D.16)  [FSCL]
#[inline(always)]
fn fscl(v: &mut [i32], a: usize, b: usize) {
    let (mut x, mut y) = (v[a], v[b]);
    y = sb(y, x.wrapping_mul(3) >> 4);
    y = sb(y, x >> 7);
    y = ad(y, x >> 10);
    x = sb(x, y.wrapping_mul(3) >> 3);
    y = sb(x >> 1, y);
    x = sb(x, y);
    v[a] = x;
    v[b] = y;
}

/// FwdTOddOdd (Table D.17)  [FTOO]
#[inline(always)]
fn ftoo(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut x0, mut x1, mut x2, mut x3) = (v[a], v[b], v[c], v[d]);
    x3 = ad(x3, x0);
    x2 = sb(x2, x1);
    let t1 = x3 >> 1;
    let t2 = x2 >> 1;
    x0 = sb(x0, t1);
    x1 = ad(x1, t2);
    x0 = ad(x0, m3(x1, 4, 3));
    x1 = sb(x1, m3(x0, 2, 2));
    x0 = ad(x0, m3(x1, 6, 3));
    x1 = sb(x1, t2);
    x0 = ad(x0, t1);
    x2 = ad(x2, x1);
    x3 = sb(x3, x0);
    v[a] = x0;
    v[b] = x1;
    v[c] = x2;
    v[d] = x3;
}

/// forward core transform in place (raster output; F order via F2R)  [fct4x4]
#[inline]
fn fct4x4(v: &mut [i32; 16]) {
    ft2(v, 4, 7, 8, 11, 0);
    ft2(v, 1, 2, 13, 14, 0);
    ft2(v, 5, 6, 9, 10, 0);
    ft2(v, 0, 3, 12, 15, 0);
    foo(v, 10, 11, 14, 15);
    fodd(v, 8, 12, 9, 13);
    fodd(v, 2, 3, 6, 7);
    ft2(v, 0, 1, 4, 5, 1);
}

/// OverlapPreFilter4x4 (Table D.18) on the area straddling 4 blocks  [pre4x4]
#[inline]
fn pre4x4(v: &mut [i32; 16]) {
    t2e(v, 0, 3, 12, 15);
    t2e(v, 1, 2, 13, 14);
    t2e(v, 4, 7, 8, 11);
    t2e(v, 5, 6, 9, 10);
    fscl(v, 0, 15);
    fscl(v, 1, 14);
    fscl(v, 4, 11);
    fscl(v, 5, 10);
    frot(v, 13, 12);
    frot(v, 9, 8);
    frot(v, 7, 3);
    frot(v, 6, 2);
    ftoo(v, 10, 11, 14, 15);
    ft2(v, 0, 12, 3, 15, 0);
    ft2(v, 1, 2, 13, 14, 0);
    ft2(v, 4, 7, 8, 11, 0);
    ft2(v, 5, 6, 9, 10, 0);
}

/// OverlapPreFilter4 (Table D.19)  [pre4]
#[inline]
fn pre4(v: &mut [i32; 4]) {
    v[0] = ad(v[0], v[3]);
    v[1] = ad(v[1], v[2]);
    v[3] = sb(v[3], ad(v[0], 1) >> 1);
    v[2] = sb(v[2], ad(v[1], 1) >> 1);
    frot(v, 2, 3);
    let (mut x0, mut x1) = (v[0], v[1]);
    let mut x2 = v[2].wrapping_neg();
    let mut x3 = v[3].wrapping_neg();
    x0 = sb(x0, x3);
    x1 = sb(x1, x2);
    x3 = ad(x3, x0 >> 1);
    x2 = ad(x2, x1 >> 1);
    x0 = sb(x0, m3(x3, 4, 3));
    x1 = sb(x1, m3(x2, 4, 3));
    *v = [x0, x1, x2, x3];
    fscl(v, 0, 3);
    fscl(v, 1, 2);
    let (mut x0, mut x1) = (v[0], v[1]);
    let n3 = ad(ad(x0, 1) >> 1, v[3]);
    x0 = sb(x0, n3);
    let n2 = ad(ad(x1, 1) >> 1, v[2]);
    x1 = sb(x1, n2);
    *v = [x0, x1, n2, n3];
}

/// first level overlap pre-filter over a plane (pw, ph multiples of 8): 4x4 areas at every
/// interior block corner, 4-point filters across the block edges of rows 0, 1, ph-2, ph-1 and
/// columns 0, 1, pw-2, pw-1, the four 2x2 corners (all areas are disjoint)  [jxw_pref]
fn pref(p: &mut [i32], pw: usize, ph: usize) {
    let mut y = 4;
    while y < ph {
        let mut x = 4;
        while x < pw {
            let o = (y - 2) * pw + x - 2;
            let mut v = [0i32; 16];
            for r in 0..4 {
                v[r * 4..r * 4 + 4].copy_from_slice(&p[o + r * pw..o + r * pw + 4]);
            }
            pre4x4(&mut v);
            for r in 0..4 {
                p[o + r * pw..o + r * pw + 4].copy_from_slice(&v[r * 4..r * 4 + 4]);
            }
            x += 4;
        }
        y += 4;
    }
    // rows 0, 1, ph - 2, ph - 1 across vertical block edges
    let mut x = 4;
    while x < pw {
        for sel in 0..4 {
            let row = if sel < 2 { sel } else { ph + sel - 4 };
            let o = row * pw + x - 2;
            let mut v = [p[o], p[o + 1], p[o + 2], p[o + 3]];
            pre4(&mut v);
            p[o..o + 4].copy_from_slice(&v);
        }
        x += 4;
    }
    // columns 0, 1, pw - 2, pw - 1 across horizontal block edges
    let mut y = 4;
    while y < ph {
        for sel in 0..4 {
            let col = if sel < 2 { sel } else { pw + sel - 4 };
            let o = (y - 2) * pw + col;
            let mut v = [p[o], p[o + pw], p[o + 2 * pw], p[o + 3 * pw]];
            pre4(&mut v);
            p[o] = v[0];
            p[o + pw] = v[1];
            p[o + 2 * pw] = v[2];
            p[o + 3 * pw] = v[3];
        }
        y += 4;
    }
    // 2x2 corners
    for sel in 0..4 {
        let r = if sel & 2 != 0 { ph - 2 } else { 0 };
        let c = if sel & 1 != 0 { pw - 2 } else { 0 };
        let o = r * pw + c;
        let mut v = [p[o], p[o + 1], p[o + pw], p[o + pw + 1]];
        pre4(&mut v);
        p[o] = v[0];
        p[o + 1] = v[1];
        p[o + pw] = v[2];
        p[o + pw + 1] = v[3];
    }
}

/// the level: sign(v) * ((|v| + offset) / q), offset (3q + 1) >> 3 (HP dead zone) or q >> 1;
/// q = 1 keeps v  [qnt]
#[inline]
fn qnt(v: i32, q: u32, hp: bool) -> i32 {
    if q <= 1 {
        return v;
    }
    let off = if hp { (q * 3 + 1) >> 3 } else { q >> 1 };
    let s = v >> 31;
    let a = ((v ^ s).wrapping_sub(s) as u32).wrapping_add(off) / q;
    (a as i32 ^ s).wrapping_sub(s)
}

/// |v| (ABS32: i32::MIN stays)
#[inline(always)]
fn abs32(v: i32) -> i32 {
    v.wrapping_abs()
}

/// sign(v) * (|v| >> bits)
#[inline(always)]
fn lev(v: i32, bits: u32) -> i32 {
    let s = v >> 31;
    let a = ((v ^ s).wrapping_sub(s) as u32) >> bits;
    (a as i32 ^ s).wrapping_sub(s)
}

// ================================================================== the codestream work record (JC)

struct Jc<'a> {
    buf: Vec<u8>, // output
    acc: u64,     // pending bits (low nb bits)
    nb: u32,
    cf: u32,
    nch: u32,
    scaled: u32,
    ol: u32,
    mbw: u32,
    mbh: u32,
    mx: u32,
    my: u32,
    lm: [i32; 2],
    dcm: u32,
    acm: u32,
    w: u32,
    h: u32,
    pl: [Vec<i32>; 3],
    pw: [u32; 3],
    ph: [u32; 3],
    lpq: Vec<i32>,  // [MB][NCH][16] quantised DC + LP (F order, after prediction)
    cbps: Vec<u32>, // [MB][NCH] CBPHP
    je: Box<Je>,
    qdc: [u32; 3],
    qhp: [u32; 3],
    qi: [u8; 8], // QP indices: DC / LP 0..2, HP 4..6
    opt: Option<&'a ImgOpt<'a>>,
    outcf: u32,
    bw: u32, // blocks per MB of the component (transform)
    bh: u32,
    cbpc: [u32; 4],
    diff: [u32; 4],
    res: [[i32; 16]; 3], // DC / LP residuals
    lv: [i32; 16],       // levels in scan order
    qu: [u32; 4],        // U / V pattern per quadrant
    qv: [u32; 4],
    hr: Vec<[i32; 16]>, // [3][16] HP residual blocks (CBP bit order, F order)
    b0: u32,            // model bits luma / chroma of the band
    b1: u32,
}

impl<'a> Jc<'a> {
    /// work record with zeroed planes for the image size (MB multiples; chroma of 4:2:2 / 4:2:0
    /// subsampled)  [jxw_jcnew]
    fn new(cf: u32, w: u32, h: u32, opt: &'a ImgOpt<'a>) -> Jc<'a> {
        let nch = if cf == YONLY { 1 } else { 3 };
        let mbw = (w + 15) >> 4;
        let mbh = (h + 15) >> 4;
        let mut jc = Jc {
            buf: Vec::new(),
            acc: 0,
            nb: 0,
            cf,
            nch,
            scaled: 0,
            ol: 0,
            mbw,
            mbh,
            mx: 0,
            my: 0,
            lm: [0; 2],
            dcm: 0,
            acm: 0,
            w,
            h,
            pl: [Vec::new(), Vec::new(), Vec::new()],
            pw: [0; 3],
            ph: [0; 3],
            lpq: Vec::new(),
            cbps: Vec::new(),
            je: Box::default(),
            qdc: [0; 3],
            qhp: [0; 3],
            qi: [0; 8],
            opt: Some(opt),
            outcf: 0,
            bw: 0,
            bh: 0,
            cbpc: [0; 4],
            diff: [0; 4],
            res: [[0; 16]; 3],
            lv: [0; 16],
            qu: [0; 4],
            qv: [0; 4],
            hr: vec![[0; 16]; 48],
            b0: 0,
            b1: 0,
        };
        for c in 0..nch as usize {
            let mut pw = mbw << 4;
            let mut ph = mbh << 4;
            if c != 0 && cf == YUV420 {
                pw >>= 1;
                ph >>= 1;
            } else if c != 0 && cf == YUV422 {
                pw >>= 1;
            }
            jc.pw[c] = pw;
            jc.ph[c] = ph;
            jc.pl[c] = vec![0; pw as usize * ph as usize];
        }
        jc
    }

    fn cancelled(&self) -> bool {
        self.opt.is_some_and(|o| o.cancelled())
    }

    // ---------------------------------------------------------------- bit writer
    /// append the low n bits of v MSB first  [pb]
    #[inline]
    fn pb(&mut self, v: u32, n: u32) {
        if n == 0 {
            return;
        }
        let m = (v as u64) & ((1u64 << n) - 1);
        let acc = (self.acc << n) | m;
        self.acc = acc;
        let mut nb = n + self.nb;
        if nb >= 32 {
            nb -= 32;
            let word = (acc >> nb) as u32;
            self.buf.extend_from_slice(&word.to_be_bytes());
        }
        self.nb = nb;
    }

    /// the symbol's code in the instance's current table, discriminants updated (fixed tables
    /// have zero delta rows)  [pv]
    #[inline]
    fn pv(&mut self, ai: usize, sym: u32) {
        let sym = sym & 15;
        let a = &mut self.je.ah[ai];
        let t = VBASE[(a.n & 15) as usize] + a.t as u32;
        let e = ENC_VLC[(t * 16 + sym) as usize] as u32;
        add_delta(a, sym);
        self.pb(e & 0xFF, e >> 8);
    }

    /// zero bits up to the next byte boundary  [jxw_align]
    fn align(&mut self) {
        let n = self.nb.wrapping_neg() & 7;
        self.pb(0, n);
    }

    /// byte-align and write the pending bits  [jxw_flush]
    fn flush(&mut self) {
        self.align();
        let mut n = self.nb;
        while n >= 8 {
            n -= 8;
            self.buf.push((self.acc >> n) as u8);
        }
        self.nb = 0;
    }

    // ---------------------------------------------------------------- transform pass
    /// pre-filter, FCT of every block (HP levels back into the block, F order), second level
    /// transform of the block DCs, DC / LP levels into lpq  [jxw_xform]
    fn xform(&mut self, c: usize) -> Result<(), i32> {
        let pw = self.pw[c] as usize;
        if self.ol != 0 {
            let ph = self.ph[c] as usize;
            pref(&mut self.pl[c], pw, ph);
        }
        self.bw = 4;
        self.bh = 4;
        if c != 0 {
            if self.cf == YUV420 {
                self.bw = 2;
                self.bh = 2;
            } else if self.cf == YUV422 {
                self.bw = 2;
            }
        }
        let (bw, bh) = (self.bw as usize, self.bh as usize);
        let qhp = self.qhp[c];
        let qdc = self.qdc[c];
        let mut dc = [0i32; 16];
        for my in 0..self.mbh as usize {
            if self.cancelled() {
                return Err(ERR_CANCEL);
            }
            for mx in 0..self.mbw as usize {
                let p = &mut self.pl[c];
                for bi in 0..bw * bh {
                    let (r, cc) = (bi / bw, bi % bw);
                    let o = ((my * bh + r) * 4) * pw + (mx * bw + cc) * 4;
                    let mut tv = [0i32; 16];
                    for rr in 0..4 {
                        tv[rr * 4..rr * 4 + 4].copy_from_slice(&p[o + rr * pw..o + rr * pw + 4]);
                    }
                    fct4x4(&mut tv);
                    dc[bi & 15] = tv[0];
                    // HP levels in F order into the block (slot 0 = 0)
                    p[o] = 0;
                    for f in 1..16 {
                        p[o + (f >> 2) * pw + (f & 3)] = qnt(tv[F2R[f] as usize], qhp, true);
                    }
                }
                // chroma DCs halved (scaled arithmetic)
                if c != 0 && self.scaled != 0 {
                    for v in dc.iter_mut() {
                        *v = ad(*v, 1) >> 1;
                    }
                }
                // second level -> tv (F order, unused entries 0)
                let mut tv = [0i32; 16];
                if bw == 4 {
                    fct4x4(&mut dc);
                    for f in 0..16 {
                        tv[f] = dc[F2R[f] as usize];
                    }
                } else {
                    ft2(&mut dc, 0, 1, 2, 3, 0);
                    if bh == 2 {
                        tv[..4].copy_from_slice(&dc[..4]);
                    } else {
                        ft2(&mut dc, 4, 5, 6, 7, 0);
                        let l4 = sb(dc[4], dc[0]); // L4 = bottom - top
                        tv[0] = ad(dc[0], ad(l4, 1) >> 1); // L0 = top + ((L4 + 1) >> 1)
                        tv[4] = l4;
                        tv[1..4].copy_from_slice(&dc[1..4]);
                        tv[5..8].copy_from_slice(&dc[5..8]);
                    }
                }
                // DC / LP levels -> LPQ[mb][c]
                let rec = (((my * self.mbw as usize + mx) * self.nch as usize) + c) * 16;
                for i in 0..16 {
                    self.lpq[rec + i] = qnt(tv[i], qdc, false);
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- symbol coding
    /// DECODE_ABS_LEVEL inverse  [plevel]
    fn plevel(&mut self, level: u32, ai: usize) {
        if level < 18 {
            let idx = LEVIDX[(level.wrapping_sub(2) & 15) as usize] as usize;
            self.pv(ai, idx as u32);
            self.pb(level.wrapping_sub(LEVREM[idx] as u32), LEVLEN[idx] as u32);
            return;
        }
        self.pv(ai, 6);
        let x = level.wrapping_sub(2);
        let fixed = 31 - x.leading_zeros(); // bsr (>= 4)
        if fixed < 19 {
            self.pb(fixed - 4, 4);
        } else {
            self.pb(15, 4);
            if fixed < 22 {
                self.pb(fixed - 19, 2);
            } else {
                self.pb(3, 2);
                self.pb(fixed - 22, 3);
            }
        }
        self.pb(x & !(1u32 << fixed), fixed);
    }

    /// DECODE_RUN inverse, maxrun 1..14  [prun]
    fn prun(&mut self, run: u32, maxrun: u32) {
        if maxrun < 5 {
            if run == maxrun {
                self.pb(0, run.wrapping_sub(1)); // run - 1 zero bits
            } else {
                self.pb(1, run); // run - 1 zero bits and a 1
            }
            return;
        }
        let bin = RUNBIN[(maxrun & 15) as usize] as usize * 5;
        let mut i = 0;
        while i < 4 {
            let base = RUNREM[bin + i] as u32;
            let flc = RUNFLC[bin + i] as u32;
            if run < (1u32 << flc) + base {
                break;
            }
            i += 1;
        }
        let base = RUNREM[bin + i] as u32;
        let flc = RUNFLC[bin + i] as u32;
        self.pv(0, i as u32); // AH 0 (fixed)
        self.pb(run.wrapping_sub(base), flc);
    }

    /// DECODE_BLOCK inverse for lv[bst .. bend) (at least one nonzero level): AH offset boff
    /// (5 LP / 13 HP), chroma bch, first location bloc -> nonzero levels  [pblock]
    fn pblock(&mut self, boff: usize, bch: usize, bst: u32, bend: u32, bloc: u32) -> u32 {
        let mut nz = [0u8; 16];
        let mut n = 0usize;
        for k in bst..bend.min(16) {
            if self.lv[k as usize] != 0 {
                nz[n & 15] = k as u8;
                n += 1;
            }
        }
        if n == 0 {
            return 0;
        }
        let a1 = bch * 3 + boff;
        let mut prev = bst;
        let mut loc = bloc;
        // first level: index, sign, level, run
        let k = (nz[0] & 15) as u32;
        let run = k.wrapping_sub(prev);
        prev = k + 1;
        let v = self.lv[k as usize];
        let sr = (run == 0) as u32;
        let mut srn = if n == 1 {
            0
        } else if nz[1] as u32 == prev {
            1
        } else {
            2
        };
        let av = abs32(v) as u32;
        let mut idx = srn * 4 | sr;
        if av > 1 {
            idx |= 2;
        }
        self.pv(a1, idx);
        self.pb((v as u32) >> 31, 1);
        let mut cont = sr & srn;
        if av > 1 {
            self.plevel(av, boff + cont as usize + 6);
        }
        if run != 0 {
            self.prun(run, 15u32.wrapping_sub(loc));
        }
        loc = loc.wrapping_add(run + 1);
        for i in 1..n {
            let k = (nz[i & 15] & 15) as u32;
            let run = k.wrapping_sub(prev);
            prev = k + 1;
            let v = self.lv[k as usize];
            if srn & 1 == 0 {
                self.prun(run, 15u32.wrapping_sub(loc));
            }
            loc = loc.wrapping_add(run + 1);
            srn = if i + 1 >= n {
                0
            } else if nz[(i + 1) & 15] as u32 == prev {
                1
            } else {
                2
            };
            let av = abs32(v) as u32;
            let mut idx = srn * 2;
            if av > 1 {
                idx |= 1;
            }
            if loc < 15 {
                self.pv(a1 + cont as usize + 1, idx);
            } else if loc == 15 {
                let (c, b) = CODE3[(idx & 3) as usize];
                self.pb(c, b);
            } else {
                self.pb(idx, 1);
            }
            self.pb((v as u32) >> 31, 1);
            cont &= srn;
            if av > 1 {
                self.plevel(av, boff + cont as usize + 6);
            }
        }
        n as u32
    }

    /// FLEXBITS / refinement: the low bits of |v|, a sign bit when the VLC level was 0 and the
    /// low bits are not  [prefine]
    fn prefine(&mut self, vals: &[i32], bits: u32) {
        if bits == 0 {
            return;
        }
        for &v in vals {
            let a = abs32(v) as u32;
            let low = a & ((1u32 << bits) - 1);
            self.pb(low, bits);
            if a >> bits == 0 && low != 0 {
                self.pb((v as u32) >> 31, 1);
            }
        }
    }

    /// number of set bits (VLC: the count for 5 symbols, count - 1 for 4) and the position code
    /// of the mask  [pmask4]
    fn pmask4(&mut self, ai: usize, mask: u32) {
        let m = mask & 15;
        let mut n = m.count_ones();
        if self.je.ah[ai].n != 5 {
            n = n.wrapping_sub(1);
        }
        self.pv(ai, n);
        let (v, b) = MASK4[m as usize];
        self.pb(v, b);
    }

    /// CBPLP with the zero / max counts  [pcbplp]
    fn pcbplp(&mut self, cbp: u32, ncomp: u32) {
        if ncomp == 1 {
            self.pb(cbp, 1);
            return;
        }
        let max = (1u32 << ncomp) - 1;
        let (lpz, lpm) = (self.je.lpz, self.je.lpm);
        if lpz > 0 && lpm >= 0 {
            self.pb(cbp, ncomp);
        } else {
            let prob = if lpm < lpz { max } else { 0 };
            if cbp == prob {
                self.pb(0, 1);
            } else {
                let x = cbp ^ prob;
                self.pb(1, 1);
                if x == 1 {
                    self.pb(0, ncomp - 1);
                } else {
                    self.pb(x >> 1, ncomp - 1);
                    self.pb(x & 1, 1);
                }
            }
        }
        let je = &mut self.je;
        je.lpz = (if cbp == 0 { je.lpz - 3 } else { je.lpz + 1 }).clamp(-8, 7);
        je.lpm = (if cbp == max { je.lpm - 3 } else { je.lpm + 1 }).clamp(-8, 7);
    }

    /// UpdateModelMB from lm  [jxw_model]
    fn model(&mut self, band: usize) {
        let lm0 = W0[band].wrapping_mul(self.lm[0]);
        let lm1 = if self.cf == YUV420 {
            self.lm[1].wrapping_mul(W2[band])
        } else if self.cf == YUV422 {
            self.lm[1].wrapping_mul(W2[band + 3])
        } else {
            // 3 components (1 component: lm1 unused)
            let v = self.lm[1].wrapping_mul(W1[band]);
            if band == 2 { v >> 4 } else { v }
        };
        let one = self.cf == YONLY || self.nch == 1;
        let m = &mut self.je.md[band];
        for j in 0..2 {
            let delta = (if j == 0 { lm0 } else { lm1 }).wrapping_sub(70) >> 2;
            let mut st = m.st[j];
            if delta <= -8 {
                st = st.wrapping_add((delta + 4).max(-16));
                if st < -8 {
                    if m.bits[j] == 0 {
                        st = -8;
                    } else {
                        st = 0;
                        m.bits[j] -= 1;
                    }
                }
            } else if delta >= 8 {
                st = st.wrapping_add((delta - 4).min(15));
                if st > 8 {
                    if m.bits[j] >= 15 {
                        m.bits[j] = 15;
                        st = 8;
                    } else {
                        st = 0;
                        m.bits[j] += 1;
                    }
                }
            }
            m.st[j] = st;
            if one {
                break;
            }
        }
    }

    /// CBPHP model counts and state  [cbpmod]
    fn cbpmod(&mut self, c1: usize, cbp: u32, blocks: u32) {
        let n = (cbp & 0xFFFF).count_ones() as i32;
        let mu = (16 / blocks) as i32;
        let z = blocks as i32 - n;
        let je = &mut self.je;
        let a = (n * mu - 3 + je.cbc0[c1]).clamp(-16, 15);
        je.cbc0[c1] = a;
        let b = (z * mu - 3 + je.cbc1[c1]).clamp(-16, 15);
        je.cbc1[c1] = b;
        je.cbst[c1] = if a < 0 {
            if a >= b { 2 } else { 1 }
        } else if b < 0 {
            2
        } else {
            0
        };
    }

    /// HP blocks of a component  [jxw_nblk]
    fn nblk(&self, c: usize) -> usize {
        if c == 0 {
            16
        } else if self.cf == YUV420 {
            4
        } else if self.cf == YUV422 {
            8
        } else {
            16
        }
    }

    /// predict, model and code the CBPHP of the macroblock (cbpc -> diff, cbps)  [jxw_cbphp]
    fn cbphp(&mut self) {
        let nch = self.nch as usize;
        let rec = (self.my * self.mbw + self.mx) as usize * nch;
        let cf = self.cf;
        for c in 0..nch {
            let left = if self.mx != 0 { self.cbps[rec - nch + c] } else { 0 };
            let top = if self.my != 0 { self.cbps[rec - self.mbw as usize * nch + c] } else { 0 };
            let cb = self.cbpc[c];
            self.cbps[rec + c] = cb;
            let c1 = (c != 0) as usize;
            let st = self.je.cbst[c1];
            let edge = |e_top: u32, e_left: u32, mx: u32, my: u32| -> u32 {
                (if mx != 0 {
                    e_left
                } else if my != 0 {
                    e_top
                } else {
                    1
                }) & 1
            };
            if c == 0 || cf == YUV444 {
                let mut d = cb;
                if st == 0 {
                    d = ((cb & 0x3300) << 2) ^ cb;
                    d ^= (d & 0xcc) << 6;
                    d ^= (d & 0x33) << 2;
                    d ^= (d << 1) & 0x20;
                    d ^= (d << 3) & 0x10;
                    d ^= (d << 1) & 0x02;
                    d ^= edge(top >> 10, left >> 5, self.mx, self.my);
                } else if st > 1 {
                    d ^= 0xFFFF;
                }
                self.diff[c] = d;
                self.cbpmod(c1, cb, 16);
            } else {
                // 4:2:0 (2 x 2) / 4:2:2 (2 wide x 4 high)
                let mut d = cb;
                if st == 0 {
                    if cf != YUV420 {
                        d ^= (d & 0x30) << 2;
                        d ^= (d & 0x0C) << 2;
                    }
                    d ^= (d & 3) << 2;
                    d ^= (d << 1) & 0x02;
                    let t = if cf == YUV420 { top >> 2 } else { top >> 6 };
                    d ^= edge(t, left >> 1, self.mx, self.my);
                } else if st > 1 {
                    d ^= 0x0F;
                    if cf != YUV420 {
                        d ^= 0xF0;
                    }
                }
                self.diff[c] = d;
                self.cbpmod(1, cb, if cf == YUV420 { 4 } else { 8 });
            }
        }
        // ---- code
        if cf == YONLY {
            let d = self.diff[0];
            let mut qm = 0;
            for q in 0..4 {
                if (d >> (q * 4)) & 15 != 0 {
                    qm |= 1 << q;
                }
            }
            self.pmask4(AHI_NCBP, qm);
            for q in 0..4 {
                if (qm >> q) & 1 == 0 {
                    continue;
                }
                let cls = QCLASS[((d >> (q * 4)) & 15) as usize];
                self.pv(AHI_NBLK5, (cls & 255).wrapping_sub(1));
                self.pb(cls >> 8, GFLC[(cls & 7) as usize]);
            }
            return;
        }
        // chroma pattern per quadrant
        for q in 0..4 {
            let (u, v) = (self.diff[1], self.diff[2]);
            let (pu, pv) = if cf == YUV444 {
                ((u >> (q * 4)) & 15, (v >> (q * 4)) & 15)
            } else if cf == YUV420 {
                ((u >> q) & 1, (v >> q) & 1)
            } else {
                // quadrant (qy, qx): bits 4qy + qx and 4qy + 2 + qx
                let s = (q >> 1) * 4 + (q & 1);
                let (a, b) = (u >> s, v >> s);
                ((a & 1) | ((a >> 1) & 2), (b & 1) | ((b >> 1) & 2))
            };
            self.qu[q as usize] = pu;
            self.qv[q as usize] = pv;
        }
        let mut qm = 0u32;
        for q in 0..4 {
            if ((self.diff[0] >> (q * 4)) & 15) | self.qu[q] | self.qv[q] != 0 {
                qm |= 1 << q;
            }
        }
        self.pmask4(AHI_NCBP, qm);
        for q in 0..4 {
            if (qm >> q) & 1 == 0 {
                continue;
            }
            let cls = QCLASS[((self.diff[0] >> (q * 4)) & 15) as usize];
            let chroma = self.qu[q] | self.qv[q];
            let sym = if chroma != 0 { Q9C[(cls & 7) as usize] } else { Q9N[(cls & 7) as usize] };
            self.pv(AHI_NBLK9, sym);
            if chroma != 0 {
                // CHROMA_PATTERN U only "1", V only "01", both "00"
                if self.qv[q] == 0 {
                    self.pb(1, 1);
                } else if self.qu[q] == 0 {
                    self.pb(1, 2);
                } else {
                    self.pb(0, 2);
                }
                let c = cls & 7;
                if c >= 3 {
                    let (v, b) = CODE3B[(c - 3) as usize]; // class 3 "1", 4 "01", 5 "00"
                    self.pb(v, b);
                }
            }
            self.pb(cls >> 8, GFLC[(cls & 7) as usize]);
            if cf != YUV420 {
                let (pu, pv) = (self.qu[q], self.qv[q]);
                self.cpat(pu);
                self.cpat(pv);
            }
        }
    }

    /// nothing for 0; 4:4:4: count (AH 1) + positions; 4:2:2: 1 "1", 2 "01", 3 "00"  [cpat]
    fn cpat(&mut self, p: u32) {
        if p == 0 {
            return;
        }
        if self.cf == YUV444 {
            self.pmask4(1, p);
        } else {
            let (v, b) = CODE3B[(p.wrapping_sub(1) & 3) as usize];
            self.pb(v, b);
        }
    }

    /// lv[k] = the VLC level of residual order[k] (sign * (|v| >> bits)), k = 1..15  [jxw_levels]
    fn levels(&mut self, vals: &[i32; 16], scan: usize, bits: u32) {
        self.lv[0] = 0;
        let ord = self.je.scan[scan].ord;
        for k in 1..16 {
            self.lv[k] = lev(vals[(ord[k] & 15) as usize], bits);
        }
    }

    /// scan_upd for every nonzero lv[k], k = 1..15 in order  [jxw_scanupd]
    fn scanupd(&mut self, scan: usize) {
        let s = &mut self.je.scan[scan];
        for k in 1..16 {
            if self.lv[k] != 0 {
                scan_upd(s, k);
            }
        }
    }

    /// any |LP residual| >> (model bits) != 0  [lpany]
    fn lpany(&self, c: usize) -> bool {
        let bits = if c == 0 { self.b0 } else { self.b1 };
        self.res[c][1..].iter().any(|&v| (abs32(v) as u32) >> bits != 0)
    }

    /// address of a block in the plane of the current macroblock (bw / bh geometry)  [hraddr]
    fn hraddr(&self, r: usize, cc: usize, c: usize) -> usize {
        let pw = self.pw[c] as usize;
        ((self.my as usize * self.bh as usize + r) * 4) * pw + (self.mx as usize * self.bw as usize + cc) * 4
    }

    /// hr (HP levels of every block minus the HP prediction of acm, CBP bit order) and cbpc
    /// from the model bits b0 / b1  [jxw_hpres]
    fn hpres(&mut self) {
        for c in 0..self.nch as usize {
            self.cbpc[c] = 0;
            let nb = self.nblk(c);
            let (tab, bw, bh): (&[u8; 16], u32, u32) = if nb == 16 {
                (&QUAD, 4, 4)
            } else if nb == 4 {
                (&RAST2, 2, 2)
            } else {
                (&RAST2, 2, 4)
            };
            self.bw = bw;
            self.bh = bh;
            let pw = self.pw[c] as usize;
            let bits = if c == 0 { self.b0 } else { self.b1 };
            for b in 0..nb {
                let rc = tab[b & 15] as usize;
                let (r, cc) = (rc >> 4, rc & 15);
                let o = self.hraddr(r, cc, c);
                let p = &self.pl[c];
                let mut h = [0i32; 16];
                for rr in 0..4 {
                    h[rr * 4..rr * 4 + 4].copy_from_slice(&p[o + rr * pw..o + rr * pw + 4]);
                }
                h[0] = 0;
                if self.acm == 0 {
                    if cc != 0 {
                        // left neighbour: slots 1, 2, 3
                        let l = self.hraddr(r, cc - 1, c);
                        for k in 1..4 {
                            h[k] = sb(h[k], p[l + k]);
                        }
                    }
                } else if self.acm == 1 && r != 0 {
                    // top neighbour: slots 4, 8, 12 (rows 1..3)
                    let t = self.hraddr(r - 1, cc, c);
                    h[4] = sb(h[4], p[t + pw]);
                    h[8] = sb(h[8], p[t + 2 * pw]);
                    h[12] = sb(h[12], p[t + 3 * pw]);
                }
                if h[1..].iter().any(|&v| (abs32(v) as u32) >> bits != 0) {
                    self.cbpc[c] |= 1 << b;
                }
                self.hr[c * 16 + b] = h;
            }
        }
    }

    /// LP of 4:2:0 / 4:2:2: CBPLP (luma, joint UV), the luma block, its refinement, the joint UV
    /// block (locations from 16 - n), U / V refinement interleaved  [jxw_lpuv]
    fn lpuv(&mut self) {
        let (n, pos) = if self.cf == YUV420 { (6usize, &UVPOS[0]) } else { (14usize, &UVPOS[1]) };
        let mut cbp = 0u32;
        let b0 = self.b0;
        let b1 = self.b1;
        if self.res[0][1..].iter().any(|&v| (abs32(v) as u32) >> b0 != 0) {
            cbp = 1;
        }
        // joint UV levels (U, V interleaved over the positions)
        let mut any = 0u32;
        for i in 0..n {
            let v = self.res[(i & 1) + 1][pos[i >> 1] as usize];
            any |= (abs32(v) as u32) >> b1;
            self.lv[i & 15] = lev(v, b1);
        }
        if any != 0 {
            cbp |= 2;
        }
        self.pcbplp(cbp, 2);
        // luma block (LP scan) -- lv is rebuilt afterwards for the UV block
        if cbp & 1 != 0 {
            let r0 = self.res[0];
            self.levels(&r0, 0, b0);
            let k = self.pblock(5, 0, 1, 16, 1);
            self.lm[0] = self.lm[0].wrapping_add(k as i32);
            self.scanupd(0);
        }
        let r0 = self.res[0];
        self.prefine(&r0[1..], b0);
        if cbp & 2 != 0 {
            for i in 0..n {
                let v = self.res[(i & 1) + 1][pos[i >> 1] as usize];
                self.lv[i & 15] = lev(v, b1);
            }
            let k = self.pblock(5, 1, 0, n as u32, 16 - n as u32);
            self.lm[1] = self.lm[1].wrapping_add(k as i32);
        }
        // refinement k = 1 .. n / 2: U[k], V[k]
        for k in 1..=n / 2 {
            let (u, v) = (self.res[1][k], self.res[2][k]);
            self.prefine(&[u], b1);
            self.prefine(&[v], b1);
        }
    }

    /// one macroblock (mx / my): DC / LP prediction, DC, LP, CBPHP, HP  [jxw_mb]
    fn mb(&mut self) {
        let nch = self.nch as usize;
        let cf = self.cf;
        let rs = nch * 16;
        let rec = (self.my * self.mbw + self.mx) as usize * rs;
        let left = if self.mx != 0 { Some(rec - rs) } else { None };
        let top = if self.my != 0 { Some(rec - self.mbw as usize * rs) } else { None };
        // ---- DC prediction mode
        let q = &self.lpq;
        let dcm = match (left, top) {
            (None, None) => 3,
            (None, Some(_)) => 1,
            (Some(_), None) => 0,
            (Some(l), Some(t)) => {
                let tl = t - rs;
                let mut sh = abs32(sb(q[tl], q[l]));
                let mut sv = abs32(sb(q[tl], q[t]));
                if cf != YONLY {
                    let f = if cf == YUV422 {
                        4
                    } else if cf == YUV420 {
                        8
                    } else {
                        2
                    };
                    sh = sh.wrapping_mul(f);
                    sv = sv.wrapping_mul(f);
                    for o in [16, 32] {
                        sh = ad(sh, abs32(sb(q[tl + o], q[l + o])));
                        sv = ad(sv, abs32(sb(q[tl + o], q[t + o])));
                    }
                }
                if sh.wrapping_mul(4) < sv {
                    1
                } else if sv.wrapping_mul(4) < sh {
                    0
                } else {
                    2
                }
            }
        };
        self.dcm = dcm;
        // ---- residuals = LPQ - prediction
        for c in 0..nch {
            let o = c * 16;
            let mut r = [0i32; 16];
            r.copy_from_slice(&q[rec + o..rec + o + 16]);
            let sub = c != 0 && cf <= YUV422 && cf != YONLY; // chroma of 4:2:0 / 4:2:2
            match dcm {
                2 => {
                    let (l, t) = (left.unwrap() + o, top.unwrap() + o);
                    r[0] = sb(r[0], ad(ad(q[l], q[t]), sub as i32) >> 1);
                }
                1 => {
                    let t = top.unwrap() + o;
                    r[0] = sb(r[0], q[t]);
                    if sub {
                        if cf == YUV420 {
                            r[2] = sb(r[2], q[t + 2]);
                        } else {
                            // 4:2:2: R2 -= T6, R6 -= Q2, R4 -= T4
                            r[2] = sb(r[2], q[t + 6]);
                            r[6] = sb(r[6], q[rec + o + 2]);
                            r[4] = sb(r[4], q[t + 4]);
                        }
                    } else {
                        for k in [4, 8, 12] {
                            r[k] = sb(r[k], q[t + k]);
                        }
                    }
                }
                0 => {
                    let l = left.unwrap() + o;
                    r[0] = sb(r[0], q[l]);
                    r[1] = sb(r[1], q[l + 1]);
                    if sub {
                        if cf != YUV420 {
                            // 4:2:2: also 4, 5
                            r[4] = sb(r[4], q[l + 4]);
                            r[5] = sb(r[5], q[l + 5]);
                        }
                    } else {
                        r[2] = sb(r[2], q[l + 2]);
                        r[3] = sb(r[3], q[l + 3]);
                    }
                }
                _ => {}
            }
            self.res[c & 3] = r;
        }
        // ---- DC
        self.b0 = self.je.md[0].bits[0] as u32;
        self.b1 = self.je.md[0].bits[1] as u32;
        self.lm = [0, 0];
        let mut qs = [0u32; 4];
        let mut code = 0u32;
        for c in 0..nch {
            let bits = if c == 0 { self.b0 } else { self.b1 };
            qs[c] = (abs32(self.res[c][0]) as u32) >> bits;
            if qs[c] != 0 {
                code |= 1 << (2 - c);
            }
        }
        if cf != YONLY {
            self.pv(2, code);
        }
        for c in 0..nch {
            if cf == YONLY {
                self.pb((qs[c] != 0) as u32, 1);
            }
            if qs[c] != 0 {
                let ai = if c == 0 { 3 } else { 4 };
                self.lm[(c != 0) as usize] += 1;
                self.plevel(qs[c].wrapping_add(1), ai);
            }
            let v = self.res[c][0];
            let bits = if c == 0 { self.b0 } else { self.b1 };
            self.pb(abs32(v) as u32, bits); // pb masks the low bits
            if v != 0 {
                self.pb((v as u32) >> 31, 1);
            }
        }
        self.model(0);
        // ---- LP
        self.b0 = self.je.md[1].bits[0] as u32;
        self.b1 = self.je.md[1].bits[1] as u32;
        self.lm = [0, 0];
        if cf == YUV420 || cf == YUV422 {
            self.lpuv();
        } else {
            let mut cbp = 0;
            for c in 0..nch {
                if self.lpany(c) {
                    cbp |= 1 << c;
                }
            }
            self.pcbplp(cbp, self.nch);
            for c in 0..nch {
                let bits = if c == 0 { self.b0 } else { self.b1 };
                if (cbp >> c) & 1 != 0 {
                    let rc = self.res[c];
                    self.levels(&rc, 0, bits);
                    let k = self.pblock(5, (c != 0) as usize, 1, 16, 1);
                    self.lm[(c != 0) as usize] += k as i32;
                    self.scanupd(0);
                }
                let rc = self.res[c];
                self.prefine(&rc[1..], bits);
            }
        }
        self.model(1);
        // ---- HP prediction mode from the predicted LP levels
        let q = &self.lpq[rec..rec + rs];
        let mut sh = ad(ad(abs32(q[1]), abs32(q[2])), abs32(q[3])); // horizontal
        let mut sv = ad(ad(abs32(q[4]), abs32(q[8])), abs32(q[12])); // vertical
        if cf != YONLY {
            let ti = if cf == YUV444 { 4 } else { 2 }; // vertical term: 4:4:4 C4, else C2
            for o in [16, 32] {
                sh = ad(sh, abs32(q[o + 1]));
                sv = ad(sv, abs32(q[o + ti]));
                if cf == YUV422 {
                    sh = ad(sh, abs32(q[o + 5]));
                    sv = ad(sv, abs32(q[o + 6]));
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
        self.b0 = self.je.md[2].bits[0] as u32;
        self.b1 = self.je.md[2].bits[1] as u32;
        self.hpres();
        self.cbphp();
        // ---- HP blocks with FLEXBITS
        self.lm = [0, 0];
        let scan = if self.acm == 1 { 2 } else { 1 };
        for c in 0..nch {
            let bits = if c == 0 { self.b0 } else { self.b1 };
            for b in 0..self.nblk(c) {
                let h = self.hr[c * 16 + b];
                if (self.cbpc[c] >> b) & 1 != 0 {
                    self.levels(&h, scan, bits);
                    let k = self.pblock(13, (c != 0) as usize, 1, 16, 1);
                    self.lm[(c != 0) as usize] += k as i32;
                    self.scanupd(scan);
                }
                self.prefine(&h[1..], bits);
            }
        }
        self.model(2);
    }

    /// QP syntax: one index (1 component), else CHANNEL_MODE uniform / separate / independent +
    /// indices  [jxw_putqp]
    fn putqp(&mut self, o: usize) {
        let q = self.qi;
        let c0 = q[o] as u32;
        if self.nch == 1 {
            self.pb(c0, 8);
            return;
        }
        let (c1, c2) = (q[o + 1] as u32, q[o + 2] as u32);
        if c1 != c2 {
            self.pb(c0 | 0x200, 10); // 10 + Y + U + V
            self.pb(c1, 8);
            self.pb(c2, 8);
        } else if c0 != c1 {
            self.pb(c0 | 0x100, 10); // 01 + Y + UV
            self.pb(c1, 8);
        } else {
            self.pb(c0, 10); // uniform: 00 + index
        }
    }

    /// the whole codestream of the planes into buf  [jxw_cs]
    fn cs(&mut self, lossless: bool, qp: Option<&[u8; 6]>) -> Result<(), i32> {
        let sc = (!lossless) as u32;
        self.scaled = sc;
        self.ol = sc;
        // QP indices
        self.qi = [0; 8];
        if let Some(qp) = qp
            && !lossless
        {
            self.qi[0] = qp[0];
            self.qi[4] = qp[3];
            if self.nch != 1 {
                self.qi[1] = qp[1];
                self.qi[2] = qp[2];
                self.qi[5] = qp[4];
                self.qi[6] = qp[5];
            }
        }
        // quantisers: DC / LP shift 1 for component 0, 0 for chroma; HP shift 1 (scaled)
        for c in 0..3 {
            self.qdc[c] = quantmap(self.qi[c] as u32, ((c == 0) as u32) & sc, sc != 0);
            self.qhp[c] = quantmap(self.qi[4 + c] as u32, sc, sc != 0);
        }
        let mbs = (self.mbw * self.mbh) as usize * self.nch as usize;
        self.buf = Vec::with_capacity(((self.pw[0] as usize * self.ph[0] as usize * self.nch as usize) >> 1) + 65536);
        self.lpq = vec![0; mbs * 16];
        self.cbps = vec![0; mbs];
        // IMAGE_HEADER
        self.pb(0x574D_5048, 32); // "WMPH"
        self.pb(0x4F54_4F00, 32); // "OTO\0"
        self.pb(0x11, 8); // version 1, sub-version 1
        self.pb(self.ol, 8); // spatial, no tiling / index table, overlap
        self.pb(0xC0, 8); // short header, long words
        self.pb((self.outcf << 4) | 1, 8); // BD8
        self.pb(self.w - 1, 16);
        self.pb(self.h - 1, 16);
        // IMAGE_PLANE_HEADER
        self.pb((self.cf << 5) | (sc << 4), 8); // bands 0 (all)
        if self.cf != YONLY {
            let cc = match self.cf {
                YUV420 => 0x22, // 4:2:0 chroma centring x / y = 2 (as WIC)
                YUV422 => 0x20,
                _ => 0,
            };
            self.pb(cc, 8);
        }
        self.pb(1, 1); // DC_IMAGE_PLANE_UNIFORM_FLAG
        self.putqp(0);
        self.pb(1, 2); // RESERVED_I_BIT, LP_IMAGE_PLANE_UNIFORM_FLAG
        self.putqp(0);
        self.pb(1, 2); // RESERVED_J_BIT, HP_IMAGE_PLANE_UNIFORM_FLAG
        self.putqp(4);
        self.align();
        self.pb(0, 16); // SubsequentBytes 0
        self.pb(0x100, 32); // TILE_STARTCODE, ARBITRARY_BYTE
        // transforms
        for c in 0..self.nch as usize {
            self.xform(c)?;
        }
        // entropy pass
        je_reset(&mut self.je);
        for my in 0..self.mbh {
            self.my = my;
            if self.cancelled() {
                return Err(ERR_CANCEL);
            }
            for mx in 0..self.mbw {
                self.mx = mx;
                if mx & 15 == 0 {
                    scan_totals_lp(&mut self.je);
                    scan_totals_hp(&mut self.je);
                }
                if (mx | my) != 0 && (mx == 0 || mx & 15 == 1) {
                    adapt_all(&mut self.je);
                }
                self.mb();
            }
        }
        self.flush();
        Ok(())
    }
}

/// discriminant update of a symbol (the delta rows of the decoder)
#[inline(always)]
fn add_delta(a: &mut Ah, sym: u32) {
    a.disc = a.disc.wrapping_add(JXV_DELTA[((a.dp + sym) & 255) as usize] as i32);
    a.disc1 = a.disc1.wrapping_add(JXV_DELTA[((a.dp1 + sym) & 255) as usize] as i32);
}

/// total[k] + 1, swap with k - 1 when larger (k > 1)  [scan_upd]
fn scan_upd(s: &mut Scan, k: usize) {
    let k = k & 15;
    let t = s.tot[k] as u32 + 1;
    s.tot[k] = t as u16;
    if k > 1 {
        let p = s.tot[k - 1] as u32;
        if t > p {
            s.tot[k - 1] = t as u16;
            s.tot[k] = p as u16;
            s.ord.swap(k - 1, k);
        }
    }
}

/// centred 2:1 decimation of n (even) samples: out[k] = (s[2k - 1] + 3 s[2k] + 3 s[2k + 1] +
/// s[2k + 2] + 4) >> 3, indices clamped; source / destination as (offset, step) in one buffer or
/// two  [jxw_decline]
fn decline(src: &[i32], so: usize, ss: usize, n: usize, dst: &mut [i32], d_o: usize, ds: usize) {
    for k in 0..n / 2 {
        let i = 2 * k;
        let a = src[so + if i == 0 { 0 } else { i - 1 } * ss];
        let b = src[so + i * ss];
        let c = src[so + (i + 1) * ss];
        let d = src[so + (i + 2).min(n - 1) * ss];
        dst[d_o + k * ds] = ad(ad(a, d), ad(b, c).wrapping_mul(3)).wrapping_add(4) >> 3;
    }
}

/// the same in place (k increasing reads ahead of the writes)
fn decline_inplace(p: &mut [i32], o: usize, n: usize) {
    for k in 0..n / 2 {
        let i = 2 * k;
        let a = p[o + if i == 0 { 0 } else { i - 1 }];
        let b = p[o + i];
        let c = p[o + i + 1];
        let d = p[o + (i + 2).min(n - 1)];
        p[o + k] = ad(ad(a, d), ad(b, c).wrapping_mul(3)).wrapping_add(4) >> 3;
    }
}

/// QP indices Y U V YHP UHP VHP (tools/jxr_enc_ref.py qp_indices: per mille interpolation of the
/// jxrlib tables, 4:4:4 above 80 % stretched x 1.5)  [jxw_qps]
fn qps(quality: u32, cf: u32) -> [u8; 6] {
    let mut f = quality.clamp(1, 100) * 10;
    let tab = if cf == YUV444 { &QPS8 } else { &QPS420 };
    if cf == YUV444 && f > 800 {
        f = ((f - 800) * 3 >> 1) + 800;
    }
    let (mut qi, mut qf) = (f / 100, f % 100);
    if qi >= 11 {
        qi = 11;
        qf = 0;
    }
    let r0 = qi as usize * 6;
    let r1 = (qi + 1).min(11) as usize * 6;
    let mut out = [0u8; 6];
    for j in 0..6 {
        out[j] = ((tab[r0 + j] as u32 * (100 - qf) + tab[r1 + j] as u32 * qf + 50) / 100) as u8;
    }
    out
}

/// append a 12-byte IFD entry
fn entry(f: &mut Vec<u8>, tag: u16, typ: u16, cnt: u32, val: u32) {
    f.extend_from_slice(&tag.to_le_bytes());
    f.extend_from_slice(&typ.to_le_bytes());
    f.extend_from_slice(&cnt.to_le_bytes());
    f.extend_from_slice(&val.to_le_bytes());
}

/// JPEG XR file of sf, see the module header  [jxr_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let (w, h) = (sf.w as u32, sf.h as u32);
    if w.wrapping_sub(1) > IMG_MAXDIM - 1 || h.wrapping_sub(1) > IMG_MAXDIM - 1 {
        return ERR_DIMS;
    }
    let ll = opt.flags & IOF_LOSSLESS != 0;
    let q = if opt.quality == 0 { JXR_QDEF } else { opt.quality };
    let mut cf = YUV444;
    if !ll {
        if opt.chroma == 422 {
            cf = YUV422;
        }
        if opt.chroma == 420 {
            cf = YUV420;
        }
    }
    // row buffer (MB-multiple width)
    let ww = ((w + 15) & !15) as usize;
    let mut row = vec![0u8; ww * 4];
    // alpha: opt.bitdepth 32 always, 24 never, else when some alpha < 255
    let alpha = match opt.bitdepth {
        24 => false,
        32 => true,
        _ => (0..h as usize).any(|y| {
            get_row(sf, y, &mut row, FMT_ARGB);
            row[..w as usize * 4].chunks_exact(4).any(|p| p[3] != 255)
        }),
    };
    let mut jc = Jc::new(cf, w, h, opt);
    jc.outcf = 7; // RGB
    let mut ja = if alpha { Some(Jc::new(YONLY, w, h, opt)) } else { None };
    // planes from the rows (straight BGRA), extended to MB multiples by edge replication
    let (pw, ph) = (jc.pw[0] as usize, jc.ph[0] as usize);
    let mut tu = Vec::new();
    let mut tv = Vec::new();
    if cf != YUV444 {
        tu = vec![0i32; pw * ph];
        tv = vec![0i32; pw * ph];
    }
    let sh = if ll { 0 } else { 3 };
    for y in 0..ph {
        if y < h as usize {
            get_row(sf, y, &mut row, FMT_ARGB);
            let last: [u8; 4] = row[(w as usize - 1) * 4..w as usize * 4].try_into().unwrap();
            for x in w as usize..pw {
                row[x * 4..x * 4 + 4].copy_from_slice(&last);
            } // rows past the image repeat the last one
        }
        for x in 0..pw {
            let p = &row[x * 4..x * 4 + 4];
            let b = (p[0] as i32 - 128).wrapping_shl(sh);
            let g = (p[1] as i32 - 128).wrapping_shl(sh);
            let r = (p[2] as i32 - 128).wrapping_shl(sh);
            let v = sb(b, r); // V = B - R
            let t = sb(ad(ad(v, 1) >> 1, r), g); // t = R - G + ((V + 1) >> 1)
            let yv = ad(t >> 1, g); // Y = G + (t >> 1)
            let u = t.wrapping_neg(); // U = -t
            let i = y * pw + x;
            jc.pl[0][i] = yv;
            if cf == YUV444 {
                jc.pl[1][i] = u;
                jc.pl[2][i] = v;
            } else {
                tu[i] = u;
                tv[i] = v;
            }
            if let Some(a) = ja.as_mut() {
                a.pl[0][i] = p[3] as i32 - 128;
            }
        }
    }
    if cf != YUV444 {
        // decimate tu / tv into the chroma planes (4:2:2: rows; 4:2:0: rows in place, then columns)
        let pw1 = jc.pw[1] as usize;
        for (k, t) in [&mut tu, &mut tv].into_iter().enumerate() {
            for r in 0..ph {
                if cf == YUV420 {
                    decline_inplace(t, r * pw, pw);
                } else {
                    decline(t, r * pw, 1, pw, &mut jc.pl[1 + k], r * pw1, 1);
                }
            }
            if cf == YUV420 {
                for col in 0..pw / 2 {
                    decline(t, col, pw, ph, &mut jc.pl[1 + k], col, pw1);
                }
            }
        }
    }
    drop(tu);
    drop(tv);
    // codestreams
    let qp = qps(q, cf);
    if let Err(e) = jc.cs(ll, Some(&qp)) {
        return e;
    }
    if let Some(a) = ja.as_mut()
        && let Err(e) = a.cs(true, None)
    {
        return e;
    }
    // metadata
    let mut icc: &[u8] = &[];
    let mut xmp: &[u8] = &[];
    let (mut dpix, mut dpiy) = (0u32, 0u32);
    if let Some(m) = opt.meta {
        if opt.flags & IOF_EMBED_ICC != 0 && m.icc.len() > 4 {
            icc = &m.icc;
        }
        if opt.flags & IOF_EMBED_XMP != 0 && m.xmp.len() > 4 {
            xmp = &m.xmp;
        }
        if m.dpix != 0 && m.dpiy != 0 {
            let k = f32::from_bits(0x3780_0000); // 2^-16
            dpix = ((m.dpix as i64 as f32) * k).to_bits();
            dpiy = ((m.dpiy as i64 as f32) * k).to_bits();
        }
    }
    // entries: GUID, width, height, image offset / bytes + optional ones
    let mut nt = 5u32;
    if !xmp.is_empty() {
        nt += 1;
    }
    if !icc.is_empty() {
        nt += 1;
    }
    if dpix != 0 {
        nt += 2;
    }
    if ja.is_some() {
        nt += 2;
    }
    let doff = nt * 12 + 8 + 2 + 4;
    let even = |n: usize| (n + 1) & !1;
    let jpos = jc.buf.len();
    let apos = ja.as_ref().map_or(0, |a| a.buf.len());
    let tot = doff as usize + even(xmp.len()) + even(icc.len()) + 16 + jpos + apos;
    let mut f = Vec::with_capacity(tot);
    f.extend_from_slice(&[0x49, 0x49, 0xBC, 0x01]);
    f.extend_from_slice(&8u32.to_le_bytes());
    f.extend_from_slice(&(nt as u16).to_le_bytes());
    let mut bo = doff;
    if !xmp.is_empty() {
        entry(&mut f, 0x02BC, 1, xmp.len() as u32, bo); // XMP (BYTE)
        bo = bo.wrapping_add(even(xmp.len()) as u32);
    }
    if !icc.is_empty() {
        entry(&mut f, 0x8773, 7, icc.len() as u32, bo); // ICC (UNDEFINED)
        bo = bo.wrapping_add(even(icc.len()) as u32);
    }
    entry(&mut f, 0xBC01, 1, 16, bo); // pixel format GUID
    let goff = bo;
    bo += 16;
    entry(&mut f, 0xBC80, 4, 1, w);
    entry(&mut f, 0xBC81, 4, 1, h);
    if dpix != 0 {
        entry(&mut f, 0xBC82, 11, 1, dpix); // resolution (FLOAT)
        entry(&mut f, 0xBC83, 11, 1, dpiy);
    }
    entry(&mut f, 0xBCC0, 4, 1, bo); // image codestream
    entry(&mut f, 0xBCC1, 4, 1, jpos as u32);
    let ioff = bo;
    bo = bo.wrapping_add(jpos as u32);
    let aoff = bo;
    if let Some(a) = ja.as_ref() {
        entry(&mut f, 0xBCC2, 4, 1, aoff); // alpha codestream
        entry(&mut f, 0xBCC3, 4, 1, a.buf.len() as u32);
    }
    f.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    f.resize(tot, 0);
    let mut put = |o: u32, b: &[u8]| f[o as usize..o as usize + b.len()].copy_from_slice(b);
    let mut o = doff;
    if !xmp.is_empty() {
        put(o, xmp);
        o += even(xmp.len()) as u32;
    }
    if !icc.is_empty() {
        put(o, icc);
    }
    let mut g = GUID;
    g[15] = if ja.is_some() { 0x0F } else { 0x0C }; // Bgra32 / Bgr24
    put(goff, &g);
    put(ioff, &jc.buf);
    if let Some(a) = ja.as_ref() {
        put(aoff, &a.buf);
    }
    *out = f;
    0
}
