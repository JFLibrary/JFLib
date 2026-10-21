// vfs -- one file namespace on every platform: "scheme://path" reaches a mounted provider,
// every other name is a path of the OS (or of the browser's virtual files)
//
//   mount(scheme, provider) / unmount(scheme) -> bool / mounted(scheme) -> bool
//   provider(scheme) -> Option<Arc<dyn Provider>>   a mount's provider (to build an Overlay or
//                                a Sub of app://, for example)
//   route(name) -> Route         Os (no scheme), Vfs(provider, path) or Bad (a scheme name
//                                with ".." or an invalid scheme); "config:/" and "config:" are
//                                the root like "config://"
//   os_path(name) -> Option<String>   the OS path behind a name of a folder mount (for APIs
//                                that need a real file); an OS path comes back unchanged
//   stat(name) -> Option<Stat>   kind, size, times of a name with a scheme (Provider::stat)
//   display_name(name) -> String the name to show the user (content://: the document's
//                                display name; else the last segment)
//   app_id() / set_app_id(id)    the name of the app's own folders (build_rs.py: the app's
//                                folder name; else the exe's name)
//   app_dir(APP_CONFIG | APP_DATA | APP_CACHE | APP_LOG) -> Option<String>   the app's own
//                                folders: Windows %APPDATA%/<id>, %LOCALAPPDATA%/<id>,
//                                %LOCALAPPDATA%/<id>/Cache, %LOCALAPPDATA%/<id>/Logs; Linux
//                                the XDG config / data / cache / state homes + /<id>; browser
//                                /config, /data, /cache + /<id>, /data/<id>/Logs
//   Built in, mounted on first use: mem:// (one MemFs per process), config://, data://,
//   cache://, logs:// (DirFs on app_dir), app:// (the app's bundle; ui::main! / rt::main! install it,
//   else empty), and the user's folders desktop://, documents://, downloads://, pictures://,
//   music://, videos:// (DirFs on user_dir, dirs.rs; only what the permissions allow); on
//   Android content:// (the documents the user chose in the system's picker: content.rs,
//   ContentFs over the ContentResolver)
//   Providers: MemFs, DirFs::new(root), Overlay::new(layers) (with whiteouts),
//   Sub::new(provider, folder), Bundle::new(bytes) (JFA1, tools/assetpack.py); archive::ZipFs
//   (lib/archive).  Provider: kind, read, list (+ write, mkdir, delete, rename, os_path, stat,
//   set_times, open -> Opened / Stream)
//   pack_bundle(files) -> Vec<u8>    a JFA1 bundle (tests, tools; build_rs.py packs with
//                                tools/assetpack.py)
//
// The public file functions (guarded.rs) route names with a scheme here: read_file,
// write_file, file_exists(_checked), mkdir, delete, rename (inside one mount), dir_list and
// trash (folder mounts); plat::fs (stat, read_dir, links, times ...: fs.rs) too.  File::open,
// create_new, open_temp and FileLock::acquire work on every mount: folder mounts open the OS
// file (os_path), MemFs and Bundle (and ZipFs) hand out streams (Provider::open: Opened), other
// providers get the whole file read and written back; a FileLock of a mount without OS files
// holds inside this process.  Paths inside a mount use '/', have no "." or empty segments and
// never "..".  Writes create missing folders.  DirFs uses the checked OS functions, so the
// app's permissions apply to it; the app's own folders are always allowed (policy.rs), the
// user's folders need rules ($DOCUMENTS ...).
//
// Overlay whiteouts: deleting or renaming what a lower layer has writes ".wh.NAME" into the top
// layer; a whited-out folder made again gets ".wh..wh..opq" (no lower entries below it).  The
// markers live in the top layer (a DirFs keeps them on disk) and are never listed.
//
// JFA1 bundle (little endian): "JFA1", u32 count, u32 names_len; count entries of 20 bytes
// sorted by name: u32 name_off, u16 name_len, u8 codec (0 stored, 1 LZ4 block), u8 0,
// u32 data_off (from the bundle start), u32 stored_len, u32 size; the names; the data.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use rt::sync::SpinLock;

/// app_dir: the app's settings folder (config://)
pub const APP_CONFIG: u32 = 0;
/// app_dir: the app's data folder (data://)
pub const APP_DATA: u32 = 1;
/// app_dir: the app's cache folder (cache://)
pub const APP_CACHE: u32 = 2;
/// app_dir: the app's log folder (logs://)
pub const APP_LOG: u32 = 3;

/// what [`Provider::stat`] reports
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    /// 1 file, 2 folder
    pub kind: u32,
    pub size: u64,
    /// nanoseconds since 1970; 0 = unknown (bundles)
    pub mtime_ns: i64,
    pub atime_ns: i64,
}

/// an open file of a provider without OS files (File::open on mem://, app://, zip mounts);
/// offsets as in plat::File
pub trait Stream: Send + Sync {
    /// bytes read at off (fewer at the end), None on an error
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize>;
    /// bytes written at off (the file grows); None for a read-only stream
    fn write_at(&self, _off: u64, _buf: &[u8]) -> Option<usize> {
        None
    }
    fn size(&self) -> Option<u64>;
    /// cut or extend to len bytes
    fn set_len(&self, _len: u64) -> bool {
        false
    }
    /// the data is where readers of the mount find it
    fn sync(&self) -> bool {
        true
    }
}

