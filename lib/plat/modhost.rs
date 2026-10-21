// modhost -- loadable modules, the host side (jxl-module): the JPEG XL writer module jxlenc
//                                                                       [jxl-module: no asm]
//
//   jxlenc_set_path(Some(path) | None)   the module file: None = <exe folder>/jxlenc.dll
//                              (Linux jxlenc.so); "off" = none.  Before the first use.
//   jxlenc_ready() -> bool     the module is loaded (loads it on the first call and keeps it;
//                              a missing / refused file stays missing for the process)
//   jxlenc_file() -> path      the file the module was (or would be) loaded from
//   jxlenc_encode(req) -> Some(file / ERR_*) | None (no module)
//   module_at(addr) -> Some((file name, image base))   the loaded module holding addr (crash
//                              reports: offsets into its .map; allocation-free)
//
// The module (modules/jxlenc) is a no_std cdylib without imports, loaded with the plugin
// loader (lib_open / lib_sym: win_lib.rs, linux_elfload.rs -- the same checks), its one export
// jxlenc_module(&HostTable) -> &ModTable (modabi.rs: the records, the memory and thread rules).
// It is our own code, called without a guard: a fault inside it is a crash with the normal
// report, a panic comes back through HostTable::panic into rt::crash::report.  The JPEG XL
// specifics (what JxlReq carries) are img_jxl::writer's; this file only moves the bytes.
// jfgui_JXLENC_TEST=panic | fault (tests) makes the module panic / fault in its encode.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ffi::c_void;
use core::fmt::Write;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use rt::sync::SpinLock;

#[path = "modabi.rs"]
pub mod modabi;
use modabi::*;

use crate::{JobCtl, LIBA_R, LIBA_X, Lib, lib_open, lib_seg, lib_sym, parallel_for, thr_count, thr_self};

/// the module's file name next to the exe
#[cfg(windows)]
pub const JXLENC_FILE: &str = "jxlenc.dll";
/// the module's file name next to the exe
#[cfg(not(windows))]
pub const JXLENC_FILE: &str = "jxlenc.so";

struct Loaded {
    _lib: Lib,          // kept mapped for the rest of the process
    tab: &'static ModTable,
}

struct State {
    path: Option<String>,
    tried: bool,
    m: Option<Loaded>,
}

// SAFETY: the ModTable reference points into the mapped image, which stays mapped and is
// only read; Lib holds addresses only.
unsafe impl Send for State {}

static JXL: SpinLock<State> = SpinLock::new(State { path: None, tried: false, m: None });
static JXL_BASE: AtomicUsize = AtomicUsize::new(0); // the loaded image (module_at)
static JXL_SPAN: AtomicUsize = AtomicUsize::new(0);

/// the loaded module holding addr: (its file name, its image base) -- for crash reports (no
/// lock, no allocation)
pub fn module_at(addr: u64) -> Option<(&'static str, u64)> {
    let b = JXL_BASE.load(Ordering::Acquire) as u64;
    if b != 0 && addr.wrapping_sub(b) < JXL_SPAN.load(Ordering::Acquire) as u64 { Some((JXLENC_FILE, b)) } else { None }
}

/// the module file to load (None = the default next to the exe, "off" = none); resets an
/// earlier failed attempt, never a loaded module
pub fn jxlenc_set_path(path: Option<String>) {
    let mut s = JXL.lock();
    if s.m.is_none() {
        s.path = path;
        s.tried = false;
    }
}

/// the folder of the running exe
fn exe_dir() -> Option<String> {
    let p = current_exe()?;
    let k = p.rfind(['/', '\\'])?;
    Some(String::from(&p[..k]))
}

/// Absolute path of the running native executable, independent of argv[0].
pub fn current_exe() -> Option<String> {
    #[cfg(windows)]
    { crate::inst::exe_path() }
    #[cfg(not(windows))]
    { self_exe() }
}

/// readlink("/proc/self/exe")
#[cfg(not(windows))]
fn self_exe() -> Option<String> {
    const SYS_READLINK: usize = 89;
    let mut buf = alloc::vec![0u8; 4096];
    // SAFETY: a NUL-terminated path and a buffer of buf.len() bytes.
    let n = unsafe { rt::os::syscall6(SYS_READLINK, c"/proc/self/exe".as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0) };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok()
}

/// the file the module is (or would be) loaded from; None = "off" / no exe folder
pub fn jxlenc_file() -> Option<String> {
    let s = JXL.lock();
    file_of(&s)
}

fn file_of(s: &State) -> Option<String> {
    match s.path.as_deref() {
        Some("off") => None,
        Some(p) => Some(String::from(p)),
        None => exe_dir().map(|d| alloc::format!("{d}/{JXLENC_FILE}")),
    }
}

