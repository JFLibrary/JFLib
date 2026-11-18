// fs -- file system access for backup and restore
//
//   stat(path) / lstat(path) -> Option<Meta>     kind, size, times (ns), mode / attributes,
//                                                 owner, device and file id, link count
//   read_dir(path) -> Option<Vec<DirEntry>>      names and kinds ("." / ".." left out); None
//                                                also when an error cut the listing short
//   read_link / symlink(target, link, dir) / hard_link(existing, new)
//   set_times(path, atime_ns, mtime_ns)          on the link itself, never its target
//   set_mode(path, mode) (Linux) / set_attrs(path, attrs) (Windows) / set_owner (Linux)
//   free_space(path) -> Option<(free, total)>    sync_dir(dir) (Linux: fsync of the directory)
//   write_file_private(path, data)               owner-only file (0600 / protected DACL),
//                                                fsync, atomic rename, directory sync; its
//                                                sibling is always a new file of its own
//   join(dir, name) / parent(path)               '' separates on Windows only (and in vfs
//                                                names); on Linux and macOS it is a character
//   mkdir_private(path)                          owner-only directory (with parents); an
//                                                existing one must be a folder (no link) of
//                                                this user (Windows: also SYSTEM's or the
//                                                administrators') nobody else may write to
//                                                (Windows: has any access to), else false;
//                                                Linux, macOS: one others may only read is
//                                                made owner-only
//   local_zone_id() -> "Europe/Berlin" | "win:W. Europe Standard Time"
//   long_path(path) (Windows): the \\?\ form every call here uses
//   Dir                                          an open folder: entries below it reached
//                                                relative to it, never through a link (the
//                                                section "folders by handle")
//   set_file_times / set_file_mode (File)        through the open file's own handle
//   fd_path(File) (Linux)                        a name of the open file itself for a program
//                                                this one starts (/proc/<pid>/fd/<n>)
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

// the separators of a path: '/' and '\' on Windows and in vfs names (a mount turns '\' into
// '/'); '/' alone in other OS paths ('\' is an ordinary character of a Linux or macOS name)
fn seps(path: &str) -> &'static [char] {
    if cfg!(windows) || vfs::has_scheme(path) { &['/', '\\'] } else { &['/'] }
}

/// "a/b" + "c" with the platform separator (Linux, macOS: a '\' is part of a name)
pub fn join(dir: &str, name: &str) -> String {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let mut s = String::from(dir);
    if !s.is_empty() && !s.ends_with(seps(dir)) {
        s.push(sep);
    }
    s.push_str(name);
    s
}

/// the parent directory of a path ("" for a bare name; Linux, macOS: a '\' is part of a name)
pub fn parent(path: &str) -> &str {
    let sp = seps(path);
    let t = path.trim_end_matches(sp);
    match t.rfind(sp) {
        Some(0) => &t[..1],
        Some(i) => &t[..i],
        None => "",
    }
}

#[cfg(windows)]
pub use win::*;
#[cfg(target_os = "linux")]
pub use lnx::*;
#[cfg(target_vendor = "apple")]
pub use apl::*;
#[cfg(windows)]
use win as sys;
#[cfg(target_os = "linux")]
use lnx as sys;
#[cfg(target_vendor = "apple")]
use apl as sys;

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
/// stands for its OS path.  Only on OS paths and folder mounts; never a target with a NUL
/// character (the link would lead to the name before it).  With a manifest the app needs write
/// access to link and to target (a relative one counted from link's folder): the rules check a
/// path as it is written, so whatever may use the link reaches the target with the link's rights.
pub fn symlink(target: &str, link: &str, dir: bool) -> bool {
    if target.contains('\0') {
        return false;
    }
    let t = match vfs::route(target) {
        Route::Os => String::from(target),
        _ => match vfs::os_path(target) {
            Some(t) => t,
            None => return false,
        },
    };
    match at(link) {
        At::Os(o) => {
            let l = o.as_deref().unwrap_or(link);
            link_target_allowed(&t, l) && sys::symlink(&t, l, dir)
        }
        _ => false,
    }
}

// the number of write_file_private's first sibling NAME.tmpN (the next ones count up); (tests)
// jfgui_TEST_PRIVATE_TMP sets it
fn tmp_seed() -> u64 {
    crate::env("jfgui_TEST_PRIVATE_TMP").and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(crate::time_us)
}

/// symlink's target as the link leads to it (a relative one from the link's folder) may be
/// written: the link's own path may be, and the rules do not follow links
fn link_target_allowed(target: &str, link: &str) -> bool {
    let Some(l) = crate::policy::normalize(link) else { return false };
    let mut base = String::from(parent(&l));
    if base.ends_with(':') {
        base.push('\\'); // a link at the root of a drive
    }
    match crate::policy::normalize_for(target, cfg!(windows), &base) {
        Some(t) => crate::policy::fs_allowed(&t, crate::policy::FS_WRITE),
        // ".." (the system resolves it after links, the rules cannot): refused under rules,
        // as such a path is everywhere; without rules every target may be written
        None => !target.is_empty() && !target.contains('\0') && crate::policy::fs_unrestricted(),
    }
}

/// (net) (device, file id) of an OS path when it is a socket file (Windows: an AF_UNIX reparse
/// point), a link not followed; without the policy check (net checked fs.write)
pub(crate) fn socket_id(path: &str) -> Option<(u64, u64)> {
    if path.contains('\0') {
        return None;
    }
    sys::socket_id(path)
}

/// a second name new for the file existing (both OS paths or folder mounts); with a manifest
/// the app needs write access to both (writing through new changes existing)
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

/// an owner-only file (0600 / protected DACL), synced and renamed into place (the sibling
/// NAME.tmpN is created new, never an existing file taken over); a mount without OS files: its
/// write (memory of this process)
pub fn write_file_private(path: &str, data: &[u8]) -> bool {
    match at(path) {
        At::Os(o) => sys::write_file_private(o.as_deref().unwrap_or(path), data),
        At::Mount(p, r) => !r.is_empty() && p.write(&r, data),
        At::Bad => false,
    }
}

/// an owner-only folder (with parents; existing parents are taken as they are).  An existing
/// folder must be the folder itself (no link or junction), belong to this user (Windows: or to
/// SYSTEM or the administrators) and let nobody else write to it (Windows: grant nobody else
/// anything), else false: someone else's folder, made before the program, may hold planted files.
/// Linux and macOS make one of this user that others may only read owner-only (0700).
pub fn mkdir_private(path: &str) -> bool {
    match at(path) {
        At::Os(o) => sys::mkdir_private(o.as_deref().unwrap_or(path)),
        At::Mount(p, r) => p.mkdir(&r),
        At::Bad => false,
    }
}

// ------------------------------------------------------------------ folders by handle

/// An open folder: what lies below it is reached relative to it and never through a link, so
/// a folder on the way that is swapped for a link (or renamed) after a check cannot lead a
/// privileged read, write, change or removal out of it.
///
///   Dir::open(path)                   the folder as it is (a link to it is followed: the
///                                     trusted start, e.g. a shared folder); OS paths and
///                                     folder mounts only
///   open_dir(rel)                     a folder below ("a/b"; no "..", nothing absolute) with no
///                                     link on the way: Linux openat2 (RESOLVE_BENEATH |
///                                     RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS), else openat
///                                     with O_NOFOLLOW for every component (kernels before 5.6,
///                                     a seccomp filter that refuses openat2)
///   names                             one component: no separator, not "." or ".." (Windows:
///                                     no ':' either); "" is the folder itself where noted
///   lstat / read_link / read_dir      links are reported, never followed
///   open_file(name)                   a regular file, read only; a link or anything else: None
///   create_new(name)                  a new file (Linux, macOS 0600), never an existing name
///   mkdir / remove / rename / symlink a link is removed or renamed as the link
///   set_times / set_mode / set_attrs  on the entry itself, never on a link's target
///   set_file_times / set_file_mode    through an open File's own handle (whatever its name
///                                     has become meanwhile)
///
/// macOS: openat with O_NOFOLLOW component by component, the *at calls on the folder.  Windows:
/// the native calls relative to an open folder -- NtCreateFile with OBJECT_ATTRIBUTES.
/// RootDirectory = the folder's handle and FILE_OPEN_REPARSE_POINT, one name at a time (a
/// folder on the way that is a junction, a symbolic link or another name surrogate is refused,
/// one that is no folder too), renames to a name relative to the target folder's handle
/// (FILE_RENAME_INFORMATION.RootDirectory), removals through the entry's own handle, links as
/// reparse data set on the new entry (FSCTL_SET_REPARSE_POINT, what CreateSymbolicLinkW writes).
/// What lies below an open folder is found through the handles, never through a path, so a
/// folder on the way renamed or swapped for a junction -- also one another program holds open
/// with delete access -- changes nothing, and no folder is locked against renaming or removal
/// meanwhile.  On a network share the server looks the names up by path itself.  The
/// manifest's rules (policy.rs) are checked on the path an entry has below the start.
pub struct Dir {
    path: String,
    os: sys::OsDir,
}

// the separators of an OS path
fn os_seps() -> &'static [char] {
    if cfg!(windows) { &['/', '\\'] } else { &['/'] }
}

// one component of a path below a Dir
fn one_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(os_seps()) && !name.contains('\0') && !(cfg!(windows) && name.contains(':'))
}

impl Dir {
    /// the folder `path` as it is (a link to it is followed); None when it is no folder
    pub fn open(path: &str) -> Option<Dir> {
        let At::Os(o) = at(path) else { return None };
        let p = o.unwrap_or_else(|| String::from(path));
        if p.is_empty() || p.contains('\0') || !crate::policy::fs_allowed(&p, crate::policy::FS_READ) {
            return None;
        }
        Some(Dir { os: sys::OsDir::open(&p)?, path: p })
    }

    /// the path the folder was opened as (with the names below its start)
    pub fn path(&self) -> &str {
        &self.path
    }

    /// a folder below this one ("a/b"): no component may be a link (or a junction), none may
    /// be ".." and the path may not be absolute
    pub fn open_dir(&self, rel: &str) -> Option<Dir> {
        let names: Vec<&str> = rel.split(os_seps()).filter(|n| !n.is_empty()).collect();
        if names.is_empty() || rel.starts_with(os_seps()) || !names.iter().all(|n| one_name(n)) {
            return None;
        }
        let mut path = self.path.clone();
        for n in &names {
            path = join(&path, n);
        }
        if !crate::policy::fs_allowed(&path, crate::policy::FS_READ) {
            return None;
        }
        Some(Dir { os: self.os.open_dir(&names)?, path })
    }

    // the path of an entry ("": the folder itself when `own`) if the rules allow `access`
    fn entry(&self, name: &str, own: bool, access: u32) -> Option<String> {
        let p = if own && name.is_empty() {
            self.path.clone()
        } else if one_name(name) {
            join(&self.path, name)
        } else {
            return None;
        };
        crate::policy::fs_allowed(&p, access).then_some(p)
    }

    /// the metadata of an entry itself ("": the folder), a link not followed
    pub fn lstat(&self, name: &str) -> Option<Meta> {
        self.entry(name, true, crate::policy::FS_READ)?;
        self.os.lstat(name)
    }

    /// the target of a symbolic link (Windows: also a junction)
    pub fn read_link(&self, name: &str) -> Option<String> {
        self.entry(name, false, crate::policy::FS_READ)?;
        self.os.read_link(name)
    }

