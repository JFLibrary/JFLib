// dec -- the TIFF reader: directory, setup, strips / tiles (in parallel for big images),
// decompressors, predictors, sample conversion, YCbCr / CIELab, JPEG strips, the EXIF block
//                                                                     [src/img/tiff.s]
//
//   probe(data, meta) -> recognised                                   tiff_probe
//   decode(data, out, dec, meta) -> 0 / ERR_*                         tiff_decode
//
// The IFDs are read through img_core::exif::Tifr (II / MM, classic and BigTIFF, value spans
// checked, cycles and more than 1024 IFDs rejected).  meta.frames = IFDs in the main chain,
// dec.frame picks the page.  Supported: strips and tiles, PlanarConfiguration 1 / 2; photometric 0 / 1 grey,
// 2 RGB, 3 palette, 5 CMYK (InkSet 1), 6 YCbCr (subsampling 1 / 2 / 4 x 1 / 2 / 4,
// ReferenceBlackWhite, YCbCrCoefficients; libtiff's TIFFRGBAImage rules; JPEG-compressed
// YCbCr through the JPEG decoder), 8 CIELab (8 / 16 bit, D50 -> sRGB, Bradford); samples
// 1..32-bit unsigned / signed integers, 16 / 24 / 32-bit floats; compression none, LZW (and
// libtiff's old bit-reversed LZW), Deflate (8, 32946), PackBits, JPEG (JPEGTables), CCITT
// 2 / 3 / 4; predictor 2 and 3 (LZW / Deflate only); FillOrder 2; ExtraSamples 1
// (associated: 8 bit -> FMT_PARGB, deeper -> unpremultiplied at full precision), 2, 0 /
// missing (alpha only for 4-sample RGB and 2-sample grey); Orientation (meta.orient; the
// dispatcher applies it), resolution, ICC (34675), XMP (700), EXIF / GPS IFDs (rebuilt as
// a standalone EXIF block).  Sample depth -> 8 bits: RGB-type 16-bit samples v >> 8, 16-bit
// grey round(v / 257), other integer depths round(v * 255 / (2^n - 1)), palette entries
// trunc(v / 257), floats linear -> sRGB (alpha linear), CMYK (255 - c) * (255 - k) / 255.
// Limits: BitsPerSample 1..32, spp <= 16, tiles <= 65536 x 65536, decompressed sizes from the
// dimensions (never from the byte counts), chunk buffers <= 4 GB and dec.maxmem, the cancel
// flag polled per strip / tile.
//
// Images of 1 MP and more with several strips / tiles are decoded in parallel
// (plat::parallel_for over the strip / tile groups, one Slot -- the per-worker buffers --
// per worker, claimed through a bitmap like the assembly's context clones; every output row
// sits behind its own lock, so groups write their disjoint areas in safe code).  The error
// of the lowest failing group is returned: results never depend on the thread count.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use img_core::*;
use rt::sync::SpinLock;

use img_core::exif::{TEnt as Tent, TIFR_MAXIFD, Tifr};
use img_core::inflate::{InflateState, inflate_raw_st};
use img_core::meta;

use crate::ccitt;
use crate::lzw::{self, LIT}; // LIT: the LZW literal area in front of each chunk buffer

// ------------------------------------------------------------------ TIFF tags / values
const T_W: u32 = 256;
const T_H: u32 = 257;
const T_BPS: u32 = 258;
const T_COMP: u32 = 259;
const T_PHOT: u32 = 262;
const T_FILL: u32 = 266;
const T_SOFF: u32 = 273;
const T_ORIENT: u32 = 274;
const T_SPP: u32 = 277;
const T_RPS: u32 = 278;
const T_SCNT: u32 = 279;
const T_XRES: u32 = 282;
const T_YRES: u32 = 283;
const T_PLANAR: u32 = 284;
const T_T4: u32 = 292;
const T_T6: u32 = 293;
const T_RUNIT: u32 = 296;
const T_PRED: u32 = 317;
const T_CMAP: u32 = 320;
const T_TW: u32 = 322;
const T_TH: u32 = 323;
const T_TOFF: u32 = 324;
const T_TCNT: u32 = 325;
const T_INK: u32 = 332;
const T_EXTRA: u32 = 338;
const T_SFMT: u32 = 339;
const T_JTAB: u32 = 347;
const T_YCOEF: u32 = 529;
const T_YSUB: u32 = 530;
const T_RBW: u32 = 532;
const T_XMP: u32 = 700;
const T_EXIF: u32 = 34665;
const T_ICC: u32 = 34675;
const T_GPS: u32 = 34853;

pub(crate) const C_NONE: u32 = 1;
const C_CCITT_RLE: u32 = 2;
const C_CCITT_T6: u32 = 4;
pub(crate) const C_LZW: u32 = 5;
const C_JPEG: u32 = 7;
pub(crate) const C_DEFLATE: u32 = 8;
pub(crate) const C_PACKBITS: u32 = 32773;
const C_ADOBE_DEFLATE: u32 = 32946;

const TC_MAXSPP: u32 = 16; // samples per pixel accepted
const TC_PARPX: u64 = 1 << 20; // decode in parallel from this many pixels (and 2 groups)
const TC_MAXTILE: u32 = 65536; // largest tile width / length
const TC_SLACK: u64 = 66048; // bytes after a chunk buffer (inflate stored-block overrun)

// conversion modes
const M_GEN: u32 = 0; // unpack + convert (any layout)
const M_RGB8: u32 = 1;
const M_GRAY8: u32 = 2;
const M_PAL8: u32 = 3;
const M_BW1: u32 = 4;
const M_YSUB: u32 = 5; // YCbCr subsampled blocks
const M_JPEG: u32 = 6;

const CV_GRAY: u32 = 0;
const CV_RGB: u32 = 1;
const CV_PAL: u32 = 2;
const CV_CMYK: u32 = 3;
const CV_YCC: u32 = 4;
const CV_LAB: u32 = 5;

const NK_ID: u32 = 0; // 8 bit
const NK_SHR: u32 = 1; // 16 bit: v >> 8
const NK_R257: u32 = 2; // 16 bit: round(v / 257)
const NK_LUT: u32 = 3; // < 16 bit: table
const NK_DIV: u32 = 4; // > 16 bit: round(v * 255 / max)
const NK_FLT: u32 = 5; // float colour: linear -> sRGB
const NK_FLA: u32 = 6; // float alpha: clamp * 255

// IMM_* by photometric 0..8
static MODELS: [u8; 9] = [IMM_GRAY, IMM_GRAY, IMM_RGB, IMM_PAL, 0, IMM_CMYK, IMM_YCC, 0, IMM_LAB];
// CV_* by photometric 0..8
static CVTAB: [u32; 9] = [CV_GRAY, CV_GRAY, CV_RGB, CV_PAL, CV_GRAY, CV_CMYK, CV_YCC, CV_GRAY, CV_LAB];
// IFD0 tags copied into the EXIF block
static X0TAGS: [u32; 11] = [0x010E, 0x010F, 0x0110, 0x0112, 0x011A, 0x011B, 0x0128, 0x0131, 0x0132, 0x013B, 0x8298];

const LAB_L8: f32 = 0.392156869; // 100 / 255
const LAB_L16: f32 = 0.00152590219; // 100 / 65535
const LAB_1_256: f32 = 0.00390625;
const LAB_1_116: f32 = 0.00862068966;
const LAB_1_500: f32 = 0.002;
const LAB_1_200: f32 = 0.005;
const LAB_6_29: f32 = 0.206896552;
const LAB_4_29: f32 = 0.137931034;
const LAB_K: f32 = 0.128418549;
// XYZ(D50, f^-1 values) -> linear sRGB: M_sRGB(D65) * Bradford(D50 -> D65) * diag(Xn, 1, Zn)
static LAB_M: [f32; 9] = [
    3.021664, -1.61686649, -0.404708137, -0.943728349, 1.91614137, 0.0275962655, 0.0693696473, -0.228991373,
    1.15918468,
];

// bit reversal of a byte (FillOrder 2)
static REV8: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};

/// YCbCr -> RGB tables (libtiff's TIFFYCbCrToRGBInit)
struct Ytab {
    crr: [i32; 256], // Cr -> R
    cbb: [i32; 256], // Cb -> B
    crg: [i32; 256], // Cr -> G (<< 16)
    cbg: [i32; 256], // Cb -> G (<< 16, + 1/2)
    y: [i32; 256],
}

