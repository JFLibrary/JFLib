// store -- where a repository keeps its objects: the Store trait and Folder, a folder of the
// file system (plat::fs, so also a vfs mount such as mem://)
//
//   Store                 init, lock, get, get_range, put, delete, list, location: objects by
//                         kind and name, the way S3, SFTP or WebDAV keep them
//   Kind                  Config ("repo"), Pack, Index, Snapshot
//   Folder::new(path)     repo.json  data/<xx>/<pack>.pack  index/<pack>.idx
//                         snapshots/<id>.snap  LOCK
//
// What the repository asks of a store (Repo::create_in / open_in):
//   - put is atomic and durable: once it returns Ok, get and list see the whole object, also
//     after a crash, and a put that failed leaves the old object or none (the commit order of a
//     backup -- packs, indexes, snapshot -- rests on it)
//   - a delete that returned Ok stays done; deleting what is not there is Ok
//   - get and list see every put that returned before them (read after write)
//   - lock keeps a second holder out, also one of another process or host, until the guard
//     is dropped
// Nothing that comes back from a store is trusted: packs, indexes and snapshots are sealed and
// repo.json carries a MAC, so a store that changes, swaps or replays objects is found out (a
// rollback to older objects of the same repository is not); names and sizes of a listing are
// checked before use.  A store sees the names, the sizes and when objects come and go: pack
// and index names are random, snapshot names are the ids given to save_snapshot.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use plat::sync::Mutex;

use crate::{CasError, CasResult};

/// what an object of a repository is
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// the repository's settings and wrapped keys; the repository uses one, named "repo"
    Config,
    /// a pack of sealed chunks (written once, read in ranges, deleted by gc)
    Pack,
    /// the sealed index of the pack of the same name
    Index,
    /// a sealed and signed snapshot, named by its id
    Snapshot,
}

/// what [`Store::lock`] hands out: the repository is held until it is dropped
pub type StoreLock = Box<dyn Any + Send + Sync>;

/// Where a repository keeps its objects.  A name is not empty and has none of '/', '\\', '.',
/// ':' (pack and index names are hex, snapshot names the ids given to save_snapshot).  The
/// rules a store keeps are those in the header of this module: put atomic and durable, read
/// after write, a lock that keeps every other holder out.
pub trait Store: Send + Sync {
    /// where it is, for messages and Repo::path ("D:/Backups/repo", "s3://bucket/prefix")
    fn location(&self) -> String;
    /// make the place of a new repository: NotEmpty when it already holds anything
    fn init(&self) -> CasResult<()>;
    /// hold the repository (InUse while another holder has it)
    fn lock(&self) -> CasResult<StoreLock>;
    /// the whole object; Ok(None) when there is none
    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>>;
    /// `len` bytes from `off` on, fewer only where the object ends (NotFound when there is
    /// none).  The default reads the whole object: a store with ranged reads does better
    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> {
        let all = self.get(kind, name)?.ok_or_else(|| CasError::NotFound(String::from(name)))?;
        let start = usize::try_from(off).unwrap_or(usize::MAX).min(all.len());
        let end = start.saturating_add(len).min(all.len());
        Ok(Vec::from(&all[start..end]))
    }
    /// store an object (replacing one of the same name), atomic and durable
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()>;
    /// remove an object; one that is not there is fine
    fn delete(&self, kind: Kind, name: &str) -> CasResult<()>;
    /// the names and sizes of the objects of a kind, in any order
    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>>;
}

impl<S: Store + ?Sized> Store for Box<S> {
    fn location(&self) -> String {
        (**self).location()
    }
    fn init(&self) -> CasResult<()> {
        (**self).init()
    }
    fn lock(&self) -> CasResult<StoreLock> {
        (**self).lock()
    }
    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>> {
        (**self).get(kind, name)
    }
    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> {
        (**self).get_range(kind, name, off, len)
    }
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()> {
        (**self).put(kind, name, data)
    }
    fn delete(&self, kind: Kind, name: &str) -> CasResult<()> {
        (**self).delete(kind, name)
    }
    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>> {
        (**self).list(kind)
    }
}

