// win_sys -- Windows files, directories, environment, well-known paths, the date
//                                         [src/plat/win/win_sys.s, win_files.s, plat_win.s]
//
// Every function converts its paths into its own buffers, so all of them may run on any
// thread.  File::read_at / write_at use explicit offsets (OVERLAPPED positions): several
// threads may use one File at the same time.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::{from_wide, time_us, to_wide};
use crate::{Date, FO_READ, FO_RW, FO_WRITE};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemTimePreciseAsFileTime(ft: *mut u64);
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: usize, disp: u32, flags: u32, tmpl: isize)
        -> isize;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, done: *mut u32, ov: *mut Overlapped) -> i32;
    fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: *mut Overlapped) -> i32;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn FlushFileBuffers(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: usize) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
    fn GetTempPathW(n: u32, buf: *mut u16) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetCurrentDirectoryW(n: u32, buf: *mut u16) -> u32;
    fn FindFirstFileW(pat: *const u16, data: *mut FindData) -> isize;
    fn FindNextFileW(h: isize, data: *mut FindData) -> i32;
    fn FindClose(h: isize) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn GetLocalTime(st: *mut [u16; 8]);
}

const INVALID: isize = -1;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_ALL: u32 = 7;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTR_DIRECTORY: u32 = 0x10;
const FILE_ATTR_NORMAL: u32 = 0x80;
const FILE_ATTR_TEMPORARY: u32 = 0x100;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const MOVEFILE_REPLACE: u32 = 1;
const MOVEFILE_COPY_ALLOWED: u32 = 2;
const MOVEFILE_WRITETHROUGH: u32 = 8;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_ALREADY_EXISTS: u32 = 183;
const IO_CHUNK: usize = 0x4000_0000; // ReadFile / WriteFile take 32-bit lengths
const TEMP_TRIES: u32 = 16;
const TEMPW_MAX: usize = 1100;
const CWD_MAX: usize = 1040;

#[repr(C)]
#[derive(Default)]
pub(crate) struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u64, // Offset, OffsetHigh
    event: isize,
}

#[repr(C)]
struct FindData {
    attr: u32,
    times: [u32; 6],
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alt: [u16; 14],
}

/// '\' -> '/', trailing slashes dropped (not of "/" or "C:/")  [slash_fix]
fn slash_fix(s: String) -> String {
    let mut b: Vec<u8> = s.into_bytes();
    for c in b.iter_mut() {
        if *c == b'\\' {
            *c = b'/';
        }
    }
    while b.len() > 1 && b[b.len() - 1] == b'/' && !(b.len() == 3 && b[1] == b':') {
        b.pop();
    }
    String::from_utf8(b).unwrap_or_default()
}

// ---------------------------------------------------------------- environment

/// (device, inode) of a path: not used on Windows (every directory is visited)
pub fn dev_ino(_path: &str) -> Option<(u64, u64)> {
    None
}

/// the value of an environment variable  [plat_env]
pub fn env(name: &str) -> Option<String> {
    let wn = to_wide(name);
    // SAFETY: wn is NUL-terminated; the first call only asks for the size.
    let need = unsafe { GetEnvironmentVariableW(wn.as_ptr(), core::ptr::null_mut(), 0) };
    if need == 0 {
        return None;
    }
    let mut buf = alloc::vec![0u16; need as usize + 1];
    // SAFETY: buf holds need wide characters.
    let n = unsafe { GetEnvironmentVariableW(wn.as_ptr(), buf.as_mut_ptr(), need) };
    if n >= need {
        return None; // changed in between
    }
    Some(from_wide(&buf[..n as usize]))
}

/// the same as env (ASMLIB's plat_getenv)
pub fn getenv(name: &str) -> Option<String> {
    env(name)
}

/// set (Some) or remove (None) an environment variable of this process  [plat_setenv]
pub fn setenv(name: &str, value: Option<&str>) -> bool {
    let wn = to_wide(name);
    let wv = value.map(to_wide);
    let pv = wv.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
    // SAFETY: NUL-terminated strings (or null = remove).
    unsafe { SetEnvironmentVariableW(wn.as_ptr(), pv) != 0 }
}

