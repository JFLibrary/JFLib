// t_kernel -- plattest --test kernel: rt's kernel table on the running kernel (Linux; lib/rt/kernel.rs)
//
//   what the open flags do (O_NOFOLLOW and O_DIRECTORY differ between the architectures: a wrong
//   value would mean something else without any error), every struct stat field the library reads,
//   the order of clone's arguments, and the helpers for the calls ARM64 does not have: poll, fork,
//   dup2, rename.  The numbers themselves are compared with the kernel's headers by
//   tools/kernel_table_check.py.  plat::fs::Dir on the kernel's *at calls and openat2: no link on
//   the way below an open folder, also none swapped in after it was opened.
//
//   Windows: plat::fs::Dir on NtCreateFile relative to the folders' handles -- a folder another
//   handle holds open with delete access (so nothing can lock it against renaming) renamed and
//   replaced by a junction after it was opened: what is done in it stays in the real folder; the
//   links it makes carry the same reparse data as CreateSymbolicLinkW's (when this account may
//   make links: the developer mode or the privilege).

#[cfg(not(any(target_os = "linux", windows)))]
pub fn t_kernel(t: &mut crate::T) {
    t.info("kernel: Linux and Windows only\n");
}

#[cfg(windows)]
pub use win::t_kernel;

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use plat::fs::{Dir, FileKind};

    use crate::T;

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: usize, disp: u32, flags: u32, tmpl: isize) -> isize;
        fn CloseHandle(h: isize) -> i32;
        fn DeviceIoControl(h: isize, code: u32, inb: usize, inn: u32, outb: *mut u8, outn: u32, ret: *mut u32, ov: usize) -> i32;
    }

    const DELETE: u32 = 0x1_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const SHARE_ALL: u32 = 7;
    const OPEN_EXISTING: u32 = 3;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;

    struct H(isize);
    impl Drop for H {
        fn drop(&mut self) {
            // SAFETY: an owned handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    // a handle of a file or folder (every share mode; OPEN_REPARSE_POINT in flags: the link itself)
    fn open(path: &str, access: u32, flags: u32) -> Option<H> {
        let w: Vec<u16> = plat::fs::long_path(path).encode_utf16().chain(core::iter::once(0)).collect();
        // SAFETY: a NUL-terminated path; no security attributes.
        let h = unsafe { CreateFileW(w.as_ptr(), access, SHARE_ALL, 0, OPEN_EXISTING, flags | BACKUP_SEMANTICS, 0) };
        (h != -1).then_some(H(h))
    }

    // "C:\...\x" of a path
    fn abs(path: &str) -> String {
        let l = plat::fs::long_path(path);
        String::from(l.strip_prefix(r"\\?\").unwrap_or(&l))
    }

    /// a junction at `link` (made as a new folder) to the folder `target` (absolute)
    fn junction(link: &str, target: &str) -> bool {
        if plat::fs::lstat(link).is_some() || !plat::mkdir(link) {
            return false;
        }
        let Some(h) = open(link, GENERIC_WRITE, OPEN_REPARSE_POINT) else { return false };
        let sub: Vec<u16> = format!(r"\??\{target}").encode_utf16().collect();
        let print: Vec<u16> = target.encode_utf16().collect();
        let (sn, pn) = (sub.len() * 2, print.len() * 2);
        // REPARSE_DATA_BUFFER of a mount point: tag, data length, reserved, the substitute
        // name's offset and length, the print name's; both names NUL-terminated
        let mut v = Vec::new();
        v.extend_from_slice(&0xA000_0003u32.to_le_bytes());
        for x in [8 + sn + 2 + pn + 2, 0, 0, sn, sn + 2, pn] {
            v.extend_from_slice(&(x as u16).to_le_bytes());
        }
        for c in sub.iter().chain(&[0]).chain(print.iter()).chain(&[0]) {
            v.extend_from_slice(&c.to_le_bytes());
        }
        let mut got = 0u32;
        // SAFETY: the reparse data and its length; no output.
        unsafe { DeviceIoControl(h.0, FSCTL_SET_REPARSE_POINT, v.as_ptr() as usize, v.len() as u32, core::ptr::null_mut(), 0, &mut got, 0) != 0 }
    }

    /// the raw reparse data of an entry
    fn reparse_data(path: &str) -> Option<Vec<u8>> {
        let h = open(path, FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT)?;
        let mut buf = alloc::vec![0u8; 16 * 1024];
        let mut got = 0u32;
        // SAFETY: a handle and an output buffer of its length.
        if unsafe { DeviceIoControl(h.0, FSCTL_GET_REPARSE_POINT, 0, 0, buf.as_mut_ptr(), buf.len() as u32, &mut got, 0) } == 0 {
            return None;
        }
        buf.truncate(got as usize);
        Some(buf)
    }

    fn read_all(f: &plat::File) -> Vec<u8> {
        let mut b = alloc::vec![0u8; 64];
        let n = f.read_at(0, &mut b).unwrap_or(0);
        b.truncate(n);
        b
    }

    fn names(d: &Dir) -> Option<Vec<(String, FileKind)>> {
        d.read_dir().map(|mut v| {
            v.sort_by(|x, y| x.name.cmp(&y.name));
            v.into_iter().map(|e| (e.name, e.kind)).collect()
        })
    }

    // a folder with everything below it; links and junctions removed as themselves
    fn remove_tree(path: &str) -> bool {
        if plat::fs::lstat(path).is_some_and(|m| m.is_dir()) {
            for e in plat::fs::read_dir(path).unwrap_or_default() {
                remove_tree(&plat::fs::join(path, &e.name));
            }
        }
        plat::delete(&plat::fs::long_path(path))
    }

    pub fn t_kernel(t: &mut T) {
        let dir = format!("{}/plattest-kernel-{:x}", plat::temp_dir().unwrap_or_default(), plat::time_us() as u32);
        t.check(plat::mkdir(&dir), "kernel: a test folder");
        dirs(t, &dir);
        links(t, &dir);
        t.check(remove_tree(&dir) && plat::fs::lstat(&dir).is_none(), "kernel: the test folder is gone");
    }

    fn dirs(t: &mut T, dir: &str) {
        let (root, out) = (format!("{dir}/root"), format!("{dir}/out"));
        for d in [&root, &format!("{root}/a"), &format!("{root}/a/b"), &out, &format!("{out}/b")] {
            plat::mkdir(d);
        }
        plat::write_file(&format!("{root}/a/b/f"), b"data");
        plat::write_file(&format!("{out}/b/f"), b"SECRET");
        let out_abs = abs(&out);
        t.check(junction(&format!("{root}/lnk"), &out_abs), "kernel: Dir (Windows): a junction to the outside folder");
        let Some(r) = Dir::open(&root) else {
            t.check(false, "kernel: Dir (Windows): Dir::open");
            return;
        };
        t.check(r.lstat("").is_some_and(|m| m.is_dir()), "kernel: Dir (Windows): lstat of the folder itself");
        let b = r.open_dir("a/b");
        t.check(b.as_ref().and_then(|b| b.open_file("f")).is_some_and(|f| read_all(&f) == b"data"), "kernel: Dir (Windows): open_dir(\"a/b\") and open_file");
        drop(b);
        t.check(r.open_dir("lnk").is_none() && r.open_dir("lnk/b").is_none() && r.open_dir("lnk\\b").is_none(), "kernel: Dir (Windows): open_dir refuses a junction on the way");
        t.check(r.open_dir("a/../a").is_none() && r.open_dir("").is_none() && r.open_dir("C:/").is_none(), "kernel: Dir (Windows): open_dir refuses '..', empty and absolute paths");
        t.check(r.lstat("lnk").is_some_and(|m| m.is_symlink()) && r.read_link("lnk").as_deref() == Some(out_abs.as_str()), "kernel: Dir (Windows): lstat and read_link of a junction");
        t.check(r.lstat("a/b").is_none() && r.lstat("..").is_none() && r.lstat("a:s").is_none(), "kernel: Dir (Windows): names are one component, no stream");
        t.check(r.open_file("lnk").is_none() && r.open_file("a").is_none(), "kernel: Dir (Windows): open_file refuses a junction and a folder");
        let want = alloc::vec![(String::from("a"), FileKind::Dir), (String::from("lnk"), FileKind::Symlink)];
        let got = names(&r);
        t.check(got.as_ref() == Some(&want), &format!("kernel: Dir (Windows): read_dir ({got:?})"));

        // another program holds "a" open with delete access: nothing can lock it against being
        // renamed now (the old Dir could not either), so nothing may depend on that
        let held = open(&format!("{root}/a"), DELETE | FILE_READ_ATTRIBUTES, 0);
        t.check(held.is_some(), "kernel: Dir (Windows): another handle holds the folder with DELETE");
        let Some(a) = r.open_dir("a") else {
            t.check(false, "kernel: Dir (Windows): open_dir(\"a\")");
            return;
        };

        // a file written, its times set through its handle, renamed into place
        let f = a.create_new("tmp");
        t.check(f.is_some() && a.create_new("tmp").is_none() && a.create_new("b").is_none(), "kernel: Dir (Windows): create_new makes a new file only");
        if let Some(f) = f {
            f.write_at(0, b"restored");
            t.check(plat::fs::set_file_times(&f, 1_000_000_000_000_000_000, 1_100_000_000_000_000_000), "kernel: set_file_times (Windows)");
            drop(f);
        }
        t.check(a.rename("tmp", &a, "file") && a.lstat("tmp").is_none(), "kernel: Dir (Windows): rename");
        t.check(a.lstat("file").is_some_and(|m| m.mtime_ns == 1_100_000_000_000_000_000 && m.size == 8), "kernel: Dir (Windows): the times set through the handle");
        t.check(a.create_new("other").is_some() && a.rename("other", &a, "file") && a.lstat("other").is_none(), "kernel: Dir (Windows): rename replaces a file");
        let b = a.open_dir("b");
        t.check(b.as_ref().is_some_and(|b| a.rename("file", b, "moved") && b.lstat("moved").is_some() && b.rename("moved", &a, "file")), "kernel: Dir (Windows): rename into another open folder and back");
        drop(b);
        t.check(a.set_times("file", 5_000_000_000, 6_000_000_000) && a.lstat("file").is_some_and(|m| m.mtime_ns == 6_000_000_000), "kernel: Dir (Windows): set_times");
        t.check(a.set_times("", 7_000_000_000, 8_000_000_000) && r.lstat("a").is_some_and(|m| m.mtime_ns == 8_000_000_000), "kernel: Dir (Windows): set_times of the folder itself");
        t.check(a.set_attrs("file", 1) && a.lstat("file").is_some_and(|m| m.attrs & 1 != 0) && a.set_attrs("file", 0) && a.lstat("file").is_some_and(|m| m.attrs & 1 == 0), "kernel: Dir (Windows): set_attrs");
        t.check(a.mkdir("m") && !a.mkdir("m") && a.lstat("m").is_some_and(|m| m.is_dir()) && a.remove("m") && a.lstat("m").is_none(), "kernel: Dir (Windows): mkdir and remove of a folder");
        t.check(!a.remove("b"), "kernel: Dir (Windows): a folder with something in it is not removed");

        // the swap: "a" renamed away while the Dir has it open (no lock any more, and the other
        // program's handle would allow it anyway) and a junction to the outside folder put in
        // its place -- what is done in the open folder stays in the real one
        let swapped = plat::rename(&format!("{root}/a"), &format!("{root}/a.old")) && junction(&format!("{root}/a"), &out_abs);
        t.check(swapped, "kernel: Dir (Windows): the folder renamed while open, a junction in its place");
        let old = format!("{root}/a.old");
        t.check(a.open_dir("b").and_then(|b| b.open_file("f")).is_some_and(|f| read_all(&f) == b"data"), "kernel: Dir (Windows): the open folder is not redirected by the junction");
        t.check(r.open_dir("a/b").is_none() && r.open_dir("a").is_none() && r.lstat("a").is_some_and(|m| m.is_symlink()), "kernel: Dir (Windows): the swapped-in junction is refused when opening again");
        t.check(a.create_new("new").is_some() && plat::fs::lstat(&format!("{old}/new")).is_some() && plat::fs::lstat(&format!("{out}/new")).is_none(), "kernel: Dir (Windows): a file is created in the real folder, not through the junction");
        let want = alloc::vec![(String::from("b"), FileKind::Dir), (String::from("file"), FileKind::File), (String::from("new"), FileKind::File)];
        let got = names(&a);
        t.check(got.as_ref() == Some(&want), &format!("kernel: Dir (Windows): read_dir lists the real folder ({got:?})"));
        t.check(a.mkdir("m2") && plat::fs::lstat(&format!("{old}/m2")).is_some() && plat::fs::lstat(&format!("{out}/m2")).is_none() && a.remove("m2"), "kernel: Dir (Windows): a folder is made in the real folder");
        t.check(a.rename("new", &a, "new2") && plat::fs::lstat(&format!("{old}/new2")).is_some() && plat::fs::lstat(&format!("{out}/new2")).is_none(), "kernel: Dir (Windows): a rename stays in the real folder");
        t.check(a.remove("new2") && a.remove("file") && plat::fs::lstat(&format!("{out}/b/f")).is_some() && plat::fs::lstat(&format!("{old}/file")).is_none(), "kernel: Dir (Windows): removals stay in the real folder");
        drop((held, a, r));
    }

    // Dir::symlink against CreateSymbolicLinkW (plat::fs::symlink): the same reparse data
    fn links(t: &mut T, dir: &str) {
        let base = format!("{dir}/links");
        plat::mkdir(&base);
        if !plat::fs::symlink("x", &format!("{base}/probe"), false) {
            t.info("kernel: Dir (Windows): this account may not make symbolic links (no developer mode, no privilege): links skipped\n");
            return;
        }
        let Some(d) = Dir::open(&base) else {
            t.check(false, "kernel: Dir (Windows): Dir::open of the links folder");
            return;
        };
        let targets = ["file", r"sub\x", "sub/x", r"..\up", r"\rooted", r"C:\abs\x", "C:/abs/y", r"\\server\share\x", r"\\?\C:\v", "C:rel"];
        for (i, tg) in targets.iter().enumerate() {
            let dl = i % 2 == 1;
            let (mine, theirs) = (format!("d{i}"), format!("{base}/c{i}"));
            let ok = d.symlink(tg, &mine, dl) && plat::fs::symlink(tg, &theirs, dl);
            let same = ok && reparse_data(&format!("{base}/{mine}")).is_some_and(|x| Some(x) == reparse_data(&theirs));
            let kinds = d.lstat(&mine).is_some_and(|m| m.is_symlink() && (m.attrs & 0x10 != 0) == dl);
            t.check(same && kinds, &format!("kernel: Dir (Windows): symlink to {tg} as CreateSymbolicLinkW makes it"));
        }
        t.check(!d.symlink("other", "d0", false) && d.read_link("d0").as_deref() == Some("file"), "kernel: Dir (Windows): symlink refuses an existing name");
        let real = abs(dir);
        t.check(d.symlink(&real, "up", true) && Dir::open(&format!("{base}/up")).and_then(|u| u.lstat("links")).is_some_and(|m| m.is_dir()), "kernel: Dir (Windows): a folder link made by the Dir leads to its target");
        t.check(d.open_dir("up").is_none() && d.remove("up") && d.lstat("up").is_none() && plat::fs::lstat(&base).is_some(), "kernel: Dir (Windows): a folder link is refused on the way and removed as the link");
    }
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
        dirs(t, &dir);
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

    fn read_all(f: &plat::File) -> Vec<u8> {
        let mut b = alloc::vec![0u8; 64];
        let n = f.read_at(0, &mut b).unwrap_or(0);
        b.truncate(n);
        b
    }

    /// plat::fs::Dir: an open folder's entries never through a link, a folder swapped for a
    /// link after it was opened not followed (bc_fs's restores and the agent's reads)
    fn dirs(t: &mut T, dir: &str) {
        use plat::fs::Dir;
        let (root, out) = (format!("{dir}/root"), format!("{dir}/out"));
        for d in [&root, &format!("{root}/a"), &format!("{root}/a/b"), &out, &format!("{out}/b")] {
            plat::mkdir(d);
        }
        plat::write_file(&format!("{root}/a/b/f"), b"data");
        plat::write_file(&format!("{out}/b/f"), b"SECRET");
        for v in ["victim", "file"] {
            plat::write_file(&format!("{out}/{v}"), b"v");
            let p = z(&format!("{out}/{v}"));
            // SAFETY: a NUL-terminated path.
            unsafe { syscall3(SYS_FCHMODAT, AT_FDCWD, p.as_ptr() as usize, 0o600) };
        }
        plat::fs::symlink(&out, &format!("{root}/lnk"), true);
        plat::fs::symlink(&format!("{out}/victim"), &format!("{root}/a/flink"), false);

        let Some(r) = Dir::open(&root) else {
            t.check(false, "kernel: Dir::open");
            return;
        };
        t.check(r.lstat("").is_some_and(|m| m.is_dir()), "kernel: Dir: lstat of the folder itself");
        let b = r.open_dir("a/b");
        t.check(b.as_ref().and_then(|b| b.open_file("f")).is_some_and(|f| read_all(&f) == b"data"), "kernel: Dir: open_dir(\"a/b\") and open_file");
        t.check(r.open_dir("lnk").is_none() && r.open_dir("lnk/b").is_none(), "kernel: Dir: open_dir refuses a link on the way");
        t.check(r.open_dir("a/../a").is_none() && r.open_dir("/").is_none() && r.open_dir("").is_none(), "kernel: Dir: open_dir refuses '..', absolute and empty paths");
        t.check(r.lstat("lnk").is_some_and(|m| m.is_symlink()) && r.read_link("lnk").as_deref() == Some(out.as_str()), "kernel: Dir: lstat and read_link of a link");
        t.check(r.lstat("a/b").is_none() && r.lstat("..").is_none(), "kernel: Dir: names are one component");
        let Some(a) = r.open_dir("a") else {
            t.check(false, "kernel: Dir: open_dir(\"a\")");
            return;
        };
        t.check(a.open_file("flink").is_none() && a.open_file("b").is_none(), "kernel: Dir: open_file refuses a link and a folder");
        t.check(!a.set_mode("flink", 0o666) && stat(&format!("{out}/victim"), 0).is_some_and(|s| s.st_mode & 0o777 == 0o600), "kernel: Dir: set_mode leaves a link and its target alone");
        t.check(a.create_new("flink").is_none(), "kernel: Dir: create_new refuses a link's name");
        let names = a.read_dir().map(|mut v| {
            v.sort_by(|x, y| x.name.cmp(&y.name));
            v.into_iter().map(|e| (e.name, e.kind)).collect::<Vec<_>>()
        });
        let want = alloc::vec![(alloc::string::String::from("b"), plat::fs::FileKind::Dir), (alloc::string::String::from("flink"), plat::fs::FileKind::Symlink)];
        t.check(names.as_ref() == Some(&want), &format!("kernel: Dir: read_dir ({names:?})"));

        // a file written, its times and mode set through its handle, renamed into place
        let f = a.create_new("tmp");
        t.check(f.is_some() && a.create_new("tmp").is_none(), "kernel: Dir: create_new makes a new file only");
        if let Some(f) = f {
            f.write_at(0, b"restored");
            t.check(plat::fs::set_file_mode(&f, 0o640) && plat::fs::set_file_times(&f, 1_000_000_000_000_000_000, 1_100_000_000_000_000_000), "kernel: set_file_mode / set_file_times");
            drop(f);
        }
        t.check(a.rename("tmp", &a, "file") && a.lstat("tmp").is_none(), "kernel: Dir: rename");
        t.check(a.lstat("file").is_some_and(|m| m.mode & 0o777 == 0o640 && m.mtime_ns == 1_100_000_000_000_000_000 && m.size == 8), "kernel: Dir: the file has the mode and times set through its handle");
        t.check(a.set_mode("file", 0o604) && a.lstat("file").is_some_and(|m| m.mode & 0o777 == 0o604), "kernel: Dir: set_mode");
        t.check(a.set_times("file", 5_000_000_000, 6_000_000_000) && a.lstat("file").is_some_and(|m| m.mtime_ns == 6_000_000_000), "kernel: Dir: set_times");
        t.check(a.set_times("", 7_000_000_000, 8_000_000_000) && r.lstat("a").is_some_and(|m| m.mtime_ns == 8_000_000_000), "kernel: Dir: set_times of the folder itself");
        t.check(a.mkdir("m") && !a.mkdir("m") && a.remove("m") && a.lstat("m").is_none(), "kernel: Dir: mkdir and remove of a folder");
        t.check(a.symlink("file", "sl", false) && a.lstat("sl").is_some_and(|m| m.is_symlink()) && a.remove("sl") && a.lstat("file").is_some(), "kernel: Dir: symlink, removed as the link");

        // the swap: "a" renamed away and a link to the outside folder put in its place after a
        // and b were opened -- what is done in them stays in the real folders
        plat::rename(&format!("{root}/a"), &format!("{root}/a.old"));
        plat::fs::symlink(&out, &format!("{root}/a"), true);
        t.check(b.as_ref().and_then(|b| b.open_file("f")).is_some_and(|f| read_all(&f) == b"data"), "kernel: Dir: an open folder is not redirected by a link swapped in");
        t.check(r.open_dir("a/b").is_none() && r.open_dir("a").is_none(), "kernel: Dir: the swapped-in link is refused when opening again");
        t.check(a.create_new("new").is_some() && plat::fs::lstat(&format!("{root}/a.old/new")).is_some() && plat::fs::lstat(&format!("{out}/new")).is_none(), "kernel: Dir: a file is created in the real folder, not through the link");
        let mode = |p: &str| stat(p, 0).map_or(0, |s| s.st_mode & 0o777);
        t.check(a.set_mode("file", 0o644) && mode(&format!("{out}/file")) == 0o600 && mode(&format!("{root}/a.old/file")) == 0o644, "kernel: Dir: a change stays in the real folder");
        t.check(a.remove("new") && a.remove("file") && plat::fs::lstat(&format!("{out}/b/f")).is_some(), "kernel: Dir: removals stay in the real folder");
        drop((a, b, r));

        for f in [format!("{root}/a.old/b/f"), format!("{root}/a.old/flink"), format!("{root}/a"), format!("{root}/lnk"), format!("{out}/b/f"), format!("{out}/victim"), format!("{out}/file")] {
            unlink(&f, 0);
        }
        for d in [format!("{root}/a.old/b"), format!("{root}/a.old"), root, format!("{out}/b"), out] {
            unlink(&d, AT_REMOVEDIR);
        }
    }
}