/// the decoder context (TC_*) without the per-worker buffers: read-only once set up
pub(crate) struct Tc<'a> {
    tr: Tifr<'a>,
    d: &'a [u8],
    esoff: Tent,
    escnt: Tent,
    etoff: Tent,
    etcnt: Tent,
    ecmap: Tent,
    ejtab: Tent,
    eicc: Tent,
    exmp: Tent,
    erbw: Tent,
    eycoef: Tent,
    ebps: Tent,
    esfmt: Tent,
    eextra: Tent,
    eysub: Tent,
    ifd: u64,
    exififd: u64,
    gpsifd: u64,
    xrn: u64,
    xrd: u64,
    yrn: u64,
    yrd: u64,
    rowb: u64,   // bytes per chunk row (one plane)
    cbytes: u64, // bytes of a full chunk (one plane)
    plstr: u64,  // distance of the plane buffers in the chunk buffer
    brb: u64,    // YCbCr subsampled: bytes per block row
    nch: u64,    // chunks per plane
    maxv: u64,   // 2^bps - 1
    w: u32,
    h: u32,
    bps: u32,
    spp: u32,
    comp: u32,
    phot: u32, // u32::MAX = missing (basic picks a default)
    planar: u32,
    pred: u32, // effective predictor (1 unless LZW / Deflate)
    sfmt: u32,
    fill: u32,
    rps: u32,
    tw: u32,
    th: u32,
    orient: u32,
    ink: u32,
    ych: u32,
    ycv: u32,
    t4: u32,
    t6: u32,
    runit: u32,
    tiled: bool,
    cw: u32, // chunk width / height
    ch: u32,
    ncx: u32, // chunks across / down
    ncy: u32,
    npl: u32,  // planes (1 or spp)
    spc: u32,  // samples per pixel in one plane
    ncol: u32, // colour samples (1 / 3 / 4)
    aidx: i32, // alpha sample index, -1 = none
    assoc: bool,
    ofmt: u8,
    mode: u32,
    cv: u32,
    nk: u32,  // NK_* colour samples
    nka: u32, // NK_* alpha
    be: bool, // multi-byte samples big endian
    float: bool,
    usub: bool,  // YCbCr subsampled block layout
    extra: u32,  // ExtraSamples[0], u32::MAX = none
    inv: bool,   // WhiteIsZero
    unpm: bool,  // unpremultiply the raw samples (associated, > 8 bit)
    sign: bool,  // signed integers -> offset binary
    xep: u32,    // EXIF block: Exif IFD position (0 = none)
    xgp: u32,    // EXIF block: GPS IFD position (0 = none)
    lut: Vec<u8>,  // sample -> 8 bit (NK_LUT)
    pal: Vec<u32>, // 2^bps palette entries, ARGB
    yt: Option<alloc::boxed::Box<Ytab>>,
}

/// the per-worker buffers (the TC_* fields tc_free_slot releases)
#[derive(Default)]
pub(crate) struct Slot {
    cbuf: Vec<u8>,  // chunk buffers: per plane LIT + data + slack
    raw: Vec<u8>,   // bit-reversed input
    uraw: Vec<u32>, // the samples of one row (generic path)
    ptmp: Vec<u8>,  // predictor 3 row
    lzwt: Option<alloc::boxed::Box<lzw::Table>>, // LZW table: offset | length << 32 per code
    cc: ccitt::Work,
    zst: Option<alloc::boxed::Box<InflateState>>, // Deflate: the slot's inflate state
}

/// the output rows, one lock each: groups running in parallel write disjoint areas
type Rows<'s> = [SpinLock<&'s mut [u8]>];

// a zeroed byte buffer or ERR_NOMEM (mem_alloc): a fallible probe first, then a zeroed
// allocation (big blocks are fresh OS pages: rt does not clear them again)
fn zbuf(n: u64) -> Result<Vec<u8>, i32> {
    let n = usize::try_from(n).map_err(|_| ERR_NOMEM)?;
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(n).map_err(|_| ERR_NOMEM)?;
    drop(probe);
    Ok(vec![0u8; n])
}

// ================================================================== entry points

/// header scan into meta (size, depth, model, alpha, orientation, dpi, page count); nothing
/// is kept  [tiff_probe]
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    let dec = ImgDec { flags: IMGD_PROBE, maxpx: IMG_MAXPX_DEF, ..Default::default() };
    let mut sf = Surface::default();
    decode(data, &mut sf, &dec, meta) == 0
}

/// decode page dec.frame into a new FMT_ARGB / FMT_PARGB surface (IMGD_PROBE: metadata
/// only, out untouched)  [tiff_decode]
pub fn decode(data: &[u8], out: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    let tr = match Tifr::open(data) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let mut tc = Tc::new(tr, data);
    match tc.run(dec, meta) {
        Ok(Some(sf)) => {
            *out = sf;
            0
        }
        Ok(None) => 0,
        Err(e) => e,
    }
}

