// linux_glibc -- a program linked against glibc (--cfg jfgui_glibc; build_rs.py section "glibc",
// docs/GPU.md "Linux"): every Linux GPU driver is a glibc library
//
//   _start, glibc_main          the process start through __libc_start_main (ld.so has set up the
//                               main thread's TLS and run libc's initialisers by then), then what
//                               linux_entry does in a static program
//   std_start()                 (feature std: every program with std on Linux is this build) the
//                               start from the program's fn main, std's arguments and environment
//                               as plat reads them from the initial stack, the end through std
//   exit(code)                  plat::exit: glibc's exit (atexit handlers, the destructors of the
//                               loaded libraries, stdio); so does returning from the program's main.
//                               Crash reports end in rt::os::exit as everywhere: exit_group, no
//                               destructor runs inside a signal handler
//   at_exit(f)                  f at that normal end, before the handlers registered earlier
//   dl_open / dl_sym / dl_close / dl_error    the system's dynamic linker (lib/gpu: libvulkan.so.1);
//                               plugins keep their own loader (linux_elfload.rs: they import nothing)
//   glibc_version()             the running libc ("2.43")
//   glibc_thread_ids()          the calling thread's FS base, pthread_self, &errno (gputest --test glibc)
//
// Threads: linux_thread.rs, its last part: pthread_create on the same mapping as in a static
// program.  Everything else of plat stays raw system calls next to glibc, which is fine: they do
// not touch errno or glibc's state.  The fault handlers (linux_crash.rs, linux_guard.rs) use
// rt_sigaction directly and leave glibc's two internal signals alone.  A driver's own threads have
// no alternate signal stack, the report runs on their stack; Mesa's block every signal but SIGSEGV
// and SIGSYS, so another fault there (SIGILL in llvmpipe's JIT code) ends the process at once, by the
// kernel, without a report.  The allocator (rt::mem) has its own mappings and takes no TLS, so a
// driver's thread may call back into it (debug messages); a child after a raw fork only makes system
// calls until its exec.  plat::setenv changes JFLib's list, which plat::env and child processes see;
// a driver's getenv reads the start environment.

use alloc::string::String;
use core::ffi::c_void;

unsafe extern "C" {
    fn __libc_start_main(main: extern "C" fn(i32, *const usize, *const usize) -> i32, argc: i32, argv: *const usize,
                         init: usize, fini: usize, rtld_fini: usize, stack_end: *const u8) -> i32;
    fn __cxa_atexit(f: extern "C" fn(*mut c_void), arg: *mut c_void, dso: *mut c_void) -> i32;
    fn __errno_location() -> *mut i32;
    #[cfg(not(feature = "std"))] // with std: std::process::exit (hosted.rs)
    #[link_name = "exit"]
    fn c_exit(code: i32) -> !;
    fn gnu_get_libc_version() -> *const u8;
    fn pthread_self() -> usize;
    fn dlopen(name: *const u8, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const u8) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
    fn dlerror() -> *const u8;
    pub(super) fn pthread_attr_init(attr: *mut PthreadAttr) -> i32;
    pub(super) fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;
    pub(super) fn pthread_attr_setstack(attr: *mut PthreadAttr, stack: *mut u8, size: usize) -> i32;
    pub(super) fn pthread_create(thread: *mut usize, attr: *const PthreadAttr, start: extern "C" fn(*mut u8) -> *mut u8,
                                 arg: *mut u8) -> i32;
    pub(super) fn pthread_join(thread: usize, ret: *mut *mut u8) -> i32;
}

/// pthread_attr_t (56 bytes on x86-64, 64 on ARM64)
#[repr(C, align(8))]
pub(super) struct PthreadAttr(pub(super) [u8; PTHREAD_ATTR]); #[cfg(target_arch = "x86_64")] pub(super) const PTHREAD_ATTR: usize = 56;

#[cfg(target_arch = "x86_64")] const ARCH_GET_FS: usize = 0x1003;
const RTLD_NOW: i32 = 2;

// the ABI's entry: rsp -> argc, argv[], 0, envp[], 0; rdx = ld.so's finaliser (registered by
// __libc_start_main).  init and fini 0: this program has neither (glibc takes both as optional).
// A program with std has the C runtime's _start and std_start.
#[cfg(all(target_arch = "x86_64", not(feature = "std")))]
core::arch::global_asm!(
    ".globl _start",
    "_start:",
    "    xor ebp, ebp",
    "    mov r9, rdx",
    "    pop rsi",
    "    mov rdx, rsp",
    "    and rsp, -16",
    "    push rax",
    "    push rsp",
    "    xor r8d, r8d",
    "    xor ecx, ecx",
    "    lea rdi, [rip + {main}]",
    "    call {start}",
    "    ud2",
    main = sym glibc_main,
    start = sym __libc_start_main,
);

#[cfg(not(feature = "std"))]
extern "C" fn glibc_main(_argc: i32, argv: *const usize, _envp: *const usize) -> i32 {
    // SAFETY: argv lies on the initial stack right after argc, as linux.rs reads it; written once
    // before anything reads it, on the only thread of this program.
    unsafe { super::START_SP = argv.sub(1) };
    super::crash::crash_init();
    super::thread::wake_init();
    crate::cpu::simd_override();
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    // SAFETY: the program's entry symbol (ui::main!, rt::main!); the return value goes to exit
    unsafe { super::jfgui_main() }
}

