// modrt -- the runtime of a loadable module (jxl-module): what `rt` is for an app, for a no_std
// cdylib without imports that runs inside a host process (PaintCraft loads modules/jxlenc)
//                                                                       [jxl-module: no asm]
//
// The module build (build_rs.py section jxl-module) passes this crate as `rt` (and
// modules/modplat as `plat`) to every library crate the module contains (simd, gfx,
// img_core, img_jpeg, img_jxl, img_jxl_enc), so their unchanged sources compile against it:
//
//   cpu, crash, fx, sync, x87   the files of lib/rt, included as they are (the same SSE /
//                               x87 float semantics, the same CPU feature bits, spin lock)
//   os                          what rt::os is, through the host: yield_now, exit (a panic),
//                               write_stdout / write_stderr (the host's log)
//   modabi                      lib/plat/modabi.rs: the host / module records
//   host()                      the host's table (set by the entry)
//   module_entry!(name, TABLE)  the module's one export: name(&HostTable) -> &TABLE
//   jxl_table::<W>()            a ModTable whose jxl_encode runs W::encode (JxlWriter) on a
//                               safe JxlRequest (the raw JxlReq unpacked here)
//   (global allocator)          the host's allocator (HostTable::alloc / free): blocks move
//                               between host and module as Vec raw parts
//   (panic handler)             the message to HostTable::panic: the host's crash report
//   (memcpy, memset, ...)       lib/plugin/mem.rs (assembly: never an export)
//
// One of the crates that may contain `unsafe` (it is the module's rt); a module's own code is
// `#![deny(unsafe_code)]` with module_entry!() as its one exception (as plugin::entry!).
#![no_std]

extern crate alloc;

#[path = "../../lib/rt/cpu.rs"]
pub mod cpu;
#[path = "../../lib/rt/crash.rs"]
pub mod crash;
#[path = "../../lib/rt/fx.rs"]
pub mod fx;
#[path = "../../lib/rt/portable/fx.rs"]
pub mod scalar_fx;
/// Portable float contract used by the common scalar SIMD backend in module builds.
pub mod portable {
    /// Explicit scalar rounding/conversion operations shared with the application runtime.
    pub use crate::scalar_fx as fx;
}
#[path = "../../lib/rt/sync.rs"]
pub mod sync;
#[path = "../../lib/rt/x87.rs"]
pub mod x87;
#[path = "../../lib/plat/modabi.rs"]
pub mod modabi;
#[path = "../../lib/plugin/mem.rs"]
mod mem;

use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use modabi::*;

static HOST: AtomicPtr<HostTable> = AtomicPtr::new(core::ptr::null_mut());

/// the host's table (a module runs only after its entry stored it; before: a fault)
#[inline]
pub fn host() -> &'static HostTable {
    let p = HOST.load(Ordering::Acquire);
    if p.is_null() {
        trap();
    }
    // SAFETY: the entry stored a pointer to the host's table, which lives as long as the
    // process (the host leaks it on purpose).
    unsafe { &*p }
}

/// ud2: a fault the host's crash handler reports
pub fn trap() -> ! {
    // SAFETY: an illegal instruction, nothing else.
    unsafe { core::arch::asm!("ud2", options(noreturn, nomem, nostack)) }
}

/// the OS calls rt makes, through the host
pub mod os {
    /// give the time slice away (the host's yield)
    pub fn yield_now() {
        (crate::host().yield_now)()
    }

    /// end the process: as a panic of the module (rt calls it only on its crash path)
    pub fn exit(code: i32) -> ! {
        let mut s = crate::crash::StackText::<64>::new();
        let _ = core::fmt::Write::write_fmt(&mut s, format_args!("exit({code}) inside the module"));
        (crate::host().panic)(s.bytes().as_ptr(), s.bytes().len())
    }

    /// a line for the host's log
    pub fn write_stdout(b: &[u8]) {
        let b = b.strip_suffix(b"\n").unwrap_or(b);
        (crate::host().log)(b.as_ptr(), b.len())
    }

    /// a line for the host's log
    pub fn write_stderr(b: &[u8]) {
        write_stdout(b)
    }
}

/// the host's allocator
struct HostAlloc;

unsafe impl GlobalAlloc for HostAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        (host().alloc)(l.size(), l.align(), 0)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        (host().alloc)(l.size(), l.align(), 1)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        (host().free)(p, l.size(), l.align())
    }
}

#[global_allocator]
static ALLOC: HostAlloc = HostAlloc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut s = crash::StackText::<1024>::new(); // no allocation on the crash path
    let _ = write!(s, "{info}");
    let p = HOST.load(Ordering::Acquire);
    if p.is_null() {
        trap();
    }
    // SAFETY: as host().
    let h = unsafe { &*p };
    (h.panic)(s.bytes().as_ptr(), s.bytes().len())
}

