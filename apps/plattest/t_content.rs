// t_content -- plattest --test content: Android's content:// names (lib/plat/content.rs) on the
// desktop, against a document provider in memory that behaves like Android's storage
// provider: path-like document IDs (a rename changes the URI), "name (1)" when a name is
// taken, trees with their own URIs, recursive deletes.  Through the ordinary file functions:
// names below a chosen folder, single documents, AtomicFile written in place, File streams,
// rename / move, mkdir / delete, stat and display names; the name cache (no listing for a
// known name, checked against the document, emptied when the host reports a change) and the
// journal of writes (a copy first, read back and compared, an interrupted write finished by
// recover() in a new provider, as after a restart); plus vfs::display_name for other names and
// the file dialog's desktop answers (no picker that answers later, FD_FOLDER).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use plat::content::{self, ContentFs, DocHost, DocInfo, DOC_CREATE, DOC_WRITE};
use plat::vfs::{self, Stream};
use plat::{AtomicFile, FO_READ, FO_RW, FO_WRITE, File};
use rt::sync::SpinLock;

use crate::T;

const AUTH: &str = "fake.docs";

type Data = Arc<SpinLock<Vec<u8>>>;

// the host's change counter (DocHost::generation); a provider that keeps only half of each write
static GEN: AtomicU64 = AtomicU64::new(0);
static LOSSY: AtomicBool = AtomicBool::new(false);

// a storage provider in memory: documents by path ("Docs/a.txt"), IDs "root:" + path
struct Fake {
    files: SpinLock<BTreeMap<String, Data>>,
    dirs: SpinLock<Vec<String>>,
}

fn enc(id: &str) -> String {
    id.replace('%', "%25").replace(':', "%3A").replace('/', "%2F")
}

fn dec(s: &str) -> String {
    s.replace("%2F", "/").replace("%3A", ":").replace("%25", "%")
}

impl Fake {
    fn new() -> Fake {
        let f = Fake { files: SpinLock::new(BTreeMap::new()), dirs: SpinLock::new(Vec::new()) };
        f.dirs.lock().extend(["Docs", "Docs/jfgui-test", "Other"].map(String::from));
        f.files.lock().insert(String::from("Docs/jfgui-test/hello.txt"), Arc::new(SpinLock::new(b"Hallo vom Host\n".to_vec())));
        f
    }

    // (tree path, document path) of a URI; the document must lie in the tree
    fn parse(&self, uri: &str) -> Option<(Option<String>, String)> {
        let rest = uri.strip_prefix("content://")?.strip_prefix(AUTH)?.strip_prefix('/')?;
        let segs: Vec<&str> = rest.split('/').collect();
        let path = |s: &str| dec(s).strip_prefix("root:").map(String::from);
        match segs.as_slice() {
            ["document", d] => Some((None, path(d)?)),
            ["tree", t] => Some((Some(path(t)?), path(t)?)),
            ["tree", t, "document", d] => {
                let (t, d) = (path(t)?, path(d)?);
                (d == t || d.starts_with(&format!("{t}/"))).then_some((Some(t), d))
            }
            _ => None,
        }
    }

    fn uri(tree: &Option<String>, path: &str) -> String {
        match tree {
            Some(t) => format!("content://{AUTH}/tree/{}/document/{}", enc(&format!("root:{t}")), enc(&format!("root:{path}"))),
            None => format!("content://{AUTH}/document/{}", enc(&format!("root:{path}"))),
        }
    }

    fn info(&self, tree: &Option<String>, path: &str) -> Option<DocInfo> {
        let name = String::from(path.rsplit('/').next().unwrap_or(path));
        if self.dirs.lock().iter().any(|d| d == path) {
            return Some(DocInfo { uri: Fake::uri(tree, path), name, dir: true, size: None, mtime_ns: 0, flags: DOC_CREATE });
        }
        let size = self.files.lock().get(path)?.lock().len() as u64;
        Some(DocInfo { uri: Fake::uri(tree, path), name, dir: false, size: Some(size), mtime_ns: 1_000_000, flags: DOC_WRITE })
    }

    fn exists(&self, path: &str) -> bool {
        self.files.lock().contains_key(path) || self.dirs.lock().iter().any(|d| d == path)
    }