/// A file tree behind a scheme.  Paths are relative ("" is the root), '/' separated and
/// normalised by route().
pub trait Provider: Send + Sync {
    /// 0 missing, 1 file, 2 folder
    fn kind(&self, path: &str) -> u32;
    /// the whole file
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// f(name, is_dir, size) for every entry of a folder -> false when it is no folder
    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool;
    /// replace or create a file (missing folders are created)
    fn write(&self, _path: &str, _data: &[u8]) -> bool {
        false
    }
    /// create a folder and its parents (an existing folder is fine)
    fn mkdir(&self, _path: &str) -> bool {
        false
    }
    /// a file or an empty folder
    fn delete(&self, _path: &str) -> bool {
        false
    }
    /// a file to another name (replacing it)
    fn rename(&self, _old: &str, _new: &str) -> bool {
        false
    }
    /// the OS path of path (folder mounts only)
    fn os_path(&self, _path: &str) -> Option<String> {
        None
    }
    /// kind, size and times of path (None: missing); the default asks kind and the folder's
    /// listing, without times
    fn stat(&self, path: &str) -> Option<Stat> {
        match self.kind(path) {
            1 => {
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                let mut size = None;
                self.list(dir, &mut |n, d, s| {
                    if !d && n == name {
                        size = Some(s);
                    }
                });
                Some(Stat { kind: 1, size: size?, ..Stat::default() })
            }
            2 => Some(Stat { kind: 2, ..Stat::default() }),
            _ => None,
        }
    }
    /// set the access and modification times (ns since 1970) -> false where the provider
    /// keeps none
    fn set_times(&self, _path: &str, _atime_ns: i64, _mtime_ns: i64) -> bool {
        false
    }
    /// File::open on the provider: a stream on the file (mode FO_READ, FO_WRITE: create /
    /// truncate, FO_RW: open or create).  The default, Generic, lets File::open use os_path,
    /// else read and write the whole file (written back on sync and when the File is dropped)
    fn open(&self, _path: &str, _mode: u32) -> Opened {
        Opened::Generic
    }
    /// the name to show the user for path (None: its last segment; content:// documents:
    /// their display name)
    fn display_name(&self, _path: &str) -> Option<String> {
        None
    }
    /// whether a sibling renamed over a file replaces it in one step (AtomicFile commits so);
    /// false (content://): AtomicFile collects the bytes and writes the file in place
    fn atomic_rename(&self) -> bool {
        true
    }
}

/// what [`Provider::open`] answers
pub enum Opened {
    /// the provider's own stream
    File(Box<dyn Stream>),
    /// missing, read-only or refused: File::open fails
    Refused,
    /// no streams of its own: File::open uses os_path, else read / write of the whole file
    Generic,
}

impl Opened {
    /// a File (OS file or stream) as the answer, None as Refused
    pub fn of(f: Option<crate::File>) -> Opened {
        match f {
            Some(f) => Opened::File(Box::new(f)),
            None => Opened::Refused,
        }
    }
}

/// where a name leads
pub enum Route {
    /// no scheme: a path of the OS
    Os,
    /// a mounted provider and the path inside it
    Vfs(Arc<dyn Provider>, String),
    /// a scheme name that cannot be used (".." or no such mount)
    Bad,
}

static MOUNTS: SpinLock<Vec<(String, Arc<dyn Provider>)>> = SpinLock::new(Vec::new());
static APP_ID: SpinLock<Option<String>> = SpinLock::new(None);

/// mount provider under scheme (lower-case letters, digits, '+', '-', '.'; at least two
/// characters); replaces an earlier mount -> false for an invalid scheme
pub fn mount(scheme: &str, provider: Arc<dyn Provider>) -> bool {
    if !scheme_ok(scheme) {
        return false;
    }
    let mut m = MOUNTS.lock();
    m.retain(|(s, _)| s != scheme);
    m.push((String::from(scheme), provider));
    true
}

/// remove a mount -> whether there was one
pub fn unmount(scheme: &str) -> bool {
    let mut m = MOUNTS.lock();
    let n = m.len();
    m.retain(|(s, _)| s != scheme);
    m.len() != n
}

/// whether scheme is mounted (the built-in schemes count once they were used)
pub fn mounted(scheme: &str) -> bool {
    MOUNTS.lock().iter().any(|(s, _)| s == scheme)
}

fn scheme_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+-.".contains(c))
}

// "scheme:" + "", "/" or "//" + rest -> (scheme, rest); None for an OS path
fn split(name: &str) -> Option<(&str, &str)> {
    let k = name.find(':')?;
    let (scheme, rest) = (&name[..k], &name[k + 1..]);
    if !scheme_ok(scheme) || !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    Some((scheme, rest.trim_start_matches('/')))
}

