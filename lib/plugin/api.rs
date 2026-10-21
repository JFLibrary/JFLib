// api -- the host's PNAPI table after pn_plugin_query, and safe wrappers of its functions
//                                                  [ASMLIB/inc/plugin.inc PA_*; PLUGINS.md §2]
//
//   api() -> Option<&Api>          the table (None before pn_plugin_query, or when the host's
//                                  table is shorter than version 1)
//   Block::new(cap) -> Option<Block>      a zeroed PA_ALLOC buffer (64-byte aligned) with a
//                                  length; freed on drop, or handed to the host (into_raw:
//                                  the encoder's output)
//   parallel_for(count, &f) -> Result     f(index, job) for 0..count on the host's workers
//                                  (each item guarded by the host; nesting allowed)
//   hash32(x, y, seed, stream), nthreads(), log(msg), log_fmt(format_args!(..)), log!(..)
//   PnAlloc                        a #[global_allocator] on PA_ALLOC / PA_FREE / PA_REALLOC for
//                                  plugins that use `alloc` (opt-in: declare it in the plugin)
//
// The table pointer is written once (pn_plugin_query, UI thread) and read by every thread
// afterwards: an atomic.  Everything else here is stateless.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::fmt::Write;
use core::ptr::{NonNull, null, null_mut};
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::{ERR_FAIL, Job, PA_SIZE_V1, PNAPI_VERSION, PluginInfo, PnApi};

static API: AtomicPtr<PnApi> = AtomicPtr::new(null_mut());

/// the host table, checked to be a complete version-1 table
#[repr(transparent)]
pub struct Api(PnApi);

/// pn_plugin_query's body (entry!): keep the table, ask the plugin
#[doc(hidden)]
pub fn query_entry(api: *const PnApi, query: fn(&'static PnApi) -> Option<&'static PluginInfo>) -> *const PluginInfo {
    // SAFETY: the host passes its PNAPI table, valid for the life of the process (or 0).
    let Some(t) = (unsafe { api.as_ref() }) else { return null() };
    if t.size >= PA_SIZE_V1 && t.version >= PNAPI_VERSION {
        API.store(api as *mut PnApi, Ordering::Release);
    }
    match query(t) {
        Some(i) => i,
        None => null(),
    }
}

/// the host's functions (None before pn_plugin_query or with an incomplete table)
pub fn api() -> Option<&'static Api> {
    let p = API.load(Ordering::Acquire);
    // SAFETY: only a complete version-1 table is stored (query_entry); it lives as long as
    // the process and is never written by the host after the call; Api is repr(transparent).
    unsafe { (p as *const Api).as_ref() }
}

impl Api {
    /// the raw table (size, version, function pointers)
    pub fn raw(&self) -> &PnApi {
        &self.0
    }

    /// PA_HASH32: deterministic per-pixel random bits (fx_hash32)
    pub fn hash32(&self, x: u32, y: u32, seed: u32, stream: u32) -> u32 {
        (self.0.hash32)(x, y, seed, stream)
    }

    /// PA_NTHREADS: the host's worker threads
    pub fn nthreads(&self) -> u32 {
        (self.0.nthreads)()
    }

    /// PA_LOG: one line (UTF-8, cut at 511 bytes) into the plugin log
    pub fn log(&self, msg: &str) {
        let mut b = Line::new();
        b.push(msg.as_bytes());
        (self.0.log)(b.cstr());
    }

    /// PA_PARFOR: f(index, job) for every index 0..count on the host's worker threads and the
    /// caller -> Ok, Err(ERR_CANCEL) (the job was cancelled) or Err(ERR_FAULT) (an item
    /// crashed: the plugin is disabled).  `job` is the PNAPI job of the run (poll it in long
    /// items with Job::is_cancelled).
    pub fn parallel_for<F: Fn(usize, &Job) + Sync>(&self, count: usize, f: &F) -> Result<(), i32> {
        let r = (self.0.parfor)(item_tramp::<F>, f as *const F as *const c_void, count as u64);
        if r == 0 { Ok(()) } else { Err(r) }
    }
}

