// fs -- file system access for backup and restore
//
//   stat(path) / lstat(path) -> Option<Meta>     kind, size, times (ns), mode / attributes,
//                                                 owner, device and file id, link count
//   read_dir(path) -> Option<Vec<DirEntry>>      names and kinds ("." / ".." left out)
//   read_link / symlink(target, link, dir) / hard_link(existing, new)
//   set_times(path, atime_ns, mtime_ns)          on the link itself, never its target
//   set_mode(path, mode) (Linux) / set_attrs(path, attrs) (Windows) / set_owner (Linux)
//   free_space(path) -> Option<(free, total)>    sync_dir(dir) (Linux: fsync of the directory)
//   write_file_private(path, data)               owner-only file (0600 / protected DACL),
//                                                fsync, atomic rename, directory sync
//   mkdir_private(path)                          owner-only directory (with parents)
//   local_zone_id() -> "Europe/Berlin" | "win:W. Europe Standard Time"
//   long_path(path) (Windows): the \\?\ form every call here uses
//
// Windows: reparse points count as links only for symbolic links and junctions; other tags
// (deduplication, cloud placeholders) are ordinary files whose data is read.  Files are
// opened with backup semantics (directories, and backup privileges when the process holds
// them).
//
// vfs names ("scheme://path", vfs.rs) work too: a folder mount's name is its OS path (and
// checked as one); mem://, bundles, ZIP archives and other mounts answer from the provider
// (stat: kind, size, times; read_dir; set_times; write_file_private / mkdir_private as
// write / mkdir).  They have no links (read_link None, symlink / hard_link false), owners,
// modes or free space.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::vfs::{self, Provider, Route};

/// what a directory entry is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    /// a symbolic link (Windows: also a junction)
    Symlink,
    /// devices, pipes, sockets
    Other,
}

/// the metadata of a file
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub kind: Option<FileKind>,
    pub size: u64,
    /// nanoseconds since 1970
    pub mtime_ns: i64,
    pub atime_ns: i64,
    /// Linux: status change; Windows: creation
    pub ctime_ns: i64,
    /// Linux: st_mode (type and permission bits); Windows: 0
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// Windows: FILE_ATTRIBUTE_*; Linux: 0
    pub attrs: u32,
    /// device (Windows: volume serial number) and file id (inode / file index)
    pub dev: u64,
    pub ino: u64,
    pub nlink: u32,
}

impl Meta {
    pub fn is_dir(&self) -> bool {
        self.kind == Some(FileKind::Dir)
    }
    pub fn is_file(&self) -> bool {
        self.kind == Some(FileKind::File)
    }
    pub fn is_symlink(&self) -> bool {
        self.kind == Some(FileKind::Symlink)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: FileKind,
}

/// "a/b" + "c" with the platform separator
pub fn join(dir: &str, name: &str) -> String {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let mut s = String::from(dir);
    if !s.is_empty() && !s.ends_with(['/', '\\']) {
        s.push(sep);
    }
    s.push_str(name);
    s
}

/// the parent directory of a path ("" for a bare name)
pub fn parent(path: &str) -> &str {
    let t = path.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(0) => &t[..1],
        Some(i) => &t[..i],
        None => "",
    }
}

#[cfg(windows)]
pub use win::*;
#[cfg(target_os = "linux")]
pub use lnx::*;
#[cfg(windows)]
use win as sys;
#[cfg(target_os = "linux")]
use lnx as sys;

// ------------------------------------------------------------------ vfs names

// where a name leads: an OS path (None: the name itself; a folder mount: its OS path) or a
// mount without OS files
enum At {
    Os(Option<String>),
    Mount(Arc<dyn Provider>, String),
    Bad,
}

fn at(path: &str) -> At {
    match vfs::route(path) {
        Route::Os => At::Os(None),
        Route::Vfs(p, rest) => match p.os_path(&rest) {
            Some(o) => At::Os(Some(o)),
            None => At::Mount(p, rest),
        },
        Route::Bad => At::Bad,
    }
}

// the metadata of a mount's entry: no links, owner, device or file id
fn meta(s: vfs::Stat) -> Meta {
    let dir = s.kind == 2;
    Meta {
        kind: Some(if dir { FileKind::Dir } else { FileKind::File }),
        size: s.size,
        mtime_ns: s.mtime_ns,
        atime_ns: s.atime_ns,
        ctime_ns: s.mtime_ns,
        mode: if cfg!(windows) { 0 } else if dir { 0o040755 } else { 0o100644 },
        attrs: if !cfg!(windows) { 0 } else if dir { 0x10 } else { 0x80 },
        nlink: 1,
        ..Meta::default()
    }
}

