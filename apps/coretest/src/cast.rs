// cast -- lib/cas: chunk boundaries survive an insertion, deduplication, trees and signed
// snapshots, the three ways to unlock, locking, immutability, gc (and a backup between its
// writer's end and its snapshot), a flipped bit found by the check, an index rebuilt from a
// pack's trailer.  All of it twice: in a folder (cas::Folder) and in a store of this test
// (Mem, a cas::Store written outside the crate, the way an S3 or SFTP store would be), the
// tampering through the Store trait in both.  Then Mem's faults: a write that fails, short
// ranges, listings with names no repository writes, a listing that fails (gc stops instead of
// taking every chunk), and the repair that reads a pack's end only.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cas::store::{Kind, Store, StoreLock};
use cas::{CasError, CasResult, CreateOptions, Node, NodeKind, Repo, Snapshot, Tree, Unlock, KIND_DATA};
use compress::Codec;
use crypto::ed25519::Keypair;
use plat::sync::Mutex;

use crate::T;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn tmp(name: &str) -> String {
    plat::fs::join(&plat::temp_dir().unwrap_or_else(|| String::from(".")), &format!("coretest-{}-{}", name, plat::random_u64() % 1_000_000_000))
}

fn rm(dir: &str) {
    if let Some(list) = plat::fs::read_dir(dir) {
        for e in list {
            let p = plat::fs::join(dir, &e.name);
            if e.kind == plat::fs::FileKind::Dir {
                rm(&p);
            } else {
                plat::delete(&p);
            }
        }
    }
    plat::delete(dir);
}

/// store a byte buffer as a stream
fn put(w: &mut cas::Writer, data: &[u8]) -> Vec<cas::ChunkId> {
    let mut pos = 0;
    let mut read = |b: &mut [u8]| {
        let n = b.len().min(data.len() - pos).min(1_000_003);
        b[..n].copy_from_slice(&data[pos..pos + n]);
        pos += n;
        Ok(n)
    };
    w.put_stream(&mut read, &|| false).map(|(ids, _)| ids).unwrap_or_default()
}

fn content(r: &Repo, ids: &[cas::ChunkId]) -> Vec<u8> {
    let mut out = Vec::new();
    for id in ids {
        out.extend(r.read_chunk(id).unwrap_or_default());
    }
    out
}

// ------------------------------------------------------------------ Mem: a store of the test

/// objects in memory; clones share them (and the lock), like two clients of one bucket
#[derive(Clone)]
struct Mem(Arc<MemInner>);

struct MemInner {
    objs: Mutex<BTreeMap<(Kind, String), Vec<u8>>>,
    held: AtomicBool,
    /// the next put of this kind fails
    fail_put: Mutex<Option<Kind>>,
    /// listings of this kind fail
    fail_list: Mutex<Option<Kind>>,
    /// get_range hands out one byte less
    short: AtomicBool,
    /// listings carry names and sizes no repository writes
    junk: AtomicBool,
    /// bytes handed out by get and get_range
    read: AtomicU64,
}

struct MemLock(Arc<MemInner>);

impl Drop for MemLock {
    fn drop(&mut self) {
        self.0.held.store(false, Ordering::SeqCst);
    }
}

impl Mem {
    fn new() -> Mem {
        Mem(Arc::new(MemInner {
            objs: Mutex::new(BTreeMap::new()),
            held: AtomicBool::new(false),
            fail_put: Mutex::new(None),
            fail_list: Mutex::new(None),
            short: AtomicBool::new(false),
            junk: AtomicBool::new(false),
            read: AtomicU64::new(0),
        }))
    }
    fn read(&self) -> u64 {
        self.0.read.load(Ordering::SeqCst)
    }
}

fn mem_err(what: &str) -> CasError {
    CasError::Io(String::from(what))
}

