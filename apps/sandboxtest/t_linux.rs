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
