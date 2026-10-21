// io -- image files: format detection, codec dispatch, load and save; the format table and
// the glue of the core codecs                    [src/img/imgio.s, imgreg.s, imgcore.s glue]
//
// Classic API (FMT_PARGB, PNG / JPEG / BMP / GIF only, through the codecs' decode_fl / save):
//   decode(data) -> Ok(surface) / Err(ERR_*)     PNG, JPEG, GIF or BMP by magic bytes (and SVG
//                                                once img_svg::register entered IMGF_SVG)
//   load(path) -> Ok(surface) / Err(ERR_*)
//   save(path, sf) / save_ex(path, sf, quality) -> 0 / ERR_*   by extension (.png .jpg .jpeg
//                                                .jfif .jpe .bmp .dib .gif, any case)
//   format(path) -> IMGF_* of those extensions (0 = unknown)
// Extended API:
//   detect(data, hint) -> IMGF_* / 0             magic bytes (TGA: hint or its probe)
//   probe(data, meta) -> IMGF_* / 0              detect + header scan into meta
//   decode_ex(data, dec, meta) -> Ok(surface) / Err(ERR_*)
//   load_ex(path, dec, meta) -> Ok(surface) / Err(ERR_*)
//   encode(sf, opt) -> Ok(file bytes) / Err(ERR_*)
//   save_opt(path, sf, opt) -> 0 / ERR_*         atomic write (the file hook)
//   opt_init(fmt) -> ImgOpt                      paint.net's defaults
//   format_info(fmt) -> Option<FmtInfo>          format_from_path(path) -> IMGF_* / 0
//   filter_list(need, cap) -> the ui_filedlg list "png=PNG\0jpg;...=JPEG\0...\0" (empty when
//                                                it does not fit into cap bytes)
//   register(fmt, codec) -> 0 / ERR_UNSUPPORTED  register_dyn(name, exts, mime, props, codec)
//   register_slot(IMGF_SVG, name, exts, mime, props, codec)   (svg) a built-in format without
//                                                a static entry (img_svg::register)
//   set_file_io(FileIo)                          the file functions of load / save (img_core
//                                                does not depend on plat: the app sets them)
// Glue for the core codecs (imgcore.s):
//   core_decode(data, sf, dec, meta, scan, decode_fl) -> 0 / ERR_*   scanner, frame / size
//                                                checks, then the classic decoder
//   core_probe(data, meta, scan) -> bool
//   dpi_ppm(dpi) / ppm_dpi(ppm)                  16.16 dpi <-> pixels per metre
//
// The format table has a static entry per built-in format (name, extensions, MIME,
// properties); a codec crate's `CODEC` gets into it through register (img_all::
// use_core_codecs / use_all_codecs).  Runtime formats (plugins) use the slots IMG_DYN0 ..
// IMGF_COUNT - 1.
//
// decode_ex: the codec gets a normalised copy of dec (maxpx filled in; ICC / EXIF / XMP
// dropped when the caller passed no ImgMeta); the ImgMeta is cleared first and gets fmt,
// orient 1, frames 1.  After a successful decode the orientation is applied (unless
// IMGD_NO_ORIENT or the codec did it: IMF_ORIENT_APPLIED) and the pixels are converted to
// FMT_ARGB (IMGD_STRAIGHT) or FMT_PARGB.  On an error the blobs are dropped (the other
// metadata fields stay).

use alloc::vec::Vec;

use crate::meta::{convert, dims_check, meta_free, orient};
use crate::*;
use gfx::{FMT_A8, FMT_ARGB, FMT_PARGB};
use rt::sync::SpinLock;

const IMG_JPEG_QUALITY: u32 = 90;
const BOX_MAX: u32 = 64; // ISOBMFF boxes looked at per level

// ================================================================== format table

const fn fi(name: &'static str, exts: &'static str, mime: &'static str, caps: u32) -> FmtInfo {
    FmtInfo { name, exts, mime, caps, iom: 0, codec: None }
}

const FREE: FmtInfo = fi("", "", "", 0);

// format properties (IFC_* without DECODE / ENCODE), by IMGF
static PROPS: [u32; 14] = [
    0,
    IFC_CORE | IFC_ALPHA | IFC_LOSSLESS | IFC_HDR | IFC_ANIM | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_CORE | IFC_LOSSY | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_CORE | IFC_ALPHA | IFC_LOSSLESS | IFC_ICC,
    IFC_CORE | IFC_ALPHA | IFC_LOSSLESS | IFC_ANIM | IFC_ICC,
    IFC_ALPHA | IFC_LOSSLESS | IFC_LOSSY | IFC_PAGES | IFC_HDR | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LOSSLESS,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_MIPS | IFC_HDR,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_ANIM | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_HDR | IFC_ANIM | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_HDR | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_HDR | IFC_ANIM | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LOSSY | IFC_LOSSLESS | IFC_HDR | IFC_ICC | IFC_EXIF | IFC_XMP,
    IFC_ALPHA | IFC_LAYERS | IFC_LOSSLESS,
];

