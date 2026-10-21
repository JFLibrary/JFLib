// guarded -- the public file functions: a name with a scheme goes to its vfs provider
// (vfs.rs), any other name is checked against the app's permissions (policy.rs) and then
// handed to the OS call (win_sys.rs, linux_sys.rs, web_files.rs: the *_raw functions, which
// the platform layer uses for its own system files)
//
// trash(path) moves into the trash (trash.rs) and counts as writing the path.
// A refused path looks like a missing or unreadable one: 0 / None / false.
// File and FileLock hold an OS file (also behind a folder mount: os_path) or a stream of a
// mount (Provider::open: MemFs, Bundle, ZipFs, Overlay, Sub; other providers: the whole file,
// read at the start and written back by sync and drop); a FileLock on a mount without OS
// files holds inside this process only.
// AtomicFile is write_file in pieces: a sibling (OS: OsAtomic of win_sys / linux_sys /
// web_files / android_files; mounts: "NAME.tmp-.." in the mount) put in place by commit,
// removed when dropped; content:// (no rename over a document): the bytes in memory, written
// in place by commit (aio.rs builds the async writes and copies on it).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use crate::policy::{fs_allowed, FS_READ, FS_WRITE};
use crate::vfs::{self, Opened, Provider, Route, Stream};
use crate::{OsFile, OsFileLock, FO_READ, FO_RW, FO_WRITE};

/// 0 missing, 1 file, 2 directory  [plat_file_exists]
pub fn file_exists(path: &str) -> u32 {
    match vfs::route(path) {
        Route::Os => if fs_allowed(path, FS_READ) { crate::file_exists_raw(path) } else { 0 },
        Route::Vfs(p, rest) => p.kind(&rest),
        Route::Bad => 0,
    }
}

/// Some(0) only for a missing path, Some(1/2) for file/directory, None for other errors.
pub fn file_exists_checked(path: &str) -> Option<u32> {
    match vfs::route(path) {
        Route::Os => if fs_allowed(path, FS_READ) { crate::file_exists_checked_raw(path) } else { None },
        Route::Vfs(p, rest) => Some(p.kind(&rest)),
        Route::Bad => None,
    }
}

/// create a directory and its missing parents; an existing directory is fine  [plat_mkdir]
pub fn mkdir(path: &str) -> bool {
    match vfs::route(path) {
        Route::Os => fs_allowed(path, FS_WRITE) && crate::mkdir_raw(path),
        Route::Vfs(p, rest) => p.mkdir(&rest),
        Route::Bad => false,
    }
}

/// a file or an empty directory  [plat_delete]
pub fn delete(path: &str) -> bool {
    match vfs::route(path) {
        Route::Os => fs_allowed(path, FS_WRITE) && crate::delete_raw(path),
        Route::Vfs(p, rest) => p.delete(&rest),
        Route::Bad => false,
    }
}

/// move a file or folder into the trash (Windows: the recycle bin; Linux: the freedesktop
/// trash) -> false: it stays where it was, never deleted for good (trash.rs).  Counts as
/// writing the path; a vfs name needs a folder mount.
pub fn trash(path: &str) -> bool {
    match vfs::route(path) {
        Route::Os => fs_allowed(path, FS_WRITE) && crate::trash::trash_raw(path),
        Route::Vfs(p, rest) => !rest.is_empty() && p.os_path(&rest).is_some_and(|o| fs_allowed(&o, FS_WRITE) && crate::trash::trash_raw(&o)),
        Route::Bad => false,
    }
}

/// rename, replacing an existing target (both names on the OS or in the same mount)  [plat_rename]
pub fn rename(old: &str, new: &str) -> bool {
    match (vfs::route(old), vfs::route(new)) {
        (Route::Os, Route::Os) => fs_allowed(old, FS_WRITE) && fs_allowed(new, FS_WRITE) && crate::rename_raw(old, new),
        (Route::Vfs(p, a), Route::Vfs(q, b)) if Arc::ptr_eq(&p, &q) => p.rename(&a, &b),
        _ => false,
    }
}

/// f(name, is_dir, size_lo) for every entry except "." and ".."  [plat_dir_list]
pub fn dir_list(path: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    match vfs::route(path) {
        Route::Os => fs_allowed(path, FS_READ) && crate::dir_list_raw(path, f),
        Route::Vfs(p, rest) => p.list(&rest, &mut |n, d, s| f(n, d, s as u32)),
        Route::Bad => false,
    }
}

/// the contents of a file  [plat_file_read]
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    match vfs::route(path) {
        Route::Os => if fs_allowed(path, FS_READ) { crate::read_file_raw(path) } else { None },
        Route::Vfs(p, rest) => p.read(&rest),
        Route::Bad => None,
    }
}