impl<'a> Tc<'a> {
    fn new(tr: Tifr<'a>, d: &'a [u8]) -> Tc<'a> {
        let z = Tent::default();
        Tc {
            tr,
            d,
            esoff: z,
            escnt: z,
            etoff: z,
            etcnt: z,
            ecmap: z,
            ejtab: z,
            eicc: z,
            exmp: z,
            erbw: z,
            eycoef: z,
            ebps: z,
            esfmt: z,
            eextra: z,
            eysub: z,
            ifd: 0,
            exififd: 0,
            gpsifd: 0,
            xrn: 0,
            xrd: 0,
            yrn: 0,
            yrd: 0,
            rowb: 0,
            cbytes: 0,
            plstr: 0,
            brb: 0,
            nch: 0,
            maxv: 0,
            w: 0,
            h: 0,
            bps: 0,
            spp: 0,
            comp: 0,
            phot: 0,
            planar: 0,
            pred: 0,
            sfmt: 0,
            fill: 0,
            rps: 0,
            tw: 0,
            th: 0,
            orient: 0,
            ink: 0,
            ych: 0,
            ycv: 0,
            t4: 0,
            t6: 0,
            runit: 0,
            tiled: false,
            cw: 0,
            ch: 0,
            ncx: 0,
            ncy: 0,
            npl: 0,
            spc: 0,
            ncol: 0,
            aidx: 0,
            assoc: false,
            ofmt: 0,
            mode: 0,
            cv: 0,
            nk: 0,
            nka: 0,
            be: false,
            float: false,
            usub: false,
            extra: 0,
            inv: false,
            unpm: false,
            sign: false,
            xep: 0,
            xgp: 0,
            lut: Vec::new(),
            pal: Vec::new(),
            yt: None,
        }
    }

    // the whole decode: Some(surface), None for IMGD_PROBE  [tc_run]
    fn run(&mut self, dec: &ImgDec, meta: &mut ImgMeta) -> Result<Option<Surface>, i32> {
        self.pages(dec, meta)?;
        self.parse()?;
        self.basic();
        self.meta(dec, meta)?;
        if dec.flags & IMGD_PROBE != 0 {
            return Ok(None);
        }
        self.setup(dec)?;
        let mut slot = Slot::default();
        let mut sf = self.alloc(dec, &mut slot)?;
        self.chunks(dec, slot, &mut sf)?;
        Ok(Some(sf))
    }

    // ============================================================== directory

    // walk the main IFD chain: meta.frames, ifd = page dec.frame (ERR_LIMIT when there is
    // no such page); the chain ends at a zero / out-of-range pointer, a cycle or 1024 IFDs;
    // an unreadable first IFD is an error  [tc_pages]
    fn pages(&mut self, dec: &ImgDec, meta: &mut ImgMeta) -> Result<(), i32> {
        let mut off = self.tr.ifd0;
        let mut n = 0u32;
        let mut sel = None;
        while off != 0 && (n as usize) < TIFR_MAXIFD {
            let c = self.tr.ifd(off);
            if c < 0 {
                if n == 0 {
                    return Err(c as i32); // first IFD unreadable: its error
                }
                break;
            }
            if n == dec.frame {
                sel = Some(off);
            }
            n += 1;
            off = self.tr.next(off);
        }
        if n == 0 {
            return Err(ERR_HDR);
        }
        meta.frames = n;
        match sel {
            Some(o) => {
                self.ifd = o;
                Ok(())
            }
            None => Err(ERR_LIMIT),
        }
    }

    // read the tags of the selected IFD into the context; defaults first; unreadable
    // entries are skipped  [tc_parse]
    fn parse(&mut self) -> Result<(), i32> {
        self.spp = 1;
        self.comp = 1;
        self.phot = u32::MAX;
        self.planar = 1;
        self.pred = 1;
        self.fill = 1;
        self.rps = u32::MAX;
        self.orient = 1;
        self.ink = 1;
        self.ych = 2;
        self.ycv = 2;
        self.runit = 2;
        let n = self.tr.count(self.ifd);
        if n < 0 {
            return Err(ERR_TRUNC);
        }
        for i in 0..n as u64 {
            let Ok(te) = self.tr.entry(self.ifd, i) else {
                continue;
            };
            let u = || -> u32 { self.tr.get(&te, 0).min(0xFFFF_FFFF) as u32 };
            match te.tag {
                T_W => self.w = u(),
                T_H => self.h = u(),
                T_COMP => self.comp = u(),
                T_PHOT => self.phot = u(),
                T_FILL => self.fill = u(),
                T_ORIENT => self.orient = u(),
                T_SPP => self.spp = u(),
                T_RPS => self.rps = u(),
                T_PLANAR => self.planar = u(),
                T_T4 => self.t4 = u(),
                T_T6 => self.t6 = u(),
                T_RUNIT => self.runit = u(),
                T_PRED => self.pred = u(),
                T_TW => self.tw = u(),
                T_TH => self.th = u(),
                T_INK => self.ink = u(),
                T_BPS => self.ebps = te,
                T_SOFF => self.esoff = te,
                T_SCNT => self.escnt = te,
                T_CMAP => self.ecmap = te,
                T_TOFF => self.etoff = te,
                T_TCNT => self.etcnt = te,
                T_EXTRA => self.eextra = te,
                T_SFMT => self.esfmt = te,
                T_JTAB => self.ejtab = te,
                T_YCOEF => self.eycoef = te,
                T_YSUB => self.eysub = te,
                T_RBW => self.erbw = te,
                T_XMP => self.exmp = te,
                T_ICC => self.eicc = te,
                T_EXIF => self.exififd = self.tr.get(&te, 0),
                T_GPS => self.gpsifd = self.tr.get(&te, 0),
                T_XRES => (self.xrn, self.xrd) = self.tr.rational(&te, 0),
                T_YRES => (self.yrn, self.yrd) = self.tr.rational(&te, 0),
                _ => {} // a tag we do not use
            }
        }
        Ok(())
    }

    // first value of an entry clamped to 0..0xFFFF, def when the entry is missing  [tc_first]
    fn first(&self, te: &Tent, def: u32) -> u32 {
        if te.count == 0 { def } else { self.tr.get(te, 0).min(0xFFFF) as u32 }
    }

    // derived values needed for the metadata: BitsPerSample, SampleFormat, ExtraSamples,
    // default photometric, colour / alpha samples, tiling  [tc_basic]
    fn basic(&mut self) {
        self.bps = self.first(&self.ebps, 1);
        let mut sf = self.first(&self.esfmt, 1);
        if sf == 4 {
            sf = 1; // "void" -> unsigned
        }
        self.sfmt = sf;
        self.float = sf == 3;
        self.extra = self.first(&self.eextra, u32::MAX);
        if self.eysub.count >= 2 {
            self.ych = self.tr.get(&self.eysub, 0).min(0xFFFF) as u32;
            self.ycv = self.tr.get(&self.eysub, 1).min(0xFFFF) as u32;
        }
        // photometric default (libtiff's guesses)
        if self.phot == u32::MAX {
            self.phot = if self.spp >= 3 {
                2
            } else if self.comp.wrapping_sub(C_CCITT_RLE) > C_CCITT_T6 - C_CCITT_RLE {
                1
            } else {
                0
            };
        }
        let ph = self.phot;
        // colour samples
        let ncol = match ph {
            2 | 6 | 8 => 3,
            5 => 4,
            _ => 1,
        };
        self.ncol = ncol;
        // alpha: ExtraSamples 1 / 2, or 0 / missing for RGBA and grey + alpha
        self.aidx = -1;
        let spp = self.spp;
        if spp > ncol {
            let x = self.extra;
            let alpha = match x {
                1 | 2 => true,
                0 | u32::MAX => (ph == 2 && spp == 4) || (ph <= 1 && spp == 2),
                _ => false,
            };
            if alpha {
                self.aidx = ncol as i32;
                if x == 1 {
                    self.assoc = true;
                }
            }
        }
        if self.tw != 0 && self.th != 0 && self.etoff.count != 0 {
            self.tiled = true;
        }
    }

    // metadata scalars and the requested blobs  [tc_meta]
    fn meta(&mut self, dec: &ImgDec, meta: &mut ImgMeta) -> Result<(), i32> {
        meta.w = self.w;
        meta.h = self.h;
        meta.bits = self.bps.min(255) as u8;
        meta.channels = self.spp.min(255) as u8;
        meta.model = if self.phot <= 8 { MODELS[self.phot as usize] } else { 0 };
        meta.orient = if self.orient.wrapping_sub(1) <= 7 { self.orient as u8 } else { 1 };
        let mut f = meta.flags;
        if self.aidx >= 0 {
            f |= IMF_ALPHA;
            if self.assoc {
                f |= IMF_SRC_PREMUL;
            }
        }
        if self.float {
            f |= IMF_FLOAT | IMF_HDR;
        }
        if self.bps > 8 {
            f |= IMF_HDR;
        }
        if self.comp != C_JPEG {
            f |= IMF_LOSSLESS;
        }
        meta.flags = f;
        // resolution
        let unit = self.runit;
        let ru = match unit {
            2 => IMR_INCH,
            3 => IMR_CM,
            _ => IMR_NONE,
        };
        meta.resunit = ru;
        if ru != 0 {
            meta.dpix = dpi(self.xrn, self.xrd, unit);
            meta.dpiy = dpi(self.yrn, self.yrd, unit);
        }
        let fl = dec.flags;
        if fl & IMGD_ICC != 0 && self.eicc.bytes != 0 {
            let o = self.eicc.voff as usize;
            meta.icc = self.d[o..o + self.eicc.bytes as usize].to_vec();
        }
        if fl & IMGD_XMP != 0 && self.exmp.bytes != 0 {
            let o = self.exmp.voff as usize;
            meta.xmp = self.d[o..o + self.exmp.bytes as usize].to_vec();
        }
        if fl & IMGD_EXIF != 0 {
            self.exif(meta);
        }
        Ok(())
    }

    // ============================================================== setup

    // validate the IFD and derive the geometry, buffer sizes, output format and conversion
    // mode  [tc_setup]
    fn setup(&mut self, dec: &ImgDec) -> Result<(), i32> {
        let e = meta::dims_check(self.w, self.h, dec);
        if e != 0 {
            return Err(e);
        }
        // samples per pixel, bits per sample
        if self.spp == 0 {
            return Err(ERR_HDR);
        }
        if self.spp > TC_MAXSPP {
            return Err(ERR_UNSUPPORTED);
        }
        if self.bps == 0 {
            return Err(ERR_HDR);
        }
        if self.bps > 32 {
            return Err(ERR_UNSUPPORTED);
        }
        let nb = self.ebps.count.min(self.spp as u64);
        for i in 1..nb {
            if self.tr.get(&self.ebps, i) != self.bps as u64 {
                return Err(ERR_UNSUPPORTED); // different depths per sample
            }
        }
        match self.sfmt {
            0 => return Err(ERR_UNSUPPORTED),
            3 => {
                if !matches!(self.bps, 16 | 24 | 32) {
                    return Err(ERR_UNSUPPORTED); // float: 16 / 24 / 32 bit
                }
            }
            2 => self.sign = true,
            1 => {}
            _ => return Err(ERR_UNSUPPORTED),
        }
        // planar configuration
        match self.planar {
            1 => {}
            2 => {
                if self.spp == 1 {
                    self.planar = 1;
                }
            }
            _ => return Err(ERR_HDR),
        }
        // compression and predictor
        let comp = self.comp;
        let mut pred = self.pred;
        match comp {
            C_LZW | C_DEFLATE | C_ADOBE_DEFLATE => {}
            _ => {
                pred = 1; // libtiff: predictors only with LZW / Deflate
                match comp {
                    C_NONE | C_PACKBITS | C_JPEG => {}
                    2..=4 => {
                        if self.bps != 1 || self.spp != 1 {
                            return Err(ERR_UNSUPPORTED); // CCITT: bilevel only
                        }
                    }
                    _ => return Err(ERR_UNSUPPORTED),
                }
            }
        }
        self.pred = pred;
        match pred {
            1 => {}
            2 => {
                if self.bps == 24 {
                    return Err(ERR_UNSUPPORTED);
                }
            }
            3 => {
                if !self.float {
                    return Err(ERR_UNSUPPORTED); // predictor 3: floating point only
                }
            }
            _ => return Err(ERR_UNSUPPORTED),
        }
        // photometric
        let ph = self.phot;
        let spp = self.spp;
        let bps = self.bps;
        if ph > 1 {
            if self.float && ph != 2 {
                return Err(ERR_UNSUPPORTED); // floats: grey and RGB only
            }
            let need3 = match ph {
                2 => true,
                3 => {
                    if bps > 16 {
                        return Err(ERR_UNSUPPORTED);
                    }
                    if self.ecmap.count < 3 {
                        return Err(ERR_HDR);
                    }
                    false
                }
                5 => {
                    if self.ink != 1 {
                        return Err(ERR_UNSUPPORTED);
                    }
                    if spp < 4 {
                        return Err(ERR_HDR);
                    }
                    false
                }
                6 => {
                    if self.comp != C_JPEG && bps != 8 {
                        return Err(ERR_UNSUPPORTED);
                    }
                    true
                }
                8 => {
                    if bps != 8 && bps != 16 {
                        return Err(ERR_UNSUPPORTED); // CIELab 8 / 16 bit
                    }
                    true
                }
                _ => return Err(ERR_UNSUPPORTED),
            };
            if need3 && spp < 3 {
                return Err(ERR_HDR);
            }
        }
        // JPEG: 8-bit contiguous data without extra samples
        if self.comp == C_JPEG {
            if bps != 8 || self.planar != 1 || spp != self.ncol || ph == 3 || ph == 8 {
                return Err(ERR_UNSUPPORTED);
            }
            self.mode = M_JPEG;
        }
        // chunk geometry
        if !self.tiled {
            let mut rps = self.rps;
            if rps == 0 || rps > self.h {
                rps = self.h;
            }
            self.ch = rps;
            self.cw = self.w;
            self.ncx = 1;
            self.ncy = (self.h + rps - 1) / rps;
        } else {
            if self.tw > TC_MAXTILE || self.th > TC_MAXTILE {
                return Err(ERR_UNSUPPORTED);
            }
            self.cw = self.tw;
            self.ch = self.th;
            self.ncx = (self.w + self.cw - 1) / self.cw;
            self.ncy = (self.h + self.ch - 1) / self.ch;
        }
        if self.planar == 2 {
            self.npl = spp;
            self.spc = 1;
        } else {
            self.npl = 1;
            self.spc = spp;
        }
        // bytes per chunk row = ceil(cw * spc * bps / 8)
        self.rowb = (self.cw as u64 * self.spc as u64 * bps as u64 + 7) >> 3;
        self.cbytes = self.rowb * self.ch as u64;
        // YCbCr with subsampling: blocks of h * v Y samples + Cb + Cr
        if ph == 6 && self.comp != C_JPEG {
            let (yh, yv) = (self.ych, self.ycv);
            if !((yh & yv) == 1 && yh == 1) {
                let ok = matches!(yh, 1 | 2 | 4) && matches!(yv, 1 | 2 | 4);
                if !ok || self.planar != 1 || spp != 3 || self.pred != 1 {
                    return Err(ERR_UNSUPPORTED);
                }
                self.usub = true;
                self.mode = M_YSUB;
                let bpr = (self.cw + yh - 1) / yh; // blocks per row
                self.brb = bpr as u64 * (yh * yv + 2) as u64;
                let brows = (self.ch + yv - 1) / yv;
                self.cbytes = brows as u64 * self.brb;
            }
        }
        // chunk offsets / byte counts
        self.nch = self.ncx as u64 * self.ncy as u64;
        let need = self.nch * self.npl as u64;
        let (eo, ec) = if self.tiled { (&self.etoff, &self.etcnt) } else { (&self.esoff, &self.escnt) };
        if eo.count < need {
            return Err(ERR_HDR);
        }
        if ec.count != 0 && ec.count < need {
            return Err(ERR_HDR);
        }
        // buffer sizes: npl * align64(LIT + cbytes + TC_SLACK) <= 4 GB and dec.maxmem
        self.plstr = (self.cbytes + LIT as u64 + TC_SLACK + 63) & !63;
        let total = self.plstr * self.npl as u64;
        if total > 1 << 32 {
            return Err(ERR_LIMIT);
        }
        let e = meta::mem_check(total, dec);
        if e != 0 {
            return Err(e);
        }
        // sample layout
        self.maxv = (1u64 << bps) - 1;
        self.be = self.tr.be && self.pred != 3; // predictor 3 leaves native (LE) samples
        self.inv = ph == 0;
        if ph == 8 {
            self.sign = false; // CIELab a / b are always signed
        }
        // normalisation of colour / alpha samples
        if self.float {
            self.nk = NK_FLT;
            self.nka = NK_FLA;
        } else if bps == 8 {
            self.nk = NK_ID;
            self.nka = NK_ID;
        } else if bps < 16 {
            self.nk = NK_LUT;
            self.nka = NK_LUT;
        } else if bps > 16 {
            self.nk = NK_DIV;
            self.nka = NK_DIV;
        } else {
            self.nk = if ph <= 1 { NK_R257 } else { NK_SHR }; // 16-bit grey: round(v / 257)
            self.nka = NK_SHR;
        }
        // output format: opaque -> as requested, associated 8 bit -> PARGB, else ARGB
        let mut of = if dec.flags & IMGD_STRAIGHT != 0 { FMT_ARGB } else { FMT_PARGB };
        if self.aidx >= 0 {
            of = FMT_ARGB;
            if self.assoc {
                if bps == 8 && !self.float {
                    of = FMT_PARGB;
                } else {
                    self.unpm = true;
                }
            }
        }
        self.ofmt = of;
        // conversion
        self.cv = if ph <= 8 { CVTAB[ph as usize] } else { CV_GRAY };
        if self.mode == M_GEN && !self.float && !self.sign && self.npl == 1 {
            if bps == 1 {
                if ph <= 1 && spp == 1 {
                    self.mode = M_BW1;
                }
            } else if bps == 8 {
                if ph <= 1 {
                    self.mode = M_GRAY8;
                } else if ph == 2 {
                    self.mode = M_RGB8;
                } else if ph == 3 && spp == 1 {
                    self.mode = M_PAL8;
                }
            }
        }
        Ok(())
    }

    // ============================================================== buffers

    // slot 0's buffers, tables, the surface  [tc_alloc]
    fn alloc(&mut self, dec: &ImgDec, slot: &mut Slot) -> Result<Surface, i32> {
        self.alloc_slot(slot)?;
        if self.nk == NK_LUT {
            let n = 1usize << self.bps; // bps 1..15
            let maxv = self.maxv;
            let mut lut = vec![0u8; n];
            for (v, e) in lut.iter_mut().enumerate() {
                *e = ((v as u64 * 510 + maxv) / (maxv * 2)) as u8; // round(v * 255 / max)
            }
            self.lut = lut;
        }
        if self.cv == CV_PAL {
            self.palette();
        }
        if self.phot == 6 {
            self.ycc_init();
        }
        let e = meta::dims_check(self.w, self.h, dec);
        if e != 0 {
            return Err(e);
        }
        Ok(Surface::new(self.w as i32, self.h as i32, self.ofmt))
    }

    // the per-worker buffers: chunk buffers (literal area 0..255 in front of each plane),
    // raw sample row, predictor 3 row, LZW table, inflate state  [tc_alloc_slot]
    fn alloc_slot(&self, s: &mut Slot) -> Result<(), i32> {
        if self.mode == M_JPEG {
            return Ok(());
        }
        let mut cbuf = zbuf(self.plstr * self.npl as u64)?;
        for p in 0..self.npl as usize {
            let b = p * self.plstr as usize;
            for (i, e) in cbuf[b..b + LIT].iter_mut().enumerate() {
                *e = i as u8;
            }
        }
        s.cbuf = cbuf;
        if self.mode == M_GEN {
            s.uraw = vec![0u32; self.cw as usize * self.spp as usize + 16];
        }
        if self.pred == 3 {
            s.ptmp = zbuf(self.rowb + 64)?;
        }
        if self.comp == C_LZW {
            s.lzwt = Some(lzw::table());
        }
        if self.comp == C_DEFLATE || self.comp == C_ADOBE_DEFLATE {
            s.zst = Some(InflateState::new()); // Z1: this slot's inflate state
        }
        Ok(())
    }

    // 2^bps opaque palette entries from the ColorMap (n = count / 3 entries per channel,
    // missing ones black); 16-bit values -> trunc(v / 257) like WIC  [tc_palette]
    fn palette(&mut self) {
        let n = 1usize << self.bps;
        let per = self.ecmap.count / 3;
        let mut pal = vec![0u32; n];
        for (i, e) in pal.iter_mut().enumerate() {
            let i = i as u64;
            let mut v = 0xFF00_0000u32;
            if i < per {
                v |= self.cmap8(i) << 16 | self.cmap8(i + per) << 8 | self.cmap8(i + per * 2);
            }
            *e = v;
        }
        self.pal = pal;
    }

    // colour map value i as 8 bits: trunc(v / 257)  [tc_cmap8]
    fn cmap8(&self, i: u64) -> u32 {
        self.tr.get(&self.ecmap, i).min(0xFFFF) as u32 / 257
    }

    // ============================================================== chunks

    // decode every strip / tile group into the surface: in parallel for images of TC_PARPX
    // pixels and more with at least 2 groups, else in order  [tc_chunks]
    fn chunks(&self, dec: &ImgDec, slot: Slot, sf: &mut Surface) -> Result<(), i32> {
        let groups = self.ncx as u64 * self.ncy as u64;
        let stride = sf.stride as usize;
        let h = self.h as usize;
        let rows: Vec<SpinLock<&mut [u8]>> = sf.px.chunks_mut(stride).take(h).map(SpinLock::new).collect();
        let mut slot = Some(slot);
        if self.w as u64 * self.h as u64 >= TC_PARPX && groups >= 2 {
            let n = plat::thr_count();
            if n != 0 {
                match self.par(dec, groups, n + 1, &mut slot, &rows) {
                    Some(r) => return r,
                    None => {} // not enough memory for the slots: in order
                }
            }
        }
        let mut s = slot.unwrap_or_default();
        for g in 0..groups {
            if dec.cancelled() {
                return Err(ERR_CANCEL);
            }
            let e = self.group(&mut s, g, &rows);
            if e != 0 {
                return Err(e);
            }
        }
        Ok(())
    }

    // the parallel driver: None = no memory for the slots (decode in order)  [tc_par]
    fn par(&self, dec: &ImgDec, groups: u64, nslot: u32, slot0: &mut Option<Slot>, rows: &Rows) -> Option<Result<(), i32>> {
        let n = (nslot.min(64) as u64).min(groups) as usize;
        // the slots' buffers must fit into dec.maxmem (else in order)
        let per = self.plstr * self.npl as u64;
        if meta::mem_check((n as u64 - 1) * per, dec) != 0 {
            return None;
        }
        let mut slots: Vec<SpinLock<Slot>> = Vec::new();
        if slots.try_reserve_exact(n).is_err() {
            return None;
        }
        slots.push(SpinLock::new(slot0.take().unwrap_or_default()));
        for _ in 1..n {
            let mut s = Slot::default();
            if self.alloc_slot(&mut s).is_err() {
                *slot0 = Some(core::mem::take(&mut *slots[0].lock()));
                return None;
            }
            slots.push(SpinLock::new(s));
        }
        let map = AtomicU64::new(0); // busy slots
        let errpk = AtomicU64::new(u64::MAX); // first failing item: index << 32 | error
        let item = |g: usize, job: &plat::JobCtl| {
            let e = if dec.cancelled() {
                ERR_CANCEL
            } else {
                // take a free slot
                let k = 'claim: loop {
                    for k in 0..n {
                        if map.fetch_or(1 << k, Ordering::AcqRel) & (1 << k) == 0 {
                            break 'claim k;
                        }
                    }
                    core::hint::spin_loop();
                };
                let e = self.group(&mut slots[k].lock(), g as u64, rows);
                map.fetch_and(!(1u64 << k), Ordering::AcqRel);
                e
            };
            if e != 0 {
                errpk.fetch_min((g as u64) << 32 | e as u32 as u64, Ordering::AcqRel);
                job.cancel();
            }
        };
        let r = plat::parallel_for(groups as usize, &item);
        let pk = errpk.load(Ordering::Acquire);
        Some(if pk != u64::MAX {
            Err(pk as u32 as i32) // the error of the lowest failing group
        } else if r.is_err() {
            Err(ERR_CANCEL)
        } else {
            Ok(())
        })
    }

    // decode strip / tile group g (all planes) and convert it into the surface  [tc_group]
    fn group(&self, s: &mut Slot, g: u64, rows: &Rows) -> i32 {
        let cy = (g / self.ncx as u64) as u32;
        let cx = (g % self.ncx as u64) as u32;
        let x0 = cx * self.cw;
        let cols = (self.w - x0).min(self.cw);
        let y0 = cy * self.ch;
        let nrows = (self.h - y0).min(self.ch);
        let idx = (cy * self.ncx + cx) as u64; // chunk index in plane 0
        if self.mode == M_JPEG {
            return self.jpeg(s, idx, x0, y0, cols, nrows, rows);
        }
        let ps = self.plstr as usize;
        for p in 0..self.npl as usize {
            let e = self.decomp(s, self.nch * p as u64 + idx, p * ps, nrows);
            if e != 0 {
                return e;
            }
        }
        self.put(s, x0, y0, cols, nrows, rows);
        0
    }

    // pointer (offset) to the data of chunk idx (None = offset outside the file) and its
    // byte count clamped to the file (missing counts: the chunk size uncompressed, else up
    // to the end of the file)  [tc_span]
    fn span(&self, idx: u64) -> Option<(usize, usize)> {
        let eo = if self.tiled { &self.etoff } else { &self.esoff };
        let off = self.tr.get(eo, idx);
        let len = self.d.len() as u64;
        if off > len {
            return None;
        }
        let ec = if self.tiled { &self.etcnt } else { &self.escnt };
        let n = if ec.count != 0 {
            self.tr.get(ec, idx)
        } else if self.comp == C_NONE {
            self.cbytes
        } else {
            len
        };
        Some((off as usize, n.min(len - off) as usize))
    }

    // decompress chunk idx into the plane buffer at cbuf[pb..] (data at pb + LIT, capacity
    // cbytes), check that `rows` rows arrived, undo the predictor  [tc_decomp]
    fn decomp(&self, s: &mut Slot, idx: u64, pb: usize, rows: u32) -> i32 {
        let need = if !self.usub {
            self.rowb * rows as u64
        } else {
            self.brb * ((rows + self.ycv - 1) / self.ycv) as u64
        };
        let Some((off, n)) = self.span(idx) else {
            return ERR_TRUNC;
        };
        let plane = &mut s.cbuf[pb..pb + self.plstr as usize];
        if n == 0 {
            plane[LIT..LIT + need as usize].fill(0); // no data: zeros
            return 0;
        }
        let mut src = &self.d[off..off + n];
        if self.fill == 2 {
            if s.raw.len() < n + 64 {
                s.raw = vec![0u8; n + 64];
            }
            for (o, &b) in s.raw.iter_mut().zip(src) {
                *o = REV8[b as usize];
            }
            src = &s.raw[..n];
        }
        let cap = self.cbytes;
        let r: i64 = match self.comp {
            C_NONE => {
                let m = (n as u64).min(cap) as usize;
                plane[LIT..LIT + m].copy_from_slice(&src[..m]);
                m as i64
            }
            C_LZW => lzw::decode(src, plane, cap as usize, s.lzwt.as_mut().expect("lzw table")),
            C_PACKBITS => packbits(src, &mut plane[LIT..], cap as usize),
            C_DEFLATE | C_ADOBE_DEFLATE => {
                if n < 2 {
                    return ERR_TRUNC;
                }
                let (b0, b1) = (src[0] as u32, src[1] as u32);
                if b0 & 15 != 8 || b1 & 0x20 != 0 || (b0 << 8 | b1) % 31 != 0 {
                    return ERR_HDR; // not deflate / preset dictionary / check bits
                }
                let dcap = (cap + TC_SLACK - 64) as usize; // room for a whole stored block / match
                match inflate_raw_st(s.zst.as_deref_mut(), &src[2..], &mut plane[LIT..LIT + dcap]) {
                    Ok((n, _)) => n as i64,
                    Err(ERR_OVERFLOW) => cap as i64, // more data than the chunk: complete
                    Err(e) => e as i64,
                }
            }
            _ => {
                // CCITT 2 / 3 / 4
                let prm = ccitt::Prm {
                    w: self.cw,
                    rows,
                    rowb: self.rowb as u32,
                    comp: self.comp,
                    opt: if self.comp == C_CCITT_T6 { self.t6 } else { self.t4 },
                };
                ccitt::decode(src, &mut plane[LIT..], &prm, &mut s.cc)
            }
        };
        if r < 0 {
            return r as i32;
        }
        if (r as u64) < need {
            return ERR_TRUNC;
        }
        match self.pred {
            2 => self.pred2(&mut plane[LIT..], rows),
            3 => self.pred3(&mut plane[LIT..], &mut s.ptmp, rows),
            _ => {}
        }
        0
    }

    // ============================================================== predictors

    // undo horizontal differencing on `rows` rows (8 / 16 / 32-bit samples in the file's
    // byte order, other depths bit-packed)  [tc_pred2]
    fn pred2(&self, buf: &mut [u8], rows: u32) {
        let n = self.cw as usize * self.spc as usize; // samples per row
        let d = self.spc as usize; // distance to the previous sample
        let bps = self.bps;
        let rowb = self.rowb as usize;
        for r in 0..rows as usize {
            let row = &mut buf[r * rowb..];
            match bps {
                8 => {
                    let row = &mut row[..n];
                    match d {
                        3 => {
                            let (mut a, mut b, mut c) = (row[0], row[1], row[2]);
                            for px in row[3..].chunks_exact_mut(3) {
                                a = a.wrapping_add(px[0]);
                                b = b.wrapping_add(px[1]);
                                c = c.wrapping_add(px[2]);
                                px[0] = a;
                                px[1] = b;
                                px[2] = c;
                            }
                        }
                        4 => {
                            let (mut a, mut b, mut c, mut e) = (row[0], row[1], row[2], row[3]);
                            for px in row[4..].chunks_exact_mut(4) {
                                a = a.wrapping_add(px[0]);
                                b = b.wrapping_add(px[1]);
                                c = c.wrapping_add(px[2]);
                                e = e.wrapping_add(px[3]);
                                px[0] = a;
                                px[1] = b;
                                px[2] = c;
                                px[3] = e;
                            }
                        }
                        _ => {
                            for i in d..n {
                                row[i] = row[i].wrapping_add(row[i - d]);
                            }
                        }
                    }
                }
                16 => {
                    for i in d..n {
                        let (p, c) = ((i - d) * 2, i * 2);
                        let v = if self.be {
                            u16::from_be_bytes([row[c], row[c + 1]])
                                .wrapping_add(u16::from_be_bytes([row[p], row[p + 1]]))
                                .to_be_bytes()
                        } else {
                            u16::from_le_bytes([row[c], row[c + 1]])
                                .wrapping_add(u16::from_le_bytes([row[p], row[p + 1]]))
                                .to_le_bytes()
                        };
                        row[c..c + 2].copy_from_slice(&v);
                    }
                }
                32 => {
                    for i in d..n {
                        let (p, c) = ((i - d) * 4, i * 4);
                        let a = [row[p], row[p + 1], row[p + 2], row[p + 3]];
                        let b = [row[c], row[c + 1], row[c + 2], row[c + 3]];
                        let v = if self.be {
                            u32::from_be_bytes(b).wrapping_add(u32::from_be_bytes(a)).to_be_bytes()
                        } else {
                            u32::from_le_bytes(b).wrapping_add(u32::from_le_bytes(a)).to_le_bytes()
                        };
                        row[c..c + 4].copy_from_slice(&v);
                    }
                }
                _ => {
                    let b = bps as u64;
                    let mask = (1u64 << b) - 1;
                    for i in d..n {
                        let pp = (i - d) as u64 * b; // previous sample
                        let w = rd64be(row, (pp >> 3) as usize);
                        let prev = (w << (pp & 7)) >> (64 - b);
                        let cp = i as u64 * b; // this sample
                        let at = (cp >> 3) as usize;
                        let w = rd64be(row, at);
                        let sh = cp & 7;
                        let fs = 64 - b - sh; // shift of the field
                        let v = ((w << sh) >> (64 - b)).wrapping_add(prev) & mask;
                        let w = (w & !(mask << fs)) | (v << fs);
                        row[at..at + 8].copy_from_slice(&w.to_be_bytes());
                    }
                }
            }
        }
    }

    // undo the floating point predictor (byte-wise differencing, then the byte planes MSB
    // first -> little-endian samples)  [tc_pred3]
    fn pred3(&self, buf: &mut [u8], tmp: &mut [u8], rows: u32) {
        let spc = self.spc as usize;
        let bsz = (self.bps >> 3) as usize; // bytes per sample
        let n = self.cw as usize * spc; // samples per row
        let rb = n * bsz; // bytes per row
        let rowb = self.rowb as usize;
        for r in 0..rows as usize {
            let row = &mut buf[r * rowb..r * rowb + rb];
            for i in spc..rb {
                row[i] = row[i].wrapping_add(row[i - spc]);
            }
            tmp[..rb].copy_from_slice(row);
            for k in 0..n {
                for j in 0..bsz {
                    row[k * bsz + j] = tmp[(bsz - 1 - j) * n + k];
                }
            }
        }
    }

    // ============================================================== pixels

    // convert a decoded chunk (group) into the surface at (x0, y0)  [tc_put]
    fn put(&self, s: &mut Slot, x0: u32, y0: u32, cols: u32, nrows: u32, rows: &Rows) {
        if self.mode == M_YSUB {
            self.ysub(s, x0, y0, cols, nrows, rows);
            return;
        }
        let ps = self.plstr as usize;
        let rowb = self.rowb as usize;
        let (a, b) = ((x0 * 4) as usize, ((x0 + cols) * 4) as usize);
        let mut pl = [0usize; 16];
        for r in 0..nrows as usize {
            for (p, e) in pl.iter_mut().enumerate().take(self.npl as usize) {
                *e = p * ps + LIT + rowb * r;
            }
            let mut g = rows[y0 as usize + r].lock();
            let dst = &mut g[a..b];
            self.row(s, &pl, dst, cols as usize);
        }
    }

    // n pixels of the current chunk row (planes at s.cbuf[pl[p]..]) -> dst  [tc_row]
    fn row(&self, s: &mut Slot, pl: &[usize; 16], dst: &mut [u8], n: usize) {
        match self.mode {
            M_RGB8 => self.rgb8(&s.cbuf[pl[0]..], dst, n),
            M_GRAY8 => self.gray8(&s.cbuf[pl[0]..], dst, n),
            M_PAL8 => {
                let pal: &[u32; 256] = self.pal[..256].try_into().unwrap(); // bps 8
                for (o, &i) in dst.chunks_exact_mut(4).zip(&s.cbuf[pl[0]..pl[0] + n]) {
                    o.copy_from_slice(&pal[i as usize].to_le_bytes());
                }
            }
            M_BW1 => {
                let (b0, b1) = if self.inv { (0xFFFF_FFFFu32, 0xFF00_0000u32) } else { (0xFF00_0000, 0xFFFF_FFFF) };
                let src = &s.cbuf[pl[0]..];
                for (x, o) in dst.chunks_exact_mut(4).enumerate().take(n) {
                    let bit = (src[x >> 3] << (x & 7)) & 0x80 != 0;
                    o.copy_from_slice(&(if bit { b1 } else { b0 }).to_le_bytes());
                }
            }
            _ => {
                self.unpack(&s.cbuf, pl, &mut s.uraw, n);
                self.conv(&mut s.uraw, dst, n);
            }
        }
    }

    // 8-bit RGB (+ alpha at sample 3, + other extra samples)  [tc_rgb8]
    fn rgb8(&self, src: &[u8], dst: &mut [u8], n: usize) {
        let mode = if self.aidx < 0 {
            0 // opaque
        } else if self.ofmt != FMT_PARGB {
            1 // straight alpha
        } else {
            2 // associated: colour <= alpha
        };
        match (self.spp, mode) {
            (3, _) => rgb8_px::<3, 0>(src, dst, n),
            (4, 0) => rgb8_px::<4, 0>(src, dst, n),
            (4, 1) => rgb8_px::<4, 1>(src, dst, n),
            (4, _) => rgb8_px::<4, 2>(src, dst, n),
            _ => {
                let spp = self.spp as usize;
                for (o, q) in dst[..n * 4].chunks_exact_mut(4).zip(src[..n * spp].chunks_exact(spp)) {
                    let px = match mode {
                        0 => [q[2], q[1], q[0], 255],
                        1 => [q[2], q[1], q[0], q[3]],
                        _ => {
                            let a = q[3];
                            [q[2].min(a), q[1].min(a), q[0].min(a), a]
                        }
                    };
                    o.copy_from_slice(&px);
                }
            }
        }
    }

    // 8-bit grey (+ alpha at sample 1, + extra samples)  [tc_gray8]
    fn gray8(&self, src: &[u8], dst: &mut [u8], n: usize) {
        let inv = if self.inv { 255u8 } else { 0 };
        let alpha = self.aidx >= 0;
        let pm = self.ofmt == FMT_PARGB;
        match (self.spp, alpha) {
            (1, _) => {
                for (o, &g) in dst[..n * 4].chunks_exact_mut(4).zip(&src[..n]) {
                    let v = g ^ inv;
                    o.copy_from_slice(&[v, v, v, 255]);
                }
            }
            (2, true) => {
                for (o, q) in dst[..n * 4].chunks_exact_mut(4).zip(src[..n * 2].chunks_exact(2)) {
                    let a = q[1];
                    let v = if pm { (q[0] ^ inv).min(a) } else { q[0] ^ inv };
                    o.copy_from_slice(&[v, v, v, a]);
                }
            }
            _ => {
                let spp = self.spp as usize;
                for (o, q) in dst[..n * 4].chunks_exact_mut(4).zip(src[..n * spp].chunks_exact(spp)) {
                    let mut v = q[0] ^ inv;
                    let mut a = 255u8;
                    if alpha {
                        a = q[1];
                        if pm {
                            v = v.min(a);
                        }
                    }
                    o.copy_from_slice(&[v, v, v, a]);
                }
            }
        }
    }

    // the samples of n pixels of the current row as u32 in uraw (pixel-interleaved, spp
    // per pixel); signed -> offset binary, floats -> f32 bit patterns  [tc_unpack]
    fn unpack(&self, cbuf: &[u8], pl: &[usize; 16], uraw: &mut [u32], n: usize) {
        let spp = self.spp as usize;
        let step = if self.npl == 1 { 1 } else { spp };
        let cnt = n * self.spc as usize; // samples in this plane row
        let be = self.be;
        for p in 0..self.npl as usize {
            let src = &cbuf[pl[p]..];
            let mut o = p;
            match self.bps {
                8 => {
                    for &v in &src[..cnt] {
                        uraw[o] = v as u32;
                        o += step;
                    }
                }
                16 => {
                    for q in src[..cnt * 2].chunks_exact(2) {
                        let b = [q[0], q[1]];
                        uraw[o] = (if be { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }) as u32;
                        o += step;
                    }
                }
                32 => {
                    for q in src[..cnt * 4].chunks_exact(4) {
                        let b = [q[0], q[1], q[2], q[3]];
                        uraw[o] = if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) };
                        o += step;
                    }
                }
                24 => {
                    for q in src[..cnt * 3].chunks_exact(3) {
                        let (lo, hi) = if be { (q[2], q[0]) } else { (q[0], q[2]) };
                        uraw[o] = (hi as u32) << 16 | (q[1] as u32) << 8 | lo as u32;
                        o += step;
                    }
                }
                b => {
                    // other depths: MSB-first bit stream
                    let b = b as u64;
                    for k in 0..cnt as u64 {
                        let pos = k * b;
                        let w = rd64be(src, (pos >> 3) as usize);
                        uraw[o] = ((w << (pos & 7)) >> (64 - b)) as u32;
                        o += step;
                    }
                }
            }
        }
        let all = &mut uraw[..n * spp];
        if self.sign {
            let x = ((self.maxv >> 1) as u32).wrapping_add(1); // 2^(bps - 1)
            for v in all.iter_mut() {
                *v ^= x;
            }
        } else if self.float {
            match self.bps {
                16 => {
                    for v in all.iter_mut() {
                        *v = meta::half_to_f32(*v);
                    }
                }
                24 => {
                    for v in all.iter_mut() {
                        *v = f24_to_f32(*v);
                    }
                }
                _ => {}
            }
        }
    }

    // one raw sample (uraw value) -> 0..255  [tc_nrm]
    #[inline(always)]
    fn nrm(&self, v: u32, kind: u32) -> u32 {
        match kind {
            NK_ID => v,
            NK_SHR => v >> 8,
            NK_R257 => ((v & 0xFFFF) * 255 + 32895) >> 16,
            NK_LUT => self.lut[(v as u64 & self.maxv) as usize] as u32,
            NK_DIV => ((v as u64 * 510 + self.maxv) / (self.maxv * 2)) as u32,
            NK_FLA => {
                let mut b = [0u8];
                meta::row_f32_to_u8(&mut b, &[f32::from_bits(v)]);
                b[0] as u32
            }
            _ => {
                let mut b = [0u8];
                meta::row_f32_to_srgb8(&mut b, &[f32::from_bits(v)]);
                b[0] as u32
            }
        }
    }

    // unpremultiply the colour samples of one raw pixel (associated alpha deeper than 8
    // bits / float): v * max / a rounded (integers), v / a (floats)  [tc_unpm]
    fn unpm(&self, q: &mut [u32]) {
        let a = q[self.aidx as usize];
        let nc = self.ncol as usize;
        if !self.float {
            let max = self.maxv;
            for v in q[..nc].iter_mut() {
                *v = if a == 0 {
                    0
                } else if *v >= a {
                    max as u32
                } else {
                    ((*v as u64 * max + (a >> 1) as u64) / a as u64) as u32
                };
            }
        } else {
            let af = f32::from_bits(a);
            for v in q[..nc].iter_mut() {
                *v = if af > 0.0 { (f32::from_bits(*v) / af).to_bits() } else { 0 }; // a <= 0 or NaN
            }
        }
    }

    // uraw (n pixels) -> BGRA pixels  [tc_conv]
    fn conv(&self, uraw: &mut [u32], dst: &mut [u8], n: usize) {
        let spp = self.spp as usize;
        let nk = self.nk;
        for (o, q) in dst.chunks_exact_mut(4).zip(uraw.chunks_mut(spp)).take(n) {
            if self.unpm {
                self.unpm(q);
            }
            let mut a = 255u32;
            if self.aidx >= 0 {
                a = self.nrm(q[self.aidx as usize], self.nka);
            }
            let mut c = match self.cv {
                CV_RGB => self.nrm(q[0], nk) << 16 | self.nrm(q[1], nk) << 8 | self.nrm(q[2], nk),
                CV_PAL => self.pal[(q[0] as u64 & self.maxv) as usize] & 0xFF_FFFF,
                CV_CMYK => {
                    let (c, m, y) = (self.nrm(q[0], nk), self.nrm(q[1], nk), self.nrm(q[2], nk));
                    let k = 255 - self.nrm(q[3], nk);
                    (255 - c) * k / 255 << 16 | (255 - m) * k / 255 << 8 | (255 - y) * k / 255
                }
                CV_YCC => self.ycc_px(self.nrm(q[0], nk), self.nrm(q[1], nk), self.nrm(q[2], nk)),
                CV_LAB => self.lab_px(q),
                _ => {
                    let mut g = self.nrm(q[0], nk);
                    if self.inv {
                        g ^= 255;
                    }
                    g * 0x010101
                }
            };
            if self.ofmt == FMT_PARGB && self.aidx >= 0 {
                // premultiplied output: colour <= alpha
                c = (c & 0xFF).min(a) | ((c >> 8) & 0xFF).min(a) << 8 | ((c >> 16) & 0xFF).min(a) << 16;
            }
            o.copy_from_slice(&(c | a << 24).to_le_bytes());
        }
    }

    // ============================================================== YCbCr / CIELab

    // value i of a RATIONAL entry as a float ((float)((double)num / den), 0 when den = 0)
    // [tc_ratf]
    fn ratf(&self, te: &Tent, i: u64) -> f32 {
        let (n, d) = self.tr.rational(te, i);
        if d == 0 {
            return 0.0;
        }
        ((n as i64 as f64) / (d as i64 as f64)) as f32
    }

    // the YCbCr tables like libtiff's TIFFYCbCrToRGBInit (YCbCrCoefficients, default
    // 0.299 / 0.587 / 0.114; ReferenceBlackWhite, default 0 255 128 255 128 255)
    // [tc_ycc_init]
    fn ycc_init(&mut self) {
        let mut luma = [0.299f32, 0.587, 0.114];
        if self.eycoef.count >= 3 {
            for (i, l) in luma.iter_mut().enumerate() {
                *l = self.ratf(&self.eycoef, i as u64);
            }
        }
        let mut rbw = [0.0f32, 255.0, 128.0, 255.0, 128.0, 255.0];
        if self.erbw.count >= 6 {
            for (i, r) in rbw.iter_mut().enumerate() {
                *r = self.ratf(&self.erbw, i as u64);
            }
        }
        // D1 = FIX(2 - 2 LR), D2 = -FIX(LR f1 / LG), D3 = FIX(2 - 2 LB), D4 = -FIX(LB f3 / LG)
        let (lr, lg, lb) = (luma[0], luma[1], luma[2]);
        let f1 = 2.0f32 - (lr + lr);
        let d1 = yfix(f1);
        let d2 = yfix(lr * f1 / lg).wrapping_neg();
        let f3 = 2.0f32 - (lb + lb);
        let d3 = yfix(f3);
        let d4 = yfix(lb * f3 / lg).wrapping_neg();
        // rb = (int)RB, den = RW - RB (1 when 0) for Y (rbw0/1), Cb (rbw2/3 - 128), Cr (rbw4/5 - 128)
        let mut rb = [0i32; 3];
        let mut den = [0f32; 3];
        for i in 0..3 {
            let (mut b, mut w) = (rbw[i * 2], rbw[i * 2 + 1]);
            if i > 0 {
                b -= 128.0;
                w -= 128.0;
            }
            rb[i] = rt::fx::cvtt(b);
            let d = w - b;
            den[i] = if d == 0.0 { 1.0 } else { d };
        }
        let mut t = alloc::boxed::Box::new(Ytab { crr: [0; 256], cbb: [0; 256], crg: [0; 256], cbg: [0; 256], y: [0; 256] });
        for i in 0..256i32 {
            let cr = code2v(i - 128, rb[2], den[2], 127.0);
            t.crr[i as usize] = cr.wrapping_mul(d1).wrapping_add(32768) >> 16;
            t.crg[i as usize] = cr.wrapping_mul(d2);
            let cb = code2v(i - 128, rb[1], den[1], 127.0);
            t.cbb[i as usize] = cb.wrapping_mul(d3).wrapping_add(32768) >> 16;
            t.cbg[i as usize] = cb.wrapping_mul(d4).wrapping_add(32768);
            t.y[i as usize] = code2v(i, rb[0], den[0], 255.0);
        }
        self.yt = Some(t);
    }

    // libtiff's TIFFYCbCrtoRGB -> 0x00RRGGBB  [tc_ycc_px]
    #[inline(always)]
    fn ycc_px(&self, y: u32, cb: u32, cr: u32) -> u32 {
        let t = self.yt.as_deref().expect("ycc tables");
        let (y, cb, cr) = ((y & 255) as usize, (cb & 255) as usize, (cr & 255) as usize);
        let yv = t.y[y];
        let r = yv.wrapping_add(t.crr[cr]);
        let g = (t.cbg[cb].wrapping_add(t.crg[cr]) >> 16).wrapping_add(yv);
        let b = yv.wrapping_add(t.cbb[cb]);
        (r.clamp(0, 255) as u32) << 16 | (g.clamp(0, 255) as u32) << 8 | b.clamp(0, 255) as u32
    }

    // CIELab (D50; L 0..100 from 8 / 16 bits, a / b signed 8 bits or 16 bits / 256) ->
    // XYZ -> Bradford D65 -> sRGB  [tc_lab_px]
    fn lab_px(&self, q: &[u32]) -> u32 {
        let mut l = q[0] as i32 as f32;
        let (a, b);
        if self.bps == 8 {
            l *= LAB_L8;
            a = q[1] as u8 as i8 as i32 as f32;
            b = q[2] as u8 as i8 as i32 as f32;
        } else {
            l *= LAB_L16;
            a = q[1] as u16 as i16 as i32 as f32 * LAB_1_256;
            b = q[2] as u16 as i16 as i32 as f32 * LAB_1_256;
        }
        let fy = (l + 16.0) * LAB_1_116;
        let fx = a * LAB_1_500 + fy;
        let fz = fy - b * LAB_1_200;
        let (x, y, z) = (lab_finv(fx), lab_finv(fy), lab_finv(fz));
        let mut lin = [0f32; 3];
        for (k, c) in lin.iter_mut().enumerate() {
            let m = &LAB_M[k * 3..k * 3 + 3];
            *c = m[0] * x + m[1] * y + m[2] * z;
        }
        let mut rgb = [0u8; 3];
        meta::row_f32_to_srgb8(&mut rgb, &lin);
        (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32
    }

    // YCbCr blocks (h x v Y, Cb, Cr) -> surface; chroma is replicated over the block
    // (libtiff TIFFRGBAImage)  [tc_ysub]
    fn ysub(&self, s: &Slot, x0: u32, y0: u32, cols: u32, nrows: u32, rows: &Rows) {
        let (bh, bv) = (self.ych, self.ycv);
        let hv = (bh * bv) as usize;
        let mut br = 0u32;
        while br * bv < nrows {
            let base = LIT + (self.brb * br as u64) as usize;
            for dy in 0..bv {
                let yy = br * bv + dy;
                if yy >= nrows {
                    break;
                }
                let mut g = rows[(y0 + yy) as usize].lock();
                let mut bc = 0u32;
                while bc * bh < cols {
                    let blk = base + (hv + 2) * bc as usize;
                    let (cb, cr) = (s.cbuf[blk + hv] as u32, s.cbuf[blk + hv + 1] as u32);
                    for dx in 0..bh {
                        let xx = bc * bh + dx;
                        if xx >= cols {
                            break;
                        }
                        let y = s.cbuf[blk + (dy * bh + dx) as usize] as u32;
                        let px = self.ycc_px(y, cb, cr) | 0xFF00_0000;
                        let at = ((x0 + xx) * 4) as usize;
                        g[at..at + 4].copy_from_slice(&px.to_le_bytes());
                    }
                    bc += 1;
                }
            }
            br += 1;
        }
    }

    // ============================================================== JPEG

    // decode a JPEG strip / tile with the JPEGTables (Photometric 6 -> YCbCr, 2 -> RGB,
    // 5 -> plain CMYK, 0 inverted grey) and copy it into the surface  [tc_jpeg]
    #[allow(clippy::too_many_arguments)]
    fn jpeg(&self, _s: &mut Slot, idx: u64, x0: u32, y0: u32, cols: u32, nrows: u32, rows: &Rows) -> i32 {
        let Some((off, n)) = self.span(idx) else {
            return ERR_TRUNC;
        };
        if n == 0 {
            return ERR_TRUNC;
        }
        let tables: &[u8] = if self.ejtab.bytes != 0 {
            let o = self.ejtab.voff as usize;
            &self.d[o..o + self.ejtab.bytes as usize]
        } else {
            &[]
        };
        let flags = match self.phot {
            6 => JDF_YCC,
            2 => JDF_RGB,
            5 => JDF_CMYK,
            _ => 0,
        };
        let jsf = match img_jpeg::decode_abbrev(tables, &self.d[off..off + n], flags) {
            Ok(sf) => sf,
            Err(e) => return e,
        };
        if (jsf.w as u32) < cols || (jsf.h as u32) < nrows {
            return ERR_HDR;
        }
        let inv = if self.phot == 0 { 0x00FF_FFFFu32 } else { 0 }; // MinIsWhite: invert
        let js = jsf.stride as usize;
        for r in 0..nrows as usize {
            let src = &jsf.px[r * js..r * js + cols as usize * 4];
            let mut g = rows[y0 as usize + r].lock();
            let dst = &mut g[(x0 * 4) as usize..((x0 + cols) * 4) as usize];
            for (o, q) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let v = u32::from_le_bytes([q[0], q[1], q[2], q[3]]) ^ inv | 0xFF00_0000;
                o.copy_from_slice(&v.to_le_bytes());
            }
        }
        0
    }

    // ============================================================== EXIF

    // meta.exif from the Exif (34665) / GPS (34853) IFDs of the page: a classic TIFF
    // structure in the byte order of the file with IFD0 = the descriptive tags of the page
    // (X0TAGS) + pointers, then copies of the Exif and GPS IFDs (entries with their values
    // relocated; IFD pointers inside them and BigTIFF-only types are dropped).  Unreadable /
    // cyclic sub-IFDs are left out, blocks over 16 MB are not built.  [tc_exif]
    fn exif(&mut self, meta: &mut ImgMeta) {
        let mut eo = 0u64;
        let mut go = 0u64;
        self.xep = 0;
        self.xgp = 0;
        if self.exififd != 0 && self.tr.ifd(self.exififd) >= 0 {
            eo = self.exififd; // visited (cycles, limit) + counted
        }
        if self.gpsifd != 0 && self.tr.ifd(self.gpsifd) >= 0 {
            go = self.gpsifd;
        }
        if eo | go == 0 {
            return;
        }
        // pointer entries present in IFD0 (positions filled in below)
        if eo != 0 {
            self.xep = 1;
        }
        if go != 0 {
            self.xgp = 1;
        }
        // sizes: IFD0 at 8, then the Exif IFD, then the GPS IFD
        let pe = self.xifd(self.ifd, 0, None, 0) + 8; // Exif IFD position
        let mut pg = pe; // GPS IFD position
        if eo != 0 {
            self.xep = pe as u32;
            pg += self.xifd(eo, 1, None, 0);
        }
        let mut total = pg;
        if go != 0 {
            self.xgp = pg as u32;
            total += self.xifd(go, 1, None, 0);
        }
        if total > 1 << 24 {
            return;
        }
        let mut buf = vec![0u8; total as usize + 16];
        if self.tr.be {
            buf[..8].copy_from_slice(&[0x4D, 0x4D, 0, 0x2A, 0, 0, 0, 8]); // MM\0*, IFD0 at 8
        } else {
            buf[..8].copy_from_slice(&[0x49, 0x49, 0x2A, 0, 8, 0, 0, 0]); // II*\0
        }
        self.xifd(self.ifd, 0, Some(&mut buf), 8);
        if eo != 0 {
            self.xifd(eo, 1, Some(&mut buf), pe);
        }
        if go != 0 {
            self.xifd(go, 1, Some(&mut buf), pg);
        }
        buf.truncate(total as usize);
        meta.exif = buf;
    }

    // the entry te is copied (mode 0: IFD0 tags of X0TAGS; mode 1: everything but IFD
    // pointers); classic types 1..12 only  [tc_xok]
    fn xok(te: &Tent, mode: u32) -> bool {
        if te.typ.wrapping_sub(1) > 11 || te.count > 0xFFFF_FFFF {
            return false;
        }
        if mode == 0 {
            X0TAGS.contains(&te.tag)
        } else {
            !matches!(te.tag, T_EXIF | T_GPS | 0xA005 | 0x014A)
        }
    }

    // store v (n = 2 / 4 bytes) at buf[p..] in the byte order of the file  [tc_xput]
    fn xput(&self, buf: &mut [u8], p: usize, v: u32, n: usize) {
        if n == 2 {
            let b = if self.tr.be { (v as u16).to_be_bytes() } else { (v as u16).to_le_bytes() };
            buf[p..p + 2].copy_from_slice(&b);
        } else {
            let b = if self.tr.be { v.to_be_bytes() } else { v.to_le_bytes() };
            buf[p..p + 4].copy_from_slice(&b);
        }
    }

    // a 12-byte IFD entry at buf[at..] with an inline u32 value  [tc_xent]
    fn xent(&self, buf: &mut [u8], at: usize, tag: u32, typ: u32, count: u32, value: u32) {
        self.xput(buf, at, tag, 2);
        self.xput(buf, at + 2, typ, 2);
        self.xput(buf, at + 4, count, 4);
        self.xput(buf, at + 8, value, 4);
    }

    // bytes of the rebuilt IFD (entries, next pointer, values); buf None: size only, else
    // written at buf[pos..].  Mode 0 adds the Exif / GPS pointer entries (xep / xgp:
    // position, 0 = none)  [tc_xifd]
    fn xifd(&self, ifd: u64, mode: u32, buf: Option<&mut [u8]>, pos: u64) -> u64 {
        let n = self.tr.count(ifd).max(0) as u64;
        let mut kept = 0u64;
        let mut vbytes = 0u64;
        // pass 1: kept entries and their out-of-line bytes
        for i in 0..n {
            let Ok(te) = self.tr.entry(ifd, i) else {
                continue;
            };
            if !Self::xok(&te, mode) {
                continue;
            }
            kept += 1;
            if te.bytes > 4 {
                vbytes += (te.bytes + 1) & !1;
            }
        }
        if mode == 0 {
            if self.xep != 0 {
                kept += 1;
            }
            if self.xgp != 0 {
                kept += 1;
            }
        }
        let size = kept * 12 + 6 + vbytes;
        let Some(buf) = buf else {
            return size;
        };
        // pass 2: write
        let pos = pos as usize;
        self.xput(buf, pos, kept as u32, 2);
        let mut ent = pos + 2; // next entry
        let mut val = pos + kept as usize * 12 + 6; // next value
        for i in 0..n {
            let Ok(te) = self.tr.entry(ifd, i) else {
                continue;
            };
            if !Self::xok(&te, mode) {
                continue;
            }
            let nb = te.bytes as usize;
            let v = if nb > 4 { val as u32 } else { 0 }; // inline value 0 (bytes copied below)
            self.xent(buf, ent, te.tag, te.typ, te.count as u32, v);
            let at = if nb > 4 {
                let a = val;
                val += (nb + 1) & !1;
                a
            } else {
                ent + 8 // inline value field
            };
            let o = te.voff as usize;
            buf[at..at + nb].copy_from_slice(&self.d[o..o + nb]);
            ent += 12;
        }
        if mode == 0 {
            if self.xep != 0 {
                self.xent(buf, ent, T_EXIF, 4, 1, self.xep);
                ent += 12;
            }
            if self.xgp != 0 {
                self.xent(buf, ent, T_GPS, 4, 1, self.xgp);
                ent += 12;
            }
        }
        buf[ent..ent + 4].fill(0); // next IFD: none
        size
    }
}

