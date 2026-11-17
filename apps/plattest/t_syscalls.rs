// t_syscalls -- plattest --test syscalls (Android; docs/ANDROID.md, AN0): what the seccomp filter of
// an app lets through.  Every system call of rt's kernel table (lib/rt/kernel.rs) runs in a child of
// its own (Bionic's fork), with arguments that can do nothing: a bad descriptor, a null or bad pointer,
// flags the kernel refuses, a process that does not exist.  The call either reaches the kernel, which
// answers (mostly with an error), or the filter ends the child with SIGSYS: Zygote's filter traps,
// it never answers ENOSYS.  The child sets the fatal signals back to their defaults first, so nothing
// leaves a tombstone.
//
// `adb shell` has no filter: everything reaches the kernel there.  The app's process has one
// (tools/android_console.py plattest --mode app -- --test syscalls); its findings are the table in
// docs/ANDROID.md, AN0.  rt_sigreturn is left out: it returns from every signal handler (Bionic's
// too), and called on its own it restores a garbage frame.
//
// Output: one line per call ("syscall NAME NUMBER: kernel (ERRNO)" or "...: SIGSYS (seccomp)"),
// then the sums.  The checks: every child ended within 3 s, the calls Bionic itself needs reached
// the kernel; a call the filter refuses is a finding, no failure.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::T;

pub fn t_syscalls(t: &mut T) {
    probe::run(t);
}

#[allow(unsafe_code)]
mod probe {
    use super::*;
    use core::ffi::{c_char, c_int, c_long};
    use rt::kernel::*;

    unsafe extern "C" {
        fn fork() -> c_int;
        fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
        fn syscall(number: c_long, ...) -> c_long;
        fn signal(signal: c_int, handler: usize) -> usize;
        fn _exit(code: c_int) -> !;
        fn __errno() -> *mut c_int;
        fn kill(pid: c_int, signal: c_int) -> c_int;
        fn usleep(us: u32) -> c_int;
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn read(fd: c_int, buf: *mut u8, count: usize) -> isize;
        fn close(fd: c_int) -> c_int;
    }

    const SIGKILL: c_int = 9;
    const SIGSYS: c_int = 31;
    const WNOHANG: c_int = 1;
    const BAD: usize = usize::MAX; // -1: a bad descriptor, pointer, size or flag set