/// Write an exclusively created sibling, flush it, then replace path (a mount: its own way of
/// replacing a file; missing folders are created). [plat_file_write]
pub fn write_file(path: &str, data: &[u8]) -> bool {
    match vfs::route(path) {
        Route::Os => fs_allowed(path, FS_WRITE) && crate::write_file_raw(path, data),
        Route::Vfs(p, rest) => !rest.is_empty() && p.write(&rest, data),
        Route::Bad => false,
    }
}

// ---------------------------------------------------------------- streaming files

/// an open file (closed when dropped): an OS file, or a stream of a vfs mount
pub struct File(Inner);

enum Inner {
    Os(OsFile),
    Vfs(Box<dyn Stream>),
}

fn mode_ok(mode: u32) -> bool {
    matches!(mode, FO_READ | FO_WRITE | FO_RW)
}

// a checked OS file
fn os_open(path: &str, mode: u32) -> Option<File> {
    if mode_ok(mode) && fs_allowed(path, if mode == FO_READ { FS_READ } else { FS_WRITE }) {
        OsFile::open_raw(path, mode).map(|f| File(Inner::Os(f)))
    } else {
        None
    }
}

/// (vfs) File::open on a provider: its own stream, its OS file, else the whole file
pub(crate) fn open_in(p: &Arc<dyn Provider>, path: &str, mode: u32) -> Option<File> {
    if !mode_ok(mode) {
        return None;
    }
    match p.open(path, mode) {
        Opened::File(s) => Some(File(Inner::Vfs(s))),
        Opened::Refused => None,
        Opened::Generic => match p.os_path(path) {
            Some(o) => os_open(&o, mode),
            None => whole(p, path, mode),
        },
    }
}

// a provider without streams: the file read at the start, written back by sync and drop
fn whole(p: &Arc<dyn Provider>, path: &str, mode: u32) -> Option<File> {
    let data = match (mode, p.kind(path)) {
        (_, 2) => return None,
        (FO_READ, 1) | (FO_RW, 1) => p.read(path)?,
        (FO_READ, _) => return None,
        _ => Vec::new(),
    };
    // FO_WRITE truncates, FO_RW creates; an existing file is written back once, which also
    // tells a read-only provider
    if mode != FO_READ && (path.is_empty() || !p.write(path, &data)) {
        return None;
    }
    let back = (mode != FO_READ).then(|| (p.clone(), String::from(path)));
    Some(File(Inner::Vfs(Box::new(Whole { writable: back.is_some(), back, st: SpinLock::new((data, false)) }))))
}

/// a file kept in memory: written back to back (provider, path) when it changed; without
/// back read-only, or an anonymous file (open_temp on a mount without OS files)
struct Whole {
    back: Option<(Arc<dyn Provider>, String)>,
    writable: bool,
    // the bytes, changed since the last write-back
    st: SpinLock<(Vec<u8>, bool)>,
}

impl Whole {
    fn anon() -> Whole {
        Whole { back: None, writable: true, st: SpinLock::new((Vec::new(), false)) }
    }
}

impl Stream for Whole {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        let g = self.st.lock();
        let off = usize::try_from(off).unwrap_or(usize::MAX).min(g.0.len());
        let n = (g.0.len() - off).min(buf.len());
        buf[..n].copy_from_slice(&g.0[off..off + n]);
        Some(n)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let mut g = self.st.lock();
        let n = vfs::write_vec(&mut g.0, off, buf)?;
        g.1 = true;
        Some(n)
    }

    fn size(&self) -> Option<u64> {
        Some(self.st.lock().0.len() as u64)
    }

    fn set_len(&self, len: u64) -> bool {
        if !self.writable {
            return false;
        }
        let mut g = self.st.lock();
        g.1 = true;
        vfs::set_len_vec(&mut g.0, len)
    }

    fn sync(&self) -> bool {
        let Some((p, path)) = &self.back else { return true };
        let mut g = self.st.lock();
        if !g.1 {
            return true;
        }
        let ok = p.write(path, &g.0);
        g.1 &= !ok;
        ok
    }
}

impl Drop for Whole {
    fn drop(&mut self) {
        self.sync();
    }
}

impl File {
    /// FO_READ (existing, read only), FO_WRITE (create / truncate), FO_RW (open or create);
    /// on every mount (vfs.rs)
    pub fn open(path: &str, mode: u32) -> Option<File> {
        match vfs::route(path) {
            Route::Os => os_open(path, mode),
            Route::Vfs(p, rest) => open_in(&p, &rest, mode),
            Route::Bad => None,
        }
    }