/// the metadata of path, links followed; a vfs name: its folder mount's file, else the
/// provider's kind, size and times (mtime also as ctime; Linux modes 0644 / 0755)
pub fn stat(path: &str) -> Option<Meta> {
    match at(path) {
        At::Os(o) => sys::stat(o.as_deref().unwrap_or(path)),
        At::Mount(p, r) => p.stat(&r).map(meta),
        At::Bad => None,
    }
}

/// the metadata of path itself (a link is not followed); mounts without OS files have no links
pub fn lstat(path: &str) -> Option<Meta> {
    match at(path) {
        At::Os(o) => sys::lstat(o.as_deref().unwrap_or(path)),
        At::Mount(p, r) => p.stat(&r).map(meta),
        At::Bad => None,
    }
}

/// the entries of a folder ("." / ".." left out)
pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
    match at(path) {
        At::Os(o) => sys::read_dir(o.as_deref().unwrap_or(path)),
        At::Mount(p, r) => {
            let mut v = Vec::new();
            let ok = p.list(&r, &mut |n, d, _| v.push(DirEntry { name: String::from(n), kind: if d { FileKind::Dir } else { FileKind::File } }));
            ok.then_some(v)
        }
        At::Bad => None,
    }
}

/// the target of a symbolic link (Windows: also a junction)
pub fn read_link(path: &str) -> Option<String> {
    match at(path) {
        At::Os(o) => sys::read_link(o.as_deref().unwrap_or(path)),
        _ => None,
    }
}

/// a symbolic link at link to target (dir: a folder's link on Windows); a vfs name as target
/// stands for its OS path.  Only on OS paths and folder mounts.
pub fn symlink(target: &str, link: &str, dir: bool) -> bool {
    let t = match vfs::route(target) {
        Route::Os => String::from(target),
        _ => match vfs::os_path(target) {
            Some(t) => t,
            None => return false,
        },
    };
    match at(link) {
        At::Os(o) => sys::symlink(&t, o.as_deref().unwrap_or(link), dir),
        _ => false,
    }
}

/// a second name new for the file existing (both OS paths or folder mounts)
pub fn hard_link(existing: &str, new: &str) -> bool {
    match (at(existing), at(new)) {
        (At::Os(e), At::Os(n)) => sys::hard_link(e.as_deref().unwrap_or(existing), n.as_deref().unwrap_or(new)),
        _ => false,
    }
}

/// access and modification times (ns since 1970), on a link itself, never its target; a
/// mount: its provider's times (MemFs keeps them; bundles and ZIP archives are read-only)
pub fn set_times(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
    match at(path) {
        At::Os(o) => sys::set_times(o.as_deref().unwrap_or(path), atime_ns, mtime_ns),
        At::Mount(p, r) => p.set_times(&r, atime_ns, mtime_ns),
        At::Bad => false,
    }
}

/// permission bits (07777; Windows: nothing to do); a mount without OS files has none
pub fn set_mode(path: &str, mode: u32) -> bool {
    match at(path) {
        At::Os(o) => sys::set_mode(o.as_deref().unwrap_or(path), mode),
        At::Mount(p, r) => cfg!(windows) && p.kind(&r) != 0,
        At::Bad => false,
    }
}

/// FILE_ATTRIBUTE_* (read-only, hidden, system, archive ...; Linux: nothing to do)
pub fn set_attrs(path: &str, attrs: u32) -> bool {
    match at(path) {
        At::Os(o) => sys::set_attrs(o.as_deref().unwrap_or(path), attrs),
        At::Mount(p, r) => !cfg!(windows) && p.kind(&r) != 0,
        At::Bad => false,
    }
}

/// owner and group (Linux; Windows: nothing to do)
pub fn set_owner(path: &str, uid: u32, gid: u32) -> bool {
    match at(path) {
        At::Os(o) => sys::set_owner(o.as_deref().unwrap_or(path), uid, gid),
        At::Mount(p, r) => cfg!(windows) && p.kind(&r) != 0,
        At::Bad => false,
    }
}

/// (free, total) bytes of the volume of path; None for mounts without OS files
pub fn free_space(path: &str) -> Option<(u64, u64)> {
    match at(path) {
        At::Os(o) => sys::free_space(o.as_deref().unwrap_or(path)),
        _ => None,
    }
}

/// make a folder's changes durable (Linux: fsync of the directory); a mount without OS files:
/// whether the folder exists
pub fn sync_dir(dir: &str) -> bool {
    match at(dir) {
        At::Os(o) => sys::sync_dir(o.as_deref().unwrap_or(dir)),
        At::Mount(p, r) => p.kind(&r) == 2,
        At::Bad => false,
    }
}