    // "a.txt" -> "a (1).txt" while taken, as Android's storage provider does
    fn free(&self, dir: &str, name: &str) -> String {
        let mut p = format!("{dir}/{name}");
        let (stem, ext) = match name.rfind('.') {
            Some(k) if k > 0 => (&name[..k], &name[k..]),
            _ => (name, ""),
        };
        let mut n = 1;
        while self.exists(&p) {
            p = format!("{dir}/{stem} ({n}){ext}");
            n += 1;
        }
        p
    }
}

struct MemStream {
    d: Data,
    writable: bool,
}

impl Stream for MemStream {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        let g = self.d.lock();
        let off = (off as usize).min(g.len());
        let n = (g.len() - off).min(buf.len());
        buf[..n].copy_from_slice(&g[off..off + n]);
        Some(n)
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let mut g = self.d.lock();
        let n = buf.len();
        // (the lossy provider) says it took everything, keeps half
        let buf = if LOSSY.load(Ordering::Relaxed) { &buf[..n / 2] } else { buf };
        let end = off as usize + buf.len();
        if g.len() < end {
            g.resize(end, 0);
        }
        g[off as usize..end].copy_from_slice(buf);
        Some(n)
    }
    fn size(&self) -> Option<u64> {
        Some(self.d.lock().len() as u64)
    }
}

// the provider as a host: several ContentFs on the same documents (a restart)
struct Host(Arc<Fake>);

impl core::ops::Deref for Host {
    type Target = Fake;
    fn deref(&self) -> &Fake {
        &self.0
    }
}

impl DocHost for Host {
    fn generation(&self) -> u64 {
        GEN.load(Ordering::Relaxed)
    }
    fn query(&self, uri: &str) -> Option<DocInfo> {
        let (t, p) = self.parse(uri)?;
        self.info(&t, &p)
    }
    fn children(&self, uri: &str) -> Option<Vec<DocInfo>> {
        let (t, p) = self.parse(uri)?;
        t.as_ref()?; // only below a tree
        let pre = format!("{p}/");
        let mut kids: Vec<String> = self.dirs.lock().iter().filter(|d| d.strip_prefix(&pre).is_some_and(|r| !r.contains('/'))).cloned().collect();
        kids.extend(self.files.lock().keys().filter(|f| f.strip_prefix(&pre).is_some_and(|r| !r.contains('/'))).cloned());
        Some(kids.iter().filter_map(|k| self.info(&t, k)).collect())
    }
    fn create(&self, parent: &str, name: &str, dir: bool) -> Option<DocInfo> {
        let (t, p) = self.parse(parent)?;
        if !self.dirs.lock().contains(&p) {
            return None;
        }
        let path = self.free(&p, name);
        if dir {
            self.dirs.lock().push(path.clone());
        } else {
            self.files.lock().insert(path.clone(), Arc::new(SpinLock::new(Vec::new())));
        }
        self.info(&t, &path)
    }
    fn delete(&self, uri: &str) -> bool {
        let Some((_, p)) = self.parse(uri) else { return false };
        let pre = format!("{p}/");
        let n = self.files.lock().len() + self.dirs.lock().len();
        self.files.lock().retain(|f, _| *f != p && !f.starts_with(&pre)); // recursive, like SAF
        self.dirs.lock().retain(|d| *d != p && !d.starts_with(&pre));
        n != self.files.lock().len() + self.dirs.lock().len()
    }
    fn rename(&self, uri: &str, name: &str) -> Option<DocInfo> {
        let (t, p) = self.parse(uri)?;
        let dir = p.rsplit_once('/').map_or("", |(d, _)| d);
        let to = self.free(dir, name);
        let file = self.files.lock().remove(&p);
        if let Some(d) = file {
            self.files.lock().insert(to.clone(), d);
        } else {
            let mut g = self.dirs.lock();
            let pre = format!("{p}/");
            for d in g.iter_mut() {
                if *d == p {
                    *d = to.clone();
                } else if let Some(r) = d.strip_prefix(&pre) {
                    *d = format!("{to}/{r}");
                }
            }
        }
        self.info(&t, &to)
    }
    fn open(&self, uri: &str, mode: u32) -> Option<Box<dyn Stream>> {
        let (_, p) = self.parse(uri)?;
        let d = self.files.lock().get(&p)?.clone();
        if mode == FO_WRITE {
            d.lock().clear();
        }
        Some(Box::new(MemStream { d, writable: mode != FO_READ }))
    }
}

fn text(path: &str) -> Option<String> {
    plat::read_file(path).and_then(|b| String::from_utf8(b).ok())
}

