// t_kernel -- plattest --test kernel: rt's kernel table on the running kernel (Linux; lib/rt/kernel.rs)
//
//   what the open flags do (O_NOFOLLOW and O_DIRECTORY differ between the architectures: a wrong
//   value would mean something else without any error), every struct stat field the library reads,
//   the order of clone's arguments, and the helpers for the calls ARM64 does not have: poll, fork,
//   dup2, rename.  The numbers themselves are compared with the kernel's headers by
//   tools/kernel_table_check.py.

#[cfg(not(target_os = "linux"))]
pub fn t_kernel(t: &mut crate::T) {
    t.info("kernel: Linux only\n");
}

#[cfg(target_os = "linux")]
pub use lnx::t_kernel;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod lnx {
    use alloc::format;
    use alloc::vec::Vec;

    use rt::os::{
        clone, dup2, fork, poll, rename, syscall3, syscall6, AT_FDCWD, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_PATH, O_RDONLY, O_RDWR, O_TMPFILE,
        O_WRONLY, SYS_CLOSE, SYS_EXIT_GROUP, SYS_FCHMODAT, SYS_FSTAT, SYS_GETTID, SYS_GETUID, SYS_MKDIRAT, SYS_NEWFSTATAT, SYS_OPENAT, SYS_PIPE2, SYS_READ,
        SYS_READLINKAT, SYS_SYMLINKAT, SYS_UNLINKAT, SYS_WAIT4, SYS_WRITE, Stat,
    };

    use crate::T;

    const ENOENT: isize = -2;
    const EBADF: isize = -9;
    const EEXIST: isize = -17;
    const ENOTDIR: isize = -20;
    const EINVAL: isize = -22;
    const ELOOP: isize = -40;
    const EOPNOTSUPP: isize = -95;
    const AT_REMOVEDIR: usize = 0x200;
    const SIGCHLD: usize = 17;
    const CLONE_PARENT_SETTID: usize = 0x10_0000;
    const CLONE_CHILD_SETTID: usize = 0x100_0000;
    const S_IFMT: u32 = 0o170000;
    const S_IFREG: u32 = 0o100000;
    const S_IFDIR: u32 = 0o040000;
    const S_IFLNK: u32 = 0o120000;
    const POLLIN: u64 = 1;

    fn z(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    fn open(path: &str, flags: usize, mode: usize) -> isize {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, flags | O_CLOEXEC, mode, 0, 0) }
    }

    fn close(fd: isize) {
        // SAFETY: a descriptor of this test.
        unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
    }

    fn write(fd: isize, b: &[u8]) -> isize {
        // SAFETY: a readable slice.
        unsafe { syscall3(SYS_WRITE, fd as usize, b.as_ptr() as usize, b.len()) }
    }

    fn stat(path: &str, flags: usize) -> Option<Stat> {
        let p = z(path);
        let mut st = Stat::default();
        // SAFETY: a NUL-terminated path; st is a struct stat.
        let r = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, p.as_ptr() as usize, &mut st as *mut Stat as usize, flags, 0, 0) };
        (r == 0).then_some(st)
    }

    fn fstat(fd: usize) -> Result<Stat, isize> {
        let mut st = Stat::default();
        // SAFETY: st is a struct stat.
        let r = unsafe { syscall3(SYS_FSTAT, fd, &mut st as *mut Stat as usize, 0) };
        if r == 0 { Ok(st) } else { Err(r) }
    }

    fn unlink(path: &str, flags: usize) {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { syscall3(SYS_UNLINKAT, AT_FDCWD, p.as_ptr() as usize, flags) };
    }

    // the child's exit code, -1 when a signal ended it
    fn reap(pid: isize) -> i32 {
        let mut status = 0i32;
        // SAFETY: status is a plain out-parameter.
        while unsafe { syscall6(SYS_WAIT4, pid as usize, &mut status as *mut i32 as usize, 0, 0, 0, 0) } == -4 {}
        if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 }
    }

    fn exit_child(code: usize) -> ! {
        // SAFETY: ends the child.
        unsafe { syscall3(SYS_EXIT_GROUP, code, 0, 0) };
        loop {}
    }

    pub fn t_kernel(t: &mut T) {
        // the temp folder: a real Linux file system (the repository under WSL is drvfs)
        let dir = format!("{}/plattest-kernel-{:x}", plat::temp_dir().unwrap_or_default(), plat::time_us() as u32);
        let d = z(&dir);
        // SAFETY: a NUL-terminated path.
        t.check(unsafe { syscall3(SYS_MKDIRAT, AT_FDCWD, d.as_ptr() as usize, 0o755) } == 0, "kernel: mkdirat");
        flags(t, &dir);
        stats(t, &dir);
        polls(t);
        forks(t);
        dups(t);
        renames(t, &dir);
        unlink(&dir, AT_REMOVEDIR);
        t.check(stat(&dir, 0).is_none(), "kernel: unlinkat(AT_REMOVEDIR) took the folder");
    }

    fn flags(t: &mut T, dir: &str) {
        let file = format!("{dir}/file");
        let link = format!("{dir}/link");
        let fd = open(&file, O_WRONLY | O_CREAT | O_EXCL, 0o640);
        t.check(fd >= 0, "kernel: O_CREAT | O_EXCL makes a file");
        close(fd);
        t.check(open(&file, O_WRONLY | O_CREAT | O_EXCL, 0o640) == EEXIST, "kernel: O_EXCL refuses an existing file");
        let (target, l) = (z("file"), z(&link));
        // SAFETY: NUL-terminated paths.
        t.check(unsafe { syscall3(SYS_SYMLINKAT, target.as_ptr() as usize, AT_FDCWD, l.as_ptr() as usize) } == 0, "kernel: symlinkat");

        let fd = open(&link, O_RDONLY, 0);
        t.check(fd >= 0, "kernel: a link is followed without O_NOFOLLOW");
        close(fd);
        t.check(open(&link, O_RDONLY | O_NOFOLLOW, 0) == ELOOP, "kernel: O_NOFOLLOW refuses a link");
        let fd = open(dir, O_RDONLY | O_DIRECTORY, 0);
        t.check(fd >= 0, "kernel: O_DIRECTORY opens a folder");
        close(fd);
        t.check(open(&file, O_RDONLY | O_DIRECTORY, 0) == ENOTDIR, "kernel: O_DIRECTORY refuses a file");
        let fd = open(&link, O_PATH | O_NOFOLLOW, 0);
        t.check(fd >= 0, "kernel: O_PATH | O_NOFOLLOW opens the link itself");
        t.check(fstat(fd as usize).is_ok_and(|st| st.st_mode & S_IFMT == S_IFLNK), "kernel: ... and fstat says it is a link");
        close(fd);

        let fd = open(dir, O_TMPFILE | O_RDWR, 0o600);
        if fd == EOPNOTSUPP || fd == EINVAL {
            t.info("  kernel: no O_TMPFILE on this file system\n");
        } else {
            t.check(fd >= 0, "kernel: O_TMPFILE makes a file without a name");
            close(fd);
        }

        let mut buf = [0u8; 16];
        // SAFETY: a NUL-terminated path and a writable buffer.
        let n = unsafe { syscall6(SYS_READLINKAT, AT_FDCWD, l.as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0) };
        t.check(n == 4 && buf[..4] == *b"file", "kernel: readlinkat reads the link");
        let f = z(&file);
        // SAFETY: as above.
        let n = unsafe { syscall6(SYS_READLINKAT, AT_FDCWD, f.as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0) };
        t.check(n == EINVAL, "kernel: readlinkat of a file is EINVAL (what link_target waits for)");
        unlink(&link, 0);
        unlink(&file, 0);
        t.check(open(&file, O_RDONLY, 0) == ENOENT, "kernel: unlinkat took the file");
    }

    fn stats(t: &mut T, dir: &str) {
        let file = format!("{dir}/stat");
        let fd = open(&file, O_WRONLY | O_CREAT | O_EXCL, 0o600);
        t.check(write(fd, b"12345") == 5, "kernel: stat: 5 bytes written");
        close(fd);
        let p = z(&file);
        // SAFETY: a NUL-terminated path.
        t.check(unsafe { syscall3(SYS_FCHMODAT, AT_FDCWD, p.as_ptr() as usize, 0o640) } == 0, "kernel: fchmodat");
        let Some(st) = stat(&file, 0) else {
            t.check(false, "kernel: newfstatat");
            return;
        };
        // SAFETY: no arguments.
        let uid = unsafe { syscall3(SYS_GETUID, 0, 0, 0) } as u32;
        t.check(st.st_mode & S_IFMT == S_IFREG, "kernel: st_mode says file");
        t.check(st.st_mode & 0o777 == 0o640, &format!("kernel: st_mode has the access bits ({:o})", st.st_mode));
        t.check(st.st_size == 5, &format!("kernel: st_size ({})", st.st_size));
        t.check(st.st_nlink == 1, &format!("kernel: st_nlink ({})", st.st_nlink));
        t.check(st.st_uid == uid, &format!("kernel: st_uid ({} of {uid})", st.st_uid));
        t.check(st.st_ino != 0, "kernel: st_ino");
        let time = |s: i64, ns: i64| s > 1_500_000_000 && (0..1_000_000_000).contains(&ns);
        t.check(time(st.st_mtime, st.st_mtime_nsec), &format!("kernel: st_mtime ({}.{})", st.st_mtime, st.st_mtime_nsec));
        t.check(time(st.st_atime, st.st_atime_nsec), "kernel: st_atime");
        t.check(time(st.st_ctime, st.st_ctime_nsec), "kernel: st_ctime");

        let fd = open(&file, O_RDONLY, 0);
        t.check(fstat(fd as usize).is_ok_and(|b| b.st_ino == st.st_ino && b.st_dev == st.st_dev && b.st_size == 5), "kernel: fstat agrees with newfstatat");
        close(fd);
        t.check(stat(dir, 0).is_some_and(|d| d.st_mode & S_IFMT == S_IFDIR), "kernel: st_mode says folder");
        t.check(plat::fs::stat(&file).is_some_and(|m| m.size == 5 && m.mode == st.st_mode && m.uid == uid && m.ino == st.st_ino && m.nlink == 1), "kernel: plat::fs::stat reads the same");
        unlink(&file, 0);
        t.check(stat("/dev/null", 0).is_some_and(|d| d.st_rdev != 0), "kernel: st_rdev of /dev/null");
    }

    fn polls(t: &mut T) {
        // no descriptors: a sleep (plat::sleep_ms)
        let start = plat::time_us();
        // SAFETY: no pollfd.
        let r = unsafe { poll(0, 0, 60) };
        let took = plat::time_us() - start;
        t.check(r == 0 && took >= 50_000, &format!("kernel: poll sleeps 60 ms ({r}, {took} us)"));
        let start = plat::time_us();
        // SAFETY: no pollfd.
        let r = unsafe { poll(0, 0, 0) };
        t.check(r == 0 && plat::time_us() - start < 50_000, "kernel: poll with 0 ms does not wait");

        let mut pair = [-1i32; 2];
        // SAFETY: pipe2 writes two descriptors.
        t.check(unsafe { syscall3(SYS_PIPE2, pair.as_mut_ptr() as usize, O_CLOEXEC, 0) } == 0, "kernel: pipe2");
        let (rd, wr) = (pair[0] as isize, pair[1] as isize);
        let readable = rd as u32 as u64 | POLLIN << 32; // struct pollfd {fd, events, revents}
        let mut pfd = [readable];
        // SAFETY: one pollfd.
        let r = unsafe { poll(pfd.as_mut_ptr() as usize, 1, 20) };
        t.check(r == 0, "kernel: poll: nothing to read, the time runs out");
        t.check(write(wr, b"x") == 1, "kernel: poll: a byte in the pipe");
        let mut pfd = [readable];
        let start = plat::time_us();
        // SAFETY: one pollfd; over a second: the timeout's seconds and nanoseconds are apart.
        let r = unsafe { poll(pfd.as_mut_ptr() as usize, 1, 2500) };
        t.check(r == 1 && (pfd[0] >> 48) & POLLIN != 0 && plat::time_us() - start < 2_000_000, "kernel: poll: readable, at once");
        let mut pfd = [readable];
        // SAFETY: one pollfd, the byte is there.
        t.check(unsafe { poll(pfd.as_mut_ptr() as usize, 1, -1) } == 1, "kernel: poll: -1 is no timeout");
        close(rd);
        close(wr);
    }

    fn forks(t: &mut T) {
        // SAFETY: the child only ends itself.
        let pid = unsafe { fork() };
        if pid == 0 {
            exit_child(7);
        }
        t.check(pid > 0 && reap(pid) == 7, "kernel: fork: the child's exit code arrives");

        // the order of clone's arguments: each id must arrive in the place named for it
        let (mut parent_tid, mut child_tid) = (0u32, 0u32);
        // SAFETY: no new stack and no shared memory (the child has its own copy of both words);
        // it only looks at them and ends itself.
        let pid = unsafe {
            clone(SIGCHLD | CLONE_PARENT_SETTID | CLONE_CHILD_SETTID, 0, &mut parent_tid as *mut u32 as usize, &mut child_tid as *mut u32 as usize, 0)
        };
        if pid == 0 {
            // SAFETY: the kernel wrote the word behind the compiler's back; gettid has no arguments.
            let ok = unsafe { core::ptr::read_volatile(&child_tid) as isize == syscall3(SYS_GETTID, 0, 0, 0) };
            exit_child(if ok { 0 } else { 9 });
        }
        // SAFETY: as above.
        let parent = unsafe { core::ptr::read_volatile(&parent_tid) } as isize;
        t.check(pid > 0 && parent == pid, &format!("kernel: clone: parent_tid holds the child ({parent}, {pid})"));
        t.check(reap(pid) == 0, "kernel: clone: child_tid holds the child in its copy");
    }

    fn dups(t: &mut T) {
        let fd = open("/dev/null", O_RDONLY, 0);
        t.check(fd > 2, "kernel: dup2: a descriptor");
        let high = 300usize;
        // SAFETY: a number this process does not use otherwise.
        t.check(unsafe { dup2(fd as usize, high) } == high as isize, "kernel: dup2 to another number");
        t.check(fstat(high).is_ok(), "kernel: ... which is open");
        // dup3 refuses this one; process::launch can get there when stdin was closed
        // SAFETY: changes nothing.
        t.check(unsafe { dup2(fd as usize, fd as usize) } == fd, "kernel: dup2 of a descriptor to itself");
        close(high as isize);
        // SAFETY: changes nothing, the descriptor is closed.
        t.check(unsafe { dup2(high, high) } == EBADF, "kernel: dup2 of a closed descriptor to itself is EBADF");
        close(fd);
        let mut b = [0u8; 1];
        // SAFETY: closed: EBADF.
        t.check(unsafe { syscall3(SYS_READ, fd as usize, b.as_mut_ptr() as usize, 1) } == EBADF, "kernel: close closed it");
    }

    fn renames(t: &mut T, dir: &str) {
        let (a, b) = (format!("{dir}/a"), format!("{dir}/b"));
        let fd = open(&a, O_WRONLY | O_CREAT | O_EXCL, 0o600);
        write(fd, b"new");
        close(fd);
        let fd = open(&b, O_WRONLY | O_CREAT | O_EXCL, 0o600);
        write(fd, b"old!");
        close(fd);
        let (pa, pb) = (z(&a), z(&b));
        // SAFETY: NUL-terminated paths.
        t.check(unsafe { rename(pa.as_ptr() as usize, pb.as_ptr() as usize) } == 0, "kernel: rename");
        t.check(stat(&a, 0).is_none(), "kernel: rename: the old name is gone");
        t.check(stat(&b, 0).is_some_and(|s| s.st_size == 3), "kernel: rename replaced the target");
        // SAFETY: NUL-terminated paths; the old name is gone.
        t.check(unsafe { rename(pa.as_ptr() as usize, pb.as_ptr() as usize) } == ENOENT, "kernel: rename of nothing is ENOENT");
        unlink(&b, 0);
    }
}