    /// every call of rt's kernel table (lib/rt/kernel.rs) but rt_sigreturn
    const CALLS: &[(&str, usize)] = &[
        ("read", SYS_READ),
        ("write", SYS_WRITE),
        ("close", SYS_CLOSE),
        ("fstat", SYS_FSTAT),
        ("lseek", SYS_LSEEK),
        ("mmap", SYS_MMAP),
        ("mprotect", SYS_MPROTECT),
        ("munmap", SYS_MUNMAP),
        ("rt_sigaction", SYS_RT_SIGACTION),
        ("rt_sigprocmask", SYS_RT_SIGPROCMASK),
        ("ioctl", SYS_IOCTL),
        ("pread64", SYS_PREAD64),
        ("pwrite64", SYS_PWRITE64),
        ("sched_yield", SYS_SCHED_YIELD),
        ("madvise", SYS_MADVISE),
        ("shmget", SYS_SHMGET),
        ("shmat", SYS_SHMAT),
        ("shmctl", SYS_SHMCTL),
        ("getpid", SYS_GETPID),
        ("socket", SYS_SOCKET),
        ("connect", SYS_CONNECT),
        ("sendto", SYS_SENDTO),
        ("recvfrom", SYS_RECVFROM),
        ("sendmsg", SYS_SENDMSG),
        ("recvmsg", SYS_RECVMSG),
        ("shutdown", SYS_SHUTDOWN),
        ("bind", SYS_BIND),
        ("listen", SYS_LISTEN),
        ("getsockname", SYS_GETSOCKNAME),
        ("getpeername", SYS_GETPEERNAME),
        ("setsockopt", SYS_SETSOCKOPT),
        ("getsockopt", SYS_GETSOCKOPT),
        ("clone", SYS_CLONE),
        ("execve", SYS_EXECVE),
        ("exit", SYS_EXIT),
        ("wait4", SYS_WAIT4),
        ("kill", SYS_KILL),
        ("uname", SYS_UNAME),
        ("shmdt", SYS_SHMDT),
        ("fcntl", SYS_FCNTL),
        ("flock", SYS_FLOCK),
        ("fsync", SYS_FSYNC),
        ("ftruncate", SYS_FTRUNCATE),
        ("getcwd", SYS_GETCWD),
        ("chdir", SYS_CHDIR),
        ("fchmod", SYS_FCHMOD),
        ("fchown", SYS_FCHOWN),
        ("sysinfo", SYS_SYSINFO),
        ("ptrace", SYS_PTRACE),
        ("getuid", SYS_GETUID),
        ("geteuid", SYS_GETEUID),
        ("setsid", SYS_SETSID),
        ("sigaltstack", SYS_SIGALTSTACK),
        ("statfs", SYS_STATFS),
        ("pivot_root", SYS_PIVOT_ROOT),
        ("prctl", SYS_PRCTL),
        ("chroot", SYS_CHROOT),
        ("mount", SYS_MOUNT),
        ("umount2", SYS_UMOUNT2),
        ("init_module", SYS_INIT_MODULE),
        ("delete_module", SYS_DELETE_MODULE),
        ("gettid", SYS_GETTID),
        ("futex", SYS_FUTEX),
        ("sched_getaffinity", SYS_SCHED_GETAFFINITY),
        ("getdents64", SYS_GETDENTS64),
        ("clock_gettime", SYS_CLOCK_GETTIME),
        ("exit_group", SYS_EXIT_GROUP),
        ("tgkill", SYS_TGKILL),
        ("kexec_load", SYS_KEXEC_LOAD),
        ("waitid", SYS_WAITID),
        ("add_key", SYS_ADD_KEY),
        ("request_key", SYS_REQUEST_KEY),
        ("keyctl", SYS_KEYCTL),
        ("inotify_add_watch", SYS_INOTIFY_ADD_WATCH),
        ("inotify_rm_watch", SYS_INOTIFY_RM_WATCH),
        ("openat", SYS_OPENAT),
        ("mkdirat", SYS_MKDIRAT),
        ("fchownat", SYS_FCHOWNAT),
        ("newfstatat", SYS_NEWFSTATAT),
        ("unlinkat", SYS_UNLINKAT),
        ("renameat", SYS_RENAMEAT),
        ("linkat", SYS_LINKAT),
        ("symlinkat", SYS_SYMLINKAT),
        ("readlinkat", SYS_READLINKAT),
        ("fchmodat", SYS_FCHMODAT),
        ("faccessat", SYS_FACCESSAT),
        ("ppoll", SYS_PPOLL),
        ("unshare", SYS_UNSHARE),
        ("utimensat", SYS_UTIMENSAT),
        ("accept4", SYS_ACCEPT4),
        ("eventfd2", SYS_EVENTFD2),
        ("dup3", SYS_DUP3),
        ("pipe2", SYS_PIPE2),
        ("inotify_init1", SYS_INOTIFY_INIT1),
        ("perf_event_open", SYS_PERF_EVENT_OPEN),
        ("open_by_handle_at", SYS_OPEN_BY_HANDLE_AT),
        ("setns", SYS_SETNS),
        ("process_vm_readv", SYS_PROCESS_VM_READV),
        ("process_vm_writev", SYS_PROCESS_VM_WRITEV),
        ("finit_module", SYS_FINIT_MODULE),
        ("renameat2", SYS_RENAMEAT2),
        ("seccomp", SYS_SECCOMP),
        ("getrandom", SYS_GETRANDOM),
        ("memfd_create", SYS_MEMFD_CREATE),
        ("kexec_file_load", SYS_KEXEC_FILE_LOAD),
        ("bpf", SYS_BPF),
        ("execveat", SYS_EXECVEAT),
        ("userfaultfd", SYS_USERFAULTFD),
        ("close_range", SYS_CLOSE_RANGE),
        ("openat2", SYS_OPENAT2),
        ("landlock_create_ruleset", SYS_LANDLOCK_CREATE_RULESET),
        ("landlock_add_rule", SYS_LANDLOCK_ADD_RULE),
        ("landlock_restrict_self", SYS_LANDLOCK_RESTRICT_SELF),
        #[cfg(target_arch = "x86_64")]
        ("arch_prctl", SYS_ARCH_PRCTL),
    ];

    /// the arguments of a call: all -1, but where -1 would mean "everything" or the child should end
    fn args(name: &str) -> [usize; 6] {
        match name {
            "exit" | "exit_group" => [0; 6],                       // the child ends with 0
            "kill" => [i32::MAX as usize, 0, 0, 0, 0, 0],          // signal 0 to no process: ESRCH
            "clone" => [0x800, 0, 0, 0, 0, 0],                     // CLONE_SIGHAND without CLONE_VM: EINVAL
            "futex" => [0, 1, 0, 0, 0, 0],                         // FUTEX_WAKE at address 0: EFAULT
            "close_range" => [BAD, 0, 0, 0, 0, 0],                 // first above last: EINVAL
            _ => [BAD; 6],
        }
    }