// the item function of parallel_for: ctx is the &F of the call above, alive until PA_PARFOR
// returns
extern "win64" fn item_tramp<F: Fn(usize, &Job) + Sync>(ctx: *const c_void, index: u64, job: *const Job) {
    // SAFETY: ctx is the closure passed to parallel_for (shared by all items: F is Sync); the
    // host passes its JOB record (or 0) for the duration of the item.
    let (f, job) = unsafe { (&*(ctx as *const F), job.as_ref()) };
    match job {
        Some(j) => f(index as usize, j),
        None => f(index as usize, &NO_JOB),
    }
}

static NO_JOB: Job = Job {
    func: null(),
    ctx: null(),
    count: 0,
    next: 0,
    done: 0,
    user: 0,
    cancel: core::sync::atomic::AtomicU32::new(0),
    state: 0,
    active: 0,
    flags: 0,
    link: null(),
    rsv: 0,
};

// SAFETY: a JOB record is shared with the host's worker threads; plugins only read the atomic
// cancel flag.
unsafe impl Sync for Job {}

impl Job {
    /// JOB_CANCEL is set: stop early (the host discards the result)
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) != 0
    }
}

/// parallel_for through api() (Err(ERR_FAIL) without the host table)
pub fn parallel_for<F: Fn(usize, &Job) + Sync>(count: usize, f: &F) -> Result<(), i32> {
    match api() {
        Some(a) => a.parallel_for(count, f),
        None => Err(ERR_FAIL),
    }
}

/// PA_HASH32 through api() (0 without the host table)
pub fn hash32(x: u32, y: u32, seed: u32, stream: u32) -> u32 {
    api().map_or(0, |a| a.hash32(x, y, seed, stream))
}

/// PA_NTHREADS through api() (1 without the host table)
pub fn nthreads() -> u32 {
    api().map_or(1, |a| a.nthreads())
}

/// PA_LOG through api() (nothing without the host table)
pub fn log(msg: &str) {
    if let Some(a) = api() {
        a.log(msg);
    }
}

/// a formatted line into the plugin log: log_fmt(format_args!("{} px", n)) (no allocation)
pub fn log_fmt(args: core::fmt::Arguments) {
    if let Some(a) = api() {
        let mut b = Line::new();
        let _ = b.write_fmt(args);
        (a.0.log)(b.cstr());
    }
}

/// "panic at FILE:LINE" into the plugin log (without core::fmt; out of line, so that the panic
/// handler stays small enough to be inlined)
#[inline(never)]
#[cold]
pub(crate) fn log_panic(file: &str, line: u32) {
    if let Some(a) = api() {
        let mut b = Line::new();
        b.push(b"panic at ");
        b.push(file.as_bytes());
        b.push(b":");
        // the line number, without anything that could panic again
        let mut d = [0u8; 10];
        let (mut v, mut k) = (line, d.len());
        while let Some(c) = k.checked_sub(1).and_then(|i| d.get_mut(i)) {
            *c = b'0' + (v % 10) as u8;
            k -= 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        b.push(d.get(k..).unwrap_or(&[]));
        (a.0.log)(b.cstr());
    }
}

/// log!("fmt", args..) -- log_fmt(format_args!(..))
#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log_fmt(format_args!($($t)*)) };
}

/// a NUL-terminated line of at most LINE_CAP - 1 bytes on the stack (cut at a character
/// boundary)
struct Line {
    b: [u8; LINE_CAP],
    n: usize,
}

const LINE_CAP: usize = 512;

impl Line {
    fn new() -> Line {
        Line { b: [0; LINE_CAP], n: 0 }
    }

    // (written without index panics: the panic handler uses it)
    fn push(&mut self, s: &[u8]) {
        for &c in s {
            if self.n >= LINE_CAP - 1 {
                // drop a cut UTF-8 sequence
                while self.n > 0 && self.b.get(self.n - 1).is_some_and(|&x| x & 0xC0 == 0x80) {
                    self.n -= 1;
                }
                if self.n > 0 && self.b.get(self.n - 1).is_some_and(|&x| x >= 0xC0) {
                    self.n -= 1;
                }
                return;
            }
            if let Some(x) = self.b.get_mut(self.n) {
                *x = if c == 0 { b' ' } else { c };
            }
            self.n += 1;
        }
    }

