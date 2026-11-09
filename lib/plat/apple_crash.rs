// apple_crash -- crash handling on macOS and iOS (arm64; the promises of linux_crash.rs)
//
// SIGSEGV, SIGBUS, SIGILL, SIGFPE and SIGTRAP run a handler on a 64 KB alternate stack (every thread
// has its own: thread_alt_stack): it fills a CrashInfo from the signal's context, prints "jfgui: crash
// signal N at pc 0x..., address 0x..." and ends in rt::crash::report (the app's hook, exit code 3).
// SA_RESETHAND: a crash inside the handler ends the process.  CrashInfo::frames: the link register,
// then the chain of frame records (fp, lr) -- Apple's arm64 ABI keeps the frame pointer, so no unwind
// tables are needed -- read only inside the crashing thread's stack.  CrashInfo::regs: x0 .. x15, pc.
// A fault just below the thread's stack is a stack overflow (CK_STACK: no hook).

use core::ffi::{c_int, c_void};
use core::fmt::Write;

use rt::crash::{CK_ACCESS, CK_BUS, CK_DIVIDE, CK_ILLEGAL, CK_OTHER, CK_STACK, CrashInfo, FRAMES_MAX, StackText};

const SIGILL: c_int = 4;
const SIGTRAP: c_int = 5;
const SIGFPE: c_int = 8;
const SIGBUS: c_int = 10;
const SIGSEGV: c_int = 11;
const SA_ONSTACK: c_int = 0x0001;
const SA_RESETHAND: c_int = 0x0004;
const SA_SIGINFO: c_int = 0x0040;
const ALT_SIZE: usize = 65536;
const GUARD_ZONE: u64 = 65536; // a fault this far below the stack is an overflow

#[repr(C)]
struct SigAction {
    handler: usize,
    mask: u32,
    flags: c_int,
}
#[repr(C)]
struct StackT {
    sp: *mut c_void,
    size: usize,
    flags: c_int,
}
#[repr(C)]
struct MachHeader([u32; 8]);

#[link(name = "System")]
unsafe extern "C" {
    fn sigaction(sig: c_int, act: *const SigAction, old: *mut SigAction) -> c_int;
    fn sigaltstack(ss: *const StackT, old: *mut StackT) -> c_int;
    fn pthread_self() -> usize;
    fn pthread_get_stackaddr_np(t: usize) -> *mut c_void;
    fn pthread_get_stacksize_np(t: usize) -> usize;
    fn _dyld_get_image_header(index: u32) -> *const MachHeader;
    fn getsegmentdata(mh: *const MachHeader, segname: *const u8, size: *mut usize) -> *mut u8;
}

/// an alternate signal stack for the calling thread (process start, every new thread): a crash
/// handler that runs out of the thread's own stack still gets its report
pub(crate) fn thread_alt_stack() {
    // SAFETY: a fresh mapping of ALT_SIZE bytes handed to the kernel as this thread's alternate
    // stack; it stays mapped for the life of the process (a thread's stack is small next to it).
    unsafe {
        let p = rt::os::pages_alloc(ALT_SIZE);
        if !p.is_null() {
            let ss = StackT { sp: p.cast(), size: ALT_SIZE, flags: 0 };
            sigaltstack(&ss, core::ptr::null_mut());
        }
    }
}

/// the alternate stack of the main thread and the handlers (process start)
pub(crate) fn crash_init() {
    thread_alt_stack();
    let sa = SigAction { handler: handler as *const () as usize, mask: 0, flags: SA_SIGINFO | SA_ONSTACK | SA_RESETHAND };
    for sig in [SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP] {
        // SAFETY: a valid sigaction record; the handler never returns.
        unsafe { sigaction(sig, &sa, core::ptr::null_mut()) };
    }
}

// Darwin's arm64 signal context: ucontext->uc_mcontext (offset 48) -> mcontext64: the exception
// state (far, esr), then the thread state x0 .. x28, fp, lr, sp, pc
const UC_MCONTEXT: usize = 48;
const MC_FAR: usize = 0;
const MC_ESR: usize = 8;
const MC_X: usize = 16;
const MC_FP: usize = 16 + 29 * 8;
const MC_LR: usize = MC_FP + 8;
const MC_SP: usize = MC_LR + 8;
const MC_PC: usize = MC_SP + 8;
const SI_ADDR: usize = 24;