fn names(path: &str) -> Vec<String> {
    let mut v = Vec::new();
    plat::dir_list(path, &mut |n, d, s| v.push(if d { format!("{n}/") } else { format!("{n}={s}") }));
    v.sort();
    v
}

pub fn t_content(t: &mut T) {
    let fake = Arc::new(Fake::new());
    t.check(vfs::mount("content", Arc::new(ContentFs::new(Host(fake.clone())))), "content: a provider mounted under content://");
    let tree = format!("content://{AUTH}/tree/{}", enc("root:Docs/jfgui-test"));
    let at = |n: &str| format!("{tree}/{n}");
    tree_names(t, &tree, &at);
    tree_writes(t, &at);
    tree_streams(t, &at);
    tree_renames(t, &tree, &at);
    tree_folders(t, &tree, &at);
    single(t, &tree, &at);
    cache(t, &fake, &at);
    journal(t, &fake, &tree, &at);
    names_and_dialogs(t);
    vfs::unmount("content");
}

fn tree_names(t: &mut T, tree: &str, at: &dyn Fn(&str) -> String) {
    t.check(plat::file_exists(tree) == 2, "content: the chosen folder is a folder");
    t.check(plat::display_name(tree) == "jfgui-test", "content: display name of the folder (not its encoded ID)");
    t.check(text(&at("hello.txt")).as_deref() == Some("Hallo vom Host\n"), "content: a file below it by its name");
    t.check(plat::display_name(&at("hello.txt")) == "hello.txt", "content: display name of a file");
    t.check(vfs::stat(&at("hello.txt")).is_some_and(|s| s.kind == 1 && s.size == 15 && s.mtime_ns == 1_000_000), "content: stat (kind, size, time)");
    t.check(plat::file_exists(&at("fehlt.txt")) == 0 && plat::read_file(&at("fehlt.txt")).is_none(), "content: a missing name");
    t.check(plat::file_exists(&format!("content://{AUTH}/tree/{}/x", enc("root:Nope"))) == 0, "content: a tree the host does not know");
    t.check(plat::file_exists("content://") == 0 && plat::read_file("content://").is_none(), "content: the bare scheme is nothing");
}

fn tree_writes(t: &mut T, at: &dyn Fn(&str) -> String) {
    t.check(plat::write_file(&at("neu/datei.txt"), b"neuer Inhalt"), "content: write a new file (its folder created)");
    t.check(plat::file_exists(&at("neu")) == 2, "content: the new folder");
    t.check(text(&at("neu/datei.txt")).as_deref() == Some("neuer Inhalt"), "content: read it back");
    t.check(plat::write_file(&at("neu/datei.txt"), b"kurz"), "content: replace it");
    t.check(text(&at("neu/datei.txt")).as_deref() == Some("kurz"), "content: the shorter contents (truncated)");
    t.check(names(&at("neu")) == ["datei.txt=4"], "content: list with sizes");
    // AtomicFile: SAF cannot rename over a document -> the bytes are written in place at commit
    if let Some(a) = AtomicFile::create(&at("neu/datei.txt")) {
        a.write_at(0, b"halb");
    }
    t.check(text(&at("neu/datei.txt")).as_deref() == Some("kurz") && names(&at("neu")) == ["datei.txt=4"],
        "content: AtomicFile dropped: unchanged, no sibling");
    let ok = AtomicFile::create(&at("neu/datei.txt")).is_some_and(|a| a.write_at(0, b"ganz neu") == Some(8) && a.commit());
    t.check(ok && text(&at("neu/datei.txt")).as_deref() == Some("ganz neu"), "content: AtomicFile commit");
    let ok = AtomicFile::create(&at("neu/frisch.txt")).is_some_and(|a| a.write_at(0, b"x") == Some(1) && a.commit());
    t.check(ok && text(&at("neu/frisch.txt")).as_deref() == Some("x"), "content: AtomicFile creates a new document");
    t.check(AtomicFile::create(&at("neu")).is_none(), "content: AtomicFile on a folder is refused");
}

