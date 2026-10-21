// content -- the documents of other apps as files: Android's Storage Access Framework
// ("content://" names) behind the vfs; the name logic runs on every platform, so the desktop
// tests it with a host of its own
//
//   ContentFs::new(host) -> a vfs Provider for the names after "content://".  On Android the
//        vfs mounts it on first use with the ContentResolver as its host (android_host.rs);
//        elsewhere only a test mounts it (apps/plattest/t_content.rs: a host in memory).
//   DocHost (trait)   what the provider asks the system: query (one document), children (of a
//        folder document), create, delete, rename, open (a Stream: Android a file descriptor)
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
// SAF has no rename over an existing document (providers pick "name (1)"), so the provider
// says atomic_rename() == false: AtomicFile (write_file in pieces, aio) collects the bytes and
// writes the document in place at commit ("wt").  rename replaces by deleting the target first.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

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
}

/// "content://" + path
pub fn uri_of(path: &str) -> String {
    format!("content://{path}")
}

// what a name inside the provider means
enum Name<'a> {
    /// a document by its URI
    Doc(String),
    /// a tree's root URI and the display names below it
    Tree(String, Vec<&'a str>),
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
        return Some(Name::Tree(uri_of(&segs[..3].join("/")), segs[3..].to_vec()));
    }
    Some(Name::Doc(uri_of(path)))
}

/// the documents of other apps behind "content://" (see the file header)
pub struct ContentFs<H: DocHost> {
    host: H,
}

impl<H: DocHost> ContentFs<H> {
    pub fn new(host: H) -> ContentFs<H> {
        ContentFs { host }
    }

    /// the host the provider asks
    pub fn host(&self) -> &H {
        &self.host
    }

    /// the document a name (after "content://") means -- its own URI, display name, size
    pub fn document(&self, path: &str) -> Option<DocInfo> {
        self.lookup(path)
    }

    // the document behind a tree root and names (each one a child of the one before)
    fn walk(&self, root: &str, names: &[&str]) -> Option<DocInfo> {
        let mut cur = self.host.query(root)?;
        for n in names {
            if !cur.dir {
                return None;
            }
            cur = self.host.children(&cur.uri)?.into_iter().find(|c| c.name == *n)?;
        }
        Some(cur)
    }

    // the document a name means
    fn lookup(&self, path: &str) -> Option<DocInfo> {
        match parse(path)? {
            Name::Doc(uri) => self.host.query(&uri),
            Name::Tree(root, names) => self.walk(&root, &names),
        }
    }

    // a new document at path (a tree name; its folder must exist) -> its description
    fn create_at(&self, path: &str, dir: bool) -> Option<DocInfo> {
        let Name::Tree(root, names) = parse(path)? else { return None };
        let (name, up) = names.split_last()?;
        let parent = self.walk(&root, up)?;
        if !parent.dir {
            return None;
        }
        self.host.create(&parent.uri, name, dir)
    }

    // the folder a tree name is in -> (root, parent names, own name)
    fn split<'a>(&self, path: &'a str) -> Option<(String, Vec<&'a str>, &'a str)> {
        let Name::Tree(root, mut names) = parse(path)? else { return None };
        let name = names.pop()?;
        Some((root, names, name))
    }

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
        for c in children {
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
        self.write_all(&d.uri, data)
    }

    fn mkdir(&self, path: &str) -> bool {
        match parse(path) {
            Some(Name::Doc(uri)) => self.host.query(&uri).is_some_and(|d| d.dir),
            Some(Name::Tree(root, names)) => {
                let Some(mut cur) = self.host.query(&root) else { return false };
                for n in names {
                    if !cur.dir {
                        return false;
                    }
                    let Some(kids) = self.host.children(&cur.uri) else { return false };
                    cur = match kids.into_iter().find(|c| c.name == n) {
                        Some(c) => c,
                        None => match self.host.create(&cur.uri, n, true) {
                            Some(c) => c,
                            None => return false,
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
        if matches!(parse(path), Some(Name::Tree(_, ref n)) if n.is_empty()) {
            return false;
        }
        let Some(d) = self.lookup(path) else { return false };
        if d.dir && self.host.children(&d.uri).is_none_or(|k| !k.is_empty()) {
            return false; // only empty folders
        }
        self.host.delete(&d.uri)
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
        // SAF renames to a free name when the new one is taken: the target goes first
        if let Some(t) = self.lookup(new) {
            if t.dir || !self.host.delete(&t.uri) {
                return false;
            }
        }
        if ua == ub {
            return self.host.rename(&src.uri, nb).is_some_and(|d| d.name == nb);
        }
        // to another folder: a copy, then the old one goes (files only)
        if src.dir {
            return false;
        }
        let Some(data) = self.read_all(&src.uri) else { return false };
        self.write(new, &data) && self.host.delete(&src.uri)
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