static FMTS: SpinLock<[FmtInfo; IMGF_COUNT as usize]> = SpinLock::new([
    FREE,
    fi("PNG", "png=PNG", "image/png", PROPS[1]),
    fi("JPEG", "jpg;jpeg;jpe;jfif;exif=JPEG", "image/jpeg", PROPS[2]),
    fi("BMP", "bmp;dib;rle=BMP", "image/bmp", PROPS[3]),
    fi("GIF", "gif=GIF", "image/gif", PROPS[4]),
    fi("TIFF", "tif;tiff=TIFF", "image/tiff", PROPS[5]),
    fi("TGA", "tga=TGA", "image/x-tga", PROPS[6]),
    fi("DDS", "dds=DDS", "image/vnd-ms.dds", PROPS[7]),
    fi("WebP", "webp=WebP", "image/webp", PROPS[8]),
    fi("AVIF", "avif=AVIF", "image/avif", PROPS[9]),
    // (the HEIC entry of the static table has no IFC_LOSSLESS, unlike its PROPS row)
    fi("HEIC", "heic;heif=HEIC", "image/heic", IFC_ALPHA | IFC_LOSSY | IFC_HDR | IFC_ICC | IFC_EXIF | IFC_XMP),
    fi("JPEG XL", "jxl=JPEG XL", "image/jxl", PROPS[11]),
    fi("JPEG XR", "jxr;wdp;hdp;wmp=JPEG XR", "image/jxr", PROPS[12]),
    fi("Paint.NET", "pdn=Paint.NET", "image/x-PaintCraft", PROPS[13]),
    FREE,
    FREE,
    FREE,
    FREE,
    FREE,
    FREE,
    FREE,
    FREE,
    fi("Photoshop PSD", "psd=Photoshop PSD", "image/vnd.adobe.photoshop", IFC_ALPHA | IFC_LOSSLESS | IFC_ICC | IFC_EXIF | IFC_XMP),
    FREE, FREE, FREE, FREE, FREE, FREE, FREE, FREE,
    FREE, FREE, FREE, FREE, FREE, FREE, FREE, FREE,
    FREE, FREE, FREE, FREE, FREE, FREE, FREE, FREE,
    FREE, FREE, FREE, FREE, FREE, FREE, FREE, FREE,
]);

/// the format table entry of fmt (None for 0, out of range or a free runtime slot)
/// [img_format_info]
pub fn format_info(fmt: u32) -> Option<FmtInfo> {
    if fmt == 0 || fmt >= IMGF_COUNT {
        return None;
    }
    let f = FMTS.lock()[fmt as usize];
    if f.name.is_empty() { None } else { Some(f) }
}

/// the registered codec of fmt  [img_codec]
pub fn codec(fmt: u32) -> Option<&'static Codec> {
    if fmt == 0 || fmt >= IMGF_COUNT {
        return None;
    }
    FMTS.lock()[fmt as usize].codec
}

/// put a codec descriptor (or None to remove one) into the format table: caps = the
/// format's properties | the codec's caps, iom = the codec's iom  [img_register]
pub fn register(fmt: u32, c: Option<&'static Codec>) -> i32 {
    if fmt == 0 || fmt >= IMGF_COUNT {
        return ERR_UNSUPPORTED;
    }
    let props = if fmt==IMGF_PSD {IFC_ALPHA | IFC_LOSSLESS | IFC_ICC | IFC_EXIF | IFC_XMP} else {PROPS.get(fmt as usize).copied().unwrap_or(0)};
    let mut t = FMTS.lock();
    let e = &mut t[fmt as usize];
    e.codec = c;
    e.caps = props | c.map_or(0, |c| c.caps);
    e.iom = c.map_or(0, |c| c.iom);
    0
}

/// a new runtime format (plugins): name "QOI", exts "qoi=QOI", mime, props (IFC_* format
/// properties; DECODE / ENCODE come from the codec) -> its IMGF (IMG_DYN0..) or 0 when every
/// slot is taken or name / exts is empty  [img_register_dyn]
pub fn register_dyn(name: &'static str, exts: &'static str, mime: &'static str, props: u32, c: &'static Codec) -> u32 {
    register_range(IMG_DYN0..IMG_DYN_END, name, exts, mime, props, c)
}

/// Separate slots for external codec hosts, preserving the native plugin ABI's IDs (the
/// last slot, IMGF_SVG, is kept for SVG).
pub fn register_external(name: &'static str, exts: &'static str, props: u32, c: &'static Codec) -> u32 {
    register_range(IMG_EXTERNAL0..IMGF_SVG, name, exts, "application/octet-stream", props, c)
}

