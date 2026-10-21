// abi -- the records of the plugin ABI, version 1, as #[repr(C)] types, and the const
// constructors of the descriptors a plugin puts into statics
//                      [ASMLIB/inc/plugin.inc, fx.inc (FXARGS, FXDESC, FXPROP), img.inc
//                       (IMGDEC, IMGMETA, IMGOPT), thread.inc (JOB), jfgui.inc (SURFACE)]
//
// Every function pointer uses the Win64 ABI (`extern "win64"`) on Linux too.  The layouts are
// asserted against the offsets of the include files at the end of this file.  Strings are
// NUL-terminated UTF-8 (`c"..."` literals); lists are static slices.  The descriptors hold raw
// pointers into the plugin's own image -- the host checks every one of them against the
// image's segments before it reads anything.

use core::ffi::{CStr, c_void};
use core::sync::atomic::AtomicU32;

use crate::{FP_ANGLE, FP_BOOL, FP_CHOICE, FP_COLOR, FP_DEC, FP_INT, FP_PAN, FP_SEED, FPF_LOG, PI_SIZE_V1, PLUGIN_MAGIC};

/// fn(ctx, index, job) of PA_PARFOR
pub type ItemFn = extern "win64" fn(ctx: *const c_void, index: u64, job: *const Job);
/// FXD_PREPARE: fn(FXARGS*) -> rax aux | 0 (0 also on error: the code in edx)
pub type PrepareFn = extern "win64" fn(*const FxArgs) -> *mut c_void;
/// FXD_RENDER: fn(FXARGS*) -> eax 0 / ERR_CANCEL / ERR_*
pub type RenderFn = extern "win64" fn(*const FxArgs) -> i32;
/// FXD_FREE: fn(aux)
pub type FreeFn = extern "win64" fn(*mut c_void);
/// FT_PROBE: fn(data, len, meta) -> eax 1 recognised / 0
pub type ProbeFn = extern "win64" fn(*const u8, usize, *mut ImgMeta) -> u32;
/// FT_DECODE: fn(data, len, sf, dec) -> eax 0 / ERR_*
pub type DecodeFn = extern "win64" fn(*const u8, usize, *mut Surface, *const ImgDec) -> i32;
/// FT_ENCODE: fn(sf, opt, &buf, &len) -> eax 0 / ERR_*
pub type EncodeFn = extern "win64" fn(*const Surface, *const ImgOpt, *mut *mut u8, *mut usize) -> i32;

/// PNAPI -- the host's function table, passed to pn_plugin_query (PA_*).  Later versions only
/// append entries; `size` says how many there are.
#[repr(C)]
pub struct PnApi {
    pub size: u32,
    pub version: u32,
    /// (size) -> zeroed, 64-byte aligned block | 0; thread safe
    pub alloc: extern "win64" fn(usize) -> *mut u8,
    /// (ptr) (0 allowed)
    pub free: extern "win64" fn(*mut u8),
    /// (ptr, size) -> block | 0
    pub realloc: extern "win64" fn(*mut u8, usize) -> *mut u8,
    /// (fn, ctx, count) -> 0 / ERR_CANCEL / ERR_FAULT: fn(ctx, index, job) on the workers
    pub parfor: extern "win64" fn(ItemFn, *const c_void, u64) -> i32,
    /// (x, y, seed, stream) -> deterministic per-pixel random bits (fx_hash32)
    pub hash32: extern "win64" fn(u32, u32, u32, u32) -> u32,
    /// (msg) a UTF-8 line into the plugin log (stderr, "PaintCraft: Plugin <file>: msg")
    pub log: extern "win64" fn(*const u8),
    /// (sf, y) -> row y of a SURFACE | 0
    pub row: extern "win64" fn(*const Surface, i32) -> *mut u8,
    /// (sf, w, h, dec) -> 0 / ERR_*: a zeroed straight FMT_ARGB surface for decoders
    pub surfalloc: extern "win64" fn(*mut Surface, i32, i32, *const ImgDec) -> i32,
    /// (job) -> 1 when the job's cancel flag is set
    pub cancelled: extern "win64" fn(*const Job) -> i32,
    /// (buf, cap, fmt, ...) -> length: the host's printf subset (C variadic; core::fmt and
    /// log_fmt are the Rust way)
    pub fmt: *const c_void,
    /// () -> worker threads
    pub nthreads: extern "win64" fn() -> u32,
}

