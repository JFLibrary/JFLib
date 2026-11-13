// Apple runtime (macOS, iOS): libSystem's allocator and calls.  macOS and iOS have no stable
// system-call interface, only libSystem is (docs/APPLE.md, decision 2), so rt goes through it as
// android.rs goes through Bionic.  Allocation never assumes a page size: Apple Silicon has 16 KiB.

use core::{alloc::{GlobalAlloc, Layout}, ffi::{c_int, c_void}};

#[link(name = "System")]
unsafe extern "C" {
    fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int;
    fn free(ptr: *mut c_void);
    fn mmap(address: *mut c_void, size: usize, protection: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(address: *mut c_void, size: usize) -> c_int;
    fn sched_yield() -> c_int;
    fn _exit(code: c_int) -> !;
    fn write(fd: c_int, bytes: *const c_void, count: usize) -> isize;
}

// <sys/mman.h>: PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON (Darwin's values, not Linux's)
const PROT_RW: c_int = 0x1 | 0x2;
const MAP_PRIVATE_ANON: c_int = 0x0002 | 0x1000;

struct AppleAllocator;
#[cfg_attr(not(feature = "std"), global_allocator)]
#[cfg_attr(feature = "std", allow(dead_code))] // std's allocator: the same malloc
static ALLOC: AppleAllocator = AppleAllocator;

// SAFETY: libSystem supplies distinct, aligned allocations and accepts its own pointers on free.
unsafe impl GlobalAlloc for AppleAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut ptr = core::ptr::null_mut();
        // SAFETY: out is writable; Layout alignment is a power of two, at least sizeof(void*).
        let result = unsafe { posix_memalign(&mut ptr, layout.align().max(core::mem::size_of::<usize>()), layout.size().max(1)) };
        if result == 0 { ptr.cast() } else { core::ptr::null_mut() }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: GlobalAlloc's caller returns a live pointer from this allocator exactly once.
        unsafe { free(ptr.cast()) };
    }
}

/// Allocate zeroed, page-aligned memory (16 KiB pages on Apple Silicon).
/// # Safety
/// The returned pointer must be released exactly once with pages_free.
pub unsafe fn pages_alloc(size: usize) -> *mut u8 {
    // SAFETY: anonymous private mapping, kernel-selected address; no supplied memory is read.
    let ptr = unsafe { mmap(core::ptr::null_mut(), size.max(1), PROT_RW, MAP_PRIVATE_ANON, -1, 0) };
    if ptr as usize == usize::MAX { core::ptr::null_mut() } else { ptr.cast() }
}
/// Release a runtime allocation.
/// # Safety
/// ptr must be null or an unreleased result of pages_alloc with the same size.
pub unsafe fn pages_free(ptr: *mut u8, size: usize) {
    if !ptr.is_null() {
        // SAFETY: caller supplies the complete mapping and its original size.
        unsafe { munmap(ptr.cast(), size.max(1)) };
    }
}
/// Yield the calling thread.
pub fn yield_now() {
    // SAFETY: no arguments or borrowed memory.
    unsafe { sched_yield() };
}
/// End the process at once with `code` (no atexit handlers, nothing of stdio flushed: rt writes
/// unbuffered).
pub fn exit(code: i32) -> ! {
    // SAFETY: terminates only the calling process.
    unsafe { _exit(code) }
}
fn write_all(fd: c_int, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        // SAFETY: bytes is readable for its length; only the descriptors of the process.
        let n = unsafe { write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n <= 0 {
            return; // closed or failing: diagnostics are best effort
        }
        bytes = &bytes[n as usize..];
    }
}
/// Write to stdout (on iOS: devicectl --console and Xcode show it).
pub fn write_stdout(bytes: &[u8]) { write_all(1, bytes); }
/// Write to stderr.
pub fn write_stderr(bytes: &[u8]) { write_all(2, bytes); }
/// stderr for crash paths: write_stderr, which already gives up where a write takes nothing.
pub fn write_stderr_crash(bytes: &[u8]) { write_stderr(bytes); }

/// Unwinding is disabled; satisfy precompiled core's unused personality reference (std has its own).
#[cfg(not(feature = "std"))]
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() -> ! { exit(4) }