/// the module is loaded (the first call loads it)
pub fn jxlenc_ready() -> bool {
    let mut s = JXL.lock();
    if !s.tried {
        s.tried = true;
        if let Some(f) = file_of(&s) {
            s.m = load(&f);
        }
    }
    s.m.is_some()
}

/// load and check the module file -> None (missing: silent; refused: one stderr line)
fn load(path: &str) -> Option<Loaded> {
    let lib = match lib_open(path) {
        Ok(l) => l,
        Err(e) => {
            if e != crate::LIBE_IO {
                note(path, "refused by the loader", e as usize);
            }
            return None;
        }
    };
    let entry = lib_sym(&lib, "jxlenc_module");
    if entry == 0 || lib_seg(&lib, entry).1 & LIBA_X == 0 {
        note(path, "no jxlenc_module export", 0);
        return None;
    }
    let host: &'static HostTable = Box::leak(Box::new(HostTable {
        size: core::mem::size_of::<HostTable>() as u32,
        version: MOD_VERSION,
        cpu: rt::cpu::features(),
        threads: thr_count(),
        alloc: h_alloc,
        free: h_free,
        parallel_for: h_parallel_for,
        job_cancel: h_job_cancel,
        job_cancelled: h_job_cancelled,
        thr_count: h_thr_count,
        thr_self: h_thr_self,
        yield_now: h_yield,
        log: h_log,
        panic: h_panic,
    }));
    // SAFETY: entry is the module's exported jxlenc_module (inside an executable segment of
    // the image lib_open mapped and relocated), extern "win64" fn(*const HostTable) ->
    // *const ModTable by the module ABI (modabi.rs).
    let f: extern "win64" fn(*const HostTable) -> *const ModTable = unsafe { core::mem::transmute(entry) };
    let t = f(host) as usize;
    // the table must lie inside a readable segment of the image, and its function in an
    // executable one
    let n = core::mem::size_of::<ModTable>();
    let (end, fl) = lib_seg(&lib, t);
    if t == 0 || t % 8 != 0 || fl & LIBA_R == 0 || end < t + n {
        note(path, "refused the host (another module version?)", 0);
        return None;
    }
    // SAFETY: t is an aligned ModTable-sized range inside a readable segment of the mapped
    // image (checked above); the image stays mapped as long as the process runs (Loaded).
    let tab: &'static ModTable = unsafe { &*(t as *const ModTable) };
    if tab.size as usize != n || tab.version != MOD_VERSION || tab.magic != MOD_MAGIC || tab.kind != MOD_JXLENC
        || lib_seg(&lib, tab.jxl_encode as usize).1 & LIBA_X == 0
    {
        note(path, "not a jxlenc module of this version", 0);
        return None;
    }
    JXL_SPAN.store(lib.span(), Ordering::Release);
    JXL_BASE.store(lib.base(), Ordering::Release);
    Some(Loaded { _lib: lib, tab })
}

fn note(path: &str, what: &str, code: usize) {
    let mut s = rt::crash::StackText::<600>::new();
    let _ = if code != 0 {
        writeln!(s, "jfgui: {path}: {what} (error {code}); JPEG XL is saved with the compatible writer")
    } else {
        writeln!(s, "jfgui: {path}: {what}; JPEG XL is saved with the compatible writer")
    };
    crate::write_stderr(s.bytes());
}

/// a JPEG XL encode for the module (img_jxl::writer): the samples (owned: they move to the
/// module) and the writer's options
pub struct JxlEncReq<'a> {
    pub w: u32,
    pub h: u32,
    pub gray: bool,
    pub alpha: bool,
    pub rgba: Vec<u8>,
    pub effort: u32,
    pub quality: u32,
    pub flags: u32,
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
    pub cancel: Option<&'a AtomicU32>,
}