/// the normalised path inside a mount ('\' -> '/', no "." / empty segments) or None (a "..")
pub fn clean(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return None,
            s if s.contains('\0') => return None,
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

/// where name leads (built-in schemes are mounted on first use)
pub fn route(name: &str) -> Route {
    let Some((scheme, rest)) = split(name) else { return Route::Os };
    let Some(path) = clean(rest) else { return Route::Bad };
    if let Some(p) = MOUNTS.lock().iter().find(|(s, _)| s == scheme).map(|(_, p)| p.clone()) {
        return Route::Vfs(p, path);
    }
    let dir = |d: Option<String>| d.map(|d| Arc::new(DirFs::new(&d)) as Arc<dyn Provider>);
    let builtin: Option<Arc<dyn Provider>> = match scheme {
        "mem" => Some(Arc::new(MemFs::new())),
        "app" => Some(Arc::new(Bundle::empty())),
        "config" => dir(app_dir(APP_CONFIG)),
        "data" => dir(app_dir(APP_DATA)),
        "cache" => dir(app_dir(APP_CACHE)),
        "logs" => dir(app_dir(APP_LOG)),
        // the documents of other apps through the ContentResolver (content.rs)
        #[cfg(target_os = "android")]
        "content" => Some(crate::web::content_fs()),
        _ => (0..crate::DIR_COUNT).find(|&k| crate::user_dir_scheme(k) == Some(scheme)).and_then(|k| dir(crate::user_dir(k))),
    };
    let Some(b) = builtin else { return Route::Bad };
    let mut m = MOUNTS.lock();
    // another thread may have mounted it meanwhile
    let p = match m.iter().find(|(s, _)| s == scheme) {
        Some((_, p)) => p.clone(),
        None => {
            m.push((String::from(scheme), b.clone()));
            b
        }
    };
    Route::Vfs(p, path)
}

/// the provider mounted under scheme (built-in schemes are mounted on first use)
pub fn provider(scheme: &str) -> Option<Arc<dyn Provider>> {
    match route(&alloc::format!("{scheme}://")) {
        Route::Vfs(p, _) => Some(p),
        _ => None,
    }
}

/// the OS path behind name: a folder mount's file, or name itself when it has no scheme
pub fn os_path(name: &str) -> Option<String> {
    match route(name) {
        Route::Os => Some(String::from(name)),
        Route::Vfs(p, path) => p.os_path(&path),
        Route::Bad => None,
    }
}

/// kind, size and times of a name with a scheme (Provider::stat; content://: what the
/// document's provider reports) -> None for a missing one and for OS paths (plat::fs::stat)
pub fn stat(name: &str) -> Option<Stat> {
    match route(name) {
        Route::Vfs(p, path) => p.stat(&path),
        _ => None,
    }
}

/// the name to show the user for a file or folder: a content:// document's display name,
/// else the last segment of the name ("C:/a/b.txt" -> "b.txt", "config://x/y" -> "y")
pub fn display_name(name: &str) -> String {
    if let Route::Vfs(p, path) = route(name) {
        if let Some(n) = p.display_name(&path) {
            return n;
        }
    }
    // '\' separates only on Windows (on Linux it is a character of the name)
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let t = name.trim_end_matches(seps);
    let n = t.rsplit(seps).next().unwrap_or(t);
    String::from(if n.is_empty() || n.ends_with(':') { name } else { n })
}

// ---------------------------------------------------------------- the app's folders

/// set the name of the app's own folders (ui::main!, rt::main!: the app folder of build_rs.py); ignored
/// unless it is a plain file name (letters, digits, '.', '-', '_', not only dots)
pub fn set_app_id(id: &str) {
    let ok = !id.is_empty() && id.len() <= 64 && !id.bytes().all(|c| c == b'.')
        && id.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c));
    if ok {
        *APP_ID.lock() = Some(String::from(id));
    }
}

/// the name of the app's own folders: set_app_id, else the exe's name without extension
pub fn app_id() -> String {
    if let Some(id) = APP_ID.lock().clone() {
        return id;
    }
    exe_stem().unwrap_or_else(|| String::from("app"))
}

#[cfg(any(windows, target_os = "linux"))]
fn exe_stem() -> Option<String> {
    let p = crate::current_exe()?;
    let name = &p[p.rfind(['/', '\\']).map_or(0, |k| k + 1)..];
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    (!stem.is_empty()).then(|| String::from(stem))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn exe_stem() -> Option<String> {
    None
}

/// the app's settings (APP_CONFIG), data (APP_DATA), cache (APP_CACHE) or log (APP_LOG) folder
pub fn app_dir(kind: u32) -> Option<String> {
    let id = app_id();
    match kind {
        APP_CONFIG => crate::config_dir().map(|d| alloc::format!("{d}/{id}")),
        APP_DATA => crate::data_dir().map(|d| alloc::format!("{d}/{id}")),
        APP_CACHE if cfg!(windows) => crate::data_dir().map(|d| alloc::format!("{d}/{id}/Cache")),
        APP_CACHE => crate::cache_dir().map(|d| alloc::format!("{d}/{id}")),
        // Linux: a folder of its own below $XDG_STATE_HOME; elsewhere below the data folder
        APP_LOG if cfg!(target_os = "linux") => crate::log_dir().map(|d| alloc::format!("{d}/{id}")),
        APP_LOG => crate::log_dir().map(|d| alloc::format!("{d}/{id}/Logs")),
        _ => None,
    }
}

// ---------------------------------------------------------------- MemFs

/// files in memory (mem://, tests, generated data); open streams share the file's bytes like
/// OS files: writes are seen at once, write / delete replace or drop the name only
pub struct MemFs {
    t: SpinLock<MemTree>,
}

#[derive(Clone, Copy, Default)]
struct Times {
    m: i64,
    a: i64,
}

struct MemFile {
    bytes: Vec<u8>,
    t: Times,
}

type Shared = Arc<SpinLock<MemFile>>;

#[derive(Default)]
struct MemTree {
    files: BTreeMap<String, Shared>,
    dirs: BTreeMap<String, Times>,
}

fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |k| &path[..k])
}

fn child_of<'a>(entry: &'a str, dir: &str) -> Option<&'a str> {
    let rest = if dir.is_empty() { entry } else { entry.strip_prefix(dir)?.strip_prefix('/')? };
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

fn now() -> Times {
    let t = crate::unix_time_ns();
    Times { m: t, a: t }
}

impl MemFs {
    pub fn new() -> MemFs {
        MemFs { t: SpinLock::new(MemTree::default()) }
    }
}

impl Default for MemFs {
    fn default() -> MemFs {
        MemFs::new()
    }
}

impl MemTree {
    fn add_dirs(&mut self, mut d: &str) {
        while !d.is_empty() {
            if !self.dirs.contains_key(d) {
                self.dirs.insert(String::from(d), now());
            }
            d = parent(d);
        }
    }

