// cast -- lib/cas: chunk boundaries survive an insertion, deduplication, trees and signed
// snapshots, the three ways to unlock, locking, immutability, gc, a flipped bit found by the
// check, an index rebuilt from a pack's trailer

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use cas::{CasError, CreateOptions, Node, NodeKind, Repo, Snapshot, Tree, Unlock, KIND_DATA};
use compress::Codec;
use crypto::ed25519::Keypair;

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

pub fn run(t: &mut T) {
    chunking(t);
    store(t);
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

fn store(t: &mut T) {
    let dir = tmp("repo");
    let master = [1u8; 32];
    let rec_secret = [2u8; 32];
    let rec_pub = crypto::x25519::public_key(&rec_secret);
    let signer = Keypair::from_seed(&[3u8; 32]);
    let opts = CreateOptions { id: String::from("repo-1"), chunk_avg: 64 * 1024, compression: Codec::Lz4, master, recovery: Some(rec_pub), signer: signer.public, immutable_days: 0 };
    let repo = Repo::create(&dir, &opts);
    t.check("create", repo.is_ok());
    let Ok(repo) = repo else { return };
    t.check("create needs an empty folder", matches!(Repo::create(&dir, &opts), Err(CasError::NotEmpty)));
    t.check("one process at a time", matches!(Repo::open(&dir, Unlock::Master(&master)), Err(CasError::InUse)));
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
        t.check("text compresses", st.new_stored < (a.len() + text.len() / 10) as u64 + 100_000);
    }
    // the same data again adds nothing
    {
        let mut w = repo.writer();
        let again = put(&mut w, &a);
        let st = w.finish().unwrap_or_default();
        t.check("dedupe", again == ida && st.new_chunks == 0 && st.new_stored == 0);
    }
    t.eq("read back", content(&repo, &ida), a.clone());
    t.eq("read back compressed", content(&repo, &idt), text.clone());
    t.eq("small chunk", repo.read_chunk(&small).unwrap_or_default(), Vec::from(&b"tiny"[..]));
    // a tree and a snapshot
    let file = |name: &str, ids: &[cas::ChunkId], size: usize| Node { name: String::from(name), kind: NodeKind::File, size: size as u64, chunks: Vec::from(ids), mtime_ns: 1_700_000_000_000_000_000, ..Node::default() };
    let tree = Tree { nodes: alloc::vec![file("a.bin", &ida, a.len()), file("text.txt", &idt, text.len())] };
    let mut w = repo.writer();
    let tid = w.put_tree(&tree).unwrap_or([0; 32]);
    let _ = w.finish();
    t.eq("tree round trip", repo.read_tree(&tid).ok(), Some(tree.clone()));
    let snap = |id: &str, roots: Vec<Node>, imm: i64| Snapshot { id: String::from(id), created: 1_800_000_000, job_id: String::from("job"), job_name: String::from("Job"), host: String::from("host"), roots, full: true, immutable_until: imm, ..Snapshot::default() };
    let root = Node { name: String::from("/srv/data"), kind: NodeKind::Dir, tree: Some(tid), ..Node::default() };
    t.check("save snapshot", repo.save_snapshot(&snap("s1", alloc::vec![root.clone()], 0), &signer).is_ok());
    let loaded = repo.load_snapshot("s1");
    t.check("load snapshot", loaded.as_ref().is_ok_and(|s| s.roots[0].tree == Some(tid) && s.job_name == "Job"));
    // a snapshot signed by a stranger is refused
    let stranger = Keypair::from_seed(&[9u8; 32]);
    let forged = cas::snapshot::seal(&repo.keys().snapshot, &stranger, &snap("s2", alloc::vec![root.clone()], 0)).unwrap_or_default();
    let _ = plat::write_file(&plat::fs::join(&plat::fs::join(&dir, "snapshots"), "s2.snap"), &forged);
    t.check("unknown signer refused", matches!(repo.load_snapshot("s2"), Err(CasError::Corrupt(_))));
    plat::delete(&plat::fs::join(&plat::fs::join(&dir, "snapshots"), "s2.snap"));
    // immutability
    t.check("immutable snapshot", repo.save_snapshot(&snap("s3", alloc::vec![root.clone()], 2_000_000_000), &signer).is_ok());
    t.check("delete refused while immutable", matches!(repo.delete_snapshot("s3", 1_900_000_000), Err(CasError::Immutable(_))));
    t.check("delete after the time", repo.delete_snapshot("s3", 2_100_000_000).is_ok());
    // passphrase unlock
    t.check("add passphrase", repo.add_passphrase("eine lange Passphrase", "job", Unlock::Master(&master)).is_ok());
    let check = repo.check(true, &mut |_, _| true);
    t.check("check clean", check.as_ref().is_ok_and(|c| c.ok() && c.snapshots == 1 && c.missing == 0 && c.damaged == 0));
    drop(repo);
    t.check("wrong master key", matches!(Repo::open(&dir, Unlock::Master(&[0u8; 32])), Err(CasError::Locked)));
    t.check("wrong passphrase", matches!(Repo::open(&dir, Unlock::Passphrase("falsch falsch falsch")), Err(CasError::Locked)));
    let by_pass = Repo::open(&dir, Unlock::Passphrase("eine lange Passphrase"));
    t.check("passphrase unlock", by_pass.as_ref().is_ok_and(|r| r.load_snapshot("s1").is_ok()));
    drop(by_pass);
    let by_kit = Repo::open(&dir, Unlock::Recovery(&rec_secret));
    t.check("recovery kit unlock", by_kit.as_ref().is_ok_and(|r| content(r, &ida) == a));
    drop(by_kit);
    let Ok(repo) = Repo::open(&dir, Unlock::Master(&master)) else {
        t.check("reopen", false);
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
    t.check("gc keeps referenced data", g.as_ref().is_ok_and(|g| g.chunks_removed > 0) && content(&repo, &ida) == a && repo.stored_bytes() < before);
    t.check("delete s1", repo.delete_snapshot("s1", 1_900_000_000).is_ok());
    let g = repo.gc(&mut |_, _| true);
    t.check("gc frees everything", g.is_ok() && repo.chunk_count() == 0 && repo.stored_bytes() == 0);
    // a flipped bit is found by the check
    let mut w = repo.writer();
    let ids = put(&mut w, &a);
    let tid = w.put_tree(&Tree { nodes: alloc::vec![file("a.bin", &ids, a.len())] }).unwrap_or([0; 32]);
    let _ = w.finish();
    let _ = repo.save_snapshot(&snap("s4", alloc::vec![Node { name: String::from("/x"), kind: NodeKind::Dir, tree: Some(tid), ..Node::default() }], 0), &signer);
    drop(repo);
    let data_dir = plat::fs::join(&dir, "data");
    let pack_path = plat::fs::read_dir(&data_dir)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|d| {
            let sub = plat::fs::join(&data_dir, &d.name);
            plat::fs::read_dir(&sub).unwrap_or_default().into_iter().map(move |f| plat::fs::join(&sub, &f.name))
        })
        .next()
        .unwrap_or_default();
    let mut b = plat::read_file(&pack_path).unwrap_or_default();
    if b.len() > 1000 {
        b[700] ^= 0x10;
        let _ = plat::write_file(&pack_path, &b);
    }
    let Ok(repo) = Repo::open(&dir, Unlock::Master(&master)) else { return };
    t.check("bit flip found", repo.check(true, &mut |_, _| true).is_ok_and(|c| c.damaged == 1 && !c.ok()));
    t.check("damaged chunk refused", ids.iter().any(|id| repo.read_chunk(id).is_err()));
    // a lost index file is rebuilt from the pack's trailer
    drop(repo);
    let idx_dir = plat::fs::join(&dir, "index");
    for e in plat::fs::read_dir(&idx_dir).unwrap_or_default() {
        plat::delete(&plat::fs::join(&idx_dir, &e.name));
    }
    let Ok(repo) = Repo::open(&dir, Unlock::Master(&master)) else { return };
    t.eq("index lost", repo.chunk_count(), 0);
    let fixed = repo.repair_index();
    t.check("index rebuilt", fixed.is_ok_and(|n| n >= 1) && repo.chunk_count() > 0);
    t.check("snapshot readable again", repo.load_snapshot("s4").is_ok_and(|s| repo.read_tree(&s.roots[0].tree.unwrap_or([0; 32])).is_ok()));
    drop(repo);
    rm(&dir);
}
