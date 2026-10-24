// Worker-local virtual files. File codecs use their existing byte and streaming APIs;
// durable storage is a separate asynchronous request confirmed by the browser host.
// Files below /handles/ are the user's own files: the host keeps a FileSystemFileHandle for
// each (web/handles.js) and every change is written back to it (web.rs write_back: write_file,
// AtomicFile's commit, rename onto it, a File stream's sync and drop); imports never are.

use crate::{FO_READ, FO_RW, FO_WRITE};
use alloc::{format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use rt::sync::SpinLock;

const FILE_MAX: usize = 256 << 20;
struct Data {
    bytes: Vec<u8>,
    revision: u64,
    persisted: u64,
}
struct Entry {
    path: String,
    data: Arc<SpinLock<Data>>,
}
struct Files {
    entries: Vec<Entry>,
    directories: Vec<String>,
    locks: Vec<String>,
}
static FILES: SpinLock<Files> = SpinLock::new(Files {
    entries: Vec::new(),
    directories: Vec::new(),
    locks: Vec::new(),
});
static TEMP: AtomicU32 = AtomicU32::new(1);
static REVISION: SpinLock<u64> = SpinLock::new(0);

// Identifiers never repeat, including after delete/recreate at the same path. A delayed
// browser completion can therefore acknowledge only the bytes that were requested.
fn next_revision() -> Option<u64> {
    let mut current = REVISION.lock();
    let next = current.checked_add(1)?;
    *current = next;
    Some(next)
}

fn path(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 32768 || value.contains('\0') {
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
    matches!(path, "/" | "/tmp" | "/config" | "/data" | "/cache" | "/documents" | "/imports")
}

/// Current browser virtual directory; it is unrelated to the host operating system.
pub fn cwd() -> String {
    String::from("/")
}
/// Configuration namespace in virtual storage.
pub fn config_dir() -> Option<String> {
    Some(String::from("/config"))
}
/// Application data namespace in virtual storage.
pub fn data_dir() -> Option<String> {
    Some(String::from("/data"))
}
/// Cache namespace in virtual storage.
pub fn cache_dir() -> Option<String> {
    Some(String::from("/cache"))
}
/// Temporary virtual namespace.
pub fn temp_dir() -> Option<String> {
    Some(String::from("/tmp"))
}
/// Home namespace for user documents imported or created in this app instance.
pub fn home() -> Option<String> {
    Some(String::from("/documents"))
}
/// Browser virtual roots; no native drives are exposed.
pub fn drives() -> Vec<String> {
    alloc::vec![String::from("/")]
}
/// Stable filesystem device/inode identity is not available in browser storage.
pub fn dev_ino(_path: &str) -> Option<(u64, u64)> {
    None
}
/// Readable virtual file, directory, or missing path; invalid names return None.
pub(crate) fn file_exists_checked_raw(name: &str) -> Option<u32> {
    let name = path(name)?;
    let fs = FILES.lock();
    Some(if fs.entries.iter().any(|e| e.path == name) {
        1
    } else if builtin(&name) || fs.directories.contains(&name) {
        2
    } else {
        0
    })
}
/// Existing virtual file (1), virtual directory (2), or absent/invalid name (0).
pub(crate) fn file_exists_raw(name: &str) -> u32 {
    file_exists_checked_raw(name).unwrap_or(0)
}
/// Create virtual parent directories; a colliding file causes failure.
pub(crate) fn mkdir_raw(name: &str) -> bool {
    let Some(name) = path(name) else {
        return false;
    };
    let mut fs = FILES.lock();
    let mut prefix = String::new();
    for part in name.split('/').filter(|p| !p.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        if fs.entries.iter().any(|e| e.path == prefix) {
            return false;
        }
        if !builtin(&prefix) && !fs.directories.contains(&prefix) {
            fs.directories.push(prefix.clone());
        }
    }
    true
}
/// Delete one virtual file or an empty non-built-in virtual directory.
pub(crate) fn delete_raw(name: &str) -> bool {
    let Some(name) = path(name) else {
        return false;
    };
    let mut fs = FILES.lock();
    if let Some(i) = fs.entries.iter().position(|e| e.path == name) {
        fs.entries.remove(i);
        return true;
    }
    let prefix = format!("{name}/");
    if builtin(&name)
        || fs.entries.iter().any(|e| e.path.starts_with(&prefix))
        || fs.directories.iter().any(|e| e.starts_with(&prefix))
    {
        return false;
    }
    if let Some(i) = fs.directories.iter().position(|e| e == &name) {
        fs.directories.remove(i);
        true
    } else {
        false
    }
}
/// Rename/replace a virtual file atomically inside this instance.
pub(crate) fn rename_raw(old: &str, new: &str) -> bool {
    let (Some(old), Some(new)) = (path(old), path(new)) else {
        return false;
    };
    if old == new {
        return file_exists_raw(&old) == 1;
    }
    let mut fs = FILES.lock();
    if builtin(&new) || fs.directories.contains(&new) {
        return false;
    }
    let Some(i) = fs.entries.iter().position(|e| e.path == old) else {
        return false;
    };
    let data = fs.entries.remove(i).data;
    fs.entries.retain(|e| e.path != new);
    data.lock().persisted = 0;
    let back = bound(&new).then(|| new.clone());
    fs.entries.push(Entry { path: new, data });
    drop(fs);
    if let Some(p) = back {
        super::write_back(&p);
    }
    true
}
/// Enumerate immediate children from a stable snapshot, without holding a lock in callbacks.
pub(crate) fn dir_list_raw(name: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    let Some(name) = path(name) else {
        return false;
    };
    if file_exists_raw(&name) != 2 {
        return false;
    }
    let prefix = if name == "/" {
        String::from("/")
    } else {
        format!("{name}/")
    };
    let entries = {
        let fs = FILES.lock();
        let mut output = Vec::new();
        for directory in ["/tmp", "/config", "/data", "/cache", "/documents", "/imports"]
            .into_iter()
            .chain(fs.directories.iter().map(|s| s.as_str()))
        {
            if let Some(tail) = directory
                .strip_prefix(&prefix)
                .filter(|s| !s.is_empty() && !s.contains('/'))
            {
                output.push((String::from(tail), true, 0));
            }
        }
        for e in &fs.entries {
            if let Some(tail) = e
                .path
                .strip_prefix(&prefix)
                .filter(|s| !s.is_empty() && !s.contains('/'))
            {
                output.push((String::from(tail), false, e.data.lock().bytes.len() as u32));
            }
        }
        output
    };
    for (name, directory, len) in entries {
        f(&name, directory, len);
    }
    true
}
/// Read a copy of a virtual file's bytes.
pub(crate) fn read_file_raw(name: &str) -> Option<Vec<u8>> {
    let file = OsFile::open_raw(name, FO_READ)?;
    Some(file.data.lock().bytes.clone())
}
/// A file of the user's own, bound to a handle of the host (a normalised path).
pub(crate) fn bound(path: &str) -> bool {
    path.starts_with("/handles/")
}
/// Atomically replace a worker-local file. This does not claim persistent storage; a file of
/// the user's own (bound) is written back to it.
pub(crate) fn write_file_raw(name: &str, bytes: &[u8]) -> bool {
    let ok = store(name, bytes);
    if ok {
        if let Some(p) = path(name).filter(|p| bound(p)) {
            super::write_back(&p);
        }
    }
    ok
}
/// Replace a worker-local file without writing anything back (host imports).
pub(crate) fn store(name: &str, bytes: &[u8]) -> bool {
    let Some(name) = path(name) else {
        return false;
    };
    if bytes.len() > FILE_MAX || file_exists_raw(&name) == 2 {
        return false;
    }
    let mut copied = Vec::new();
    if copied.try_reserve_exact(bytes.len()).is_err() {
        return false;
    }
    copied.extend_from_slice(bytes);
    let Some(revision) = next_revision() else {
        return false;
    };
    let mut fs = FILES.lock();
    fs.entries.retain(|e| e.path != name);
    fs.entries.push(Entry {
        path: name,
        data: Arc::new(SpinLock::new(Data {
            bytes: copied,
            revision,
            persisted: 0,
        })),
    });
    true
}

/// A file collected in memory and put in place in one step by commit (write_file_raw), the
/// streaming writers of guarded.rs / aio.rs.  Dropped without commit, nothing changes.
pub(crate) struct OsAtomic {
    name: String,
    buf: SpinLock<Vec<u8>>,
}
impl OsAtomic {
    /// A writer of name; a folder there refuses.
    pub(crate) fn begin(name: &str) -> Option<Self> {
        let p = path(name)?;
        if file_exists_raw(&p) == 2 {
            return None;
        }
        Some(Self { name: p, buf: SpinLock::new(Vec::new()) })
    }
    /// Bytes written at an offset of the pending contents (at most FILE_MAX in all).
    pub(crate) fn write_at(&self, offset: u64, bytes: &[u8]) -> Option<usize> {
        if offset.checked_add(bytes.len() as u64)? > FILE_MAX as u64 {
            return None;
        }
        crate::vfs::write_vec(&mut self.buf.lock(), offset, bytes)
    }
    /// Replace the file with the written bytes.
    pub(crate) fn commit(self) -> bool {
        let bytes = core::mem::take(&mut *self.buf.lock());
        write_file_raw(&self.name, &bytes)
    }
}

/// An open virtual file handle; rename or replacement does not invalidate an existing handle.
pub(crate) struct OsFile {
    data: Arc<SpinLock<Data>>,
    writable: bool,
    temporary: Option<String>,
    // a writable handle on a file of the user's own: its path (written back by sync and drop)
    back: Option<String>,
}
impl OsFile {
    /// Open a virtual file with the standard FO_READ/FO_WRITE/FO_RW contract.
    pub(crate) fn open_raw(name: &str, mode: u32) -> Option<Self> {
        let name = path(name)?;
        if mode > FO_RW || file_exists_raw(&name) == 2 {
            return None;
        }
        if mode == FO_WRITE || (mode == FO_RW && file_exists_raw(&name) == 0) {
            // (a file of the user's: written back once there is something, by sync / drop)
            if !store(&name, &[]) {
                return None;
            }
        }
        let fs = FILES.lock();
        Some(Self {
            data: fs.entries.iter().find(|e| e.path == name)?.data.clone(),
            writable: mode != FO_READ,
            temporary: None,
            back: (mode != FO_READ && bound(&name)).then(|| name.clone()),
        })
    }
    /// Create a virtual scratch file that is removed when this handle is dropped.
    pub(crate) fn open_temp_raw(directory: Option<&str>) -> Option<Self> {
        let name = format!(
            "{}/jfgui-{}.tmp",
            directory.unwrap_or("/tmp"),
            TEMP.fetch_add(1, Ordering::Relaxed)
        );
        let mut file = Self::open_raw(&name, FO_WRITE)?;
        file.temporary = Some(name);
        Some(file)
    }
    /// Read available bytes at an offset, returning zero at EOF.
    pub fn read_at(&self, offset: u64, out: &mut [u8]) -> Option<usize> {
        let offset = usize::try_from(offset).ok()?;
        let data = self.data.lock();
        let len = data.bytes.len().saturating_sub(offset).min(out.len());
        if len != 0 {
            out[..len].copy_from_slice(&data.bytes[offset..offset + len]);
        }
        Some(len)
    }
    /// Write bytes at an offset, growing with zeroes; reject overflow or read-only handles.
    pub fn write_at(&self, offset: u64, bytes: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let offset = usize::try_from(offset).ok()?;
        let end = offset
            .checked_add(bytes.len())
            .filter(|&end| end <= FILE_MAX)?;
        let revision = next_revision()?;
        let mut data = self.data.lock();
        if end > data.bytes.len() {
            let extra = end - data.bytes.len();
            data.bytes.try_reserve(extra).ok()?; // grows by doubling: appends stay linear
            data.bytes.resize(end, 0);
        }
        data.bytes[offset..end].copy_from_slice(bytes);
        data.revision = revision;
        Some(bytes.len())
    }
    /// Current virtual length.
    pub fn size(&self) -> Option<u64> {
        Some(self.data.lock().bytes.len() as u64)
    }
    /// True only when the current revision has a successful persistence completion; a file of
    /// the user's own is written back first (false until the host confirmed it).
    pub fn sync(&self) -> bool {
        let done = {
            let data = self.data.lock();
            data.persisted != 0 && data.persisted == data.revision
        };
        if !done {
            if let Some(p) = &self.back {
                super::write_back(p);
            }
        }
        done
    }
}
impl Drop for OsFile {
    fn drop(&mut self) {
        if let Some(name) = &self.temporary {
            let _ = delete_raw(name);
        }
        if self.back.is_some() {
            self.sync();
        }
    }
}

/// Nonblocking exclusive lock inside one browser instance's virtual namespace.
pub(crate) struct OsFileLock {
    path: String,
}
impl OsFileLock {
    /// Acquire a lock until Drop; this is not an inter-tab or operating-system lock.
    pub(crate) fn acquire_raw(name: &str) -> Option<Self> {
        let name = path(name)?;
        let mut fs = FILES.lock();
        if fs.locks.contains(&name) {
            return None;
        }
        fs.locks.push(name.clone());
        Some(Self { path: name })
    }
}
impl Drop for OsFileLock {
    fn drop(&mut self) {
        FILES.lock().locks.retain(|p| p != &self.path);
    }
}

/// The normalised path of a name (it need not exist).
pub(super) fn normal(name: &str) -> Option<String> {
    path(name)
}
/// The normalised path and the current revision of a file.
pub(super) fn revision(name: &str) -> Option<(String, u64)> {
    let name = path(name)?;
    let fs = FILES.lock();
    let revision = fs.entries.iter().find(|e| e.path == name)?.data.lock().revision;
    Some((name, revision))
}
pub(super) fn snapshot(name: &str) -> Option<(String, u64, Vec<u8>)> {
    let name = path(name)?;
    let fs = FILES.lock();
    let data = fs.entries.iter().find(|e| e.path == name)?.data.lock();
    Some((name, data.revision, data.bytes.clone()))
}
pub(super) fn persisted(name: &str, revision: u64) {
    if let Some(entry) = FILES.lock().entries.iter().find(|e| e.path == name) {
        let mut data = entry.data.lock();
        if data.revision == revision {
            data.persisted = revision;
        }
    }
}