    fn kind(&self, path: &str) -> u32 {
        if self.files.contains_key(path) {
            1
        } else if path.is_empty() || self.dirs.contains_key(path) {
            2
        } else {
            0
        }
    }

    // a new file may be created at path: no folder there, no file above it
    fn can_create(&self, path: &str) -> bool {
        let mut d = parent(path);
        while !d.is_empty() {
            if self.files.contains_key(d) {
                return false;
            }
            d = parent(d);
        }
        !path.is_empty() && !self.dirs.contains_key(path)
    }
}

impl Provider for MemFs {
    fn kind(&self, path: &str) -> u32 {
        self.t.lock().kind(path)
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let f = self.t.lock().files.get(path).cloned()?;
        let g = f.lock();
        Some(g.bytes.clone())
    }

    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        let entries: Vec<(String, bool, u64)> = {
            let t = self.t.lock();
            if t.kind(path) != 2 {
                return false;
            }
            let dirs = t.dirs.keys().filter_map(|d| child_of(d, path)).map(|n| (String::from(n), true, 0));
            let files = t.files.iter().filter_map(|(k, v)| child_of(k, path).map(|n| (String::from(n), false, v.lock().bytes.len() as u64)));
            dirs.chain(files).collect()
        };
        for (n, d, s) in entries {
            f(&n, d, s);
        }
        true
    }

    fn write(&self, path: &str, data: &[u8]) -> bool {
        let mut t = self.t.lock();
        if !t.can_create(path) {
            return false; // the root, a folder, or a file where a folder should be
        }
        t.add_dirs(parent(path));
        t.files.insert(String::from(path), Arc::new(SpinLock::new(MemFile { bytes: data.to_vec(), t: now() })));
        true
    }

    fn mkdir(&self, path: &str) -> bool {
        let mut t = self.t.lock();
        let mut d = path;
        while !d.is_empty() {
            if t.files.contains_key(d) {
                return false;
            }
            d = parent(d);
        }
        t.add_dirs(path);
        true
    }

    fn delete(&self, path: &str) -> bool {
        let mut t = self.t.lock();
        if t.files.remove(path).is_some() {
            return true;
        }
        let busy = t.files.keys().chain(t.dirs.keys()).any(|e| child_of(e, path).is_some());
        !path.is_empty() && !busy && t.dirs.remove(path).is_some()
    }

    fn rename(&self, old: &str, new: &str) -> bool {
        let mut t = self.t.lock();
        if old == new {
            return t.files.contains_key(old);
        }
        if !t.files.contains_key(old) || !t.can_create(new) {
            return false;
        }
        let Some(f) = t.files.remove(old) else { return false };
        t.add_dirs(parent(new));
        t.files.insert(String::from(new), f);
        true
    }

    fn stat(&self, path: &str) -> Option<Stat> {
        let t = self.t.lock();
        if let Some(f) = t.files.get(path) {
            let g = f.lock();
            return Some(Stat { kind: 1, size: g.bytes.len() as u64, mtime_ns: g.t.m, atime_ns: g.t.a });
        }
        if path.is_empty() {
            return Some(Stat { kind: 2, ..Stat::default() });
        }
        t.dirs.get(path).map(|d| Stat { kind: 2, size: 0, mtime_ns: d.m, atime_ns: d.a })
    }

    fn set_times(&self, path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        let mut t = self.t.lock();
        let times = Times { m: mtime_ns, a: atime_ns };
        if let Some(f) = t.files.get(path) {
            f.lock().t = times;
            return true;
        }
        match t.dirs.get_mut(path) {
            Some(d) => {
                *d = times;
                true
            }
            None => false,
        }
    }

    fn open(&self, path: &str, mode: u32) -> Opened {
        match self.stream(path, mode) {
            Some(s) => Opened::File(s),
            None => Opened::Refused,
        }
    }
}

impl MemFs {
    // an open file: FO_WRITE empties an existing one, FO_RW / FO_WRITE create a missing one
    fn stream(&self, path: &str, mode: u32) -> Option<Box<dyn Stream>> {
        if mode > crate::FO_RW {
            return None;
        }
        let mut t = self.t.lock();
        let file = match (t.files.get(path).cloned(), mode) {
            (Some(f), crate::FO_WRITE) => {
                let mut g = f.lock();
                g.bytes.clear();
                g.t = now();
                drop(g);
                f
            }
            (Some(f), _) => f,
            (None, crate::FO_READ) => return None,
            (None, _) => {
                if !t.can_create(path) {
                    return None;
                }
                t.add_dirs(parent(path));
                let f: Shared = Arc::new(SpinLock::new(MemFile { bytes: Vec::new(), t: now() }));
                t.files.insert(String::from(path), f.clone());
                f
            }
        };
        Some(Box::new(MemStream { f: file, writable: mode != crate::FO_READ }))
    }
}

/// an open MemFs file
struct MemStream {
    f: Shared,
    writable: bool,
}

// read from bytes at off -> the count
fn read_slice(bytes: &[u8], off: u64, buf: &mut [u8]) -> usize {
    let off = usize::try_from(off).unwrap_or(usize::MAX).min(bytes.len());
    let n = (bytes.len() - off).min(buf.len());
    buf[..n].copy_from_slice(&bytes[off..off + n]);
    n
}

// write into a growing buffer at off (zeros fill a gap) -> None when it cannot grow
pub(crate) fn write_vec(v: &mut Vec<u8>, off: u64, buf: &[u8]) -> Option<usize> {
    let off = usize::try_from(off).ok()?;
    let end = off.checked_add(buf.len())?;
    if end > v.len() {
        v.try_reserve(end - v.len()).ok()?;
        v.resize(end, 0);
    }
    v[off..end].copy_from_slice(buf);
    Some(buf.len())
}

