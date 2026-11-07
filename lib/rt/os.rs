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
        let mut off = 0;
        while off < b.len() {
            let chunk = (b.len() - off).min(1 << 30);
            let mut done = 0u32;
            // SAFETY: b[off..] is a valid buffer; a missing console handle makes WriteFile fail harmlessly.
            let ok = unsafe { WriteFile(GetStdHandle(n), b.as_ptr().add(off), chunk as u32, &mut done, 0) };
            if ok == 0 || done == 0 {
                break;
            }
            off += done as usize;
        }
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

    // what the C runtime defines, for a program without it (with std the C runtime is there)
    #[cfg(not(feature = "std"))]
    #[unsafe(no_mangle)]
    pub static _fltused: i32 = 0;

    #[cfg(not(feature = "std"))]
    #[unsafe(no_mangle)]
    pub extern "C" fn __CxxFrameHandler3() -> ! {
        exit(4)
    }

    // stack probe: frames over 4 KB touch every page from the top down
    #[cfg(not(feature = "std"))]
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

    pub use crate::kernel::*;

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

    /// poll with the timeout in milliseconds (negative: none), by ppoll: ARM64 has no poll
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

    /// clone with its arguments in the kernel's order
    ///
    /// # Safety
    /// See syscall6; with a new stack or CLONE_VM the child must not return from here (a thread's
    /// start is assembly: linux_thread.rs).
    #[inline]
    pub unsafe fn clone(flags: usize, stack: usize, parent_tid: usize, child_tid: usize, tls: usize) -> isize {
        let [a, b, c] = clone_args(parent_tid, child_tid, tls);
        unsafe { syscall6(SYS_CLONE, flags, stack, a, b, c, 0) }
    }

    /// fork, by clone with only the child's exit signal (SIGCHLD): ARM64 has no fork
    ///
    /// # Safety
    /// As fork: in the child of a process with threads only what is safe there.
    #[inline]
    pub unsafe fn fork() -> isize {
        unsafe { clone(17, 0, 0, 0, 0) }
    }

    /// dup2, by dup3: ARM64 has no dup2.  dup3 refuses old == new, dup2 only looks whether old
    /// is open then
    ///
    /// # Safety
    /// new is closed first if it is open: it must not be in use elsewhere.
    #[inline]
    pub unsafe fn dup2(old: usize, new: usize) -> isize {
        if old == new {
            // SAFETY: fcntl(F_GETFD) changes nothing.
            let r = unsafe { syscall3(SYS_FCNTL, old, 1, 0) };
            return if r < 0 { r } else { old as isize };
        }
        unsafe { syscall3(SYS_DUP3, old, new, 0) }
    }

    /// rename, replacing new, by renameat2 (renameat before Linux 3.15): ARM64 has no rename
    ///
    /// # Safety
    /// old and new must be NUL-terminated paths.
    #[inline]
    pub unsafe fn rename(old: usize, new: usize) -> isize {
        let r = unsafe { syscall6(SYS_RENAMEAT2, AT_FDCWD, old, AT_FDCWD, new, 0, 0) };
        if r != -38 {
            return r; // not ENOSYS
        }
        unsafe { syscall6(SYS_RENAMEAT, AT_FDCWD, old, AT_FDCWD, new, 0, 0) }
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
            if r == -4 {
                continue; // EINTR
            }
            if r == -11 {
                // EAGAIN on a non-blocking descriptor (a pipe the reader does not empty): wait
                // until it takes more instead of spinning on sched_yield
                let mut pfd = [fd as i32, 4]; // struct pollfd: fd, events POLLOUT, revents 0
                // SAFETY: one pollfd record, no timeout.
                unsafe { poll(pfd.as_mut_ptr() as usize, 1, -1) };
                continue;
            }
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

    // std defines these (and links libgcc_s)
    #[cfg(not(feature = "std"))]
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}

    #[cfg(not(feature = "std"))]
    #[unsafe(no_mangle)]
    pub extern "C" fn _Unwind_Resume() -> ! {
        exit(4)
    }
}
