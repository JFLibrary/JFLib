// t_linux -- sandboxtest's Linux checks: raw system calls that bypass the in-process policy, so
// a refusal here is the kernel (Landlock / seccomp) enforcing the manifest, not plat::policy.
// Guarded by plat::sandbox::status(): a layer the kernel does not support is skipped.

use alloc::vec::Vec;

use plat::sandbox::Status;
use rt::os::{syscall3, syscall6, AT_FDCWD, O_CREAT, O_DIRECTORY, O_RDONLY, O_RDWR, SYS_CLOSE, SYS_CONNECT, SYS_EXECVE, SYS_EXIT_GROUP, SYS_OPENAT, SYS_PTRACE, SYS_SOCKET, SYS_WAIT4};

use crate::T;



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
        deny_list(t, st);
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
    // SAFETY: the child uses only the preallocated buffers, then execve
    // (denied by seccomp) and exit_group.
    let pid = unsafe { rt::os::fork() };
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

/// a program against glibc: what a GPU driver needs under the sandbox (the loader's and the
/// driver's files, the render node), then the driver itself: a device, a buffer through the GPU.
/// Without a driver a SKIP.  The windows' output needs nothing more (lib/ui/gpu_out.rs)
#[cfg(jfgui_glibc)]
pub fn gpu_checks(t: &mut T) {
    for dir in ["/usr/share/vulkan/icd.d", "/usr/lib", "/usr/lib64", "/etc/vulkan"] {
        let r = open(dir, O_RDONLY | O_DIRECTORY);
        t.check(r != EACCES, &alloc::format!("sandboxtest: a driver can read {dir} ({r})"));
    }
    let r = open("/dev/dri/renderD128", O_RDWR);
    // (a node only its group may open, WSL: nobody's fault here; the sandbox would refuse the same way)
    let mut st = rt::os::Stat::default();
    let path = z("/dev/dri/renderD128");
    // SAFETY: a NUL-terminated path, st is a struct stat.
    let open_to_all = unsafe { syscall6(rt::os::SYS_NEWFSTATAT, AT_FDCWD, path.as_ptr() as usize, &mut st as *mut rt::os::Stat as usize, 0, 0, 0) } == 0 && st.st_mode & 0o006 == 0o006;
    if open_to_all {
        t.check(r != EACCES, &alloc::format!("sandboxtest: a driver can open the render node ({r})"));
    } else {
        t.skip("sandboxtest: the render node: no node for everybody here");
    }
    let inst = match gpu::Instance::new() {
        Ok(i) => i,
        Err(e) => {
            t.skip(&alloc::format!("sandboxtest: the GPU under the sandbox: {e}"));
            return;
        }
    };
    let adapter = match inst.adapter() {
        Ok(a) => a,
        Err(e) => {
            t.skip(&alloc::format!("sandboxtest: the GPU under the sandbox: {e}"));
            return;
        }
    };
    let name = adapter.info().name.clone();
    let run = || -> gpu::Result<u32> {
        let dev = adapter.create_device(&gpu::DeviceDesc::DEFAULT)?;
        let a = dev.create_buffer(&gpu::BufferDesc { size: 4096, usage: gpu::BufferUsage::NONE, memory: gpu::MemoryKind::Upload })?;
        let b = dev.create_buffer(&gpu::BufferDesc { size: 4096, usage: gpu::BufferUsage::NONE, memory: gpu::MemoryKind::Readback })?;
        a.write(0, &[0x1234_5678u32])?;
        let mut e = dev.encoder()?;
        e.copy_buffer(&a, 0, &b, 0, 4096);
        e.barrier(gpu::Access::COPY_WRITE, gpu::Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        dev.wait(id, 5_000_000_000)?;
        let mut out = [0u32; 1];
        b.read(0, &mut out)?;
        Ok(out[0])
    };
    let r = run();
    t.check(matches!(r, Ok(0x1234_5678)), &alloc::format!("sandboxtest: {name} works under the sandbox: {r:x?}"));
}

// --------------------------------------------------------------- every denied call
//
// Each system call of the filter's deny list (plat::sandbox::denied_syscalls, the numbers of this
// processor from rt's kernel table; execve / execveat when no program is allowed) is called once with
// arguments that do no harm and that, without the filter, get another answer than EPERM wherever the
// kernel allows that: a bad pointer (EFAULT), a bad descriptor (EBADF), unknown flags (EINVAL), or a
// call that would succeed and change nothing (0).  With the filter every one must give EPERM.  Some
// check the capability first and give EPERM without the filter too (kexec, modules, pivot_root; bpf
// and open_by_handle_at on older kernels): for them EPERM shows that the call fails, not who refused
// it.  A deny-list entry without a probe here is a failure.

const PEEKDATA: usize = 2;
const NO_FD: usize = usize::MAX;

/// (name, number, arguments, the answer without the filter)
fn probes() -> Vec<(&'static str, usize, [usize; 5], &'static str)> {
    use rt::os::*;
    // SAFETY: getpid has no arguments.
    let pid = unsafe { syscall3(SYS_GETPID, 0, 0, 0) } as usize;
    alloc::vec![
        ("ptrace", SYS_PTRACE, [PEEKDATA, 0, 0, 0, 0], "ESRCH: pid 0"),
        ("process_vm_readv", SYS_PROCESS_VM_READV, [pid, 0, 0, 0, 0], "0: nothing to read"),
        ("process_vm_writev", SYS_PROCESS_VM_WRITEV, [pid, 0, 0, 0, 0], "0: nothing to write"),
        ("kexec_load", SYS_KEXEC_LOAD, [0; 5], "EPERM too: CAP_SYS_BOOT"),
        ("kexec_file_load", SYS_KEXEC_FILE_LOAD, [NO_FD, NO_FD, 0, 0, 0], "EPERM too: CAP_SYS_BOOT"),
        ("init_module", SYS_INIT_MODULE, [0; 5], "EPERM too: CAP_SYS_MODULE"),
        ("finit_module", SYS_FINIT_MODULE, [NO_FD, 0, 0, 0, 0], "EPERM too: CAP_SYS_MODULE"),
        ("delete_module", SYS_DELETE_MODULE, [0; 5], "EPERM too: CAP_SYS_MODULE"),
        ("mount", SYS_MOUNT, [0; 5], "EFAULT: no target"),
        ("umount2", SYS_UMOUNT2, [0, 0x8000_0000, 0, 0, 0], "EINVAL: unknown flag"),
        ("pivot_root", SYS_PIVOT_ROOT, [0; 5], "EPERM too: CAP_SYS_ADMIN"),
        ("chroot", SYS_CHROOT, [0; 5], "EFAULT: no path"),
        ("bpf", SYS_BPF, [0x7FFF_FFFF, 0, 0, 0, 0], "EINVAL: unknown command (EPERM before Linux 5.19 without CAP_BPF)"),
        ("perf_event_open", SYS_PERF_EVENT_OPEN, [0, 0, NO_FD, NO_FD, 0x8000_0000], "EINVAL: unknown flag"),
        ("userfaultfd", SYS_USERFAULTFD, [0xFFFF_FFFF, 0, 0, 0, 0], "EINVAL: unknown flags"),
        ("keyctl", SYS_KEYCTL, [0x7FFF_FFFF, 0, 0, 0, 0], "EOPNOTSUPP: unknown operation"),
        ("add_key", SYS_ADD_KEY, [0; 5], "EFAULT: no type"),
        ("request_key", SYS_REQUEST_KEY, [0; 5], "EFAULT: no type"),
        ("open_by_handle_at", SYS_OPEN_BY_HANDLE_AT, [NO_FD, 0, 0, 0, 0], "EBADF / EFAULT (EPERM before Linux 6.9)"),
        ("setns", SYS_SETNS, [NO_FD, 0, 0, 0, 0], "EBADF"),
        ("unshare", SYS_UNSHARE, [0; 5], "0: nothing to unshare"),
        ("execve", SYS_EXECVE, [0; 5], "EFAULT: no program"),
        ("execveat", SYS_EXECVEAT, [NO_FD, 0, 0, 0, 0], "EFAULT / EBADF: no program"),
    ]
}

fn deny_list(t: &mut T, st: &Status) {
    let probes = probes();
    let mut list: Vec<usize> = plat::sandbox::denied_syscalls().to_vec();
    if st.exec_denied {
        list.extend([rt::os::SYS_EXECVE, rt::os::SYS_EXECVEAT]);
    }
    let mut refused = 0;
    for &nr in &list {
        let Some(&(name, _, a, without)) = probes.iter().find(|p| p.1 == nr) else {
            t.check(false, &alloc::format!("sandboxtest: a probe for denied system call {nr}"));
            continue;
        };
        // SAFETY: arguments that do no harm without the filter (see above): null pointers, no
        // descriptor, unknown flags, or nothing to do.
        let r = unsafe { syscall6(nr, a[0], a[1], a[2], a[3], a[4], 0) };
        refused += (r == EPERM) as usize;
        t.check(r == EPERM, &alloc::format!("sandboxtest: seccomp refuses {name} ({nr}): {r}, without the filter {without}"));
    }
    t.check(refused == list.len() && list.len() >= 21,
            &alloc::format!("sandboxtest: all {} denied system calls fail with EPERM", list.len()));
}