impl<S: Store + ?Sized> Store for Arc<S> {
    fn location(&self) -> String {
        (**self).location()
    }
    fn init(&self) -> CasResult<()> {
        (**self).init()
    }
    fn lock(&self) -> CasResult<StoreLock> {
        (**self).lock()
    }
    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>> {
        (**self).get(kind, name)
    }
    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> {
        (**self).get_range(kind, name, off, len)
    }
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()> {
        (**self).put(kind, name, data)
    }
    fn delete(&self, kind: Kind, name: &str) -> CasResult<()> {
        (**self).delete(kind, name)
    }
    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>> {
        (**self).list(kind)
    }
}

/// whether a store may be asked for this name (Store's rule)
pub fn name_ok(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', '.', ':', '\0'])
}

/// open pack files a Folder keeps (reads of chunks come in ranges, often of the same pack)
const OPEN_FILES: usize = 32;

/// A repository in a folder of the file system: repo.json, data/<xx>/<pack>.pack (the first
/// two hex digits), index/<pack>.idx, snapshots/<id>.snap and LOCK (plat::FileLock: one
/// holder at a time on this machine).  Files and folders are the owner's only
/// (write_file_private, mkdir_private); a write goes to a temporary sibling that is synced
/// and renamed into place.  The path can be a vfs name too (mem://, a mount).
pub struct Folder {
    path: String,
    files: Mutex<Vec<(String, Arc<plat::File>)>>,
}

impl Folder {
    pub fn new(path: &str) -> Folder {
        Folder { path: String::from(path), files: Mutex::new(Vec::new()) }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn join(&self, a: &str) -> String {
        plat::fs::join(&self.path, a)
    }

    fn dir(&self, kind: Kind) -> String {
        match kind {
            Kind::Config => self.path.clone(),
            Kind::Pack => self.join("data"),
            Kind::Index => self.join("index"),
            Kind::Snapshot => self.join("snapshots"),
        }
    }

    fn file(&self, kind: Kind, name: &str) -> CasResult<String> {
        // (a pack goes into the folder of its first two bytes: they must be whole letters,
        // name_ok lets "€a" through, and &name[..2] cut the euro sign in two)
        if !name_ok(name) || (kind == Kind::Pack && !name.is_char_boundary(2)) {
            return Err(CasError::Io(format!("invalid name {:?}", name)));
        }
        Ok(match kind {
            Kind::Config => self.join(&format!("{}.json", name)),
            Kind::Pack => plat::fs::join(&plat::fs::join(&self.dir(Kind::Pack), &name[..2]), &format!("{}.pack", name)),
            Kind::Index => plat::fs::join(&self.dir(kind), &format!("{}.idx", name)),
            Kind::Snapshot => plat::fs::join(&self.dir(kind), &format!("{}.snap", name)),
        })
    }

    fn open(&self, path: &str) -> CasResult<Option<Arc<plat::File>>> {
        let mut files = self.files.lock();
        if let Some(i) = files.iter().position(|(p, _)| p == path) {
            let e = files.remove(i);
            let f = e.1.clone();
            files.push(e);
            return Ok(Some(f));
        }
        let Some(f) = plat::File::open(&plat::fs::long_path(path), plat::FO_READ) else {
            return if missing(path) { Ok(None) } else { Err(io("cannot open", path)) };
        };
        let f = Arc::new(f);
        if files.len() >= OPEN_FILES {
            files.remove(0);
        }
        files.push((String::from(path), f.clone()));
        Ok(Some(f))
    }

    /// the entries of a folder; a folder that is not there has none
    fn entries(dir: &str) -> CasResult<Vec<plat::fs::DirEntry>> {
        match plat::fs::read_dir(dir) {
            Some(l) => Ok(l),
            None if missing(dir) => Ok(Vec::new()),
            None => Err(io("cannot list", dir)),
        }
    }

    fn listed(&self, dir: &str, suffix: &str, out: &mut Vec<(String, u64)>) -> CasResult<()> {
        for e in Folder::entries(dir)? {
            let Some(stem) = e.name.strip_suffix(suffix) else { continue };
            // temporary siblings (NAME.tmpN) and anything else end differently or have a '.'
            if e.kind == plat::fs::FileKind::Dir || !name_ok(stem) {
                continue;
            }
            let p = plat::fs::join(dir, &e.name);
            let Some(m) = plat::fs::stat(&p) else { continue }; // (gone since the listing)
            out.push((String::from(stem), m.size));
        }
        Ok(())
    }
}

fn io(what: &str, path: &str) -> CasError {
    CasError::Io(format!("{} {}", what, path))
}

// nothing there (not: there, but unreadable)
fn missing(path: &str) -> bool {
    plat::file_exists_checked(path) == Some(0)
}

impl Store for Folder {
    fn location(&self) -> String {
        self.path.clone()
    }

