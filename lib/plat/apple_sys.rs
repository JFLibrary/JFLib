// apple_sys -- files, directories and well-known paths on macOS and iOS (libSystem; the same
// functions as linux_sys.rs, whose comments say what each one promises)
//
// No static buffers: every function may run on any thread.  OsFile::read_at / write_at are pread /
// pwrite, so several threads may use one file at the same time.  sync is F_FULLFSYNC: on Apple a
// plain fsync leaves the data in the drive's cache.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_char;
use core::sync::atomic::{AtomicU32, Ordering};

use super::ffi::{self, Stat};
use super::{env, time_us};
use crate::{FO_READ, FO_RW, FO_WRITE};

const LINK_MAX: usize = 32; // symbolic links followed (MAXSYMLINKS)
const IO_CHUNK: usize = 0x4000_0000;
const TEMP_TRIES: u32 = 16;

fn path_z(path: &str) -> Vec<u8> {
    let mut p: Vec<u8> = path.as_bytes().to_vec();
    p.push(0);
    p
}

fn zp(p: &[u8]) -> *const c_char {
    p.as_ptr().cast()
}

/// struct stat of a path (links followed), Err(errno) when it cannot be read
fn stat_z(p: &[u8]) -> Result<Stat, i32> {
    let mut st = Stat::default();
    // SAFETY: p is NUL-terminated; st is a struct stat.
    if unsafe { ffi::stat(zp(p), &mut st) } == 0 { Ok(st) } else { Err(ffi::errno()) }
}

/// (st_dev, st_ino) of a path (links followed), None when it cannot be read  [fd_visit_once]
pub fn dev_ino(path: &str) -> Option<(u64, u64)> {
    let st = stat_z(&path_z(path)).ok()?;
    Some((st.st_dev as u32 as u64, st.st_ino))
}