// 16.16 dots per inch (unit 2 inch, 3 cm; 0 unknown); rounded, clamped to 0xFFFFFFFF
// (the same rules as exif.s)  [tc_dpi]
fn dpi(num: u64, den: u64, unit: u32) -> u32 {
    let mut d = den & 0xFFFF_FFFF;
    if d == 0 {
        return 0;
    }
    let mut n = (num & 0xFFFF_FFFF) << 16;
    if unit == 3 {
        n *= 254;
        d *= 100;
    }
    ((n + (d >> 1)) / d).min(0xFFFF_FFFF) as u32
}

// FIX(clamp(f, 0, 2)) = (int)(f * 65536 + 0.5)  [tc_yfix]
fn yfix(f: f32) -> i32 {
    let f = rt::fx::minss(rt::fx::maxss(f, 0.0), 2.0);
    rt::fx::cvttd(f as f64 * 65536.0 + 0.5)
}

// (int)clamp((c - rb) * cr / den, -4096, 4096) -- libtiff's Code2V + CLAMPw  [tc_code2v]
fn code2v(c: i32, rb: i32, den: f32, cr: f32) -> i32 {
    let x = c.wrapping_sub(rb) as f32 * cr / den;
    rt::fx::cvtt(rt::fx::minss(rt::fx::maxss(x, -4096.0), 4096.0))
}