// ---------------------------------------------------------------- well-known directories

/// %APPDATA%, else %USERPROFILE%/AppData/Roaming; '/' separators, no trailing slash
pub fn config_dir() -> Option<String> {
    if let Some(v) = env("APPDATA").filter(|v| !v.is_empty()) {
        return Some(slash_fix(v));
    }
    let p = env("USERPROFILE").filter(|v| !v.is_empty())?;
    Some(slash_fix(p + "/AppData/Roaming"))
}

/// %LOCALAPPDATA%, else %USERPROFILE%/AppData/Local; '/' separators, no trailing slash
pub fn data_dir() -> Option<String> {
    if let Some(v) = env("LOCALAPPDATA").filter(|v| !v.is_empty()) {
        return Some(slash_fix(v));
    }
    let p = env("USERPROFILE").filter(|v| !v.is_empty())?;
    Some(slash_fix(p + "/AppData/Local"))
}

/// the same folder as data_dir: Windows keeps an app's cache below its local folder
/// (vfs.rs: cache:// = <data_dir>/<app>/Cache)
pub fn cache_dir() -> Option<String> {
    data_dir()
}

/// GetTempPathW in the same form as config_dir
pub fn temp_dir() -> Option<String> {
    let mut buf = [0u16; TEMPW_MAX];
    // SAFETY: buf holds TEMPW_MAX wide characters.
    let n = unsafe { GetTempPathW(TEMPW_MAX as u32, buf.as_mut_ptr()) } as usize;
    if n == 0 || n >= TEMPW_MAX {
        return None;
    }
    let s = from_wide(&buf[..n]);
    if s.is_empty() { None } else { Some(slash_fix(s)) }
}

/// the current directory, native separators ("" on failure)  [plat_cwd]
pub fn cwd() -> String {
    let mut buf = [0u16; CWD_MAX];
    // SAFETY: buf holds CWD_MAX wide characters.
    let n = unsafe { GetCurrentDirectoryW(CWD_MAX as u32, buf.as_mut_ptr()) } as usize;
    if n == 0 || n >= CWD_MAX {
        return String::new();
    }
    from_wide(&buf[..n])
}

// ---------------------------------------------------------------- directories and names

fn attributes(path: &str) -> u32 {
    let w = to_wide(path);
    // SAFETY: NUL-terminated path.
    unsafe { GetFileAttributesW(w.as_ptr()) }
}

/// 0 missing, 1 file, 2 directory  [plat_file_exists]
pub(crate) fn file_exists_raw(path: &str) -> u32 {
    match attributes(path) {
        u32::MAX => 0,
        a if a & FILE_ATTR_DIRECTORY != 0 => 2,
        _ => 1,
    }
}

// one directory: created, or already a directory
fn mkdir1(path: &str) -> bool {
    let w = to_wide(path);
    // SAFETY: NUL-terminated path.
    if unsafe { CreateDirectoryW(w.as_ptr(), 0) } != 0 {
        return true;
    }
    let a = attributes(path);
    a != u32::MAX && a & FILE_ATTR_DIRECTORY != 0
}

/// create a directory and its missing parents; an existing directory is fine  [plat_mkdir]
pub(crate) fn mkdir_raw(path: &str) -> bool {
    if mkdir1(path) {
        return true;
    }
    let b = path.as_bytes();
    for i in 1..b.len() {
        if (b[i] == b'/' || b[i] == b'\\') && !matches!(b[i - 1], b':' | b'/' | b'\\') {
            mkdir1(&path[..i]); // errors are sorted out by the last step
        }
    }
    mkdir1(path)
}

/// a file or an empty directory  [plat_delete]
pub(crate) fn delete_raw(path: &str) -> bool {
    let w = to_wide(path);
    // SAFETY: NUL-terminated path.
    unsafe {
        if DeleteFileW(w.as_ptr()) != 0 {
            return true;
        }
        let a = GetFileAttributesW(w.as_ptr());
        a != u32::MAX && a & FILE_ATTR_DIRECTORY != 0 && RemoveDirectoryW(w.as_ptr()) != 0
    }
}

