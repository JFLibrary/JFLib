// Android runtime: Bionic allocation and logging for the native, host-driven UI.
// New platform adapter; no assembly reference. Allocation never assumes a page size.
// The kernel calls plat makes the Linux way (futex, eventfd; docs/ANDROID.md decision 2) go through
// syscall6 / syscall3 / poll with the numbers of rt::kernel, as on Linux; threads, allocation and
// signals stay Bionic's.

use core::{alloc::{GlobalAlloc, Layout}, ffi::{c_int, c_void}, sync::atomic::{AtomicBool, Ordering}};

pub use crate::kernel::*;

/// # Safety
/// Raw system call: the arguments must be valid for the call.  Only calls Zygote's seccomp filter
/// lets an app make (docs/ANDROID.md, AN0's table): any other ends the process with SIGSYS.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r: isize;
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c,
                         in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    r
}

/// # Safety
/// As above.  ARM64: the number in x8, the result in x0; the kernel keeps every other register.
#[cfg(target_arch = "aarch64")]
#[inline]
pub unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r: isize;
    unsafe {
        core::arch::asm!("svc 0", in("x8") n, inlateout("x0") a as isize => r, in("x1") b, in("x2") c,
                         in("x3") d, in("x4") e, in("x5") f, options(nostack));
    }
    r
}

/// # Safety
/// See syscall6.
#[inline]
pub unsafe fn syscall3(n: usize, a: usize, b: usize, c: usize) -> isize {
    unsafe { syscall6(n, a, b, c, 0, 0, 0) }
}

/// poll with the timeout in milliseconds (negative: none), by ppoll (ARM64 has no poll)
///
/// # Safety
/// fds must point to n struct pollfd.
#[inline]
pub unsafe fn poll(fds: usize, n: usize, ms: i32) -> isize {
    // ppoll writes the time left back
    let mut ts = [(ms / 1000) as i64, (ms % 1000) as i64 * 1_000_000];
    let timeout = if ms < 0 { 0 } else { ts.as_mut_ptr() as usize };
    // SAFETY: fds as the caller promises, ts lives during the call; no signal mask.
    unsafe { syscall6(SYS_PPOLL, fds, n, timeout, 0, 8, 0) }
}

#[link(name = "c")]
unsafe extern "C" {
    fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int;
    fn free(ptr: *mut c_void);
    fn mmap(address: *mut c_void, size: usize, protection: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(address: *mut c_void, size: usize) -> c_int;
    fn sched_yield() -> c_int;
    fn _exit(code: c_int) -> !;
    fn write(fd: c_int, bytes: *const c_void, count: usize) -> isize;
}
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const u8, text: *const u8) -> c_int;
}

struct AndroidAllocator;
#[cfg_attr(not(feature = "std"), global_allocator)]
#[cfg_attr(feature = "std", allow(dead_code))] // std's allocator: the same malloc
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
// a program without a UI, run from `adb shell` (android/exe_main.c): stdout and stderr
static CONSOLE: AtomicBool = AtomicBool::new(false);
/// (android/exe_main.c) the output goes to stdout and stderr, not to logcat
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_console() {
    CONSOLE.store(true, Ordering::Relaxed);
}
fn log(bytes: &[u8], priority: i32) {
    if CONSOLE.load(Ordering::Relaxed) {
        // SAFETY: bytes is readable for its length; only the descriptors of the process.
        unsafe { write(if priority == 6 { 2 } else { 1 }, bytes.as_ptr().cast(), bytes.len()) };
        return;
    }
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
/// stderr for crash paths: logcat, as write_stderr.
pub fn write_stderr_crash(bytes: &[u8]) { write_stderr(bytes); }

/// Unwinding is disabled; satisfy precompiled core's unused personality reference (std has its own).
#[cfg(not(feature = "std"))]
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() -> ! { exit(4) }
