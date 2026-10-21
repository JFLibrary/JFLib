// modabi -- the ABI of loadable modules (jxl-module): the records the host (modhost.rs) and a
// module (the module runtime modules/modrt, which includes this file) exchange
//                                                                       [jxl-module: no asm]
//
// A module is a no_std cdylib linked against nothing (no imports; built and checked like a
// Rust PaintCraft plugin, build_rs.py section jxl-module) that exports exactly one function
//
//     <name>_module(host: *const HostTable) -> *const ModTable          extern "win64"
//
// (`jxlenc_module` for the JPEG XL writer).  The host keeps the module loaded for the rest of
// the process; the module keeps the HostTable pointer.  Null = the module refuses the host
// (another MOD_VERSION).  Every function pointer is extern "win64" on both systems (as the
// plugin ABI), so the records are the same on Windows and Linux.
//
// Memory: the module's global allocator is the host's (HostTable::alloc / free = the host's
// GlobalAlloc with the same Layout), so blocks move between both sides as Vec<u8> raw parts
// (JxlReq::rgba to the module, JxlRes to the host) without a copy.
// Threads: parallel_for runs item(ctx, index, job) on the host's worker pool (and the calling
// thread); thr_count / thr_self are the pool's, so per-thread arrays of the module work as in
// the host.  A panic of the module ends in HostTable::panic: the host's normal crash report
// (stderr "panic: jxlenc: ...", the crash hook, exit code 3); a fault inside the module code
// goes to the host's crash handlers like any other.
//
// Version 1.  Layouts are asserted below.

use core::ffi::c_void;

/// ModTable::magic ("MODM" little endian)
pub const MOD_MAGIC: u32 = 0x4D44_4F4D;
/// the ABI version of this file
pub const MOD_VERSION: u32 = 1;
/// ModTable::kind of the JPEG XL writer
pub const MOD_JXLENC: u32 = 1;

/// JxlReq::blobs: which of the blobs are present (None vs Some)
pub const JXB_ICC: u32 = 1;
pub const JXB_EXIF: u32 = 2;
pub const JXB_XMP: u32 = 4;

/// JxlReq::test: crash tests of the module (plat::jxlenc_*: jfgui_JXLENC_TEST=panic / fault)
pub const JXT_PANIC: u32 = 1;
pub const JXT_FAULT: u32 = 2;

/// one item of a parallel loop: item(ctx, index, job)
pub type ItemFn = extern "win64" fn(ctx: *const c_void, index: usize, job: *const c_void);

/// the host's functions (the module keeps the pointer)
#[repr(C)]
pub struct HostTable {
    pub size: u32,    // size_of::<HostTable>()
    pub version: u32, // MOD_VERSION
    pub cpu: u32,     // the host's rt::cpu::features() (--simd sse2: 0, the module then uses SSE2 too)
    pub threads: u32, // workers of the host's pool
    /// a block of `size` bytes aligned to `align` from the host's allocator (zeroed != 0:
    /// cleared), null when out of memory
    pub alloc: extern "win64" fn(size: usize, align: usize, zeroed: u32) -> *mut u8,
    /// free a block of alloc with its size and alignment
    pub free: extern "win64" fn(p: *mut u8, size: usize, align: usize),
    /// item(ctx, i, job) for i in 0..count on the pool; 0 = done, 1 = cancelled
    pub parallel_for: extern "win64" fn(count: usize, ctx: *const c_void, item: ItemFn) -> u32,
    /// job.cancel() of the job an item got
    pub job_cancel: extern "win64" fn(job: *const c_void),
    /// job.cancelled()
    pub job_cancelled: extern "win64" fn(job: *const c_void) -> u32,
    /// the pool's thr_count() / thr_self()
    pub thr_count: extern "win64" fn() -> u32,
    pub thr_self: extern "win64" fn() -> usize,
    /// give the time slice away (spin locks)
    pub yield_now: extern "win64" fn(),
    /// one line on stderr ("<module>: msg")
    pub log: extern "win64" fn(msg: *const u8, len: usize),
    /// a panic of the module (UTF-8 message): the host's crash report, exit code 3
    pub panic: extern "win64" fn(msg: *const u8, len: usize) -> !,
}

/// what a module offers
#[repr(C)]
pub struct ModTable {
    pub size: u32,    // size_of::<ModTable>()
    pub version: u32, // MOD_VERSION
    pub magic: u32,   // MOD_MAGIC
    pub kind: u32,    // MOD_JXLENC
    /// the JPEG XL writer: 0 and res filled, or ERR_* (res empty)
    pub jxl_encode: extern "win64" fn(req: *const JxlReq, res: *mut JxlRes) -> i32,
}

/// a JPEG XL encode: the samples of img_jxl::writer::image_of and the writer's options
#[repr(C)]
pub struct JxlReq {
    pub size: u32, // size_of::<JxlReq>()
    pub w: u32,
    pub h: u32,
    pub gray: u32,  // Px::gray
    pub alpha: u32, // Px::alpha
    pub effort: u32,
    pub quality: u32,
    pub flags: u32, // IO_FLAGS
    pub blobs: u32, // JXB_*: which blobs are present
    pub test: u32,  // JXT_* (0)
    /// Px::rgba (R, G, B, A per pixel) as Vec raw parts: the module owns it from now on
    pub rgba: *mut u8,
    pub rgba_len: usize,
    pub rgba_cap: usize,
    pub icc: *const u8,
    pub icc_len: usize,
    pub exif: *const u8,
    pub exif_len: usize,
    pub xmp: *const u8,
    pub xmp_len: usize,
    /// the cancel flag (an AtomicU32), null = none
    pub cancel: *const u32,
}

/// the file the writer made, as Vec raw parts (the host owns it from now on)
#[repr(C)]
pub struct JxlRes {
    pub data: *mut u8,
    pub len: usize,
    pub cap: usize,
}

const _: () = assert!(core::mem::size_of::<HostTable>() == 16 + 10 * 8);
const _: () = assert!(core::mem::size_of::<ModTable>() == 24);
const _: () = assert!(core::mem::offset_of!(ModTable, jxl_encode) == 16);
const _: () = assert!(core::mem::size_of::<JxlReq>() == 40 + 10 * 8);
const _: () = assert!(core::mem::offset_of!(JxlReq, rgba) == 40);
const _: () = assert!(core::mem::size_of::<JxlRes>() == 24);