fn tree_streams(t: &mut T, at: &dyn Fn(&str) -> String) {
    let f = File::open(&at("neu/datei.txt"), FO_RW);
    let ok = f.as_ref().is_some_and(|f| f.write_at(8, b"!") == Some(1) && f.size() == Some(9) && f.sync());
    drop(f);
    t.check(ok && text(&at("neu/datei.txt")).as_deref() == Some("ganz neu!"), "content: File FO_RW writes at the end");
    let f = File::open(&at("neu/datei.txt"), FO_READ);
    let mut b = [0u8; 3];
    t.check(f.as_ref().and_then(|f| f.read_at(5, &mut b)) == Some(3) && &b == b"neu", "content: File FO_READ at an offset");
    t.check(f.is_some_and(|f| f.write_at(0, b"x").is_none()), "content: a read-only stream refuses writes");
    let f = File::open(&at("neu/strom.bin"), FO_WRITE);
    t.check(f.is_some_and(|f| f.write_at(2, b"ab") == Some(2) && f.size() == Some(4)), "content: File FO_WRITE creates the document");
    t.check(plat::read_file(&at("neu/strom.bin")).as_deref() == Some(&b"\0\0ab"[..]), "content: its bytes");
    t.check(File::open(&at("neu/fehlt.bin"), FO_READ).is_none() && File::open(&at("neu"), FO_RW).is_none(), "content: no stream on a missing name or a folder");
}

fn tree_renames(t: &mut T, tree: &str, at: &dyn Fn(&str) -> String) {
    t.check(plat::rename(&at("neu/datei.txt"), &at("neu/umbenannt.txt")), "content: rename inside a folder");
    t.check(plat::file_exists(&at("neu/datei.txt")) == 0 && text(&at("neu/umbenannt.txt")).as_deref() == Some("ganz neu!"), "content: the new name, the old one gone");
    t.check(plat::rename(&at("neu/umbenannt.txt"), &at("neu/frisch.txt")), "content: rename over an existing file");
    t.check(names(&at("neu")) == ["frisch.txt=9", "strom.bin=4"], "content: replaced, no \"frisch (1).txt\"");
    t.check(plat::rename(&at("neu/frisch.txt"), &at("verschoben.txt")), "content: move to another folder (copy, delete)");
    t.check(text(&at("verschoben.txt")).as_deref() == Some("ganz neu!") && plat::file_exists(&at("neu/frisch.txt")) == 0, "content: moved");
    t.check(!plat::rename(&at("verschoben.txt"), &format!("content://{AUTH}/tree/{}/x.txt", enc("root:Other"))), "content: no rename across trees");
    t.check(!plat::rename(&at("neu"), &at("hello.txt/x")), "content: no folder move");
    let _ = tree;
}

fn tree_folders(t: &mut T, tree: &str, at: &dyn Fn(&str) -> String) {
    t.check(plat::mkdir(&at("a/b/c")) && plat::file_exists(&at("a/b/c")) == 2, "content: mkdir with parents");
    t.check(plat::mkdir(&at("a/b")), "content: mkdir of an existing folder");
    t.check(!plat::mkdir(&at("hello.txt/x")), "content: no folder below a file");
    t.check(!plat::delete(&at("a/b")), "content: a folder with entries stays (no recursive delete)");
    t.check(plat::delete(&at("a/b/c")) && plat::delete(&at("a/b")) && plat::delete(&at("a")), "content: empty folders are deleted");
    t.check(plat::delete(&at("verschoben.txt")) && plat::delete(&at("neu/strom.bin")) && plat::delete(&at("neu")), "content: files and their folder deleted");
    t.check(!plat::delete(tree) && plat::file_exists(tree) == 2, "content: the chosen folder itself is never deleted");
    t.check(names(tree) == ["hello.txt=15"], "content: the folder as it was");
}

fn single(t: &mut T, tree: &str, at: &dyn Fn(&str) -> String) {
    // a document by its own URI (what ACTION_OPEN_DOCUMENT / ACTION_CREATE_DOCUMENT hand out)
    let doc = format!("content://{AUTH}/document/{}", enc("root:Docs/jfgui-test/hello.txt"));
    t.check(plat::file_exists(&doc) == 1 && plat::display_name(&doc) == "hello.txt", "content: a single document");
    t.check(text(&doc).as_deref() == Some("Hallo vom Host\n"), "content: read it");
    let ok = AtomicFile::create(&doc).is_some_and(|a| a.write_at(0, b"ersetzt") == Some(7) && a.commit());
    t.check(ok && text(&doc).as_deref() == Some("ersetzt"), "content: AtomicFile on a document without siblings");
    t.check(plat::write_file(&doc, b"Hallo vom Host\n"), "content: write_file on it");
    t.check(!plat::mkdir(&format!("{doc}/x")) && !plat::delete(&format!("{doc}/x")), "content: nothing below a single document");
    t.check(!plat::rename(&doc, &format!("content://{AUTH}/document/{}", enc("root:Docs/y.txt"))), "content: no rename of a single document");
    // Android's own form of a document inside a tree, as ContentFs::document reports it
    let canon = ContentFs::new(Host(Arc::new(Fake::new()))).document(at("hello.txt").strip_prefix("content://").unwrap_or("")).map(|d| d.uri).unwrap_or_default();
    t.check(canon.starts_with(&format!("content://{AUTH}/tree/")) && canon.contains("/document/"), "content: document() gives the tree's own URI form");
    t.check(text(&canon).as_deref() == Some("Hallo vom Host\n"), "content: that form reads the same file");
    t.check(plat::policy::user_doc_allowed(tree), "content: without a manifest the user's documents are allowed");
}