impl Store for Mem {
    fn location(&self) -> String {
        String::from("mem:test")
    }
    fn init(&self) -> CasResult<()> {
        if self.0.objs.lock().is_empty() { Ok(()) } else { Err(CasError::NotEmpty) }
    }
    fn lock(&self) -> CasResult<StoreLock> {
        if self.0.held.swap(true, Ordering::SeqCst) {
            return Err(CasError::InUse);
        }
        Ok(Box::new(MemLock(self.0.clone())))
    }
    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>> {
        let o = self.0.objs.lock().get(&(kind, String::from(name))).cloned();
        self.0.read.fetch_add(o.as_ref().map_or(0, |b| b.len() as u64), Ordering::SeqCst);
        Ok(o)
    }
    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> {
        let objs = self.0.objs.lock();
        let b = objs.get(&(kind, String::from(name))).ok_or_else(|| CasError::NotFound(String::from(name)))?;
        let start = (off as usize).min(b.len());
        let mut end = start.saturating_add(len).min(b.len());
        if self.0.short.load(Ordering::SeqCst) && end > start {
            end -= 1;
        }
        self.0.read.fetch_add((end - start) as u64, Ordering::SeqCst);
        Ok(Vec::from(&b[start..end]))
    }
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()> {
        if self.0.fail_put.lock().take_if(|k| *k == kind).is_some() {
            return Err(mem_err("put refused"));
        }
        if !cas::store::name_ok(name) {
            return Err(mem_err("bad name"));
        }
        self.0.objs.lock().insert((kind, String::from(name)), Vec::from(data));
        Ok(())
    }
    fn delete(&self, kind: Kind, name: &str) -> CasResult<()> {
        self.0.objs.lock().remove(&(kind, String::from(name)));
        Ok(())
    }
    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>> {
        if *self.0.fail_list.lock() == Some(kind) {
            return Err(mem_err("list refused"));
        }
        let mut v: Vec<(String, u64)> = self.0.objs.lock().iter().filter(|((k, _), _)| *k == kind).map(|((_, n), b)| (n.clone(), b.len() as u64)).collect();
        if self.0.junk.load(Ordering::SeqCst) {
            for n in ["", "../x", "a/b", "s1.tmp4", "C:", "bad\0name"] {
                v.push((String::from(n), u64::MAX));
            }
            // a pack id that is not there, of an absurd size, and a name that is none
            if kind == Kind::Pack || kind == Kind::Index {
                v.push((String::from("0123456789abcdef0123456789abcdef"), u64::MAX));
                v.push((String::from("0123456789abcdef0123456789abcdeg"), u64::MAX));
            }
        }
        Ok(v)
    }
}

// ------------------------------------------------------------------ the tests

pub fn run(t: &mut T) {
    chunking(t);
    let dir = tmp("repo");
    store(t, "folder", &|| Box::new(cas::Folder::new(&dir)));
    // a pack named by the public Store API with a letter of more than one byte first: its
    // folder (the first two bytes) cut the letter in two and panicked (third review)
    let f = cas::Folder::new(&dir);
    t.check("folder: a pack name with a multi-byte letter first is refused", f.get(Kind::Pack, "€a").is_err() && f.put(Kind::Pack, "a€", b"x").is_err() && f.get(Kind::Pack, "ab").is_ok());
    rm(&dir);
    let mem = Mem::new();
    store(t, "mem", &|| Box::new(mem.clone()));
    faults(t);
}

fn chunking(t: &mut T) {
    let ch = cas::chunker::Chunker::new(&[7u8; 32], 64 * 1024);
    let mut r = Rng(99);
    let data = r.bytes(3 << 20);
    let a = ch.split(&data);
    t.check("chunk sizes", a.iter().all(|&n| n <= ch.max) && a[..a.len() - 1].iter().all(|&n| n >= ch.min) && a.iter().sum::<usize>() == data.len());
    let avg = data.len() / a.len();
    t.check("chunk average", avg > 32 * 1024 && avg < 160 * 1024);
    // insert 100 bytes near the start: the later boundaries stay
    let mut changed = Vec::from(&data[..5000]);
    changed.extend(r.bytes(100));
    changed.extend_from_slice(&data[5000..]);
    let b = ch.split(&changed);
    let ends = |v: &[usize], shift: usize| -> Vec<usize> {
        let mut p = 0;
        v.iter().map(|n| { p += n; p - shift }).collect()
    };
    let ea: alloc::collections::BTreeSet<usize> = ends(&a, 0).into_iter().filter(|&e| e > 400_000).collect();
    let eb: alloc::collections::BTreeSet<usize> = ends(&b, 100).into_iter().filter(|&e| e > 400_000).collect();
    t.check("boundaries survive an insertion", ea == eb);
    // another key, other boundaries
    let other = cas::chunker::Chunker::new(&[8u8; 32], 64 * 1024);
    t.check("keyed gear", other.split(&data) != a);
}

