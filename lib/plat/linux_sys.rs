// linux_sys -- Linux files, directories, well-known paths   [src/plat/linux/lx_sys.s, plat_linux.s]
//
// No static buffers: every function may run on any thread.  File::read_at / write_at are
// pread / pwrite, so several threads may use one File at the same time.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use rt::os::*;

use super::{env, time_us};
use crate::{FO_READ, FO_RW, FO_WRITE};

const SYS_PREAD64: usize = 17;
const SYS_PWRITE64: usize = 18;
const SYS_FSYNC: usize = 74;
const SYS_FCHMOD: usize = 91;
const SYS_GETCWD: usize = 79;
const SYS_RENAME: usize = 82;
const SYS_GETDENTS64: usize = 217;
const SYS_OPENAT: usize = 257;
const SYS_MKDIRAT: usize = 258;
const SYS_NEWFSTATAT: usize = 262;
const SYS_UNLINKAT: usize = 263;
const SYS_GETPID: usize = 39;
const AT_FDCWD: usize = -100isize as usize;
const AT_REMOVEDIR: usize = 0x200;
const O_WRONLY: usize = 1;
const O_RDWR: usize = 2;
const O_CREAT: usize = 0x40;
const O_EXCL: usize = 0x80;
const O_TRUNC: usize = 0x200;
const O_DIRECTORY: usize = 0x10000;
const O_CLOEXEC: usize = 0x80000;
const O_TMPFILE: usize = 0x410000; // __O_TMPFILE | O_DIRECTORY
const EPERM: isize = 1;
const EINTR: isize = 4;
const EEXIST: isize = 17;
const EISDIR: isize = 21;
const S_IFMT: u32 = 0xF000;
const S_IFDIR: u32 = 0x4000;
const S_IFREG: u32 = 0x8000;
const IO_CHUNK: usize = 0x4000_0000;
const TEMP_TRIES: u32 = 16;
const DL_BUF: usize = 32768;

fn path_z(path: &str) -> Vec<u8> {
    let mut p: Vec<u8> = path.as_bytes().to_vec();
    p.push(0);
    p
}

/// struct stat of a path: (st_mode, st_size), None when it cannot be read
fn stat_at(dirfd: usize, name: *const u8) -> Option<(u32, i64)> {
    let mut st = [0u64; 18]; // 144 bytes
    // SAFETY: name is NUL-terminated; st holds a struct stat.
    if unsafe { syscall6(SYS_NEWFSTATAT, dirfd, name as usize, st.as_mut_ptr() as usize, 0, 0, 0) } != 0 {
        return None;
    }
    Some(((st[3] & 0xFFFF_FFFF) as u32, st[6] as i64)) // st_mode at 24, st_size at 48
}

/// (st_dev, st_ino) of a path (links followed), None when it cannot be read: the identity
/// fontdb uses to visit a directory once  [fd_visit_once]
pub fn dev_ino(path: &str) -> Option<(u64, u64)> {
    let p = path_z(path);
    let mut st = [0u64; 18]; // 144 bytes
    // SAFETY: p is NUL-terminated; st holds a struct stat.
    if unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, p.as_ptr() as usize, st.as_mut_ptr() as usize, 0, 0, 0) } != 0 {
        return None;
    }
    Some((st[0], st[1]))
}