/// SURFACE (jfgui.inc SF_*): pixels as rows of `stride` bytes, 0xAARRGGBB per pixel
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Surface {
    pub px: *mut u8,
    pub w: i32,
    pub h: i32,
    pub stride: i32,
    pub fmt: u8,
    pub flags: u8,
    pub opac: u8,
    pub blend: u8,
    pub aux: u64,
}

/// JOB (thread.inc): plugins only read `cancel`
#[repr(C)]
pub struct Job {
    pub func: *const c_void,
    pub ctx: *const c_void,
    pub count: u64,
    pub next: u64,
    pub done: u64,
    pub user: u64,
    /// nonzero: the job is cancelled (written by the host at any time)
    pub cancel: AtomicU32,
    pub state: u32,
    pub active: u32,
    pub flags: u32,
    pub link: *const c_void,
    pub rsv: u64,
}

/// FXARGS (fx.inc FXA_*): one prepare / render call
#[repr(C)]
pub struct FxArgs {
    /// source (FMT_ARGB, straight), read-only, the whole image
    pub src: *const Surface,
    /// destination: written only inside the ROI
    pub dst: *const Surface,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// FX_NSLOTS 8-byte parameter slots (see FxProp)
    pub param: *const u64,
    /// what prepare returned, or 0
    pub aux: *const c_void,
    /// poll JOB_CANCEL once per row (0 = none)
    pub job: *const Job,
    /// the selection (TMASK) or 0; not available to plugins of the Rust host
    pub sel: *const c_void,
}

/// IMGDEC (img.inc ID_*): decode options
#[repr(C)]
pub struct ImgDec {
    /// IMGD_*
    pub flags: u32,
    pub frame: u32,
    pub hint: u32,
    pub rsv: u32,
    pub maxpx: u64,
    /// IMGMETA to fill, or 0
    pub meta: *mut ImgMeta,
    /// pointer to a u32 that cancels when nonzero, or 0
    pub cancel: *const AtomicU32,
    pub maxmem: u64,
}

/// IMGMETA (img.inc IM_*): what a probe / decode reports about the image
#[repr(C)]
pub struct ImgMeta {
    pub fmt: u32,
    /// stored width / height
    pub w: u32,
    pub h: u32,
    /// bits per sample of the source
    pub bits: u8,
    /// IMM_*
    pub model: u8,
    pub orient: u8,
    /// samples per pixel incl. alpha (0 = unknown)
    pub channels: u8,
    /// IMF_*
    pub flags: u32,
    pub frames: u32,
    pub dpix: u32,
    pub dpiy: u32,
    pub cicp: u32,
    pub gamma: u32,
    pub icc: *const u8,
    pub icclen: u64,
    pub exif: *const u8,
    pub exiflen: u64,
    pub xmp: *const u8,
    pub xmplen: u64,
    pub delay: u32,
    pub loops: u32,
    pub quality: u32,
    pub resunit: u32,
    pub chrm: [u16; 8],
    pub dxgi: u16,
    pub texfl: u8,
    pub rsv: u8,
    pub texn: u32,
}

/// IMGOPT (img.inc IO_*): encoder options
#[repr(C)]
pub struct ImgOpt {
    pub fmt: u32,
    /// 1..100 (0 = format default)
    pub quality: u32,
    /// IOF_*
    pub flags: u32,
    pub bitdepth: u32,
    pub samplebits: u32,
    pub dither: u32,
    pub athresh: u32,
    pub chroma: u32,
    pub effort: u32,
    pub ddsfmt: u32,
    pub mipfilter: u32,
    pub tiffcomp: u32,
    pub meta: *const ImgMeta,
    pub cancel: *const AtomicU32,
}

/// FXPROP (fx.inc FXP_*): one property of an effect, shown in the generic dialog
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FxProp {
    /// FP_*
    pub typ: u8,
    /// decimals shown (FP_DEC / FP_ANGLE)
    pub dec: u8,
    /// first slot in the parameter block
    pub slot: u8,
    /// FPF_*
    pub flags: u8,
    pub rsv: u32,
    /// the label of the dialog
    pub name: *const u8,
    pub min: f64,
    pub max: f64,
    /// default (FP_COLOR: the colour in the low 32 bits of the bit pattern)
    pub def: f64,
    /// FP_CHOICE: the choice list (strings, then an empty one); FP_PAN: the y default as f64
    /// bits
    pub choices: *const u8,
}

