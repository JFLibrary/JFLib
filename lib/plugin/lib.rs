// plugin -- the PaintCraft plugin ABI for plugins written in Rust (no_std cdylib, no imports)
//                     [ASMLIB/inc/plugin.inc; docs apps/PaintCraft/docs/PLUGINS.md, plugins/README.md]
//
// A plugin is a library exporting one function, pn_plugin_query(PNAPI*) -> PLUGININFO*, that
// PaintCraft (the Rust and the assembly build alike) loads from <root>/Effects, FileTypes or
// Shapes.  It is linked against nothing: no rt, no plat, no system library -- the host's
// functions come through the PNAPI table.  This crate is all a plugin depends on (+ core):
//
//   abi       the records of plugin.inc / fx.inc / img.inc as #[repr(C)] types (layouts
//             asserted): PnApi, PluginInfo, FxDesc, FxProp, FtDesc, ShapeDesc, FxArgs,
//             Surface, Job, ImgDec, ImgMeta, ImgOpt; their const constructors
//   api       the host table after pn_plugin_query (api()): Block (a PA_ALLOC buffer),
//             parallel_for, hash32, log / log!, nthreads; PnAlloc (opt-in global allocator)
//   fx        Effect (render / prepare of an effect or adjustment) and its safe view Fx
//             (source rows, the destination rows of the ROI, parameters, cancel polling)
//   ft        FileType (probe / decode / encode of a file type): Decoder, Image
//   mem       memcpy / memmove / memset / memcmp / bcmp (and the MSVC support symbols) for
//             core, in assembly so that they never become exports
//
// The unsafe parts (raw host records, the trampolines the host calls) live here; a plugin's
// own code stays safe: `#![deny(unsafe_code)]`, with entry!() as its one exception (the
// #[unsafe(no_mangle)] export).  Minimal plugin (see plugins/sepiastr):
//
//   #![no_std]
//   #![deny(unsafe_code)]
//   use plugin::*;
//   struct Sepia;
//   impl Effect for Sepia {
//       type Aux = ();
//       fn render(fx: &mut Fx, _: Option<&()>) -> Result<(), i32> { ... }
//   }
//   static PROPS: [FxProp; 1] = [FxProp::int(0, c"Stärke", 0.0, 100.0, 60.0)];
//   static FX: FxDesc = FxDesc::new::<Sepia>(c"Sepia-Stärke", FXF_ADJ, FXC_ADJUST, &PROPS);
//   static FX_LIST: [&FxDesc; 1] = [&FX];
//   static INFO: PluginInfo = PluginInfo::new(c"Sepia-Stärke (Beispiel)", c"1.0").effects(&FX_LIST);
//   fn query(_: &'static PnApi) -> Option<&'static PluginInfo> { Some(&INFO) }
//   plugin::entry!(query);
//   #[panic_handler]
//   fn panic(info: &core::panic::PanicInfo) -> ! { plugin::panic(info) }
//
// Panics: `plugin::panic` logs "panic at FILE:LINE" through PA_LOG and executes ud2,
// `plugin::trap` only executes ud2 -- the host's crash guard catches it and disables the plugin
// ("Absturz in render (ungültiger Befehl ...)"), like any other fault inside plugin code.
//
// Build: python build_rs.py plugin [NAME] (section rust-plugin: cdylib, panic=abort, fat LTO,
// linked against nothing, checked like PaintCraft's loader checks it).

#![no_std]

pub mod abi;
pub mod api;
pub mod ft;
pub mod fx;
mod mem;

pub use abi::*;
pub use api::{Block, PnAlloc, api, hash32, log, log_fmt, nthreads, parallel_for};
pub use ft::{Decoder, FileType, Image};
pub use fx::{Effect, Fx};

// ------------------------------------------------------------------ plugin.inc
pub const PNAPI_VERSION: u32 = 1; // the PNAPI version this crate is written for
pub const PLUGIN_MAGIC: u32 = 0x4950_4E50; // "PNPI" little endian
pub const PA_SIZE_V1: u32 = 96; // PNAPI table size of version 1
pub const PI_SIZE_V1: u32 = 88; // PLUGININFO size of version 1

// ------------------------------------------------------------------ error codes (jfgui.inc)
pub const ERR_FAIL: i32 = -1;
pub const ERR_TRUNC: i32 = -1; // input ends early
pub const ERR_HDR: i32 = -2; // bad header / signature
pub const ERR_NOMEM: i32 = -8; // allocation failed
pub const ERR_UNSUPPORTED: i32 = -9; // valid but unsupported variant
pub const ERR_DIMS: i32 = -12; // width / height out of range
pub const ERR_IO: i32 = -13;
pub const ERR_CANCEL: i32 = -14; // cancelled through a cancel flag
pub const ERR_LIMIT: i32 = -15; // resource limit
pub const ERR_FAULT: i32 = -64; // a guarded call faulted (the plugin is disabled)