    /// the entries of the folder ("." / ".." left out); None also when an error cut it short
    pub fn read_dir(&self) -> Option<Vec<DirEntry>> {
        self.entry("", true, crate::policy::FS_READ)?;
        self.os.read_dir()
    }

    /// a regular file, read only (never through a link; a folder, device or pipe: None)
    pub fn open_file(&self, name: &str) -> Option<crate::File> {
        self.entry(name, false, crate::policy::FS_READ)?;
        self.os.open_file(name)
    }

    /// a new file, read and write (Linux, macOS: 0600); an existing name, also a link: None
    pub fn create_new(&self, name: &str) -> Option<crate::File> {
        self.entry(name, false, crate::policy::FS_WRITE)?;
        self.os.create_new(name)
    }

    /// a new folder (Linux, macOS: 0777 less the umask); false also when the name exists
    pub fn mkdir(&self, name: &str) -> bool {
        self.entry(name, false, crate::policy::FS_WRITE).is_some() && self.os.mkdir(name)
    }

    /// a file, a link (never what it points to) or an empty folder
    pub fn remove(&self, name: &str) -> bool {
        self.entry(name, false, crate::policy::FS_WRITE).is_some() && self.os.remove(name)
    }

    /// rename `name` to `to_name` in `to` (this folder or another Dir on the same volume),
    /// replacing an existing file there; a link is renamed as the link
    pub fn rename(&self, name: &str, to: &Dir, to_name: &str) -> bool {
        self.entry(name, false, crate::policy::FS_WRITE).is_some() && to.entry(to_name, false, crate::policy::FS_WRITE).is_some() && self.os.rename(name, &to.os, to_name)
    }

    /// a symbolic link `name` to `target` (dir: a folder's link on Windows); the rules as for
    /// symlink: the target as the link leads to it must be writable
    pub fn symlink(&self, target: &str, name: &str, dir: bool) -> bool {
        let Some(l) = self.entry(name, false, crate::policy::FS_WRITE) else { return false };
        !target.is_empty() && !target.contains('\0') && link_target_allowed(target, &l) && self.os.symlink(target, name, dir)
    }

    /// access and modification times (ns since 1970) of an entry itself ("": the folder)
    pub fn set_times(&self, name: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        self.entry(name, true, crate::policy::FS_WRITE).is_some() && self.os.set_times(name, atime_ns, mtime_ns)
    }

    /// permission bits (07777) of an entry ("": the folder); a link is left alone (Windows:
    /// nothing to do)
    pub fn set_mode(&self, name: &str, mode: u32) -> bool {
        self.entry(name, true, crate::policy::FS_WRITE).is_some() && self.os.set_mode(name, mode)
    }

    /// FILE_ATTRIBUTE_* of an entry ("": the folder; Linux, macOS: nothing to do)
    pub fn set_attrs(&self, name: &str, attrs: u32) -> bool {
        self.entry(name, true, crate::policy::FS_WRITE).is_some() && self.os.set_attrs(name, attrs)
    }
}

/// access and modification times of an open file, through its handle; a mount's stream: false
pub fn set_file_times(f: &crate::File, atime_ns: i64, mtime_ns: i64) -> bool {
    f.os().is_some_and(|o| sys::set_file_times(o, atime_ns, mtime_ns))
}

/// permission bits (07777) of an open file, through its handle (Windows: nothing to do); a
/// mount's stream: false
pub fn set_file_mode(f: &crate::File, mode: u32) -> bool {
    f.os().is_some_and(|o| sys::set_file_mode(o, mode))
}

/// A path that leads a program of this user (one this process starts, such as QEMU given a disk
/// file made through a Dir) to the open file `f` itself, whatever its name has become or was put
/// in its place meanwhile.  Linux: /proc/<this process>/fd/<n>, valid while `f` stays open and
/// this process runs (keep `f` as long as the other program may open the path again); None on
/// Windows and macOS and for a mount's stream.
pub fn fd_path(f: &crate::File) -> Option<String> {
    sys::fd_path(f.os()?)
}

// ------------------------------------------------------------------ Windows