/// FXDESC (fx.inc FXD_*): an effect or adjustment
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FxDesc {
    /// ignored for plugins (the host assigns one)
    pub id: u32,
    /// FXF_*
    pub flags: u32,
    /// the menu name (1..47 bytes, unique; also the settings key)
    pub name: *const u8,
    pub prepare: Option<PrepareFn>,
    pub render: Option<RenderFn>,
    pub free: Option<FreeFn>,
    pub nprops: u32,
    pub rsv: u32,
    pub props: *const FxProp,
    /// FXC_*
    pub category: u32,
    pub rsv2: u32,
}

/// FTDESC (plugin.inc FT_*): a file type (decoder and / or encoder)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FtDesc {
    /// display name ("QOI")
    pub name: *const u8,
    /// 1..8 extensions without dots, ';'-separated ("qoi")
    pub exts: *const u8,
    /// MIME type or 0
    pub mime: *const u8,
    pub probe: Option<ProbeFn>,
    pub decode: Option<DecodeFn>,
    pub encode: Option<EncodeFn>,
    /// IFC_*
    pub caps: u32,
    pub rsv: u32,
}

/// SHAPEDESC (plugin.inc PSD_*): a shape of the Formen tool
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShapeDesc {
    /// display name (1..63 bytes)
    pub name: *const u8,
    /// path data in the Geometry syntax of paint.net ShapeKit XAML ("M 0,0 L 100,0 ...")
    pub data: *const u8,
}

/// PLUGININFO (plugin.inc PI_*): what pn_plugin_query returns
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginInfo {
    pub magic: u32,
    /// size of this record (>= PI_SIZE_V1)
    pub size: u32,
    /// the lowest PNAPI version the plugin needs
    pub apimin: u32,
    pub flags: u32,
    /// 1..63 bytes
    pub name: *const u8,
    /// 1..31 bytes
    pub version: *const u8,
    pub author: *const u8,
    pub url: *const u8,
    pub nfx: u32,
    pub nft: u32,
    /// FXDESC*[nfx]
    pub fx: *const *const FxDesc,
    /// FTDESC[nft]
    pub ft: *const FtDesc,
    pub nshape: u32,
    pub rsv: u32,
    /// SHAPEDESC[nshape]
    pub shapes: *const ShapeDesc,
}

// SAFETY: the descriptors are immutable data in the plugin image; the raw pointers only lead
// to other immutable statics (strings, lists, functions).
unsafe impl Sync for PluginInfo {}
// SAFETY: as above.
unsafe impl Sync for FxDesc {}
// SAFETY: as above.
unsafe impl Sync for FxProp {}
// SAFETY: as above.
unsafe impl Sync for FtDesc {}
// SAFETY: as above.
unsafe impl Sync for ShapeDesc {}

const fn cs(s: &'static CStr) -> *const u8 {
    s.as_ptr().cast()
}

const fn ocs(s: Option<&'static CStr>) -> *const u8 {
    match s {
        Some(s) => cs(s),
        None => core::ptr::null(),
    }
}

impl PluginInfo {
    /// a plugin named `name` (1..63 bytes) in version `version` (1..31 bytes), PNAPI 1,
    /// nothing in it yet
    pub const fn new(name: &'static CStr, version: &'static CStr) -> PluginInfo {
        PluginInfo {
            magic: PLUGIN_MAGIC,
            size: PI_SIZE_V1,
            apimin: 1,
            flags: 0,
            name: cs(name),
            version: cs(version),
            author: core::ptr::null(),
            url: core::ptr::null(),
            nfx: 0,
            nft: 0,
            fx: core::ptr::null(),
            ft: core::ptr::null(),
            nshape: 0,
            rsv: 0,
            shapes: core::ptr::null(),
        }
    }

    pub const fn author(mut self, author: &'static CStr) -> PluginInfo {
        self.author = cs(author);
        self
    }

    /// origin / URL (1..127 bytes)
    pub const fn url(mut self, url: &'static CStr) -> PluginInfo {
        self.url = cs(url);
        self
    }

    /// the lowest PNAPI version the plugin needs (default 1)
    pub const fn api_min(mut self, v: u32) -> PluginInfo {
        self.apimin = v;
        self
    }

