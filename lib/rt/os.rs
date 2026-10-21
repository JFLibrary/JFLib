// os -- the OS calls the runtime itself needs: pages, console output, exit; plus the
// symbols a no_std binary must define (MSVC float marker, stack probe, the unwinding
// personality routines the precompiled `core` names -- with panic=abort none of them runs).
// Everything else of the platform is in the `plat` crate.

pub use imp::*;

#[cfg(windows)]
mod imp {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetStdHandle(n: u32) -> isize;
        fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: usize) -> i32;
        fn ExitProcess(code: u32) -> !;
        fn VirtualAlloc(addr: usize, size: usize, typ: u32, prot: u32) -> *mut u8;
        fn VirtualFree(addr: *mut u8, size: usize, typ: u32) -> i32;
        fn SwitchToThread() -> i32;
    }

    /// give the rest of the time slice to another ready thread
    pub fn yield_now() {
        // SAFETY: no arguments.
        unsafe { SwitchToThread() };
    }

    pub fn exit(code: i32) -> ! {
        // SAFETY: ends the process.
        unsafe { ExitProcess(code as u32) }
    }

    fn write_handle(n: u32, b: &[u8]) {
        let mut done = 0u32;
        // SAFETY: b is a valid buffer; a missing console handle makes WriteFile fail harmlessly.
        unsafe { WriteFile(GetStdHandle(n), b.as_ptr(), b.len() as u32, &mut done, 0) };
    }

    pub fn write_stdout(b: &[u8]) {
        write_handle(-11i32 as u32, b)
    }

    pub fn write_stderr(b: &[u8]) {
        write_handle(-12i32 as u32, b)
    }

    /// zeroed pages, or null
    ///
    /// # Safety
    /// Free them with pages_free.
    pub unsafe fn pages_alloc(n: usize) -> *mut u8 {
        // MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE
        unsafe { VirtualAlloc(0, n, 0x3000, 4) }
    }

    /// # Safety
    /// p must come from pages_alloc.
    pub unsafe fn pages_free(p: *mut u8, _n: usize) {
        unsafe { VirtualFree(p, 0, 0x8000) };
    }

    #[unsafe(no_mangle)]
    pub static _fltused: i32 = 0;

    #[unsafe(no_mangle)]
    pub extern "C" fn __CxxFrameHandler3() -> ! {
        exit(4)
    }

    // stack probe: frames over 4 KB touch every page from the top down
    core::arch::global_asm!(
        ".globl __chkstk",
        "__chkstk:",
        "    push rcx",
        "    push rax",
        "    cmp rax, 4096",
        "    lea rcx, [rsp + 24]",
        "    jb 2f",
        "1:",
        "    sub rcx, 4096",
        "    test [rcx], rcx",
        "    sub rax, 4096",
        "    cmp rax, 4096",
        "    ja 1b",
        "2:",
        "    sub rcx, rax",
        "    test [rcx], rcx",
        "    pop rax",
        "    pop rcx",
        "    ret",
    );
}

#[cfg(target_os = "linux")]
mod imp {
    use core::arch::asm;

    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_FSTAT: usize = 5;
    pub const SYS_POLL: usize = 7;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_MUNMAP: usize = 11;
    pub const SYS_MADVISE: usize = 28;
    pub const SYS_SCHED_YIELD: usize = 24;
    pub const SYS_SOCKET: usize = 41;
    pub const SYS_CONNECT: usize = 42;
    pub const SYS_SENDMSG: usize = 46;
    pub const SYS_RECVMSG: usize = 47;
    pub const SYS_FTRUNCATE: usize = 77;
    pub const SYS_CLOCK_GETTIME: usize = 228;
    pub const SYS_EXIT_GROUP: usize = 231;
    pub const SYS_MEMFD_CREATE: usize = 319;

    /// # Safety
    /// Raw system call: the arguments must be valid for the call.
    #[inline]
    pub unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
        let r: isize;
        unsafe {
            asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c,
                 in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack));
        }
        r
    }

    /// # Safety
    /// See syscall6.
    #[inline]
    pub unsafe fn syscall3(n: usize, a: usize, b: usize, c: usize) -> isize {
        unsafe { syscall6(n, a, b, c, 0, 0, 0) }
    }

    pub fn exit(code: i32) -> ! {
        // SAFETY: ends the process.
        unsafe { syscall3(SYS_EXIT_GROUP, code as usize, 0, 0) };
        loop {}
    }

    /// give the rest of the time slice to another ready thread
    pub fn yield_now() {
        // SAFETY: no arguments.
        unsafe { syscall3(SYS_SCHED_YIELD, 0, 0, 0) };
    }

    pub fn write_fd(fd: usize, b: &[u8]) {
        let mut off = 0;
        while off < b.len() {
            // SAFETY: b is a valid buffer.
            let r = unsafe { syscall3(SYS_WRITE, fd, b.as_ptr() as usize + off, b.len() - off) };
            if r <= 0 {
                break;
            }
            off += r as usize;
        }
    }

    pub fn write_stdout(b: &[u8]) {
        write_fd(1, b)
    }

    pub fn write_stderr(b: &[u8]) {
        write_fd(2, b)
    }

    /// zeroed pages, or null
    ///
    /// # Safety
    /// Free them with pages_free.
    pub unsafe fn pages_alloc(n: usize) -> *mut u8 {
        // PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS
        let r = unsafe { syscall6(SYS_MMAP, 0, n, 3, 0x22, usize::MAX, 0) };
        if r < 0 {
            return core::ptr::null_mut();
        }
        if n >= 4 << 20 {
            // MADV_HUGEPAGE for large runs (images, planes): ~10x fewer page faults; JPEG / PNG
            // loads 15-25 % faster in WSL (perf-load).  A no-op where THP is off; result ignored.
            // SAFETY: r, n is the mapping just created.
            unsafe { syscall3(SYS_MADVISE, r as usize, n, 14) };
        }
        r as *mut u8
    }

    /// # Safety
    /// p, n must come from pages_alloc.
    pub unsafe fn pages_free(p: *mut u8, n: usize) {
        unsafe { syscall3(SYS_MUNMAP, p as usize, n, 0) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}

    #[unsafe(no_mangle)]
    pub extern "C" fn _Unwind_Resume() -> ! {
        exit(4)
    }
}