#[cfg(windows)]
mod win {
    use super::{DirEntry, FileKind, Meta};
    use alloc::string::String;
    use alloc::vec::Vec;

    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    const READ_CONTROL: u32 = 0x2_0000;
    const SE_FILE_OBJECT: u32 = 1;
    const OWNER_INFO: u32 = 1;
    const DACL_INFO: u32 = 4;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const SHARE_ALL: u32 = 7;
    const OPEN_EXISTING: u32 = 3;
    const CREATE_NEW: u32 = 1;
    const ERROR_NO_MORE_FILES: u32 = 18;
    const ERROR_FILE_EXISTS: u32 = 80;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const TEMP_TRIES: u32 = 16;
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
        fn GetLastError() -> u32;
        fn DeviceIoControl(h: isize, code: u32, inb: usize, inn: u32, outb: *mut u8, outn: u32, ret: *mut u32, ov: usize) -> i32;
        fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
        fn CreateHardLinkW(new: *const u16, existing: *const u16, sa: usize) -> i32;
        fn SetFileTime(h: isize, created: *const u64, accessed: *const u64, written: *const u64) -> i32;
        fn SetFileAttributesW(name: *const u16, attrs: u32) -> i32;
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: *mut crate::win::Overlapped) -> i32;
        fn FlushFileBuffers(h: isize) -> i32;
        fn CreateDirectoryW(name: *const u16, sa: usize) -> i32;
        fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GetDynamicTimeZoneInformation(tz: *mut u8) -> u32;
        fn GetFullPathNameW(name: *const u16, n: u32, buf: *mut u16, part: usize) -> u32;
        fn GetCurrentProcess() -> isize;
    }

    #[link(name = "advapi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, rev: u32, sd: *mut usize, len: *mut u32) -> i32;
        fn GetSecurityInfo(h: isize, kind: u32, info: u32, owner: *mut usize, group: *mut usize, dacl: *mut usize, sacl: *mut usize, sd: *mut usize) -> u32;
        fn ConvertSidToStringSidW(sid: usize, text: *mut *mut u16) -> i32;
        fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
        fn GetTokenInformation(token: isize, class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32;
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
        // (a FILETIME from 2^63 on, which SetFileTime takes, is clamped instead of turning negative)
        let t = i64::try_from((ft[1] as u64) << 32 | ft[0] as u64).unwrap_or(i64::MAX);
        t.saturating_sub(EPOCH_DIFF).saturating_mul(100)
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

    pub(crate) fn socket_id(path: &str) -> Option<(u64, u64)> {
        const TAG_AF_UNIX: u32 = 0x8000_0023;
        let h = open(path, FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT)?;
        let m = meta_of(&h)?;
        (m.attrs & ATTR_REPARSE != 0 && reparse_tag(&h) == TAG_AF_UNIX).then_some((m.dev, m.ino))
    }

    pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        read_dir_raw(path)
    }

    fn read_dir_raw(path: &str) -> Option<Vec<DirEntry>> {
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
        let end = loop {
            let n = d.name.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&d.name[..n]);
            if name != "." && name != ".." {
                out.push(DirEntry { kind: kind_of(d.attr, d.reserved[0]), name });
            }
            // SAFETY: the search handle and record from above; GetLastError right after the
            // failed call.
            if unsafe { FindNextFileW(h, &mut d) } == 0 {
                break unsafe { GetLastError() };
            }
        };
        // SAFETY: the search handle, closed once.
        unsafe { FindClose(h) };
        // only the end of the listing is a whole listing; another error leaves out what follows
        (end == ERROR_NO_MORE_FILES).then_some(out)
    }

    pub fn read_link(path: &str) -> Option<String> {
        if !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
            return None;
        }
        read_link_raw(path)
    }

    fn read_link_raw(path: &str) -> Option<String> {
        link_of(&open(path, FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT)?)
    }

    // the target of the symbolic link or junction an open handle (OPEN_REPARSE_POINT) is
    fn link_of(h: &H) -> Option<String> {
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
        if !(crate::policy::fs_allowed(existing, crate::policy::FS_WRITE) && crate::policy::fs_allowed(new, crate::policy::FS_WRITE)) {
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
        set_times_raw(path, atime_ns, mtime_ns)
    }

    fn set_times_raw(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
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
        set_attrs_raw(path, attrs)
    }

    fn set_attrs_raw(path: &str, attrs: u32) -> bool {
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

    /// A security descriptor: full access for SYSTEM, the administrators, the owner and the user
    /// this process runs as.  That user by name as well: in a process run as administrator a new
    /// object's owner is the administrators' group (an elevated token's default owner), so "the
    /// owner" alone would lock the same user out of it in a process without those rights.
    fn private_sa() -> Option<(SecurityAttributes, usize)> {
        let me = user_sid().map(|s| alloc::format!("(A;OICI;FA;;;{})", s)).unwrap_or_default();
        let sddl = wz(&alloc::format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW){}", me));
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
        // a new sibling of ours (CREATE_NEW: a file of that name -- someone else's, with its own
        // access rights -- is never taken over); only that one is written and removed again
        let Some((sa, sd)) = private_sa() else { return false };
        let mut made = None;
        let seed = super::tmp_seed();
        for k in 0..TEMP_TRIES {
            let tmp = alloc::format!("{}.tmp{}", path, (seed + k as u64) % 1_000_000);
            let w = lp(&tmp);
            // SAFETY: a valid path and security attributes; GetLastError right after the call.
            let (h, e) = unsafe {
                let h = CreateFileW(w.as_ptr(), GENERIC_WRITE, 0, &sa as *const SecurityAttributes as usize, CREATE_NEW, WRITE_THROUGH, 0);
                (h, GetLastError())
            };
            if h != INVALID {
                made = Some((H(h), tmp, w));
                break;
            }
            if e != ERROR_FILE_EXISTS && e != ERROR_ALREADY_EXISTS {
                break;
            }
        }
        // SAFETY: the descriptor from ConvertString..., no longer needed.
        unsafe { LocalFree(sd as *mut core::ffi::c_void) };
        let Some((h, tmp, w)) = made else { return false };
        let mut off = 0;
        let mut ok = true;
        while ok && off < data.len() {
            let n = (data.len() - off).min(1 << 30) as u32;
            let mut done = 0u32;
            // SAFETY: the buffer slice is valid for n bytes.
            ok = unsafe { WriteFile(h.0, data[off..].as_ptr(), n, &mut done, core::ptr::null_mut()) } != 0 && done != 0;
            off += done as usize;
        }
        // SAFETY: a valid handle.
        ok = ok && unsafe { FlushFileBuffers(h.0) } != 0;
        drop(h);
        if !ok {
            crate::delete_raw(&tmp);
            return false;
        }
        let dst = lp(path);
        // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH where no POSIX rename exists (win_sys)
        let ok = crate::replace_move(&w, &dst, 1 | 8);
        if !ok {
            crate::delete_raw(&tmp);
        }
        ok
    }

    pub fn mkdir_private(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if lstat(path).is_some() {
            return private_dir(path);
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !up.ends_with(':') && !make_parents(up) {
            return false;
        }
        create_private(path) || private_dir(path)
    }

    // the parents of mkdir_private's folder: an existing one is taken as it is (C:\ProgramData,
    // a share), new ones are private
    fn make_parents(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat(path).is_some_and(|m| m.is_dir()) {
            return true;
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !up.ends_with(':') && !make_parents(up) {
            return false;
        }
        create_private(path) || stat(path).is_some_and(|m| m.is_dir())
    }

    fn create_private(path: &str) -> bool {
        let w = lp(path);
        let Some((sa, sd)) = private_sa() else { return false };
        // SAFETY: valid path and security attributes.
        let ok = unsafe { CreateDirectoryW(w.as_ptr(), &sa as *const SecurityAttributes as usize) } != 0;
        // SAFETY: as above.
        unsafe { LocalFree(sd as *mut core::ffi::c_void) };
        ok
    }

    /// an existing folder mkdir_private hands out: a folder itself (no link or junction), owned
    /// by SYSTEM, the administrators or this user, whose DACL grants nobody else anything (someone
    /// else's folder, made before the program, may hold planted files or let them be read)
    fn private_dir(path: &str) -> bool {
        let Some(h) = open(path, READ_CONTROL | FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT) else { return false };
        if !meta_of(&h).is_some_and(|m| m.is_dir()) {
            return false;
        }
        let (mut owner, mut dacl, mut sd) = (0usize, 0usize, 0usize);
        // SAFETY: a handle with READ_CONTROL; owner and DACL point into sd, freed below.
        let e = unsafe { GetSecurityInfo(h.0, SE_FILE_OBJECT, OWNER_INFO | DACL_INFO, &mut owner, core::ptr::null_mut(), &mut dacl, core::ptr::null_mut(), &mut sd) };
        if e != 0 {
            return false;
        }
        let me = user_sid();
        let trusted = |s: &str| s == "S-1-5-18" || s == "S-1-5-32-544" || me.as_deref() == Some(s);
        // (a null DACL grants everyone everything)
        let ok = sid_text(owner).is_some_and(|s| trusted(&s))
            && dacl != 0
            // CREATOR OWNER (S-1-3-0) and OWNER RIGHTS (S-1-3-4) stand for the owner checked here
            // and for whoever may create entries
            && grants_only(dacl, &|s| trusted(s) || s == "S-1-3-0" || s == "S-1-3-4");
        // SAFETY: the descriptor from GetSecurityInfo, no longer used.
        unsafe { LocalFree(sd as *mut core::ffi::c_void) };
        ok
    }

    /// whether every allowing ACE of the ACL at acl names a SID ok() accepts (or grants nothing)
    fn grants_only(acl: usize, ok: &dyn Fn(&str) -> bool) -> bool {
        // SAFETY: an ACL from GetSecurityInfo: its 8-byte header holds its size (AclSize).
        let size = unsafe { core::ptr::read_unaligned((acl + 2) as *const u16) } as usize;
        if size < 8 {
            return false;
        }
        // SAFETY: the ACL is AclSize bytes long.
        let b = unsafe { core::slice::from_raw_parts(acl as *const u8, size) };
        let count = u16::from_le_bytes([b[4], b[5]]) as usize;
        let mut off = 8;
        for _ in 0..count {
            if off + 4 > size {
                return false;
            }
            let len = u16::from_le_bytes([b[off + 2], b[off + 3]]) as usize;
            if len < 4 || off + len > size {
                return false;
            }
            match b[off] {
                // ACCESS_ALLOWED_ACE and ACCESS_ALLOWED_CALLBACK_ACE: header, mask, SID
                0 | 9 => {
                    if len < 16 {
                        return false;
                    }
                    let mask = u32::from_le_bytes([b[off + 4], b[off + 5], b[off + 6], b[off + 7]]);
                    if mask != 0 && !sid_text(b[off + 8..].as_ptr() as usize).is_some_and(|s| ok(&s)) {
                        return false;
                    }
                }
                // the object ACEs (directory objects, not files): never trusted
                5 | 11 => return false,
                _ => {}
            }
            off += len;
        }
        true
    }

    /// "S-1-5-..." of a SID
    fn sid_text(sid: usize) -> Option<String> {
        let mut p: *mut u16 = core::ptr::null_mut();
        // SAFETY: a SID from the system; the text is freed with LocalFree below.
        if sid == 0 || unsafe { ConvertSidToStringSidW(sid, &mut p) } == 0 || p.is_null() {
            return None;
        }
        let mut n = 0;
        // SAFETY: a NUL-terminated string from ConvertSidToStringSidW.
        while unsafe { *p.add(n) } != 0 {
            n += 1;
        }
        // SAFETY: n characters before the NUL.
        let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) });
        // SAFETY: allocated by ConvertSidToStringSidW.
        unsafe { LocalFree(p as *mut core::ffi::c_void) };
        Some(s)
    }

    /// the SID of the user this process runs as
    fn user_sid() -> Option<String> {
        let mut t = 0isize;
        // SAFETY: TOKEN_QUERY on the own process's token (the pseudo handle needs no closing).
        if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut t) } == 0 {
            return None;
        }
        let t = H(t);
        let mut buf = [0usize; 64]; // TOKEN_USER (the SID's pointer first) and the SID
        let mut n = 0u32;
        // SAFETY: the buffer holds the given number of bytes; TokenUser = 1.
        if unsafe { GetTokenInformation(t.0, 1, buf.as_mut_ptr() as *mut u8, core::mem::size_of_val(&buf) as u32, &mut n) } == 0 {
            return None;
        }
        sid_text(buf[0])
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

    // ---- Dir: every entry opened relative to the handle of its folder

    #[link(name = "ntdll", kind = "raw-dylib")]
    unsafe extern "system" {
        fn NtCreateFile(h: *mut isize, access: u32, oa: *const ObjAttrs, io: *mut [usize; 2], alloc: *const i64, attrs: u32, share: u32, disp: u32, opts: u32, ea: *const u8, ea_len: u32) -> i32;
        fn NtSetInformationFile(h: isize, io: *mut [usize; 2], info: *const u8, len: u32, class: u32) -> i32;
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(h: isize, class: u32, info: *const u8, size: u32) -> i32;
        fn GetCurrentThread() -> isize;
    }

    #[link(name = "advapi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn ImpersonateSelf(level: u32) -> i32;
        fn RevertToSelf() -> i32;
        fn OpenThreadToken(thread: isize, access: u32, as_self: i32, token: *mut isize) -> i32;
        fn AdjustTokenPrivileges(token: isize, disable_all: i32, new: *const u8, len: u32, old: *mut u8, ret: *mut u32) -> i32;
    }

    /// UNICODE_STRING (lengths in bytes)
    #[repr(C)]
    struct UnicodeStr {
        len: u16,
        max: u16,
        buf: *const u16,
    }

    /// OBJECT_ATTRIBUTES: a name relative to the folder `root`
    #[repr(C)]
    struct ObjAttrs {
        len: u32,
        root: isize,
        name: *const UnicodeStr,
        attrs: u32,
        sd: usize,
        qos: usize,
    }

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_LIST_DIRECTORY: u32 = 1;
    const DELETE: u32 = 0x1_0000;
    const SYNCHRONIZE: u32 = 0x10_0000;
    const SHARE_READ: u32 = 1;
    const ATTR_NORMAL: u32 = 0x80;
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    // NtCreateFile: disposition and options
    const FILE_OPEN: u32 = 1;
    const FILE_CREATE: u32 = 2;
    const FILE_DIRECTORY_FILE: u32 = 0x1;
    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
    const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x4000;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x20_0000;
    // a reparse point that names another place (symbolic link, junction, ...)
    const TAG_NAME_SURROGATE: u32 = 0x2000_0000;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    // FILE_INFORMATION_CLASS (NtSetInformationFile) and FILE_INFO_BY_HANDLE_CLASS
    const FILE_RENAME_INFORMATION: u32 = 10;
    const FILE_RENAME_INFORMATION_EX: u32 = 65;
    const FILE_BASIC_INFO: u32 = 0;
    const FILE_DISPOSITION_INFO: u32 = 4;
    const FILE_FULL_DIR_INFO: u32 = 14;
    const FILE_DISPOSITION_INFO_EX: u32 = 21;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
    const SE_CREATE_SYMBOLIC_LINK_PRIVILEGE: u32 = 35;
    const STATUS_NOT_IMPLEMENTED: i32 = 0xC000_0002_u32 as i32;
    const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003_u32 as i32;
    const STATUS_INVALID_PARAMETER: i32 = 0xC000_000D_u32 as i32;
    const STATUS_ACCESS_DENIED: i32 = 0xC000_0022_u32 as i32;
    const STATUS_OBJECT_NAME_INVALID: i32 = 0xC000_0033_u32 as i32;
    const STATUS_SHARING_VIOLATION: i32 = 0xC000_0043_u32 as i32;
    const STATUS_FILE_LOCK_CONFLICT: i32 = 0xC000_0054_u32 as i32;
    const STATUS_LOCK_NOT_GRANTED: i32 = 0xC000_0055_u32 as i32;
    const STATUS_NOT_SUPPORTED: i32 = 0xC000_00BB_u32 as i32;

    /// an open folder: its handle (FILE_READ_ATTRIBUTES); every entry below is opened relative
    /// to it, one name at a time
    pub struct OsDir(H);

    // `name` (one component; "": the folder itself once more) opened in the folder `root`:
    // NtCreateFile with OBJECT_ATTRIBUTES.RootDirectory, so the name is looked up in that very
    // folder, whatever path leads to it now (synchronous, as CreateFileW) -> else the NTSTATUS
    fn nt_open(root: isize, name: &str, access: u32, share: u32, disp: u32, opts: u32, attrs: u32) -> Result<H, i32> {
        let w: Vec<u16> = name.encode_utf16().collect();
        let bytes = u16::try_from(w.len() * 2).map_err(|_| STATUS_OBJECT_NAME_INVALID)?;
        let us = UnicodeStr { len: bytes, max: bytes, buf: if w.is_empty() { core::ptr::null() } else { w.as_ptr() } };
        let oa = ObjAttrs { len: core::mem::size_of::<ObjAttrs>() as u32, root, name: &us, attrs: OBJ_CASE_INSENSITIVE, sd: 0, qos: 0 };
        let (mut h, mut io) = (0isize, [0usize; 2]);
        // SAFETY: the records (and the name they point to) live during the call; no allocation
        // size, no extended attributes.
        let st = unsafe { NtCreateFile(&mut h, access | SYNCHRONIZE | FILE_READ_ATTRIBUTES, &oa, &mut io, core::ptr::null(), attrs, share, disp, opts | FILE_SYNCHRONOUS_IO_NONALERT, core::ptr::null(), 0) };
        if st < 0 { Err(st) } else { Ok(H(h)) }
    }

    // an existing entry of `root` as itself: a link (or junction) is opened as the link
    fn entry_h(root: isize, name: &str, access: u32, opts: u32) -> Result<H, i32> {
        nt_open(root, name, access, SHARE_ALL, FILE_OPEN, opts | FILE_OPEN_REPARSE_POINT | FILE_OPEN_FOR_BACKUP_INTENT, 0)
    }

    // the folder `name` in `root`: no link, junction or other name surrogate, and a folder;
    // another reparse point (a cloud folder) through its driver when that is the same folder
    fn sub_dir(root: isize, name: &str) -> Option<H> {
        let h = entry_h(root, name, 0, FILE_DIRECTORY_FILE).ok()?;
        let m = meta_of(&h)?;
        if !m.is_dir() || m.attrs & ATTR_REPARSE != 0 && reparse_tag(&h) & TAG_NAME_SURROGATE != 0 {
            return None;
        }
        if m.attrs & ATTR_REPARSE == 0 {
            return Some(h);
        }
        match nt_open(root, name, 0, SHARE_ALL, FILE_OPEN, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT, 0) {
            Ok(f) => meta_of(&f).is_some_and(|n| n.is_dir() && (n.dev, n.ino) == (m.dev, m.ino)).then_some(f),
            Err(_) => Some(h), // no driver for it: the folder itself
        }
    }

    // the entry of an open handle (DELETE) removed: at once (POSIX semantics) where the file
    // system can, else when the last handle closes; a folder only when it is empty
    fn delete_h(h: &H) -> bool {
        let ex: u32 = 1 | 2; // FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
        let del: u32 = 1; // FILE_DISPOSITION_INFO.DeleteFile
        // SAFETY: 4-byte FILE_DISPOSITION_INFO_EX / FILE_DISPOSITION_INFO records.
        unsafe {
            SetFileInformationByHandle(h.0, FILE_DISPOSITION_INFO_EX, &ex as *const u32 as *const u8, 4) != 0
                || SetFileInformationByHandle(h.0, FILE_DISPOSITION_INFO, &del as *const u32 as *const u8, 4) != 0
        }
    }

    // the entry of an open handle (DELETE) renamed to `name` in the folder `root`, replacing a
    // file there (POSIX semantics where the file system has them) -> the NTSTATUS, and whether
    // it came from the plain rename (a file system without the POSIX one)
    fn rename_h(h: &H, root: isize, name: &str) -> Result<(), (i32, bool)> {
        let w: Vec<u16> = name.encode_utf16().collect();
        let n = w.len() * 2;
        // FILE_RENAME_INFORMATION: Flags (ReplaceIfExists), RootDirectory (at 8), FileNameLength
        // (bytes, at 16), FileName (at 20); at least the record's own size (24)
        let size = (20 + n + 2).max(24);
        let mut info = alloc::vec![0u64; size.div_ceil(8)];
        let p = info.as_mut_ptr() as *mut u8;
        // SAFETY: info holds size bytes, 8-byte aligned; the name is copied without overlap.
        unsafe {
            (p.add(8) as *mut isize).write(root);
            (p.add(16) as *mut u32).write(n as u32);
            core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, p.add(20), n);
        }
        let set = |flags: u32, class: u32| -> i32 {
            let mut io = [0usize; 2];
            // SAFETY: the record above, size bytes; the handle is open with DELETE.
            unsafe {
                (p as *mut u32).write(flags);
                NtSetInformationFile(h.0, &mut io, p, size as u32, class)
            }
        };
        // FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS
        let st = set(1 | 2, FILE_RENAME_INFORMATION_EX);
        if st >= 0 {
            return Ok(());
        }
        if !matches!(st, STATUS_INVALID_INFO_CLASS | STATUS_INVALID_PARAMETER | STATUS_NOT_SUPPORTED | STATUS_NOT_IMPLEMENTED) {
            return Err((st, false));
        }
        let st = set(1, FILE_RENAME_INFORMATION);
        if st >= 0 { Ok(()) } else { Err((st, true)) }
    }

    // the reparse data CreateSymbolicLinkW writes for `target`: a relative one (also "\x") as it
    // is, SYMLINK_FLAG_RELATIVE; an absolute one ("C:\x", "\\server\share\x", "\\?\...") as the
    // NT path "\??\..." with the name as given to show ("C:x" made absolute, shown so)
    fn symlink_data(target: &str) -> Option<Vec<u8>> {
        let b = target.as_bytes();
        let sep = |i: usize| b.get(i).is_some_and(|c| *c == b'\\' || *c == b'/');
        let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
        let (flags, sub, print) = if sep(0) && sep(1) && matches!(b.get(2), Some(b'?' | b'.')) && sep(3) {
            (0u32, alloc::format!(r"\??\{}", &target[4..]), String::from(target))
        } else if (sep(0) && sep(1)) || drive {
            // \\?\C:\... or \\?\UNC\server\...: absolute, the separators and dots as Windows has them
            let full = long_path(target);
            let rest = full.strip_prefix(r"\\?\")?;
            (0, alloc::format!(r"\??\{}", rest), if drive && !sep(2) { String::from(rest) } else { String::from(target) })
        } else {
            (1, String::from(target), String::from(target))
        };
        let (p, s): (Vec<u16>, Vec<u16>) = (print.encode_utf16().collect(), sub.encode_utf16().collect());
        let (pn, sn) = (p.len() * 2, s.len() * 2);
        let len = u16::try_from(12 + pn + sn).ok().filter(|l| *l as usize + 8 <= 16 * 1024)?;
        // REPARSE_DATA_BUFFER: tag, data length, reserved; substitute name offset and length,
        // print name offset and length (the print name first in the buffer), flags; the names
        let mut v = Vec::with_capacity(8 + len as usize);
        v.extend_from_slice(&TAG_SYMLINK.to_le_bytes());
        for x in [len, 0, pn as u16, sn as u16, 0, pn as u16] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&flags.to_le_bytes());
        for c in p.iter().chain(s.iter()) {
            v.extend_from_slice(&c.to_le_bytes());
        }
        Some(v)
    }

    // a link made as `name` in the folder `root`: the new entry, then its reparse data; the
    // entry is removed again when the data cannot be set -> the error of FSCTL_SET_REPARSE_POINT
    // (0: the entry could not be made)
    fn new_link(root: isize, name: &str, dir: bool, data: &[u8]) -> Result<(), u32> {
        let opts = FILE_OPEN_REPARSE_POINT | if dir { FILE_DIRECTORY_FILE } else { FILE_NON_DIRECTORY_FILE };
        let h = nt_open(root, name, FILE_WRITE_ATTRIBUTES | DELETE, 0, FILE_CREATE, opts, if dir { 0 } else { ATTR_NORMAL }).map_err(|_| 0u32)?;
        let mut got = 0u32;
        // SAFETY: the reparse data and its length, no output; GetLastError right after a failure.
        let e = unsafe {
            if DeviceIoControl(h.0, FSCTL_SET_REPARSE_POINT, data.as_ptr() as usize, data.len() as u32, core::ptr::null_mut(), 0, &mut got, 0) != 0 {
                return Ok(());
            }
            GetLastError()
        };
        delete_h(&h);
        Err(e)
    }

    // f() with the privilege to create symbolic links enabled for this thread alone (on a copy
    // of the process's token, as CreateSymbolicLinkW does): an administrator or a service holds
    // it without having it enabled.  It counts when the entry is created (its handle opened),
    // not when the reparse data is set.  false when the thread already impersonates someone (its
    // token stays untouched) or the privilege is not held
    fn with_link_privilege(f: impl FnOnce() -> bool) -> bool {
        let mut t = 0isize;
        // SAFETY: the pseudo handle of this thread; a thread that already impersonates someone
        // keeps its token untouched (and the call fails)
        unsafe {
            if OpenThreadToken(GetCurrentThread(), 8, 1, &mut t) != 0 {
                CloseHandle(t);
                return false;
            }
            if ImpersonateSelf(2) == 0 {
                return false;
            }
        }
        // TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY; TOKEN_PRIVILEGES with one LUID_AND_ATTRIBUTES:
        // the privilege's LUID, SE_PRIVILEGE_ENABLED
        let tp: [u32; 4] = [1, SE_CREATE_SYMBOLIC_LINK_PRIVILEGE, 0, 2];
        // SAFETY: the thread's own impersonation token, closed below; a 16-byte TOKEN_PRIVILEGES;
        // GetLastError right after AdjustTokenPrivileges (ERROR_NOT_ALL_ASSIGNED: not held)
        let ok = unsafe {
            OpenThreadToken(GetCurrentThread(), 0x20 | 8, 1, &mut t) != 0 && {
                let t = H(t);
                AdjustTokenPrivileges(t.0, 0, tp.as_ptr() as *const u8, 0, core::ptr::null_mut(), core::ptr::null_mut()) != 0 && GetLastError() == 0
            }
        } && f();
        // SAFETY: ends the impersonation started above.
        unsafe { RevertToSelf() };
        ok
    }

    impl OsDir {
        pub fn open(path: &str) -> Option<OsDir> {
            // the start as it is (a link to it followed); every share mode: nothing is locked
            let h = open(path, FILE_READ_ATTRIBUTES, 0)?;
            meta_of(&h).is_some_and(|m| m.is_dir()).then_some(OsDir(h))
        }

        pub fn open_dir(&self, names: &[&str]) -> Option<OsDir> {
            let mut cur: Option<H> = None;
            for n in names {
                let base = cur.as_ref().map_or((self.0).0, |h| h.0);
                cur = Some(sub_dir(base, n)?);
            }
            cur.map(OsDir)
        }

        pub fn lstat(&self, name: &str) -> Option<Meta> {
            if name.is_empty() {
                return meta_of(&self.0);
            }
            meta_of(&entry_h((self.0).0, name, 0, 0).ok()?)
        }

        pub fn read_link(&self, name: &str) -> Option<String> {
            link_of(&entry_h((self.0).0, name, 0, 0).ok()?)
        }

        pub fn read_dir(&self) -> Option<Vec<DirEntry>> {
            // a handle of its own (the position of a listing belongs to the handle)
            let h = nt_open((self.0).0, "", FILE_LIST_DIRECTORY, SHARE_ALL, FILE_OPEN, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT, 0).ok()?;
            let mut buf = alloc::vec![0u64; 8 * 1024];
            let mut out = Vec::new();
            let mut first = true;
            loop {
                // SAFETY: the buffer holds its size in bytes; GetLastError right after a failure.
                if unsafe { GetFileInformationByHandleEx(h.0, FILE_FULL_DIR_INFO, buf.as_mut_ptr() as *mut u8, (buf.len() * 8) as u32) } == 0 {
                    let e = unsafe { GetLastError() };
                    // only the end of the listing is a whole listing (a volume's root may be empty)
                    return (e == ERROR_NO_MORE_FILES || first && e == ERROR_FILE_NOT_FOUND).then_some(out);
                }
                first = false;
                // SAFETY: the buffer as bytes (u64s are 8 bytes each).
                let b = unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
                let rd = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                // FILE_FULL_DIR_INFO: next entry, ..., attributes (56), name length (60), EA size
                // = the reparse tag of a reparse point (64), name (68)
                let mut off = 0usize;
                loop {
                    let (next, attrs, n, tag) = (rd(off)?, rd(off + 56)?, rd(off + 60)? as usize, rd(off + 64)?);
                    let w: Vec<u16> = b.get(off + 68..off + 68 + n)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    let name = String::from_utf16_lossy(&w);
                    if name != "." && name != ".." {
                        out.push(DirEntry { kind: kind_of(attrs, tag), name });
                    }
                    if next == 0 {
                        break;
                    }
                    off += next as usize;
                }
            }
        }

        pub fn open_file(&self, name: &str) -> Option<crate::File> {
            let root = (self.0).0;
            let first = entry_h(root, name, GENERIC_READ, FILE_NON_DIRECTORY_FILE).ok()?;
            let m = meta_of(&first)?;
            if !m.is_file() {
                return None;
            }
            if m.attrs & ATTR_REPARSE == 0 {
                return Some(owned(first));
            }
            // another reparse point (deduplication, a cloud file): its data through its driver,
            // if the name still leads to the same file
            let h = nt_open(root, name, GENERIC_READ, SHARE_ALL, FILE_OPEN, FILE_NON_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT, 0).ok()?;
            meta_of(&h).is_some_and(|n| n.is_file() && (n.dev, n.ino) == (m.dev, m.ino)).then(|| owned(h))
        }

        pub fn create_new(&self, name: &str) -> Option<crate::File> {
            // (as OsFile::create_new_raw: read and write, others may read)
            nt_open((self.0).0, name, GENERIC_READ | GENERIC_WRITE, SHARE_READ, FILE_CREATE, FILE_NON_DIRECTORY_FILE, ATTR_NORMAL).ok().map(owned)
        }

        pub fn mkdir(&self, name: &str) -> bool {
            // (default security, as CreateDirectoryW)
            nt_open((self.0).0, name, FILE_LIST_DIRECTORY, SHARE_ALL, FILE_CREATE, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT, ATTR_NORMAL).is_ok()
        }

        pub fn remove(&self, name: &str) -> bool {
            entry_h((self.0).0, name, DELETE, 0).is_ok_and(|h| delete_h(&h))
        }

        pub fn rename(&self, name: &str, to: &OsDir, to_name: &str) -> bool {
            // a program that holds the file (or the one replaced) without sharing deletion, a
            // virus scanner for a moment, is waited for up to about a quarter second (replace_move)
            let mut wait = 1u32;
            for left in (0..9).rev() {
                let e = match entry_h((self.0).0, name, DELETE, 0).map_err(|st| (st, false)).and_then(|h| rename_h(&h, (to.0).0, to_name)) {
                    Ok(()) => return true,
                    Err(e) => e,
                };
                let busy = matches!(e.0, STATUS_SHARING_VIOLATION | STATUS_FILE_LOCK_CONFLICT | STATUS_LOCK_NOT_GRANTED) || (e.1 && e.0 == STATUS_ACCESS_DENIED);
                if left == 0 || !busy {
                    return false;
                }
                crate::sleep_ms(wait);
                wait *= 2;
            }
            false
        }

        pub fn symlink(&self, target: &str, name: &str, dir: bool) -> bool {
            // (the developer mode lets anyone make one; else the privilege, enabled for the
            // thread and the entry made anew)
            let Some(data) = symlink_data(target) else { return false };
            let root = (self.0).0;
            match new_link(root, name, dir, &data) {
                Ok(()) => true,
                Err(ERROR_PRIVILEGE_NOT_HELD) => with_link_privilege(|| new_link(root, name, dir, &data).is_ok()),
                Err(_) => false,
            }
        }

        pub fn set_times(&self, name: &str, atime_ns: i64, mtime_ns: i64) -> bool {
            let Ok(h) = entry_h((self.0).0, name, FILE_WRITE_ATTRIBUTES, 0) else { return false };
            let (a, m) = (ns_ft(atime_ns), ns_ft(mtime_ns));
            // SAFETY: a handle with FILE_WRITE_ATTRIBUTES; creation time unchanged (null).
            unsafe { SetFileTime(h.0, core::ptr::null(), &a, &m) != 0 }
        }

        pub fn set_mode(&self, _name: &str, _mode: u32) -> bool {
            true
        }

        pub fn set_attrs(&self, name: &str, attrs: u32) -> bool {
            let Ok(h) = entry_h((self.0).0, name, FILE_WRITE_ATTRIBUTES, 0) else { return false };
            // FILE_BASIC_INFO: four times (0: unchanged), the attributes (as set_attrs: never 0,
            // which would mean unchanged)
            let mut info = [0u64; 5];
            info[4] = (attrs & 0x27 | if attrs & 0x27 == 0 { ATTR_NORMAL } else { 0 }) as u64;
            // SAFETY: a 40-byte FILE_BASIC_INFO; a handle with FILE_WRITE_ATTRIBUTES.
            unsafe { SetFileInformationByHandle(h.0, FILE_BASIC_INFO, info.as_ptr() as *const u8, 40) != 0 }
        }
    }

    fn owned(h: H) -> crate::File {
        let raw = h.0;
        core::mem::forget(h);
        crate::File::from_os(crate::OsFile::from_raw(raw))
    }

    pub fn set_file_times(f: &crate::OsFile, atime_ns: i64, mtime_ns: i64) -> bool {
        let (a, m) = (ns_ft(atime_ns), ns_ft(mtime_ns));
        // SAFETY: the file's handle; creation time unchanged (null).
        unsafe { SetFileTime(f.raw(), core::ptr::null(), &a, &m) != 0 }
    }

    pub fn set_file_mode(_f: &crate::OsFile, _mode: u32) -> bool {
        true
    }

    pub fn fd_path(_f: &crate::OsFile) -> Option<String> {
        None
    }
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
mod lnx {
    use super::{DirEntry, FileKind, Meta};
    use alloc::string::String;
    use alloc::vec::Vec;
    use rt::os::{syscall3, syscall6, AT_FDCWD, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_NONBLOCK, O_PATH, O_RDONLY, O_RDWR, O_WRONLY, SYS_CLOSE, SYS_FCHMOD, SYS_FCHMODAT, SYS_FCHOWNAT, SYS_FSTAT, SYS_FSYNC, SYS_GETDENTS64, SYS_GETEUID, SYS_LINKAT, SYS_MKDIRAT, SYS_NEWFSTATAT, SYS_OPENAT, SYS_OPENAT2, SYS_READLINKAT, SYS_RENAMEAT, SYS_RENAMEAT2, SYS_STATFS, SYS_SYMLINKAT, SYS_UNLINKAT, SYS_UTIMENSAT, SYS_WRITE, Stat};

    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
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
            let r = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, flags | O_CLOEXEC, mode, 0, 0) };
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
        stat_in(AT_FDCWD, &p, flags)
    }

    // newfstatat of a NUL-terminated name relative to dir
    fn stat_in(dir: usize, name: &[u8], flags: usize) -> Option<Meta> {
        let mut st = Stat::default();
        // SAFETY: a NUL-terminated name; st is a struct stat.
        let r = unsafe { syscall6(SYS_NEWFSTATAT, dir, name.as_ptr() as usize, &mut st as *mut Stat as usize, flags, 0, 0) };
        if r < 0 {
            return None;
        }
        Some(meta_of(&st))
    }

    fn meta_of(st: &Stat) -> Meta {
        let t = |s: i64, ns: i64| s.saturating_mul(1_000_000_000).saturating_add(ns);
        Meta {
            kind: Some(kind_of_mode(st.st_mode)),
            size: st.st_size as u64,
            atime_ns: t(st.st_atime, st.st_atime_nsec),
            mtime_ns: t(st.st_mtime, st.st_mtime_nsec),
            ctime_ns: t(st.st_ctime, st.st_ctime_nsec),
            mode: st.st_mode,
            uid: st.st_uid,
            gid: st.st_gid,
            attrs: 0,
            dev: st.st_dev,
            ino: st.st_ino,
            nlink: st.st_nlink as u32,
        }
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

    pub(crate) fn socket_id(path: &str) -> Option<(u64, u64)> {
        statx(path, AT_SYMLINK_NOFOLLOW).filter(|m| m.mode & 0o170000 == 0o140000).map(|m| (m.dev, m.ino))
    }

    pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let fd = open(path, O_RDONLY | O_DIRECTORY, 0)?;
        list_fd(&fd, &|name| lstat(&super::join(path, name)).and_then(|m| m.kind).unwrap_or(FileKind::Other))
    }

    // the entries of an open folder (getdents64 from its current position); `unknown`: the
    // kind of an entry the file system does not name (d_type 0)
    fn list_fd(fd: &Fd, unknown: &dyn Fn(&str) -> FileKind) -> Option<Vec<DirEntry>> {
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
                        0 => unknown(&name),
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
        let r = unsafe { syscall6(SYS_READLINKAT, AT_FDCWD, p.as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0) };
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
        unsafe { syscall3(SYS_SYMLINKAT, t.as_ptr() as usize, AT_FDCWD, l.as_ptr() as usize) >= 0 }
    }

    pub fn hard_link(existing: &str, new: &str) -> bool {
        if !(crate::policy::fs_allowed(existing, crate::policy::FS_WRITE) && crate::policy::fs_allowed(new, crate::policy::FS_WRITE)) {
            return false;
        }
        let (e, n) = (z(existing), z(new));
        // SAFETY: NUL-terminated paths.
        unsafe { syscall6(SYS_LINKAT, AT_FDCWD, e.as_ptr() as usize, AT_FDCWD, n.as_ptr() as usize, 0, 0) >= 0 }
    }

    pub fn set_times(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let p = z(path);
        let ts = [atime_ns.div_euclid(1_000_000_000), atime_ns.rem_euclid(1_000_000_000), mtime_ns.div_euclid(1_000_000_000), mtime_ns.rem_euclid(1_000_000_000)];
        // SAFETY: two timespecs; the link itself (AT_SYMLINK_NOFOLLOW).
        unsafe { syscall6(SYS_UTIMENSAT, AT_FDCWD, p.as_ptr() as usize, ts.as_ptr() as usize, AT_SYMLINK_NOFOLLOW, 0, 0) >= 0 }
    }

    /// permission bits (07777); a symbolic link is left alone and never followed (the entry is
    /// opened with O_NOFOLLOW and changed through that descriptor, so a link swapped in
    /// meanwhile cannot redirect the change)
    pub fn set_mode(path: &str, mode: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        // O_PATH: also an entry the owner may not read (mode 000 / 0200), a socket or a device
        // without opening it; changed through /proc/self/fd/N (fchmod refuses O_PATH)
        if let Some(fd) = open(path, O_PATH | O_NOFOLLOW, 0) {
            let link = z(&alloc::format!("/proc/self/fd/{}", fd.0));
            // SAFETY: fchmodat on a NUL-terminated path.
            let r = unsafe { syscall6(SYS_FCHMODAT, AT_FDCWD, link.as_ptr() as usize, (mode & 0o7777) as usize, 0, 0, 0) };
            if r != -2 {
                return r >= 0; // (-ENOENT: no /proc)
            }
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
        unsafe { syscall6(SYS_FCHOWNAT, AT_FDCWD, p.as_ptr() as usize, uid as usize, gid as usize, AT_SYMLINK_NOFOLLOW, 0) >= 0 }
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
        // SAFETY: struct statfs is 120 bytes on x86-64 and ARM64.
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
        fsync_dir(dir)
    }

    /// sync_dir without the manifest's check: the folder of a file that was allowed and has
    /// just been renamed into place (a rule for that file alone does not cover its folder)
    fn fsync_dir(dir: &str) -> bool {
        let Some(fd) = open(if dir.is_empty() { "." } else { dir }, O_RDONLY | O_DIRECTORY, 0) else { return false };
        // SAFETY: an owned descriptor.
        unsafe { syscall3(SYS_FSYNC, fd.0, 0, 0) >= 0 }
    }

    pub fn write_file_private(path: &str, data: &[u8]) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        // O_EXCL: a guessed name that exists (a planted link, a stale file) is never written through
        let mut opened = None;
        let seed = super::tmp_seed();
        for k in 0..16u64 {
            let name = alloc::format!("{}.tmp{}", path, seed.wrapping_add(k) % 1_000_000);
            if let Some(fd) = open(&name, O_WRONLY | O_CREAT | O_EXCL, 0o600) {
                opened = Some((name, fd));
                break;
            }
        }
        let Some((tmp, fd)) = opened else { return false };
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
        if !synced || unsafe { rt::os::rename(t.as_ptr() as usize, d.as_ptr() as usize) } < 0 {
            unlink(&tmp);
            return false;
        }
        fsync_dir(super::parent(path))
    }

    fn unlink(path: &str) {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { syscall3(SYS_UNLINKAT, AT_FDCWD, p.as_ptr() as usize, 0) };
    }

    pub fn mkdir_private(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if statx(path, AT_SYMLINK_NOFOLLOW).is_some() {
            return private_dir(path);
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !make_parents(up) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { syscall3(SYS_MKDIRAT, AT_FDCWD, p.as_ptr() as usize, 0o700) };
        r >= 0 || private_dir(path)
    }

    // the parents of mkdir_private's folder: an existing one is taken as it is (/var/lib, a
    // mount), new ones are private
    fn make_parents(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat(path).is_some_and(|m| m.is_dir()) {
            return true;
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !make_parents(up) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { syscall3(SYS_MKDIRAT, AT_FDCWD, p.as_ptr() as usize, 0o700) };
        r >= 0 || stat(path).is_some_and(|m| m.is_dir())
    }

    /// an existing folder mkdir_private hands out: a folder itself (no link) of this user (root's:
    /// root's) that nobody else may write to (someone else's, made before the program, may hold
    /// planted files); one others may only read is made owner-only
    fn private_dir(path: &str) -> bool {
        let Some(m) = statx(path, AT_SYMLINK_NOFOLLOW) else { return false };
        // SAFETY: geteuid has no arguments.
        let euid = unsafe { syscall3(SYS_GETEUID, 0, 0, 0) } as u32;
        if !m.is_dir() || m.uid != euid || m.mode & 0o022 != 0 {
            return false;
        }
        m.mode & 0o077 == 0 || set_mode(path, 0o700)
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

    // ---- Dir: the *at calls on an open folder

    const AT_EMPTY_PATH: usize = 0x1000;
    const AT_REMOVEDIR: usize = 0x200;
    const EPERM: isize = 1;
    const ENOENT: isize = 2;
    const E2BIG: isize = 7;
    const EAGAIN: isize = 11;
    const EISDIR: isize = 21;
    const EINVAL: isize = 22;
    const ENOSYS: isize = 38;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;
    const S_IFMT: u32 = 0o170000;

    /// an open folder (O_RDONLY | O_DIRECTORY: also listed through it)
    pub struct OsDir(Fd);

    // openat of a NUL-terminated name relative to dir
    fn open_in(dir: usize, name: &[u8], flags: usize, mode: usize) -> Option<Fd> {
        loop {
            // SAFETY: a NUL-terminated name.
            let r = unsafe { syscall6(SYS_OPENAT, dir, name.as_ptr() as usize, flags | O_CLOEXEC, mode, 0, 0) };
            if r == -EINTR {
                continue;
            }
            return (r >= 0).then_some(Fd(r as usize));
        }
    }

    // an open regular file (fstat) as a File; anything else is closed again
    fn regular(fd: Fd) -> Option<crate::File> {
        let mut st = Stat::default();
        // SAFETY: an owned descriptor; st is a struct stat.
        if unsafe { syscall3(SYS_FSTAT, fd.0, &mut st as *mut Stat as usize, 0) } < 0 || st.st_mode & S_IFMT != 0o100000 {
            return None;
        }
        Some(owned(fd))
    }

    fn owned(fd: Fd) -> crate::File {
        let raw = fd.0 as i32;
        core::mem::forget(fd);
        crate::File::from_os(crate::OsFile::from_raw(raw))
    }

    fn timespecs(atime_ns: i64, mtime_ns: i64) -> [i64; 4] {
        [atime_ns.div_euclid(1_000_000_000), atime_ns.rem_euclid(1_000_000_000), mtime_ns.div_euclid(1_000_000_000), mtime_ns.rem_euclid(1_000_000_000)]
    }

    impl OsDir {
        pub fn open(path: &str) -> Option<OsDir> {
            open(path, O_RDONLY | O_DIRECTORY, 0).map(OsDir)
        }

        pub fn open_dir(&self, names: &[&str]) -> Option<OsDir> {
            let flags = O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC;
            let rel = z(&names.join("/"));
            // struct open_how: flags, mode, resolve
            let how: [u64; 3] = [flags as u64, 0, RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS];
            loop {
                // SAFETY: a NUL-terminated path and a 24-byte open_how.
                let r = unsafe { syscall6(SYS_OPENAT2, (self.0).0, rel.as_ptr() as usize, how.as_ptr() as usize, 24, 0, 0) };
                if r >= 0 {
                    return Some(OsDir(Fd(r as usize)));
                }
                if r == -EINTR {
                    continue;
                }
                // no openat2 (before 5.6, a seccomp filter), or a rename meanwhile (EAGAIN):
                // the same rule one component at a time; anything else (ELOOP: a link) is final
                if ![-ENOSYS, -EPERM, -EAGAIN, -EINVAL, -E2BIG].contains(&r) {
                    return None;
                }
                break;
            }
            let mut cur: Option<Fd> = None;
            for n in names {
                let base = cur.as_ref().map_or((self.0).0, |f| f.0);
                cur = Some(open_in(base, &z(n), flags, 0)?);
            }
            cur.map(OsDir)
        }

        pub fn lstat(&self, name: &str) -> Option<Meta> {
            if name.is_empty() {
                return stat_in((self.0).0, b"\0", AT_EMPTY_PATH);
            }
            stat_in((self.0).0, &z(name), AT_SYMLINK_NOFOLLOW)
        }

        pub fn read_link(&self, name: &str) -> Option<String> {
            let p = z(name);
            let mut buf = alloc::vec![0u8; 4096];
            // SAFETY: a NUL-terminated name; the buffer is writable.
            let r = unsafe { syscall6(SYS_READLINKAT, (self.0).0, p.as_ptr() as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0) };
            if r < 0 {
                return None;
            }
            buf.truncate(r as usize);
            Some(String::from_utf8_lossy(&buf).into_owned())
        }

        pub fn read_dir(&self) -> Option<Vec<DirEntry>> {
            // a descriptor of its own: getdents moves the position
            let fd = open_in((self.0).0, b".\0", O_RDONLY | O_DIRECTORY, 0)?;
            list_fd(&fd, &|name| self.lstat(name).and_then(|m| m.kind).unwrap_or(FileKind::Other))
        }

        pub fn open_file(&self, name: &str) -> Option<crate::File> {
            // O_NONBLOCK: a pipe swapped in does not block the open (and is refused)
            regular(open_in((self.0).0, &z(name), O_RDONLY | O_NOFOLLOW | O_NONBLOCK, 0)?)
        }

        pub fn create_new(&self, name: &str) -> Option<crate::File> {
            open_in((self.0).0, &z(name), O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600).map(owned)
        }

        pub fn mkdir(&self, name: &str) -> bool {
            let p = z(name);
            // SAFETY: a NUL-terminated name; 0777 (the umask applies).
            unsafe { syscall3(SYS_MKDIRAT, (self.0).0, p.as_ptr() as usize, 0o777) >= 0 }
        }

        pub fn remove(&self, name: &str) -> bool {
            let p = z(name);
            // SAFETY: a NUL-terminated name.
            unsafe {
                let r = syscall3(SYS_UNLINKAT, (self.0).0, p.as_ptr() as usize, 0);
                r == 0 || ((r == -EISDIR || r == -EPERM) && syscall3(SYS_UNLINKAT, (self.0).0, p.as_ptr() as usize, AT_REMOVEDIR) == 0)
            }
        }

        pub fn rename(&self, name: &str, to: &OsDir, to_name: &str) -> bool {
            let (a, b) = (z(name), z(to_name));
            // SAFETY: NUL-terminated names; renameat where there is no renameat2 (before 3.15)
            unsafe {
                let r = syscall6(SYS_RENAMEAT2, (self.0).0, a.as_ptr() as usize, (to.0).0, b.as_ptr() as usize, 0, 0);
                if r != -ENOSYS {
                    return r == 0;
                }
                syscall6(SYS_RENAMEAT, (self.0).0, a.as_ptr() as usize, (to.0).0, b.as_ptr() as usize, 0, 0) == 0
            }
        }

        pub fn symlink(&self, target: &str, name: &str, _dir: bool) -> bool {
            let (t, l) = (z(target), z(name));
            // SAFETY: NUL-terminated target and name.
            unsafe { syscall3(SYS_SYMLINKAT, t.as_ptr() as usize, (self.0).0, l.as_ptr() as usize) >= 0 }
        }

        pub fn set_times(&self, name: &str, atime_ns: i64, mtime_ns: i64) -> bool {
            let ts = timespecs(atime_ns, mtime_ns);
            if name.is_empty() {
                // SAFETY: two timespecs; a null name changes the folder itself (futimens)
                return unsafe { syscall6(SYS_UTIMENSAT, (self.0).0, 0, ts.as_ptr() as usize, 0, 0, 0) >= 0 };
            }
            let p = z(name);
            // SAFETY: a NUL-terminated name and two timespecs; the link itself.
            unsafe { syscall6(SYS_UTIMENSAT, (self.0).0, p.as_ptr() as usize, ts.as_ptr() as usize, AT_SYMLINK_NOFOLLOW, 0, 0) >= 0 }
        }

        /// as set_mode: through a descriptor of the entry (O_PATH | O_NOFOLLOW, a link refused),
        /// never through a name looked up again
        pub fn set_mode(&self, name: &str, mode: u32) -> bool {
            let bits = (mode & 0o7777) as usize;
            if name.is_empty() {
                // SAFETY: an owned descriptor.
                return unsafe { syscall3(SYS_FCHMOD, (self.0).0, bits, 0) >= 0 };
            }
            let p = z(name);
            if let Some(fd) = open_in((self.0).0, &p, O_PATH | O_NOFOLLOW, 0) {
                let mut st = Stat::default();
                // SAFETY: an owned descriptor; st is a struct stat.
                if unsafe { syscall3(SYS_FSTAT, fd.0, &mut st as *mut Stat as usize, 0) } < 0 || st.st_mode & S_IFMT == 0o120000 {
                    return false;
                }
                let link = z(&alloc::format!("/proc/self/fd/{}", fd.0));
                // SAFETY: fchmodat on a NUL-terminated path.
                let r = unsafe { syscall6(SYS_FCHMODAT, AT_FDCWD, link.as_ptr() as usize, bits, 0, 0, 0) };
                if r != -ENOENT {
                    return r >= 0; // (-ENOENT: no /proc)
                }
            }
            match open_in((self.0).0, &p, O_RDONLY | O_NOFOLLOW | O_NONBLOCK, 0) {
                // SAFETY: an owned descriptor.
                Some(fd) => unsafe { syscall3(SYS_FCHMOD, fd.0, bits, 0) >= 0 },
                None => false,
            }
        }

        pub fn set_attrs(&self, _name: &str, _attrs: u32) -> bool {
            true
        }
    }

    pub fn set_file_times(f: &crate::OsFile, atime_ns: i64, mtime_ns: i64) -> bool {
        let ts = timespecs(atime_ns, mtime_ns);
        // SAFETY: the file's descriptor and two timespecs; a null name: the file itself (futimens)
        unsafe { syscall6(SYS_UTIMENSAT, f.raw() as usize, 0, ts.as_ptr() as usize, 0, 0, 0) >= 0 }
    }

    pub fn set_file_mode(f: &crate::OsFile, mode: u32) -> bool {
        // SAFETY: the file's descriptor.
        unsafe { syscall3(SYS_FCHMOD, f.raw() as usize, (mode & 0o7777) as usize, 0) >= 0 }
    }

    /// (the magic link of /proc leads to the open file, not through its name)
    pub fn fd_path(f: &crate::OsFile) -> Option<String> {
        Some(alloc::format!("/proc/{}/fd/{}", crate::process_id(), f.raw()))
    }

    /// the \\?\ form exists on Windows only
    pub fn long_path(path: &str) -> String {
        String::from(path)
    }
}

// ------------------------------------------------------------------ macOS and iOS

// the same functions as lnx through libSystem (docs/APPLE.md): Darwin's struct stat, readdir, the
// *at calls with AT_FDCWD = -2; no O_PATH, so set_mode leaves a link alone through lstat first
#[cfg(target_vendor = "apple")]
mod apl {
    use super::{DirEntry, FileKind, Meta};
    use crate::apple::ffi::{self, Stat};
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::ffi::{c_char, c_int};

    const AT_FDCWD: c_int = -2;
    const AT_SYMLINK_NOFOLLOW: c_int = 0x0020;
    const STATFS_SIZE: usize = 2168; // struct statfs with 64-bit inodes

    #[link(name = "System")]
    unsafe extern "C" {
        #[link_name = "symlink"]
        fn c_symlink(target: *const c_char, link: *const c_char) -> c_int;
        #[link_name = "link"]
        fn c_link(existing: *const c_char, new: *const c_char) -> c_int;
        fn utimensat(dir: c_int, path: *const c_char, ts: *const ffi::Timespec, flags: c_int) -> c_int;
        fn fchmodat(dir: c_int, path: *const c_char, mode: u16, flags: c_int) -> c_int;
        fn fchownat(dir: c_int, path: *const c_char, uid: u32, gid: u32, flags: c_int) -> c_int;
        #[cfg_attr(all(target_os = "macos", target_arch = "x86_64"), link_name = "statfs$INODE64")] fn statfs(path: *const c_char, buf: *mut u8) -> c_int;
        fn geteuid() -> u32;
        fn openat(dir: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
        #[cfg_attr(all(target_os = "macos", target_arch = "x86_64"), link_name = "fstatat$INODE64")] fn fstatat(dir: c_int, path: *const c_char, st: *mut Stat, flags: c_int) -> c_int;
        fn mkdirat(dir: c_int, path: *const c_char, mode: u16) -> c_int;
        fn unlinkat(dir: c_int, path: *const c_char, flags: c_int) -> c_int;
        fn renameat(from_dir: c_int, from: *const c_char, to_dir: c_int, to: *const c_char) -> c_int;
        fn readlinkat(dir: c_int, path: *const c_char, buf: *mut c_char, size: usize) -> isize;
        fn symlinkat(target: *const c_char, dir: c_int, link: *const c_char) -> c_int;
        #[cfg_attr(all(target_os = "macos", target_arch = "x86_64"), link_name = "fdopendir$INODE64")] fn fdopendir(fd: c_int) -> *mut core::ffi::c_void;
        fn futimens(fd: c_int, ts: *const ffi::Timespec) -> c_int;
    }

    fn z(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    fn zp(v: &[u8]) -> *const c_char {
        v.as_ptr().cast()
    }

    struct Fd(c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: an owned descriptor.
            unsafe { ffi::close(self.0) };
        }
    }

    fn open(path: &str, flags: c_int, mode: u32) -> Option<Fd> {
        let p = z(path);
        loop {
            // SAFETY: a NUL-terminated path; the mode is the variadic argument.
            let r = unsafe { ffi::open(zp(&p), flags | ffi::O_CLOEXEC, mode) };
            if r < 0 && ffi::errno() == ffi::EINTR {
                continue;
            }
            return (r >= 0).then_some(Fd(r));
        }
    }

    fn kind_of_mode(mode: u32) -> FileKind {
        match mode & 0o170000 {
            0o100000 => FileKind::File,
            0o040000 => FileKind::Dir,
            0o120000 => FileKind::Symlink,
            _ => FileKind::Other,
        }
    }

    fn stat_raw(path: &str, follow: bool) -> Option<Meta> {
        let p = z(path);
        let mut st = Stat::default();
        // SAFETY: a NUL-terminated path; st is a struct stat.
        let r = unsafe { if follow { ffi::stat(zp(&p), &mut st) } else { ffi::lstat(zp(&p), &mut st) } };
        if r < 0 {
            return None;
        }
        Some(meta_of(&st))
    }

    fn meta_of(st: &Stat) -> Meta {
        let t = |ts: ffi::Timespec| ts.tv_sec.saturating_mul(1_000_000_000).saturating_add(ts.tv_nsec);
        Meta {
            kind: Some(kind_of_mode(st.st_mode as u32)),
            size: st.st_size as u64,
            atime_ns: t(st.st_atimespec),
            mtime_ns: t(st.st_mtimespec),
            ctime_ns: t(st.st_ctimespec),
            mode: st.st_mode as u32,
            uid: st.st_uid,
            gid: st.st_gid,
            attrs: 0,
            dev: st.st_dev as u32 as u64,
            ino: st.st_ino,
            nlink: st.st_nlink as u32,
        }
    }

    pub fn stat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        stat_raw(path, true)
    }

    pub fn lstat(path: &str) -> Option<Meta> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        stat_raw(path, false)
    }

    pub(crate) fn socket_id(path: &str) -> Option<(u64, u64)> {
        stat_raw(path, false).filter(|m| m.mode & 0o170000 == 0o140000).map(|m| (m.dev, m.ino))
    }

    pub fn read_dir(path: &str) -> Option<Vec<DirEntry>> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path; the stream is closed below.
        let dir = unsafe { ffi::opendir(zp(&p)) };
        if dir.is_null() {
            return None;
        }
        let out = list_stream(dir, &|name| lstat(&super::join(path, name)).and_then(|m| m.kind).unwrap_or(FileKind::Other));
        // SAFETY: our stream.
        unsafe { ffi::closedir(dir) };
        Some(out)
    }

    // the entries of an open stream; `unknown`: the kind of an entry the file system does not
    // name (d_type 0)
    fn list_stream(dir: *mut core::ffi::c_void, unknown: &dyn Fn(&str) -> FileKind) -> Vec<DirEntry> {
        let mut out = Vec::new();
        loop {
            // SAFETY: an open stream; the entry stays valid until the next readdir.
            let e = unsafe { ffi::readdir(dir) };
            if e.is_null() {
                break;
            }
            // SAFETY: e is an entry of this stream with d_namlen bytes of name; no reference to the
            // whole record (it may be shorter than struct dirent).
            let (name, ty) = unsafe {
                let n = ((*e).d_namlen as usize).min(1024);
                let np = core::ptr::addr_of!((*e).d_name) as *const u8;
                (String::from_utf8_lossy(core::slice::from_raw_parts(np, n)).into_owned(), (*e).d_type)
            };
            if name != "." && name != ".." {
                let kind = match ty {
                    4 => FileKind::Dir,
                    8 => FileKind::File,
                    10 => FileKind::Symlink,
                    0 => unknown(&name),
                    _ => FileKind::Other,
                };
                out.push(DirEntry { name, kind });
            }
        }
        out
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
        // SAFETY: the buffer is writable for its length.
        let r = unsafe { ffi::readlink(zp(&p), buf.as_mut_ptr().cast(), buf.len()) };
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
        unsafe { c_symlink(zp(&t), zp(&l)) == 0 }
    }

    pub fn hard_link(existing: &str, new: &str) -> bool {
        if !(crate::policy::fs_allowed(existing, crate::policy::FS_WRITE) && crate::policy::fs_allowed(new, crate::policy::FS_WRITE)) {
            return false;
        }
        let (e, n) = (z(existing), z(new));
        // SAFETY: NUL-terminated paths.
        unsafe { c_link(zp(&e), zp(&n)) == 0 }
    }

    pub fn set_times(path: &str, atime_ns: i64, mtime_ns: i64) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let p = z(path);
        let ts = [
            ffi::Timespec { tv_sec: atime_ns.div_euclid(1_000_000_000), tv_nsec: atime_ns.rem_euclid(1_000_000_000) },
            ffi::Timespec { tv_sec: mtime_ns.div_euclid(1_000_000_000), tv_nsec: mtime_ns.rem_euclid(1_000_000_000) },
        ];
        // SAFETY: two timespecs; the link itself (AT_SYMLINK_NOFOLLOW).
        unsafe { utimensat(AT_FDCWD, zp(&p), ts.as_ptr(), AT_SYMLINK_NOFOLLOW) == 0 }
    }

    /// permission bits (07777); a symbolic link is left alone and never followed: the entry is
    /// opened with O_NOFOLLOW and changed through that descriptor, an entry the owner may not read
    /// through fchmodat without following
    pub fn set_mode(path: &str, mode: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat_raw(path, false).is_none_or(|m| m.is_symlink()) {
            return false;
        }
        let bits = (mode & 0o7777) as u16;
        match open(path, ffi::O_RDONLY | ffi::O_NOFOLLOW | ffi::O_NONBLOCK, 0) {
            // SAFETY: an owned descriptor.
            Some(fd) => unsafe { ffi::fchmod(fd.0, bits) == 0 },
            None => {
                let p = z(path);
                // SAFETY: a NUL-terminated path, never followed.
                unsafe { fchmodat(AT_FDCWD, zp(&p), bits, AT_SYMLINK_NOFOLLOW) == 0 }
            }
        }
    }

    pub fn set_owner(path: &str, uid: u32, gid: u32) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path; the link itself.
        unsafe { fchownat(AT_FDCWD, zp(&p), uid, gid, AT_SYMLINK_NOFOLLOW) == 0 }
    }

    pub fn set_attrs(_path: &str, _attrs: u32) -> bool {
        true
    }

    /// (free for this user, total) bytes: f_bavail and f_blocks times f_bsize
    pub fn free_space(path: &str) -> Option<(u64, u64)> {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_READ)) {
            return None;
        }
        let p = z(path);
        let mut b = alloc::vec![0u8; STATFS_SIZE];
        // SAFETY: b holds a struct statfs.
        if unsafe { statfs(zp(&p), b.as_mut_ptr()) } < 0 {
            return None;
        }
        let u64_at = |o: usize| u64::from_ne_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
        let bsize = u32::from_ne_bytes(b[0..4].try_into().unwrap_or([0; 4])) as u64;
        Some((u64_at(24).saturating_mul(bsize), u64_at(8).saturating_mul(bsize)))
    }

    pub fn sync_dir(dir: &str) -> bool {
        if !(crate::policy::fs_allowed(dir, crate::policy::FS_WRITE)) {
            return false;
        }
        fsync_dir(dir)
    }

    /// sync_dir without the manifest's check: the folder of a file that was allowed and has
    /// just been renamed into place (a rule for that file alone does not cover its folder)
    fn fsync_dir(dir: &str) -> bool {
        let Some(fd) = open(if dir.is_empty() { "." } else { dir }, ffi::O_RDONLY | ffi::O_DIRECTORY, 0) else { return false };
        // SAFETY: an owned descriptor.
        unsafe { ffi::fsync(fd.0) == 0 }
    }

    pub fn write_file_private(path: &str, data: &[u8]) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        // O_EXCL: a guessed name that exists (a planted link, a stale file) is never written through
        let mut opened = None;
        let seed = super::tmp_seed();
        for k in 0..16u64 {
            let name = alloc::format!("{}.tmp{}", path, seed.wrapping_add(k) % 1_000_000);
            if let Some(fd) = open(&name, ffi::O_WRONLY | ffi::O_CREAT | ffi::O_EXCL, 0o600) {
                opened = Some((name, fd));
                break;
            }
        }
        let Some((tmp, fd)) = opened else { return false };
        let mut off = 0;
        while off < data.len() {
            // SAFETY: the slice is readable for its length.
            let n = unsafe { ffi::write(fd.0, data[off..].as_ptr().cast(), (data.len() - off).min(1 << 30)) };
            if n < 0 && ffi::errno() == ffi::EINTR {
                continue;
            }
            if n <= 0 {
                drop(fd);
                unlink(&tmp);
                return false;
            }
            off += n as usize;
        }
        // SAFETY: an owned descriptor; F_FULLFSYNC writes the drive's cache too (fsync where unsupported).
        let synced = unsafe { ffi::fcntl(fd.0, ffi::F_FULLFSYNC) == 0 || ffi::fsync(fd.0) == 0 };
        drop(fd);
        let (t, d) = (z(&tmp), z(path));
        // SAFETY: NUL-terminated paths.
        if !synced || unsafe { ffi::rename(zp(&t), zp(&d)) } < 0 {
            unlink(&tmp);
            return false;
        }
        fsync_dir(super::parent(path))
    }

    fn unlink(path: &str) {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { ffi::unlink(zp(&p)) };
    }

    pub fn mkdir_private(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat_raw(path, false).is_some() {
            return private_dir(path);
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !make_parents(up) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { ffi::mkdir(zp(&p), 0o700) };
        r == 0 || private_dir(path)
    }

    // the parents of mkdir_private's folder: an existing one is taken as it is (/Library, a
    // volume), new ones are private
    fn make_parents(path: &str) -> bool {
        if !(crate::policy::fs_allowed(path, crate::policy::FS_WRITE)) {
            return false;
        }
        if stat(path).is_some_and(|m| m.is_dir()) {
            return true;
        }
        let up = super::parent(path);
        if !up.is_empty() && up != path && !make_parents(up) {
            return false;
        }
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { ffi::mkdir(zp(&p), 0o700) };
        r == 0 || stat(path).is_some_and(|m| m.is_dir())
    }

    /// an existing folder mkdir_private hands out: a folder itself (no link) of this user (root's:
    /// root's) that nobody else may write to (someone else's, made before the program, may hold
    /// planted files); one others may only read is made owner-only
    fn private_dir(path: &str) -> bool {
        let Some(m) = stat_raw(path, false) else { return false };
        // SAFETY: geteuid has no arguments.
        if !m.is_dir() || m.uid != unsafe { geteuid() } || m.mode & 0o022 != 0 {
            return false;
        }
        m.mode & 0o077 == 0 || set_mode(path, 0o700)
    }

    /// TZ, else the target of /etc/localtime (/var/db/timezone/zoneinfo/Europe/Berlin)
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
        let t = read_link_raw("/etc/localtime")?;
        t.find("zoneinfo/").map(|i| String::from(&t[i + 9..]))
    }

    // ---- Dir: the *at calls on an open folder, O_NOFOLLOW component by component

    const AT_REMOVEDIR: c_int = 0x0080;

    /// an open folder (O_RDONLY | O_DIRECTORY)
    pub struct OsDir(Fd);

    // openat of a name relative to dir
    fn open_in(dir: c_int, name: &str, flags: c_int, mode: u32) -> Option<Fd> {
        let p = z(name);
        loop {
            // SAFETY: a NUL-terminated name; the mode is the variadic argument.
            let r = unsafe { openat(dir, zp(&p), flags | ffi::O_CLOEXEC, mode) };
            if r < 0 && ffi::errno() == ffi::EINTR {
                continue;
            }
            return (r >= 0).then_some(Fd(r));
        }
    }

    fn owned(fd: Fd) -> crate::File {
        let raw = fd.0;
        core::mem::forget(fd);
        crate::File::from_os(crate::OsFile::from_raw(raw))
    }

    fn timespecs(atime_ns: i64, mtime_ns: i64) -> [ffi::Timespec; 2] {
        [
            ffi::Timespec { tv_sec: atime_ns.div_euclid(1_000_000_000), tv_nsec: atime_ns.rem_euclid(1_000_000_000) },
            ffi::Timespec { tv_sec: mtime_ns.div_euclid(1_000_000_000), tv_nsec: mtime_ns.rem_euclid(1_000_000_000) },
        ]
    }

    fn fd_meta(fd: c_int) -> Option<Meta> {
        let mut st = Stat::default();
        // SAFETY: an open descriptor; st is a struct stat.
        (unsafe { ffi::fstat(fd, &mut st) } == 0).then(|| meta_of(&st))
    }

    impl OsDir {
        pub fn open(path: &str) -> Option<OsDir> {
            open(path, ffi::O_RDONLY | ffi::O_DIRECTORY, 0).map(OsDir)
        }

        pub fn open_dir(&self, names: &[&str]) -> Option<OsDir> {
            let mut cur: Option<Fd> = None;
            for n in names {
                let base = cur.as_ref().map_or((self.0).0, |f| f.0);
                cur = Some(open_in(base, n, ffi::O_RDONLY | ffi::O_DIRECTORY | ffi::O_NOFOLLOW, 0)?);
            }
            cur.map(OsDir)
        }

        pub fn lstat(&self, name: &str) -> Option<Meta> {
            if name.is_empty() {
                return fd_meta((self.0).0);
            }
            let p = z(name);
            let mut st = Stat::default();
            // SAFETY: a NUL-terminated name; st is a struct stat; the link itself.
            (unsafe { fstatat((self.0).0, zp(&p), &mut st, AT_SYMLINK_NOFOLLOW) } == 0).then(|| meta_of(&st))
        }

        pub fn read_link(&self, name: &str) -> Option<String> {
            let p = z(name);
            let mut buf = alloc::vec![0u8; 4096];
            // SAFETY: a NUL-terminated name; the buffer is writable for its length.
            let r = unsafe { readlinkat((self.0).0, zp(&p), buf.as_mut_ptr().cast(), buf.len()) };
            if r < 0 {
                return None;
            }
            buf.truncate(r as usize);
            Some(String::from_utf8_lossy(&buf).into_owned())
        }

        pub fn read_dir(&self) -> Option<Vec<DirEntry>> {
            // a descriptor of its own for the stream (it takes the descriptor over)
            let fd = open_in((self.0).0, ".", ffi::O_RDONLY | ffi::O_DIRECTORY, 0)?;
            // SAFETY: an owned descriptor, handed to the stream on success.
            let dir = unsafe { fdopendir(fd.0) };
            if dir.is_null() {
                return None;
            }
            core::mem::forget(fd);
            let out = list_stream(dir, &|name| self.lstat(name).and_then(|m| m.kind).unwrap_or(FileKind::Other));
            // SAFETY: our stream (closes the descriptor).
            unsafe { ffi::closedir(dir) };
            Some(out)
        }

        pub fn open_file(&self, name: &str) -> Option<crate::File> {
            // O_NONBLOCK: a pipe swapped in does not block the open (and is refused)
            let fd = open_in((self.0).0, name, ffi::O_RDONLY | ffi::O_NOFOLLOW | ffi::O_NONBLOCK, 0)?;
            fd_meta(fd.0).is_some_and(|m| m.is_file()).then(|| owned(fd))
        }

        pub fn create_new(&self, name: &str) -> Option<crate::File> {
            open_in((self.0).0, name, ffi::O_RDWR | ffi::O_CREAT | ffi::O_EXCL | ffi::O_NOFOLLOW, 0o600).map(owned)
        }

        pub fn mkdir(&self, name: &str) -> bool {
            let p = z(name);
            // SAFETY: a NUL-terminated name; 0777 (the umask applies).
            unsafe { mkdirat((self.0).0, zp(&p), 0o777) == 0 }
        }

        pub fn remove(&self, name: &str) -> bool {
            let p = z(name);
            // SAFETY: a NUL-terminated name; a folder (not a link to one) with AT_REMOVEDIR.
            unsafe { unlinkat((self.0).0, zp(&p), 0) == 0 || (self.lstat(name).is_some_and(|m| m.is_dir()) && unlinkat((self.0).0, zp(&p), AT_REMOVEDIR) == 0) }
        }

        pub fn rename(&self, name: &str, to: &OsDir, to_name: &str) -> bool {
            let (a, b) = (z(name), z(to_name));
            // SAFETY: NUL-terminated names.
            unsafe { renameat((self.0).0, zp(&a), (to.0).0, zp(&b)) == 0 }
        }

        pub fn symlink(&self, target: &str, name: &str, _dir: bool) -> bool {
            let (t, l) = (z(target), z(name));
            // SAFETY: NUL-terminated target and name.
            unsafe { symlinkat(zp(&t), (self.0).0, zp(&l)) == 0 }
        }

        pub fn set_times(&self, name: &str, atime_ns: i64, mtime_ns: i64) -> bool {
            let ts = timespecs(atime_ns, mtime_ns);
            if name.is_empty() {
                // SAFETY: an owned descriptor and two timespecs.
                return unsafe { futimens((self.0).0, ts.as_ptr()) == 0 };
            }
            let p = z(name);
            // SAFETY: a NUL-terminated name and two timespecs; the link itself.
            unsafe { utimensat((self.0).0, zp(&p), ts.as_ptr(), AT_SYMLINK_NOFOLLOW) == 0 }
        }

        /// as set_mode: through a descriptor of the entry (O_NOFOLLOW), else fchmodat without
        /// following; a link is left alone
        pub fn set_mode(&self, name: &str, mode: u32) -> bool {
            let bits = (mode & 0o7777) as u16;
            if name.is_empty() {
                // SAFETY: an owned descriptor.
                return unsafe { ffi::fchmod((self.0).0, bits) == 0 };
            }
            if self.lstat(name).is_none_or(|m| m.is_symlink()) {
                return false;
            }
            match open_in((self.0).0, name, ffi::O_RDONLY | ffi::O_NOFOLLOW | ffi::O_NONBLOCK, 0) {
                // SAFETY: an owned descriptor.
                Some(fd) => unsafe { ffi::fchmod(fd.0, bits) == 0 },
                None => {
                    let p = z(name);
                    // SAFETY: a NUL-terminated name, never followed.
                    unsafe { fchmodat((self.0).0, zp(&p), bits, AT_SYMLINK_NOFOLLOW) == 0 }
                }
            }
        }

        pub fn set_attrs(&self, _name: &str, _attrs: u32) -> bool {
            true
        }
    }

    pub fn set_file_times(f: &crate::OsFile, atime_ns: i64, mtime_ns: i64) -> bool {
        let ts = timespecs(atime_ns, mtime_ns);
        // SAFETY: the file's descriptor and two timespecs.
        unsafe { futimens(f.raw(), ts.as_ptr()) == 0 }
    }

    pub fn set_file_mode(f: &crate::OsFile, mode: u32) -> bool {
        // SAFETY: the file's descriptor.
        unsafe { ffi::fchmod(f.raw(), (mode & 0o7777) as u16) == 0 }
    }

    /// (/dev/fd/<n> names a descriptor of the process that opens it, not of this one)
    pub fn fd_path(_f: &crate::OsFile) -> Option<String> {
        None
    }

    /// the \\?\ form exists on Windows only
    pub fn long_path(path: &str) -> String {
        String::from(path)
    }
}