    /// Create a named file exclusively; never open an existing path (a mount without OS
    /// files: exclusive inside this process).
    #[cfg(any(windows, target_os = "linux"))]
    pub fn create_new(path: &str) -> Option<File> {
        let new_os = |o: &str| if fs_allowed(o, FS_WRITE) { OsFile::create_new_raw(o).map(|f| File(Inner::Os(f))) } else { None };
        match vfs::route(path) {
            Route::Os => new_os(path),
            Route::Vfs(p, rest) => match p.os_path(&rest) {
                Some(o) => new_os(&o),
                None => {
                    let _g = NEW_LOCK.lock();
                    if rest.is_empty() || p.kind(&rest) != 0 { None } else { open_in(&p, &rest, FO_WRITE) }
                }
            },
            Route::Bad => None,
        }
    }

    /// a new file without a lasting name in dir (None = temp_dir) that disappears when it is
    /// closed (also after a crash); a mount without OS files: a file in memory  [plat_file_open_temp]
    pub fn open_temp(dir: Option<&str>) -> Option<File> {
        let temp_os = |d: &str| if fs_allowed(d, FS_WRITE) { OsFile::open_temp_raw(Some(d)).map(|f| File(Inner::Os(f))) } else { None };
        let Some(d) = dir else { return OsFile::open_temp_raw(None).map(|f| File(Inner::Os(f))) };
        match vfs::route(d) {
            Route::Os => temp_os(d),
            Route::Vfs(p, rest) => match p.os_path(&rest) {
                Some(o) => temp_os(&o),
                None => (p.kind(&rest) == 2).then(|| File(Inner::Vfs(Box::new(Whole::anon())))),
            },
            Route::Bad => None,
        }
    }

    /// bytes read at off (fewer at the end of the file), None on an error before any byte
    pub fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        match &self.0 {
            Inner::Os(f) => f.read_at(off, buf),
            Inner::Vfs(s) => s.read_at(off, buf),
        }
    }

    /// bytes written at off (the file grows as needed), None on an error before any byte
    pub fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        match &self.0 {
            Inner::Os(f) => f.write_at(off, buf),
            Inner::Vfs(s) => s.write_at(off, buf),
        }
    }

    pub fn size(&self) -> Option<u64> {
        match &self.0 {
            Inner::Os(f) => f.size(),
            Inner::Vfs(s) => s.size(),
        }
    }

    /// write cached data to the disk (a mount: where its readers find it)
    pub fn sync(&self) -> bool {
        match &self.0 {
            Inner::Os(f) => f.sync(),
            Inner::Vfs(s) => s.sync(),
        }
    }

    /// cut or extend the file to `len` bytes
    pub fn set_len(&self, len: u64) -> bool {
        match &self.0 {
            #[cfg(any(windows, target_os = "linux", target_os = "android"))]
            Inner::Os(f) => f.set_len(len),
            #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
            Inner::Os(_) => false,
            Inner::Vfs(s) => s.set_len(len),
        }
    }
}

impl Stream for File {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        File::read_at(self, off, buf)
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        File::write_at(self, off, buf)
    }
    fn size(&self) -> Option<u64> {
        File::size(self)
    }
    fn set_len(&self, len: u64) -> bool {
        File::set_len(self, len)
    }
    fn sync(&self) -> bool {
        File::sync(self)
    }
}

// ---------------------------------------------------------------- atomic streaming writes

/// The streaming form of write_file: the bytes go into a sibling of the target, commit puts
/// it in place in one step (readers see the old file or all of the new one); dropped without
/// commit, the sibling is removed and the target is unchanged.  Checked and routed like
/// write_file when it is created.  OS paths: an exclusive "NAME.tmp-PID-SEQ-TIME" beside the
/// file, fsync and rename (Linux keeps the old file's access bits; the browser: in memory until
/// commit).  Mounts: a "NAME.tmp-.." file of the mount, renamed by the provider (one without
/// rename gets the whole file written); a provider whose rename cannot replace a file
/// (`atomic_rename() == false`, content://) gets the bytes written in place at commit.
pub struct AtomicFile(Option<AtomicInner>);

enum AtomicInner {
    Os(crate::OsAtomic),
    Vfs { p: Arc<dyn Provider>, tmp: String, dst: String, f: Option<File> },
    // a provider without rename over a file (content://): the bytes, written at commit
    InPlace { p: Arc<dyn Provider>, dst: String, buf: SpinLock<Vec<u8>> },
}

static ATOMIC_SEQ: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