// cut or extend a buffer to len
pub(crate) fn set_len_vec(v: &mut Vec<u8>, len: u64) -> bool {
    let Ok(len) = usize::try_from(len) else { return false };
    if len > v.len() && v.try_reserve(len - v.len()).is_err() {
        return false;
    }
    v.resize(len, 0);
    true
}

impl Stream for MemStream {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        Some(read_slice(&self.f.lock().bytes, off, buf))
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let mut g = self.f.lock();
        let n = write_vec(&mut g.bytes, off, buf)?;
        g.t.m = crate::unix_time_ns();
        Some(n)
    }

    fn size(&self) -> Option<u64> {
        Some(self.f.lock().bytes.len() as u64)
    }

    fn set_len(&self, len: u64) -> bool {
        if !self.writable {
            return false;
        }
        let mut g = self.f.lock();
        g.t.m = crate::unix_time_ns();
        set_len_vec(&mut g.bytes, len)
    }
}

// ---------------------------------------------------------------- DirFs

/// an OS folder as the root (config://, data://, cache://, mounts of the app); every call
/// goes through the checked OS functions, so the app's permissions apply
pub struct DirFs {
    root: String,
}

impl DirFs {
    /// root: an absolute OS folder (created on the first write)
    pub fn new(root: &str) -> DirFs {
        DirFs { root: String::from(root.trim_end_matches(['/', '\\'])) }
    }

    fn full(&self, path: &str) -> String {
        if path.is_empty() { self.root.clone() } else { alloc::format!("{}/{}", self.root, path) }
    }
}

impl Provider for DirFs {
    fn kind(&self, path: &str) -> u32 {
        crate::file_exists(&self.full(path))
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        crate::read_file(&self.full(path))
    }

    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        crate::dir_list(&self.full(path), &mut |n, d, s| f(n, d, s as u64))
    }

    fn write(&self, path: &str, data: &[u8]) -> bool {
        let full = self.full(path);
        crate::mkdir(&self.full(parent(path)));
        !path.is_empty() && crate::write_file(&full, data)
    }

    fn mkdir(&self, path: &str) -> bool {
        crate::mkdir(&self.full(path))
    }

    fn delete(&self, path: &str) -> bool {
        !path.is_empty() && crate::delete(&self.full(path))
    }

    fn rename(&self, old: &str, new: &str) -> bool {
        crate::mkdir(&self.full(parent(new)));
        !old.is_empty() && !new.is_empty() && crate::rename(&self.full(old), &self.full(new))
    }

    fn os_path(&self, path: &str) -> Option<String> {
        Some(self.full(path))
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn stat(&self, path: &str) -> Option<Stat> {
        let m = crate::fs::stat(&self.full(path))?;
        let kind = if m.is_dir() { 2 } else if m.is_file() { 1 } else { return None };
        Some(Stat { kind, size: if kind == 1 { m.size } else { 0 }, mtime_ns: m.mtime_ns, atime_ns: m.atime_ns })
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn set_times(&self, path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        crate::fs::set_times(&self.full(path), atime_ns, mtime_ns)
    }
}

// ---------------------------------------------------------------- Overlay, Sub

// the names an Overlay keeps in its top layer: ".wh.NAME" hides NAME of the lower layers,
// ".wh..wh..opq" in a folder hides everything below it in the lower layers (aufs' names)
const WH: &str = ".wh.";
const OPAQUE: &str = ".wh..wh..opq";

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { String::from(name) } else { alloc::format!("{dir}/{name}") }
}

// the whiteout of path: ".wh.NAME" next to it
fn whiteout(path: &str) -> String {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    join(dir, &alloc::format!("{WH}{name}"))
}

// a name the overlay reserves for its markers
fn is_marker(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|n| n.starts_with(WH))
}

// "a/b/c" -> "a", "a/b", "a/b/c"
fn prefixes(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(k, _)| &path[..k]).chain((!path.is_empty()).then_some(path))
}

/// several providers in one tree: a path comes from the first layer that has it, a folder
/// lists the entries of every layer, writes go to the first layer.  Deleting or renaming what a
/// lower layer has leaves a whiteout in the top layer (".wh.NAME"; a folder made again after
/// that gets ".wh..wh..opq" and shows no lower entries), so a DirFs on top keeps them across
/// runs; the markers are never listed, and names starting with ".wh." cannot be used.
pub struct Overlay {
    layers: Vec<Arc<dyn Provider>>,
}

impl Overlay {
    /// layers[0] is on top
    pub fn new(layers: Vec<Arc<dyn Provider>>) -> Overlay {
        Overlay { layers }
    }

    // whether the top layer hides path from the lower ones: a whiteout of path or of a folder
    // above it, an opaque folder or a file above it
    fn hidden(&self, path: &str) -> bool {
        let Some(top) = self.layers.first() else { return false };
        if self.layers.len() < 2 {
            return false;
        }
        top.kind(OPAQUE) == 1
            || prefixes(path).any(|q| top.kind(&whiteout(q)) == 1 || (q != path && (top.kind(q) == 1 || top.kind(&join(q, OPAQUE)) == 1)))
    }

    // the layer that shows path, or None
    fn layer_of(&self, path: &str) -> Option<&Arc<dyn Provider>> {
        if is_marker(path) {
            return None;
        }
        let top = self.layers.first()?;
        if top.kind(path) != 0 {
            return Some(top);
        }
        if self.hidden(path) {
            return None;
        }
        self.layers[1..].iter().find(|l| l.kind(path) != 0)
    }

