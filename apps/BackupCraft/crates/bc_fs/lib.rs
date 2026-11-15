// bc_fs -- the file system work the server and the agent share: an entry's metadata the way a
// backup keeps it, folder listings, shared-folder checks and the safe restore primitives
//
//   entry / listing          lstat-based (links are reported, never followed)
//   inside_os                a path inside one of the shared folders (no "..", Windows paths
//                            compared case-insensitively)
//   root_of / no_links       the shared folder of a path; no link (or junction) between that
//                            folder and the path, so a link inside a shared folder cannot lead
//                            a read or a write out of it (the folder itself may be a link)
//   open_dir / open_parent   a folder in a shared folder opened (plat::fs::Dir) without
//                            following a link on the way; what is read, written, changed or
//                            removed in it after that is relative to the open folder, so a
//                            folder swapped for a link after a check is not followed either
//   ensure_dir               create the missing folders of a path; none of them may be a link,
//                            so a link planted in the target cannot redirect restored data
//   free_target              an existing entry: skip, "name (wiederhergestellt N).ext" or replace
//   temp_name / finish_file  data goes to "<name>.bcrestore-<random>" and is renamed into
//                            place when complete and synced; times and mode set through the
//                            file's own handle before, attributes after (set-user-ID and
//                            set-group-ID never: the owner is not restored, the file belongs to
//                            whoever restores it -- root)
//   remove_tree_in           every listing and removal relative to the open folders
//
// The *_in / *_at forms work in an open folder; the forms with a path open its folder as it is
// (a server's own folders, bcctl).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::agent::{AgentEntry, AgentListing};
use plat::fs::{FileKind, Meta};
pub use plat::fs::Dir;

/// the wire name of a file kind
pub fn kind_name(k: Option<FileKind>) -> &'static str {
    match k {
        Some(FileKind::File) => "file",
        Some(FileKind::Dir) => "dir",
        Some(FileKind::Symlink) => "link",
        _ => "other",
    }
}

fn entry_of(name: &str, m: Meta, link: String) -> AgentEntry {
    AgentEntry {
        name: String::from(name),
        kind: String::from(kind_name(m.kind)),
        size: if m.is_dir() { 0 } else { m.size },
        mtime_ns: m.mtime_ns,
        atime_ns: m.atime_ns,
        ctime_ns: m.ctime_ns,
        mode: m.mode,
        uid: m.uid,
        gid: m.gid,
        attrs: m.attrs,
        ino: m.ino,
        dev: m.dev,
        link,
    }
}

/// an entry with everything a backup keeps (links are not followed)
pub fn entry(path: &str, name: &str) -> Option<AgentEntry> {
    let m = plat::fs::lstat(path)?;
    let link = if m.is_symlink() { plat::fs::read_link(path).unwrap_or_default() } else { String::new() };
    Some(entry_of(name, m, link))
}

/// `entry` of `name` in an open folder ("": the folder itself), reported as `shown`
pub fn entry_at(d: &Dir, name: &str, shown: &str) -> Option<AgentEntry> {
    let m = d.lstat(name)?;
    let link = if m.is_symlink() { d.read_link(name).unwrap_or_default() } else { String::new() };
    Some(entry_of(shown, m, link))
}

/// `entry` of a path in the shared folder `root` (open_parent: no link on the way)
pub fn entry_in(root: Option<&str>, path: &str) -> Option<AgentEntry> {
    match open_parent(root, path, false) {
        Ok((d, name)) => entry_at(&d, &name, &name),
        // a root ("/", "C:\"): the folder itself
        Err(_) if plat::fs::parent(path).is_empty() || file_name(path).is_empty() => entry_at(&open_dir(root, path, false).ok()?, "", file_name(path)),
        Err(_) => None,
    }
}

/// a page of a folder in the shared folder `root` (open_dir: no link on the way): folders
/// first, then by name (case-insensitive)
pub fn listing(root: Option<&str>, path: &str, offset: usize, limit: usize) -> Option<AgentListing> {
    let d = open_dir(root, path, false).ok()?;
    let mut items = d.read_dir()?;
    items.sort_by(|a, b| ((a.kind != FileKind::Dir), a.name.to_lowercase()).cmp(&((b.kind != FileKind::Dir), b.name.to_lowercase())));
    let limit = limit.max(1);
    let more = items.len() > offset.saturating_add(limit);
    let entries = items
        .iter()
        .skip(offset)
        .take(limit)
        .map(|e| entry_at(&d, &e.name, &e.name).unwrap_or(AgentEntry { name: e.name.clone(), kind: String::from("other"), ..AgentEntry::default() }))
        .collect();
    Some(AgentListing { path: String::from(path), entries, more })
}