    fn init(&self) -> CasResult<()> {
        if plat::fs::read_dir(&self.path).is_some_and(|l| !l.is_empty()) {
            return Err(CasError::NotEmpty);
        }
        for p in [self.path.clone(), self.dir(Kind::Pack), self.dir(Kind::Index), self.dir(Kind::Snapshot)] {
            if !plat::fs::mkdir_private(&p) {
                return Err(io("cannot create", &p));
            }
        }
        Ok(())
    }

    fn lock(&self) -> CasResult<StoreLock> {
        let l = plat::FileLock::acquire(&self.join("LOCK")).ok_or(CasError::InUse)?;
        Ok(Box::new(l))
    }

    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>> {
        let p = self.file(kind, name)?;
        match plat::read_file(&p) {
            Some(b) => Ok(Some(b)),
            None if missing(&p) => Ok(None),
            None => Err(io("cannot read", &p)),
        }
    }

    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> {
        let p = self.file(kind, name)?;
        let f = self.open(&p)?.ok_or_else(|| CasError::NotFound(p.clone()))?;
        let mut buf = alloc::vec![0u8; len];
        let mut done = 0;
        while done < len {
            match f.read_at(off.saturating_add(done as u64), &mut buf[done..]) {
                Some(0) => break,
                Some(n) => done += n,
                None => return Err(io("cannot read", &p)),
            }
        }
        buf.truncate(done);
        Ok(buf)
    }

    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()> {
        let p = self.file(kind, name)?;
        if kind == Kind::Pack {
            let dir = String::from(plat::fs::parent(&p));
            if !plat::fs::mkdir_private(&dir) {
                return Err(io("cannot create", &dir));
            }
        }
        // (a cached handle would read the replaced file)
        self.files.lock().retain(|(q, _)| *q != p);
        if !plat::fs::write_file_private(&p, data) {
            return Err(io("cannot write", &p));
        }
        Ok(())
    }

    fn delete(&self, kind: Kind, name: &str) -> CasResult<()> {
        let p = self.file(kind, name)?;
        // an open handle keeps Windows from deleting the file
        self.files.lock().retain(|(q, _)| *q != p);
        if !plat::delete(&p) && !missing(&p) {
            return Err(io("cannot delete", &p));
        }
        plat::fs::sync_dir(plat::fs::parent(&p));
        Ok(())
    }

    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>> {
        let mut out = Vec::new();
        match kind {
            Kind::Pack => {
                let data = self.dir(Kind::Pack);
                for d in Folder::entries(&data)? {
                    if d.kind == plat::fs::FileKind::Dir {
                        let at = out.len();
                        self.listed(&plat::fs::join(&data, &d.name), ".pack", &mut out)?;
                        // only where get finds them: below their first two characters
                        let mut k = at;
                        while k < out.len() {
                            if out[k].0.get(..2) == Some(d.name.as_str()) {
                                k += 1;
                            } else {
                                out.swap_remove(k);
                            }
                        }
                    }
                }
            }
            Kind::Config => self.listed(&self.path, ".json", &mut out)?,
            Kind::Index => self.listed(&self.dir(kind), ".idx", &mut out)?,
            Kind::Snapshot => self.listed(&self.dir(kind), ".snap", &mut out)?,
        }
        Ok(out)
    }
}
