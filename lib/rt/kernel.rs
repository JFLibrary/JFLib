// kernel -- the Linux kernel ABI, one table per architecture: system call numbers, open flags,
// struct stat, the order of clone's arguments.  Nothing else defines them (build_rs.py lint):
// a wrong number compiles and calls something else (57 is fork on x86-64, close on ARM64), a wrong
// flag means something else (O_NOFOLLOW here is O_LARGEFILE there).  Both tables name the same
// things, line by line; tools/kernel_table_check.py compares them with the kernel's headers.
// Everything is also reached as rt::os::*, which has the helpers for the calls ARM64 does not
// have (poll, fork, dup2, rename).
//
//   SYS_*        system calls; SYS_ARCH_PRCTL only on x86-64
//   O_*          open flags
//   Stat         struct stat; Stat::default() to pass it to fstat / newfstatat
//   clone_args   clone's arguments after flags and stack, in the kernel's order
//
// The same on both architectures: AT_FDCWD, and what is not listed here (AT_* flags, S_IF*, errno
// values, F_*, MAP_*, PROT_*, the clone flags, struct statfs).

pub use arch::*;

/// the current directory as the base of a relative path in the *at calls
pub const AT_FDCWD: usize = -100isize as usize;

#[cfg(target_arch = "x86_64")]
mod arch {
    use core::mem::{offset_of, size_of};

    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_FSTAT: usize = 5;
    pub const SYS_LSEEK: usize = 8;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_MPROTECT: usize = 10;
    pub const SYS_MUNMAP: usize = 11;
    pub const SYS_RT_SIGACTION: usize = 13;
    pub const SYS_RT_SIGPROCMASK: usize = 14;
    pub const SYS_RT_SIGRETURN: usize = 15;
    pub const SYS_IOCTL: usize = 16;
    pub const SYS_PREAD64: usize = 17;
    pub const SYS_PWRITE64: usize = 18;
    pub const SYS_SCHED_YIELD: usize = 24;
    pub const SYS_MADVISE: usize = 28;
    pub const SYS_SHMGET: usize = 29;
    pub const SYS_SHMAT: usize = 30;
    pub const SYS_SHMCTL: usize = 31;
    pub const SYS_GETPID: usize = 39;
    pub const SYS_SOCKET: usize = 41;
    pub const SYS_CONNECT: usize = 42;
    pub const SYS_SENDTO: usize = 44;
    pub const SYS_RECVFROM: usize = 45;
    pub const SYS_SENDMSG: usize = 46;
    pub const SYS_RECVMSG: usize = 47;
    pub const SYS_SHUTDOWN: usize = 48;
    pub const SYS_BIND: usize = 49;
    pub const SYS_LISTEN: usize = 50;
    pub const SYS_GETSOCKNAME: usize = 51;
    pub const SYS_GETPEERNAME: usize = 52;
    pub const SYS_SETSOCKOPT: usize = 54;
    pub const SYS_GETSOCKOPT: usize = 55;
    pub const SYS_CLONE: usize = 56;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_EXIT: usize = 60;
    pub const SYS_WAIT4: usize = 61;
    pub const SYS_KILL: usize = 62;
    pub const SYS_UNAME: usize = 63;
    pub const SYS_SHMDT: usize = 67;
    pub const SYS_FCNTL: usize = 72;
    pub const SYS_FLOCK: usize = 73;
    pub const SYS_FSYNC: usize = 74;
    pub const SYS_FTRUNCATE: usize = 77;
    pub const SYS_GETCWD: usize = 79;
    pub const SYS_CHDIR: usize = 80;
    pub const SYS_FCHMOD: usize = 91;
    pub const SYS_FCHOWN: usize = 93;
    pub const SYS_SYSINFO: usize = 99;
    pub const SYS_PTRACE: usize = 101;
    pub const SYS_GETUID: usize = 102;
    pub const SYS_GETEUID: usize = 107;
    pub const SYS_SETSID: usize = 112;
    pub const SYS_SIGALTSTACK: usize = 131;
    pub const SYS_STATFS: usize = 137;
    pub const SYS_PIVOT_ROOT: usize = 155;
    pub const SYS_PRCTL: usize = 157;
    pub const SYS_CHROOT: usize = 161;
    pub const SYS_MOUNT: usize = 165;
    pub const SYS_UMOUNT2: usize = 166;
    pub const SYS_INIT_MODULE: usize = 175;
    pub const SYS_DELETE_MODULE: usize = 176;
    pub const SYS_GETTID: usize = 186;
    pub const SYS_FUTEX: usize = 202;
    pub const SYS_SCHED_GETAFFINITY: usize = 204;
    pub const SYS_GETDENTS64: usize = 217;
    pub const SYS_CLOCK_GETTIME: usize = 228;
    pub const SYS_EXIT_GROUP: usize = 231;
    pub const SYS_TGKILL: usize = 234;
    pub const SYS_KEXEC_LOAD: usize = 246;
    pub const SYS_WAITID: usize = 247;
    pub const SYS_ADD_KEY: usize = 248;
    pub const SYS_REQUEST_KEY: usize = 249;
    pub const SYS_KEYCTL: usize = 250;
    pub const SYS_INOTIFY_ADD_WATCH: usize = 254;
    pub const SYS_INOTIFY_RM_WATCH: usize = 255;
    pub const SYS_OPENAT: usize = 257;
    pub const SYS_MKDIRAT: usize = 258;
    pub const SYS_FCHOWNAT: usize = 260;
    pub const SYS_NEWFSTATAT: usize = 262;
    pub const SYS_UNLINKAT: usize = 263;
    pub const SYS_RENAMEAT: usize = 264;
    pub const SYS_LINKAT: usize = 265;
    pub const SYS_SYMLINKAT: usize = 266;
    pub const SYS_READLINKAT: usize = 267;
    pub const SYS_FCHMODAT: usize = 268;
    pub const SYS_FACCESSAT: usize = 269;
    pub const SYS_PPOLL: usize = 271;
    pub const SYS_UNSHARE: usize = 272;
    pub const SYS_UTIMENSAT: usize = 280;
    pub const SYS_ACCEPT4: usize = 288;
    pub const SYS_EVENTFD2: usize = 290;
    pub const SYS_DUP3: usize = 292;
    pub const SYS_PIPE2: usize = 293;
    pub const SYS_INOTIFY_INIT1: usize = 294;
    pub const SYS_PERF_EVENT_OPEN: usize = 298;
    pub const SYS_OPEN_BY_HANDLE_AT: usize = 304;
    pub const SYS_SETNS: usize = 308;
    pub const SYS_PROCESS_VM_READV: usize = 310;
    pub const SYS_PROCESS_VM_WRITEV: usize = 311;
    pub const SYS_FINIT_MODULE: usize = 313;
    pub const SYS_RENAMEAT2: usize = 316;
    pub const SYS_SECCOMP: usize = 317;
    pub const SYS_GETRANDOM: usize = 318;
    pub const SYS_MEMFD_CREATE: usize = 319;
    pub const SYS_KEXEC_FILE_LOAD: usize = 320;
    pub const SYS_BPF: usize = 321;
    pub const SYS_EXECVEAT: usize = 322;
    pub const SYS_USERFAULTFD: usize = 323;
    pub const SYS_CLOSE_RANGE: usize = 436;
    pub const SYS_LANDLOCK_CREATE_RULESET: usize = 444;
    pub const SYS_LANDLOCK_ADD_RULE: usize = 445;
    pub const SYS_LANDLOCK_RESTRICT_SELF: usize = 446;
    // x86-64 only: ARM64 sets the thread pointer with TPIDR_EL0 (ARM3)
    pub const SYS_ARCH_PRCTL: usize = 158;

