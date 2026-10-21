// img_core -- the shared part of the image codecs: records of img.inc (decode options,
// metadata, encode options, codec descriptor, format table), error codes, and the modules
// every codec builds on                                              [ASMLIB/inc/img.inc]
//
//   ImgDec        decode options (IMGDEC): flags IMGD_*, frame, hint, pixel budget, cancel
//   ImgMeta       decoded metadata (IMGMETA); to_bytes() = the 128-byte record of the
//                 assembly with the blob pointers zeroed (codectest op 261 compares it),
//                 incl. IM_CHRM (PNG cHRM) and the DDS fields IM_DXGI / IM_TEXFL / IM_TEXN
//   ImgOpt        encode options (IMGOPT); from_bytes() reads the 64-byte record
//   Codec         a codec descriptor (IMGCODEC): probe / decode / encode, caps, iom; and
//                 for the core formats the functions the assembly calls directly: scan
//                 (png_meta / jpeg_meta / ...), decode_fl (png_decode_fl / ...: the classic
//                 decoders of img_decode and BMP's embedded PNG / JPEG), save (png_encode /
//                 jpeg_encode / ...: the classic encoders of img_save); Codec::EMPTY for
//                 struct update syntax
//   FmtInfo       a format table entry (IMGFMT)
//
// Modules (owners during the R6 wave, see rewrite/AGENTS.md):
//   checksum, inflate, deflate, gzip, meta (imgmeta.s), exif, icc, io (imgio.s, imgreg.s,
//   imgcore.s glue), bits (the bit readers of img_int.inc)  -- R6-core
//   yuv                                                     -- R6-jpeg
//   quantize                                                -- R6-bgt (bmp / gif / tga)
//   resample, xform, mip, ops (imgops.s)                    -- R6-xf
//   icc_xform (+ icc_eval, icc_bi: icc_xform.s, icc.inc)    -- R6-icc
//
// Decoded images are gfx::Surface values (FMT_PARGB, or FMT_ARGB with IMGD_STRAIGHT).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

pub use gfx::Surface;

pub mod bits;
pub mod checksum;
pub mod deflate;
pub mod exif;
pub mod gzip;
pub mod icc;
mod icc_bi; // R6-icc
mod icc_eval; // R6-icc
pub mod icc_xform; // R6-icc
pub mod inflate;
pub mod io;
pub mod meta;
pub mod mip;
pub mod ops;
pub mod quantize;
pub mod resample;
pub mod xform;
pub mod yuv;

// ------------------------------------------------------------------ the worker pool (main agent)

/// runs f(i) for every i in 0..n, possibly on several threads (thr_parallel_for)
pub type ParallelFor = fn(usize, &(dyn Fn(usize) + Sync));

static PAR: rt::sync::SpinLock<Option<ParallelFor>> = rt::sync::SpinLock::new(None);

/// the loop the codecs' parallel parts run on (the asm's thr_parallel_for: ICC LUT grids, YUV
/// bands); the apps pass a wrapper of plat::parallel_for -- img_core does not depend on plat.
/// Without it everything runs on the calling thread (the same bytes)
pub fn set_parallel(f: ParallelFor) {
    *PAR.lock() = Some(f);
}

/// the loop set by set_parallel
pub fn parallel() -> Option<ParallelFor> {
    *PAR.lock()
}

// ------------------------------------------------------------------ errors (jfgui.inc)
pub const ERR_FAIL: i32 = -1;
pub const ERR_TRUNC: i32 = -1; // input ends early
pub const ERR_HDR: i32 = -2; // bad header / signature
pub const ERR_BLOCK: i32 = -3; // bad deflate block or PNG filter
pub const ERR_CODE: i32 = -4; // bad Huffman code
pub const ERR_DIST: i32 = -5; // distance too far back / invalid
pub const ERR_OVERFLOW: i32 = -6; // output larger than the buffer
pub const ERR_CHECKSUM: i32 = -7; // Adler-32 mismatch
pub const ERR_NOMEM: i32 = -8; // allocation failed
pub const ERR_UNSUPPORTED: i32 = -9; // valid but unsupported variant
pub const ERR_CRC: i32 = -10; // PNG chunk CRC mismatch
pub const ERR_CHUNK: i32 = -11; // bad / missing PNG chunk
pub const ERR_DIMS: i32 = -12; // dimensions out of range
pub const ERR_IO: i32 = -13; // file could not be read / written
pub const ERR_CANCEL: i32 = -14; // cancelled through a cancel flag
pub const ERR_LIMIT: i32 = -15; // resource limit