    /// the effects / adjustments (at most 16)
    pub const fn effects(mut self, list: &'static [&'static FxDesc]) -> PluginInfo {
        self.nfx = list.len() as u32;
        self.fx = list.as_ptr().cast();
        self
    }

    /// the file types (at most 4)
    pub const fn file_types(mut self, list: &'static [FtDesc]) -> PluginInfo {
        self.nft = list.len() as u32;
        self.ft = list.as_ptr();
        self
    }

    /// the shapes (at most 32)
    pub const fn shapes(mut self, list: &'static [ShapeDesc]) -> PluginInfo {
        self.nshape = list.len() as u32;
        self.shapes = list.as_ptr();
        self
    }
}

impl ShapeDesc {
    pub const fn new(name: &'static CStr, data: &'static CStr) -> ShapeDesc {
        ShapeDesc { name: cs(name), data: cs(data) }
    }
}

impl FtDesc {
    /// a file type with the functions of T (only those T says it has); `exts` = "a;b",
    /// `caps` = IFC_*
    pub const fn new<T: crate::FileType>(
        name: &'static CStr,
        exts: &'static CStr,
        mime: Option<&'static CStr>,
        caps: u32,
    ) -> FtDesc {
        FtDesc {
            name: cs(name),
            exts: cs(exts),
            mime: ocs(mime),
            probe: if T::PROBE { Some(crate::ft::probe_tramp::<T>) } else { None },
            decode: if T::DECODE { Some(crate::ft::decode_tramp::<T>) } else { None },
            encode: if T::ENCODE { Some(crate::ft::encode_tramp::<T>) } else { None },
            caps,
            rsv: 0,
        }
    }
}

impl FxDesc {
    /// the effect E named `name` (the menu name, 1..47 bytes), FXF_* `flags` (FXF_PREPARE is
    /// added when E has a prepare step), FXC_* `category`, the properties of its dialog
    pub const fn new<E: crate::Effect>(name: &'static CStr, flags: u32, category: u32, props: &'static [FxProp]) -> FxDesc {
        let prep = E::PREPARE;
        FxDesc {
            id: 0,
            flags: if prep { flags | crate::FXF_PREPARE } else { flags },
            name: cs(name),
            prepare: if prep { Some(crate::fx::prepare_tramp::<E>) } else { None },
            render: Some(crate::fx::render_tramp::<E>),
            free: if prep { Some(crate::fx::free_tramp::<E>) } else { None },
            nprops: props.len() as u32,
            rsv: 0,
            props: props.as_ptr(),
            category,
            rsv2: 0,
        }
    }
}

impl FxProp {
    const fn base(typ: u8, slot: u8, name: &'static CStr, min: f64, max: f64, def: f64) -> FxProp {
        FxProp { typ, dec: 0, slot, flags: 0, rsv: 0, name: cs(name), min, max, def, choices: core::ptr::null() }
    }

    /// FP_INT: an integer slider min..max (slot: i64)
    pub const fn int(slot: u8, name: &'static CStr, min: f64, max: f64, def: f64) -> FxProp {
        FxProp::base(FP_INT, slot, name, min, max, def)
    }

    /// FP_DEC: a number with `dec` decimals (slot: f64)
    pub const fn dec(slot: u8, name: &'static CStr, dec: u8, min: f64, max: f64, def: f64) -> FxProp {
        let mut p = FxProp::base(FP_DEC, slot, name, min, max, def);
        p.dec = dec;
        p
    }

    /// FP_ANGLE: degrees with `dec` decimals (slot: f64)
    pub const fn angle(slot: u8, name: &'static CStr, dec: u8, min: f64, max: f64, def: f64) -> FxProp {
        let mut p = FxProp::base(FP_ANGLE, slot, name, min, max, def);
        p.dec = dec;
        p
    }

    /// FP_BOOL: a check box (slot: i64 0 / 1)
    pub const fn boolean(slot: u8, name: &'static CStr, def: bool) -> FxProp {
        FxProp::base(FP_BOOL, slot, name, 0.0, 1.0, if def { 1.0 } else { 0.0 })
    }

    /// FP_CHOICE: one of the entries of `list` = b"first\0second\0...\0\0" (UTF-8; slot: i64
    /// index); `def` = the index chosen by default
    pub const fn choice(slot: u8, name: &'static CStr, list: &'static [u8], def: u32) -> FxProp {
        let n = strlist_len(list);
        assert!(def < n, "FxProp::choice: default index outside the list");
        let mut p = FxProp::base(FP_CHOICE, slot, name, 0.0, (n - 1) as f64, def as f64);
        p.choices = list.as_ptr();
        p
    }