/// (svg) a built-in format that has no entry in the static table (IMGF_SVG): its name,
/// extension list, MIME type, properties and codec into slot fmt (None empties the slot)
/// -> 0 / ERR_UNSUPPORTED (fmt out of range, or a static / runtime format's slot)
pub fn register_slot(fmt: u32, name: &'static str, exts: &'static str, mime: &'static str, props: u32, c: Option<&'static Codec>) -> i32 {
    if fmt != IMGF_SVG {
        return ERR_UNSUPPORTED;
    }
    let mut t = FMTS.lock();
    t[fmt as usize] = match c {
        Some(c) => FmtInfo { name, exts, mime, caps: (c.caps & (IFC_DECODE | IFC_ENCODE)) | (props & !(IFC_DECODE | IFC_ENCODE | IFC_CORE)), iom: c.iom, codec: Some(c) },
        None => FREE,
    };
    0
}
fn register_range(range: core::ops::Range<u32>, name: &'static str, exts: &'static str, mime: &'static str, props: u32, c: &'static Codec) -> u32 {
    if name.is_empty() || exts.is_empty() {
        return 0;
    }
    let mut t = FMTS.lock();
    for k in range {
        let e = &mut t[k as usize];
        if e.name.is_empty() {
            *e = FmtInfo {
                name,
                exts,
                mime,
                caps: (c.caps & (IFC_DECODE | IFC_ENCODE)) | (props & !(IFC_DECODE | IFC_ENCODE | IFC_CORE)),
                iom: c.iom,
                codec: Some(c),
            };
            return k;
        }
    }
    0
}

/// a ui_filedlg extension list "png=PNG\0jpg;jpeg;...=JPEG\0...\0" of every format whose
/// caps contain all bits of need; empty when it does not fit into cap bytes (incl. the
/// final empty entry)  [img_filter_list]
pub fn filter_list(need: u32, cap: usize) -> Vec<u8> {
    let t = *FMTS.lock();
    let mut out = Vec::new();
    for f in &t[1..] {
        if f.caps & need != need || f.exts.is_empty() {
            continue;
        }
        let n = f.exts.len() + 1;
        if out.len() + n + 1 > cap {
            return Vec::new();
        }
        out.extend_from_slice(f.exts.as_bytes());
        out.push(0);
    }
    if out.len() >= cap {
        return Vec::new();
    }
    out.push(0);
    out
}

/// an ImgOpt with paint.net's defaults for fmt: quality 95 (JPEG / WebP / AVIF / HEIC), 90
/// (JPEG XL / XR), dithering 7, transparency threshold 128, JPEG chroma 4:2:0, TGA 32 bit +
/// RLE, DDS BC1, JPEG XL effort 7  [img_opt_init]
pub fn opt_init<'a>(fmt: u32) -> ImgOpt<'a> {
    // quality, flags, bitdepth, dither + 1, threshold + 1, chroma, effort, ddsfmt
    static DEFS: [[u32; 8]; 14] = [
        [0; 8],
        [0, 0, 0, 8, 129, 0, 0, 0],
        [95, 0, 0, 0, 0, 420, 0, 0],
        [0, 0, 0, 8, 129, 0, 0, 0],
        [0, 0, 0, 8, 129, 0, 0, 0],
        [0, 0, 0, 8, 129, 0, 0, 0],
        [0, IOF_RLE, 32, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, DDSF_BC1],
        [95, 0, 0, 0, 0, 0, 0, 0],
        [95, 0, 0, 0, 0, 420, 0, 0],
        [95, 0, 0, 0, 0, 420, 0, 0],
        [90, 0, 0, 0, 0, 0, 7, 0],
        [90, 0, 0, 0, 0, 444, 0, 0],
        [0; 8],
    ];
    let mut o = ImgOpt { fmt, ..Default::default() };
    if fmt == 0 || fmt >= IMGF_COUNT {
        return o;
    }
    let d = DEFS.get(fmt as usize).copied().unwrap_or([0; 8]);
    o.quality = d[0];
    o.flags = d[1];
    o.bitdepth = d[2];
    o.dither = d[3];
    o.athresh = d[4];
    o.chroma = d[5];
    o.effort = d[6];
    o.ddsfmt = d[7];
    o
}

// ================================================================== file names

