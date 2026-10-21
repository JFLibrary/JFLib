// modplat -- the platform layer of a loadable module (jxl-module): the part of `plat` the
// module's library crates call, through the host (modules/modrt: host())
//                                                                       [jxl-module: no asm]
//
// The module build passes this crate as `plat`:
//
//   parallel_for(count, &f) -> Ok / Err(Cancelled)   f(index, job) on the HOST's worker pool
//                              (HostTable::parallel_for: the workers and the calling thread;
//                              nesting allowed) -- the same loop plat's pool runs in an app
//   JobCtl                     cancel(), cancelled() of the running loop
//   thr_count(), thr_self()    the host pool's (per-thread arrays of thr_count() + 1 entries)
//   jxlenc_* / JxlEncReq       img_jxl::writer's module path names: inside a module there is
//                              no module to load (jxlenc_ready() = false)
//
// One of the crates that may contain `unsafe` (it is the module's plat).
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::AtomicU32;

use rt::host;

/// a loop ended through cancel()
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// the running loop as its items see it (the host's job)
pub struct JobCtl {
    job: *const c_void,
}

impl JobCtl {
    /// skip the remaining items
    pub fn cancel(&self) {
        (host().job_cancel)(self.job)
    }
    /// cancel() was called
    pub fn cancelled(&self) -> bool {
        (host().job_cancelled)(self.job) != 0
    }
}

type Body<'a> = &'a (dyn Fn(usize, &JobCtl) + Sync);

extern "win64" fn item(ctx: *const c_void, index: usize, job: *const c_void) {
    // SAFETY: ctx is the address of the loop body reference of parallel_for below, which
    // lives until the host's parallel_for returned (after the last item).
    let f: Body = unsafe { *(ctx as *const Body) };
    f(index, &JobCtl { job })
}

/// f(index, job) for every index in 0..count on the host's worker pool and this thread
pub fn parallel_for(count: usize, f: &(dyn Fn(usize, &JobCtl) + Sync)) -> Result<(), Cancelled> {
    let body: Body = f;
    let r = (host().parallel_for)(count, &body as *const Body as *const c_void, item);
    if r == 0 { Ok(()) } else { Err(Cancelled) }
}

/// the host pool's workers
pub fn thr_count() -> u32 {
    (host().thr_count)()
}

/// 0..n-1 on worker k of the host's pool, n on any other thread
pub fn thr_self() -> usize {
    (host().thr_self)()
}

/// (img_jxl::writer) the module path: no module inside a module
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

/// (img_jxl::writer) nothing to set inside a module
pub fn jxlenc_set_path(_path: Option<String>) {}

/// (img_jxl::writer) no module inside a module
pub fn jxlenc_ready() -> bool {
    false
}

/// (img_jxl::writer) no module inside a module
pub fn jxlenc_encode(_req: JxlEncReq) -> Option<Result<Vec<u8>, i32>> {
    None
}