// ------------------------------------------------------------------ formats (jfgui.inc IMGF_*)
pub const IMGF_PNG: u32 = 1;
pub const IMGF_JPEG: u32 = 2;
pub const IMGF_BMP: u32 = 3;
pub const IMGF_GIF: u32 = 4;
pub const IMGF_TIFF: u32 = 5;
pub const IMGF_TGA: u32 = 6;
pub const IMGF_DDS: u32 = 7;
pub const IMGF_WEBP: u32 = 8;
pub const IMGF_AVIF: u32 = 9;
pub const IMGF_HEIC: u32 = 10;
pub const IMGF_JXL: u32 = 11;
pub const IMGF_JXR: u32 = 12;
pub const IMGF_PDN: u32 = 13;
pub const IMG_DYN0: u32 = 14; // runtime formats (plugins), start inclusive
pub const IMG_DYN_END: u32 = 22; // runtime formats, end exclusive
// Keep the eight established plugin slots (14 through 21) stable; new built-ins follow them.
pub const IMGF_PSD: u32 = 22;
pub const IMG_EXTERNAL0: u32 = 23; // out-of-process codecs; native PNAPI IDs stay stable
pub const IMGF_COUNT: u32 = 55;
/// (svg) SVG: the last slot, outside the static table until img_svg::register fills it
/// (register_external stops before it)
pub const IMGF_SVG: u32 = IMGF_COUNT - 1;

// ------------------------------------------------------------------ limits
pub const IMG_MAXDIM: u32 = 65535; // largest width / height (ERR_DIMS)
pub const IMG_MAXPX_DEF: u64 = 1 << 28; // default pixel budget (ERR_LIMIT)

// ------------------------------------------------------------------ decode flags (ImgDec::flags)
pub const IMGD_STRAIGHT: u32 = 1; // FMT_ARGB instead of FMT_PARGB
pub const IMGD_ICC: u32 = 2; // copy an embedded ICC profile into ImgMeta::icc
pub const IMGD_EXIF: u32 = 4; // copy the EXIF block (TIFF structure)
pub const IMGD_XMP: u32 = 8; // copy the XMP packet
pub const IMGD_NO_ORIENT: u32 = 16; // do not apply the EXIF / container orientation
pub const IMGD_PROBE: u32 = 32; // header + metadata only
pub const IMGD_THUMB: u32 = 64; // an embedded preview / reduced size is acceptable

// ------------------------------------------------------------------ metadata (ImgMeta)
pub const IMM_GRAY: u8 = 1; // colour models
pub const IMM_RGB: u8 = 2;
pub const IMM_PAL: u8 = 3;
pub const IMM_CMYK: u8 = 4;
pub const IMM_YCC: u8 = 5;
pub const IMM_LAB: u8 = 6;
pub const IMM_YCCK: u8 = 7;

pub const IMF_ALPHA: u32 = 1; // flags
pub const IMF_SRC_PREMUL: u32 = 2;
pub const IMF_LOSSLESS: u32 = 4;
pub const IMF_ANIMATED: u32 = 8;
pub const IMF_HDR: u32 = 16;
pub const IMF_ORIENT_APPLIED: u32 = 32;
pub const IMF_FLOAT: u32 = 64;
pub const IMF_INTERLACED: u32 = 128;
pub const IMF_SRGB: u32 = 256;

pub const IMR_NONE: u32 = 0; // resolution units
pub const IMR_INCH: u32 = 1;
pub const IMR_CM: u32 = 2;

// ------------------------------------------------------------------ encode options (ImgOpt)
pub const IOF_LOSSLESS: u32 = 1;
pub const IOF_RLE: u32 = 2;
pub const IOF_MIPMAPS: u32 = 4;
pub const IOF_SRGB: u32 = 8;
pub const IOF_CUBE_CROSS: u32 = 16;
pub const IOF_EMBED_ICC: u32 = 32;
pub const IOF_EMBED_EXIF: u32 = 64;
pub const IOF_EMBED_XMP: u32 = 128;
pub const IOF_INTERLACE: u32 = 256;