// str_ends: path ends with suffix (ASCII case-insensitive)
fn ends_ci(path: &[u8], suffix: &[u8]) -> bool {
    path.len() >= suffix.len() && path[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

static EXT_CLASSIC: [(&str, u32); 8] = [
    (".png", IMGF_PNG),
    (".jpg", IMGF_JPEG),
    (".jpeg", IMGF_JPEG),
    (".jfif", IMGF_JPEG),
    (".jpe", IMGF_JPEG),
    (".bmp", IMGF_BMP),
    (".dib", IMGF_BMP),
    (".gif", IMGF_GIF),
];

static EXT_ALL: [(&str, u32); 24] = [
    (".png", IMGF_PNG),
    (".jpg", IMGF_JPEG),
    (".jpeg", IMGF_JPEG),
    (".jpe", IMGF_JPEG),
    (".jfif", IMGF_JPEG),
    (".exif", IMGF_JPEG),
    (".bmp", IMGF_BMP),
    (".dib", IMGF_BMP),
    (".rle", IMGF_BMP),
    (".gif", IMGF_GIF),
    (".tif", IMGF_TIFF),
    (".tiff", IMGF_TIFF),
    (".tga", IMGF_TGA),
    (".dds", IMGF_DDS),
    (".webp", IMGF_WEBP),
    (".avif", IMGF_AVIF),
    (".heic", IMGF_HEIC),
    (".heif", IMGF_HEIC),
    (".jxl", IMGF_JXL),
    (".jxr", IMGF_JXR),
    (".wdp", IMGF_JXR),
    (".hdp", IMGF_JXR),
    (".wmp", IMGF_JXR),
    (".pdn", IMGF_PDN),
];

// a path as the assembly sees it: up to the first NUL
fn cpath(path: &[u8]) -> &[u8] {
    &path[..path.iter().position(|&b| b == 0).unwrap_or(path.len())]
}

/// IMGF_PNG / JPEG / BMP / GIF from the file extension (any case), 0 when unknown
/// [img_format]
pub fn format(path: &str) -> u32 {
    let p = cpath(path.as_bytes());
    EXT_CLASSIC.iter().find(|(s, _)| ends_ci(p, s.as_bytes())).map_or(0, |e| e.1)
}

/// IMGF_* from the file extension (any case), 0 when unknown; the registered runtime
/// formats after the built-in ones  [img_format_from_path]
pub fn format_from_path(path: &str) -> u32 {
    format_from_bytes(path.as_bytes())
}

/// format_from_path of a path given as bytes (up to the first NUL)
pub fn format_from_bytes(path: &[u8]) -> u32 {
    let p = cpath(path);
    if let Some(e) = EXT_ALL.iter().find(|(s, _)| ends_ci(p, s.as_bytes())) {
        return e.1;
    }
    dyn_ext_lookup(p)
}

// dyn_ext_lookup: the runtime format whose extension list names the extension of path
fn dyn_ext_lookup(path: &[u8]) -> u32 {
    let t = *FMTS.lock();
    for k in IMG_DYN0..IMGF_COUNT {
        let f = &t[k as usize];
        if f.exts.is_empty() {
            continue;
        }
        let list = f.exts.as_bytes().split(|&b| b == b'=').next().unwrap_or(&[]);
        for ext in list.split(|&b| b == b';') {
            if ext.is_empty() {
                continue;
            }
            let mut buf = [0u8; 30];
            buf[0] = b'.';
            let n = ext.len().min(29);
            buf[1..1 + n].copy_from_slice(&ext[..n]);
            if ends_ci(path, &buf[..1 + n]) {
                return k;
            }
        }
    }
    0
}

// ================================================================== file access

/// the file functions of load / save (img_core sits below plat)
#[derive(Clone, Copy)]
pub struct FileIo {
    pub read: fn(&str) -> Option<Vec<u8>>,
    pub write: fn(&str, &[u8]) -> bool, // atomic write
}

static FILE_IO: SpinLock<Option<FileIo>> = SpinLock::new(None);

/// set the file functions (call once at start-up, e.g. with plat::read_file / write_file)
pub fn set_file_io(io: FileIo) {
    *FILE_IO.lock() = Some(io);
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let io = *FILE_IO.lock();
    io.and_then(|f| (f.read)(path))
}

fn write_file(path: &str, data: &[u8]) -> bool {
    let io = *FILE_IO.lock();
    io.is_some_and(|f| (f.write)(path, data))
}

// ================================================================== classic API

/// decode PNG, JPEG, GIF or BMP data (by its magic bytes) into a new FMT_PARGB surface
/// [img_decode]
pub fn decode(data: &[u8]) -> Result<Surface, i32> {
    let len = data.len();
    let fmt = if len >= 8 && data[..8] == PNG_SIG {
        IMGF_PNG
    } else if len >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        IMGF_JPEG
    } else if len >= 6 && &data[..4] == b"GIF8" && data[5] == b'a' {
        IMGF_GIF
    } else if len < 2 {
        return Err(ERR_TRUNC);
    } else if &data[..2] == b"BM" {
        IMGF_BMP
    } else if let Some(c) = codec(IMGF_SVG).filter(|c| c.probe.is_some_and(|p| p(data, &mut ImgMeta::default()))) {
        // (svg) SVG once img_svg::register entered it
        let Some(f) = c.decode_fl else { return Err(ERR_UNSUPPORTED) };
        let mut sf = Surface::default();
        let r = f(data, &mut sf, 0);
        return if r != 0 { Err(r) } else { Ok(sf) };
    } else {
        return Err(ERR_UNSUPPORTED);
    };
    let Some(f) = codec(fmt).and_then(|c| c.decode_fl) else { return Err(ERR_UNSUPPORTED) };
    let mut sf = Surface::default();
    let r = f(data, &mut sf, 0);
    if r != 0 { Err(r) } else { Ok(sf) }
}

/// read a PNG, JPEG, GIF or BMP file into a new FMT_PARGB surface  [img_load]
pub fn load(path: &str) -> Result<Surface, i32> {
    match read_file(path) {
        Some(d) => decode(&d),
        None => Err(ERR_IO),
    }
}

/// write sf in the format of the file extension (see format); JPEG with quality 90
/// [img_save]
pub fn save(path: &str, sf: &Surface) -> i32 {
    save_ex(path, sf, 0)
}

/// save with a JPEG quality (1..100, 0 = 90)  [img_save_ex]
pub fn save_ex(path: &str, sf: &Surface, quality: i32) -> i32 {
    let q = if quality <= 0 { IMG_JPEG_QUALITY } else { quality.min(100) as u32 };
    if sf.w <= 0 || sf.h <= 0 {
        return ERR_DIMS;
    }
    if sf.fmt > FMT_A8 {
        return ERR_UNSUPPORTED;
    }
    let fmt = format(path);
    if fmt == 0 {
        return ERR_UNSUPPORTED;
    }
    let Some(f) = codec(fmt).and_then(|c| c.save) else { return ERR_UNSUPPORTED };
    let Some(buf) = f(sf, q) else { return ERR_NOMEM };
    if write_file(path, &buf) { 0 } else { ERR_IO }
}

// ================================================================== detection

pub const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// the format of data by its magic bytes (order: PNG, JPEG, GIF, DDS, WebP, TIFF / BigTIFF,
/// JPEG XR, ISOBMFF (AVIF / HEIC), JPEG XL container, PDN, gzip (legacy PDN), JPEG XL
/// codestream, BMP, runtime formats, TGA by hint or its registered probe) -> IMGF_* / 0
/// [img_detect]
pub fn detect(data: &[u8], hint: u32) -> u32 {
    let len = data.len();
    if len >= 8 && data[..8] == PNG_SIG {
        return IMGF_PNG;
    }
    if len >= 3 {
        if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
            return IMGF_JPEG;
        }
        if len >= 4 {
            let c = le32(data, 0);
            if len >= 6 && &data[..4] == b"GIF8" && (&data[4..6] == b"7a" || &data[4..6] == b"9a") {
                return IMGF_GIF;
            }
            if &data[..4] == b"DDS " {
                return IMGF_DDS;
            }
            if &data[..4] == b"RIFF" && len >= 12 && &data[8..12] == b"WEBP" {
                return IMGF_WEBP;
            }
            if c == 0x002A_4949 || c == 0x2A00_4D4D || c == 0x002B_4949 || c == 0x2B00_4D4D {
                return IMGF_TIFF;
            }
            if c & 0x00FF_FFFF == 0x00BC_4949 {
                return IMGF_JXR;
            }
            if len >= 12 {
                if &data[4..8] == b"ftyp" {
                    let k = bmff_kind(data);
                    if k != 0 {
                        return k;
                    }
                } else if c == 0x0C00_0000 && &data[4..8] == b"JXL " && le32(data, 8) == 0x0A87_0A0D {
                    return IMGF_JXL;
                }
            }
            if &data[..4] == b"PDN3" {
                return IMGF_PDN;
            }
        }
        if data[0] == 0x1F && data[1] == 0x8B && data[2] == 8 {
            if hint == IMGF_SVG && codec(IMGF_SVG).is_some() {
                return IMGF_SVG; // (svg) gzip of a .svgz file
            }
            return IMGF_PDN; // gzip: legacy .pdn
        }
    }
    if len >= 2 {
        if data[0] == 0xFF && data[1] == 0x0A {
            return IMGF_JXL; // JPEG XL codestream
        }
        if &data[..2] == b"BM" {
            if len < 18 {
                return IMGF_BMP; // too short to tell: the decoder reports ERR_TRUNC
            }
            if [12, 40, 52, 56, 64, 108, 124].contains(&le32(data, 14)) {
                return IMGF_BMP;
            }
        }
    }
    // runtime formats, then TGA
    let k = dyn_detect(data, hint);
    if k != 0 {
        return k;
    }
    if hint == IMGF_TGA {
        return IMGF_TGA;
    }
    if let Some(p) = codec(IMGF_TGA).and_then(|c| c.probe) {
        let mut m = ImgMeta::default();
        if p(data, &mut m) {
            return IMGF_TGA;
        }
    }
    0
}