fn opts(master: [u8; 32], recovery: Option<[u8; 32]>, signer: &Keypair) -> CreateOptions {
    CreateOptions { id: String::from("repo-1"), chunk_avg: 64 * 1024, compression: Codec::Lz4, master, recovery, signer: signer.public, immutable_days: 0 }
}

fn file(name: &str, ids: &[cas::ChunkId], size: usize) -> Node {
    Node { name: String::from(name), kind: NodeKind::File, size: size as u64, chunks: Vec::from(ids), mtime_ns: 1_700_000_000_000_000_000, ..Node::default() }
}

fn snap(id: &str, roots: Vec<Node>, imm: i64) -> Snapshot {
    Snapshot { id: String::from(id), created: 1_800_000_000, job_id: String::from("job"), job_name: String::from("Job"), host: String::from("host"), roots, full: true, immutable_until: imm, ..Snapshot::default() }
}

fn dir_of(tid: cas::ChunkId) -> Node {
    Node { name: String::from("/x"), kind: NodeKind::Dir, tree: Some(tid), ..Node::default() }
}

/// a repository's life in the store mk() hands out (each call: a new handle on the same place)
fn store(t: &mut T, tag: &str, mk: &dyn Fn() -> Box<dyn Store>) {
    let c = |s: &str| format!("{tag}: {s}");
    let master = [1u8; 32];
    let rec_secret = [2u8; 32];
    let rec_pub = crypto::x25519::public_key(&rec_secret);
    let signer = Keypair::from_seed(&[3u8; 32]);
    let opts = opts(master, Some(rec_pub), &signer);
    let repo = Repo::create_in(mk(), &opts);
    t.check(&c("create"), repo.is_ok());
    let Ok(repo) = repo else { return };
    t.check(&c("create needs an empty store"), matches!(Repo::create_in(mk(), &opts), Err(CasError::NotEmpty)));
    t.check(&c("one holder at a time"), matches!(Repo::open_in(mk(), Unlock::Master(&master)), Err(CasError::InUse)));
    t.check(&c("the id without opening it"), cas::peek_id_in(&*mk()).is_ok_and(|id| id == "repo-1") && repo.path() == mk().location());
    let mut r = Rng(5);
    let a = r.bytes(1 << 20);
    let text: Vec<u8> = b"Dateiserver-Sicherung ".iter().copied().cycle().take(2 << 20).collect();
    let (ida, idt, small);
    {
        let mut w = repo.writer();
        ida = put(&mut w, &a);
        idt = put(&mut w, &text);
        small = w.put(KIND_DATA, b"tiny").unwrap_or([0; 32]);
        let st = w.finish().unwrap_or_default();
        t.check(&c("text compresses"), st.new_stored < (a.len() + text.len() / 10) as u64 + 100_000);
    }
    // the same data again adds nothing
    {
        let mut w = repo.writer();
        let again = put(&mut w, &a);
        let st = w.finish().unwrap_or_default();
        t.check(&c("dedupe"), again == ida && st.new_chunks == 0 && st.new_stored == 0);
    }
    t.eq(&c("read back"), content(&repo, &ida), a.clone());
    t.eq(&c("read back compressed"), content(&repo, &idt), text.clone());
    t.eq(&c("small chunk"), repo.read_chunk(&small).unwrap_or_default(), Vec::from(&b"tiny"[..]));
    // a tree and a snapshot
    let tree = Tree { nodes: alloc::vec![file("a.bin", &ida, a.len()), file("text.txt", &idt, text.len())] };
    let mut w = repo.writer();
    let tid = w.put_tree(&tree).unwrap_or([0; 32]);
    let _ = w.finish();
    t.eq(&c("tree round trip"), repo.read_tree(&tid).ok(), Some(tree.clone()));
    let root = Node { name: String::from("/srv/data"), kind: NodeKind::Dir, tree: Some(tid), ..Node::default() };
    t.check(&c("save snapshot"), repo.save_snapshot(&snap("s1", alloc::vec![root.clone()], 0), &signer).is_ok());
    let loaded = repo.load_snapshot("s1");
    t.check(&c("load snapshot"), loaded.as_ref().is_ok_and(|s| s.roots[0].tree == Some(tid) && s.job_name == "Job"));
    // a snapshot signed by a stranger is refused
    let stranger = Keypair::from_seed(&[9u8; 32]);
    let forged = cas::snapshot::seal(&repo.keys().snapshot, &stranger, &snap("s2", alloc::vec![root.clone()], 0)).unwrap_or_default();
    let _ = mk().put(Kind::Snapshot, "s2", &forged);
    t.check(&c("unknown signer refused"), matches!(repo.load_snapshot("s2"), Err(CasError::Corrupt(_))));
    let _ = mk().delete(Kind::Snapshot, "s2");
    // ... and may not save one through the API either (it is not trusted by saving)
    t.check(&c("unknown signer may not save"), matches!(repo.save_snapshot(&snap("s2", alloc::vec![root.clone()], 0), &stranger), Err(CasError::UntrustedSigner)) && repo.snapshot_ids() == ["s1"]);
    t.check(&c("unknown signer still refused on load"), matches!(repo.load_snapshot("s2"), Err(CasError::NotFound(_))));
    // trust_signer lets another key save (a restored server with a new key)
    t.check(&c("trust another signer"), repo.trust_signer(&stranger.public).is_ok());
    t.check(&c("trusted signer saves"), repo.save_snapshot(&snap("s2", alloc::vec![root.clone()], 0), &stranger).is_ok() && repo.load_snapshot("s2").is_ok());
    t.check(&c("delete the second signer's snapshot"), repo.delete_snapshot("s2", 1_900_000_000).is_ok());
    // immutability
    t.check(&c("immutable snapshot"), repo.save_snapshot(&snap("s3", alloc::vec![root.clone()], 2_000_000_000), &signer).is_ok());
    t.check(&c("delete refused while immutable"), matches!(repo.delete_snapshot("s3", 1_900_000_000), Err(CasError::Immutable(_))));
    t.check(&c("delete after the time"), repo.delete_snapshot("s3", 2_100_000_000).is_ok());
    // passphrase unlock
    t.check(&c("add passphrase"), repo.add_passphrase("eine lange Passphrase", "job", Unlock::Master(&master)).is_ok());
    let check = repo.check(true, &mut |_, _| true);
    t.check(&c("check clean"), check.as_ref().is_ok_and(|c| c.ok() && c.snapshots == 1 && c.missing == 0 && c.damaged == 0));
    drop(repo);
    t.check(&c("wrong master key"), matches!(Repo::open_in(mk(), Unlock::Master(&[0u8; 32])), Err(CasError::Locked)));
    t.check(&c("wrong passphrase"), matches!(Repo::open_in(mk(), Unlock::Passphrase("falsch falsch falsch")), Err(CasError::Locked)));
    let by_pass = Repo::open_in(mk(), Unlock::Passphrase("eine lange Passphrase"));
    t.check(&c("passphrase unlock"), by_pass.as_ref().is_ok_and(|r| r.load_snapshot("s1").is_ok()));
    drop(by_pass);
    let by_kit = Repo::open_in(mk(), Unlock::Recovery(&rec_secret));
    t.check(&c("recovery kit unlock"), by_kit.as_ref().is_ok_and(|r| content(r, &ida) == a));
    drop(by_kit);
    // repo.json with its MAC taken away (and a signer changed): not sealed again, refused
    let cfg_text = mk().get(Kind::Config, "repo").ok().flatten().unwrap_or_default();
    let text = String::from_utf8_lossy(&cfg_text).into_owned();
    let stripped = match text.find("\"mac\":\"") {
        Some(i) => {
            let end = text[i + 7..].find('"').map_or(text.len(), |k| i + 8 + k);
            let (head, tail) = (text[..i].trim_end(), &text[end..]);
            match (head.strip_suffix(','), tail.trim_start().strip_prefix(',')) {
                (Some(h), _) => format!("{h}{tail}"),
                (None, Some(t2)) => format!("{head}{t2}"),
                _ => format!("{head}{tail}"),
            }
        }
        None => text.clone(),
    };
    t.check(&c("repo.json holds a MAC"), stripped != text);
    let _ = mk().put(Kind::Config, "repo", stripped.as_bytes());
    t.check(&c("repo.json without its MAC refused"), matches!(Repo::open_in(mk(), Unlock::Master(&master)), Err(CasError::Corrupt(_))));
    t.check(&c("... and left as it was"), mk().get(Kind::Config, "repo").ok().flatten().is_some_and(|b| !String::from_utf8_lossy(&b).contains("\"mac\"")));
    let _ = mk().put(Kind::Config, "repo", &cfg_text);
    let Ok(repo) = Repo::open_in(mk(), Unlock::Master(&master)) else {
        t.check(&c("reopen"), false);
        return;
    };
    // gc: drop the snapshot -> everything goes
    {
        let mut w = repo.writer();
        let _ = put(&mut w, &r.bytes(300_000)); // never referenced: garbage
        let _ = w.finish();
    }
    let before = repo.stored_bytes();
    let g = repo.gc(&mut |_, _| true);
    t.check(&c("gc keeps referenced data"), g.as_ref().is_ok_and(|g| g.chunks_removed > 0) && content(&repo, &ida) == a && repo.stored_bytes() < before);
    t.check(&c("delete s1"), repo.delete_snapshot("s1", 1_900_000_000).is_ok());
    let g = repo.gc(&mut |_, _| true);
    t.check(&c("gc frees everything"), g.is_ok() && repo.chunk_count() == 0 && repo.stored_bytes() == 0 && mk().list(Kind::Index).is_ok_and(|l| l.is_empty()));
    // between a writer's end and its snapshot the chunks belong to no snapshot.  The plain
    // order (finish, a gc, save_snapshot): the snapshot is refused, not stored without its data
    let repo = Arc::new(repo);
    let mut w = repo.writer();
    let ids = put(&mut w, &r.bytes(200_000));
    let tid = w.put_tree(&Tree { nodes: alloc::vec![file("l.bin", &ids, 200_000)] }).unwrap_or([0; 32]);
    let _ = w.finish();
    let _ = repo.gc(&mut |_, _| true);
    t.check(&c("a snapshot whose chunks a gc took is refused"), matches!(repo.save_snapshot(&snap("s5", alloc::vec![dir_of(tid)], 0), &signer), Err(CasError::NotFound(_))) && repo.snapshot_ids().is_empty());
    // finish_unsaved: a gc of another thread waits until the snapshot is saved, and keeps it
    let mut w = repo.writer();
    let ids = put(&mut w, &r.bytes(200_000));
    let tid = w.put_tree(&Tree { nodes: alloc::vec![file("l.bin", &ids, 200_000)] }).unwrap_or([0; 32]);
    let unsaved = w.finish_unsaved().ok().map(|(_, u)| u);
    let done = Arc::new(AtomicU32::new(0)); // 1: the gc removed nothing, 2: it did (or failed)
    let gc_of = |repo: Arc<Repo>, done: Arc<AtomicU32>| {
        let kept = repo.gc(&mut |_, _| true).is_ok_and(|g| g.chunks_removed == 0);
        done.store(if kept { 1 } else { 2 }, Ordering::SeqCst);
    };
    let (r2, d2) = (repo.clone(), done.clone());
    let h = plat::thread_spawn(0, Box::new(move || gc_of(r2, d2)));
    plat::sleep_ms(150);
    t.check(&c("gc waits for an unsaved writer"), done.load(Ordering::SeqCst) == 0);
    let saved = unsaved.is_some_and(|u| u.save_snapshot(&snap("s6", alloc::vec![dir_of(tid)], 0), &signer).is_ok());
    match h {
        Some(h) => h.join(),
        None => gc_of(repo.clone(), done.clone()), // (no threads here: the gc after the save)
    }
    t.check(&c("the gc after it keeps the snapshot's chunks"), saved && done.load(Ordering::SeqCst) == 1 && content(&repo, &ids).len() == 200_000);
    t.check(&c("a snapshot after a gc that took none of it"), repo.save_snapshot(&snap("s7", alloc::vec![dir_of(tid)], 0), &signer).is_ok());
    let gone = repo.delete_snapshot("s6", 1_900_000_000).is_ok() && repo.delete_snapshot("s7", 1_900_000_000).is_ok();
    t.check(&c("the two snapshots deleted, gc frees everything again"), gone && repo.gc(&mut |_, _| true).is_ok() && repo.chunk_count() == 0);
    // a flipped bit is found by the check
    let mut w = repo.writer();
    let ids = put(&mut w, &a);
    let tid = w.put_tree(&Tree { nodes: alloc::vec![file("a.bin", &ids, a.len())] }).unwrap_or([0; 32]);
    let _ = w.finish();
    let _ = repo.save_snapshot(&snap("s4", alloc::vec![dir_of(tid)], 0), &signer);
    drop(repo);
    let s = mk();
    let first = s.list(Kind::Pack).ok().and_then(|l| l.into_iter().next()).map(|(n, _)| n).unwrap_or_default();
    let mut b = s.get(Kind::Pack, &first).ok().flatten().unwrap_or_default();
    if b.len() > 1000 {
        b[700] ^= 0x10;
        let _ = s.put(Kind::Pack, &first, &b);
    }
    let Ok(repo) = Repo::open_in(mk(), Unlock::Master(&master)) else { return };
    t.check(&c("bit flip found"), repo.check(true, &mut |_, _| true).is_ok_and(|c| c.damaged == 1 && !c.ok()));
    t.check(&c("damaged chunk refused"), ids.iter().any(|id| repo.read_chunk(id).is_err()));
    // a lost index file is rebuilt from the pack's trailer
    drop(repo);
    let lose_index = || {
        let s = mk();
        for (n, _) in s.list(Kind::Index).unwrap_or_default() {
            let _ = s.delete(Kind::Index, &n);
        }
    };
    lose_index();
    let Ok(repo) = Repo::open_in(mk(), Unlock::Master(&master)) else { return };
    t.eq(&c("index lost"), repo.chunk_count(), 0);
    let fixed = repo.repair_index();
    t.check(&c("index rebuilt"), fixed.is_ok_and(|n| n >= 1) && repo.chunk_count() > 0);
    t.check(&c("snapshot readable again"), repo.load_snapshot("s4").is_ok_and(|s| repo.read_tree(&s.roots[0].tree.unwrap_or([0; 32])).is_ok()));
    // lost once more, and a gc comes first: it rebuilds the index too, the packs stay
    drop(repo);
    lose_index();
    let Ok(repo) = Repo::open_in(mk(), Unlock::Master(&master)) else { return };
    let packed = repo.stored_bytes();
    let g = repo.gc(&mut |_, _| true);
    t.check(&c("gc after a lost index keeps the snapshot's packs"), g.is_ok_and(|g| g.packs_deleted == 0) && packed > 0 && repo.stored_bytes() == packed);
    t.check(&c("and the snapshot readable"), repo.chunk_count() > 0 && repo.load_snapshot("s4").is_ok_and(|s| repo.read_tree(&s.roots[0].tree.unwrap_or([0; 32])).is_ok()));
    if tag == "folder" {
        // what a crash leaves in the folder (a temporary sibling, a pack below another
        // prefix) is no object
        let dir = mk().location();
        let snaps = plat::fs::join(&dir, "snapshots");
        let _ = plat::write_file(&plat::fs::join(&snaps, "s9.snap.tmp123"), b"x");
        let _ = plat::fs::mkdir_private(&plat::fs::join(&dir, "data/zz"));
        let _ = plat::write_file(&plat::fs::join(&dir, "data/zz/0123456789abcdef0123456789abcdef.pack"), b"x");
        t.check(&c("leftovers of a crash are no objects"), repo.snapshot_ids() == ["s4"] && mk().list(Kind::Pack).is_ok_and(|l| l.iter().all(|(n, _)| !n.starts_with("0123"))));
    }
    drop(repo);
}

