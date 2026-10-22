// content -- the documents of other apps as files: Android's Storage Access Framework
// ("content://" names) behind the vfs; the name logic runs on every platform, so the desktop
// tests it with a host of its own
//
//   ContentFs::new(host) -> a vfs Provider for the names after "content://".  On Android the
//        vfs mounts it on first use with the ContentResolver as its host (android_host.rs,
//        with_journal in the app's data folder, recover() at once); elsewhere only a test
//        mounts it (apps/plattest/t_content.rs: a host in memory).
//   .with_journal(dir)   writes go through a copy in dir first (below); recover() finishes the
//        writes a crash interrupted -> how many
//   DocHost (trait)   what the provider asks the system: query (one document), children (of a
//        folder document), create, delete, rename, open (a Stream: Android a file descriptor),
//        generation (a counter the system bumps when a provider reports a change)
//   DocInfo           uri, display name, folder?, size, mtime, flags (DOC_*)
//   uri_of(path)      "content://" + path -- the document's own URI where the name is one
//
// Names (the path after "content://"; the file dialog hands them out, apps join names below
// a chosen folder as with any other folder):
//   AUTH/document/ID            one document (ACTION_OPEN_DOCUMENT / ACTION_CREATE_DOCUMENT):
//                               read, write, File streams, stat, display name -- no siblings
//   AUTH/tree/ID                a folder the user chose (ACTION_OPEN_DOCUMENT_TREE)
//   AUTH/tree/ID/a/b.txt        below it by display names (JFLib's own form: every name is
//                               looked up among the children of the folder before it)
//   AUTH/tree/ID/document/ID2   Android's own form of a document inside a tree
//   anything else               handed to the host as one document
// Inside a tree mkdir, write (new files are created), delete (files, empty folders; never the
// chosen folder itself), rename (inside one folder, replacing; to another folder: copy and
// delete, files only) and dir_list work.
//
// Names below a tree are cached: name -> the document's URI (and folder?), filled by every walk
// and listing, forgotten on create / delete / rename through the provider, emptied when the
// host's generation changes (Android: a ContentObserver on the provider's authority) and
// unused after CACHE_TTL_US.  A cached document is asked once (query) before it is used: gone,
// renamed or no longer a file / folder, the name is walked again from the nearest cached folder
// (from the tree's root at worst).  So a name costs one query instead of one listing per
// folder; only the leaf is checked, a folder moved elsewhere by another app stays cached until
// a change is reported or its time is up.
//
// Writing: SAF has no rename over an existing document (providers pick "name (1)", and a new
// document would get a new ID, losing the grants), so the provider says atomic_rename() ==
// false: AtomicFile (write_file in pieces, aio) collects the bytes and writes the document in
// place.  rename replaces by deleting the target first.  write (write_file, AtomicFile's commit)
// is made as safe as SAF allows:
//   1. with a journal folder, the new bytes (and the URI) go there first: one file per document
//      (JOURNAL, written atomically on internal storage);
//   2. the document is opened truncated ("rwt" / "wt") and written, synced, then read back and
//      compared (size and every byte) -- once more if that fails;
//   3. the journal file goes.  A crash or a failed write leaves it: recover() (Android: when
//      content:// is first used in a process) writes it again into the document if it still
//      exists (else the copy is dropped).
// The document keeps its URI (grants stay valid).  What remains: between the truncation and the
// end of the write other readers see a part; without a journal (none set) a crash leaves that
// part; File streams (File::open) write in place without any of this; opening a document for
// writing, deleting or renaming it drops its pending journal copy.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::vfs::{Opened, Provider, Stat, Stream};
use crate::{FO_READ, FO_RW, FO_WRITE};

/// DocInfo::flags: the document can be written
pub const DOC_WRITE: u32 = 1;
/// DocInfo::flags: the document can be deleted
pub const DOC_DELETE: u32 = 2;
/// DocInfo::flags: the document can be renamed
pub const DOC_RENAME: u32 = 4;
/// DocInfo::flags: the folder takes new documents
pub const DOC_CREATE: u32 = 8;