// dyn_detect: the hint slot first (its probe; accepted without one), then every other probe
// in slot order
fn dyn_detect(data: &[u8], hint: u32) -> u32 {
    let t = *FMTS.lock();
    for pass in (IMG_DYN0 - 1)..IMGF_COUNT {
        let k = if pass == IMG_DYN0 - 1 {
            if hint < IMG_DYN0 {
                continue;
            }
            hint
        } else {
            pass
        };
        if k >= IMGF_COUNT {
            continue;
        }
        let f = &t[k as usize];
        if f.name.is_empty() {
            continue;
        }
        let Some(c) = f.codec else { continue };
        match c.probe {
            None => {
                if pass == IMG_DYN0 - 1 {
                    return k;
                }
            }
            Some(p) => {
                if pass != IMG_DYN0 - 1 && k == hint {
                    continue; // the hint slot was asked already
                }
                let mut m = ImgMeta::default();
                if p(data, &mut m) {
                    return k;
                }
            }
        }
    }
    0
}

// brand_kind: 1 AVIF brand, 2 HEIC brand, 3 generic HEIF / MIAF brand, 0 other
fn brand_kind(b: &[u8]) -> u32 {
    match b {
        b"avif" | b"avis" => 1,
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"hevm" | b"hevs" => 2,
        b"mif1" | b"msf1" | b"miaf" => 3,
        _ => 0,
    }
}