pub const DDSF_BC1: u32 = 1;
pub const DDSF_BC2: u32 = 2;
pub const DDSF_BC3: u32 = 3;
pub const DDSF_BC4: u32 = 4;
pub const DDSF_BC5: u32 = 5;
pub const DDSF_BC6H: u32 = 6;
pub const DDSF_BC7: u32 = 7;
pub const DDSF_BGRA8: u32 = 8;
pub const DDSF_BGRX8: u32 = 9;
pub const DDSF_RGBA8: u32 = 10;
pub const DDSF_BGR5A1: u32 = 11;
pub const DDSF_BGRA4: u32 = 12;
pub const DDSF_B5G6R5: u32 = 13;
pub const DDSF_L8: u32 = 14;
pub const DDSF_A8: u32 = 15;
pub const DDSF_R8G8: u32 = 16;

pub const MIPF_BOX: u32 = 1;
pub const MIPF_LINEAR: u32 = 2;
pub const MIPF_CUBIC: u32 = 3;
pub const MIPF_NEAREST: u32 = 4;

pub const TIFC_NONE: u32 = 1;
pub const TIFC_LZW: u32 = 2;
pub const TIFC_DEFLATE: u32 = 3;
pub const TIFC_PACKBITS: u32 = 4;

pub const IOM_QUALITY: u32 = 1; // which ImgOpt fields an encoder honours
pub const IOM_LOSSLESS: u32 = 2;
pub const IOM_RLE: u32 = 4;
pub const IOM_MIPMAPS: u32 = 8;
pub const IOM_BITDEPTH: u32 = 16;
pub const IOM_SAMPLEBITS: u32 = 32;
pub const IOM_DITHER: u32 = 64;
pub const IOM_ATHRESH: u32 = 128;
pub const IOM_CHROMA: u32 = 256;
pub const IOM_EFFORT: u32 = 512;
pub const IOM_DDSFMT: u32 = 1024;
pub const IOM_MIPFILTER: u32 = 2048;
pub const IOM_TIFFCOMP: u32 = 4096;
pub const IOM_SRGB: u32 = 8192;
pub const IOM_CUBE: u32 = 16384;
pub const IOM_ICC: u32 = 32768;
pub const IOM_EXIF: u32 = 65536;
pub const IOM_XMP: u32 = 131072;
pub const IOM_INTERLACE: u32 = 262144;

// ------------------------------------------------------------------ format table (FmtInfo::caps)
pub const IFC_DECODE: u32 = 1;
pub const IFC_ENCODE: u32 = 2;
pub const IFC_ALPHA: u32 = 4;
pub const IFC_LAYERS: u32 = 8;
pub const IFC_ANIM: u32 = 16;
pub const IFC_PAGES: u32 = 32;
pub const IFC_MIPS: u32 = 64;
pub const IFC_LOSSY: u32 = 128;
pub const IFC_LOSSLESS: u32 = 256;
pub const IFC_HDR: u32 = 512;
pub const IFC_ICC: u32 = 1024;
pub const IFC_EXIF: u32 = 2048;
pub const IFC_XMP: u32 = 4096;
pub const IFC_CORE: u32 = 8192;

/// decode options (IMGDEC); Default = premultiplied output, orientation applied, frame 0,
/// no hint, IMG_MAXPX_DEF pixels, no metadata blobs, no cancel flag
#[derive(Clone, Copy, Default)]
pub struct ImgDec<'a> {
    pub flags: u32,  // IMGD_*
    pub frame: u32,  // frame / page / mip level
    pub hint: u32,   // IMGF_* expected from the file name (0 = none)
    pub maxpx: u64,  // max w * h (0 = IMG_MAXPX_DEF) -> ERR_LIMIT
    pub maxmem: u64, // memory budget for codec scratch (0 = no limit)
    pub cancel: Option<&'a AtomicU32>,
}

impl ImgDec<'_> {
    /// the pixel budget (maxpx, or the default)
    pub fn max_pixels(&self) -> u64 {
        if self.maxpx == 0 { IMG_MAXPX_DEF } else { self.maxpx }
    }
    /// the cancel flag is set
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }
}