/// an owner-only file (0600 / protected DACL), synced and renamed into place; a mount without
/// OS files: its write (memory of this process)
pub fn write_file_private(path: &str, data: &[u8]) -> bool {
    match at(path) {
        At::Os(o) => sys::write_file_private(o.as_deref().unwrap_or(path), data),
        At::Mount(p, r) => !r.is_empty() && p.write(&r, data),
        At::Bad => false,
    }
}

/// an owner-only folder (with parents)
pub fn mkdir_private(path: &str) -> bool {
    match at(path) {
        At::Os(o) => sys::mkdir_private(o.as_deref().unwrap_or(path)),
        At::Mount(p, r) => p.mkdir(&r),
        At::Bad => false,
    }
}

// ------------------------------------------------------------------ Windows

#[cfg(windows)]
mod win {
    use super::{DirEntry, FileKind, Meta};
    use alloc::string::String;
    use alloc::vec::Vec;

    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const SHARE_ALL: u32 = 7;
    const OPEN_EXISTING: u32 = 3;
    const CREATE_ALWAYS: u32 = 2;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const WRITE_THROUGH: u32 = 0x8000_0000;
    const ATTR_DIRECTORY: u32 = 0x10;
    const ATTR_REPARSE: u32 = 0x400;
    const ATTR_DEVICE: u32 = 0x40;
    const TAG_SYMLINK: u32 = 0xA000_000C;
    const TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
    const INVALID: isize = -1;
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000; // 1601 -> 1970 in 100 ns

    /// BY_HANDLE_FILE_INFORMATION (FILETIMEs are two DWORDs: 4-byte alignment)
    #[repr(C)]
    struct ByHandleInfo {
        attrs: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_hi: u32,
        size_lo: u32,
        links: u32,
        index_hi: u32,
        index_lo: u32,
    }

    /// WIN32_FIND_DATAW (laid out like win_sys.rs's record: the declarations must agree);
    /// reserved[0] holds the reparse tag
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