/// drop trailing '/' (but keep "/")
fn trim_slash(mut s: String) -> String {
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

// ---------------------------------------------------------------- well-known directories

/// $XDG_CONFIG_HOME (absolute), else $HOME/.config; no trailing slash
pub fn config_dir() -> Option<String> {
    if let Some(v) = env("XDG_CONFIG_HOME").filter(|v| v.starts_with('/')) {
        return Some(trim_slash(v));
    }
    let home = trim_slash(env("HOME").filter(|v| !v.is_empty())?);
    let s = if home.ends_with('/') { home + ".config" } else { home + "/.config" };
    Some(trim_slash(s))
}

/// $XDG_DATA_HOME (absolute), else $HOME/.local/share; no trailing slash
pub fn data_dir() -> Option<String> {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// $XDG_CACHE_HOME (absolute), else $HOME/.cache; no trailing slash
pub fn cache_dir() -> Option<String> {
    xdg_dir("XDG_CACHE_HOME", ".cache")
}

// an XDG base directory: the variable when it is absolute, else $HOME/fallback
fn xdg_dir(var: &str, fallback: &str) -> Option<String> {
    if let Some(v) = env(var).filter(|v| v.starts_with('/')) {
        return Some(trim_slash(v));
    }
    let home = trim_slash(env("HOME").filter(|v| !v.is_empty())?);
    let s = if home.ends_with('/') { home + fallback } else { home + "/" + fallback };
    Some(trim_slash(s))
}

/// $TMPDIR (absolute), else /tmp
pub fn temp_dir() -> Option<String> {
    match env("TMPDIR").filter(|v| v.starts_with('/')) {
        Some(v) => Some(trim_slash(v)),
        None => Some(String::from("/tmp")),
    }
}

/// the current directory ("" on failure)  [plat_cwd]
pub fn cwd() -> String {
    let mut buf = alloc::vec![0u8; 4096];
    // SAFETY: buf holds 4096 bytes.
    let r = unsafe { syscall3(SYS_GETCWD, buf.as_mut_ptr() as usize, buf.len(), 0) };
    if r <= 0 {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

// ---------------------------------------------------------------- directories and names

/// 0 missing, 1 file, 2 directory  [plat_file_exists]
pub(crate) fn file_exists_raw(path: &str) -> u32 {
    let p = path_z(path);
    match stat_at(AT_FDCWD, p.as_ptr()) {
        None => 0,
        Some((m, _)) if m & S_IFMT == S_IFDIR => 2,
        Some(_) => 1,
    }
}

// one directory: created, or already a directory
fn mkdir1(path: &[u8]) -> bool {
    let mut p = path.to_vec();
    p.push(0);
    // SAFETY: NUL-terminated path; 0777 (the umask applies).
    let r = unsafe { syscall3(SYS_MKDIRAT, AT_FDCWD, p.as_ptr() as usize, 0o777) };
    if r == 0 {
        return true;
    }
    r == -EEXIST && matches!(stat_at(AT_FDCWD, p.as_ptr()), Some((m, _)) if m & S_IFMT == S_IFDIR)
}

/// create a directory and its missing parents; an existing directory is fine  [plat_mkdir]
pub(crate) fn mkdir_raw(path: &str) -> bool {
    let b = path.as_bytes();
    if mkdir1(b) {
        return true;
    }
    for i in 1..b.len() {
        if b[i] == b'/' && b[i - 1] != b'/' {
            mkdir1(&b[..i]); // errors are sorted out by the last step
        }
    }
    mkdir1(b)
}

/// a file or an empty directory  [plat_delete]
pub(crate) fn delete_raw(path: &str) -> bool {
    let p = path_z(path);
    // SAFETY: NUL-terminated path.
    unsafe {
        let r = syscall3(SYS_UNLINKAT, AT_FDCWD, p.as_ptr() as usize, 0);
        if r == 0 {
            return true;
        }
        (r == -EISDIR || r == -EPERM) && syscall3(SYS_UNLINKAT, AT_FDCWD, p.as_ptr() as usize, AT_REMOVEDIR) == 0
    }
}

/// rename, replacing an existing target atomically (same file system)  [plat_rename]
pub(crate) fn rename_raw(old: &str, new: &str) -> bool {
    let (o, n) = (path_z(old), path_z(new));
    // SAFETY: NUL-terminated paths.
    unsafe { syscall3(SYS_RENAME, o.as_ptr() as usize, n.as_ptr() as usize, 0) == 0 }
}

/// f(name, is_dir, size_lo) for every entry except "." and ".." (links followed)  [plat_dir_list]
pub(crate) fn dir_list_raw(path: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    let p = path_z(path);
    // SAFETY: NUL-terminated path; the descriptor is closed on every path.
    let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, O_DIRECTORY | O_CLOEXEC, 0, 0, 0) };
    if fd < 0 {
        return false;
    }
    let fd = fd as usize;
    let mut buf = alloc::vec![0u8; DL_BUF];
    let mut ok = true;
    'fill: loop {
        // SAFETY: buf holds DL_BUF bytes.
        let n = unsafe { syscall3(SYS_GETDENTS64, fd, buf.as_mut_ptr() as usize, DL_BUF) };
        if n == -EINTR {
            continue;
        }
        if n < 0 {
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        let mut off = 0usize;
        while off < n as usize {
            let reclen = u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if reclen == 0 {
                break 'fill;
            }
            let dtype = buf[off + 18];
            let name = &buf[off + 19..off + reclen];
            let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
            if name != b"." && name != b".." {
                let mut nz = name.to_vec();
                nz.push(0);
                let (is_dir, size) = match stat_at(fd, nz.as_ptr()) {
                    Some((m, s)) => (m & S_IFMT == S_IFDIR, s as u32),
                    None => (dtype == 4, 0), // broken link etc.: trust d_type
                };
                f(&String::from_utf8_lossy(name), is_dir, size);
            }
            off += reclen;
        }
    }
    // SAFETY: our descriptor.
    unsafe { syscall3(SYS_CLOSE, fd, 0, 0) };
    ok
}

// ---------------------------------------------------------------- whole files

/// the contents of a file, read until the end: files whose size is unknown (/proc) work
/// too  [plat_file_read]
pub(crate) fn read_file_raw(path: &str) -> Option<Vec<u8>> {
    let f = OsFile::open_raw(path, FO_READ)?;
    let size = f.size()?;
    if size > 0x7FFF_FFF0 {
        return None;
    }
    // plat-x11: capacity size + 4096, doubled while full, as plat_file_read (a /proc file
    // reports size 0; linuxtest --test files reads /proc/self/stat)
    let mut data = alloc::vec![0u8; size as usize + 4096];
    let mut n = 0;
    loop {
        n += f.read_at(n as u64, &mut data[n..])?;
        if n < data.len() {
            break; // the end of the file came first
        }
        data.resize(data.len() * 2, 0);
    }
    data.truncate(n);
    Some(data)
}

/// Write an exclusive sibling, preserve existing access bits, fsync and rename. [plat_file_write]
pub(crate) fn write_file_raw(path: &str, data: &[u8]) -> bool {
    OsAtomic::begin(path).is_some_and(|a| a.write_at(0, data) == Some(data.len()) && a.commit())
}

/// A file written beside its target and renamed over it by commit (write_file_raw and the
/// streaming writers of guarded.rs / aio.rs).  Dropped without commit, the sibling is removed.
pub(crate) struct OsAtomic {
    f: Option<OsFile>,
    tmp: Vec<u8>, // NUL-terminated; empty once renamed
    dst: Vec<u8>,
    mode: Option<usize>,
}

impl OsAtomic {
    /// An exclusively created sibling "PATH.tmp-PID-SEQ-TIME"; an existing regular file keeps its
    /// access bits at commit, anything else at path (a folder, a device) refuses.
    pub(crate) fn begin(path: &str) -> Option<OsAtomic> {
        if path.is_empty() || path.contains('\0') { return None; }
        let dst = path_z(path);
        let mut st = [0u64; 18];
        // SAFETY: NUL-terminated path and storage for the x86-64 stat structure.
        let status = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, dst.as_ptr() as usize, st.as_mut_ptr() as usize, 0, 0, 0) };
        let mode = match status {
            0 if st[3] as u32 & S_IFMT == S_IFREG => Some(st[3] as usize & 0o777),
            -2 => None, // ENOENT: new files retain the usual umask-controlled mode
            _ => return None,
        };
        // SAFETY: no arguments.
        let pid = unsafe { syscall3(SYS_GETPID, 0, 0, 0) } as u32;
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = path_z(&format!("{path}.tmp-{pid:x}-{seq:x}-{:x}", time_us() as u32));
            // SAFETY: NUL-terminated path; O_EXCL never opens a pre-existing entry.
            let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, tmp.as_ptr() as usize,
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, if mode.is_some() { 0o600 } else { 0o666 }, 0, 0) };
            if fd < 0 {
                if fd == -EEXIST { continue; }
                return None;
            }
            return Some(OsAtomic { f: Some(OsFile(fd as i32)), tmp, dst, mode });
        }
        None
    }

    /// bytes written at off of the sibling
    pub(crate) fn write_at(&self, off: u64, data: &[u8]) -> Option<usize> {
        self.f.as_ref()?.write_at(off, data)
    }

    /// the old file's access bits, fsync, rename over the target -> false: the target is
    /// unchanged and the sibling gone
    pub(crate) fn commit(mut self) -> bool {
        let Some(f) = self.f.take() else { return false };
        let mut ok = true;
        if let Some(mode) = self.mode {
            // SAFETY: f owns its fd; restore access bits only, not setuid/setgid bits.
            ok &= unsafe { syscall3(SYS_FCHMOD, f.0 as usize, mode, 0) } == 0;
        }
        ok &= f.sync();
        drop(f);
        // SAFETY: both paths are NUL-terminated.
        if ok && unsafe { syscall3(SYS_RENAME, self.tmp.as_ptr() as usize, self.dst.as_ptr() as usize, 0) } == 0 {
            self.tmp.clear();
            return true;
        }
        false
    }
}