/// decoded metadata (IMGMETA)
#[derive(Clone, Default, Debug, PartialEq)]
pub struct ImgMeta {
    pub fmt: u32,      // IMGF_*
    pub w: u32,        // stored width (before orientation)
    pub h: u32,        // stored height
    pub bits: u8,      // bits per sample of the source (1..16, 32 = float)
    pub model: u8,     // IMM_*
    pub orient: u8,    // EXIF orientation 1..8
    pub channels: u8,  // samples per pixel incl. alpha (0 = unknown)
    pub flags: u32,    // IMF_*
    pub frames: u32,   // frames / pages / mip levels
    pub dpix: u32,     // dots per inch 16.16 (0 = unknown)
    pub dpiy: u32,
    pub cicp: u32,     // primaries | transfer << 8 | matrix << 16 | full range << 24
    pub gamma: u32,    // file gamma * 100000 (PNG gAMA)
    pub icc: Vec<u8>,  // IMGD_ICC
    pub exif: Vec<u8>, // IMGD_EXIF: TIFF structure without "Exif\0\0"
    pub xmp: Vec<u8>,  // IMGD_XMP
    pub delay: u32,    // ms (animations)
    pub loops: u32,    // animation loop count (0 = forever)
    pub quality: u32,  // estimated encoder quality 1..100
    pub resunit: u32,  // IMR_*
    pub chrm: [u16; 8], // IM_CHRM: PNG cHRM white x, y, red x, y, green x, y, blue x, y / 50000
    pub dxgi: u16,     // IM_DXGI: DXGI_FORMAT of DDS pixel data
    pub texfl: u8,     // IM_TEXFL: TXF_*
    pub texn: u32,     // IM_TEXN: DDS array size / cubes / depth
}

impl ImgMeta {
    /// the 128-byte IMGMETA record of the assembly (blob pointers 0, lengths kept)
    pub fn to_bytes(&self) -> [u8; 128] {
        let mut b = [0u8; 128];
        let mut put = |o: usize, v: &[u8]| b[o..o + v.len()].copy_from_slice(v);
        put(0, &self.fmt.to_le_bytes());
        put(4, &self.w.to_le_bytes());
        put(8, &self.h.to_le_bytes());
        put(12, &[self.bits, self.model, self.orient, self.channels]);
        put(16, &self.flags.to_le_bytes());
        put(20, &self.frames.to_le_bytes());
        put(24, &self.dpix.to_le_bytes());
        put(28, &self.dpiy.to_le_bytes());
        put(32, &self.cicp.to_le_bytes());
        put(36, &self.gamma.to_le_bytes());
        put(48, &(self.icc.len() as u64).to_le_bytes());
        put(64, &(self.exif.len() as u64).to_le_bytes());
        put(80, &(self.xmp.len() as u64).to_le_bytes());
        put(88, &self.delay.to_le_bytes());
        put(92, &self.loops.to_le_bytes());
        put(96, &self.quality.to_le_bytes());
        put(100, &self.resunit.to_le_bytes());
        for (k, v) in self.chrm.iter().enumerate() {
            put(104 + k * 2, &v.to_le_bytes());
        }
        put(120, &self.dxgi.to_le_bytes());
        put(122, &[self.texfl]);
        put(124, &self.texn.to_le_bytes());
        b
    }
}

/// encode options (IMGOPT); Default = the format's defaults
#[derive(Clone, Copy, Default)]
pub struct ImgOpt<'a> {
    pub fmt: u32,        // IMGF_* (save: 0 = from the file extension)
    pub quality: u32,    // 1..100 (0 = format default)
    pub flags: u32,      // IOF_*
    pub bitdepth: u32,   // bits per pixel 0 auto / 32 / 24 / 8
    pub samplebits: u32, // bits per sample 8 / 10 / 12 (0 = 8)
    pub dither: u32,     // dithering level + 1 (0 = default 7, 1 = off .. 9 = level 8)
    pub athresh: u32,    // transparency threshold + 1 (0 = default 128)
    pub chroma: u32,     // 444 / 422 / 420 / 400 (0 = format default)
    pub effort: u32,     // 1..9 (0 = default); PNG: deflate level
    pub ddsfmt: u32,     // DDSF_*
    pub mipfilter: u32,  // MIPF_*
    pub tiffcomp: u32,   // TIFC_*
    pub meta: Option<&'a ImgMeta>, // metadata to embed
    pub cancel: Option<&'a AtomicU32>,
}

impl<'a> ImgOpt<'a> {
    /// the scalar fields of a 64-byte IMGOPT record (IO_META / IO_CANCEL ignored)
    pub fn from_bytes(b: &[u8]) -> ImgOpt<'a> {
        let u = |o: usize| if o + 4 <= b.len() { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) } else { 0 };
        ImgOpt {
            fmt: u(0),
            quality: u(4),
            flags: u(8),
            bitdepth: u(12),
            samplebits: u(16),
            dither: u(20),
            athresh: u(24),
            chroma: u(28),
            effort: u(32),
            ddsfmt: u(36),
            mipfilter: u(40),
            tiffcomp: u(44),
            meta: None,
            cancel: None,
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }
}