    // a lower layer shows path (the top layer has to hide it after a delete or a rename)
    fn lower_has(&self, path: &str) -> bool {
        !self.hidden(path) && self.layers.iter().skip(1).any(|l| l.kind(path) != 0)
    }

    // before a write below path's folder: whited-out folders above it come back opaque
    fn reopen_above(&self, path: &str) -> bool {
        let Some(top) = self.layers.first() else { return false };
        for q in prefixes(parent(path)) {
            let wh = whiteout(q);
            if top.kind(&wh) == 1 && !(top.mkdir(q) && top.write(&join(q, OPAQUE), &[]) && top.delete(&wh)) {
                return false;
            }
        }
        true
    }

    // after a write of path: its own whiteout goes (a folder is opaque then)
    fn clear_whiteout(&self, path: &str, dir: bool) {
        let Some(top) = self.layers.first() else { return };
        let wh = whiteout(path);
        if top.kind(&wh) == 1 && (!dir || top.write(&join(path, OPAQUE), &[])) {
            top.delete(&wh);
        }
    }

    // hide path of the lower layers when one still shows it
    fn hide(&self, path: &str) -> bool {
        !self.lower_has(path) || self.layers.first().is_some_and(|t| t.write(&whiteout(path), &[]))
    }
}

impl Overlay {
    // File::open: reads from the layer that shows path, writes to the top layer (FO_RW copies
    // a lower layer's file up first)
    fn open_top(&self, path: &str, mode: u32) -> Option<crate::File> {
        let top = self.layers.first()?;
        if is_marker(path) || mode > crate::FO_RW {
            return None;
        }
        if mode == crate::FO_READ {
            return crate::open_in(self.layer_of(path)?, path, mode);
        }
        if !self.reopen_above(path) || !top.mkdir(parent(path)) {
            return None;
        }
        if mode == crate::FO_RW && top.kind(path) == 0 && self.kind(path) == 1 {
            let data = self.read(path)?;
            if !top.write(path, &data) {
                return None;
            }
        }
        let f = crate::open_in(top, path, mode)?;
        self.clear_whiteout(path, false);
        Some(f)
    }
}

impl Provider for Overlay {
    fn kind(&self, path: &str) -> u32 {
        self.layer_of(path).map_or(0, |l| l.kind(path))
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let l = self.layer_of(path)?;
        if l.kind(path) == 1 { l.read(path) } else { None }
    }

    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        if is_marker(path) {
            return false;
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut gone: BTreeSet<String> = BTreeSet::new();
        let mut opaque = false;
        let mut any = false;
        for (i, l) in self.layers.iter().enumerate() {
            if i == 1 && (opaque || self.hidden(path)) {
                break;
            }
            any |= l.list(path, &mut |n, d, s| {
                if let Some(h) = n.strip_prefix(WH) {
                    if i == 0 {
                        if n == OPAQUE {
                            opaque = true;
                        } else {
                            gone.insert(String::from(h));
                        }
                    }
                } else if (i == 0 || !gone.contains(n)) && seen.insert(String::from(n)) {
                    f(n, d, s);
                }
            });
        }
        any
    }

    fn write(&self, path: &str, data: &[u8]) -> bool {
        let Some(top) = self.layers.first() else { return false };
        let ok = !is_marker(path) && self.reopen_above(path) && top.write(path, data);
        if ok {
            self.clear_whiteout(path, false);
        }
        ok
    }

    fn mkdir(&self, path: &str) -> bool {
        let Some(top) = self.layers.first() else { return false };
        if path.is_empty() {
            return top.mkdir(path);
        }
        let ok = !is_marker(path) && self.reopen_above(path) && top.mkdir(path);
        if ok {
            self.clear_whiteout(path, true);
        }
        ok
    }

    fn delete(&self, path: &str) -> bool {
        let Some(top) = self.layers.first() else { return false };
        let kind = self.kind(path);
        if path.is_empty() || kind == 0 {
            return false;
        }
        if kind == 2 {
            let mut empty = true;
            self.list(path, &mut |_, _, _| empty = false);
            if !empty {
                return false;
            }
            if top.kind(path) == 2 {
                // the top folder holds only markers now
                let mut marks = Vec::new();
                top.list(path, &mut |n, _, _| marks.push(join(path, n)));
                if !marks.iter().all(|m| top.delete(m)) || !top.delete(path) {
                    return false;
                }
            }
        } else if top.kind(path) == 1 && !top.delete(path) {
            return false;
        }
        self.hide(path)
    }

    fn rename(&self, old: &str, new: &str) -> bool {
        let Some(top) = self.layers.first() else { return false };
        if old.is_empty() || new.is_empty() || is_marker(new) || old == new {
            return false;
        }
        match self.kind(old) {
            // a folder only when no lower layer has it
            2 if !self.lower_has(old) => {
                let ok = self.reopen_above(new) && top.rename(old, new);
                if ok {
                    self.clear_whiteout(new, true);
                }
                ok
            }
            1 => {
                let moved = if top.kind(old) == 1 {
                    self.reopen_above(new) && top.rename(old, new)
                } else {
                    // a lower layer's file: copied up under the new name
                    self.read(old).is_some_and(|d| self.reopen_above(new) && top.write(new, &d))
                };
                if moved {
                    self.clear_whiteout(new, false);
                }
                moved && self.hide(old)
            }
            _ => false,
        }
    }

    fn os_path(&self, path: &str) -> Option<String> {
        match self.layer_of(path) {
            Some(l) => l.os_path(path),
            None if is_marker(path) => None,
            None => self.layers.first()?.os_path(path), // a new file lands in the top layer
        }
    }