/// how long a cached name is used without a reported change (µs)
pub const CACHE_TTL_US: u64 = 60_000_000;
// names kept at most (then the cache starts again)
const CACHE_MAX: usize = 4096;
// a journal file: "JFCJ", u32 URI length, URI, u64 length, bytes, u64 FNV-1a of the bytes
const JOURNAL: &[u8; 4] = b"JFCJ";

// (tests) folder listings a walk asked for; the byte count after which the next write stops
static LISTINGS: AtomicU64 = AtomicU64::new(0);
static INTERRUPT: AtomicU64 = AtomicU64::new(u64::MAX);

/// (tests) how many folder listings (DocHost::children) the walks of every ContentFs asked for
#[doc(hidden)]
pub fn listings() -> u64 {
    LISTINGS.load(Ordering::Relaxed)
}

/// (tests) the next write of a document stops after n bytes and fails, its journal copy kept
/// -- what a crash in the middle of a write leaves
#[doc(hidden)]
pub fn interrupt_next_write(n: u64) {
    INTERRUPT.store(n, Ordering::Relaxed);
}

/// one document as the host describes it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocInfo {
    /// the document's URI ("content://AUTH/document/ID", inside a tree
    /// "content://AUTH/tree/T/document/ID")
    pub uri: String,
    /// what the user sees (OpenableColumns.DISPLAY_NAME)
    pub name: String,
    pub dir: bool,
    /// bytes; None when the provider does not know
    pub size: Option<u64>,
    /// ns since 1970, 0 = unknown
    pub mtime_ns: i64,
    /// DOC_*
    pub flags: u32,
}

/// what ContentFs asks the system (Android: the ContentResolver and DocumentsContract)
pub trait DocHost: Send + Sync {
    /// the document behind uri (a tree's URI: its root folder) -> None when it is missing or
    /// not allowed
    fn query(&self, uri: &str) -> Option<DocInfo>;
    /// the documents in the folder document uri (a URI of DocInfo::uri inside a tree)
    fn children(&self, uri: &str) -> Option<Vec<DocInfo>>;
    /// a new document (dir: a folder) named name in the folder parent -> its description
    fn create(&self, parent: &str, name: &str, dir: bool) -> Option<DocInfo>;
    fn delete(&self, uri: &str) -> bool;
    /// the document under a new display name in its folder -> its description (the URI may
    /// change; a provider may pick another name when name is taken)
    fn rename(&self, uri: &str, name: &str) -> Option<DocInfo>;
    /// the document opened: FO_READ, FO_WRITE (truncated) or FO_RW
    fn open(&self, uri: &str, mode: u32) -> Option<Box<dyn Stream>>;
    /// a counter that changes when a provider reports a change of its documents (the name
    /// cache starts again then); 0 = never
    fn generation(&self) -> u64 {
        0
    }
}

/// "content://" + path
pub fn uri_of(path: &str) -> String {
    format!("content://{path}")
}

// what a name inside the provider means
enum Name<'a> {
    /// a document by its URI
    Doc(String),
    /// a tree: its name ("AUTH/tree/ID", the cache's key), its root URI and the display names
    /// below it
    Tree(&'a str, String, Vec<&'a str>),
}

fn parse(path: &str) -> Option<Name<'_>> {
    if path.is_empty() {
        return None;
    }
    let segs: Vec<&str> = path.split('/').collect();
    if segs.len() >= 3 && segs[1] == "tree" {
        if segs.len() == 5 && segs[3] == "document" {
            return Some(Name::Doc(uri_of(path)));
        }
        let base = &path[..segs[0].len() + segs[1].len() + segs[2].len() + 2];
        return Some(Name::Tree(base, uri_of(base), segs[3..].to_vec()));
    }
    Some(Name::Doc(uri_of(path)))
}

// the cache's key of names below a tree
fn key(base: &str, names: &[&str]) -> String {
    let mut k = String::from(base);
    for n in names {
        k.push('/');
        k.push_str(n);
    }
    k
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x0100_0000_01b3))
}

#[derive(Clone)]
struct Cached {
    uri: String,
    dir: bool,
    at: u64,
}

#[derive(Default)]
struct Cache {
    seen: u64,
    map: BTreeMap<String, Cached>,
}