/// rename, replacing an existing target  [plat_rename]
pub(crate) fn rename_raw(old: &str, new: &str) -> bool {
    let (wo, wn) = (to_wide(old), to_wide(new));
    // SAFETY: NUL-terminated paths.
    unsafe { MoveFileExW(wo.as_ptr(), wn.as_ptr(), MOVEFILE_REPLACE | MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITETHROUGH) != 0 }
}

/// f(name, is_dir, size_lo) for every entry except "." and ".."  [plat_dir_list]
pub(crate) fn dir_list_raw(path: &str, f: &mut dyn FnMut(&str, bool, u32)) -> bool {
    let mut pat = String::from(path);
    if !(pat.ends_with('/') || pat.ends_with('\\')) || pat.is_empty() {
        pat.push('\\');
    }
    pat.push('*');
    let w = to_wide(&pat);
    // SAFETY: FindData is the WIN32_FIND_DATAW layout; the handle is closed.
    unsafe {
        let mut d: FindData = core::mem::zeroed();
        let h = FindFirstFileW(w.as_ptr(), &mut d);
        if h == INVALID {
            return false;
        }
        loop {
            let n = d.name.iter().position(|&c| c == 0).unwrap_or(260);
            let name = &d.name[..n];
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                f(&from_wide(name), d.attr & FILE_ATTR_DIRECTORY != 0, d.size_low);
            }
            if FindNextFileW(h, &mut d) == 0 {
                break;
            }
        }
        FindClose(h);
    }
    true
}

// ---------------------------------------------------------------- whole files

/// the contents of a file (at most 2 GB)  [plat_file_read]
pub(crate) fn read_file_raw(path: &str) -> Option<Vec<u8>> {
    let f = OsFile::open_mode(path, GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING)?;
    let size = f.size()?;
    if size > 0x7FFF_FFF0 {
        return None;
    }
    let mut data = alloc::vec![0u8; size as usize];
    let got = f.read_at(0, &mut data)?;
    data.truncate(got);
    Some(data)
}

/// Write an exclusively created sibling, flush it, then replace path. [plat_file_write]
pub(crate) fn write_file_raw(path: &str, data: &[u8]) -> bool {
    OsAtomic::begin(path).is_some_and(|a| a.write_at(0, data) == Some(data.len()) && a.commit())
}

/// A file written beside its target and moved over it by commit (write_file_raw and the
/// streaming writers of guarded.rs / aio.rs).  Dropped without commit, the sibling is deleted.
pub(crate) struct OsAtomic {
    f: Option<OsFile>,
    tmp: Vec<u16>, // NUL-terminated; empty once moved
    dst: Vec<u16>,
}

impl OsAtomic {
    /// an exclusively created sibling "PATH.tmp-PID-SEQ-TIME" (not shared while it is written);
    /// a folder at path refuses
    pub(crate) fn begin(path: &str) -> Option<OsAtomic> {
        if path.is_empty() || path.contains('\0') || file_exists_checked_raw(path) == Some(2) { return None; }
        // SAFETY: no arguments.
        let pid = unsafe { GetCurrentProcessId() };
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = format!("{path}.tmp-{pid:x}-{seq:x}-{:x}", time_us() as u32);
            let Some(f) = OsFile::open_mode(&tmp, GENERIC_WRITE, 0, CREATE_NEW) else {
                // SAFETY: no arguments; inspect the failed creation.
                let e = unsafe { GetLastError() };
                if e == ERROR_FILE_EXISTS || e == ERROR_ALREADY_EXISTS { continue; }
                return None;
            };
            return Some(OsAtomic { f: Some(f), tmp: to_wide(&tmp), dst: to_wide(path) });
        }
        None
    }

    /// bytes written at off of the sibling
    pub(crate) fn write_at(&self, off: u64, data: &[u8]) -> Option<usize> {
        self.f.as_ref()?.write_at(off, data)
    }

    /// flush, move over the target (write-through) -> false: the target is unchanged and the
    /// sibling gone
    pub(crate) fn commit(mut self) -> bool {
        let Some(f) = self.f.take() else { return false };
        let ok = f.sync();
        drop(f);
        // SAFETY: NUL-terminated paths; tmp was created by begin.
        if ok && unsafe { MoveFileExW(self.tmp.as_ptr(), self.dst.as_ptr(), MOVEFILE_REPLACE | MOVEFILE_WRITETHROUGH) } != 0 {
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
            unsafe { DeleteFileW(self.tmp.as_ptr()) };
        }
    }
}