    pub const O_RDONLY: usize = 0;
    pub const O_WRONLY: usize = 1;
    pub const O_RDWR: usize = 2;
    pub const O_CREAT: usize = 0x40;
    pub const O_EXCL: usize = 0x80;
    pub const O_TRUNC: usize = 0x200;
    pub const O_NONBLOCK: usize = 0x800;
    pub const O_DIRECTORY: usize = 0x1_0000;
    pub const O_NOFOLLOW: usize = 0x2_0000;
    pub const O_CLOEXEC: usize = 0x8_0000;
    pub const O_PATH: usize = 0x20_0000;
    pub const O_TMPFILE: usize = 0x40_0000 | O_DIRECTORY;

    /// struct stat of fstat / newfstatat, 144 bytes
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Stat {
        pub st_dev: u64,
        pub st_ino: u64,
        pub st_nlink: u64,
        pub st_mode: u32,
        pub st_uid: u32,
        pub st_gid: u32,
        __pad0: u32,
        pub st_rdev: u64,
        pub st_size: i64,
        pub st_blksize: i64,
        pub st_blocks: i64,
        pub st_atime: i64,
        pub st_atime_nsec: i64,
        pub st_mtime: i64,
        pub st_mtime_nsec: i64,
        pub st_ctime: i64,
        pub st_ctime_nsec: i64,
        __unused: [i64; 3],
    }
    const _: () = assert!(size_of::<Stat>() == 144 && offset_of!(Stat, st_mode) == 24 && offset_of!(Stat, st_size) == 48 && offset_of!(Stat, st_mtime) == 88);