    /// FP_PAN: a point -1..1 (slots slot and slot + 1: f64 x, y)
    pub const fn pan(slot: u8, name: &'static CStr, dec: u8, x: f64, y: f64) -> FxProp {
        let mut p = FxProp::base(FP_PAN, slot, name, -1.0, 1.0, x);
        p.dec = dec;
        p.choices = y.to_bits() as usize as *const u8;
        p
    }

    /// FP_COLOR: a straight ARGB colour (slot: the colour in the low 32 bits)
    pub const fn color(slot: u8, name: &'static CStr, argb: u32) -> FxProp {
        FxProp::base(FP_COLOR, slot, name, 0.0, 0.0, f64::from_bits(argb as u64))
    }

    /// FP_SEED: a random seed with a "Neu würfeln" button (slot: i64)
    pub const fn seed(slot: u8, name: &'static CStr) -> FxProp {
        FxProp::base(FP_SEED, slot, name, 0.0, 2147483647.0, 0.0)
    }

    /// the slider maps logarithmically (FPF_LOG)
    pub const fn log(mut self) -> FxProp {
        self.flags |= FPF_LOG;
        self
    }
}

/// the number of strings of a STRLIST (b"a\0b\0\0"); a list that does not end with an empty
/// string, has none or more than 32 entries is a compile-time error
const fn strlist_len(list: &[u8]) -> u32 {
    let n = list.len();
    assert!(n >= 3 && list[n - 1] == 0 && list[n - 2] == 0 && list[0] != 0, "a choice list is b\"a\\0b\\0\\0\"");
    let mut k = 0;
    let mut i = 0;
    while i < n - 1 {
        if list[i] == 0 {
            assert!(list[i - 1] != 0, "a choice list has no empty entries");
            k += 1;
        }
        i += 1;
    }
    assert!(k <= 32, "at most 32 choices");
    k
}

// every constructor evaluates at compile time (they are only used in statics)
const _: [FxProp; 9] = [
    FxProp::int(0, c"a", 0.0, 10.0, 5.0),
    FxProp::dec(1, c"b", 2, 0.0, 1.0, 0.5).log(),
    FxProp::angle(2, c"c", 1, -180.0, 180.0, 0.0),
    FxProp::boolean(3, c"d", true),
    FxProp::choice(4, c"e", b"x\0y\0\0", 1),
    FxProp::pan(5, c"f", 2, 0.25, -0.5),
    FxProp::color(7, c"g", 0xFF10_2030),
    FxProp::seed(8, c"h"),
    FxProp::int(9, c"i", 0.0, 1.0, 0.0),
];
const _: () = assert!(strlist_len(b"x\0y\0z\0\0") == 3);
const _: () = assert!(FxProp::color(0, c"p", 0xFF10_2030).def.to_bits() == 0xFF10_2030);
const _: ShapeDesc = ShapeDesc::new(c"s", c"M 0,0 L 1,1 Z");

// ------------------------------------------------------------------ layouts (the include files)
macro_rules! at {
    ($t:ty, $f:ident, $o:expr) => {
        const _: () = assert!(core::mem::offset_of!($t, $f) == $o);
    };
}
macro_rules! size {
    ($t:ty, $n:expr) => {
        const _: () = assert!(core::mem::size_of::<$t>() == $n);
    };
}