/// a store that fails, cuts or makes things up
fn faults(t: &mut T) {
    let mem = Mem::new();
    let signer = Keypair::from_seed(&[3u8; 32]);
    let master = [1u8; 32];
    let Ok(repo) = Repo::create_in(mem.clone(), &opts(master, None, &signer)) else {
        t.check("store faults: create", false);
        return;
    };
    let mut r = Rng(77);
    let data = r.bytes(400_000);
    // the index of a pack cannot be written: the writer fails, the pack is left without one
    *mem.0.fail_put.lock() = Some(Kind::Index);
    let mut w = repo.writer();
    let ids = put(&mut w, &data);
    let tid = w.put_tree(&Tree { nodes: alloc::vec![file("d.bin", &ids, data.len())] }).unwrap_or([0; 32]);
    t.check("store faults: a failed write of the index fails the writer", matches!(w.finish(), Err(CasError::Io(_))));
    t.check("store faults: ... and the check finds the pack without an index", repo.check(false, &mut |_, _| true).is_ok_and(|c| c.orphan_packs == 1));
    // repaired from the pack's end: two ranges, not the whole pack
    let pack_size = mem.list(Kind::Pack).ok().and_then(|l| l.first().map(|(_, n)| *n)).unwrap_or(0);
    let before = mem.read();
    let fixed = repo.repair_index();
    let read = mem.read() - before;
    t.check("store faults: the index rebuilt", fixed.is_ok_and(|n| n == 1) && content(&repo, &ids) == data);
    t.check(&format!("store faults: the repair read the pack's end only ({read} of {pack_size} bytes)"), read > 0 && read * 4 < pack_size);
    t.check("store faults: a snapshot of it", repo.save_snapshot(&snap("f1", alloc::vec![dir_of(tid)], 0), &signer).is_ok());
    // a snapshot that cannot be written is not there
    *mem.0.fail_put.lock() = Some(Kind::Snapshot);
    t.check("store faults: a failed write of a snapshot", matches!(repo.save_snapshot(&snap("f2", alloc::vec![dir_of(tid)], 0), &signer), Err(CasError::Io(_))) && repo.snapshot_ids() == ["f1"]);
    // ranges a byte short: refused, no panic
    mem.0.short.store(true, Ordering::SeqCst);
    t.check("store faults: a short range is refused", ids.iter().all(|id| matches!(repo.read_chunk(id), Err(CasError::Corrupt(_)))));
    t.check("store faults: ... and the check counts it", repo.check(true, &mut |_, _| true).is_ok_and(|c| c.damaged > 0));
    mem.0.short.store(false, Ordering::SeqCst);
    // listings with names no repository writes and absurd sizes: left out, nothing overflows
    mem.0.junk.store(true, Ordering::SeqCst);
    t.check("store faults: names that are no snapshot id are left out", repo.snapshot_ids() == ["f1"]);
    let g = repo.gc(&mut |_, _| true);
    t.check("store faults: gc beside them", g.is_ok() && content(&repo, &ids) == data);
    t.check("store faults: the check beside them", repo.check(true, &mut |_, _| true).is_ok_and(|c| c.ok()));
    drop(repo);
    t.check("store faults: they open", Repo::open_in(mem.clone(), Unlock::Master(&master)).is_ok_and(|r| content(&r, &ids) == data));
    mem.0.junk.store(false, Ordering::SeqCst);
    // a listing of the snapshots that fails stops gc and check: no snapshot, but not "none"
    let Ok(repo) = Repo::open_in(mem.clone(), Unlock::Master(&master)) else { return };
    *mem.0.fail_list.lock() = Some(Kind::Snapshot);
    t.check("store faults: gc stops when the snapshots cannot be listed", repo.gc(&mut |_, _| true).is_err() && repo.check(false, &mut |_, _| true).is_err());
    *mem.0.fail_list.lock() = None;
    t.check("store faults: ... and took nothing", repo.chunk_count() > 0 && content(&repo, &ids) == data && repo.gc(&mut |_, _| true).is_ok_and(|g| g.packs_deleted == 0));
}
