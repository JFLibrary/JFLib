// files -- the files of a device: flash: and nvram: of the IOS-like devices, a small Linux
// tree on the hosts; the records in the project (Device::files) and the store the simulator
// works on (jfn_sim keeps one per device, the services of later milestones -- SNMP, AAA, mail
// -- read and write through it as TFTP and FTP do)
//
//   File { path, data, mtime, dir }      one record of Device::files: a file (bytes) or a
//                                        directory the user made; mtime in seconds since 1970
//                                        on the device's clock
//   Store::for_model(model)              an empty store: the capacity of the model, IOS-like
//                                        or Linux paths, the system image of an IOS-like device
//   load(&[File]) / records()            from the project and back (in path order)
//   norm(path) -> canonical path         "flash:/x" ("flash:x", "x" on IOS), "/srv/tftp/x"
//                                        (relative to /root on Linux; "." and ".." resolved)
//   stat / read / write / append / remove / mkdir / rmdir / remove_tree / list
//   used / free / under(dir)             bytes stored (the image included), what is left, the
//                                        bytes of the files below a directory (quotas)
//   capacity_of(model) / image_of(model) the size limit and the system image of a model
//   is_text(bytes)                       how a record keeps its data
//
// The record of a file in the .jfnet file (every member but "path" may be left out):
//
//   {"path": "flash:/r1-config", "text": "hostname R1\n...", "mtime": 1767225605}
//   {"path": "/srv/tftp/logo.bin", "base64": "iVBORw0KGgo...", "mtime": 1767225700}
//   {"path": "/srv/ftp/upload", "dir": true}
//
// Text that is UTF-8 without control characters (tab, line feed and carriage return allowed)
// is kept as "text", everything else as "base64" (RFC 4648).  A device without files has an
// empty store, so projects from before keep working.
//
// Paths: an IOS-like device has the file systems flash: and nvram: with flat names (nvram:'s
// startup-config is the device's startup configuration, which the command line shows there; it
// is no file of the store); its system image ("jf-r4.bin", the name show version gives) is in
// flash: as a read-only entry of the model's size that takes space but holds no bytes.  A
// Linux host has the directories /, /etc, /home, /home/user, /root, /srv, /srv/ftp, /srv/tftp,
// /tmp, /var and /var/log from the start (they cannot be removed); the files of /etc the shell
// knows (resolv.conf, hosts, hostname) are made from the device's state, not stored.  Names
// are at most 255 bytes, paths 1024; a write that does not fit is refused as a whole.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Field, Json, Value};

use crate::catalog::{CliStyle, DeviceKind, Model};

/// a file or a directory of a device as the project keeps it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub data: Vec<u8>,
    /// seconds since 1970 on the device's clock (0: unknown)
    pub mtime: i64,
    /// a directory (it has no data)
    pub dir: bool,
}

/// the data is text a record keeps as it is: UTF-8 without control characters but tab, line
/// feed and carriage return
pub fn is_text(b: &[u8]) -> bool {
    match core::str::from_utf8(b) {
        Ok(s) => s.chars().all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r')),
        Err(_) => false,
    }
}

impl Json for File {
    fn to_json(&self) -> Value {
        let mut o = agent_core::jmap::empty_object();
        o.insert("path", Value::string(&self.path));
        if self.dir {
            o.insert("dir", Value::Bool(true));
        } else if is_text(&self.data) {
            o.insert("text", Value::string(core::str::from_utf8(&self.data).unwrap_or("")));
        } else {
            o.insert("base64", Value::string(&crypto::b64::encode(&self.data)));
        }
        if self.mtime != 0 {
            o.insert("mtime", Value::i64(self.mtime));
        }
        o
    }
    fn from_json(v: &Value) -> Option<File> {
        v.as_object()?;
        let path = String::take(v.get("path"));
        let dir = bool::take(v.get("dir"));
        let data = match (v.get("text").and_then(Value::as_str), v.get("base64").and_then(Value::as_str)) {
            (Some(t), _) => Vec::from(t.as_bytes()),
            (None, Some(b)) => crypto::b64::decode(b.trim())?,
            (None, None) => Vec::new(),
        };
        Some(File { path, data: if dir { Vec::new() } else { data }, mtime: i64::take(v.get("mtime")), dir })
    }
}