/// whether `path` lies in one of `roots` (an empty root allows everything)
pub fn inside_os(roots: &[String], path: &str, win: bool) -> bool {
    if path.split(['/', '\\']).any(|p| p == "..") {
        return false;
    }
    let norm = |s: &str| -> String {
        let t = s.replace('\\', "/");
        let t = String::from(t.trim_end_matches('/'));
        if win { t.to_lowercase() } else { t }
    };
    let p = norm(path);
    roots.iter().any(|r| {
        let r = norm(r);
        r.is_empty() || p == r || p.starts_with(&format!("{}/", r))
    })
}

/// `inside_os` with this machine's path rules
pub fn inside(roots: &[String], path: &str) -> bool {
    inside_os(roots, path, cfg!(windows))
}

/// the shared folder (the longest one) `path` lies in, on this machine
pub fn root_of<'a>(roots: &'a [String], path: &str) -> Option<&'a str> {
    roots.iter().filter(|r| inside(core::slice::from_ref(*r), path)).max_by_key(|r| r.len()).map(|r| r.as_str())
}

/// no link and no special entry between `root` and `path`: every existing folder below the
/// root on the way, and `path` itself when `last`
pub fn no_links(root: &str, path: &str, last: bool) -> Result<(), String> {
    let rest = path.get(root.len()..).unwrap_or("");
    let parts: Vec<&str> = rest.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let n = if last { parts.len() } else { parts.len().saturating_sub(1) };
    let mut cur = String::from(root);
    for (i, p) in parts[..n].iter().enumerate() {
        cur = plat::fs::join(&cur, p);
        match plat::fs::lstat(&cur) {
            None => return Ok(()), // nothing below exists yet
            Some(m) if m.is_symlink() => return Err(format!("{} ist eine Verknüpfung", cur)),
            Some(m) if m.is_dir() => {}
            Some(_) if i + 1 == parts.len() => {} // the path itself may be a file
            Some(_) => return Err(format!("{} ist kein Ordner", cur)),
        }
    }
    Ok(())
}

/// an absolute path of a Windows or Unix file system
pub fn absolute(p: &str, win: bool) -> bool {
    if win {
        let b = p.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || p.starts_with("\\\\")
    } else {
        p.starts_with('/')
    }
}

/// `dir` + separator + `name` with the separator of the target system
pub fn join_os(dir: &str, name: &str, win: bool) -> String {
    let mut s = String::from(dir);
    if !s.ends_with(['/', '\\']) {
        s.push(if win { '\\' } else { '/' });
    }
    s.push_str(name);
    s
}

/// a name from a restore point that is safe as one path component
pub fn safe_name(n: &str, win: bool) -> bool {
    !n.is_empty() && n != "." && n != ".." && !n.contains(['/', '\\', '\0']) && !(win && n.contains(':'))
}

/// shared folders as given by a user: trimmed, absolute, no "..", without duplicates
pub fn clean_roots(list: &[String], win: bool) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for r in list {
        let r = r.trim();
        if r.is_empty() {
            continue;
        }
        if !absolute(r, win) || r.split(['/', '\\']).any(|p| p == "..") {
            return Err(format!("{} ist kein absoluter Pfad", r));
        }
        if !out.iter().any(|o| o == r) {
            out.push(String::from(r));
        }
    }
    Ok(out)
}

/// create the folders of `dir` that are missing; none of them may be a link
pub fn ensure_dir(dir: &str) -> Result<(), String> {
    match plat::fs::lstat(dir) {
        Some(m) if m.is_dir() => return Ok(()),
        Some(m) if m.is_symlink() => return Err(format!("{} ist eine Verknüpfung – dort wird nicht wiederhergestellt", dir)),
        Some(_) => return Err(format!("{} ist eine Datei", dir)),
        None => {}
    }
    let up = plat::fs::parent(dir);
    if !up.is_empty() && up != dir {
        ensure_dir(up)?;
    }
    if plat::mkdir(&plat::fs::long_path(dir)) || plat::fs::lstat(dir).is_some_and(|m| m.is_dir()) {
        Ok(())
    } else {
        Err(format!("{} kann nicht angelegt werden", dir))
    }
}