    fn stat(&self, path: &str) -> Option<Stat> {
        self.layer_of(path)?.stat(path)
    }

    fn set_times(&self, path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        let Some(top) = self.layers.first() else { return false };
        let ready = match (top.kind(path), self.kind(path)) {
            (0, 1) => self.read(path).is_some_and(|d| self.write(path, &d)), // copied up first
            (0, 2) => self.mkdir(path),
            (0, _) => false,
            _ => true,
        };
        ready && top.set_times(path, atime_ns, mtime_ns)
    }

    fn open(&self, path: &str, mode: u32) -> Opened {
        Opened::of(self.open_top(path, mode))
    }
}

/// one folder of another provider as a root (a theme folder of app://)
pub struct Sub {
    inner: Arc<dyn Provider>,
    dir: String,
}

impl Sub {
    /// dir: a folder of inner ('/' separated; None for "..")
    pub fn new(inner: Arc<dyn Provider>, dir: &str) -> Option<Sub> {
        Some(Sub { inner, dir: clean(dir)? })
    }

    fn full(&self, path: &str) -> String {
        match (self.dir.is_empty(), path.is_empty()) {
            (true, _) => String::from(path),
            (false, true) => self.dir.clone(),
            (false, false) => alloc::format!("{}/{}", self.dir, path),
        }
    }
}

impl Provider for Sub {
    fn kind(&self, path: &str) -> u32 {
        self.inner.kind(&self.full(path))
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.inner.read(&self.full(path))
    }
    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.inner.list(&self.full(path), f)
    }
    fn write(&self, path: &str, data: &[u8]) -> bool {
        !path.is_empty() && self.inner.write(&self.full(path), data)
    }
    fn mkdir(&self, path: &str) -> bool {
        self.inner.mkdir(&self.full(path))
    }
    fn delete(&self, path: &str) -> bool {
        !path.is_empty() && self.inner.delete(&self.full(path))
    }
    fn rename(&self, old: &str, new: &str) -> bool {
        !old.is_empty() && !new.is_empty() && self.inner.rename(&self.full(old), &self.full(new))
    }
    fn os_path(&self, path: &str) -> Option<String> {
        self.inner.os_path(&self.full(path))
    }
    fn stat(&self, path: &str) -> Option<Stat> {
        self.inner.stat(&self.full(path))
    }
    fn set_times(&self, path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        self.inner.set_times(&self.full(path), atime_ns, mtime_ns)
    }
    fn open(&self, path: &str, mode: u32) -> Opened {
        Opened::of(crate::open_in(&self.inner, &self.full(path), mode).filter(|_| !path.is_empty()))
    }
    fn display_name(&self, path: &str) -> Option<String> {
        self.inner.display_name(&self.full(path))
    }
    fn atomic_rename(&self) -> bool {
        self.inner.atomic_rename()
    }
}

// ---------------------------------------------------------------- Bundle

const MAGIC: &[u8; 4] = b"JFA1";
const ENTRY: usize = 20;
const CODEC_STORED: u8 = 0;
const CODEC_LZ4: u8 = 1;

struct Item {
    name: String,
    codec: u8,
    off: usize,
    stored: usize,
    size: usize,
}

/// a read-only JFA1 bundle (app://: the app's assets folder, packed by build_rs.py)
pub struct Bundle {
    data: BundleBytes,
    items: Vec<Item>,
}

#[derive(Clone)]
enum BundleBytes {
    Static(&'static [u8]),
    Owned(Arc<[u8]>),
}

impl BundleBytes {
    fn get(&self) -> &[u8] {
        match self {
            BundleBytes::Static(b) => b,
            BundleBytes::Owned(b) => b,
        }
    }
}

/// an open file of a bundle: a stored file in place, a packed one unpacked once
struct BundleStream {
    data: BundleBytes,
    off: usize,
    len: usize,
}

impl Stream for BundleStream {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        Some(read_slice(&self.data.get()[self.off..self.off + self.len], off, buf))
    }

    fn size(&self) -> Option<u64> {
        Some(self.len as u64)
    }
}

fn rd32(b: &[u8], i: usize) -> usize {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize
}

impl Bundle {
    /// a bundle without files
    pub fn empty() -> Bundle {
        Bundle { data: BundleBytes::Static(&[]), items: Vec::new() }
    }

    /// the bundle embedded in the program (None: malformed)
    pub fn from_static(b: &'static [u8]) -> Option<Bundle> {
        let items = index(b)?;
        Some(Bundle { data: BundleBytes::Static(b), items })
    }

    /// a bundle in memory (None: malformed)
    pub fn new(b: Vec<u8>) -> Option<Bundle> {
        let items = index(&b)?;
        Some(Bundle { data: BundleBytes::Owned(Arc::from(b)), items })
    }

    /// the number of files
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// whether the bundle has no files
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// the paths of all files, sorted
    pub fn names(&self) -> Vec<&str> {
        self.items.iter().map(|i| i.name.as_str()).collect()
    }

    fn find(&self, path: &str) -> Result<usize, usize> {
        self.items.binary_search_by(|i| i.name.as_bytes().cmp(path.as_bytes()))
    }

    // whether some name lies below the folder path
    fn is_dir(&self, path: &str) -> bool {
        if path.is_empty() {
            return true;
        }
        let pre = alloc::format!("{path}/");
        let k = self.find(&pre).unwrap_or_else(|k| k);
        self.items.get(k).is_some_and(|i| i.name.starts_with(&pre))
    }
}