// bmff_kind: an ISOBMFF file with "ftyp" at offset 4: the major brand, then the compatible
// brands, then (MIAF / HEIF generic brands) the primary item's type
fn bmff_kind(data: &[u8]) -> u32 {
    let size = be32(data, 0) as usize;
    if size < 16 {
        return 0;
    }
    let size = size.min(data.len());
    let k = brand_kind(&data[8..12]);
    let mut seen = match k {
        1 => return IMGF_AVIF,
        2 => return IMGF_HEIC,
        0 => 0u32,
        _ => 4u32,
    };
    let mut p = 16;
    while p + 4 <= size {
        match brand_kind(&data[p..p + 4]) {
            1 => seen |= 1,
            2 => seen |= 2,
            3 => seen |= 4,
            _ => {}
        }
        p += 4;
    }
    match seen {
        1 | 5 => return IMGF_AVIF,
        2 | 6 => return IMGF_HEIC,
        0 => return 0,
        _ => {}
    }
    match &bmff_primary_type(data).to_le_bytes() {
        b"av01" => return IMGF_AVIF,
        b"hvc1" | b"hev1" => return IMGF_HEIC,
        _ => {}
    }
    if seen & 1 != 0 {
        IMGF_AVIF
    } else if seen & 2 != 0 {
        IMGF_HEIC
    } else {
        0
    }
}

// bmff_box(data, end, pos) -> (box size incl. header, type, header size); None without a
// complete header; size 0 extends to end, a box running past end is cut at end
fn bmff_box(data: &[u8], end: usize, pos: usize) -> Option<(usize, u32, usize)> {
    if pos > end || end - pos < 8 {
        return None;
    }
    let rest = end - pos;
    let s = be32(data, pos) as u64;
    let typ = le32(data, pos + 4);
    let (size, hdr) = match s {
        1 => {
            if rest < 16 {
                return None;
            }
            (u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()), 16usize)
        }
        0 => (rest as u64, 8),
        _ => (s, 8),
    };
    if size < hdr as u64 {
        return None;
    }
    Some((size.min(rest as u64) as usize, typ, hdr))
}

const T_META: u32 = u32::from_le_bytes(*b"meta");
const T_PITM: u32 = u32::from_le_bytes(*b"pitm");
const T_IINF: u32 = u32::from_le_bytes(*b"iinf");
const T_INFE: u32 = u32::from_le_bytes(*b"infe");
const T_AV01: u32 = u32::from_le_bytes(*b"av01");
const T_HVC1: u32 = u32::from_le_bytes(*b"hvc1");
const T_HEV1: u32 = u32::from_le_bytes(*b"hev1");

