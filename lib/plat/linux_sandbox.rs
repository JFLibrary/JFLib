// linux_sandbox -- the Linux half of plat::sandbox (docs/PERMISSIONS.md "OS sandbox"): Landlock
// for the filesystem and TCP, seccomp-bpf for a deny list of dangerous syscalls.  Raw syscalls,
// no libc.  Installed once at start-up (sandbox::apply, through boot.rs) on the only thread;
// Landlock and seccomp are inherited by the threads started afterwards.
//
//   apply(&LinuxPlan) -> Status   NO_NEW_PRIVS, then the Landlock ruleset the plan grants
//                                 (ABI detected), then the seccomp filter; `strict` exits the
//                                 process when Landlock or seccomp could not be installed.

use alloc::string::String;
use alloc::vec::Vec;

use rt::os::{syscall3, syscall6};

use crate::policy::LinuxPlan;
use crate::sandbox::Status;

// syscall numbers (x86-64)
const SYS_OPENAT: usize = 257;
const SYS_CLOSE: usize = 3;
const SYS_FSTAT: usize = 5;
const SYS_PRCTL: usize = 157;
const SYS_SECCOMP: usize = 317;
const SYS_LANDLOCK_CREATE_RULESET: usize = 444;
const SYS_LANDLOCK_ADD_RULE: usize = 445;
const SYS_LANDLOCK_RESTRICT_SELF: usize = 446;

const AT_FDCWD: usize = -100i64 as usize;
const O_PATH: usize = 0x20_0000;
const O_CLOEXEC: usize = 0x8_0000;

const PR_SET_NO_NEW_PRIVS: usize = 38;

// Landlock filesystem access rights
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;
const FS_BASE: u64 = 0x1FFF; // bits 0..12 (read, write, remove, make*)
const FS_READ: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;
const ACCESS_FILE: u64 = FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE | FS_TRUNCATE; // the rights a file can carry
const S_IFMT: u32 = 0xF000;
const S_IFDIR: u32 = 0x4000;

// Landlock network access rights (ABI 4+)
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;

const LANDLOCK_CREATE_RULESET_VERSION: usize = 1;
const RULE_PATH_BENEATH: usize = 1;
const RULE_NET_PORT: usize = 2;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

#[repr(C)]
struct NetPortAttr {
    allowed_access: u64,
    port: u64,
}

// the Landlock ABI the kernel reports, or -1 when Landlock is missing
fn landlock_abi() -> i32 {
    // SAFETY: the version query takes a null attr and size 0.
    let r = unsafe { syscall3(SYS_LANDLOCK_CREATE_RULESET, 0, 0, LANDLOCK_CREATE_RULESET_VERSION) };
    if r < 0 {
        -1
    } else {
        r as i32
    }
}

fn z(path: &str) -> Vec<u8> {
    let mut v = path.as_bytes().to_vec();
    v.push(0);
    v
}

// an O_PATH descriptor of path, or -1 (missing / no access)
fn open_path(path: &str) -> i32 {
    let p = z(path);
    // SAFETY: NUL-terminated path; O_PATH opens files and folders without reading them.
    let r = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, O_PATH | O_CLOEXEC, 0, 0, 0) };
    if r < 0 {
        -1
    } else {
        r as i32
    }
}

fn close(fd: i32) {
    // SAFETY: an owned descriptor opened by open_path / create_ruleset.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
}