    #[repr(C)]
    struct SecurityAttributes {
        len: u32,
        descriptor: usize,
        inherit: i32,
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: usize, disp: u32, flags: u32, tmpl: isize) -> isize;
        fn CloseHandle(h: isize) -> i32;
        fn GetFileInformationByHandle(h: isize, info: *mut ByHandleInfo) -> i32;
        fn GetFileInformationByHandleEx(h: isize, class: u32, info: *mut u8, size: u32) -> i32;
        fn FindFirstFileExW(name: *const u16, level: u32, data: *mut FindData, op: u32, filter: usize, flags: u32) -> isize;
        fn FindNextFileW(h: isize, data: *mut FindData) -> i32;
        fn FindClose(h: isize) -> i32;
        fn DeviceIoControl(h: isize, code: u32, inb: usize, inn: u32, outb: *mut u8, outn: u32, ret: *mut u32, ov: usize) -> i32;
        fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
        fn CreateHardLinkW(new: *const u16, existing: *const u16, sa: usize) -> i32;
        fn SetFileTime(h: isize, created: *const u64, accessed: *const u64, written: *const u64) -> i32;
        fn SetFileAttributesW(name: *const u16, attrs: u32) -> i32;
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: *mut crate::win::Overlapped) -> i32;
        fn FlushFileBuffers(h: isize) -> i32;
        fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32;
        fn CreateDirectoryW(name: *const u16, sa: usize) -> i32;
        fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GetDynamicTimeZoneInformation(tz: *mut u8) -> u32;
        fn GetFullPathNameW(name: *const u16, n: u32, buf: *mut u16, part: usize) -> u32;
    }

    #[link(name = "advapi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, rev: u32, sd: *mut usize, len: *mut u32) -> i32;
    }

    fn wz(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    /// the \\?\ form of a path (absolute, backslashes): no MAX_PATH limit, no name rewriting
    pub fn long_path(path: &str) -> String {
        if path.starts_with(r"\\?\") {
            return String::from(path);
        }
        let w = wz(path);
        let mut buf = alloc::vec![0u16; 1024];
        loop {
            // SAFETY: buf holds buf.len() u16; the call writes at most that many.
            let n = unsafe { GetFullPathNameW(w.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), 0) } as usize;
            if n == 0 {
                return String::from(path);
            }
            if n < buf.len() {
                buf.truncate(n);
                break;
            }
            buf.resize(n + 1, 0);
        }
        let full = String::from_utf16_lossy(&buf);
        if let Some(unc) = full.strip_prefix(r"\\") {
            alloc::format!(r"\\?\UNC\{}", unc)
        } else {
            alloc::format!(r"\\?\{}", full)
        }
    }

    fn lp(path: &str) -> Vec<u16> {
        wz(&long_path(path))
    }

    struct H(isize);
    impl Drop for H {
        fn drop(&mut self) {
            // SAFETY: an owned handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(path: &str, access: u32, flags: u32) -> Option<H> {
        let w = lp(path);
        // SAFETY: a valid wide path; no security attributes.
        let h = unsafe { CreateFileW(w.as_ptr(), access, SHARE_ALL, 0, OPEN_EXISTING, flags | BACKUP_SEMANTICS, 0) };
        (h != INVALID).then_some(H(h))
    }

    fn ft_ns(ft: [u32; 2]) -> i64 {
        (((ft[1] as u64) << 32 | ft[0] as u64) as i64 - EPOCH_DIFF).saturating_mul(100)
    }

    fn ns_ft(ns: i64) -> u64 {
        (ns / 100 + EPOCH_DIFF).max(0) as u64
    }

    fn reparse_tag(h: &H) -> u32 {
        let mut b = [0u32; 2]; // FILE_ATTRIBUTE_TAG_INFO: attributes, reparse tag
        // SAFETY: an 8-byte output record.
        let ok = unsafe { GetFileInformationByHandleEx(h.0, 9, b.as_mut_ptr() as *mut u8, 8) };
        if ok != 0 { b[1] } else { 0 }
    }

    fn kind_of(attrs: u32, tag: u32) -> FileKind {
        if attrs & ATTR_REPARSE != 0 && (tag == TAG_SYMLINK || tag == TAG_MOUNT_POINT) {
            FileKind::Symlink
        } else if attrs & ATTR_DIRECTORY != 0 {
            FileKind::Dir
        } else if attrs & ATTR_DEVICE != 0 {
            FileKind::Other
        } else {
            FileKind::File
        }
    }

    fn meta_of(h: &H) -> Option<Meta> {
        // SAFETY: the record is plain data, zero is a valid start.
        let mut i: ByHandleInfo = unsafe { core::mem::zeroed() };
        // SAFETY: a valid handle and output record.
        if unsafe { GetFileInformationByHandle(h.0, &mut i) } == 0 {
            return None;
        }
        let tag = if i.attrs & ATTR_REPARSE != 0 { reparse_tag(h) } else { 0 };
        Some(Meta {
            kind: Some(kind_of(i.attrs, tag)),
            size: (i.size_hi as u64) << 32 | i.size_lo as u64,
            mtime_ns: ft_ns(i.written),
            atime_ns: ft_ns(i.accessed),
            ctime_ns: ft_ns(i.created),
            mode: 0,
            uid: 0,
            gid: 0,
            attrs: i.attrs,
            dev: i.volume as u64,
            ino: (i.index_hi as u64) << 32 | i.index_lo as u64,
            nlink: i.links,
        })
    }

    pub fn stat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        meta_of(&open(path, FILE_READ_ATTRIBUTES, 0)?)
    }

    pub fn lstat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        meta_of(&open(path, FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT)?)
    }

    pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let pat = wz(&super::join(&long_path(path), "*"));
        // SAFETY: plain data, filled by the call.
        let mut d: FindData = unsafe { core::mem::zeroed() };
        // FindExInfoBasic, FindExSearchNameMatch, FIND_FIRST_EX_LARGE_FETCH
        // SAFETY: a valid pattern and output record.
        let h = unsafe { FindFirstFileExW(pat.as_ptr(), 1, &mut d, 0, 0, 2) };
        if h == INVALID {
            return None;
        }
        let mut out = Vec::new();
        loop {
            let n = d.name.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&d.name[..n]);
            if name != "." && name != ".." {
                out.push(DirEntry { kind: kind_of(d.attr, d.reserved[0]), name });
            }
            // SAFETY: the search handle and record from above.
            if unsafe { FindNextFileW(h, &mut d) } == 0 {
                break;
            }
        }
        // SAFETY: the search handle, closed once.
        unsafe { FindClose(h) };
        Some(out)
    }

    pub fn read_link(path: &str) -> Option<String> {
        if !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
            return None;
        }
        let h = open(path, FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT)?;
        let mut buf = alloc::vec![0u8; 16 * 1024];
        let mut got = 0u32;
        // SAFETY: a valid handle and output buffer.
        if unsafe { DeviceIoControl(h.0, FSCTL_GET_REPARSE_POINT, 0, 0, buf.as_mut_ptr(), buf.len() as u32, &mut got, 0) } == 0 {
            return None;
        }
        let rd16 = |o: usize| u16::from_le_bytes([buf[o], buf[o + 1]]) as usize;
        let tag = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let (path_buf, sub_off, sub_len, print_off, print_len) = match tag {
            TAG_SYMLINK => (20, rd16(8), rd16(10), rd16(12), rd16(14)),
            TAG_MOUNT_POINT => (16, rd16(8), rd16(10), rd16(12), rd16(14)),
            _ => return None,
        };
        let text = |off: usize, len: usize| -> Option<String> {
            let b = buf.get(path_buf + off..path_buf + off + len)?;
            let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&w))
        };
        let print = text(print_off, print_len)?;
        if !print.is_empty() {
            return Some(print);
        }
        let sub = text(sub_off, sub_len)?;
        Some(String::from(sub.strip_prefix(r"\??\").unwrap_or(&sub)))
    }

    /// a symbolic link (without the developer mode or the privilege this fails)
    pub fn symlink(target: &str, link: &str, dir: bool) -> bool {
        if !(crate::policy::fs_allowed(link, crate::policy::FS_WRITE)) {
            return false;
        }
        let (t, l) = (wz(target), lp(link));
        // SYMBOLIC_LINK_FLAG_DIRECTORY, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE
        // SAFETY: valid wide strings.
        unsafe { CreateSymbolicLinkW(l.as_ptr(), t.as_ptr(), if dir { 1 } else { 0 } | 2) != 0 }
    }

    pub fn hard_link(existing: &str, new: &str) -> bool {
        if !(crate::policy::fs_allowed(existing, crate::policy::FS_READ) && crate::policy::fs_allowed(new, crate::policy::FS_WRITE)) {
            return false;
        }
        let (e, n) = (lp(existing), lp(new));
        // SAFETY: valid wide strings.
        unsafe { CreateHardLinkW(n.as_ptr(), e.as_ptr(), 0) != 0 }
    }

    pub fn set_times(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let Some(h) = open(path, FILE_WRITE_ATTRIBUTES, OPEN_REPARSE_POINT) else { return false };
        let (a, m) = (ns_ft(atime_ns), ns_ft(mtime_ns));
        // SAFETY: a valid handle; creation time unchanged (null).
        unsafe { SetFileTime(h.0, core::ptr::null(), &a, &m) != 0 }
    }

    /// FILE_ATTRIBUTE_* (read-only, hidden, system, archive ...)
    pub fn set_attrs(path: &str, attrs: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let w = lp(path);
        // SAFETY: a valid wide path.
        unsafe { SetFileAttributesW(w.as_ptr(), attrs & 0x27 | if attrs & 0x27 == 0 { 0x80 } else { 0 }) != 0 }
    }

    pub fn set_mode(_path: &str, _mode: u32) -> bool {
        true
    }

    pub fn set_owner(_path: &str, _uid: u32, _gid: u32) -> bool {
        true
    }

    pub fn free_space(path: &str) -> Option<(u64, u64)> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let w = lp(path);
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: valid output pointers.
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
        (ok != 0).then_some((avail, total))
    }

    pub fn sync_dir(_dir: &str) -> bool {
        true // NTFS journals the directory change with the rename
    }

    /// a security descriptor: full access for SYSTEM, the administrators and the owner only
    fn private_sa() -> Option<(SecurityAttributes, usize)> {
        let sddl = wz("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)");
        let mut sd = 0usize;
        // SAFETY: valid SDDL; the descriptor is freed with LocalFree by the caller.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, core::ptr::null_mut()) } == 0 {
            return None;
        }
        Some((SecurityAttributes { len: core::mem::size_of::<SecurityAttributes>() as u32, descriptor: sd, inherit: 0 }, sd))
    }

    pub fn write_file_private(path: &str, data: &[u8]) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let tmp = alloc::format!("{}.tmp{}", path, crate::time_us() % 1_000_000);
        let w = lp(&tmp);
        let Some((sa, sd)) = private_sa() else { return false };
        // SAFETY: a valid path and security attributes.
        let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_WRITE, 0, &sa as *const SecurityAttributes as usize, CREATE_ALWAYS, WRITE_THROUGH, 0) };
        // SAFETY: the descriptor from ConvertString..., no longer needed.
        unsafe { LocalFree(sd as *mut core::ffi::c_void) };
        if h == INVALID {
            return false;
        }
        let h = H(h);
        let mut off = 0;
        while off < data.len() {
            let n = (data.len() - off).min(1 << 30) as u32;
            let mut done = 0u32;
            // SAFETY: the buffer slice is valid for n bytes.
            if unsafe { WriteFile(h.0, data[off..].as_ptr(), n, &mut done, core::ptr::null_mut()) } == 0 || done == 0 {
                return false;
            }
            off += done as usize;
        }
        // SAFETY: a valid handle.
        if unsafe { FlushFileBuffers(h.0) } == 0 {
            return false;
        }
        drop(h);
        let dst = lp(path);
        // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
        // SAFETY: valid wide paths.
        let ok = unsafe { MoveFileExW(w.as_ptr(), dst.as_ptr(), 1 | 8) } != 0;
        if !ok {
            crate::delete_raw(&tmp);
        }
        ok
    }

    pub fn mkdir_private(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat(path).is_some_and(|m| m.is_dir()) {
            return true;
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !up.ends_with(':') && !mkdir_private(up) {
            return false;
        }
        let w = lp(path);
        let Some((sa, sd)) = private_sa() else { return false };
        // SAFETY: valid path and security attributes.
        let ok = unsafe { CreateDirectoryW(w.as_ptr(), &sa as *const SecurityAttributes as usize) } != 0;
        // SAFETY: as above.
        unsafe { LocalFree(sd as *mut core::ffi::c_void) };
        ok || stat(path).is_some_and(|m| m.is_dir())
    }

    pub fn local_zone_id() -> Option<String> {
        let mut b = [0u8; 432];
        // SAFETY: DYNAMIC_TIME_ZONE_INFORMATION is 432 bytes.
        let r = unsafe { GetDynamicTimeZoneInformation(b.as_mut_ptr()) };
        if r == u32::MAX {
            return None;
        }
        let w: Vec<u16> = b[172..428].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        if w.is_empty() {
            return None;
        }
        Some(alloc::format!("win:{}", String::from_utf16_lossy(&w)))
    }
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
mod lnx {
    use super::{DirEntry, FileKind, Meta};
    use alloc::string::String;
    use alloc::vec::Vec;
    use rt::os::{syscall3, syscall6};

    const AT_FDCWD: isize = -100;
    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
    const O_RDONLY: usize = 0;
    const O_WRONLY: usize = 1;
    const O_CREAT: usize = 0x40;
    const O_TRUNC: usize = 0x200;
    const O_DIRECTORY: usize = 0x10000;
    const O_CLOEXEC: usize = 0x80000;
    const O_NOFOLLOW: usize = 0x20000;
    const O_NONBLOCK: usize = 0x800;
    const SYS_FCHMOD: usize = 91;
    const SYS_WRITE: usize = 1;
    const SYS_CLOSE: usize = 3;
    const SYS_FSYNC: usize = 74;
    const SYS_RENAME: usize = 82;
    const SYS_MKDIR: usize = 83;
    const SYS_STATFS: usize = 137;
    const SYS_GETDENTS64: usize = 217;
    const SYS_OPENAT: usize = 257;
    const SYS_FCHOWNAT: usize = 260;
    const SYS_NEWFSTATAT: usize = 262;
    const SYS_UNLINK: usize = 87;
    const SYS_LINKAT: usize = 265;
    const SYS_SYMLINKAT: usize = 266;
    const SYS_READLINKAT: usize = 267;
    const SYS_UTIMENSAT: usize = 280;
    const EINTR: isize = 4;

    fn z(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    struct Fd(usize);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: an owned descriptor.
            unsafe { syscall3(SYS_CLOSE, self.0, 0, 0) };
        }
    }

    fn open(path: &str, flags: usize, mode: usize) -> Option<Fd> {
        let p = z(path);
        loop {
            // SAFETY: a NUL-terminated path.
            let r = unsafe { syscall6(SYS_OPENAT, AT_FDCWD as usize, p.as_ptr() as usize, flags | O_CLOEXEC, mode, 0, 0) };
            if r == -EINTR {
                continue;
            }
            return (r >= 0).then_some(Fd(r as usize));
        }
    }

    fn rd_u64(b: &[u8], o: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[o..o + 8]);
        u64::from_ne_bytes(a)
    }

    fn rd_u32(b: &[u8], o: usize) -> u32 {
        u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    fn kind_of_mode(mode: u32) -> FileKind {
        match mode & 0o170000 {
            0o100000 => FileKind::File,
            0o040000 => FileKind::Dir,
            0o120000 => FileKind::Symlink,
            _ => FileKind::Other,
        }
    }

    fn statx(path: &str, flags: usize) -> Option<Meta> {
        let p = z(path);
        let mut b = [0u8; 144];
        // SAFETY: struct stat is 144 bytes on x86_64.
        let r = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD as usize, p.as_ptr() as usize, b.as_mut_ptr() as usize, flags, 0, 0) };
        if r < 0 {
            return None;
        }
        let t = |o: usize| (rd_u64(&b, o) as i64).saturating_mul(1_000_000_000).saturating_add(rd_u64(&b, o + 8) as i64);
        let mode = rd_u32(&b, 24);
        Some(Meta {
            kind: Some(kind_of_mode(mode)),
            size: rd_u64(&b, 48),
            atime_ns: t(72),
            mtime_ns: t(88),
            ctime_ns: t(104),
            mode,
            uid: rd_u32(&b, 28),
            gid: rd_u32(&b, 32),
            attrs: 0,
            dev: rd_u64(&b, 0),
            ino: rd_u64(&b, 8),
            nlink: rd_u64(&b, 16) as u32,
        })
    }

    pub fn stat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        statx(path, 0)
    }

    pub fn lstat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        statx(path, AT_SYMLINK_NOFOLLOW)
    }

    pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let fd = open(path, O_RDONLY | O_DIRECTORY, 0)?;
        let mut out = Vec::new();
        let mut buf = alloc::vec![0u8; 32 * 1024];
        loop {
            // SAFETY: the buffer is writable for its length.
            let n = unsafe { syscall3(SYS_GETDENTS64, fd.0, buf.as_mut_ptr() as usize, buf.len()) };
            if n == -EINTR {
                continue;
            }
            if n < 0 {
                return None;
            }
            if n == 0 {
                break;
            }
            let mut o = 0usize;
            while o + 19 <= n as usize {
                let reclen = u16::from_ne_bytes([buf[o + 16], buf[o + 17]]) as usize;
                if reclen == 0 {
                    break;
                }
                let ty = buf[o + 18];
                let name_bytes = &buf[o + 19..o + reclen];
                let end = name_bytes.iter().position(|&c| c == 0).unwrap_or(name_bytes.len());
                let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
                if name != "." && name != ".." {
                    let kind = match ty {
                        4 => FileKind::Dir,
                        8 => FileKind::File,
                        10 => FileKind::Symlink,
                        0 => lstat(&super::join(path, &name)).and_then(|m| m.kind).unwrap_or(FileKind::Other),
                        _ => FileKind::Other,
                    };
                    out.push(DirEntry { name, kind });
                }
                o += reclen;
            }
        }
        Some(out)
    }

    pub fn read_link(path: &str) -> Option<String> {
        if !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
            return None;
        }
        read_link_raw(path)
    }

    // read_link without the policy check (the system files of local_zone_id)
    fn read_link_raw(path: &str) -> Option<String> {
        let p = z(path);
        let mut buf = alloc::vec![0u8; 4096];
        // SAFETY: the buffer is writable.
        let r = unsafe { syscall6(SYS_READLINKAT, AT_FDCWD as usize, p.as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0) };
        if r < 0 {
            return None;
        }
        buf.truncate(r as usize);
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    pub fn symlink(target: &str, link: &str, _dir: bool) -> bool {
        if !(crate::policy::fs_allowed(link, crate::policy::FS_WRITE)) {
            return false;
        }
        let (t, l) = (z(target), z(link));
        // SAFETY: NUL-terminated paths.
        unsafe { syscall3(SYS_SYMLINKAT, t.as_ptr() as usize, AT_FDCWD as usize, l.as_ptr() as usize) >= 0 }
    }

    pub fn hard_link(existing: &str, new: &str) -> bool {
        if !(crate::policy::fs_allowed(existing, crate::policy::FS_READ) && crate::policy::fs_allowed(new, crate::policy::FS_WRITE)) {
            return false;
        }
        let (e, n) = (z(existing), z(new));
        // SAFETY: NUL-terminated paths.
        unsafe { syscall6(SYS_LINKAT, AT_FDCWD as usize, e.as_ptr() as usize, AT_FDCWD as usize, n.as_ptr() as usize, 0, 0) >= 0 }
    }

    pub fn set_times(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let p = z(path);
        let ts = [atime_ns.div_euclid(1_000_000_000), atime_ns.rem_euclid(1_000_000_000), mtime_ns.div_euclid(1_000_000_000), mtime_ns.rem_euclid(1_000_000_000)];
        // SAFETY: two timespecs; the link itself (AT_SYMLINK_NOFOLLOW).
        unsafe { syscall6(SYS_UTIMENSAT, AT_FDCWD as usize, p.as_ptr() as usize, ts.as_ptr() as usize, AT_SYMLINK_NOFOLLOW, 0, 0) >= 0 }
    }

    /// permission bits (07777); a symbolic link is left alone and never followed (the entry is
    /// opened with O_NOFOLLOW and changed through that descriptor, so a link swapped in
    /// meanwhile cannot redirect the change)
    pub fn set_mode(path: &str, mode: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        match open(path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK, 0) {
            // SAFETY: an owned descriptor.
            Some(fd) => unsafe { syscall3(SYS_FCHMOD, fd.0, (mode & 0o7777) as usize, 0) >= 0 },
            None => false,
        }
    }

    pub fn set_owner(path: &str, uid: u32, gid: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path; the link itself.
        unsafe { syscall6(SYS_FCHOWNAT, AT_FDCWD as usize, p.as_ptr() as usize, uid as usize, gid as usize, AT_SYMLINK_NOFOLLOW, 0) >= 0 }
    }

    pub fn set_attrs(_path: &str, _attrs: u32) -> bool {
        true
    }

    pub fn free_space(path: &str) -> Option<(u64, u64)> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let p = z(path);
        let mut b = [0u8; 120];
        // SAFETY: struct statfs is 120 bytes on x86_64.
        if unsafe { syscall3(SYS_STATFS, p.as_ptr() as usize, b.as_mut_ptr() as usize, 0) } < 0 {
            return None;
        }
        let bsize = rd_u64(&b, 8);
        let frsize = rd_u64(&b, 72);
        let unit = if frsize != 0 { frsize } else { bsize };
        Some((rd_u64(&b, 32).saturating_mul(unit), rd_u64(&b, 16).saturating_mul(unit)))
    }

    pub fn sync_dir(dir: &str) -> bool {
        if !(crate::policy::fs_allowed(dir, crate::policy::FS_WRITE)) {
            return false;
        }
        let Some(fd) = open(if dir.is_empty() { "." } else { dir }, O_RDONLY | O_DIRECTORY, 0) else { return false };
        // SAFETY: an owned descriptor.
        unsafe { syscall3(SYS_FSYNC, fd.0, 0, 0) >= 0 }
    }

    pub fn write_file_private(path: &str, data: &[u8]) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let tmp = alloc::format!("{}.tmp{}", path, crate::time_us() % 1_000_000);
        let Some(fd) = open(&tmp, O_WRONLY | O_CREAT | O_TRUNC, 0o600) else { return false };
        let mut off = 0;
        while off < data.len() {
            // SAFETY: the slice is readable for its length.
            let n = unsafe { syscall3(SYS_WRITE, fd.0, data[off..].as_ptr() as usize, (data.len() - off).min(1 << 30)) };
            if n == -EINTR {
                continue;
            }
            if n <= 0 {
                drop(fd);
                unlink(&tmp);
                return false;
            }
            off += n as usize;
        }
        // SAFETY: an owned descriptor.
        let synced = unsafe { syscall3(SYS_FSYNC, fd.0, 0, 0) } >= 0;
        drop(fd);
        let (t, d) = (z(&tmp), z(path));
        // SAFETY: NUL-terminated paths.
        if !synced || unsafe { syscall3(SYS_RENAME, t.as_ptr() as usize, d.as_ptr() as usize, 0) } < 0 {
            unlink(&tmp);
            return false;
        }
        sync_dir(super::parent(path))
    }

    fn unlink(path: &str) {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { syscall3(SYS_UNLINK, p.as_ptr() as usize, 0, 0) };
    }

    pub fn mkdir_private(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat(path).is_some_and(|m| m.is_dir()) {
            return true;
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !mkdir_private(up) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { syscall3(SYS_MKDIR, p.as_ptr() as usize, 0o700, 0) };
        r >= 0 || stat(path).is_some_and(|m| m.is_dir())
    }

    pub fn local_zone_id() -> Option<String> {
        if let Some(tz) = crate::env("TZ") {
            let tz = tz.trim_start_matches(':');
            if let Some(i) = tz.find("zoneinfo/") {
                return Some(String::from(&tz[i + 9..]));
            }
            if !tz.is_empty() && !tz.starts_with('/') {
                return Some(String::from(tz));
            }
        }
        if let Some(t) = read_link_raw("/etc/localtime") {
            if let Some(i) = t.find("zoneinfo/") {
                return Some(String::from(&t[i + 9..]));
            }
        }
        crate::read_file_raw("/etc/timezone").map(|b| String::from(String::from_utf8_lossy(&b).trim())).filter(|s| !s.is_empty())
    }

    /// the \\?\ form exists on Windows only
    pub fn long_path(path: &str) -> String {
        String::from(path)
    }
}