// ---------------------------------------------------------------- streaming files

/// an open file (closed when dropped)
pub(crate) struct OsFile(isize);

/// Nonblocking exclusive sidecar lock. Drop releases the OS handle; keep the file in place.
/// All cooperating writers must lock the same path before checking and replacing data.
pub(crate) struct OsFileLock { _file: OsFile }
impl OsFileLock {
    /// Open or create without truncating. None means busy, invalid path or I/O failure.
    pub(crate) fn acquire_raw(path: &str) -> Option<Self> {
        if !matches!(file_exists_checked_raw(path), Some(0 | 1)) { return None; }
        OsFile::open_mode(path, GENERIC_READ | GENERIC_WRITE, 0, OPEN_ALWAYS)
            .map(|file| Self { _file: file })
    }
}

/// Some(0) only for a missing path, Some(1/2) for file/directory, None for other errors.
pub(crate) fn file_exists_checked_raw(path: &str) -> Option<u32> {
    if path.is_empty() || path.contains('\0') { return None; }
    let w = to_wide(path);
    // SAFETY: w is NUL-terminated; GetLastError immediately follows a failed call.
    unsafe {
        let attr = GetFileAttributesW(w.as_ptr());
        if attr != u32::MAX { return Some(if attr & FILE_ATTR_DIRECTORY != 0 { 2 } else { 1 }); }
        match GetLastError() { 2 | 3 => Some(0), _ => None }
    }
}

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

impl OsFile {
    /// Create a named file exclusively, allowing a helper to read while its owner writes.
    pub(crate) fn create_new_raw(path: &str) -> Option<OsFile> {
        Self::open_mode(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, CREATE_NEW)
    }

    fn open_mode(path: &str, access: u32, share: u32, disp: u32) -> Option<OsFile> {
        Self::create(path, access, share, disp, FILE_ATTR_NORMAL)
    }

    fn create(path: &str, access: u32, share: u32, disp: u32, flags: u32) -> Option<OsFile> {
        let w = to_wide(path);
        // SAFETY: NUL-terminated path.
        let h = unsafe { CreateFileW(w.as_ptr(), access, share, 0, disp, flags, 0) };
        if h == INVALID { None } else { Some(OsFile(h)) }
    }

    /// FO_READ (existing, read only), FO_WRITE (create / truncate), FO_RW (open or create)
    pub(crate) fn open_raw(path: &str, mode: u32) -> Option<OsFile> {
        match mode {
            FO_READ => Self::open_mode(path, GENERIC_READ, FILE_SHARE_ALL, OPEN_EXISTING),
            FO_WRITE => Self::open_mode(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, CREATE_ALWAYS),
            FO_RW => Self::open_mode(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ, OPEN_ALWAYS),
            _ => None,
        }
    }

    /// a new file "jfgui-*.tmp" in dir (None = temp_dir) that the system deletes when it is
    /// closed (also after a crash)  [plat_file_open_temp]
    pub(crate) fn open_temp_raw(dir: Option<&str>) -> Option<OsFile> {
        let dir = match dir {
            Some(d) => String::from(d),
            None => temp_dir()?,
        };
        // SAFETY: no arguments.
        let pid = unsafe { GetCurrentProcessId() };
        for _ in 0..TEMP_TRIES {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
            let name = format!("{dir}/jfgui-{pid:x}-{seq:x}-{:x}.tmp", time_us() as u32);
            if let Some(f) = Self::create(&name, GENERIC_READ | GENERIC_WRITE, 0, CREATE_NEW,
                                          FILE_ATTR_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE) {
                return Some(f);
            }
            // SAFETY: no arguments.
            let e = unsafe { GetLastError() };
            if e != ERROR_FILE_EXISTS && e != ERROR_ALREADY_EXISTS {
                return None;
            }
        }
        None
    }

