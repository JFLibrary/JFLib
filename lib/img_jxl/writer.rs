// writer -- the interface of the new JPEG XL writer (jxl-plus) as the decoder crate sees it:
// its options, the image samples it reads, and which writer runs (jxl-module)
//                                                            [jxl-plus / jxl-module: no asm]
//
//   Opts                       the writer's options parsed from an ImgOpt (effort 1..9, 0 = 7;
//                              quality, flags, the ICC / EXIF / XMP blobs, cancel)
//   IOF_JXL_*                  the writer's IO_FLAGS bits (COMPAT, 1THREAD, PROGRESSIVE,
//                              NOPATCHES, the photon noise ISO in bits 24..31)
//   Px, read_image(sf), image_of(sf)   the 8-bit RGBA samples of a surface (gray: R = G = B
//                              everywhere; alpha: a pixel not opaque); image_of checks the size
//                              first (ERR_DIMS) like the writer does
//   set_static(f)              the writer linked into the app (img_jxl_enc::use_static())
//   use_module(path)           the writer of the loadable module (PaintCraft): <exe folder>/jxlenc.dll
//                              (Linux jxlenc.so), or path; "off" = none.  plat loads it on the
//                              first JPEG XL save and keeps it (plat::jxlenc_*)
//   kind() -> Kind             which writer a save without IOF_JXL_COMPAT uses: Static, Module
//                              (loads it), Compat (no writer installed, or the module is missing)
//   write(sf, opts) -> Some(file / ERR_*) | None (no new writer: the caller uses the assembly's)
//
// The image goes to the module as these RGBA samples (read on the host: the same pass as the
// linked writer), the options as their scalar fields and blobs; the module runs the same
// img_jxl_enc::enew::encode_px, so the file is the same byte for byte (any thread count: its
// parallel parts run on the host's worker pool).  Without any writer installed, or without the
// module file, saving uses the assembly-compatible writer (enc.rs / venc.rs: quality, lossless,
// EXIF / XMP; effort, progressive, noise, ICC are ignored).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use core::sync::atomic::{AtomicU32, Ordering};

use gfx::FMT_ARGB;
use img_core::meta::get_row;
use img_core::{ImgOpt, Surface, ERR_DIMS, IMG_MAXDIM, IOF_EMBED_ICC, IOF_LOSSLESS};
use rt::sync::SpinLock;

/// IO_FLAGS bit: the assembly-compatible writer (byte-identical to the asm reference)
pub const IOF_JXL_COMPAT: u32 = 0x10_0000;
/// IO_FLAGS bit: no worker threads (the output is the same either way)
pub const IOF_JXL_1THREAD: u32 = 0x20_0000;
/// IO_FLAGS bit: progressive (VarDCT: DC + 2 AC passes; lossless: squeeze, responsive)
pub const IOF_JXL_PROGRESSIVE: u32 = 0x40_0000;
/// IO_FLAGS bit (jxl-tools): no patches (libjxl's patches = 0; for comparisons)
pub const IOF_JXL_NOPATCHES: u32 = 0x80_0000;
/// IO_FLAGS bits 24..31 (jxl-tools): photon noise ISO / 100 (lossy only; 0 = off)
pub const IOF_JXL_NOISE_SHIFT: u32 = 24;
/// the mask of the photon noise bits (jxl-tools)
pub const IOF_JXL_NOISE_MASK: u32 = 0xFF00_0000;

/// the encoder options
#[derive(Clone, Copy)]
pub struct Opts<'a> {
    pub effort: u32,
    pub quality: u32,
    pub flags: u32,
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
    pub cancel: Option<&'a AtomicU32>,
    pub splines: Option<&'a crate::spline::Quantized>, // jxl-tools: the test-only spline writer
}

impl<'a> Opts<'a> {
    /// the options of an ImgOpt: the ICC profile with IOF_EMBED_ICC, EXIF / XMP when present
    /// (the container writer looks at IOF_EMBED_EXIF / _XMP)
    pub fn from_imgopt(opt: &ImgOpt<'a>) -> Opts<'a> {
        let (mut icc, mut exif, mut xmp) = (None, None, None);
        if let Some(m) = opt.meta {
            if opt.flags & IOF_EMBED_ICC != 0 && !m.icc.is_empty() {
                icc = Some(&m.icc[..]);
            }
            if !m.exif.is_empty() {
                exif = Some(&m.exif[..]);
            }
            if !m.xmp.is_empty() {
                xmp = Some(&m.xmp[..]);
            }
        }
        Opts { effort: opt.effort, quality: opt.quality, flags: opt.flags, icc, exif, xmp, cancel: opt.cancel, splines: None }
    }
    /// the effort 1..9 (0 = 7)
    pub fn effort(&self) -> u32 {
        if self.effort == 0 { 7 } else { self.effort.clamp(1, 9) }
    }
    /// worker threads allowed (no IOF_JXL_1THREAD)
    pub fn threads(&self) -> bool {
        self.flags & IOF_JXL_1THREAD == 0
    }
    /// lossless: IOF_LOSSLESS or quality 100 and more
    pub fn lossless(&self) -> bool {
        self.flags & IOF_LOSSLESS != 0 || self.quality >= 100
    }
    /// the cancel flag is set
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }
    /// the photon noise ISO (0 = none)  [jxl-tools]
    pub fn noise_iso(&self) -> u32 {
        (self.flags >> IOF_JXL_NOISE_SHIFT) * 100
    }
    /// patches allowed at this effort (libjxl: lossless 5+, lossy 7+)  [jxl-tools]
    pub fn patches(&self, lossless: bool) -> bool {
        self.flags & IOF_JXL_NOPATCHES == 0 && self.effort() >= if lossless { 5 } else { 7 }
    }
}