/// probe(data, meta) -> recognised (header + metadata scan, no pixels)
pub type ProbeFn = fn(&[u8], &mut ImgMeta) -> bool;
/// decode(data, out, dec, meta) -> 0 / ERR_*  (meta: the caller's ImgMeta, already
/// cleared with fmt set; dec.flags tell which blobs to copy)
pub type DecodeFn = fn(&[u8], &mut Surface, &ImgDec, &mut ImgMeta) -> i32;
/// encode(surface, opt, out) -> 0 / ERR_*  (FMT_XRGB / PARGB / ARGB / A8 input)
pub type EncodeFn = fn(&Surface, &ImgOpt, &mut Vec<u8>) -> i32;
/// scan(data, meta, flags) -> 0 / ERR_*: the header / metadata scanner of a core format
/// (png_meta, jpeg_meta, bmp_meta, gif_meta; flags = IMGD_* of the blobs to copy)
pub type ScanFn = fn(&[u8], &mut ImgMeta, u32) -> i32;
/// decode_fl(data, out, jdf) -> 0 / ERR_*: the classic decoder (png_decode_fl, ...);
/// jdf = JDF_STRAIGHT (FMT_ARGB) | JDF_NO_ORIENT, else FMT_PARGB
pub type DecodeFlFn = fn(&[u8], &mut Surface, u32) -> i32;
/// save(surface, quality) -> file bytes or None (out of memory, unsupported surface): the
/// classic encoder of img_save (png_encode, jpeg_encode(quality), bmp_encode, gif_encode)
pub type SaveFn = fn(&Surface, u32) -> Option<Vec<u8>>;

/// a codec descriptor (IMGCODEC); every codec crate exports one `pub static CODEC: Codec`
/// (`Codec { probe: Some(..), decode: Some(..), ..Codec::EMPTY }`)
pub struct Codec {
    pub probe: Option<ProbeFn>,
    pub decode: Option<DecodeFn>,
    pub encode: Option<EncodeFn>,
    pub caps: u32, // IFC_DECODE / IFC_ENCODE really implemented
    pub iom: u32,  // IOM_* the encoder honours
    pub scan: Option<ScanFn>,          // core formats: metadata scanner
    pub decode_fl: Option<DecodeFlFn>, // core formats: classic decoder (img_decode)
    pub save: Option<SaveFn>,          // core formats: classic encoder (img_save)
}

impl Codec {
    pub const EMPTY: Codec =
        Codec { probe: None, decode: None, encode: None, caps: 0, iom: 0, scan: None, decode_fl: None, save: None };
}

// ------------------------------------------------------------------ core codec options (img.inc CA)
pub const JDF_STRAIGHT: u32 = 1; // FMT_ARGB output
pub const JDF_NO_ORIENT: u32 = 2; // keep the stored orientation
pub const JDF_RGB: u32 = 4; // 3 components are R, G, B (TIFF Photometric 2)
pub const JDF_YCC: u32 = 8; // 3 components are YCbCr (Photometric 6)
pub const JDF_CMYK: u32 = 16; // 4 components are plain CMYK (Photometric 5)
pub const PNG_IOM: u32 = IOM_BITDEPTH | IOM_DITHER | IOM_ATHRESH | IOM_EFFORT | IOM_ICC | IOM_EXIF | IOM_XMP;
pub const JPEG_IOM: u32 = IOM_QUALITY | IOM_CHROMA | IOM_ICC | IOM_EXIF | IOM_XMP;
pub const BMP_IOM: u32 = IOM_BITDEPTH | IOM_DITHER;
pub const GIF_IOM: u32 = IOM_DITHER | IOM_ATHRESH;
pub const TXF_CUBE: u8 = 1; // ImgMeta::texfl (DDS)
pub const TXF_VOLUME: u8 = 2;
pub const TXF_ARRAY: u8 = 4;
pub const TXF_CROSS: u8 = 8;
pub const TXF_PREMUL: u8 = 16;
pub const TXF_LEGACY: u8 = 32;

/// a format table entry (IMGFMT)
#[derive(Clone, Copy)]
pub struct FmtInfo {
    pub name: &'static str,
    pub exts: &'static str, // ui file dialog entry "png=PNG" / "jpg;jpeg;...=JPEG"
    pub mime: &'static str,
    pub caps: u32,          // IFC_* format properties | registered DECODE / ENCODE
    pub iom: u32,           // IOM_* of the registered encoder
    pub codec: Option<&'static Codec>,
}