// grant the manifest's folders and the system paths the library reads, and the TCP ports of its
// net rules; st records whether the ruleset is enforced
fn landlock(plan: &LinuxPlan, abi: i32, st: &mut Status) {
    let handled_fs = FS_BASE | if abi >= 2 { FS_REFER } else { 0 } | if abi >= 3 { FS_TRUNCATE } else { 0 };
    let handle_net = abi >= 4;
    let net = if handle_net {
        (if plan.connect_restrict { NET_CONNECT_TCP } else { 0 }) | (if plan.listen_restrict { NET_BIND_TCP } else { 0 })
    } else {
        0
    };
    let attr = RulesetAttr { handled_access_fs: handled_fs, handled_access_net: net };
    let size = if handle_net { 16 } else { 8 };
    // SAFETY: a ruleset attribute of the declared size.
    let fd = unsafe { syscall3(SYS_LANDLOCK_CREATE_RULESET, &attr as *const _ as usize, size, 0) };
    if fd < 0 {
        return; // no Landlock (old kernel, LSM off): best effort, nothing enforced
    }
    let fd = fd as i32;

    // the folders the manifest reaches (writable ones created first: a rule needs an existing
    // path), plus the system paths the library reads
    for d in &plan.make_dirs {
        if crate::file_exists_raw(d) != 2 {
            crate::mkdir_raw(d);
        }
    }
    for (path, w) in &plan.fs {
        add_path(fd, path, if *w { handled_fs } else { FS_READ });
    }
    for dir in SYSTEM_READ {
        add_path(fd, dir, FS_READ);
    }
    for dir in SYSTEM_WRITE {
        add_path(fd, dir, handled_fs);
    }
    // the X11 display cookie (Wayland needs none; connecting to the socket is not a file op)
    if let Some(x) = crate::env("XAUTHORITY").filter(|x| x.starts_with('/')) {
        add_path(fd, &x, FS_READ);
    } else if let Some(h) = crate::env("HOME") {
        add_path(fd, &alloc::format!("{}/.Xauthority", h), FS_READ);
    }
    if let Some(rt) = crate::env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/')) {
        add_path(fd, &rt, handled_fs); // the documents portal mount and local sockets
    }
    if let Some(tmp) = crate::env("TMPDIR").filter(|d| d.starts_with('/')) {
        add_path(fd, &tmp, handled_fs);
    }

    // the network ports the manifest allows (ABI 4+)
    if handle_net {
        if plan.connect_restrict {
            for &p in &plan.connect_ports {
                add_net(fd, NET_CONNECT_TCP, p);
            }
        }
        if plan.listen_restrict {
            for &p in &plan.listen_ports {
                add_net(fd, NET_BIND_TCP, p);
            }
        }
    }

    // NO_NEW_PRIVS is needed to enforce without privilege
    no_new_privs();
    // SAFETY: an owned ruleset descriptor, no flags.
    let r = unsafe { syscall3(SYS_LANDLOCK_RESTRICT_SELF, fd as usize, 0, 0) };
    close(fd);
    if r == 0 {
        st.landlock_fs = true;
        st.landlock_net = net != 0;
    }
}

fn add_path(ruleset: i32, path: &str, access: u64) {
    let pfd = open_path(path);
    if pfd < 0 {
        return;
    }
    // the kernel rejects (EINVAL) directory-only rights on a file: a granted file keeps its own
    let mut st = [0u64; 18]; // struct stat; st_mode at 24
    // SAFETY: a live descriptor and storage for the x86-64 stat structure.
    let is_dir = unsafe { syscall3(SYS_FSTAT, pfd as usize, st.as_mut_ptr() as usize, 0) } == 0 && st[3] as u32 & S_IFMT == S_IFDIR;
    let access = if is_dir { access } else { access & ACCESS_FILE };
    if access == 0 {
        close(pfd);
        return;
    }
    let attr = PathBeneathAttr { allowed_access: access, parent_fd: pfd };
    // SAFETY: a path_beneath attribute with a live O_PATH descriptor.
    unsafe { syscall6(SYS_LANDLOCK_ADD_RULE, ruleset as usize, RULE_PATH_BENEATH, &attr as *const _ as usize, 0, 0, 0) };
    close(pfd);
}

fn add_net(ruleset: i32, access: u64, port: u16) {
    let attr = NetPortAttr { allowed_access: access, port: port as u64 };
    // SAFETY: a net_port attribute, no flags.
    unsafe { syscall6(SYS_LANDLOCK_ADD_RULE, ruleset as usize, RULE_NET_PORT, &attr as *const _ as usize, 0, 0, 0) };
}

fn no_new_privs() {
    // SAFETY: prctl(PR_SET_NO_NEW_PRIVS, 1) with the required zero arguments.
    unsafe { syscall6(SYS_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0, 0) };
}

// system folders the library may read (fonts, locale, themes, resolver, /proc, /sys, the
// loader paths); non-existent ones are skipped
const SYSTEM_READ: &[&str] = &["/usr", "/etc", "/bin", "/lib", "/lib64", "/sbin", "/opt", "/proc", "/sys", "/run", "/var"];
// folders the library writes to (temp files, device nodes like /dev/null and /dev/dri, the
// WSLg runtime mount)
const SYSTEM_WRITE: &[&str] = &["/tmp", "/dev", "/mnt/wslg"];

