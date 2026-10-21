// Android's files: the virtual names of the host adapter (/config, /data, /cache, /tmp,
// /documents, /imports -- the same as in the browser, so the permissions and the vfs see one
// layout) on real files of the app's internal storage, through Bionic.  They survive restarts.
//
//   /cache/..  -> <cacheDir>/cache/..   (the system may clear it when storage runs low)
//   /tmp/..    -> <cacheDir>/tmp/..     (emptied when the app starts)
//   /..        -> <filesDir>/fs/..      (config, data, documents, imports and anything else)
// GalleryActivity passes filesDir / cacheDir before the start (jfgui_ANDROID_FILES,
// jfgui_ANDROID_CACHE); without them every file call fails.
//
// write_file and AtomicFile write a sibling "NAME.tmp-PID-SEQ-TIME", fsync it and rename it
// over the target (readers see the old or the new file, also after a crash).  FileLock is
// flock (across processes of the app).  open_temp unlinks its file at once.  Writing needs the
// folder to exist (as on Linux; config:// etc. create theirs).  The documents of other apps
// are content:// names (content.rs, android_host.rs), not paths.

use crate::{FO_READ, FO_RW, FO_WRITE};
use alloc::{format, string::String, vec::Vec};
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use rt::sync::SpinLock;

#[link(name = "c")]
unsafe extern "C" {
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn pread(fd: i32, buf: *mut u8, n: usize, off: i64) -> isize;
    fn pwrite(fd: i32, buf: *const u8, n: usize, off: i64) -> isize;
    fn lseek(fd: i32, off: i64, whence: i32) -> i64;
    fn fsync(fd: i32) -> i32;
    fn ftruncate(fd: i32, len: i64) -> i32;
    fn flock(fd: i32, op: i32) -> i32;
    fn rename(old: *const u8, new: *const u8) -> i32;
    fn mkdir(path: *const u8, mode: u32) -> i32;
    fn unlink(path: *const u8) -> i32;
    fn rmdir(path: *const u8) -> i32;
    fn opendir(path: *const u8) -> *mut c_void;
    fn readdir(dir: *mut c_void) -> *const u8;
    fn closedir(dir: *mut c_void) -> i32;
    fn stat(path: *const u8, st: *mut u8) -> i32;
    fn getpid() -> i32;
    fn __errno() -> *mut i32;
}

// open flags and errors (the same on arm64 and x86_64)
const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_TRUNC: i32 = 0o1000;
const O_CLOEXEC: i32 = 0o2000000;
const SEEK_END: i32 = 2;
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const ENOENT: i32 = 2;
const EEXIST: i32 = 17;
const ENOTDIR: i32 = 20;
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const DT_DIR: u8 = 4;
const DT_REG: u8 = 8;
// struct stat: st_mode moves, st_size (48) and st_mtim (88) do not
#[cfg(target_arch = "x86_64")]
const ST_MODE: usize = 24;
#[cfg(not(target_arch = "x86_64"))]
const ST_MODE: usize = 16;
const ST_SIZE: usize = 48;
const ST_MTIME: usize = 88;
// struct dirent (LP64 Bionic): d_ino, d_off, d_reclen, d_type, d_name
const D_TYPE: usize = 18;
const D_NAME: usize = 19;

const BUILTIN: [&str; 6] = ["/tmp", "/config", "/data", "/cache", "/documents", "/imports"];
static ROOTS: SpinLock<Option<(String, String)>> = SpinLock::new(None);
static TEMP: AtomicU32 = AtomicU32::new(1);

fn errno() -> i32 {
    // SAFETY: Bionic's thread-local errno slot.
    unsafe { *__errno() }
}

// NUL-terminated copy of a path
fn z(path: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(path.len() + 1);
    v.extend_from_slice(path.as_bytes());
    v.push(0);
    v
}