impl Field for File {
    fn put(&self) -> Value {
        self.to_json()
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(File::from_json).unwrap_or_default()
    }
}

/// why the store refused something (errno_text: the Linux wording, ios_text: the IOS-like one)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsErr {
    NotFound,
    IsDir,
    NotDir,
    Exists,
    NoSpace,
    /// a directory of the system, the system image
    ReadOnly,
    BadName,
    NotEmpty,
}

impl FsErr {
    /// strerror's text
    pub fn errno_text(self) -> &'static str {
        match self {
            FsErr::NotFound => "No such file or directory",
            FsErr::IsDir => "Is a directory",
            FsErr::NotDir => "Not a directory",
            FsErr::Exists => "File exists",
            FsErr::NoSpace => "No space left on device",
            FsErr::ReadOnly => "Operation not permitted",
            FsErr::BadName => "File name too long",
            FsErr::NotEmpty => "Directory not empty",
        }
    }
    /// the reason in the parentheses of an IOS-like "%Error ... (...)" line
    pub fn ios_text(self) -> &'static str {
        match self {
            FsErr::NotFound => "No such file or directory",
            FsErr::IsDir => "Is a directory",
            FsErr::NotDir => "Not a directory",
            FsErr::Exists => "File exists",
            FsErr::NoSpace => "No space left on device",
            FsErr::ReadOnly => "Permission denied",
            FsErr::BadName => "Invalid argument",
            FsErr::NotEmpty => "Directory not empty",
        }
    }
}

/// what stat tells about a path
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub size: u64,
    pub mtime: i64,
    pub dir: bool,
    /// a directory of the system or the system image (no record, cannot be removed)
    pub builtin: bool,
}

/// the directories every Linux host has
pub const LINUX_DIRS: [&str; 11] = ["/", "/etc", "/home", "/home/user", "/root", "/srv", "/srv/ftp", "/srv/tftp", "/tmp", "/var", "/var/log"];
/// the file systems of an IOS-like device
pub const IOS_FS: [&str; 2] = ["flash:", "nvram:"];
/// the longest name and path
pub const NAME_MAX: usize = 255;
pub const PATH_MAX: usize = 1024;

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// the bytes a device of the model can store: flash of the IOS-like devices, the disk of hosts
pub fn capacity_of(m: &Model) -> u64 {
    use DeviceKind::*;
    match m.kind {
        Router | Firewall => 256 * MIB,
        L3Switch => 512 * MIB,
        Switch => 128 * MIB,
        AccessPoint => 64 * MIB,
        Pc => 512 * GIB,
        Laptop => 256 * GIB,
        Smartphone | Tablet => 64 * GIB,
        SmartTv => 8 * GIB,
        Printer => GIB,
        IpPhone | IotCamera => 256 * MIB,
        IotSensor => 16 * MIB,
        HomeRouter => 128 * MIB,
        Server | LoadBalancer | Database | Cache | Queue | TrafficGen | ContainerHost => 2048 * GIB,
        Hub | Modem | Cloud | Note => 0,
    }
}

/// the system image of an IOS-like model in flash: its name ("jf-r4.bin", as show version
/// names it) and size
pub fn image_of(m: &Model) -> Option<(String, u64)> {
    if m.cli != CliStyle::Ios {
        return None;
    }
    let size = match m.kind {
        DeviceKind::Router | DeviceKind::Firewall => 40 * MIB + 312 * KIB,
        DeviceKind::L3Switch => 48 * MIB + 168 * KIB,
        DeviceKind::Switch => 24 * MIB + 640 * KIB,
        _ => 16 * MIB + 96 * KIB,
    };
    Some((format!("{}.bin", m.id), size))
}

/// a stored file or directory
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    data: Vec<u8>,
    mtime: i64,
    dir: bool,
}

/// the files of one device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    /// IOS-like paths (flash:, nvram:), else Linux paths
    pub ios: bool,
    pub capacity: u64,
    /// the system image in flash: (name, size)
    pub image: Option<(String, u64)>,
    files: BTreeMap<String, Entry>,
    /// counts the changes (a view may cache what it read)
    pub ver: u32,
}