    /// clone(flags, stack, a, b, c): child_tid before tls
    pub const fn clone_args(parent_tid: usize, child_tid: usize, tls: usize) -> [usize; 3] {
        [parent_tid, child_tid, tls]
    }
}

#[cfg(target_arch = "aarch64")]
mod arch {
    use core::mem::{offset_of, size_of};

    pub const SYS_READ: usize = 63;
    pub const SYS_WRITE: usize = 64;
    pub const SYS_CLOSE: usize = 57;
    pub const SYS_FSTAT: usize = 80;
    pub const SYS_LSEEK: usize = 62;
    pub const SYS_MMAP: usize = 222;
    pub const SYS_MPROTECT: usize = 226;
    pub const SYS_MUNMAP: usize = 215;
    pub const SYS_RT_SIGACTION: usize = 134;
    pub const SYS_RT_SIGPROCMASK: usize = 135;
    pub const SYS_RT_SIGRETURN: usize = 139;
    pub const SYS_IOCTL: usize = 29;
    pub const SYS_PREAD64: usize = 67;
    pub const SYS_PWRITE64: usize = 68;
    pub const SYS_SCHED_YIELD: usize = 124;
    pub const SYS_MADVISE: usize = 233;
    pub const SYS_SHMGET: usize = 194;
    pub const SYS_SHMAT: usize = 196;
    pub const SYS_SHMCTL: usize = 195;
    pub const SYS_GETPID: usize = 172;
    pub const SYS_SOCKET: usize = 198;
    pub const SYS_CONNECT: usize = 203;
    pub const SYS_SENDTO: usize = 206;
    pub const SYS_RECVFROM: usize = 207;
    pub const SYS_SENDMSG: usize = 211;
    pub const SYS_RECVMSG: usize = 212;
    pub const SYS_SHUTDOWN: usize = 210;
    pub const SYS_BIND: usize = 200;
    pub const SYS_LISTEN: usize = 201;
    pub const SYS_GETSOCKNAME: usize = 204;
    pub const SYS_GETPEERNAME: usize = 205;
    pub const SYS_SETSOCKOPT: usize = 208;
    pub const SYS_GETSOCKOPT: usize = 209;
    pub const SYS_CLONE: usize = 220;
    pub const SYS_EXECVE: usize = 221;
    pub const SYS_EXIT: usize = 93;
    pub const SYS_WAIT4: usize = 260;
    pub const SYS_KILL: usize = 129;
    pub const SYS_UNAME: usize = 160;
    pub const SYS_SHMDT: usize = 197;
    pub const SYS_FCNTL: usize = 25;
    pub const SYS_FLOCK: usize = 32;
    pub const SYS_FSYNC: usize = 82;
    pub const SYS_FTRUNCATE: usize = 46;
    pub const SYS_GETCWD: usize = 17;
    pub const SYS_CHDIR: usize = 49;
    pub const SYS_FCHMOD: usize = 52;
    pub const SYS_FCHOWN: usize = 55;
    pub const SYS_SYSINFO: usize = 179;
    pub const SYS_PTRACE: usize = 117;
    pub const SYS_GETUID: usize = 174;
    pub const SYS_GETEUID: usize = 175;
    pub const SYS_SETSID: usize = 157;
    pub const SYS_SIGALTSTACK: usize = 132;
    pub const SYS_STATFS: usize = 43;
    pub const SYS_PIVOT_ROOT: usize = 41;
    pub const SYS_PRCTL: usize = 167;
    pub const SYS_CHROOT: usize = 51;
    pub const SYS_MOUNT: usize = 40;
    pub const SYS_UMOUNT2: usize = 39;
    pub const SYS_INIT_MODULE: usize = 105;
    pub const SYS_DELETE_MODULE: usize = 106;
    pub const SYS_GETTID: usize = 178;
    pub const SYS_FUTEX: usize = 98;
    pub const SYS_SCHED_GETAFFINITY: usize = 123;
    pub const SYS_GETDENTS64: usize = 61;
    pub const SYS_CLOCK_GETTIME: usize = 113;
    pub const SYS_EXIT_GROUP: usize = 94;
    pub const SYS_TGKILL: usize = 131;
    pub const SYS_KEXEC_LOAD: usize = 104;
    pub const SYS_WAITID: usize = 95;
    pub const SYS_ADD_KEY: usize = 217;
    pub const SYS_REQUEST_KEY: usize = 218;
    pub const SYS_KEYCTL: usize = 219;
    pub const SYS_INOTIFY_ADD_WATCH: usize = 27;
    pub const SYS_INOTIFY_RM_WATCH: usize = 28;
    pub const SYS_OPENAT: usize = 56;
    pub const SYS_MKDIRAT: usize = 34;
    pub const SYS_FCHOWNAT: usize = 54;
    pub const SYS_NEWFSTATAT: usize = 79;
    pub const SYS_UNLINKAT: usize = 35;
    pub const SYS_RENAMEAT: usize = 38;
    pub const SYS_LINKAT: usize = 37;
    pub const SYS_SYMLINKAT: usize = 36;
    pub const SYS_READLINKAT: usize = 78;
    pub const SYS_FCHMODAT: usize = 53;
    pub const SYS_FACCESSAT: usize = 48;
    pub const SYS_PPOLL: usize = 73;
    pub const SYS_UNSHARE: usize = 97;
    pub const SYS_UTIMENSAT: usize = 88;
    pub const SYS_ACCEPT4: usize = 242;
    pub const SYS_EVENTFD2: usize = 19;
    pub const SYS_DUP3: usize = 24;
    pub const SYS_PIPE2: usize = 59;
    pub const SYS_INOTIFY_INIT1: usize = 26;
    pub const SYS_PERF_EVENT_OPEN: usize = 241;
    pub const SYS_OPEN_BY_HANDLE_AT: usize = 265;
    pub const SYS_SETNS: usize = 268;
    pub const SYS_PROCESS_VM_READV: usize = 270;
    pub const SYS_PROCESS_VM_WRITEV: usize = 271;
    pub const SYS_FINIT_MODULE: usize = 273;
    pub const SYS_RENAMEAT2: usize = 276;
    pub const SYS_SECCOMP: usize = 277;
    pub const SYS_GETRANDOM: usize = 278;
    pub const SYS_MEMFD_CREATE: usize = 279;
    pub const SYS_KEXEC_FILE_LOAD: usize = 294;
    pub const SYS_BPF: usize = 280;
    pub const SYS_EXECVEAT: usize = 281;
    pub const SYS_USERFAULTFD: usize = 282;
    pub const SYS_CLOSE_RANGE: usize = 436;
    pub const SYS_LANDLOCK_CREATE_RULESET: usize = 444;
    pub const SYS_LANDLOCK_ADD_RULE: usize = 445;
    pub const SYS_LANDLOCK_RESTRICT_SELF: usize = 446;