    /// what the child's exit code means (the errno of the call, 0: it returned a value)
    fn errno_name(code: c_int) -> String {
        let name = match code {
            0 => "returned",
            1 => "EPERM",
            2 => "ENOENT",
            3 => "ESRCH",
            5 => "EIO",
            7 => "E2BIG",
            9 => "EBADF",
            10 => "ECHILD",
            12 => "ENOMEM",
            13 => "EACCES",
            14 => "EFAULT",
            22 => "EINVAL",
            25 => "ENOTTY",
            34 => "ERANGE",
            36 => "ENAMETOOLONG",
            38 => "ENOSYS",
            88 => "ENOTSOCK",
            93 => "EPROTONOSUPPORT",
            95 => "EOPNOTSUPP",
            97 => "EAFNOSUPPORT",
            _ => return format!("errno {code}"),
        };
        String::from(name)
    }

    enum Outcome {
        Kernel(c_int),
        Trapped,
        Signal(c_int),
        Hung,
        NoChild,
    }

    /// one call in a child of its own
    fn probe(nr: usize, a: [usize; 6]) -> Outcome {
        // SAFETY: fork in a process with threads: the child only calls async-signal-safe functions
        // (signal, syscall, _exit) and touches no lock; the parent waits for it below.
        let pid = unsafe { fork() };
        if pid == 0 {
            // SAFETY: the child: default actions for the fatal signals (no tombstone through
            // debuggerd's handlers), the call with arguments that can do nothing, the end.
            unsafe {
                for s in [4, 5, 6, 7, 8, 11, SIGSYS] {
                    signal(s, 0);
                }
                let r = syscall(nr as c_long, a[0] as c_long, a[1] as c_long, a[2] as c_long, a[3] as c_long, a[4] as c_long, a[5] as c_long);
                _exit(if r == -1 { (*__errno()).clamp(1, 250) } else { 0 })
            }
        }
        if pid < 0 {
            return Outcome::NoChild;
        }
        let mut status: c_int = 0;
        for _ in 0..300 {
            // SAFETY: our own child; status is writable.
            let r = unsafe { waitpid(pid, &mut status, WNOHANG) };
            if r == pid {
                let sig = status & 0x7f;
                return match sig {
                    0 => Outcome::Kernel((status >> 8) & 0xff),
                    SIGSYS => Outcome::Trapped,
                    _ => Outcome::Signal(sig),
                };
            }
            // SAFETY: no memory involved.
            unsafe { usleep(10_000) };
        }
        // SAFETY: our own child, ended and reaped.
        unsafe {
            kill(pid, SIGKILL);
            waitpid(pid, &mut status, 0);
        }
        Outcome::Hung
    }

    /// the Seccomp line of /proc/self/status (0 none, 2 a filter)
    fn seccomp() -> String {
        let mut buf = [0u8; 4096];
        // SAFETY: a NUL-terminated path, a buffer of its length, our own descriptor.
        let n = unsafe {
            let fd = open(c"/proc/self/status".as_ptr(), 0x80000);
            if fd < 0 {
                return String::from("?");
            }
            let n = read(fd, buf.as_mut_ptr(), buf.len());
            close(fd);
            n
        };
        let text = String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned();
        text.lines().find_map(|l| l.strip_prefix("Seccomp:")).map_or(String::from("?"), |v| String::from(v.trim()))
    }

    pub fn run(t: &mut T) {
        t.info(&format!("syscalls: seccomp mode {} (0: none, 2: a filter)\n", seccomp()));
        let (mut kernel, mut trapped, mut odd) = (Vec::new(), Vec::new(), Vec::new());
        for &(name, nr) in CALLS {
            let what = match probe(nr, args(name)) {
                Outcome::Kernel(code) => {
                    kernel.push(name);
                    format!("kernel ({})", errno_name(code))
                }
                Outcome::Trapped => {
                    trapped.push(name);
                    String::from("SIGSYS (seccomp)")
                }
                Outcome::Signal(s) => {
                    odd.push(name);
                    format!("signal {s}")
                }
                Outcome::Hung => {
                    odd.push(name);
                    String::from("still running after 3 s")
                }
                Outcome::NoChild => {
                    odd.push(name);
                    String::from("fork failed")
                }
            };
            crate::out(&format!("syscall {name} {nr}: {what}\n"));
        }
        crate::out(&format!("syscalls: {} reach the kernel, {} end with SIGSYS: {}\n", kernel.len(), trapped.len(), trapped.join(" ")));
        t.check(odd.is_empty(), "syscalls: every child ended by itself or by SIGSYS");
        if !odd.is_empty() {
            crate::out(&format!("  (other ends: {})\n", odd.join(" ")));
        }
        // what Bionic itself calls all the time: a filter that refused these would end every app
        let bionic = ["read", "write", "close", "mmap", "munmap", "futex", "openat", "clock_gettime", "getpid", "gettid", "exit_group"];
        t.check(bionic.iter().all(|b| kernel.contains(b)), "syscalls: the calls Bionic itself makes reach the kernel");
    }
}