// the separators of this machine's paths
fn seps() -> &'static [char] {
    if cfg!(windows) { &['/', '\\'] } else { &['/'] }
}

/// the last name of a path ("" for a root)
pub fn file_name(path: &str) -> &str {
    let t = path.trim_end_matches(seps());
    t[plat::fs::parent(t).len()..].trim_start_matches(seps())
}

/// the folder `dir` opened (plat::fs::Dir).  In the shared folder `root`: no folder below it
/// on the way may be a link, and none swapped for one later redirects what is done in the open
/// folder (all of that is relative to it).  Without a root, or for a folder outside it (the
/// shared folder's own parents): as it is.  `create`: missing folders are made (as ensure_dir:
/// none of them may be a link).
pub fn open_dir(root: Option<&str>, dir: &str, create: bool) -> Result<Dir, String> {
    let r = match root {
        Some(r) if inside(&[String::from(r)], dir) => r,
        // the shared folder's own parents are taken as it is
        Some(r) if !inside(&[String::from(dir)], r) => return Err(format!("{} liegt nicht in {}", dir, r)),
        _ => {
            if create {
                ensure_dir(dir)?;
            }
            return Dir::open(dir).ok_or_else(|| format!("{} kann nicht geöffnet werden", dir));
        }
    };
    // (the shared folder itself is taken as it is, also a link; made when missing, as before)
    if create && plat::fs::stat(r).is_none() {
        ensure_dir(r)?;
    }
    let start = Dir::open(r).ok_or_else(|| format!("{} kann nicht geöffnet werden", r))?;
    let rest = dir.get(r.len()..).unwrap_or("").trim_start_matches(seps());
    if rest.split(seps()).all(|p| p.is_empty()) {
        return Ok(start);
    }
    // one call for the whole way when it exists (Linux: openat2); else step by step, making
    // what is missing or naming why not
    if let Some(d) = start.open_dir(rest) {
        return Ok(d);
    }
    let (mut cur, mut path) = (start, String::from(r));
    for name in rest.split(seps()).filter(|p| !p.is_empty()) {
        path = plat::fs::join(&path, name);
        cur = sub_dir(&cur, name, &path, create)?;
    }
    Ok(cur)
}

/// the folder `name` in the open folder `d` (`path`: its path, for messages), made when it is
/// missing and `create`; a link or a file there is refused, and a link swapped in after that
/// check is not opened
pub fn sub_dir(d: &Dir, name: &str, path: &str, create: bool) -> Result<Dir, String> {
    if create && d.lstat(name).is_none() && !d.mkdir(name) && d.lstat(name).is_none() {
        return Err(format!("{} kann nicht angelegt werden", path));
    }
    match d.lstat(name) {
        None => Err(format!("{}: nicht vorhanden", path)),
        Some(m) if m.is_symlink() && create => Err(format!("{} ist eine Verknüpfung – dort wird nicht wiederhergestellt", path)),
        Some(m) if m.is_symlink() => Err(format!("{} ist eine Verknüpfung", path)),
        Some(m) if !m.is_dir() && create => Err(format!("{} ist eine Datei", path)),
        Some(m) if !m.is_dir() => Err(format!("{} ist kein Ordner", path)),
        Some(_) => d.open_dir(name).ok_or_else(|| format!("{} kann nicht geöffnet werden", path)),
    }
}

/// the folder of `path` opened as open_dir does, and the name of `path` in it
pub fn open_parent(root: Option<&str>, path: &str, create: bool) -> Result<(Dir, String), String> {
    let name = file_name(path);
    let parent = plat::fs::parent(path);
    if name.is_empty() || parent.is_empty() {
        return Err(format!("{}: kein Eintrag eines Ordners", path));
    }
    // "C:" is the drive's current folder, "C:\" its root
    let parent = if cfg!(windows) && parent.ends_with(':') { format!("{}\\", parent) } else { String::from(parent) };
    Ok((open_dir(root, &parent, create)?, String::from(name)))
}

/// what happens to an entry that exists at the destination
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// write here (the name is free, or replacing was asked for)
    Free,
    /// written under another name
    Renamed,
    /// left alone
    Skip,
}

/// the destination of a restored entry under the conflict rule ("skip", "rename", "overwrite")
pub fn free_target(dest: &str, conflict: &str) -> (Target, String) {
    free_target_by(dest, conflict, &|p| plat::fs::lstat(p).is_some())
}