/// the samples of the image
pub struct Px {
    pub w: u32,
    pub h: u32,
    pub gray: bool,
    pub alpha: bool,
    pub rgba: Vec<u8>, // R, G, B, A per pixel
}

/// read the surface as 8-bit RGBA
pub fn read_image(sf: &Surface) -> Px {
    let w = sf.w as usize;
    let h = sf.h as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let mut row = vec![0u8; w * 4 + 64];
    let mut gray = true;
    let mut alpha = false;
    for y in 0..h {
        get_row(sf, y, &mut row, FMT_ARGB);
        let o = &mut rgba[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
            o[x * 4] = r;
            o[x * 4 + 1] = g;
            o[x * 4 + 2] = b;
            o[x * 4 + 3] = a;
            if a != 255 {
                alpha = true;
            }
            if r != g || r != b {
                gray = false;
            }
        }
    }
    Px { w: w as u32, h: h as u32, gray, alpha, rgba }
}

/// the samples of a surface the writer accepts (1..IMG_MAXDIM on each side, else ERR_DIMS)
pub fn image_of(sf: &Surface) -> Result<Px, i32> {
    let (w, h) = (sf.w, sf.h);
    if w <= 0 || h <= 0 || w as u32 > IMG_MAXDIM || h as u32 > IMG_MAXDIM {
        return Err(ERR_DIMS);
    }
    Ok(read_image(sf))
}

/// a writer linked into the app: the samples and options in, the file out
pub type WriterFn = fn(&Px, &Opts) -> Result<Vec<u8>, i32>;

/// which writer saves JPEG XL files without IOF_JXL_COMPAT
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// the writer linked into the app (img_jxl_enc)
    Static,
    /// the loadable module (jxlenc.dll / jxlenc.so next to the exe)
    Module,
    /// none: the assembly-compatible writer
    Compat,
}

#[derive(Clone, Copy)]
enum Hook {
    None,
    Static(WriterFn),
    Module,
}

static HOOK: SpinLock<Hook> = SpinLock::new(Hook::None);

/// the writer linked into the app (img_jxl_enc::use_static() installs enew::encode_px)
pub fn set_static(f: WriterFn) {
    *HOOK.lock() = Hook::Static(f);
}

/// the writer of the module file: None = <exe folder>/jxlenc.dll (.so), Some(path) = that file,
/// Some("off") = none (the assembly-compatible writer; tests).  Loaded on first use.
pub fn use_module(path: Option<&str>) {
    plat::jxlenc_set_path(path.map(String::from));
    *HOOK.lock() = Hook::Module;
}

/// which writer saves (Module only when the module file loads: this loads it)
pub fn kind() -> Kind {
    let h = *HOOK.lock();
    match h {
        Hook::Static(_) => Kind::Static,
        Hook::Module if plat::jxlenc_ready() => Kind::Module,
        _ => Kind::Compat,
    }
}

/// the new writer on sf: Some(file / ERR_*), None when there is none (no writer installed, or
/// the module file is missing / refused)
pub fn write(sf: &Surface, o: &Opts) -> Option<Result<Vec<u8>, i32>> {
    let h = *HOOK.lock();
    match h {
        Hook::Static(f) => Some(image_of(sf).and_then(|px| f(&px, o))),
        Hook::Module => {
            if !plat::jxlenc_ready() {
                return None;
            }
            let px = match image_of(sf) {
                Ok(px) => px,
                Err(e) => return Some(Err(e)),
            };
            let req = plat::JxlEncReq {
                w: px.w,
                h: px.h,
                gray: px.gray,
                alpha: px.alpha,
                rgba: px.rgba,
                effort: o.effort,
                quality: o.quality,
                flags: o.flags,
                icc: o.icc,
                exif: o.exif,
                xmp: o.xmp,
                cancel: o.cancel,
            };
            plat::jxlenc_encode(req)
        }
        Hook::None => None,
    }
}