    fn cstr(&mut self) -> *const u8 {
        self.n = self.n.min(LINE_CAP - 1);
        if let Some(x) = self.b.get_mut(self.n) {
            *x = 0;
        }
        self.b.as_ptr()
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

// ------------------------------------------------------------------ Block

/// a byte buffer in host memory (PA_ALLOC: zeroed, 64-byte aligned): capacity fixed at
/// creation, a length that grows with push / extend.  Dropping it frees it (PA_FREE);
/// into_raw hands it to the host (an encoder's output, which the host frees).
pub struct Block {
    p: NonNull<u8>,
    cap: usize,
    len: usize,
}

// SAFETY: the block is owned memory like a Box<[u8]>; PA_FREE is thread safe.
unsafe impl Send for Block {}
// SAFETY: shared access only reads.
unsafe impl Sync for Block {}

impl Block {
    /// a zeroed block of cap bytes (None: no host table / out of memory)
    pub fn new(cap: usize) -> Option<Block> {
        let a = api()?;
        if cap > isize::MAX as usize {
            return None;
        }
        let p = NonNull::new((a.0.alloc)(cap.max(1)))?;
        Some(Block { p, cap, len: 0 })
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// the whole capacity (zeroed where nothing was written)
    pub fn buf(&mut self) -> &mut [u8] {
        // SAFETY: cap bytes of an owned host block (zeroed when allocated).
        unsafe { core::slice::from_raw_parts_mut(self.p.as_ptr(), self.cap) }
    }

    /// the first len bytes
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: len <= cap bytes of an owned host block.
        unsafe { core::slice::from_raw_parts(self.p.as_ptr(), self.len) }
    }

    /// set the length (n <= capacity, else a panic)
    pub fn set_len(&mut self, n: usize) {
        assert!(n <= self.cap, "Block::set_len beyond the capacity");
        self.len = n;
    }

    /// append one byte (a panic when full)
    pub fn push(&mut self, b: u8) {
        let n = self.len;
        self.buf()[n] = b;
        self.len = n + 1;
    }

    /// append bytes (a panic when they do not fit)
    pub fn extend(&mut self, s: &[u8]) {
        let n = self.len;
        self.buf()[n..n + s.len()].copy_from_slice(s);
        self.len = n + s.len();
    }

    /// hand the block to the host: (pointer, length); the host frees it
    pub fn into_raw(self) -> (*mut u8, usize) {
        let r = (self.p.as_ptr(), self.len);
        core::mem::forget(self);
        r
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        if let Some(a) = api() {
            (a.0.free)(self.p.as_ptr());
        }
    }
}

// ------------------------------------------------------------------ global allocator

/// a global allocator on the host's PA_ALLOC / PA_FREE / PA_REALLOC (alignment up to 64; thread
/// safe).  Opt-in for plugins that use the alloc crate:
///   extern crate alloc;
///   #[global_allocator]
///   static A: plugin::PnAlloc = plugin::PnAlloc;
/// Allocations before pn_plugin_query fail (null).
pub struct PnAlloc;

// SAFETY: PA_ALLOC returns zeroed, 64-byte aligned, distinct blocks of at least the size asked
// for (or null); PA_FREE / PA_REALLOC take those blocks back.  Larger alignments are refused.
unsafe impl GlobalAlloc for PnAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        match api() {
            Some(a) if l.align() <= 64 => (a.0.alloc)(l.size().max(1)),
            _ => null_mut(),
        }
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        // SAFETY: PA_ALLOC blocks are zeroed.
        unsafe { self.alloc(l) }
    }

    unsafe fn dealloc(&self, p: *mut u8, _l: Layout) {
        if let Some(a) = api() {
            (a.0.free)(p);
        }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        match api() {
            Some(a) if l.align() <= 64 => (a.0.realloc)(p, size.max(1)),
            _ => null_mut(),
        }
    }
}
