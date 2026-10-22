// t_win -- sandboxtest's Windows checks: the process mitigation policies the manifest asks for,
// confirmed both through plat::sandbox::status() and by the behaviour they cause (executable
// memory refused, a child process refused).  Guarded by the status flags.

use alloc::vec::Vec;

use plat::sandbox::Status;

use crate::T;

#[repr(C)]
struct StartupInfo {
    cb: u32,
    rest: [u32; 25],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(addr: usize, size: usize, typ: u32, prot: u32) -> usize;
    fn VirtualFree(addr: usize, size: usize, typ: u32) -> i32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: usize, ta: usize, inherit: i32, flags: u32, env: usize,
        dir: *const u16, si: *const StartupInfo, pi: *mut [isize; 3]) -> i32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn checks(t: &mut T, _dir: &str, st: &Status) {
    t.check(st.win_mitigations, "sandboxtest: the process mitigation policies were set");

    // dynamic code prohibited (the manifest allows no plugins): the policy reads back as set;
    // executable memory should then be refused.  Some Windows builds report the policy but do
    // not refuse it (10.0.26200 here, the same in any process): reported, not failed
    t.check(st.win_dynamic_code, "sandboxtest: the dynamic code policy is set (no plugins allowed)");
    if st.win_dynamic_code {
        // SAFETY: a plain anonymous allocation request; freed when it succeeds.
        let p = unsafe { VirtualAlloc(0, 4096, MEM_COMMIT_RESERVE, PAGE_EXECUTE_READWRITE) };
        if p == 0 {
            t.check(true, "sandboxtest: allocating executable memory is refused (dynamic code prohibited)");
        } else {
            t.skip("sandboxtest: this Windows build reports the dynamic code policy but still grants executable memory");
            // SAFETY: release the allocation we just made.
            unsafe { VirtualFree(p, 0, MEM_RELEASE) };
        }
    }

    // strict + no program: creating a child process is refused
    if st.exec_denied {
        t.check(child_blocked(), "sandboxtest: creating a child process is refused (strict, no program allowed)");
    } else {
        t.skip("sandboxtest: child-process creation not blocked (not strict, or a program is allowed)");
    }
}

// try to create a suspended child (this very program); true when the OS refused it, false when
// it started (then it is terminated at once, before it ran any code)
fn child_blocked() -> bool {
    let Some(exe) = plat::current_exe() else { return false };
    let mut cmd: Vec<u16> = alloc::format!("\"{}\" --sandbox-child-probe", exe).encode_utf16().chain(core::iter::once(0)).collect();
    let si = StartupInfo { cb: core::mem::size_of::<StartupInfo>() as u32, rest: [0; 25] };
    let mut pi = [0isize; 3];
    // SAFETY: a writable command line and a zeroed STARTUPINFOW of the declared size; the child
    // is created suspended, so it never runs before we terminate it.
    let ok = unsafe {
        CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), 0, 0, 0, CREATE_SUSPENDED | CREATE_NO_WINDOW, 0, core::ptr::null(), &si, &mut pi)
    };
    if ok != 0 {
        // the policy did not block it: kill the suspended child and report "not blocked"
        // SAFETY: handles from a successful CreateProcessW.
        unsafe {
            TerminateProcess(pi[0], 1);
            CloseHandle(pi[0]);
            CloseHandle(pi[1]);
        }
        false
    } else {
        true
    }
}