/// drop trailing '/' (but keep "/")
fn trim_slash(mut s: String) -> String {
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

/// $HOME (absolute), else the home folder of the user database (getpwuid_r; a relative $HOME
/// counts as none)
fn home_dir() -> Option<String> {
    if let Some(h) = env("HOME") {
        return h.starts_with('/').then(|| trim_slash(h));
    }
    #[link(name = "System")]
    unsafe extern "C" {
        fn getuid() -> u32;
        fn getpwuid_r(uid: u32, pw: *mut [u64; 9], buf: *mut u8, size: usize, out: *mut *mut [u64; 9]) -> i32;
    }
    let mut pw = [0u64; 9]; // struct passwd: pw_dir at offset 48
    let mut buf = alloc::vec![0u8; 4096];
    let mut out: *mut [u64; 9] = core::ptr::null_mut();
    // SAFETY: a struct passwd and a buffer of its size for the strings it points into.
    let r = unsafe { getpwuid_r(getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() || pw[6] == 0 {
        return None;
    }
    // SAFETY: pw_dir points into buf, NUL-terminated.
    let dir = unsafe { core::ffi::CStr::from_ptr(pw[6] as *const core::ffi::c_char) }.to_string_lossy().into_owned();
    dir.starts_with('/').then(|| trim_slash(dir))
}

// ---------------------------------------------------------------- well-known directories

/// ~/Library/Application Support (on iOS inside the app's container, whose root is $HOME).  The same
/// folder as data_dir, as Rust's `dirs` crate has it: config:// and data:// share the app's folder.
pub fn config_dir() -> Option<String> {
    Some(home_dir()? + "/Library/Application Support")
}

/// ~/Library/Application Support
pub fn data_dir() -> Option<String> {
    config_dir()
}

/// ~/Library/Caches
pub fn cache_dir() -> Option<String> {
    Some(home_dir()? + "/Library/Caches")
}

/// $TMPDIR (absolute; macOS gives each user one, iOS each app), else /tmp
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
    if unsafe { ffi::getcwd(buf.as_mut_ptr().cast(), buf.len()) }.is_null() {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

// ---------------------------------------------------------------- directories and names

/// 0 missing, 1 file, 2 directory  [plat_file_exists]
pub(crate) fn file_exists_raw(path: &str) -> u32 {
    match stat_z(&path_z(path)) {
        Err(_) => 0,
        Ok(st) if st.st_mode & ffi::S_IFMT == ffi::S_IFDIR => 2,
        Ok(_) => 1,
    }
}

// one directory: created, or already a directory
fn mkdir1(path: &[u8]) -> bool {
    let p = {
        let mut p = path.to_vec();
        p.push(0);
        p
    };
    // SAFETY: NUL-terminated path; 0777 (the umask applies).
    if unsafe { ffi::mkdir(zp(&p), 0o777) } == 0 {
        return true;
    }
    ffi::errno() == ffi::EEXIST && matches!(stat_z(&p), Ok(st) if st.st_mode & ffi::S_IFMT == ffi::S_IFDIR)
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
        if ffi::unlink(zp(&p)) == 0 {
            return true;
        }
        let e = ffi::errno();
        (e == ffi::EPERM || e == ffi::EISDIR) && ffi::rmdir(zp(&p)) == 0
    }
}

/// rename, replacing an existing target atomically (same file system)  [plat_rename]
pub(crate) fn rename_raw(old: &str, new: &str) -> bool {
    let (o, n) = (path_z(old), path_z(new));
    // SAFETY: NUL-terminated paths.
    unsafe { ffi::rename(zp(&o), zp(&n)) == 0 }
}

/// f(name, is_dir, size_lo) for every entry except "." and ".." (links followed)  [plat_dir_list]
pub(crate) fn dir_list_raw(path: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    let p = path_z(path);
    // SAFETY: NUL-terminated path; the stream is closed below.
    let dir = unsafe { ffi::opendir(zp(&p)) };
    if dir.is_null() {
        return false;
    }
    let base = if path.ends_with('/') { String::from(path) } else { format!("{path}/") };
    loop {
        // SAFETY: dir is an open stream; the entry stays valid until the next readdir.
        let e = unsafe { ffi::readdir(dir) };
        if e.is_null() {
            break; // the end (or an error, which readdir does not tell apart without errno)
        }
        // SAFETY: e points to a struct dirent of this stream whose name holds d_namlen bytes; no
        // reference to the whole record is made (readdir's buffer may end before its 1024 bytes).
        let (name, dtype) = unsafe {
            let n = ((*e).d_namlen as usize).min(1024);
            let p = core::ptr::addr_of!((*e).d_name) as *const u8;
            (core::slice::from_raw_parts(p, n).to_vec(), (*e).d_type)
        };
        if name == b"." || name == b".." {
            continue;
        }
        let full = path_z(&format!("{base}{}", String::from_utf8_lossy(&name)));
        let (is_dir, size) = match stat_z(&full) {
            Ok(st) => (st.st_mode & ffi::S_IFMT == ffi::S_IFDIR, st.st_size as u32),
            Err(_) => (dtype == ffi::DT_DIR, 0), // a broken link etc.: trust d_type
        };
        f(&String::from_utf8_lossy(&name), is_dir, size);
    }
    // SAFETY: our stream.
    unsafe { ffi::closedir(dir) };
    true
}

// ---------------------------------------------------------------- whole files

/// the contents of a file, read until the end  [plat_file_read]
pub(crate) fn read_file_raw(path: &str) -> Option<Vec<u8>> {
    let f = OsFile::open_raw(path, FO_READ)?;
    let size = f.size()?;
    if size > 0x7FFF_FFF0 {
        return None;
    }
    // memory that is not there is None, not an abort (as linux_sys.rs)
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(size as usize + 4096).ok()?;
    drop(probe);
    let mut data = alloc::vec![0u8; size as usize + 4096];
    let mut n = 0;
    loop {
        n += f.read_at(n as u64, &mut data[n..])?;
        if n < data.len() {
            break; // the end of the file came first
        }
        data.try_reserve_exact(data.len()).ok()?;
        data.resize(data.len() * 2, 0);
    }
    data.truncate(n);
    Some(data)
}

/// Write an exclusive sibling, preserve existing access bits, sync and rename. [plat_file_write]
pub(crate) fn write_file_raw(path: &str, data: &[u8]) -> bool {
    OsAtomic::begin(path).is_some_and(|a| a.write_at(0, data) == Some(data.len()) && a.commit())
}

/// A file written beside its target and renamed over it by commit.  Dropped without commit, the
/// sibling is removed.
pub(crate) struct OsAtomic {
    f: Option<OsFile>,
    tmp: Vec<u8>, // NUL-terminated; empty once renamed
    dst: Vec<u8>,
    mode: Option<u16>,
    owner: (u32, u32), // uid, gid of the replaced file
}

/// The file a symbolic link at path finally points to (path itself when it is no link or does not
/// exist).  None for a loop or a link that cannot be read.
fn link_target(path: &str) -> Option<String> {
    let mut cur = String::from(path);
    for _ in 0..LINK_MAX {
        let z = path_z(&cur);
        let mut buf = [0u8; 4096];
        // SAFETY: NUL-terminated path and a writable buffer of the given size.
        let n = unsafe { ffi::readlink(zp(&z), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = ffi::errno();
            return if e == ffi::EINVAL || e == ffi::ENOENT { Some(cur) } else { None }; // no link / nothing there
        }
        if n == 0 || n as usize >= buf.len() {
            return None;
        }
        let t = core::str::from_utf8(&buf[..n as usize]).ok()?;
        cur = if t.starts_with('/') {
            String::from(t)
        } else {
            match cur.rfind('/') {
                Some(k) => format!("{}{}", &cur[..=k], t),
                None => String::from(t),
            }
        };
    }
    None
}

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

impl OsAtomic {
    /// An exclusively created sibling "PATH.tmp-PID-SEQ-TIME"; an existing regular file keeps its
    /// access bits at commit, anything else at path (a folder, a device) refuses.
    pub(crate) fn begin(path: &str) -> Option<OsAtomic> {
        if path.is_empty() || path.contains('\0') {
            return None;
        }
        let path = link_target(path)?;
        let path = path.as_str();
        let dst = path_z(path);
        let (mode, owner) = match stat_z(&dst) {
            Ok(st) if st.st_mode & ffi::S_IFMT == ffi::S_IFREG => (Some(st.st_mode & 0o777), (st.st_uid, st.st_gid)),
            Err(e) if e == ffi::ENOENT => (None, (0, 0)), // new files get the usual umask-controlled mode
            _ => return None,
        };
        // SAFETY: no arguments.
        let pid = unsafe { ffi::getpid() } as u32;
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = path_z(&format!("{path}.tmp-{pid:x}-{seq:x}-{:x}", time_us() as u32));
            let perm: u32 = if mode.is_some() { 0o600 } else { 0o666 };
            // SAFETY: NUL-terminated path; O_EXCL never opens a pre-existing entry; the mode is the
            // variadic argument.
            let fd = unsafe { ffi::open(zp(&tmp), ffi::O_WRONLY | ffi::O_CREAT | ffi::O_EXCL | ffi::O_CLOEXEC, perm) };
            if fd < 0 {
                if ffi::errno() == ffi::EEXIST {
                    continue;
                }
                return None;
            }
            return Some(OsAtomic { f: Some(OsFile(fd)), tmp, dst, mode, owner });
        }
        None
    }

    /// bytes written at off of the sibling
    pub(crate) fn write_at(&self, off: u64, data: &[u8]) -> Option<usize> {
        self.f.as_ref()?.write_at(off, data)
    }

    /// the old file's access bits, sync, rename over the target -> false: the target is unchanged
    /// and the sibling gone
    pub(crate) fn commit(mut self) -> bool {
        let Some(f) = self.f.take() else { return false };
        let mut ok = true;
        if let Some(mode) = self.mode {
            // SAFETY: f owns its descriptor; the owner is restored when the process may, then the
            // access bits, not the setuid/setgid bits.
            unsafe { ffi::fchown(f.0, self.owner.0, self.owner.1) };
            ok &= unsafe { ffi::fchmod(f.0, mode) } == 0;
        }
        ok &= f.sync();
        drop(f);
        // SAFETY: both paths are NUL-terminated.
        if ok && unsafe { ffi::rename(zp(&self.tmp), zp(&self.dst)) } == 0 {
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
            unsafe { ffi::unlink(zp(&self.tmp)) };
        }
    }
}

// ---------------------------------------------------------------- streaming files

/// an open file (closed when dropped)
pub(crate) struct OsFile(i32);

/// Nonblocking advisory sidecar lock. Drop closes the descriptor and releases the lock.
pub(crate) struct OsFileLock {
    _file: OsFile,
}
impl OsFileLock {
    /// Open or create without truncating. None means busy, invalid path or I/O failure.
    pub(crate) fn acquire_raw(path: &str) -> Option<Self> {
        if !matches!(file_exists_checked_raw(path), Some(0 | 1)) {
            return None;
        }
        let file = OsFile::open_raw(path, FO_RW)?;
        // SAFETY: file owns a valid descriptor; flock(LOCK_EX | LOCK_NB) never blocks.
        let result = unsafe { ffi::flock(file.0, ffi::LOCK_EX | ffi::LOCK_NB) };
        if result == 0 { Some(Self { _file: file }) } else { None }
    }
}

/// Some(0) only for a missing path, Some(1/2) for file/directory, None for other errors.
pub(crate) fn file_exists_checked_raw(path: &str) -> Option<u32> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    match stat_z(&path_z(path)) {
        Ok(st) => match st.st_mode & ffi::S_IFMT {
            ffi::S_IFDIR => Some(2),
            ffi::S_IFREG => Some(1),
            _ => None,
        },
        Err(e) if e == ffi::ENOENT || e == ffi::ENOTDIR => Some(0),
        Err(_) => None,
    }
}

fn open_z(p: &[u8], flags: i32, mode: u32) -> Option<OsFile> {
    // SAFETY: NUL-terminated path; the mode is the variadic argument (used with O_CREAT only).
    let fd = unsafe { ffi::open(zp(p), flags, mode) };
    if fd < 0 { None } else { Some(OsFile(fd)) }
}

impl OsFile {
    /// Create a private named file exclusively; never follow or overwrite an existing path.
    #[allow(dead_code)] // (fs.rs: write_file_private, AP3)
    pub(crate) fn create_new_raw(path: &str) -> Option<OsFile> {
        open_z(&path_z(path), ffi::O_RDWR | ffi::O_CREAT | ffi::O_EXCL | ffi::O_CLOEXEC, 0o600)
    }

    /// FO_READ (existing, read only), FO_WRITE (create / truncate), FO_RW (open or create)
    pub(crate) fn open_raw(path: &str, mode: u32) -> Option<OsFile> {
        let flags = match mode {
            FO_READ => ffi::O_RDONLY | ffi::O_CLOEXEC,
            FO_WRITE => ffi::O_RDWR | ffi::O_CREAT | ffi::O_TRUNC | ffi::O_CLOEXEC,
            FO_RW => ffi::O_RDWR | ffi::O_CREAT | ffi::O_CLOEXEC,
            _ => return None,
        };
        open_z(&path_z(path), flags, 0o666)
    }

    /// read only, never through a final symbolic link
    #[allow(dead_code)]
    pub(crate) fn open_read_nofollow(path: &str) -> Option<OsFile> {
        open_z(&path_z(path), ffi::O_RDONLY | ffi::O_CLOEXEC | ffi::O_NOFOLLOW, 0)
    }

    /// a file without a name in dir (None = temp_dir): created and unlinked at once; nothing is
    /// left behind when it is closed or the process dies
    pub(crate) fn open_temp_raw(dir: Option<&str>) -> Option<OsFile> {
        let dir = match dir {
            Some(d) => String::from(d),
            None => temp_dir()?,
        };
        // SAFETY: no arguments.
        let pid = unsafe { ffi::getpid() } as u32;
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
            let name = path_z(&format!("{dir}/.jfgui-{pid:x}-{seq:x}-{:x}.tmp", time_us() as u32));
            if let Some(f) = open_z(&name, ffi::O_RDWR | ffi::O_CREAT | ffi::O_EXCL | ffi::O_CLOEXEC, 0o600) {
                // SAFETY: NUL-terminated path.
                unsafe { ffi::unlink(zp(&name)) };
                return Some(f);
            }
            if ffi::errno() != ffi::EEXIST {
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
            let r = unsafe { ffi::pread(self.0, buf.as_mut_ptr().add(done).cast(), n, (off + done as u64) as i64) };
            if r < 0 {
                if ffi::errno() == ffi::EINTR {
                    continue;
                }
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
            let r = unsafe { ffi::pwrite(self.0, buf.as_ptr().add(done).cast(), n, (off + done as u64) as i64) };
            if r < 0 {
                if ffi::errno() == ffi::EINTR {
                    continue;
                }
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
        let mut st = Stat::default();
        // SAFETY: st is a struct stat.
        if unsafe { ffi::fstat(self.0, &mut st) } != 0 {
            return None;
        }
        Some(st.st_size as u64)
    }

    /// write cached data to the disk: F_FULLFSYNC (a plain fsync leaves it in the drive's
    /// cache), fsync where the file system does not support it
    pub fn sync(&self) -> bool {
        // SAFETY: our descriptor; F_FULLFSYNC takes no argument.
        unsafe { ffi::fcntl(self.0, ffi::F_FULLFSYNC) == 0 || ffi::fsync(self.0) == 0 }
    }

    /// cut or extend the file to `len` bytes (ftruncate)
    #[allow(dead_code)] // (the callers come with AP3)
    pub fn set_len(&self, len: u64) -> bool {
        // SAFETY: our descriptor.
        unsafe { ffi::ftruncate(self.0, len as i64) == 0 }
    }
}

impl Drop for OsFile {
    fn drop(&mut self) {
        // SAFETY: the descriptor is owned by this file.
        unsafe { ffi::close(self.0) };
    }
}