// (mode, size, mtime ns) of a real path; Err(errno)
fn stat_z(p: &[u8]) -> Result<(u32, u64, i64), i32> {
    let mut st = [0u64; 32]; // 256 bytes, 8-aligned: more than either struct stat
    // SAFETY: p is NUL-terminated; st is writable and larger than struct stat.
    if unsafe { stat(p.as_ptr(), st.as_mut_ptr().cast()) } != 0 {
        return Err(errno());
    }
    // every field read here starts at a multiple of 8 (little endian: st_mode is the low half)
    let b = |o: usize| -> u64 { st[o / 8] };
    Ok((b(ST_MODE) as u32, b(ST_SIZE), (b(ST_MTIME) as i64).saturating_mul(1_000_000_000).saturating_add(b(ST_MTIME + 8) as i64)))
}

// the virtual path normalized: "/a/b" ("..": up, never above "/")
fn path(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in value.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn builtin(path: &str) -> bool {
    path == "/" || BUILTIN.contains(&path)
}

// mkdir -p of a real folder
fn mkdirs(real: &str) -> bool {
    let mut at = 1;
    while at <= real.len() {
        let end = real[at..].find('/').map_or(real.len(), |k| at + k);
        let p = z(&real[..end]);
        // SAFETY: NUL-terminated path.
        if unsafe { mkdir(p.as_ptr(), 0o700) } != 0 && errno() != EEXIST {
            return false;
        }
        at = end + 1;
    }
    true
}

// everything below a real folder, then the folder (the /tmp sweep at start)
fn remove_tree(real: &str) {
    let mut kids = Vec::new();
    list_real(real, &mut |n, d, _| kids.push((format!("{real}/{n}"), d)));
    for (p, d) in kids {
        if d {
            remove_tree(&p);
        } else {
            let p = z(&p);
            // SAFETY: NUL-terminated path.
            unsafe { unlink(p.as_ptr()) };
        }
    }
    let p = z(real);
    // SAFETY: NUL-terminated path.
    unsafe { rmdir(p.as_ptr()) };
}

// filesDir, cacheDir (from the host's environment, set up once: the built-in folders, /tmp emptied)
fn roots() -> Option<(String, String)> {
    let mut g = ROOTS.lock();
    if let Some(r) = g.as_ref() {
        return Some(r.clone());
    }
    let files = super::env("jfgui_ANDROID_FILES").filter(|d| d.starts_with('/'))?;
    let cache = super::env("jfgui_ANDROID_CACHE").filter(|d| d.starts_with('/'))?;
    let (files, cache) = (String::from(files.trim_end_matches('/')), String::from(cache.trim_end_matches('/')));
    remove_tree(&format!("{cache}/tmp"));
    for d in ["config", "data", "documents", "imports"] {
        mkdirs(&format!("{files}/fs/{d}"));
    }
    mkdirs(&format!("{cache}/cache"));
    mkdirs(&format!("{cache}/tmp"));
    *g = Some((files.clone(), cache.clone()));
    Some((files, cache))
}

// the real path of a normalized virtual one
fn real(v: &str) -> Option<String> {
    let (files, cache) = roots()?;
    let under = |root: &str| v == root || v.starts_with(root) && v.as_bytes().get(root.len()) == Some(&b'/');
    Some(if under("/cache") || under("/tmp") {
        format!("{cache}{v}")
    } else if v == "/" {
        format!("{files}/fs")
    } else {
        format!("{files}/fs{v}")
    })
}

// both: the normalized virtual path and its real one (NUL-terminated)
fn resolve(name: &str) -> Option<(String, Vec<u8>)> {
    let v = path(name)?;
    let r = real(&v)?;
    Some((v, z(&r)))
}

fn list_real(real: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
    let p = z(real);
    // SAFETY: NUL-terminated path; the DIR stream is closed below.
    let d = unsafe { opendir(p.as_ptr()) };
    if d.is_null() {
        return false;
    }
    let mut entries = Vec::new();
    loop {
        // SAFETY: d is an open DIR stream; the entry stays valid until the next readdir.
        let e = unsafe { readdir(d) };
        if e.is_null() {
            break;
        }
        // SAFETY: d_name is NUL-terminated inside the entry.
        let (typ, name) = unsafe {
            let typ = *e.add(D_TYPE);
            let mut n = 0;
            while *e.add(D_NAME + n) != 0 {
                n += 1;
            }
            (typ, core::slice::from_raw_parts(e.add(D_NAME), n).to_vec())
        };
        let Ok(name) = String::from_utf8(name) else { continue };
        if name == "." || name == ".." {
            continue;
        }
        entries.push((name, typ));
    }
    // SAFETY: d came from opendir above.
    unsafe { closedir(d) };
    for (name, typ) in entries {
        let full = z(&format!("{real}/{name}"));
        let (dir, size) = match (typ, stat_z(&full)) {
            (_, Ok((m, s, _))) => (m & S_IFMT == S_IFDIR, if m & S_IFMT == S_IFDIR { 0 } else { s }),
            (DT_DIR, _) => (true, 0),
            (DT_REG, _) => (false, 0),
            _ => continue,
        };
        f(&name, dir, size);
    }
    true
}

/// Current virtual directory; it is unrelated to the process's working directory.
pub fn cwd() -> String {
    String::from("/")
}
/// The app's settings below internal storage.
pub fn config_dir() -> Option<String> {
    Some(String::from("/config"))
}
/// The app's data below internal storage.
pub fn data_dir() -> Option<String> {
    Some(String::from("/data"))
}
/// The cache below the app's cache folder (the system may clear it).
pub fn cache_dir() -> Option<String> {
    Some(String::from("/cache"))
}
/// Temporary files (emptied when the app starts).
pub fn temp_dir() -> Option<String> {
    Some(String::from("/tmp"))
}
/// The app's own documents (internal storage; the user's documents are content:// names).
pub fn home() -> Option<String> {
    Some(String::from("/documents"))
}
/// One virtual root.
pub fn drives() -> Vec<String> {
    alloc::vec![String::from("/")]
}
/// Device / inode identity is not exposed for the virtual names.
pub fn dev_ino(_path: &str) -> Option<(u64, u64)> {
    None
}
/// File (1), folder (2) or missing (Some(0)); None for an invalid name or another error.
pub(crate) fn file_exists_checked_raw(name: &str) -> Option<u32> {
    let (v, r) = resolve(name)?;
    if builtin(&v) {
        return Some(2);
    }
    match stat_z(&r) {
        Ok((m, _, _)) => Some(if m & S_IFMT == S_IFDIR { 2 } else { 1 }),
        Err(ENOENT) | Err(ENOTDIR) => Some(0),
        Err(_) => None,
    }
}
/// Existing file (1), folder (2), or absent/invalid name (0).
pub(crate) fn file_exists_raw(name: &str) -> u32 {
    file_exists_checked_raw(name).unwrap_or(0)
}
/// Create a folder and its missing parents; an existing folder is fine.
pub(crate) fn mkdir_raw(name: &str) -> bool {
    let Some(v) = path(name) else { return false };
    let Some(r) = real(&v) else { return false };
    builtin(&v) || mkdirs(&r)
}
/// Delete a file or an empty folder (never a built-in one).
pub(crate) fn delete_raw(name: &str) -> bool {
    let Some((v, r)) = resolve(name) else { return false };
    if builtin(&v) {
        return false;
    }
    match stat_z(&r) {
        // SAFETY: NUL-terminated path.
        Ok((m, _, _)) if m & S_IFMT == S_IFDIR => unsafe { rmdir(r.as_ptr()) == 0 },
        // SAFETY: NUL-terminated path.
        Ok(_) => unsafe { unlink(r.as_ptr()) == 0 },
        Err(_) => false,
    }
}
/// Rename a file or folder, replacing a file at the new name in one step.
pub(crate) fn rename_raw(old: &str, new: &str) -> bool {
    let (Some((a, ra)), Some((b, rb))) = (resolve(old), resolve(new)) else { return false };
    if builtin(&a) || builtin(&b) {
        return false;
    }
    // SAFETY: both paths are NUL-terminated.
    unsafe { rename(ra.as_ptr(), rb.as_ptr()) == 0 }
}
/// f(name, is_dir, size) for every entry of a folder.
pub(crate) fn dir_list_raw(name: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    let Some(v) = path(name) else { return false };
    let Some(r) = real(&v) else { return false };
    if !list_real(&r, &mut |n, d, s| f(n, d, s as u32)) {
        return false;
    }
    if v == "/" {
        // the folders of the cache live beside filesDir
        f("cache", true, 0);
        f("tmp", true, 0);
    }
    true
}
/// The whole file.
pub(crate) fn read_file_raw(name: &str) -> Option<Vec<u8>> {
    let file = OsFile::open_raw(name, FO_READ)?;
    let n = usize::try_from(file.size()?).ok()?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).ok()?;
    out.resize(n, 0);
    let mut at = 0;
    while at < n {
        match file.read_at(at as u64, &mut out[at..])? {
            0 => break,
            k => at += k,
        }
    }
    out.truncate(at);
    Some(out)
}
/// Replace a file in one step: a sibling written, synced and renamed over it.
pub(crate) fn write_file_raw(name: &str, bytes: &[u8]) -> bool {
    OsAtomic::begin(name).is_some_and(|a| a.write_at(0, bytes) == Some(bytes.len()) && a.commit())
}