// bmff_primary_type: the item type of the primary item (meta / pitm / iinf / infe v2 / v3)
// when it is av01 / hvc1 / hev1, else the first such item type seen, else 0
fn bmff_primary_type(data: &[u8]) -> u32 {
    let len = data.len();
    let mut primary = u32::MAX;
    let mut first = 0u32;
    let mut ptype = 0u32;
    let coded = |t: u32| t == T_AV01 || t == T_HVC1 || t == T_HEV1;
    let mut pos = 0usize;
    let mut boxes = 0;
    let mut meta = None;
    while boxes < BOX_MAX {
        boxes += 1;
        let Some((size, typ, hdr)) = bmff_box(data, len, pos) else { break };
        let end = pos + size;
        if typ == T_META {
            meta = Some((pos + hdr + 4, end));
            break;
        }
        pos = end;
    }
    if let Some((fc, mend)) = meta {
        // pass 1: pitm
        let mut p = fc;
        let mut n = 0;
        while n < BOX_MAX {
            n += 1;
            let Some((size, typ, hdr)) = bmff_box(data, mend, p) else { break };
            let cend = p + size;
            if typ == T_PITM {
                let c = p + hdr;
                if c + 6 <= cend {
                    if data[c] == 0 {
                        primary = u16::from_be_bytes([data[c + 4], data[c + 5]]) as u32;
                        break;
                    }
                    if c + 8 <= cend {
                        primary = be32(data, c + 4);
                        break;
                    }
                }
            }
            p = cend;
        }
        // pass 2: iinf / infe
        let mut p = fc;
        let mut n = 0;
        'pass2: while n < BOX_MAX {
            n += 1;
            let Some((size, typ, hdr)) = bmff_box(data, mend, p) else { break };
            let iend = p + size;
            if typ != T_IINF {
                p = iend;
                continue;
            }
            let v = p + hdr;
            if v + 6 > iend {
                break;
            }
            let mut e = v + 6;
            if data[v] != 0 {
                e += 2;
            }
            let mut k = 0;
            while k < 4096 {
                k += 1;
                let Some((size, typ, hdr)) = bmff_box(data, iend, e) else { break 'pass2 };
                let fend = e + size;
                if typ == T_INFE {
                    let c = e + hdr;
                    if c < fend {
                        let item = match data[c] {
                            2 if c + 12 <= fend => Some((u16::from_be_bytes([data[c + 4], data[c + 5]]) as u32, le32(data, c + 8))),
                            3 if c + 14 <= fend => Some((be32(data, c + 4), le32(data, c + 10))),
                            _ => None,
                        };
                        if let Some((id, t)) = item {
                            if id == primary {
                                ptype = t;
                            }
                            if first == 0 && coded(t) {
                                first = t;
                            }
                        }
                    }
                }
                e = fend;
            }
            break;
        }
    }
    if coded(ptype) { ptype } else { first }
}

/// detect without a hint; with a meta: it is cleared (fmt, orient 1, frames 1 set) and the
/// registered codec's probe reads the header into it (w = 0 when the header could not be
/// read; no blobs)  [img_probe]
pub fn probe(data: &[u8], meta: Option<&mut ImgMeta>) -> u32 {
    let fmt = detect(data, 0);
    if let Some(m) = meta {
        *m = ImgMeta { fmt, orient: 1, frames: 1, ..Default::default() };
        if fmt != 0
            && let Some(p) = codec(fmt).and_then(|c| c.probe)
        {
            p(data, m);
            meta_free(m);
        }
    }
    fmt
}

// ================================================================== decode / encode

/// decode any registered format into a new surface: FMT_ARGB with IMGD_STRAIGHT, else
/// FMT_PARGB (see the module header)  [img_decode_ex]
pub fn decode_ex(data: &[u8], dec: Option<&ImgDec>, meta: Option<&mut ImgMeta>) -> Result<Surface, i32> {
    let mut d = dec.copied().unwrap_or_default();
    if d.maxpx == 0 {
        d.maxpx = IMG_MAXPX_DEF;
    }
    let mut local = ImgMeta::default();
    let m = match meta {
        Some(m) => {
            *m = ImgMeta::default();
            m
        }
        None => {
            d.flags &= !(IMGD_ICC | IMGD_EXIF | IMGD_XMP);
            &mut local
        }
    };
    m.orient = 1;
    m.frames = 1;
    let r = decode_body(data, &d, m);
    if r.is_err() {
        meta_free(m);
    }
    r
}

fn decode_body(data: &[u8], d: &ImgDec, m: &mut ImgMeta) -> Result<Surface, i32> {
    if d.cancelled() {
        return Err(ERR_CANCEL);
    }
    let fmt = detect(data, d.hint);
    m.fmt = fmt;
    if fmt == 0 {
        return Err(if data.len() >= 2 { ERR_UNSUPPORTED } else { ERR_TRUNC });
    }
    let Some(f) = codec(fmt).and_then(|c| c.decode) else { return Err(ERR_UNSUPPORTED) };
    let mut sf = Surface::default();
    let r = f(data, &mut sf, d, m);
    if r != 0 {
        return Err(r);
    }
    if d.flags & IMGD_PROBE != 0 {
        return Ok(sf);
    }
    if d.cancelled() {
        return Err(ERR_CANCEL);
    }
    if sf.px.is_empty() {
        return Err(ERR_FAIL);
    }
    if m.w == 0 {
        m.w = sf.w as u32;
        m.h = sf.h as u32;
    }
    if m.flags & IMF_ORIENT_APPLIED == 0 && d.flags & IMGD_NO_ORIENT == 0 && (2..=8).contains(&m.orient) {
        let r = orient(&mut sf, m.orient as u32);
        if r != 0 {
            return Err(r);
        }
        m.flags |= IMF_ORIENT_APPLIED;
    }
    let r = convert(&mut sf, if d.flags & IMGD_STRAIGHT != 0 { FMT_ARGB } else { FMT_PARGB });
    if r != 0 {
        return Err(r);
    }
    Ok(sf)
}

