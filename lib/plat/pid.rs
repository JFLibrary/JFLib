// Whether a process runs (process_alive) and this process's id (process_id), on every target:
// Windows OpenProcess and a wait of 0 ms, Linux the kill system call with signal 0, Apple and
// Android libc's kill; the browser has no processes.

/// This process's id, as `std::process::id()` gives it (for apps without std); 0 in the browser.
pub fn process_id() -> u32 {
    imp::id()
}

/// Whether a process with this id runs now: the owner of a lock file, a helper started earlier.
/// A process of another user that this one may not open or signal counts too.  Ids are reused:
/// once a process ended, another one may get its id, so a lock file should hold more than the
/// id where that matters (the program, its start time).  A child of this process that ended
/// still counts on Linux, Apple and Android until it is reaped (`process::Child::try_wait`
/// keeps it, dropping the `Child` reaps it).  Inside a Flatpak sandbox only the sandbox's own
/// processes are seen (its PID namespace).  0 is never a process; the browser has none (false).
pub fn process_alive(pid: u32) -> bool {
    pid != 0 && imp::alive(pid)
}

#[cfg(windows)]
mod imp {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetCurrentProcessId() -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn WaitForSingleObject(h: isize, ms: u32) -> u32;
        fn CloseHandle(h: isize) -> i32;
        fn GetLastError() -> u32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const WAIT_TIMEOUT: u32 = 258;
    const ERROR_ACCESS_DENIED: u32 = 5;

    pub(super) fn id() -> u32 {
        // SAFETY: no arguments, no preconditions.
        unsafe { GetCurrentProcessId() }
    }

    pub(super) fn alive(pid: u32) -> bool {
        // SAFETY: no pointers; the handle is closed below.  An ended process that others still
        // hold opens too: its handle is signalled, the wait below tells it apart.
        let h = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h == 0 {
            // one this process may not open (another user's, a protected one) runs; a missing
            // one gives ERROR_INVALID_PARAMETER
            // SAFETY: no arguments.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        // SAFETY: h is an open process handle with SYNCHRONIZE, closed once.
        unsafe {
            let running = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
            CloseHandle(h);
            running
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use rt::os::{syscall3, SYS_GETPID, SYS_KILL};
    const EPERM: isize = 1;

    pub(super) fn id() -> u32 {
        // SAFETY: getpid has no arguments and cannot fail.
        unsafe { syscall3(SYS_GETPID, 0, 0, 0) as u32 }
    }

    pub(super) fn alive(pid: u32) -> bool {
        // above i32::MAX the kernel's pid_t is negative: a process group, never one process
        if pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: signal 0 sends nothing, it only checks; a pid above 0 names one process.
        let r = unsafe { syscall3(SYS_KILL, pid as usize, 0, 0) };
        r == 0 || r == -EPERM
    }
}

#[cfg(any(target_vendor = "apple", target_os = "android"))]
mod imp {
    #[cfg_attr(target_vendor = "apple", link(name = "System"))]
    #[cfg_attr(target_os = "android", link(name = "c"))]
    unsafe extern "C" {
        fn getpid() -> i32;
        fn kill(pid: i32, sig: i32) -> i32;
        #[cfg_attr(target_vendor = "apple", link_name = "__error")]
        #[cfg_attr(target_os = "android", link_name = "__errno")]
        fn errno_location() -> *mut i32;
    }
    const EPERM: i32 = 1;

    pub(super) fn id() -> u32 {
        // SAFETY: getpid has no arguments and cannot fail.
        unsafe { getpid() as u32 }
    }

    pub(super) fn alive(pid: u32) -> bool {
        // above i32::MAX pid_t is negative: a process group, never one process
        if pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: signal 0 sends nothing, it only checks; errno is this thread's.
        unsafe { kill(pid as i32, 0) == 0 || *errno_location() == EPERM }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    pub(super) fn id() -> u32 {
        0
    }

    pub(super) fn alive(_pid: u32) -> bool {
        false
    }
}
