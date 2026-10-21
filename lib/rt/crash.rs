// crash -- crash reports: the hook an app installs, the record the platform fills  [docs/modules/CRASH.md]
//
//   set_hook(Some(f))      f(&CrashInfo) runs once when the process crashes or panics, after
//                          the stderr line and before the process ends with exit code 3.  A
//                          crash inside it (or a second crashing thread) ends the process at
//                          once; a stack overflow skips it.
//   set_pre_hook(Some(f))  the library's part (plat::log: the newest log lines), before the
//                          app's hook; the same rules
//   report(&info) -> !     the end of every crash (the platform's fault handlers, the panic
//                          handler): the pre-hook, the hook, then exit(3)
//   StackText<N>           a fixed buffer for core::fmt -- crash paths do not allocate (the
//                          allocator lock may be held by the crashing thread)
//
// Rust adds one kind, CK_PANIC: the panic message is in CrashInfo::msg.

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const CK_ACCESS: u32 = 1; // access violation / SIGSEGV
pub const CK_ILLEGAL: u32 = 2; // illegal or privileged instruction
pub const CK_DIVIDE: u32 = 3; // integer division by zero / overflow, SIGFPE
pub const CK_BUS: u32 = 4; // SIGBUS, misalignment, in-page error
pub const CK_STACK: u32 = 5; // stack overflow (no hook: there is no stack left)
pub const CK_OTHER: u32 = 6;
pub const CK_PANIC: u32 = 7; // a Rust panic (msg)
pub const FRAMES_MAX: usize = 32;

/// what the crash handler knows (the CRASHINFO record of ASMLIB)
pub struct CrashInfo<'a> {
    pub code: u32,   // Windows exception code / Linux signal number (panic: 0)
    pub kind: u32,   // CK_*
    pub access: u32, // access violations: 0 read, 1 write, 8 execute, 2 unknown
    pub addr: u64,   // data address of an access violation / bus error (0 = none)
    pub ip: u64,     // instruction pointer
    pub base: u64,   // image base: offset = address - base (Linux 0: the .map lists addresses)
    pub code0: u64,  // the executable code: code0..code1
    pub code1: u64,
    pub regs: [u64; 17], // rax rbx rcx rdx rsi rdi rbp rsp r8 .. r15 rip
    pub frames: [u64; FRAMES_MAX], // the call chain: return addresses, innermost first (plat)
    pub nframes: u32,
    pub msg: &'a str, // CK_PANIC: the message
}

impl CrashInfo<'_> {
    pub const fn empty() -> Self {
        CrashInfo {
            code: 0,
            kind: CK_OTHER,
            access: 2,
            addr: 0,
            ip: 0,
            base: 0,
            code0: 0,
            code1: 0,
            regs: [0; 17],
            frames: [0; FRAMES_MAX],
            nframes: 0,
            msg: "",
        }
    }
}

pub type CrashHook = fn(&CrashInfo);

static HOOK: AtomicUsize = AtomicUsize::new(0);
static PRE_HOOK: AtomicUsize = AtomicUsize::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);

/// install (Some) or remove (None) the crash hook  [plat_crash_hook]
pub fn set_hook(f: Option<CrashHook>) {
    HOOK.store(f.map_or(0, |f| f as usize), Ordering::SeqCst);
}

/// install (Some) or remove (None) the library's hook that runs before the app's
pub fn set_pre_hook(f: Option<CrashHook>) {
    PRE_HOOK.store(f.map_or(0, |f| f as usize), Ordering::SeqCst);
}

/// a crash is being reported (a second one ends the process at once)
pub fn busy() -> bool {
    BUSY.load(Ordering::SeqCst)
}

/// run the hook once, then exit(3)
pub fn report(info: &CrashInfo) -> ! {
    if BUSY.swap(true, Ordering::SeqCst) {
        crate::os::exit(3); // a crash in the hook / on a second thread
    }
    for h in [PRE_HOOK.load(Ordering::SeqCst), HOOK.load(Ordering::SeqCst)] {
        if h != 0 && info.kind != CK_STACK {
            // SAFETY: h was stored from a CrashHook by set_hook / set_pre_hook.
            let f: CrashHook = unsafe { core::mem::transmute::<usize, CrashHook>(h) };
            f(info);
        }
    }
    crate::os::exit(3)
}

/// formatting into a fixed buffer (overflow is cut off)
pub struct StackText<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Default for StackText<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> StackText<N> {
    pub const fn new() -> Self {
        StackText { buf: [0; N], len: 0 }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
    pub fn as_str(&self) -> &str {
        // only whole str pieces are appended (cut at a char boundary below)
        core::str::from_utf8(self.bytes()).unwrap_or("")
    }
}

impl<const N: usize> fmt::Write for StackText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut n = s.len().min(N - self.len);
        while n < s.len() && !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