/// free_target for a destination in the open folder `d` (dest's folder)
pub fn free_target_in(d: &Dir, dest: &str, conflict: &str) -> (Target, String) {
    free_target_by(dest, conflict, &|p| d.lstat(file_name(p)).is_some())
}

fn free_target_by(dest: &str, conflict: &str, exists: &dyn Fn(&str) -> bool) -> (Target, String) {
    if !exists(dest) {
        return (Target::Free, String::from(dest));
    }
    match conflict {
        "overwrite" => (Target::Free, String::from(dest)),
        "rename" => {
            let dir = plat::fs::parent(dest);
            let name = dest[dir.len()..].trim_start_matches(['/', '\\']);
            let (stem, ext) = match name.rfind('.') {
                Some(i) if i > 0 => (&name[..i], &name[i..]),
                _ => (name, ""),
            };
            for k in 1..1000 {
                let n = if k == 1 { format!("{} (wiederhergestellt){}", stem, ext) } else { format!("{} (wiederhergestellt {}){}", stem, k, ext) };
                let p = plat::fs::join(dir, &n);
                if !exists(&p) {
                    return (Target::Renamed, p);
                }
            }
            (Target::Skip, String::from(dest))
        }
        _ => (Target::Skip, String::from(dest)),
    }
}

/// the temporary file a restored file is written to
pub fn temp_name(dest: &str) -> String {
    format!("{}.bcrestore-{:08x}", dest, plat::random_u64() as u32)
}

/// set-user-ID and set-group-ID: not restored, the owner is not restored either (a file of
/// any user in a backup would come back as a set-user-ID file of root)
const MODE_SETID: u32 = 0o6000;

/// times, permissions (Linux, without MODE_SETID) and attributes (Windows) of a restored entry
/// (by path: a folder of bcctl or of the server's own; in a shared folder apply_meta_in)
pub fn apply_meta(path: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) {
    if mtime_ns != 0 {
        plat::fs::set_times(&plat::fs::long_path(path), atime_ns, mtime_ns);
    }
    if cfg!(target_os = "linux") && mode != 0 {
        plat::fs::set_mode(path, mode & !MODE_SETID);
    }
    if cfg!(windows) && attrs & 0x27 != 0 {
        plat::fs::set_attrs(path, attrs);
    }
}

/// apply_meta on `name` in an open folder ("": the folder itself), never through a link
pub fn apply_meta_in(d: &Dir, name: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) {
    if mtime_ns != 0 {
        d.set_times(name, atime_ns, mtime_ns);
    }
    if cfg!(target_os = "linux") && mode != 0 {
        d.set_mode(name, mode & !MODE_SETID);
    }
    if cfg!(windows) && attrs & 0x27 != 0 {
        d.set_attrs(name, attrs);
    }
}

/// a complete temporary file `tmp` of the open folder `d` into place as `dest` (a name in the
/// same folder): its times and mode set and its data synced through its own handle (never
/// reopened by name), renamed in the open folder, its attributes applied; on failure it is
/// removed
pub fn finish_file_in(file: plat::File, d: &Dir, tmp: &str, dest: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) -> Result<(), String> {
    if mtime_ns != 0 {
        plat::fs::set_file_times(&file, atime_ns, mtime_ns);
    }
    if cfg!(target_os = "linux") && mode != 0 {
        plat::fs::set_file_mode(&file, mode & !MODE_SETID);
    }
    let synced = file.sync();
    drop(file);
    if !synced || !d.rename(tmp, d, dest) {
        d.remove(tmp);
        return Err(String::from("konnte nicht an seinen Platz gelegt werden"));
    }
    apply_meta_in(d, dest, 0, 0, 0, attrs);
    Ok(())
}

/// finish_file_in with the paths of the temporary file and the destination (in one folder,
/// opened as it is)
pub fn finish_file(file: plat::File, tmp: &str, dest: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) -> Result<(), String> {
    match open_parent(None, tmp, false) {
        Ok((d, t)) => finish_file_in(file, &d, &t, file_name(dest), atime_ns, mtime_ns, mode, attrs),
        Err(_) => {
            drop(file);
            plat::delete(&plat::fs::long_path(tmp));
            Err(String::from("konnte nicht an seinen Platz gelegt werden"))
        }
    }
}

/// remove a file, a link (never what it points to) or a folder with everything below it (links
/// below are removed as links)
pub fn remove_tree(path: &str) -> Result<(), String> {
    remove_tree_in(None, path)
}