impl Drop for OsAtomic {
    fn drop(&mut self) {
        drop(self.f.take());
        if !self.tmp.is_empty() {
            // SAFETY: NUL-terminated path; only our own temporary entry is removed.
            unsafe { syscall3(SYS_UNLINKAT, AT_FDCWD, self.tmp.as_ptr() as usize, 0) };
        }
    }
}

// ---------------------------------------------------------------- streaming files

/// an open file (closed when dropped)
pub(crate) struct OsFile(i32);

/// Nonblocking advisory sidecar lock. Drop closes the descriptor and releases the lock.
/// All cooperating writers must lock the same path; never delete an active sidecar.
pub(crate) struct OsFileLock { _file: OsFile }
impl OsFileLock {
    /// Open or create without truncating. None means busy, invalid path or I/O failure.
    pub(crate) fn acquire_raw(path: &str) -> Option<Self> {
        if !matches!(file_exists_checked_raw(path), Some(0 | 1)) { return None; }
        let file = OsFile::open_raw(path, FO_RW)?;
        // SAFETY: file owns a valid descriptor. flock(LOCK_EX | LOCK_NB) never blocks.
        let result = unsafe { syscall3(73, file.0 as usize, 6, 0) };
        if result == 0 { Some(Self { _file: file }) } else { None }
    }
}