impl Store {
    /// an empty store of a device of the model
    pub fn for_model(m: &Model) -> Store {
        Store { ios: m.cli == CliStyle::Ios, capacity: capacity_of(m), image: image_of(m), files: BTreeMap::new(), ver: 0 }
    }
    /// an empty store with its own limit (tests, tools)
    pub fn new(ios: bool, capacity: u64, image: Option<(String, u64)>) -> Store {
        Store { ios, capacity, image, files: BTreeMap::new(), ver: 0 }
    }
    /// takes the files of the project (a record with a bad path, or under a missing directory,
    /// is left out; a directory the device has anyway too)
    pub fn load(&mut self, files: &[File]) {
        self.files.clear();
        let mut v: Vec<&File> = files.iter().collect();
        // directories before what is in them
        v.sort_by(|a, b| a.path.len().cmp(&b.path.len()).then(a.path.cmp(&b.path)));
        for f in v {
            let Ok(p) = self.norm(&f.path) else { continue };
            if self.builtin_dir(&p) || self.is_image(&p) || !self.parent_ok(&p) {
                continue;
            }
            self.files.insert(p, Entry { data: if f.dir { Vec::new() } else { f.data.clone() }, mtime: f.mtime, dir: f.dir });
        }
        self.ver = self.ver.wrapping_add(1);
    }
    /// the records for the project, in path order
    pub fn records(&self) -> Vec<File> {
        self.files.iter().map(|(p, e)| File { path: p.clone(), data: e.data.clone(), mtime: e.mtime, dir: e.dir }).collect()
    }
    /// how many records
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    // ---------------------------------------------------------------- paths