impl AtomicFile {
    /// a writer of path (None: refused, a folder, an invalid name, or the sibling could not be
    /// created)
    pub fn create(path: &str) -> Option<AtomicFile> {
        match vfs::route(path) {
            Route::Os => {
                if !fs_allowed(path, FS_WRITE) {
                    return None;
                }
                crate::OsAtomic::begin(path).map(|a| AtomicFile(Some(AtomicInner::Os(a))))
            }
            Route::Vfs(p, rest) => {
                if rest.is_empty() || p.kind(&rest) == 2 {
                    return None;
                }
                if !p.atomic_rename() {
                    return Some(AtomicFile(Some(AtomicInner::InPlace { p, dst: rest, buf: SpinLock::new(Vec::new()) })));
                }
                if let Some((dir, _)) = rest.rsplit_once('/') {
                    p.mkdir(dir);
                }
                for _ in 0..16 {
                    let seq = ATOMIC_SEQ.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    let tmp = alloc::format!("{rest}.tmp-{seq:x}-{:x}", crate::time_us() as u32);
                    if p.kind(&tmp) != 0 {
                        continue;
                    }
                    let f = open_in(&p, &tmp, FO_WRITE)?;
                    return Some(AtomicFile(Some(AtomicInner::Vfs { p, tmp, dst: rest, f: Some(f) })));
                }
                None
            }
            Route::Bad => None,
        }
    }

    /// bytes written at off (the file grows as needed), None on an error
    pub fn write_at(&self, off: u64, data: &[u8]) -> Option<usize> {
        match self.0.as_ref()? {
            AtomicInner::Os(a) => a.write_at(off, data),
            AtomicInner::Vfs { f, .. } => f.as_ref()?.write_at(off, data),
            AtomicInner::InPlace { buf, .. } => vfs::write_vec(&mut buf.lock(), off, data),
        }
    }

    /// put the written file in place -> false: the target is unchanged, the sibling gone
    pub fn commit(mut self) -> bool {
        match self.0.take() {
            None => false,
            Some(AtomicInner::Os(a)) => a.commit(),
            Some(AtomicInner::Vfs { p, tmp, dst, f }) => {
                let ok = f.is_some_and(|f| f.sync());
                // a provider without rename: the whole file under the new name
                let ok = ok && (p.rename(&tmp, &dst) || p.read(&tmp).is_some_and(|d| p.write(&dst, &d)));
                if p.kind(&tmp) == 1 {
                    p.delete(&tmp);
                }
                ok
            }
            Some(AtomicInner::InPlace { p, dst, buf }) => p.write(&dst, &buf.lock()),
        }
    }
}

impl Drop for AtomicFile {
    fn drop(&mut self) {
        if let Some(AtomicInner::Vfs { p, tmp, f, .. }) = &mut self.0 {
            drop(f.take());
            p.delete(tmp); // not committed: the sibling goes (an OsAtomic removes its own)
        }
    }
}

// create_new on mounts without OS files: one check-and-create at a time
#[cfg(any(windows, target_os = "linux"))]
static NEW_LOCK: SpinLock<()> = SpinLock::new(());

// the FileLocks of mounts without OS files: provider (address) and path
static VFS_LOCKS: SpinLock<Vec<(usize, String)>> = SpinLock::new(Vec::new());

/// Nonblocking exclusive sidecar lock; drop releases it.  All cooperating writers must lock
/// the same path before checking and replacing data.  OS files: across processes (Windows: an
/// exclusive handle, Linux: flock); a mount without OS files: inside this process.
pub struct FileLock(LockInner);

enum LockInner {
    Os { _lock: OsFileLock },
    Vfs { p: Arc<dyn Provider>, path: String },
}

impl FileLock {
    /// Open or create without truncating. None means busy, invalid path, refused or I/O failure.
    pub fn acquire(path: &str) -> Option<Self> {
        let os = |o: &str| if fs_allowed(o, FS_WRITE) { OsFileLock::acquire_raw(o).map(|l| FileLock(LockInner::Os { _lock: l })) } else { None };
        let (p, rest) = match vfs::route(path) {
            Route::Os => return os(path),
            Route::Vfs(p, rest) => match p.os_path(&rest) {
                Some(o) => return os(&o),
                None => (p, rest),
            },
            Route::Bad => return None,
        };
        let key = Arc::as_ptr(&p) as *const () as usize;
        let mut g = VFS_LOCKS.lock();
        if rest.is_empty() || g.iter().any(|(k, n)| *k == key && *n == rest) {
            return None;
        }
        match p.kind(&rest) {
            1 => {}
            0 if p.write(&rest, &[]) => {}
            _ => return None,
        }
        g.push((key, rest.clone()));
        Some(FileLock(LockInner::Vfs { p, path: rest }))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if let LockInner::Vfs { p, path } = &self.0 {
            let key = Arc::as_ptr(p) as *const () as usize;
            VFS_LOCKS.lock().retain(|(k, n)| !(*k == key && n == path));
        }
    }
}