/// the module's writer on req: Some(file / ERR_*), None without a module
pub fn jxlenc_encode(req: JxlEncReq) -> Option<Result<Vec<u8>, i32>> {
    if !jxlenc_ready() {
        return None;
    }
    let tab: &'static ModTable = JXL.lock().m.as_ref()?.tab;
    let test = match crate::getenv("jfgui_JXLENC_TEST").as_deref() {
        Some("panic") => JXT_PANIC,
        Some("fault") => JXT_FAULT,
        _ => 0,
    };
    let blob = |b: Option<&[u8]>| b.map_or((core::ptr::null(), 0), |s| (s.as_ptr(), s.len()));
    let (icc, icc_len) = blob(req.icc);
    let (exif, exif_len) = blob(req.exif);
    let (xmp, xmp_len) = blob(req.xmp);
    let mut rgba = ManuallyDrop::new(req.rgba); // the module frees it
    let r = JxlReq {
        size: core::mem::size_of::<JxlReq>() as u32,
        w: req.w,
        h: req.h,
        gray: req.gray as u32,
        alpha: req.alpha as u32,
        effort: req.effort,
        quality: req.quality,
        flags: req.flags,
        blobs: if req.icc.is_some() { JXB_ICC } else { 0 }
            | if req.exif.is_some() { JXB_EXIF } else { 0 }
            | if req.xmp.is_some() { JXB_XMP } else { 0 },
        test,
        rgba: rgba.as_mut_ptr(),
        rgba_len: rgba.len(),
        rgba_cap: rgba.capacity(),
        icc,
        icc_len,
        exif,
        exif_len,
        xmp,
        xmp_len,
        cancel: req.cancel.map_or(core::ptr::null(), |c| c.as_ptr() as *const u32),
    };
    let mut res = JxlRes { data: core::ptr::null_mut(), len: 0, cap: 0 };
    let e = (tab.jxl_encode)(&r, &mut res);
    if e != 0 {
        return Some(Err(e));
    }
    if res.data.is_null() || res.len > res.cap {
        return Some(Err(-1));
    }
    // SAFETY: the module built the file as a Vec<u8> whose blocks come from its global
    // allocator, which is h_alloc / h_free (this process's GlobalAlloc with the same layout:
    // capacity bytes, alignment 1); it handed over ownership with these raw parts.
    Some(Ok(unsafe { Vec::from_raw_parts(res.data, res.len, res.cap) }))
}

// ---------------------------------------------------------------- the host table

extern "win64" fn h_alloc(size: usize, align: usize, zeroed: u32) -> *mut u8 {
    match Layout::from_size_align(size, align) {
        // SAFETY: a valid layout of non-zero size.
        Ok(l) if size != 0 => unsafe {
            if zeroed != 0 { alloc::alloc::alloc_zeroed(l) } else { alloc::alloc::alloc(l) }
        },
        _ => core::ptr::null_mut(),
    }
}

extern "win64" fn h_free(p: *mut u8, size: usize, align: usize) {
    if p.is_null() || size == 0 {
        return;
    }
    if let Ok(l) = Layout::from_size_align(size, align) {
        // SAFETY: the module frees a block of h_alloc with the layout it allocated it with.
        unsafe { alloc::alloc::dealloc(p, l) };
    }
}

extern "win64" fn h_parallel_for(count: usize, ctx: *const c_void, item: ItemFn) -> u32 {
    let ctx = ctx as usize; // the module's loop body; valid until parallel_for returns
    let r = parallel_for(count, &|i, job: &JobCtl| item(ctx as *const c_void, i, job as *const JobCtl as *const c_void));
    r.is_err() as u32
}

extern "win64" fn h_job_cancel(job: *const c_void) {
    // SAFETY: the module passes back the job pointer h_parallel_for gave the item, during
    // that item (the job lives until parallel_for returns).
    unsafe { (*(job as *const JobCtl)).cancel() }
}

extern "win64" fn h_job_cancelled(job: *const c_void) -> u32 {
    // SAFETY: as h_job_cancel.
    unsafe { (*(job as *const JobCtl)).cancelled() as u32 }
}

extern "win64" fn h_thr_count() -> u32 {
    thr_count()
}

extern "win64" fn h_thr_self() -> usize {
    thr_self()
}

extern "win64" fn h_yield() {
    rt::os::yield_now()
}

/// the module's text (UTF-8 by the ABI; anything else shows as "?")
fn text<'a>(msg: *const u8, len: usize) -> &'a str {
    if msg.is_null() || len > 4096 {
        return "?";
    }
    // SAFETY: the module passes a buffer of len bytes that lives during the call.
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(msg, len) }).unwrap_or("?")
}

extern "win64" fn h_log(msg: *const u8, len: usize) {
    let mut s = rt::crash::StackText::<4200>::new();
    let _ = writeln!(s, "jxlenc: {}", text(msg, len));
    crate::write_stderr(s.bytes());
}

/// a panic inside the module: what rt's panic handler does for the host's own panics
extern "win64" fn h_panic(msg: *const u8, len: usize) -> ! {
    if rt::crash::busy() {
        rt::os::exit(3);
    }
    let mut s = rt::crash::StackText::<1100>::new(); // no allocation on the crash path
    let _ = writeln!(s, "panic: jxlenc: {}", text(msg, len));
    crate::write_stderr(s.bytes());
    let line = s.as_str().trim_end();
    let mut ci = rt::crash::CrashInfo::empty();
    ci.kind = rt::crash::CK_PANIC;
    ci.msg = line.strip_prefix("panic: ").unwrap_or(line);
    rt::crash::report(&ci)
}