/// the entry of module_entry!: keep the host's table (version 1 only), take its CPU features
/// (--simd sse2 in the host: SSE2 kernels here too) -> the module's table, or null
pub fn entry(h: *const HostTable, tab: &'static ModTable) -> *const ModTable {
    if h.is_null() {
        return core::ptr::null();
    }
    // SAFETY: the host passes its table (read once here, then kept).
    let t = unsafe { &*h };
    if t.version != MOD_VERSION || (t.size as usize) < core::mem::size_of::<HostTable>() {
        return core::ptr::null();
    }
    HOST.store(h as *mut HostTable, Ordering::Release);
    if t.cpu & cpu::CPU_AVX2 == 0 && cpu::features() & cpu::CPU_AVX2 != 0 {
        cpu::force_baseline();
    }
    tab
}

/// the module's one export: `name(&HostTable) -> &TABLE` (extern "win64", #[no_mangle])
#[macro_export]
macro_rules! module_entry {
    ($name:ident, $table:path) => {
        /// the module's entry (lib/plat/modabi.rs)
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "win64" fn $name(host: *const $crate::modabi::HostTable) -> *const $crate::modabi::ModTable {
            $crate::entry(host, &$table)
        }
    };
}

/// a JPEG XL encode as the module's writer gets it (JxlReq unpacked)
pub struct JxlRequest<'a> {
    pub w: u32,
    pub h: u32,
    pub gray: bool,
    pub alpha: bool,
    pub rgba: Vec<u8>, // owned (the host's allocation)
    pub effort: u32,
    pub quality: u32,
    pub flags: u32,
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
    pub cancel: Option<&'a AtomicU32>,
}

/// the writer of a jxlenc module
pub trait JxlWriter {
    /// the file, or ERR_*
    fn encode(req: JxlRequest<'_>) -> Result<Vec<u8>, i32>;
}

/// the ModTable of a jxlenc module with the writer W
pub const fn jxl_table<W: JxlWriter>() -> ModTable {
    ModTable {
        size: core::mem::size_of::<ModTable>() as u32,
        version: MOD_VERSION,
        magic: MOD_MAGIC,
        kind: MOD_JXLENC,
        jxl_encode: jxl_encode::<W>,
    }
}

/// a blob of the request (None when its JXB_* bit is clear)
///
/// # Safety
/// p, n describe a buffer that lives as long as the request (or n == 0).
unsafe fn blob<'a>(on: bool, p: *const u8, n: usize) -> Option<&'a [u8]> {
    if !on {
        return None;
    }
    if p.is_null() || n == 0 {
        return Some(&[]);
    }
    // SAFETY: the caller's contract.
    Some(unsafe { core::slice::from_raw_parts(p, n) })
}

const ERR_HDR: i32 = -2;

extern "win64" fn jxl_encode<W: JxlWriter>(req: *const JxlReq, res: *mut JxlRes) -> i32 {
    if req.is_null() || res.is_null() {
        return ERR_HDR;
    }
    // SAFETY: the host passes its request record for the time of the call.
    let r = unsafe { &*req };
    if r.size as usize != core::mem::size_of::<JxlReq>() || r.rgba.is_null() || r.rgba_len > r.rgba_cap {
        return ERR_HDR;
    }
    // SAFETY: the host hands over its Vec<u8> of the samples (raw parts); its blocks come from
    // the host's allocator, which is this module's global allocator (same layout rules).
    let rgba = unsafe { Vec::from_raw_parts(r.rgba, r.rgba_len, r.rgba_cap) };
    match r.test {
        JXT_PANIC => panic!("test panic inside the module (jfgui_JXLENC_TEST=panic)"),
        // SAFETY: a deliberate 4-byte write to address 0x10 (the crash tests of the host).
        JXT_FAULT => unsafe { core::arch::asm!("mov dword ptr [{0}], {1:e}", in(reg) 0x10usize, in(reg) 16u32, options(nostack)) },
        _ => {}
    }
    // SAFETY: the blobs and the cancel flag live in the host during the call.
    let q = unsafe {
        JxlRequest {
            w: r.w,
            h: r.h,
            gray: r.gray != 0,
            alpha: r.alpha != 0,
            rgba,
            effort: r.effort,
            quality: r.quality,
            flags: r.flags,
            icc: blob(r.blobs & JXB_ICC != 0, r.icc, r.icc_len),
            exif: blob(r.blobs & JXB_EXIF != 0, r.exif, r.exif_len),
            xmp: blob(r.blobs & JXB_XMP != 0, r.xmp, r.xmp_len),
            cancel: (r.cancel as *const AtomicU32).as_ref(),
        }
    };
    match W::encode(q) {
        Ok(v) => {
            let mut v = ManuallyDrop::new(v); // the host owns it now
            // SAFETY: the host's result record, written once.
            unsafe { *res = JxlRes { data: v.as_mut_ptr(), len: v.len(), cap: v.capacity() } };
            0
        }
        Err(e) => e,
    }
}