/// the documents of other apps behind "content://" (see the file header)
pub struct ContentFs<H: DocHost> {
    host: H,
    journal: Option<String>,
    cache: SpinLock<Cache>,
}

impl<H: DocHost> ContentFs<H> {
    pub fn new(host: H) -> ContentFs<H> {
        ContentFs { host, journal: None, cache: SpinLock::new(Cache::default()) }
    }

    /// keep a copy of every write in dir (a folder of the app's own, any plat name) until the
    /// document holds all of it; recover() finishes what a crash interrupted
    pub fn with_journal(mut self, dir: &str) -> ContentFs<H> {
        self.journal = Some(String::from(dir.trim_end_matches('/')));
        self
    }

    /// the host the provider asks
    pub fn host(&self) -> &H {
        &self.host
    }

    /// the document a name (after "content://") means -- its own URI, display name, size
    pub fn document(&self, path: &str) -> Option<DocInfo> {
        self.lookup(path)
    }

    // ------------------------------------------------------------ the name cache

    // the cache, emptied when the host reported a change
    fn cache(&self) -> rt::sync::Guard<'_, Cache> {
        let now = self.host.generation();
        let mut c = self.cache.lock();
        if c.seen != now {
            c.seen = now;
            c.map.clear();
        }
        c
    }

    fn cached(&self, key: &str) -> Option<Cached> {
        let now = crate::time_us();
        let mut c = self.cache();
        match c.map.get(key) {
            Some(e) if now.saturating_sub(e.at) < CACHE_TTL_US => Some(e.clone()),
            Some(_) => {
                c.map.remove(key);
                None
            }
            None => None,
        }
    }

    fn remember(&self, key: String, d: &DocInfo) {
        let at = crate::time_us();
        let mut c = self.cache();
        if c.map.len() >= CACHE_MAX {
            c.map.clear();
        }
        c.map.insert(key, Cached { uri: d.uri.clone(), dir: d.dir, at });
    }

    // a name and everything below it
    fn forget(&self, key: &str) {
        let below = format!("{key}/");
        self.cache().map.retain(|k, _| k != key && !k.starts_with(&below));
    }

    // the document behind a tree root and names (each one a child of the one before): from the
    // cache when it has the name and the document still answers to it, else walked from the
    // nearest cached folder (the root at worst)
    fn walk(&self, base: &str, root: &str, names: &[&str]) -> Option<DocInfo> {
        let full = key(base, names);
        if let Some(c) = self.cached(&full) {
            let fits = |d: &DocInfo| d.dir == c.dir && names.last().is_none_or(|n| d.name == *n);
            match self.host.query(&c.uri) {
                Some(d) if fits(&d) => return Some(d),
                _ => self.forget(&full),
            }
        }
        // the nearest cached folder above the name
        let start = (1..names.len()).rev().find_map(|k| self.cached(&key(base, &names[..k])).filter(|c| c.dir).map(|c| (k, c.uri)));
        if let Some((k, uri)) = start {
            match self.walk_from(base, names, k, uri.clone()) {
                Ok(d) => return d,
                // a name missing below the cached folder counts once that folder still answers
                // to its name; gone or another one by now: from the root
                Err(true) if self.host.query(&uri).is_some_and(|a| a.dir && a.name == names[k - 1]) => return None,
                Err(_) => self.forget(&key(base, &names[..k])),
            }
        }
        let r = self.host.query(root)?;
        self.remember(String::from(base), &r);
        if names.is_empty() || !r.dir {
            return names.is_empty().then_some(r);
        }
        self.walk_from(base, names, 0, r.uri).unwrap_or(None)
    }

    // names[k..] below the folder uri (names[..k]), each folder listed (its children cached) ->
    // Ok(the document / None: a file on the way), Err(true): a name missing, Err(false): the
    // folder at k cannot be listed
    fn walk_from(&self, base: &str, names: &[&str], k: usize, mut uri: String) -> Result<Option<DocInfo>, bool> {
        for i in k..names.len() {
            LISTINGS.fetch_add(1, Ordering::Relaxed);
            let Some(kids) = self.host.children(&uri) else { return if i == k { Err(false) } else { Ok(None) } };
            let mut next = None;
            for c in kids {
                let mut at = key(base, &names[..i]);
                at.push('/');
                at.push_str(&c.name);
                self.remember(at, &c);
                if c.name == names[i] {
                    next = Some(c);
                }
            }
            let Some(d) = next else { return Err(true) };
            if i + 1 == names.len() {
                return Ok(Some(d));
            }
            if !d.dir {
                return Ok(None);
            }
            uri = d.uri;
        }
        Ok(None)
    }

    // the document a name means
    fn lookup(&self, path: &str) -> Option<DocInfo> {
        match parse(path)? {
            Name::Doc(uri) => self.host.query(&uri),
            Name::Tree(base, root, names) => self.walk(base, &root, &names),
        }
    }

    // a new document at path (a tree name; its folder must exist) -> its description
    fn create_at(&self, path: &str, dir: bool) -> Option<DocInfo> {
        let Name::Tree(base, root, names) = parse(path)? else { return None };
        let (name, up) = names.split_last()?;
        let parent = self.walk(base, &root, up)?;
        if !parent.dir {
            return None;
        }
        let d = self.host.create(&parent.uri, name, dir)?;
        if d.name == *name {
            self.remember(key(base, &names), &d);
        }
        Some(d)
    }

    // the folder a tree name is in -> (root, parent names, own name)
    fn split<'a>(&self, path: &'a str) -> Option<(String, Vec<&'a str>, &'a str)> {
        let Name::Tree(_, root, mut names) = parse(path)? else { return None };
        let name = names.pop()?;
        Some((root, names, name))
    }

    // the cache's key of a tree name (None: no tree name)
    fn key_of(path: &str) -> Option<String> {
        match parse(path)? {
            Name::Tree(base, _, names) => Some(key(base, &names)),
            Name::Doc(_) => None,
        }
    }

    // ------------------------------------------------------------ reading and writing

    fn read_all(&self, uri: &str) -> Option<Vec<u8>> {
        let s = self.host.open(uri, FO_READ)?;
        let mut out = Vec::new();
        if let Some(n) = s.size() {
            out.try_reserve(usize::try_from(n).ok()?).ok()?;
        }
        let mut buf = alloc::vec![0u8; 64 * 1024];
        loop {
            let n = s.read_at(out.len() as u64, &mut buf)?;
            if n == 0 {
                return Some(out);
            }
            out.try_reserve(n).ok()?;
            out.extend_from_slice(&buf[..n]);
        }
    }

    fn write_all(&self, uri: &str, data: &[u8]) -> bool {
        let Some(s) = self.host.open(uri, FO_WRITE) else { return false };
        let mut off = 0;
        while off < data.len() {
            match s.write_at(off as u64, &data[off..]) {
                Some(n) if n > 0 => off += n,
                _ => return false,
            }
        }
        s.sync()
    }

    // the document holds exactly data (read back: size and every byte)
    fn verify(&self, uri: &str, data: &[u8]) -> bool {
        let Some(s) = self.host.open(uri, FO_READ) else { return false };
        if s.size().is_some_and(|n| n != data.len() as u64) {
            return false;
        }
        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut off = 0;
        loop {
            match s.read_at(off as u64, &mut buf) {
                Some(0) => return off == data.len(),
                Some(n) if off + n <= data.len() && buf[..n] == data[off..off + n] => off += n,
                _ => return false,
            }
        }
    }

    // the journal file of a document
    fn journal_file(&self, uri: &str) -> Option<String> {
        Some(format!("{}/{:016x}.jw", self.journal.as_ref()?, fnv(uri.as_bytes())))
    }

    // a pending journal copy of a document goes (the app writes it some other way now)
    fn drop_journal(&self, uri: &str) {
        if let Some(j) = self.journal_file(uri) {
            if crate::file_exists(&j) == 1 {
                crate::delete(&j);
            }
        }
    }

    // the document replaced by data, as safely as SAF allows (see the file header)
    fn put(&self, uri: &str, data: &[u8]) -> bool {
        let journal = self.journal_file(uri);
        if let (Some(j), Some(dir)) = (&journal, &self.journal) {
            let mut rec = Vec::new();
            if rec.try_reserve(data.len() + uri.len() + 24).is_err() {
                return false;
            }
            rec.extend_from_slice(JOURNAL);
            rec.extend_from_slice(&(uri.len() as u32).to_le_bytes());
            rec.extend_from_slice(uri.as_bytes());
            rec.extend_from_slice(&(data.len() as u64).to_le_bytes());
            rec.extend_from_slice(data);
            rec.extend_from_slice(&fnv(data).to_le_bytes());
            // no copy, no write: the document stays as it is
            if !crate::mkdir(dir) || !crate::write_file(j, &rec) {
                return false;
            }
        }
        let cut = INTERRUPT.swap(u64::MAX, Ordering::Relaxed);
        if cut != u64::MAX {
            let n = usize::try_from(cut).unwrap_or(usize::MAX).min(data.len());
            self.write_all(uri, &data[..n]);
            return false;
        }
        for _ in 0..2 {
            if self.write_all(uri, data) && self.verify(uri, data) {
                if let Some(j) = &journal {
                    crate::delete(j);
                }
                return true;
            }
        }
        false // the copy stays: recover() writes it at the next start
    }

    /// finish the writes a crash (or a failure) left in the journal: each copy is written into
    /// its document again if that still exists, then removed; copies of documents that are gone
    /// or not granted any more, and damaged ones, are removed -> how many documents were
    /// completed
    pub fn recover(&self) -> u32 {
        let Some(dir) = self.journal.clone() else { return 0 };
        let mut files = Vec::new();
        crate::dir_list(&dir, &mut |n: &str, d: bool, _| {
            if !d && n.ends_with(".jw") {
                files.push(n.to_owned());
            }
        });
        let mut done = 0;
        for f in files {
            let path = format!("{dir}/{f}");
            let Some(rec) = crate::read_file(&path) else { continue };
            let parsed = (|| {
                let rest = rec.strip_prefix(JOURNAL)?;
                let n = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
                let uri = core::str::from_utf8(rest.get(4..4 + n)?).ok()?;
                let rest = &rest[4 + n..];
                let len = usize::try_from(u64::from_le_bytes(rest.get(..8)?.try_into().ok()?)).ok()?;
                let data = rest.get(8..8usize.checked_add(len)?)?;
                let sum = u64::from_le_bytes(rest.get(8 + len..16 + len)?.try_into().ok()?);
                (sum == fnv(data) && rest.len() == 16 + len).then_some((uri, data))
            })();
            let finished = match parsed {
                Some((uri, data)) => match self.host.query(uri) {
                    Some(d) if !d.dir => {
                        let ok = self.write_all(uri, data) && self.verify(uri, data);
                        done += ok as u32;
                        ok
                    }
                    _ => true, // gone: nothing to finish
                },
                None => true,
            };
            if finished {
                crate::delete(&path);
            }
        }
        done
    }
}