/// A sibling "NAME.tmp-PID-SEQ-TIME" renamed over the file by commit (write_file_raw, the
/// streaming writers of guarded.rs / aio.rs).  Dropped without commit, the sibling goes.
pub(crate) struct OsAtomic {
    f: Option<OsFile>,
    tmp: Vec<u8>,
    dst: Vec<u8>,
}
impl OsAtomic {
    /// A writer of name; a folder there refuses.
    pub(crate) fn begin(name: &str) -> Option<Self> {
        let (v, dst) = resolve(name)?;
        if builtin(&v) {
            return None;
        }
        match stat_z(&dst) {
            Ok((m, _, _)) if m & S_IFMT == S_IFDIR => return None,
            Ok(_) | Err(ENOENT) => {}
            Err(_) => return None,
        }
        // SAFETY: no arguments.
        let pid = unsafe { getpid() } as u32;
        let base = &dst[..dst.len() - 1];
        for _ in 0..16 {
            let seq = TEMP.fetch_add(1, Ordering::Relaxed);
            let mut tmp = base.to_vec();
            tmp.extend_from_slice(format!(".tmp-{pid:x}-{seq:x}-{:x}", crate::time_us() as u32).as_bytes());
            tmp.push(0);
            // SAFETY: NUL-terminated path; O_EXCL never opens an existing entry.
            let fd = unsafe { open(tmp.as_ptr(), O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0o600u32) };
            if fd < 0 {
                if errno() == EEXIST {
                    continue;
                }
                return None;
            }
            return Some(Self { f: Some(OsFile { fd, writable: true }), tmp, dst });
        }
        None
    }
    /// Bytes written at an offset of the sibling.
    pub(crate) fn write_at(&self, offset: u64, bytes: &[u8]) -> Option<usize> {
        self.f.as_ref()?.write_at(offset, bytes)
    }
    /// fsync, then rename over the target -> false: the target is unchanged, the sibling gone.
    pub(crate) fn commit(mut self) -> bool {
        let Some(f) = self.f.take() else { return false };
        let ok = f.sync();
        drop(f);
        // SAFETY: both paths are NUL-terminated.
        if ok && unsafe { rename(self.tmp.as_ptr(), self.dst.as_ptr()) } == 0 {
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
            // SAFETY: NUL-terminated path of our own sibling.
            unsafe { unlink(self.tmp.as_ptr()) };
        }
    }
}