/// Some(0) only for a missing path, Some(1/2) for file/directory, None for other errors.
pub(crate) fn file_exists_checked_raw(path: &str) -> Option<u32> {
    if path.is_empty() || path.contains('\0') { return None; }
    let p = path_z(path);
    let mut st = [0u64; 18];
    // SAFETY: p is NUL-terminated and st holds the x86-64 Linux stat structure.
    let result = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, p.as_ptr() as usize, st.as_mut_ptr() as usize, 0, 0, 0) };
    match result {
        0 => match st[3] as u32 & S_IFMT { S_IFDIR => Some(2), S_IFREG => Some(1), _ => None },
        -2 | -20 => Some(0), // ENOENT, ENOTDIR
        _ => None,
    }
}

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

impl OsFile {
    /// Create a private named file exclusively; never follow or overwrite an existing path.
    pub(crate) fn create_new_raw(path: &str) -> Option<OsFile> {
        let p = path_z(path);
        // SAFETY: NUL-terminated path; O_EXCL rejects existing paths (including symlinks).
        let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize,
            O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600, 0, 0) };
        if fd < 0 { None } else { Some(OsFile(fd as i32)) }
    }

    /// FO_READ (existing, read only), FO_WRITE (create / truncate), FO_RW (open or create)
    pub(crate) fn open_raw(path: &str, mode: u32) -> Option<OsFile> {
        let flags = match mode {
            FO_READ => O_CLOEXEC,
            FO_WRITE => O_RDWR | O_CREAT | O_TRUNC | O_CLOEXEC,
            FO_RW => O_RDWR | O_CREAT | O_CLOEXEC,
            _ => return None,
        };
        let p = path_z(path);
        // SAFETY: NUL-terminated path; 0666 (the umask applies).
        let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, flags, 0o666, 0, 0) };
        if fd < 0 { None } else { Some(OsFile(fd as i32)) }
    }

    /// a file without a name in dir (None = temp_dir): O_TMPFILE, else created and unlinked
    /// at once; nothing is left behind when it is closed or the process dies
    pub(crate) fn open_temp_raw(dir: Option<&str>) -> Option<OsFile> {
        let dir = match dir {
            Some(d) => String::from(d),
            None => temp_dir()?,
        };
        let dz = path_z(&dir);
        // SAFETY: NUL-terminated path; 0600.
        let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, dz.as_ptr() as usize, O_TMPFILE | O_RDWR | O_CLOEXEC, 0o600, 0, 0) };
        if fd >= 0 {
            return Some(OsFile(fd as i32));
        }
        // no O_TMPFILE (drvfs, old kernels): create, then unlink at once
        // SAFETY: no arguments.
        let pid = unsafe { syscall3(SYS_GETPID, 0, 0, 0) } as u32;
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
            let name = path_z(&format!("{dir}/.jfgui-{pid:x}-{seq:x}-{:x}.tmp", time_us() as u32));
            // SAFETY: NUL-terminated path.
            let fd = unsafe {
                syscall6(SYS_OPENAT, AT_FDCWD, name.as_ptr() as usize, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600, 0, 0)
            };
            if fd >= 0 {
                // SAFETY: NUL-terminated path.
                unsafe { syscall3(SYS_UNLINKAT, AT_FDCWD, name.as_ptr() as usize, 0) };
                return Some(OsFile(fd as i32));
            }
            if fd != -EEXIST {
                return None;
            }
        }
        None
    }

    /// bytes read at off (fewer at the end of the file), None on an error before any byte
    pub fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < buf.len() {
            let n = (buf.len() - done).min(IO_CHUNK);
            // SAFETY: buf[done..] holds at least n bytes.
            let r = unsafe {
                syscall6(SYS_PREAD64, self.0 as usize, buf.as_mut_ptr() as usize + done, n, off as usize + done, 0, 0)
            };
            if r == -EINTR {
                continue;
            }
            if r < 0 {
                if done > 0 {
                    break;
                }
                return None;
            }
            if r == 0 {
                break; // end of file
            }
            done += r as usize;
        }
        Some(done)
    }

    /// bytes written at off (the file grows as needed), None on an error before any byte
    pub fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < buf.len() {
            let n = (buf.len() - done).min(IO_CHUNK);
            // SAFETY: buf[done..] holds at least n bytes.
            let r = unsafe {
                syscall6(SYS_PWRITE64, self.0 as usize, buf.as_ptr() as usize + done, n, off as usize + done, 0, 0)
            };
            if r == -EINTR {
                continue;
            }
            if r < 0 {
                if done > 0 {
                    break;
                }
                return None;
            }
            if r == 0 {
                break;
            }
            done += r as usize;
        }
        Some(done)
    }

    pub fn size(&self) -> Option<u64> {
        let mut st = [0u64; 18];
        // SAFETY: st holds a struct stat.
        if unsafe { syscall3(SYS_FSTAT, self.0 as usize, st.as_mut_ptr() as usize, 0) } != 0 {
            return None;
        }
        Some(st[6])
    }

    /// write cached data to the disk (fsync)
    pub fn sync(&self) -> bool {
        // SAFETY: our descriptor.
        unsafe { syscall3(SYS_FSYNC, self.0 as usize, 0, 0) == 0 }
    }

    /// cut or extend the file to `len` bytes (ftruncate)
    pub fn set_len(&self, len: u64) -> bool {
        // SAFETY: our descriptor.
        unsafe { syscall3(77, self.0 as usize, len as usize, 0) == 0 }
    }
}

impl Drop for OsFile {
    fn drop(&mut self) {
        // SAFETY: the descriptor is owned by this File.
        unsafe { syscall3(SYS_CLOSE, self.0 as usize, 0, 0) };
    }
}
