// linux_guard -- crash guard: the Linux part     [src/plat/linux/lx_guard.s; docs/modules/PLUGIN_LOADER.md]
//
//   (crate) plat_tid() -> u64           gettid()
//   (crate) guard_thread()              give the calling thread an alternate signal stack
//                                       (64 KB, mmap) unless it has one -- so a stack
//                                       overflow inside a guard can still be handled
//   (crate) guard_os_install()          rt_sigaction SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP:
//                                       guard_sig (SA_SIGINFO | SA_ONSTACK), old actions kept
//
// guard_sig runs on the faulting thread: with an active guard (plib.rs) it unlinks the
// record, stores the fault and edits the ucontext (rip = the guard's fault address, rsp /
// rbp of the trampoline's body, DF = 0); rt_sigreturn then resumes there.  Without a guard
// the previous handler runs (linux_crash.rs: message + exit 3), or -- if it was SIG_DFL /
// SIG_IGN -- the default action is restored and the signal re-raised.

use core::sync::atomic::{AtomicUsize, Ordering};

use rt::os::syscall6;

use crate::{GK_ACCESS, GK_ARITH, GK_BREAK, GK_BUS, GK_ILLEGAL, guard_fault_ip, guard_hit};

const SYS_MMAP: usize = 9;
const SYS_RT_SIGACTION: usize = 13;
const SYS_GETPID: usize = 39;
const SYS_SIGALTSTACK: usize = 131;
const SYS_GETTID: usize = 186;
const SYS_TGKILL: usize = 234;
const SIGILL: i32 = 4;
const SIGTRAP: i32 = 5;
const SIGBUS: i32 = 7;
const SIGFPE: i32 = 8;
const SIGSEGV: i32 = 11;
const SA_SIGINFO: usize = 4;
const SA_ONSTACK: usize = 0x0800_0000;
const SA_RESTORER: usize = 0x0400_0000;
const SS_DISABLE: usize = 2;
const ALT_SIZE: usize = 65536;
const PROT_RW: usize = 3;
const MAP_PRIVATE_ANON: usize = 0x22;
const UC_GREGS: usize = 40; // ucontext: uc_flags, uc_link, uc_stack, then the registers
const REG_RBP: usize = 10;
const REG_RSP: usize = 15;
const REG_RIP: usize = 16;
const REG_EFL: usize = 17;
const SIGI_ADDR: usize = 16; // siginfo: si_addr

// the previous k_sigaction of every signal: handler, flags, restorer, mask
#[allow(clippy::declare_interior_mutable_const)]
const OLD0: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static OLD_ACT: [[AtomicUsize; 4]; 32] = [OLD0; 32];

// return path of guard_sig
core::arch::global_asm!(
    ".globl jfgui_rs_guard_sigret",
    "jfgui_rs_guard_sigret:",
    "    mov eax, 15",
    "    syscall",
    "    hlt",
);

unsafe extern "C" {
    fn jfgui_rs_guard_sigret();
}

fn sys(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    // SAFETY: the callers pass valid kernel structures (see each call).
    unsafe { syscall6(n, a, b, c, d, 0, 0) }
}

/// the id of the calling thread (gettid)  [plat_tid]
pub(crate) fn plat_tid() -> u64 {
    sys(SYS_GETTID, 0, 0, 0, 0) as u64
}

/// an alternate signal stack for this thread (once)  [plat_guard_thread]
pub(crate) fn guard_thread() {
    let mut ss = [0usize; 3]; // ss_sp, ss_flags, ss_size
    ss[1] = SS_DISABLE;
    if sys(SYS_SIGALTSTACK, 0, ss.as_mut_ptr() as usize, 0, 0) != 0 || ss[1] & SS_DISABLE == 0 {
        return; // error, or it already has one
    }
    // SAFETY: an anonymous mapping (fd -1, offset 0).
    let p = unsafe { syscall6(SYS_MMAP, 0, ALT_SIZE, PROT_RW, MAP_PRIVATE_ANON, usize::MAX, 0) };
    if (p as usize) > (-4096isize) as usize {
        return;
    }
    let ss = [p as usize, 0, ALT_SIZE];
    sys(SYS_SIGALTSTACK, ss.as_ptr() as usize, 0, 0, 0);
}

/// guard_sig for the fault signals, the previous actions kept  [guard_os_install]
pub(crate) fn guard_os_install() {
    guard_thread();
    let sa = [guard_sig as *const () as usize, SA_SIGINFO | SA_ONSTACK | SA_RESTORER,
              jfgui_rs_guard_sigret as *const () as usize, 0];
    for sig in [SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP] {
        let mut old = [0usize; 4];
        sys(SYS_RT_SIGACTION, sig as usize, sa.as_ptr() as usize, old.as_mut_ptr() as usize, 8);
        for (k, v) in old.iter().enumerate() {
            OLD_ACT[sig as usize][k].store(*v, Ordering::SeqCst);
        }
    }
}

/// the signal handler (SysV entry: sig, siginfo, ucontext)  [guard_sig]
extern "C" fn guard_sig(sig: i32, si: *mut u8, uc: *mut u8) {
    let kind = match sig {
        SIGSEGV => GK_ACCESS,
        SIGBUS => GK_BUS,
        SIGILL => GK_ILLEGAL,
        SIGFPE => GK_ARITH,
        _ => GK_BREAK,
    };
    // SAFETY: the kernel passes a valid siginfo and ucontext for this signal.
    unsafe {
        let greg = |i: usize| uc.add(UC_GREGS + i * 8) as *mut u64;
        let pc = *greg(REG_RIP);
        let addr = if kind == GK_ACCESS || kind == GK_BUS { *(si.add(SIGI_ADDR) as *const u64) } else { 0 };
        if let Some((rsp, rbp)) = guard_hit(plat_tid(), kind, sig as u32, pc, addr) {
            *greg(REG_RIP) = guard_fault_ip() as u64;
            *greg(REG_RSP) = rsp;
            *greg(REG_RBP) = rbp;
            *greg(REG_EFL) &= !0x400; // DF = 0
            return;
        }
    }
    // not ours: the previous handler, or the default action
    let h = OLD_ACT[(sig & 31) as usize][0].load(Ordering::SeqCst);
    if h > 1 {
        // SAFETY: a SA_SIGINFO handler installed before us (the crash handler).
        let f: extern "C" fn(i32, *mut u8, *mut u8) = unsafe { core::mem::transmute(h) };
        f(sig, si, uc);
        return;
    }
    let dfl = [0usize; 4];
    sys(SYS_RT_SIGACTION, sig as usize, dfl.as_ptr() as usize, 0, 8);
    let pid = sys(SYS_GETPID, 0, 0, 0, 0);
    sys(SYS_TGKILL, pid as usize, plat_tid() as usize, sig as usize, 0); // pending until we return
}