/// remove_tree of a path in the shared folder `root`: its folder is opened without following a
/// link on the way (open_parent), every listing and removal below happens in the open folders,
/// so a folder swapped for a link meanwhile (a large tree gives time for it) is removed as the
/// link instead of leading the removal out of the shared folder
pub fn remove_tree_in(root: Option<&str>, path: &str) -> Result<(), String> {
    let (d, name) = open_parent(root, path, false)?;
    remove_tree_at(&d, &name, path)
}

/// remove_tree of `name` in the open folder `d` (`path`: its path, for messages)
pub fn remove_tree_at(d: &Dir, name: &str, path: &str) -> Result<(), String> {
    let m = d.lstat(name).ok_or_else(|| format!("{}: nicht vorhanden", path))?;
    if m.is_dir() {
        // (a link swapped in meanwhile is not opened: it is removed as the link below)
        if let Some(sub) = d.open_dir(name) {
            for e in sub.read_dir().unwrap_or_default() {
                remove_tree_at(&sub, &e.name, &plat::fs::join(path, &e.name))?;
            }
        }
    }
    if d.remove(name) { Ok(()) } else { Err(format!("{} kann nicht gelöscht werden", path)) }
}

/// FILE_ATTRIBUTE_DIRECTORY: on Windows a link to a folder carries it
pub const ATTR_DIRECTORY: u32 = 0x10;
/// a symbolic link at `dest` (an existing entry is replaced only when `replace`; `dir`: a link
/// to a folder, which Windows creates differently)
pub fn make_link(target: &str, dest: &str, replace: bool, dir: bool) -> Result<(), String> {
    if plat::fs::lstat(dest).is_some() {
        if !replace {
            return Err(String::from("existiert bereits"));
        }
        plat::delete(&plat::fs::long_path(dest));
    }
    if plat::fs::symlink(target, dest, dir) { Ok(()) } else { Err(String::from("Verknüpfung konnte nicht angelegt werden")) }
}

/// make_link of `name` in an open folder
pub fn make_link_in(d: &Dir, target: &str, name: &str, replace: bool, dir: bool) -> Result<(), String> {
    if d.lstat(name).is_some() {
        if !replace {
            return Err(String::from("existiert bereits"));
        }
        d.remove(name);
    }
    if d.symlink(target, name, dir) { Ok(()) } else { Err(String::from("Verknüpfung konnte nicht angelegt werden")) }
}

/// a log file of a program: lines with the time (UTC), "<name>.log" in a folder, renamed to
/// "<name>.1.log" ... "<name>.<keep>.log" when it reaches `max` bytes; the lines also go to
/// standard output (a console, journald)
pub struct Log {
    path: Option<String>,
    max: u64,
    keep: u32,
    file: plat::sync::Mutex<Option<(plat::File, u64)>>,
}

impl Log {
    /// a log in `dir` (None: standard output only)
    pub fn new(dir: Option<&str>, name: &str) -> Log {
        let path = dir.filter(|d| plat::fs::mkdir_private(d)).map(|d| plat::fs::join(d, &format!("{}.log", name)));
        Log { path, max: 10 << 20, keep: 5, file: plat::sync::Mutex::new(None) }
    }

    fn rotate(&self, path: &str) {
        let stem = &path[..path.len() - 4];
        plat::delete(&format!("{}.{}.log", stem, self.keep));
        for i in (1..self.keep).rev() {
            plat::rename(&format!("{}.{}.log", stem, i), &format!("{}.{}.log", stem, i + 1));
        }
        plat::rename(path, &format!("{}.1.log", stem));
    }

    pub fn line(&self, text: &str) {
        let t = datetime::rfc3339(datetime::Timestamp::from_secs(plat::unix_time_ns().div_euclid(1_000_000_000)));
        let l = format!("{} {}\n", t, text.trim_end());
        plat::write_stdout(l.as_bytes());
        let Some(path) = &self.path else { return };
        let mut g = self.file.lock();
        if g.as_ref().is_some_and(|(_, size)| *size >= self.max) {
            *g = None;
            self.rotate(path);
        }
        if g.is_none() {
            if let Some(f) = plat::File::open(path, plat::FO_RW) {
                let size = f.size().unwrap_or(0);
                *g = Some((f, size));
            }
        }
        if let Some((f, size)) = g.as_mut() {
            if f.write_at(*size, l.as_bytes()).is_some() {
                *size += l.len() as u64;
            }
        }
    }
}