extern "C" fn handler(sig: c_int, si: *const u8, uc: *const u8) -> ! {
    if rt::crash::busy() {
        rt::os::exit(3);
    }
    let mut ci = CrashInfo::empty();
    ci.code = sig as u32;
    ci.kind = match sig {
        SIGSEGV => CK_ACCESS,
        SIGBUS => CK_BUS,
        SIGILL | SIGTRAP => CK_ILLEGAL,
        SIGFPE => CK_DIVIDE,
        _ => CK_OTHER,
    };
    // SAFETY: the kernel passes a valid siginfo and ucontext with its machine context.
    let (si_addr, sp, fp, lr) = unsafe {
        let mc = *(uc.add(UC_MCONTEXT) as *const *const u8);
        let rd = |o: usize| *(mc.add(o) as *const u64);
        let si_addr = *(si.add(SI_ADDR) as *const u64);
        if ci.kind == CK_ACCESS || ci.kind == CK_BUS {
            ci.addr = si_addr;
            // a data abort's syndrome: bit 6 (WnR) is a write; an instruction abort (EC 0x20, 0x21) a fetch
            let esr = *(mc.add(MC_ESR) as *const u32);
            ci.access = if matches!(esr >> 26, 0x20 | 0x21) { 8 } else { esr >> 6 & 1 };
            if ci.addr == 0 {
                ci.addr = rd(MC_FAR);
            }
        }
        for i in 0..16 {
            ci.regs[i] = rd(MC_X + i * 8);
        }
        ci.ip = rd(MC_PC);
        ci.regs[16] = ci.ip;
        (si_addr, rd(MC_SP), rd(MC_FP), rd(MC_LR))
    };
    // SAFETY: the calling thread's own stack bounds.
    let (top, bottom) = unsafe {
        let t = pthread_self();
        let top = pthread_get_stackaddr_np(t) as u64;
        (top, top.saturating_sub(pthread_get_stacksize_np(t) as u64))
    };
    if ci.kind == CK_ACCESS && ci.addr < bottom && bottom - ci.addr <= GUARD_ZONE {
        ci.kind = CK_STACK;
    }
    code_range(&mut ci);
    call_chain(&mut ci, lr, fp, sp.min(fp).max(bottom), top);
    let mut line = StackText::<160>::new();
    let _ = writeln!(line, "jfgui: crash signal {sig} at pc 0x{:x}, address 0x{si_addr:x} (image base 0x{:x})", ci.ip, ci.base);
    rt::os::write_stderr(line.bytes());
    rt::crash::report(&ci)
}

/// base = the main image's header (offsets = address - base), code0 / code1 = its __TEXT segment
fn code_range(ci: &mut CrashInfo) {
    // SAFETY: dyld's header of image 0 (the program) and its __TEXT segment.
    unsafe {
        let mh = _dyld_get_image_header(0);
        if mh.is_null() {
            return;
        }
        ci.base = mh as u64;
        let mut size = 0usize;
        let text = getsegmentdata(mh, c"__TEXT".as_ptr().cast(), &mut size);
        if !text.is_null() {
            ci.code0 = text as u64;
            ci.code1 = text as u64 + size as u64;
        }
    }
}

/// frames: the link register (a leaf like crash_test saves no frame record), then the frame records
/// fp -> (previous fp, return address), each inside lo..top of the thread's stack and above the last
fn call_chain(ci: &mut CrashInfo, lr: u64, mut fp: u64, lo: u64, top: u64) {
    let mut n = 0;
    if lr != 0 {
        ci.frames[n] = lr;
        n += 1;
    }
    let mut last = 0u64;
    while n < FRAMES_MAX && fp % 16 == 0 && fp >= lo && fp + 16 <= top && fp > last {
        // SAFETY: fp .. fp + 16 lies inside the crashing thread's mapped stack (checked above).
        let (prev, ret) = unsafe { (*(fp as *const u64), *((fp + 8) as *const u64)) };
        if ret == 0 {
            break;
        }
        if ret != ci.frames[n.saturating_sub(1)] || n == 0 {
            ci.frames[n] = ret;
            n += 1;
        }
        last = fp;
        fp = prev;
    }
    ci.nframes = n as u32;
}