    /// bytes read at off (fewer at the end of the file), None on an error before any byte
    pub fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < buf.len() {
            let n = (buf.len() - done).min(IO_CHUNK) as u32;
            let mut ov = Overlapped { offset: off + done as u64, ..Default::default() };
            let mut got = 0u32;
            // SAFETY: buf[done..] holds at least n bytes; ov lives during the (synchronous) call.
            let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr().add(done), n, &mut got, &mut ov) };
            if ok == 0 {
                // SAFETY: no arguments.
                if unsafe { GetLastError() } == ERROR_HANDLE_EOF || done > 0 {
                    break;
                }
                return None;
            }
            if got == 0 {
                break; // end of file
            }
            done += got as usize;
        }
        Some(done)
    }

    /// bytes written at off (the file grows as needed), None on an error before any byte
    pub fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < buf.len() {
            let n = (buf.len() - done).min(IO_CHUNK) as u32;
            let mut ov = Overlapped { offset: off + done as u64, ..Default::default() };
            let mut put = 0u32;
            // SAFETY: buf[done..] holds at least n bytes; ov lives during the call.
            let ok = unsafe { WriteFile(self.0, buf.as_ptr().add(done), n, &mut put, &mut ov) };
            if ok == 0 {
                if done > 0 {
                    break;
                }
                return None;
            }
            if put == 0 {
                break;
            }
            done += put as usize;
        }
        Some(done)
    }

    pub fn size(&self) -> Option<u64> {
        let mut s = -1i64;
        // SAFETY: plain out-parameter.
        if unsafe { GetFileSizeEx(self.0, &mut s) } == 0 || s < 0 { None } else { Some(s as u64) }
    }

    /// write cached data to the disk
    pub fn sync(&self) -> bool {
        // SAFETY: a valid handle.
        unsafe { FlushFileBuffers(self.0) != 0 }
    }

    /// cut or extend the file to `len` bytes
    pub fn set_len(&self, len: u64) -> bool {
        #[link(name = "kernel32", kind = "raw-dylib")]
        unsafe extern "system" {
            fn SetFileInformationByHandle(h: isize, class: u32, info: *const u8, size: u32) -> i32;
        }
        // FileEndOfFileInfo (6): FILE_END_OF_FILE_INFO { LARGE_INTEGER EndOfFile }
        let v = len as i64;
        // SAFETY: an 8-byte record for a valid handle.
        unsafe { SetFileInformationByHandle(self.0, 6, &v as *const i64 as *const u8, 8) != 0 }
    }
}

impl Drop for OsFile {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this File.
        unsafe { CloseHandle(self.0) };
    }
}

// ---------------------------------------------------------------- date

/// wall-clock time: nanoseconds since 1970-01-01 00:00 UTC
pub fn unix_time_ns() -> i64 {
    let mut ft = 0u64;
    // SAFETY: FILETIME is a 64-bit count of 100 ns since 1601.
    unsafe { GetSystemTimePreciseAsFileTime(&mut ft) };
    (ft as i64).wrapping_sub(116_444_736_000_000_000).wrapping_mul(100)
}

/// the local date and time; the flag is false (local time, not UTC)  [plat_date]
pub fn date() -> (Date, bool) {
    let mut st = [0u16; 8];
    // SAFETY: SYSTEMTIME is eight u16.
    unsafe { GetLocalTime(&mut st) };
    let d = Date { year: st[0], month: st[1], wday: st[2], day: st[3], hour: st[4], min: st[5], sec: st[6], ms: st[7] };
    (d, false)
}