// plugin.inc: PA_*
size!(PnApi, 96);
at!(PnApi, size, 0);
at!(PnApi, version, 4);
at!(PnApi, alloc, 8);
at!(PnApi, free, 16);
at!(PnApi, realloc, 24);
at!(PnApi, parfor, 32);
at!(PnApi, hash32, 40);
at!(PnApi, log, 48);
at!(PnApi, row, 56);
at!(PnApi, surfalloc, 64);
at!(PnApi, cancelled, 72);
at!(PnApi, fmt, 80);
at!(PnApi, nthreads, 88);
const _: () = assert!(core::mem::size_of::<PnApi>() == crate::PA_SIZE_V1 as usize);
// PI_*
size!(PluginInfo, 88);
at!(PluginInfo, magic, 0);
at!(PluginInfo, size, 4);
at!(PluginInfo, apimin, 8);
at!(PluginInfo, flags, 12);
at!(PluginInfo, name, 16);
at!(PluginInfo, version, 24);
at!(PluginInfo, author, 32);
at!(PluginInfo, url, 40);
at!(PluginInfo, nfx, 48);
at!(PluginInfo, nft, 52);
at!(PluginInfo, fx, 56);
at!(PluginInfo, ft, 64);
at!(PluginInfo, nshape, 72);
at!(PluginInfo, rsv, 76);
at!(PluginInfo, shapes, 80);
const _: () = assert!(core::mem::size_of::<PluginInfo>() == PI_SIZE_V1 as usize);
// FT_*, PSD_*
size!(FtDesc, 56);
at!(FtDesc, name, 0);
at!(FtDesc, exts, 8);
at!(FtDesc, mime, 16);
at!(FtDesc, probe, 24);
at!(FtDesc, decode, 32);
at!(FtDesc, encode, 40);
at!(FtDesc, caps, 48);
at!(FtDesc, rsv, 52);
size!(ShapeDesc, 16);
at!(ShapeDesc, name, 0);
at!(ShapeDesc, data, 8);
// fx.inc: FXA_*, FXD_*, FXP_*
size!(FxArgs, 64);
at!(FxArgs, src, 0);
at!(FxArgs, dst, 8);
at!(FxArgs, x0, 16);
at!(FxArgs, y0, 20);
at!(FxArgs, x1, 24);
at!(FxArgs, y1, 28);
at!(FxArgs, param, 32);
at!(FxArgs, aux, 40);
at!(FxArgs, job, 48);
at!(FxArgs, sel, 56);
size!(FxDesc, 64);
at!(FxDesc, id, 0);
at!(FxDesc, flags, 4);
at!(FxDesc, name, 8);
at!(FxDesc, prepare, 16);
at!(FxDesc, render, 24);
at!(FxDesc, free, 32);
at!(FxDesc, nprops, 40);
at!(FxDesc, props, 48);
at!(FxDesc, category, 56);
size!(FxProp, 48);
at!(FxProp, typ, 0);
at!(FxProp, dec, 1);
at!(FxProp, slot, 2);
at!(FxProp, flags, 3);
at!(FxProp, name, 8);
at!(FxProp, min, 16);
at!(FxProp, max, 24);
at!(FxProp, def, 32);
at!(FxProp, choices, 40);
// jfgui.inc: SF_*
size!(Surface, 32);
at!(Surface, px, 0);
at!(Surface, w, 8);
at!(Surface, h, 12);
at!(Surface, stride, 16);
at!(Surface, fmt, 20);
at!(Surface, flags, 21);
at!(Surface, opac, 22);
at!(Surface, blend, 23);
at!(Surface, aux, 24);
// thread.inc: JOB_*
size!(Job, 80);
at!(Job, ctx, 8);
at!(Job, count, 16);
at!(Job, user, 40);
at!(Job, cancel, 48);
at!(Job, state, 52);
at!(Job, link, 64);
at!(Job, rsv, 72);
// img.inc: ID_*, IM_*, IO_*
size!(ImgDec, 48);
at!(ImgDec, flags, 0);
at!(ImgDec, frame, 4);
at!(ImgDec, hint, 8);
at!(ImgDec, maxpx, 16);
at!(ImgDec, meta, 24);
at!(ImgDec, cancel, 32);
at!(ImgDec, maxmem, 40);
size!(ImgMeta, 128);
at!(ImgMeta, w, 4);
at!(ImgMeta, h, 8);
at!(ImgMeta, bits, 12);
at!(ImgMeta, model, 13);
at!(ImgMeta, orient, 14);
at!(ImgMeta, channels, 15);
at!(ImgMeta, flags, 16);
at!(ImgMeta, frames, 20);
at!(ImgMeta, dpix, 24);
at!(ImgMeta, cicp, 32);
at!(ImgMeta, gamma, 36);
at!(ImgMeta, icc, 40);
at!(ImgMeta, icclen, 48);
at!(ImgMeta, exif, 56);
at!(ImgMeta, xmp, 72);
at!(ImgMeta, delay, 88);
at!(ImgMeta, loops, 92);
at!(ImgMeta, quality, 96);
at!(ImgMeta, resunit, 100);
at!(ImgMeta, chrm, 104);
at!(ImgMeta, dxgi, 120);
at!(ImgMeta, texfl, 122);
at!(ImgMeta, texn, 124);
size!(ImgOpt, 64);
at!(ImgOpt, quality, 4);
at!(ImgOpt, flags, 8);
at!(ImgOpt, effort, 32);
at!(ImgOpt, tiffcomp, 44);
at!(ImgOpt, meta, 48);
at!(ImgOpt, cancel, 56);