// the name cache: a known name needs no listing; a document gone behind the provider's back is
// noticed (its query fails); a change the host reports empties the cache
fn cache(t: &mut T, fake: &Arc<Fake>, at: &dyn Fn(&str) -> String) {
    t.check(plat::write_file(&at("c/d/e.txt"), b"tief") && plat::write_file(&at("c/d/f.txt"), b"daneben"), "cache: files two folders down");
    let n = content::listings();
    t.check(text(&at("c/d/e.txt")).as_deref() == Some("tief") && text(&at("c/d/f.txt")).as_deref() == Some("daneben")
        && plat::file_exists(&at("c/d")) == 2 && vfs::stat(&at("c/d/e.txt")).is_some_and(|s| s.size == 4), "cache: read, kind, stat");
    t.check(content::listings() == n, "cache: known names need no folder listing");
    // a sibling never seen before: listed from the cached folder c/d, not from the root
    fake.files.lock().insert(String::from("Docs/jfgui-test/c/d/g.txt"), Arc::new(SpinLock::new(b"neu".to_vec())));
    t.check(text(&at("c/d/g.txt")).as_deref() == Some("neu") && content::listings() == n + 1, "cache: a new name: one listing (of its folder)");
    // gone behind the provider's back (no change reported): the document's own answer counts
    fake.files.lock().remove("Docs/jfgui-test/c/d/e.txt");
    t.check(plat::file_exists(&at("c/d/e.txt")) == 0 && plat::read_file(&at("c/d/e.txt")).is_none(), "cache: a document gone meanwhile is missing");
    // a cached folder renamed behind its back: walked again from the root
    let moved: Vec<(String, Data)> = fake.files.lock().iter().filter(|(k, _)| k.starts_with("Docs/jfgui-test/c/")).map(|(k, v)| (k.replacen("/c/", "/c2/", 1), v.clone())).collect();
    fake.files.lock().retain(|k, _| !k.starts_with("Docs/jfgui-test/c/"));
    fake.files.lock().extend(moved);
    for d in fake.dirs.lock().iter_mut() {
        if d == "Docs/jfgui-test/c" || d.starts_with("Docs/jfgui-test/c/") {
            *d = d.replacen("jfgui-test/c", "jfgui-test/c2", 1);
        }
    }
    t.check(text(&at("c/d/f.txt")).is_none() && text(&at("c2/d/f.txt")).as_deref() == Some("daneben"), "cache: a folder renamed meanwhile");
    // a reported change: the next lookup walks again
    let n = content::listings();
    t.check(text(&at("c2/d/f.txt")).is_some() && content::listings() == n, "cache: cached again");
    GEN.fetch_add(1, Ordering::Relaxed);
    t.check(text(&at("c2/d/f.txt")).is_some() && content::listings() == n + 3, "cache: emptied by a reported change (walked again: three listings)");
    t.check(plat::delete(&at("c2/d/f.txt")) && plat::delete(&at("c2/d/g.txt")) && plat::delete(&at("c2/d")) && plat::delete(&at("c2")), "cache: cleaned up");
    t.check(plat::file_exists(&at("c2/d")) == 0 && plat::file_exists(&at("c2")) == 0, "cache: deleted names are forgotten");
}

fn journal_files() -> Vec<String> {
    let mut v = Vec::new();
    plat::dir_list("mem://journal", &mut |n, _, _| v.push(String::from(n)));
    v
}

