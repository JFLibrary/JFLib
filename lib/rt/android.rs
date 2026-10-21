// Android runtime: Bionic allocation and logging for the native, host-driven UI.
// New platform adapter; no assembly reference. Allocation never assumes a page size.

use core::{alloc::{GlobalAlloc, Layout}, ffi::{c_int, c_void}};

#[link(name = "c")]
unsafe extern "C" {
    fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int;
    fn free(ptr: *mut c_void);
    fn mmap(address: *mut c_void, size: usize, protection: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(address: *mut c_void, size: usize) -> c_int;
    fn sched_yield() -> c_int;
    fn _exit(code: c_int) -> !;
}
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const u8, text: *const u8) -> c_int;
}

struct AndroidAllocator;
#[global_allocator]
static ALLOC: AndroidAllocator = AndroidAllocator;

// SAFETY: Bionic supplies distinct, aligned allocations and accepts its own pointers on free.
unsafe impl GlobalAlloc for AndroidAllocator {
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

/// Allocate zeroed, page-aligned memory on both 4 KiB and 16 KiB Android devices.
/// # Safety
/// The returned pointer must be released exactly once with pages_free.
pub unsafe fn pages_alloc(size: usize) -> *mut u8 {
    // SAFETY: anonymous private mapping, kernel-selected address; no supplied memory is read.
    let ptr = unsafe { mmap(core::ptr::null_mut(), size.max(1), 3, 0x22, -1, 0) };
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
/// Terminate this app process on unrecoverable runtime failure.
pub fn exit(code: i32) -> ! {
    // SAFETY: terminates only the calling process.
    unsafe { _exit(code) }
}
fn log(bytes: &[u8], priority: i32) {
    for chunk in bytes.chunks(1000) {
        let mut text = [0u8; 1001];
        text[..chunk.len()].copy_from_slice(chunk);
        // SAFETY: tag and text are NUL-terminated and live for the synchronous logging call.
        unsafe { __android_log_write(priority, c"jfgui".as_ptr().cast(), text.as_ptr()) };
    }
}
/// Write diagnostics to Android logcat.
pub fn write_stdout(bytes: &[u8]) { log(bytes, 4); }
/// Write errors to Android logcat.
pub fn write_stderr(bytes: &[u8]) { log(bytes, 6); }

/// Unwinding is disabled; satisfy precompiled core's unused personality reference.
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() -> ! { exit(4) }
