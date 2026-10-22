// sandbox -- OS-level confinement that mirrors the permission manifest (docs/PERMISSIONS.md
// "OS sandbox"), as defence in depth for the in-process policy (policy.rs): a bug or native
// code can bypass `plat`, the kernel cannot be bypassed.  Opt in with a `sandbox` (best effort)
// or `sandbox strict` (refuse to start when the OS cannot enforce it) line in the manifest.
//
//   apply()          install the sandbox for the active policy, once, at start-up (boot.rs,
//                    before the app's own code runs); nothing without a `sandbox` line
//   status() -> Status   what was applied (tests, a diagnostics page)
//
// Linux: Landlock restricts the filesystem to the folders the manifest grants plus the system
// paths the library reads (ABI detected, best effort on older kernels), and TCP bind / connect
// to the manifest's ports (ABI 4+); seccomp-bpf denies a list of dangerous syscalls (ptrace,
// process_vm_*, kexec, module loading, mount, bpf, perf_event_open, userfaultfd, keyctl ...)
// and execve when no program is allowed.  Windows: process mitigation policies (image load,
// extension points, dynamic code; strict adds strict handle checks and child-process
// creation).  The browser and Android have no such API; apply() does nothing there.

/// what [`apply`] installed (the fields not relevant to this OS stay false / -1)
#[derive(Clone, Copy)]
pub struct Status {
    /// 0 none, 1 `sandbox`, 2 `sandbox strict`
    pub level: u8,
    /// Linux: the Landlock ABI the kernel reports (-1 = no Landlock)
    pub landlock_abi: i32,
    /// Linux: the filesystem ruleset is enforced
    pub landlock_fs: bool,
    /// Linux: TCP bind / connect is restricted (Landlock ABI 4+)
    pub landlock_net: bool,
    /// Linux: the seccomp filter is installed
    pub seccomp: bool,
    /// execve is denied (no program allowed): Linux seccomp, Windows child-process policy
    pub exec_denied: bool,
    /// Windows: the process mitigation policies were set
    pub win_mitigations: bool,
    /// Windows: dynamic code is prohibited (no native plugins)
    pub win_dynamic_code: bool,
}

impl Status {
    /// nothing applied
    pub const NONE: Status = Status {
        level: 0,
        landlock_abi: -1,
        landlock_fs: false,
        landlock_net: false,
        seccomp: false,
        exec_denied: false,
        win_mitigations: false,
        win_dynamic_code: false,
    };
}

impl Default for Status {
    fn default() -> Status {
        Status::NONE
    }
}

#[cfg(any(windows, target_os = "linux"))]
static STATUS: rt::sync::SpinLock<Status> = rt::sync::SpinLock::new(Status::NONE);

/// install the OS sandbox the active policy asks for (nothing without a `sandbox` line); call
/// once, before the app's own code and before any worker thread is started
#[cfg(target_os = "linux")]
pub fn apply() {
    let plan = crate::policy::sandbox_plan_linux();
    if plan.level == 0 {
        return;
    }
    let s = crate::linux_sandbox::apply(&plan);
    *STATUS.lock() = s;
}

/// install the OS sandbox the active policy asks for (nothing without a `sandbox` line)
#[cfg(windows)]
pub fn apply() {
    let plan = crate::policy::sandbox_plan_win();
    if plan.level == 0 {
        return;
    }
    let s = crate::win_sandbox::apply(&plan);
    *STATUS.lock() = s;
}

/// the browser and Android have no such API
#[cfg(not(any(windows, target_os = "linux")))]
pub fn apply() {}

/// what [`apply`] installed
#[cfg(any(windows, target_os = "linux"))]
pub fn status() -> Status {
    *STATUS.lock()
}

/// the browser and Android install nothing
#[cfg(not(any(windows, target_os = "linux")))]
pub fn status() -> Status {
    Status::default()
}