    pub const O_RDONLY: usize = 0;
    pub const O_WRONLY: usize = 1;
    pub const O_RDWR: usize = 2;
    pub const O_CREAT: usize = 0x40;
    pub const O_EXCL: usize = 0x80;
    pub const O_TRUNC: usize = 0x200;
    pub const O_NONBLOCK: usize = 0x800;
    pub const O_DIRECTORY: usize = 0x4000;
    pub const O_NOFOLLOW: usize = 0x8000;
    pub const O_CLOEXEC: usize = 0x8_0000;
    pub const O_PATH: usize = 0x20_0000;
    pub const O_TMPFILE: usize = 0x40_0000 | O_DIRECTORY;

    /// struct stat of fstat / newfstatat, 128 bytes
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Stat {
        pub st_dev: u64,
        pub st_ino: u64,
        pub st_mode: u32,
        pub st_nlink: u32,
        pub st_uid: u32,
        pub st_gid: u32,
        pub st_rdev: u64,
        __pad1: u64,
        pub st_size: i64,
        pub st_blksize: i32,
        __pad2: i32,
        pub st_blocks: i64,
        pub st_atime: i64,
        pub st_atime_nsec: i64,
        pub st_mtime: i64,
        pub st_mtime_nsec: i64,
        pub st_ctime: i64,
        pub st_ctime_nsec: i64,
        __unused: [u32; 2],
    }
    const _: () = assert!(size_of::<Stat>() == 128 && offset_of!(Stat, st_mode) == 16 && offset_of!(Stat, st_size) == 48 && offset_of!(Stat, st_mtime) == 88);

    /// clone(flags, stack, a, b, c): tls before child_tid (CONFIG_CLONE_BACKWARDS)
    pub const fn clone_args(parent_tid: usize, child_tid: usize, tls: usize) -> [usize; 3] {
        [parent_tid, tls, child_tid]
    }
}