    /// the canonical path: IOS-like "flash:/name" (a bare name is in flash:), Linux an
    /// absolute path without "." and ".." (a relative one starts at /root)
    pub fn norm(&self, path: &str) -> Result<String, FsErr> {
        let path = path.trim();
        if path.is_empty() || path.len() > PATH_MAX || path.contains('\0') {
            return Err(FsErr::BadName);
        }
        if self.ios {
            let (fs, rest) = match path.find(':') {
                Some(i) => (&path[..=i], &path[i + 1..]),
                None => ("flash:", path),
            };
            let Some(fs) = IOS_FS.iter().find(|x| x.eq_ignore_ascii_case(fs)) else {
                return Err(FsErr::NotFound);
            };
            let name = rest.trim_start_matches('/');
            if name.contains(['/', ':']) || name.contains(char::is_whitespace) || name.len() > NAME_MAX {
                return Err(FsErr::BadName);
            }
            return Ok(format!("{}/{}", fs, name));
        }
        let mut parts: Vec<&str> = Vec::new();
        let full = if path.starts_with('/') { String::from(path) } else { format!("/root/{}", path) };
        for c in full.split('/') {
            match c {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c if c.len() > NAME_MAX => return Err(FsErr::BadName),
                c => parts.push(c),
            }
        }
        Ok(format!("/{}", parts.join("/")))
    }
    /// the directory a canonical path is in ("flash:/" for "flash:/x", "/srv" for "/srv/tftp")
    pub fn parent(&self, p: &str) -> String {
        if self.ios {
            return match p.find(":/") {
                Some(i) => String::from(&p[..i + 2]),
                None => String::from(p),
            };
        }
        match p.rfind('/') {
            Some(0) | None => String::from("/"),
            Some(i) => String::from(&p[..i]),
        }
    }
    /// the last part of a canonical path
    pub fn name_of<'a>(&self, p: &'a str) -> &'a str {
        if self.ios {
            return p.split_once(":/").map_or(p, |(_, n)| n);
        }
        p.rsplit('/').next().unwrap_or(p)
    }
    fn builtin_dir(&self, p: &str) -> bool {
        if self.ios { IOS_FS.iter().any(|fs| p.len() == fs.len() + 1 && p.starts_with(fs) && p.ends_with('/')) } else { LINUX_DIRS.contains(&p) }
    }
    fn is_image(&self, p: &str) -> bool {
        self.image.as_ref().is_some_and(|(n, _)| p.strip_prefix("flash:/").is_some_and(|x| x.eq_ignore_ascii_case(n)))
    }
    fn parent_ok(&self, p: &str) -> bool {
        let d = self.parent(p);
        self.builtin_dir(&d) || self.files.get(&d).is_some_and(|e| e.dir)
    }

    // ---------------------------------------------------------------- questions

    /// what is at a path (canonical or not)
    pub fn stat(&self, path: &str) -> Option<Stat> {
        let p = self.norm(path).ok()?;
        if self.builtin_dir(&p) {
            return Some(Stat { size: 0, mtime: 0, dir: true, builtin: true });
        }
        if self.is_image(&p) {
            return self.image.as_ref().map(|(_, s)| Stat { size: *s, mtime: 0, dir: false, builtin: true });
        }
        self.files.get(&p).map(|e| Stat { size: e.data.len() as u64, mtime: e.mtime, dir: e.dir, builtin: false })
    }
    pub fn exists(&self, path: &str) -> bool {
        self.stat(path).is_some()
    }
    pub fn is_dir(&self, path: &str) -> bool {
        self.stat(path).is_some_and(|s| s.dir)
    }
    /// the bytes of a file (the system image has none to give: ReadOnly)
    pub fn read(&self, path: &str) -> Result<&[u8], FsErr> {
        let p = self.norm(path)?;
        if self.builtin_dir(&p) {
            return Err(FsErr::IsDir);
        }
        if self.is_image(&p) {
            return Err(FsErr::ReadOnly);
        }
        match self.files.get(&p) {
            Some(e) if e.dir => Err(FsErr::IsDir),
            Some(e) => Ok(&e.data),
            None => Err(FsErr::NotFound),
        }
    }
    /// the entries of a directory: name and stat, in name order (IOS-like: the image first)
    pub fn list(&self, dir: &str) -> Result<Vec<(String, Stat)>, FsErr> {
        let d = self.norm(dir)?;
        match self.stat(&d) {
            None => return Err(FsErr::NotFound),
            Some(s) if !s.dir => return Err(FsErr::NotDir),
            Some(_) => {}
        }
        let mut v: Vec<(String, Stat)> = Vec::new();
        if self.ios && d == "flash:/" {
            if let Some((n, s)) = &self.image {
                v.push((n.clone(), Stat { size: *s, mtime: 0, dir: false, builtin: true }));
            }
        }
        if !self.ios {
            for b in LINUX_DIRS.iter().filter(|b| **b != "/" && self.parent(b) == d) {
                v.push((String::from(self.name_of(b)), Stat { size: 0, mtime: 0, dir: true, builtin: true }));
            }
        }
        for (p, e) in &self.files {
            if self.parent(p) == d {
                v.push((String::from(self.name_of(p)), Stat { size: e.data.len() as u64, mtime: e.mtime, dir: e.dir, builtin: false }));
            }
        }
        if self.ios {
            v[usize::from(d == "flash:/" && self.image.is_some())..].sort_by(|a, b| a.0.cmp(&b.0));
        } else {
            v.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Ok(v)
    }
    /// bytes stored, the system image included
    pub fn used(&self) -> u64 {
        self.files.values().map(|e| e.data.len() as u64).sum::<u64>() + self.image.as_ref().map_or(0, |i| i.1)
    }
    /// bytes left
    pub fn free(&self) -> u64 {
        self.capacity.saturating_sub(self.used())
    }
    /// the bytes of the files at or below a directory
    pub fn under(&self, dir: &str) -> u64 {
        let Ok(d) = self.norm(dir) else { return 0 };
        let pre = if d.ends_with('/') { d.clone() } else { format!("{}/", d) };
        self.files.iter().filter(|(p, _)| p.starts_with(&pre)).map(|(_, e)| e.data.len() as u64).sum()
    }
    /// the path lies at or below a directory (both canonical)
    pub fn within(&self, p: &str, dir: &str) -> bool {
        p == dir || p.starts_with(&if dir.ends_with('/') { String::from(dir) } else { format!("{}/", dir) })
    }

    // ---------------------------------------------------------------- changes

    /// a file with these bytes (made or replaced): its parent must be a directory, and the
    /// device must have room for the difference
    pub fn write(&mut self, path: &str, data: &[u8], mtime: i64) -> Result<(), FsErr> {
        let p = self.norm(path)?;
        if self.builtin_dir(&p) {
            return Err(FsErr::IsDir);
        }
        if self.is_image(&p) {
            return Err(FsErr::ReadOnly);
        }
        if self.ios && self.name_of(&p).is_empty() {
            return Err(FsErr::BadName);
        }
        let old = match self.files.get(&p) {
            Some(e) if e.dir => return Err(FsErr::IsDir),
            Some(e) => e.data.len() as u64,
            None => 0,
        };
        if !self.parent_ok(&p) {
            return Err(if self.exists(&self.parent(&p)) { FsErr::NotDir } else { FsErr::NotFound });
        }
        if data.len() as u64 > old && data.len() as u64 - old > self.free() {
            return Err(FsErr::NoSpace);
        }
        self.files.insert(p, Entry { data: Vec::from(data), mtime, dir: false });
        self.ver = self.ver.wrapping_add(1);
        Ok(())
    }
    /// more bytes at the end of a file (made when it is missing)
    pub fn append(&mut self, path: &str, data: &[u8], mtime: i64) -> Result<(), FsErr> {
        let mut all = match self.read(path) {
            Ok(b) => Vec::from(b),
            Err(FsErr::NotFound) => Vec::new(),
            Err(e) => return Err(e),
        };
        all.extend_from_slice(data);
        self.write(path, &all, mtime)
    }
    /// removes a file
    pub fn remove(&mut self, path: &str) -> Result<(), FsErr> {
        let p = self.norm(path)?;
        if self.builtin_dir(&p) {
            return Err(FsErr::IsDir);
        }
        if self.is_image(&p) {
            return Err(FsErr::ReadOnly);
        }
        match self.files.get(&p) {
            Some(e) if e.dir => Err(FsErr::IsDir),
            Some(_) => {
                self.files.remove(&p);
                self.ver = self.ver.wrapping_add(1);
                Ok(())
            }
            None => Err(FsErr::NotFound),
        }
    }
    /// makes a directory (Linux; `parents` as mkdir -p: the missing ones on the way too, and
    /// no error when it is there)
    pub fn mkdir(&mut self, path: &str, mtime: i64, parents: bool) -> Result<(), FsErr> {
        let p = self.norm(path)?;
        if self.ios {
            return Err(FsErr::ReadOnly);
        }
        if let Some(s) = self.stat(&p) {
            return if parents && s.dir { Ok(()) } else { Err(FsErr::Exists) };
        }
        if !self.parent_ok(&p) {
            let d = self.parent(&p);
            if !parents || self.exists(&d) {
                return Err(if self.exists(&d) { FsErr::NotDir } else { FsErr::NotFound });
            }
            self.mkdir(&d, mtime, true)?;
        }
        self.files.insert(p, Entry { data: Vec::new(), mtime, dir: true });
        self.ver = self.ver.wrapping_add(1);
        Ok(())
    }
    /// removes an empty directory the user made
    pub fn rmdir(&mut self, path: &str) -> Result<(), FsErr> {
        let p = self.norm(path)?;
        if self.builtin_dir(&p) {
            return Err(FsErr::ReadOnly);
        }
        match self.files.get(&p) {
            Some(e) if !e.dir => return Err(FsErr::NotDir),
            None => return Err(FsErr::NotFound),
            Some(_) => {}
        }
        if self.files.keys().any(|k| k != &p && self.within(k, &p)) {
            return Err(FsErr::NotEmpty);
        }
        self.files.remove(&p);
        self.ver = self.ver.wrapping_add(1);
        Ok(())
    }
    /// removes a directory the user made with everything in it (rm -r)
    pub fn remove_tree(&mut self, path: &str) -> Result<(), FsErr> {
        let p = self.norm(path)?;
        if self.builtin_dir(&p) {
            return Err(FsErr::ReadOnly);
        }
        if !self.files.contains_key(&p) {
            return Err(FsErr::NotFound);
        }
        let gone: Vec<String> = self.files.keys().filter(|k| self.within(k, &p)).cloned().collect();
        for k in gone {
            self.files.remove(&k);
        }
        self.ver = self.ver.wrapping_add(1);
        Ok(())
    }
}
