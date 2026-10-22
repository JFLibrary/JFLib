// t_linux -- sandboxtest's Linux checks: raw system calls that bypass the in-process policy, so
// a refusal here is the kernel (Landlock / seccomp) enforcing the manifest, not plat::policy.
// Guarded by plat::sandbox::status(): a layer the kernel does not support is skipped.

use alloc::vec::Vec;

use plat::sandbox::Status;
use rt::os::{syscall3, syscall6};

use crate::T;

const SYS_CLOSE: usize = 3;
const SYS_FORK: usize = 57;
const SYS_EXECVE: usize = 59;
const SYS_WAIT4: usize = 61;
const SYS_PTRACE: usize = 101;
const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_OPENAT: usize = 257;
const SYS_EXIT_GROUP: usize = 231;

const AT_FDCWD: usize = -100i64 as usize;
const O_RDONLY: usize = 0;
const O_RDWR: usize = 2;
const O_CREAT: usize = 0x40;
const O_DIRECTORY: usize = 0x1_0000;

const EACCES: isize = -13;
const EPERM: isize = -1;

fn z(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

// openat(AT_FDCWD, path, flags) -> the raw result (fd >= 0, or -errno)
fn open(path: &str, flags: usize) -> isize {
    let p = z(path);
    // SAFETY: NUL-terminated path; mode 0o600 only matters with O_CREAT.
    let r = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, flags, 0o600, 0, 0) };
    if r >= 0 {
        // SAFETY: an owned descriptor just returned.
        unsafe { syscall3(SYS_CLOSE, r as usize, 0, 0) };
    }
    r
}

pub fn checks(t: &mut T, _dir: &str, st: &Status) {
    // --- allowed: the system paths the library reads ---
    t.check(open("/etc/hosts", O_RDONLY) >= 0, "sandboxtest: /etc/hosts is readable (always allowed)");
    t.check(open("/usr/share/fonts", O_RDONLY | O_DIRECTORY) >= 0 || open("/usr", O_RDONLY | O_DIRECTORY) >= 0,
        "sandboxtest: the system font folder is readable");

    // --- forbidden by Landlock: the user's home, outside the grants ---
    if st.landlock_fs {
        if let Some(home) = plat::env("HOME").filter(|h| h.starts_with('/')) {
            t.check(open(&home, O_RDONLY | O_DIRECTORY) == EACCES, "sandboxtest: reading $HOME is blocked by Landlock");
            let out = alloc::format!("{}/.jfgui-sbxtest-{:x}", home, plat::time_us() as u32);
            let r = open(&out, O_RDWR | O_CREAT);
            t.check(r == EACCES, "sandboxtest: writing under $HOME is blocked by Landlock");
            if r >= 0 {
                let _ = plat::delete(&out);
            }
        } else {
            t.skip("sandboxtest: no $HOME to probe");
        }
    } else {
        t.skip("sandboxtest: Landlock filesystem not enforced (old kernel)");
    }

    // --- forbidden by Landlock net: a TCP connection the manifest does not allow ---
    if st.landlock_net {
        t.check(tcp_connect([127, 0, 0, 1], 9) == EACCES, "sandboxtest: a TCP connection is blocked by Landlock");
    } else {
        t.skip("sandboxtest: Landlock network not enforced");
    }

    // --- forbidden by seccomp: ptrace and execve (no process rule) ---
    if st.seccomp {
        // SAFETY: ptrace(PTRACE_TRACEME) -- denied by the filter, so it has no effect.
        let r = unsafe { syscall6(SYS_PTRACE, 0, 0, 0, 0, 0, 0) };
        t.check(r == EPERM, "sandboxtest: ptrace is blocked by seccomp");
        if st.exec_denied {
            t.check(exec_blocked(), "sandboxtest: execve is blocked by seccomp (no program allowed)");
        } else {
            t.skip("sandboxtest: the manifest allows programs, execve not denied");
        }
    } else {
        t.skip("sandboxtest: seccomp not installed");
    }
}

// socket(AF_INET, SOCK_STREAM) + connect to ip:port -> connect's raw result
fn tcp_connect(ip: [u8; 4], port: u16) -> isize {
    // SAFETY: socket(AF_INET=2, SOCK_STREAM=1, 0); the descriptor is closed below.
    let fd = unsafe { syscall3(SYS_SOCKET, 2, 1, 0) };
    if fd < 0 {
        return fd;
    }
    // sockaddr_in: family (LE u16), port (BE u16), addr (4 bytes), 8 bytes padding
    let mut sa = [0u8; 16];
    sa[0] = 2; // AF_INET
    sa[2] = (port >> 8) as u8;
    sa[3] = port as u8;
    sa[4..8].copy_from_slice(&ip);
    // SAFETY: a 16-byte sockaddr_in and an owned socket descriptor.
    let r = unsafe { syscall3(SYS_CONNECT, fd as usize, sa.as_ptr() as usize, 16) };
    // SAFETY: closing the owned socket.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
    r
}

// fork a child that tries to execve /bin/sh; true when the child was blocked (exit code 42,
// set after a failed execve), false when /bin/sh actually ran (exit code 7)
fn exec_blocked() -> bool {
    let sh = z("/bin/sh");
    let c = z("-c");
    let script = z("exit 7");
    // argv and envp are built before the fork; the child only makes syscalls
    let argv = [sh.as_ptr() as usize, c.as_ptr() as usize, script.as_ptr() as usize, 0];
    let envp = [0usize];
    // SAFETY: fork has no arguments; the child uses only the preallocated buffers, then execve
    // (denied by seccomp) and exit_group.
    let pid = unsafe { syscall3(SYS_FORK, 0, 0, 0) };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        // SAFETY: NUL-terminated argv / envp arrays that outlive this child.
        unsafe {
            syscall3(SYS_EXECVE, sh.as_ptr() as usize, argv.as_ptr() as usize, envp.as_ptr() as usize);
            syscall3(SYS_EXIT_GROUP, 42, 0, 0); // reached only when execve was refused
        }
        loop {}
    }
    let mut status: i32 = 0;
    // SAFETY: reap this child; status is a plain out-parameter.
    while unsafe { syscall6(SYS_WAIT4, pid as usize, &mut status as *mut i32 as usize, 0, 0, 0, 0) } == -4 {}
    let exited = status & 0x7f == 0;
    let code = (status >> 8) & 0xff;
    exited && code == 42
}