/// read a file and decode_ex it; a zero dec.hint comes from the file extension; ERR_IO
/// when unreadable  [img_load_ex]
pub fn load_ex(path: &str, dec: Option<&ImgDec>, meta: Option<&mut ImgMeta>) -> Result<Surface, i32> {
    let mut d = dec.copied().unwrap_or_default();
    if d.hint == 0 {
        d.hint = format_from_path(path);
    }
    match read_file(path) {
        Some(data) => decode_ex(&data, Some(&d), meta),
        None => Err(ERR_IO),
    }
}

/// encode sf (FMT_XRGB, PARGB, ARGB or A8) as opt.fmt  [img_encode]
/// Errors: ERR_UNSUPPORTED (format unknown / not registered / no encoder, or another
/// surface format), ERR_DIMS (empty surface), ERR_CANCEL and the codec errors.
pub fn encode(sf: &Surface, opt: Option<&ImgOpt>) -> Result<Vec<u8>, i32> {
    let o = opt.copied().unwrap_or_default();
    if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() {
        return Err(ERR_DIMS);
    }
    if sf.fmt > FMT_ARGB {
        return Err(ERR_UNSUPPORTED);
    }
    if o.cancelled() {
        return Err(ERR_CANCEL);
    }
    let Some(f) = codec(o.fmt).and_then(|c| c.encode) else { return Err(ERR_UNSUPPORTED) };
    let mut buf = Vec::new();
    let r = f(sf, &o, &mut buf);
    if r != 0 { Err(r) } else { Ok(buf) }
}

/// encode and write the file atomically; fmt 0 takes the format from the extension;
/// ERR_IO when writing fails  [img_save_opt]
pub fn save_opt(path: &str, sf: &Surface, opt: Option<&ImgOpt>) -> i32 {
    let mut o = opt.copied().unwrap_or_default();
    if o.fmt == 0 {
        o.fmt = format_from_path(path);
        if o.fmt == 0 {
            return ERR_UNSUPPORTED;
        }
    }
    match encode(sf, Some(&o)) {
        Ok(buf) => {
            if write_file(path, &buf) {
                0
            } else {
                ERR_IO
            }
        }
        Err(e) => e,
    }
}

// ================================================================== core codec glue (imgcore.s)

/// run the metadata scanner; with IMGD_PROBE return its result; else refuse frame > 0,
/// check the header size (ERR_LIMIT) and call the classic decoder with JDF_STRAIGHT
/// (IMGD_STRAIGHT) | JDF_NO_ORIENT (IMGD_NO_ORIENT)  [core_decode]
pub fn core_decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta, scan: ScanFn, decode_fl: DecodeFlFn) -> i32 {
    let r = scan(data, meta, dec.flags);
    if dec.flags & IMGD_PROBE != 0 {
        return r;
    }
    if dec.frame != 0 {
        return ERR_UNSUPPORTED;
    }
    if r == 0 && dims_check(meta.w, meta.h, dec) == ERR_LIMIT {
        return ERR_LIMIT; // (a failed scan: the decoder reports the error)
    }
    if dec.cancelled() {
        return ERR_CANCEL;
    }
    let mut jdf = dec.flags & IMGD_STRAIGHT;
    if dec.flags & IMGD_NO_ORIENT != 0 {
        jdf |= JDF_NO_ORIENT;
    }
    decode_fl(data, sf, jdf)
}

/// the scanner without blobs: recognised (meta filled)  [core_probe]
pub fn core_probe(data: &[u8], meta: &mut ImgMeta, scan: ScanFn) -> bool {
    scan(data, meta, 0) == 0
}

/// pixels per metre from a 16.16 dpi value (rounded, at most 0x7FFFFFFF)  [dpi_ppm]
pub fn dpi_ppm(dpi: u32) -> u32 {
    ((dpi as u64 * 10000 + 127 * 65536) / (254 * 65536)).min(0x7FFF_FFFF) as u32
}

/// 16.16 dpi from pixels per metre (0 -> 0)  [ppm_dpi]
pub fn ppm_dpi(ppm: u32) -> u32 {
    ((ppm as u64 * (254 * 65536) + 5000) / 10000).min(0xFFFF_FFFF) as u32
}