/// (feature std) the start of a program with std, from its `fn main` (rt::main!, ui::main! or the
/// program's own): std's arguments and environment as a block like the initial stack, then what
/// glibc_main does, with rt's panic hook first; the end through std::process::exit
#[cfg(feature = "std")]
pub fn std_start() -> ! {
    // SAFETY: written once, first thing in the program's main, before anything reads it.
    unsafe { super::START_SP = crate::hosted::initial_block() };
    rt::panic_hook();
    super::crash::crash_init();
    super::thread::wake_init();
    crate::cpu::simd_override();
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    // SAFETY: the program's entry symbol (ui::main!, rt::main!).
    let code = unsafe { super::jfgui_main() };
    crate::exit(code) // hosted.rs: std::process::exit
}

/// end the process: glibc's exit (at_exit handlers, the destructors of the loaded libraries);
/// from any thread, not from a signal handler
#[cfg(not(feature = "std"))]
pub fn exit(code: i32) -> ! {
    // SAFETY: ends the process.
    unsafe { c_exit(code) }
}

extern "C" fn run_at_exit(f: *mut c_void) {
    // SAFETY: at_exit stored a fn() here.
    let f = unsafe { core::mem::transmute::<*mut c_void, fn()>(f) };
    f();
}

/// run f at the normal end of the process (plat::exit, or the program's main returns), not after
/// a crash; before the handlers registered earlier
pub fn at_exit(f: fn()) {
    // SAFETY: a fn pointer fits the argument; 0: not tied to a library that could be unloaded.
    unsafe { __cxa_atexit(run_at_exit, f as *mut c_void, core::ptr::null_mut()) };
}

/// open a library through the system's dynamic linker (a name, searched as ld.so does, or a path)
/// and run its initialisers -> its handle
pub fn dl_open(name: &str) -> Option<usize> {
    if name.contains('\0') {
        return None;
    }
    let mut z: alloc::vec::Vec<u8> = name.as_bytes().to_vec();
    z.push(0);
    // SAFETY: z is NUL-terminated.
    let h = unsafe { dlopen(z.as_ptr(), RTLD_NOW) };
    if h.is_null() { None } else { Some(h as usize) }
}

/// the address of a symbol of a library from dl_open (0: none); name ends in NUL
pub fn dl_sym(handle: usize, name: &[u8]) -> usize {
    if name.last() != Some(&0) {
        return 0;
    }
    // SAFETY: a handle of dl_open, a NUL-terminated name.
    unsafe { dlsym(handle as *mut c_void, name.as_ptr()) as usize }
}

/// close a library from dl_open
pub fn dl_close(handle: usize) {
    // SAFETY: a handle of dl_open; the caller no longer uses what it got from it.
    unsafe { dlclose(handle as *mut c_void) };
}

unsafe fn c_text(p: *const u8) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated string (the caller's source says so).
    unsafe { super::cstr(p) }
}

/// why the calling thread's last dl_open / dl_sym failed ("": it did not, or it was already asked)
pub fn dl_error() -> String {
    // SAFETY: dlerror returns null or a string that stays valid until this thread's next dl call.
    unsafe { c_text(dlerror()) }
}

/// the version of the running glibc ("2.43")
pub fn glibc_version() -> String {
    // SAFETY: a static NUL-terminated string.
    unsafe { c_text(gnu_get_libc_version()) }
}

/// the calling thread's FS base, pthread_self() and the address of errno: its thread control block
/// and thread-local storage (a raw clone would have 0 and the creator's)
#[cfg(target_arch = "x86_64")] pub fn glibc_thread_ids() -> (usize, usize, usize) {
    let mut fs = 0usize;
    // SAFETY: arch_prctl writes the FS base to fs; the two libc calls have no preconditions on a
    // thread started by glibc.
    unsafe {
        rt::os::syscall3(rt::os::SYS_ARCH_PRCTL, ARCH_GET_FS, &mut fs as *mut usize as usize, 0);
        (fs, pthread_self(), __errno_location() as usize)
    }
}

// ---------------------------------------------------------------- ARM64 (docs/ARM64.md)
//
// _start as above in ARM64's registers: ld.so leaves its finaliser in x0 and sp on argc (16-aligned);
// __libc_start_main(main = x0, argc = w1, argv = x2, init = x3, fini = x4, rtld_fini = x5, stack_end
// = x6), frame pointer and link register 0 end the call chain.  glibc_thread_ids reads TPIDR_EL0
// instead of the FS base: ARM64 has no arch_prctl, and its TLS is variant I, so the thread pointer
// points at the TCB right above glibc's struct pthread (pthread_self() lies a little below it).
#[cfg(target_arch = "aarch64")]
pub(super) const PTHREAD_ATTR: usize = 64;

#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
core::arch::global_asm!(
    ".globl _start",
    "_start:",
    "    mov x29, xzr",
    "    mov x30, xzr",
    "    mov x5, x0",
    "    ldr x1, [sp]",
    "    add x2, sp, #8",
    "    mov x6, sp",
    "    mov x3, xzr",
    "    mov x4, xzr",
    "    adrp x0, {main}",
    "    add x0, x0, :lo12:{main}",
    "    bl {start}",
    "    brk #1",
    main = sym glibc_main,
    start = sym __libc_start_main,
);

/// the calling thread's thread pointer (TPIDR_EL0: its TCB), pthread_self() and the address of errno
#[cfg(target_arch = "aarch64")]
pub fn glibc_thread_ids() -> (usize, usize, usize) {
    let tp: usize;
    // SAFETY: reads the thread pointer register only; the two libc calls have no preconditions on a
    // thread started by glibc.
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) tp, options(nomem, nostack, preserves_flags));
        (tp, pthread_self(), __errno_location() as usize)
    }
}