// writes through a journal: nothing left after a good write; an interrupted or unverified write
// keeps its copy, which recover() (a new provider: a restart) writes into the document
fn journal(t: &mut T, fake: &Arc<Fake>, tree: &str, at: &dyn Fn(&str) -> String) {
    let fresh = || ContentFs::new(Host(fake.clone())).with_journal("mem://journal");
    let uri_of = |n: &str| ContentFs::new(Host(fake.clone())).document(at(n).strip_prefix("content://").unwrap_or("")).map(|d| d.uri);
    vfs::mount("content", Arc::new(fresh()));
    t.check(plat::write_file(&at("j.txt"), b"erster Inhalt") && text(&at("j.txt")).as_deref() == Some("erster Inhalt"), "journal: a write");
    t.check(journal_files().is_empty(), "journal: no copy left after a good write");
    let uri = uri_of("j.txt");
    // a crash in the middle: a part in the document, the whole in the journal
    content::interrupt_next_write(4);
    t.check(!plat::write_file(&at("j.txt"), b"zweiter, ganzer Inhalt"), "journal: the interrupted write fails");
    t.check(text(&at("j.txt")).as_deref() == Some("zwei") && journal_files().len() == 1, "journal: a part written, the copy kept");
    let restarted = fresh();
    t.check(restarted.recover() == 1 && journal_files().is_empty(), "journal: recover() finishes it (a new provider, as after a restart)");
    vfs::mount("content", Arc::new(restarted));
    t.check(text(&at("j.txt")).as_deref() == Some("zweiter, ganzer Inhalt"), "journal: the whole contents after recovery");
    t.check(uri.is_some() && uri_of("j.txt") == uri, "journal: the document kept its URI");
    // a provider that loses bytes: read back, compared, refused; the copy stays for later
    LOSSY.store(true, Ordering::Relaxed);
    let ok = plat::write_file(&at("j.txt"), b"verlorene Bytes");
    LOSSY.store(false, Ordering::Relaxed);
    t.check(!ok && journal_files().len() == 1, "journal: a write the provider did not keep is refused (verified)");
    t.check(fresh().recover() == 1 && text(&at("j.txt")).as_deref() == Some("verlorene Bytes"), "journal: finished once the provider works");
    // a stream for writing takes the document over: its pending copy goes
    content::interrupt_next_write(1);
    plat::write_file(&at("j.txt"), b"nie ganz");
    t.check(journal_files().len() == 1, "journal: pending again");
    drop(File::open(&at("j.txt"), FO_RW));
    t.check(journal_files().is_empty() && fresh().recover() == 0, "journal: a stream opened for writing drops the copy");
    // copies of a document that is gone, damaged copies: dropped
    content::interrupt_next_write(0);
    plat::write_file(&at("k.txt"), b"weg");
    fake.files.lock().remove("Docs/jfgui-test/k.txt");
    plat::write_file("mem://journal/0123456789abcdef.jw", b"JFCJ kaputt");
    t.check(journal_files().len() == 2 && fresh().recover() == 0 && journal_files().is_empty(), "journal: copies of gone documents and damaged ones are dropped");
    // AtomicFile goes through the same write
    let ok = AtomicFile::create(&at("j.txt")).is_some_and(|a| a.write_at(0, b"atomar") == Some(6) && a.commit());
    t.check(ok && text(&at("j.txt")).as_deref() == Some("atomar") && journal_files().is_empty(), "journal: AtomicFile's commit");
    t.check(plat::delete(&at("j.txt")) && names(tree) == ["hello.txt=15"], "journal: cleaned up");
}

fn names_and_dialogs(t: &mut T) {
    t.check(plat::display_name("mem://a/b.txt") == "b.txt" && plat::display_name("config://") == "config://", "display_name: vfs names");
    t.check(plat::display_name("/home/u/Bild.png") == "Bild.png" && plat::display_name("/home/u/Ordner/") == "Ordner", "display_name: OS paths");
    if cfg!(windows) {
        t.check(plat::display_name("C:\\Users\\u\\a.txt") == "a.txt" && plat::display_name("C:/") == "C:/", "display_name: Windows paths");
        // GetOpenFileNameW picks files only: the library's own dialog chooses folders
        let mut p = String::new();
        t.check(plat::file_dialog(plat::FD_FOLDER, &mut p, &[]) == (-1, 0), "file_dialog: FD_FOLDER -> the own dialog (Windows)");
    } else {
        t.check(plat::display_name("/a/b\\c.txt") == "b\\c.txt", "display_name: '\\' is a character of Linux names");
    }
    t.check(plat::file_dialog_poll().is_none(), "file_dialog_poll: desktop dialogs answer at once");
}