// the entries of a JFA1 bundle, every range checked; None when anything is out of place
fn index(b: &[u8]) -> Option<Vec<Item>> {
    if b.is_empty() {
        return Some(Vec::new());
    }
    if b.len() < 12 || &b[..4] != MAGIC {
        return None;
    }
    let (count, pool_len) = (rd32(b, 4), rd32(b, 8));
    let pool = 12usize.checked_add(count.checked_mul(ENTRY)?)?;
    if pool.checked_add(pool_len)? > b.len() {
        return None;
    }
    let mut items: Vec<Item> = Vec::with_capacity(count);
    for k in 0..count {
        let e = 12 + k * ENTRY;
        let (no, nl) = (rd32(b, e), u16::from_le_bytes([b[e + 4], b[e + 5]]) as usize);
        let (codec, off, stored, size) = (b[e + 6], rd32(b, e + 8), rd32(b, e + 12), rd32(b, e + 16));
        if no.checked_add(nl)? > pool_len || off.checked_add(stored)? > b.len() || codec > CODEC_LZ4 {
            return None;
        }
        if codec == CODEC_STORED && stored != size {
            return None;
        }
        let name = core::str::from_utf8(&b[pool + no..pool + no + nl]).ok()?;
        if clean(name).as_deref() != Some(name) || name.is_empty() {
            return None;
        }
        if items.last().is_some_and(|l| l.name.as_bytes() >= name.as_bytes()) {
            return None; // not sorted, or twice
        }
        items.push(Item { name: name.to_owned(), codec, off, stored, size });
    }
    Some(items)
}

impl Provider for Bundle {
    fn kind(&self, path: &str) -> u32 {
        if self.find(path).is_ok() {
            1
        } else if self.is_dir(path) {
            2
        } else {
            0
        }
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let i = &self.items[self.find(path).ok()?];
        let blob = &self.data.get()[i.off..i.off + i.stored];
        match i.codec {
            CODEC_LZ4 => compress::lz4::decompress(blob, i.size).filter(|d| d.len() == i.size),
            _ => Some(blob.to_vec()),
        }
    }

    fn stat(&self, path: &str) -> Option<Stat> {
        match self.find(path) {
            Ok(k) => Some(Stat { kind: 1, size: self.items[k].size as u64, ..Stat::default() }),
            Err(_) => self.is_dir(path).then_some(Stat { kind: 2, ..Stat::default() }),
        }
    }

    fn open(&self, path: &str, mode: u32) -> Opened {
        let Ok(k) = self.find(path) else { return Opened::Refused };
        if mode != crate::FO_READ {
            return Opened::Refused;
        }
        let i = &self.items[k];
        let s = if i.codec == CODEC_STORED {
            BundleStream { data: self.data.clone(), off: i.off, len: i.stored }
        } else {
            let Some(d) = self.read(path) else { return Opened::Refused };
            BundleStream { len: d.len(), data: BundleBytes::Owned(Arc::from(d)), off: 0 }
        };
        Opened::File(Box::new(s))
    }

    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        if self.kind(path) != 2 {
            return false;
        }
        let pre = if path.is_empty() { String::new() } else { alloc::format!("{path}/") };
        let start = self.find(&pre).unwrap_or_else(|k| k);
        let mut last_dir = String::new();
        for i in self.items[start..].iter().take_while(|i| i.name.starts_with(&pre)) {
            let rest = &i.name[pre.len()..];
            match rest.find('/') {
                None => f(rest, false, i.size as u64),
                Some(k) => {
                    if rest[..k] != last_dir {
                        last_dir = String::from(&rest[..k]);
                        f(&last_dir, true, 0);
                    }
                }
            }
        }
        true
    }
}

/// a JFA1 bundle of (name, data) pairs: LZ4 where that saves a tenth (tests, tools); None for
/// a name route() would refuse, a name twice, or a bundle beyond 4 GiB
pub fn pack_bundle(files: &[(&str, &[u8])]) -> Option<Vec<u8>> {
    let mut v: Vec<(&str, &[u8])> = files.to_vec();
    v.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    if v.windows(2).any(|w| w[0].0 == w[1].0) || v.iter().any(|(n, _)| n.is_empty() || clean(n).as_deref() != Some(*n)) {
        return None;
    }
    let blobs: Vec<(u8, Vec<u8>, usize)> = v
        .iter()
        .map(|(_, d)| {
            let c = if d.is_empty() { Vec::new() } else { compress::lz4::compress(d) };
            if !d.is_empty() && c.len() * 10 <= d.len() * 9 { (CODEC_LZ4, c, d.len()) } else { (CODEC_STORED, d.to_vec(), d.len()) }
        })
        .collect();
    let pool_len: usize = v.iter().map(|(n, _)| n.len()).sum();
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    let u32_of = |x: usize| u32::try_from(x).ok();
    out.extend_from_slice(&u32_of(v.len())?.to_le_bytes());
    out.extend_from_slice(&u32_of(pool_len)?.to_le_bytes());
    let mut off = 12 + ENTRY * v.len() + pool_len;
    let mut name_off = 0;
    for ((n, _), (codec, blob, size)) in v.iter().zip(&blobs) {
        out.extend_from_slice(&u32_of(name_off)?.to_le_bytes());
        out.extend_from_slice(&u16::try_from(n.len()).ok()?.to_le_bytes());
        out.push(*codec);
        out.push(0);
        out.extend_from_slice(&u32_of(off)?.to_le_bytes());
        out.extend_from_slice(&u32_of(blob.len())?.to_le_bytes());
        out.extend_from_slice(&u32_of(*size)?.to_le_bytes());
        name_off += n.len();
        off += blob.len();
    }
    for (n, _) in &v {
        out.extend_from_slice(n.as_bytes());
    }
    for (_, blob, _) in &blobs {
        out.extend_from_slice(blob);
    }
    u32_of(out.len())?;
    Some(out)
}