// CIE f^-1: t^3 above 6/29, else (t - 4/29) * 3 (6/29)^2  [LAB_FINV]
#[inline(always)]
fn lab_finv(t: f32) -> f32 {
    if t > LAB_6_29 { t * (t * t) } else { (t - LAB_4_29) * LAB_K }
}

// float24: 1 sign, 7 exponent (bias 63), 16 mantissa -> f32 bits  [tc_unpack .tu_f24]
fn f24_to_f32(v: u32) -> u32 {
    let sign = (v & 0x80_0000) << 8;
    let e = (v >> 16) & 0x7F;
    let m = v & 0xFFFF;
    if e == 0 {
        (m as i32 as f32 * f32::from_bits(0x1880_0000)).to_bits() | sign // denormal: m * 2^-78
    } else if e == 0x7F {
        (m << 7) | 0x7F80_0000 | sign
    } else {
        ((e + 127 - 63) << 23) | (m << 7) | sign
    }
}

// tc_rgb8 for S samples per pixel; MODE 0 opaque, 1 straight alpha (sample 3), 2
// associated alpha (colour <= alpha)
#[inline(always)]
fn rgb8_px<const S: usize, const MODE: u8>(src: &[u8], dst: &mut [u8], n: usize) {
    for (o, q) in dst[..n * 4].chunks_exact_mut(4).zip(src[..n * S].chunks_exact(S)) {
        let q: &[u8; S] = q.try_into().unwrap();
        let px = match MODE {
            0 => [q[2], q[1], q[0], 255],
            1 => [q[2], q[1], q[0], q[3]],
            _ => {
                let a = q[3];
                [q[2].min(a), q[1].min(a), q[0].min(a), a]
            }
        };
        o.copy_from_slice(&px);
    }
}

// a big-endian u64 at buf[at..at + 8] (the chunk buffers have slack behind the data)
#[inline(always)]
fn rd64be(buf: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[at..at + 8]);
    u64::from_be_bytes(b)
}

// ================================================================== decompressors

/// PackBits until the output (cap bytes) is full or the input ends -> bytes written
/// [tc_packbits]
pub(crate) fn packbits(src: &[u8], dst: &mut [u8], cap: usize) -> i64 {
    let (n, mut i, mut o) = (src.len(), 0usize, 0usize);
    while o < cap && i < n {
        let c = src[i] as i8 as i32;
        i += 1;
        if c >= 0 {
            let take = ((c + 1) as usize).min(n - i); // literal: c + 1 bytes
            let m = take.min(cap - o);
            dst[o..o + m].copy_from_slice(&src[i..i + m]);
            o += m;
            i += take;
        } else if c != -128 {
            if i >= n {
                break;
            }
            let b = src[i];
            i += 1;
            let m = ((1 - c) as usize).min(cap - o); // 1 - c copies
            dst[o..o + m].fill(b);
            o += m;
        }
    }
    o as i64
}