// --------------------------------------------------------------- seccomp

#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

fn sf(code: u16, jt: u8, jf: u8, k: u32) -> SockFilter {
    SockFilter { code, jt, jf, k }
}

const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
const X32_BIT: u32 = 0x4000_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7FFF_0000;
const SECCOMP_RET_ERRNO_EPERM: u32 = 0x0005_0001;
const SECCOMP_SET_MODE_FILTER: usize = 1;

// syscalls denied with EPERM: tracing, cross-process memory, kexec, module loading, mount,
// bpf, perf_event_open, userfaultfd, keyrings, handle reopen, namespace juggling
const DENY: &[u32] = &[
    101, // ptrace
    310, 311, // process_vm_readv / writev
    246, 320, // kexec_load / kexec_file_load
    175, 313, 176, // init_module / finit_module / delete_module
    165, 166, 155, 161, // mount / umount2 / pivot_root / chroot
    321, // bpf
    298, // perf_event_open
    323, // userfaultfd
    250, 248, 249, // keyctl / add_key / request_key
    304, // open_by_handle_at
    308, 272, // setns / unshare
];

// install the deny-list filter; execve / execveat are added when no program is allowed
fn seccomp(exec_allowed: bool, st: &mut Status) {
    let mut prog: Vec<SockFilter> = Vec::new();
    let n = DENY.len() + if exec_allowed { 0 } else { 2 };
    let deny_idx = (5 + n) as u8; // the index of the EPERM return
    prog.push(sf(0x20, 0, 0, 4)); // 0: load arch (offset 4)
    prog.push(sf(0x15, 0, deny_idx - 2, AUDIT_ARCH_X86_64)); // 1: arch != x86-64 -> deny
    prog.push(sf(0x20, 0, 0, 0)); // 2: load nr (offset 0)
    prog.push(sf(0x35, deny_idx - 4, 0, X32_BIT)); // 3: nr >= x32 bit -> deny
    let mut i: u8 = 4;
    let mut emit = |k: u32, prog: &mut Vec<SockFilter>| {
        prog.push(sf(0x15, deny_idx - i - 1, 0, k)); // JEQ k -> deny
        i += 1;
    };
    for &k in DENY {
        emit(k, &mut prog);
    }
    if !exec_allowed {
        emit(59, &mut prog); // execve
        emit(322, &mut prog); // execveat
    }
    prog.push(sf(0x06, 0, 0, SECCOMP_RET_ALLOW)); // allow
    prog.push(sf(0x06, 0, 0, SECCOMP_RET_ERRNO_EPERM)); // deny
    let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
    // SAFETY: a classic BPF program that lives for the call; SET_MODE_FILTER, no flags.
    let r = unsafe { syscall3(SYS_SECCOMP, SECCOMP_SET_MODE_FILTER, 0, &fprog as *const _ as usize) };
    if r == 0 {
        st.seccomp = true;
        st.exec_denied = !exec_allowed;
    }
}

// --------------------------------------------------------------- entry

/// install the Landlock ruleset and seccomp filter the plan asks for
pub fn apply(plan: &LinuxPlan) -> Status {
    let mut st = Status { level: plan.level, ..Status::NONE };
    no_new_privs();
    let abi = landlock_abi();
    st.landlock_abi = abi;
    if abi >= 1 {
        landlock(plan, abi, &mut st);
    }
    seccomp(plan.exec_allowed, &mut st);
    // strict: the OS enforcement is the point, so refuse to run without it
    if plan.level >= 2 && !(st.landlock_fs && st.seccomp) {
        let mut msg = String::from("error: sandbox strict: the OS could not enforce the permissions (");
        if !st.landlock_fs {
            msg.push_str("no Landlock");
        }
        if !st.landlock_fs && !st.seccomp {
            msg.push_str(", ");
        }
        if !st.seccomp {
            msg.push_str("no seccomp");
        }
        msg.push_str(")\n");
        crate::write_stderr(msg.as_bytes());
        crate::exit(2);
    }
    st
}