/// An open file (closed when dropped).
pub(crate) struct OsFile {
    fd: i32,
    writable: bool,
}
impl OsFile {
    /// FO_READ (existing), FO_WRITE (create / truncate), FO_RW (open or create).
    pub(crate) fn open_raw(name: &str, mode: u32) -> Option<Self> {
        let (v, r) = resolve(name)?;
        if builtin(&v) {
            return None;
        }
        let flags = match mode {
            FO_READ => O_RDONLY,
            FO_WRITE => O_RDWR | O_CREAT | O_TRUNC,
            FO_RW => O_RDWR | O_CREAT,
            _ => return None,
        } | O_CLOEXEC;
        if let Ok((m, _, _)) = stat_z(&r) {
            if m & S_IFMT == S_IFDIR {
                return None;
            }
        }
        // SAFETY: NUL-terminated path; the descriptor is owned by the result.
        let fd = unsafe { open(r.as_ptr(), flags, 0o600u32) };
        (fd >= 0).then_some(Self { fd, writable: mode != FO_READ })
    }
    /// A scratch file in directory (None: /tmp), its name removed at once.
    pub(crate) fn open_temp_raw(directory: Option<&str>) -> Option<Self> {
        for _ in 0..16 {
            let name = format!("{}/jfgui-{:x}-{:x}.tmp", directory.unwrap_or("/tmp"), TEMP.fetch_add(1, Ordering::Relaxed), crate::time_us() as u32);
            let (v, r) = resolve(&name)?;
            if builtin(&v) {
                return None;
            }
            // SAFETY: NUL-terminated path; O_EXCL never opens an existing entry.
            let fd = unsafe { open(r.as_ptr(), O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600u32) };
            if fd < 0 {
                if errno() == EEXIST {
                    continue;
                }
                return None;
            }
            // SAFETY: NUL-terminated path of the file just created.
            unsafe { unlink(r.as_ptr()) };
            return Some(Self { fd, writable: true });
        }
        None
    }
    /// Bytes read at an offset, zero at the end.
    pub fn read_at(&self, offset: u64, out: &mut [u8]) -> Option<usize> {
        let off = i64::try_from(offset).ok()?;
        // SAFETY: out is writable for its length; fd is open.
        let n = unsafe { pread(self.fd, out.as_mut_ptr(), out.len(), off) };
        usize::try_from(n).ok()
    }
    /// Bytes written at an offset (the file grows).
    pub fn write_at(&self, offset: u64, bytes: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let off = i64::try_from(offset).ok()?;
        let mut done = 0;
        while done < bytes.len() {
            // SAFETY: the rest of bytes is readable; fd is open.
            let n = unsafe { pwrite(self.fd, bytes[done..].as_ptr(), bytes.len() - done, off + done as i64) };
            if n <= 0 {
                return (done > 0).then_some(done);
            }
            done += n as usize;
        }
        Some(done)
    }
    /// The file's length.
    pub fn size(&self) -> Option<u64> {
        // SAFETY: fd is open; positional reads and writes do not use the offset.
        let n = unsafe { lseek(self.fd, 0, SEEK_END) };
        u64::try_from(n).ok()
    }
    /// The data on the storage (fsync).
    pub fn sync(&self) -> bool {
        // SAFETY: fd is open.
        !self.writable || unsafe { fsync(self.fd) } == 0
    }
    /// Cut or extend the file.
    pub fn set_len(&self, len: u64) -> bool {
        // SAFETY: fd is open.
        self.writable && i64::try_from(len).is_ok_and(|l| unsafe { ftruncate(self.fd, l) } == 0)
    }
}
impl Drop for OsFile {
    fn drop(&mut self) {
        // SAFETY: the descriptor is owned and closed once.
        unsafe { close(self.fd) };
    }
}

/// An exclusive flock on a sidecar file until Drop (busy: None).
pub(crate) struct OsFileLock {
    _f: OsFile,
}
impl OsFileLock {
    pub(crate) fn acquire_raw(name: &str) -> Option<Self> {
        let f = OsFile::open_raw(name, FO_RW)?;
        // SAFETY: fd is open; nonblocking.
        (unsafe { flock(f.fd, LOCK_EX | LOCK_NB) } == 0).then_some(Self { _f: f })
    }
}

/// (web.rs) path, revision and bytes of a file -- the export of download()
pub(super) fn snapshot(name: &str) -> Option<(String, u64, Vec<u8>)> {
    let v = path(name)?;
    let data = read_file_raw(&v)?;
    Some((v, 0, data))
}
/// (web.rs) nothing to remember: the files are on the storage once written
pub(super) fn persisted(_name: &str, _revision: u64) {}
/// (web.rs persist) the file's data on the storage
pub(super) fn sync_path(name: &str) -> bool {
    OsFile::open_raw(name, FO_READ).is_some_and(|f| {
        // a read-only descriptor syncs the file's data too
        // SAFETY: fd is open.
        unsafe { fsync(f.fd) == 0 }
    })
}