impl<H: DocHost> Provider for ContentFs<H> {
    fn kind(&self, path: &str) -> u32 {
        match self.lookup(path) {
            Some(d) if d.dir => 2,
            Some(_) => 1,
            None => 0,
        }
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let d = self.lookup(path)?;
        if d.dir {
            return None;
        }
        self.read_all(&d.uri)
    }

    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        let Some(d) = self.lookup(path).filter(|d| d.dir) else { return false };
        let Some(children) = self.host.children(&d.uri) else { return false };
        let at = ContentFs::<H>::key_of(path);
        for c in children {
            if let Some(k) = &at {
                self.remember(format!("{k}/{}", c.name), &c);
            }
            f(&c.name, c.dir, c.size.unwrap_or(0));
        }
        true
    }

    fn write(&self, path: &str, data: &[u8]) -> bool {
        let d = match self.lookup(path) {
            Some(d) if d.dir => return false,
            Some(d) => d,
            None => {
                // a new file below a chosen folder: its folders first
                let Some((_, up, _)) = self.split(path) else { return false };
                if !up.is_empty() && !self.mkdir(&path[..path.rfind('/').unwrap_or(0)]) {
                    return false;
                }
                match self.create_at(path, false) {
                    Some(d) => d,
                    None => return false,
                }
            }
        };
        self.put(&d.uri, data)
    }

    fn mkdir(&self, path: &str) -> bool {
        match parse(path) {
            Some(Name::Doc(uri)) => self.host.query(&uri).is_some_and(|d| d.dir),
            Some(Name::Tree(base, root, names)) => {
                let Some(mut cur) = self.walk(base, &root, &[]) else { return false };
                for k in 1..=names.len() {
                    if !cur.dir {
                        return false;
                    }
                    cur = match self.walk(base, &root, &names[..k]) {
                        Some(c) => c,
                        None => match self.host.create(&cur.uri, names[k - 1], true) {
                            Some(c) if c.name == names[k - 1] => {
                                self.remember(key(base, &names[..k]), &c);
                                c
                            }
                            _ => return false,
                        },
                    };
                }
                cur.dir
            }
            None => false,
        }
    }

    fn delete(&self, path: &str) -> bool {
        // never the folder the user chose: it is the user's, not the app's
        if matches!(parse(path), Some(Name::Tree(_, _, ref n)) if n.is_empty()) {
            return false;
        }
        let Some(d) = self.lookup(path) else { return false };
        if d.dir && self.host.children(&d.uri).is_none_or(|k| !k.is_empty()) {
            return false; // only empty folders
        }
        let ok = self.host.delete(&d.uri);
        if let Some(k) = ContentFs::<H>::key_of(path) {
            self.forget(&k);
        }
        if ok {
            self.drop_journal(&d.uri);
        }
        ok
    }

    fn rename(&self, old: &str, new: &str) -> bool {
        let (Some((ra, ua, _)), Some((rb, ub, nb))) = (self.split(old), self.split(new)) else { return false };
        if ra != rb {
            return false;
        }
        let Some(src) = self.lookup(old) else { return false };
        if old == new {
            return true;
        }
        let (ko, kn) = (ContentFs::<H>::key_of(old).unwrap_or_default(), ContentFs::<H>::key_of(new).unwrap_or_default());
        // SAF renames to a free name when the new one is taken: the target goes first
        if let Some(t) = self.lookup(new) {
            if t.dir || !self.host.delete(&t.uri) {
                return false;
            }
            self.drop_journal(&t.uri);
        }
        self.forget(&kn);
        self.drop_journal(&src.uri);
        if ua == ub {
            let r = self.host.rename(&src.uri, nb);
            self.forget(&ko);
            return match r {
                Some(d) if d.name == nb => {
                    self.remember(kn, &d);
                    true
                }
                _ => false,
            };
        }
        // to another folder: a copy, then the old one goes (files only)
        if src.dir {
            return false;
        }
        let Some(data) = self.read_all(&src.uri) else { return false };
        let ok = self.write(new, &data) && self.host.delete(&src.uri);
        self.forget(&ko);
        ok
    }

    fn stat(&self, path: &str) -> Option<Stat> {
        let d = self.lookup(path)?;
        Some(Stat { kind: if d.dir { 2 } else { 1 }, size: d.size.unwrap_or(0), mtime_ns: d.mtime_ns, atime_ns: 0 })
    }

    fn open(&self, path: &str, mode: u32) -> Opened {
        if !matches!(mode, FO_READ | FO_WRITE | FO_RW) {
            return Opened::Refused;
        }
        let d = match self.lookup(path) {
            Some(d) if d.dir => return Opened::Refused,
            Some(d) => d,
            None if mode == FO_READ => return Opened::Refused,
            None => match self.create_at(path, false) {
                Some(d) => d,
                None => return Opened::Refused,
            },
        };
        if mode != FO_READ {
            self.drop_journal(&d.uri);
        }
        match self.host.open(&d.uri, mode) {
            Some(s) => Opened::File(s),
            None => Opened::Refused,
        }
    }

    fn display_name(&self, path: &str) -> Option<String> {
        self.lookup(path).map(|d| d.name)
    }

    fn atomic_rename(&self) -> bool {
        false
    }
}
