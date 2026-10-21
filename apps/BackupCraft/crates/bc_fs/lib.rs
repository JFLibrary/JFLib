// bc_fs -- the file system work the server and the agent share: an entry's metadata the way a
// backup keeps it, folder listings, shared-folder checks and the safe restore primitives
//
//   entry / listing          lstat-based (links are reported, never followed)
//   inside_os                a path inside one of the shared folders (no "..", Windows paths
//                            compared case-insensitively)
//   root_of / no_links       the shared folder of a path; no link (or junction) between that
//                            folder and the path, so a link inside a shared folder cannot lead
//                            a read or a write out of it (the folder itself may be a link)
//   ensure_dir               create the missing folders of a path; none of them may be a link,
//                            so a link planted in the target cannot redirect restored data
//   free_target              an existing entry: skip, "name (wiederhergestellt N).ext" or replace
//   temp_name / finish_file  data goes to "<name>.bcrestore-<random>" and is renamed into
//                            place when complete and synced; times, mode, attributes follow
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::agent::{AgentEntry, AgentListing};
use plat::fs::FileKind;

/// the wire name of a file kind
pub fn kind_name(k: Option<FileKind>) -> &'static str {
    match k {
        Some(FileKind::File) => "file",
        Some(FileKind::Dir) => "dir",
        Some(FileKind::Symlink) => "link",
        _ => "other",
    }
}

/// an entry with everything a backup keeps (links are not followed)
pub fn entry(path: &str, name: &str) -> Option<AgentEntry> {
    let m = plat::fs::lstat(path)?;
    let link = if m.is_symlink() { plat::fs::read_link(path).unwrap_or_default() } else { String::new() };
    Some(AgentEntry {
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
    })
}

/// a page of a folder: folders first, then by name (case-insensitive)
pub fn listing(path: &str, offset: usize, limit: usize) -> Option<AgentListing> {
    let mut items = plat::fs::read_dir(path)?;
    items.sort_by(|a, b| ((a.kind != FileKind::Dir), a.name.to_lowercase()).cmp(&((b.kind != FileKind::Dir), b.name.to_lowercase())));
    let limit = limit.max(1);
    let more = items.len() > offset.saturating_add(limit);
    let entries = items
        .iter()
        .skip(offset)
        .take(limit)
        .map(|e| {
            let p = plat::fs::join(path, &e.name);
            entry(&p, &e.name).unwrap_or(AgentEntry { name: e.name.clone(), kind: String::from("other"), ..AgentEntry::default() })
        })
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
    if plat::fs::lstat(dest).is_none() {
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
                if plat::fs::lstat(&p).is_none() {
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

/// times, permissions (Linux) and attributes (Windows) of a restored entry
pub fn apply_meta(path: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) {
    if mtime_ns != 0 {
        plat::fs::set_times(&plat::fs::long_path(path), atime_ns, mtime_ns);
    }
    if cfg!(target_os = "linux") && mode != 0 {
        plat::fs::set_mode(path, mode);
    }
    if cfg!(windows) && attrs & 0x27 != 0 {
        plat::fs::set_attrs(path, attrs);
    }
}

/// a complete temporary file into place: synced through its own handle (never reopened by
/// name), renamed, its metadata applied
pub fn finish_file(file: plat::File, tmp: &str, dest: &str, atime_ns: i64, mtime_ns: i64, mode: u32, attrs: u32) -> Result<(), String> {
    let lp = plat::fs::long_path(tmp);
    let synced = file.sync();
    drop(file);
    if !synced || !plat::rename(&lp, &plat::fs::long_path(dest)) {
        plat::delete(&lp);
        return Err(String::from("konnte nicht an seinen Platz gelegt werden"));
    }
    apply_meta(dest, atime_ns, mtime_ns, mode, attrs);
    Ok(())
}

/// remove a file, a link (never what it points to) or a folder with everything below it (links
/// below are removed as links)
pub fn remove_tree(path: &str) -> Result<(), String> {
    let m = plat::fs::lstat(path).ok_or_else(|| format!("{}: nicht vorhanden", path))?;
    if m.is_dir() && !m.is_symlink() {
        for e in plat::fs::read_dir(path).unwrap_or_default() {
            remove_tree(&plat::fs::join(path, &e.name))?;
        }
    }
    if plat::delete(&plat::fs::long_path(path)) { Ok(()) } else { Err(format!("{} kann nicht gelöscht werden", path)) }
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