// ------------------------------------------------------------------ fx.inc
pub const FX_NSLOTS: usize = 32; // parameter slots per effect
pub const FXF_ADJ: u32 = 1; // adjustment (Korrekturen) rather than effect (Effekte)
pub const FXF_PREPARE: u32 = 2; // has a prepare step
pub const FXF_SEED: u32 = 4; // uses a random seed (has an FP_SEED property)
pub const FXF_BANDS: u32 = 8; // render wants full-width row bands
pub const FXF_WHOLE: u32 = 16; // render must get the whole image in one call
pub const FXF_NO_DIALOG: u32 = 32; // applied at once, without the property dialog
pub const FXF_SLOW: u32 = 64; // hint: show progress
pub const FXC_ADJUST: u32 = 0; // categories (FXD_CATEGORY)
pub const FXC_ARTISTIC: u32 = 1;
pub const FXC_BLUR: u32 = 2;
pub const FXC_COLOR: u32 = 3;
pub const FXC_DISTORT: u32 = 4;
pub const FXC_NOISE: u32 = 5;
pub const FXC_OBJECT: u32 = 6;
pub const FXC_PHOTO: u32 = 7;
pub const FXC_RENDER: u32 = 8;
pub const FXC_STYLIZE: u32 = 9;
pub const FP_INT: u8 = 1; // property types: slot i64
pub const FP_DEC: u8 = 2; // f64
pub const FP_BOOL: u8 = 3; // i64 0 / 1
pub const FP_CHOICE: u8 = 4; // i64 index
pub const FP_ANGLE: u8 = 5; // f64 degrees
pub const FP_PAN: u8 = 6; // two slots: f64 x, f64 y (-1..1)
pub const FP_COLOR: u8 = 7; // straight ARGB in the low half
pub const FP_SEED: u8 = 8; // i64 seed
pub const FPF_LOG: u8 = 1; // the slider maps logarithmically

// ------------------------------------------------------------------ img.inc / jfgui.inc
pub const FMT_PARGB: u8 = 1; // premultiplied 0xAARRGGBB
pub const FMT_ARGB: u8 = 3; // straight 0xAARRGGBB (bytes B, G, R, A)
pub const IMG_MAXDIM: u32 = 65535; // largest width / height of a decoded image
pub const IMGD_STRAIGHT: u32 = 1; // decode flags (ImgDec::flags)
pub const IMGD_ICC: u32 = 2;
pub const IMGD_EXIF: u32 = 4;
pub const IMGD_XMP: u32 = 8;
pub const IMGD_NO_ORIENT: u32 = 16;
pub const IMGD_PROBE: u32 = 32; // header + metadata only: pixels optional
pub const IMGD_THUMB: u32 = 64;
pub const IMM_GRAY: u8 = 1; // colour models (ImgMeta::model)
pub const IMM_RGB: u8 = 2;
pub const IMM_PAL: u8 = 3;
pub const IMM_CMYK: u8 = 4;
pub const IMM_YCC: u8 = 5;
pub const IMF_ALPHA: u32 = 1; // ImgMeta::flags
pub const IMF_SRC_PREMUL: u32 = 2;
pub const IMF_LOSSLESS: u32 = 4;
pub const IMF_ANIMATED: u32 = 8;
pub const IMF_HDR: u32 = 16;
pub const IFC_ALPHA: u32 = 4; // file type capabilities (FtDesc::caps)
pub const IFC_ANIM: u32 = 16;
pub const IFC_PAGES: u32 = 32;
pub const IFC_LOSSY: u32 = 128;
pub const IFC_LOSSLESS: u32 = 256;
pub const IFC_HDR: u32 = 512;

/// export pn_plugin_query(PNAPI*) -> PLUGININFO* | 0, calling `query` with the host's table
/// (kept for api() when it is a complete version-1 table).  `query: fn(&'static PnApi) ->
/// Option<&'static PluginInfo>`; None refuses loading.  The plugin's one unsafe item.
#[macro_export]
macro_rules! entry {
    ($query:path) => {
        /// pn_plugin_query(PNAPI*) -> PLUGININFO* | 0 -- the plugin's only export
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "win64" fn pn_plugin_query(api: *const $crate::PnApi) -> *const $crate::PluginInfo {
            $crate::api::query_entry(api, $query)
        }
    };
}

/// the panic handler of a plugin: "panic at FILE:LINE" into the plugin log when the host
/// table is known, then a fault the host's guard catches (the plugin is disabled).  The
/// message itself is left out: formatting it would link core::fmt into every plugin (+6 KB);
/// a handler of your own can log it with log!("{}", info.message()).  (Inlined, so that the
/// optimizer sees that the message is never read and drops it.)
#[inline(always)]
pub fn panic(info: &core::panic::PanicInfo) -> ! {
    match info.location() {
        Some(l) => api::log_panic(l.file(), l.line()),
        None => api::log_panic("?", 0),
    }
    trap()
}

/// an illegal instruction (ud2): the host's crash guard disables the plugin
pub fn trap() -> ! {
    // SAFETY: ud2 raises #UD and never returns; no memory or stack is touched.
    unsafe { core::arch::asm!("ud2", options(noreturn, nomem, nostack)) }
}
