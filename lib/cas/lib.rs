// cas -- a content-addressed, deduplicating, encrypted chunk store (BackupCraft's repository
// format "bcrepo-1")
//
//   Repo::create(path, &CreateOptions) / Repo::open(path, Unlock::{Master, Passphrase,
//        Recovery})                       one process holds a repository at a time (LOCK)
//   repo.writer() -> Writer               put(kind, bytes) / put_stream(reader) -> chunk ids,
//                                         finish() (packs and indexes are durable);
//                                         .codec(c, level): this writer's compression
//                                         (the repository's own is the default)
//   repo.read_chunk(id) / read_tree(id)   repo.save_snapshot / load_snapshot / snapshot_ids /
//                                         delete_snapshot (refused while immutable)
//   repo.check(read_data, progress)       snapshots, signatures, references, every blob
//   repo.gc(progress)                     mark & sweep, repack sparse packs
//   repo.add_passphrase / set_recovery / set_immutable_days / trust_signer
//
// Layout:  repo.json  data/<xx>/<pack>.pack  index/<pack>.idx  snapshots/<id>.snap  LOCK
// Keys:    one random repository key, wrapped in repo.json for the server's master key, for
//          passphrases (Argon2id) and for the recovery kit (sealed box); per purpose keys are
//          derived from it with BLAKE3 derive_key (chunk ids, data, index, snapshots, gear).
// Chunks:  id = BLAKE3-keyed(id key, plain) -> identical data is stored once, the ids reveal
//          nothing without the key.  Commit order of a backup: packs (fsync) -> indexes
//          (fsync) -> snapshot (atomic rename, directory sync); a crash leaves at most
//          unreferenced packs, which gc removes.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod chunker;
pub mod pack;
pub mod snapshot;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use agent_core::json::{self, Limits, Value};
use compress::Codec;
use crypto::{aead, argon2, b64, ed25519};
use plat::sync::{Mutex, RwLock};

use crate::chunker::Chunker;
use crate::pack::{Loc, PackBuf, PackId};
pub use crate::snapshot::{Node, NodeKind, SnapStats, Snapshot, Tree};

pub const FORMAT: &str = "bcrepo-1";
pub type ChunkId = [u8; 32];
pub const KIND_DATA: u8 = 0;
pub const KIND_TREE: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasError {
    Io(String),
    /// damaged or forged data
    Corrupt(String),
    /// none of the unlock methods fits
    Locked,
    /// another process uses the repository
    InUse,
    NotARepository,
    NotEmpty,
    /// a snapshot that may not be deleted yet
    Immutable(i64),
    NotFound(String),
    Cancelled,
    TooLarge,
}

impl core::fmt::Display for CasError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CasError::Io(s) => write!(f, "Ein-/Ausgabefehler: {}", s),
            CasError::Corrupt(s) => write!(f, "beschädigte Daten: {}", s),
            CasError::Locked => f.write_str("der Schlüssel passt nicht"),
            CasError::InUse => f.write_str("das Repository wird von einem anderen Prozess verwendet"),
            CasError::NotARepository => f.write_str("kein BackupCraft-Repository"),
            CasError::NotEmpty => f.write_str("der Ordner ist nicht leer"),
            CasError::Immutable(t) => write!(f, "unveränderlich bis {}", t),
            CasError::NotFound(s) => write!(f, "nicht gefunden: {}", s),
            CasError::Cancelled => f.write_str("abgebrochen"),
            CasError::TooLarge => f.write_str("zu groß"),
        }
    }
}

pub type CasResult<T> = Result<T, CasError>;

pub(crate) fn big_limits() -> Limits {
    Limits { bytes: 256 << 20, depth: 40, values: 20_000_000, string_bytes: 16 << 20, members: 5_000_000 }
}

pub fn hex_id(id: &ChunkId) -> String {
    crypto::hex(id)
}

pub fn parse_id(s: &str) -> Option<ChunkId> {
    let b = crypto::unhex(s)?;
    (b.len() == 32).then(|| {
        let mut id = [0u8; 32];
        id.copy_from_slice(&b);
        id
    })
}

fn io(what: &str, path: &str) -> CasError {
    CasError::Io(format!("{} {}", what, path))
}

/// the per-purpose keys of a repository
pub struct Keys {
    pub id: [u8; 32],
    pub data: [u8; 32],
    pub index: [u8; 32],
    pub snapshot: [u8; 32],
    pub gear: [u8; 32],
}

impl Keys {
    fn derive(repo_key: &[u8; 32]) -> Keys {
        let d = |c: &str| crypto::blake3::derive_key(c, repo_key);
        Keys {
            id: d("BackupCraft bcrepo-1 chunk id"),
            data: d("BackupCraft bcrepo-1 data"),
            index: d("BackupCraft bcrepo-1 index"),
            snapshot: d("BackupCraft bcrepo-1 snapshot"),
            gear: d("BackupCraft bcrepo-1 gear"),
        }
    }

    pub fn chunk_id(&self, data: &[u8]) -> ChunkId {
        crypto::blake3::keyed_hash(&self.id, data)
    }
}

/// how to open a repository
pub enum Unlock<'a> {
    /// the server's repository wrap key
    Master(&'a [u8; 32]),
    Passphrase(&'a str),
    /// the recovery kit's X25519 secret
    Recovery(&'a [u8; 32]),
}

pub struct CreateOptions {
    pub id: String,
    /// average chunk size (a power of two; 1 MiB)
    pub chunk_avg: usize,
    pub compression: Codec,
    /// the server's wrap key
    pub master: [u8; 32],
    pub recovery: Option<[u8; 32]>,
    /// the public key whose snapshots this repository trusts
    pub signer: [u8; 32],
    pub immutable_days: u32,
}

const AAD_MASTER: &[u8] = b"bcrepo-1 master";
const AAD_PASS: &[u8] = b"bcrepo-1 passphrase";
const AAD_RECOVERY: &[u8] = b"bcrepo-1 recovery";

fn wrap(kek: &[u8; 32], aad: &[u8], repo_key: &[u8; 32]) -> (String, String) {
    let nonce: [u8; 24] = plat::random_bytes();
    (b64::encode_url(&nonce), b64::encode_url(&aead::x_seal(kek, &nonce, aad, repo_key)))
}

fn unwrap(kek: &[u8; 32], aad: &[u8], nonce: &str, wrapped: &str) -> Option<[u8; 32]> {
    let n = b64::decode_url(nonce)?;
    let w = b64::decode_url(wrapped)?;
    if n.len() != 24 {
        return None;
    }
    let mut nn = [0u8; 24];
    nn.copy_from_slice(&n);
    let k = aead::x_open(kek, &nn, aad, &w)?;
    (k.len() == 32).then(|| {
        let mut out = [0u8; 32];
        out.copy_from_slice(&k);
        out
    })
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn join(a: &str, b: &str) -> String {
    plat::fs::join(a, b)
}

/// statistics of a writer
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteStats {
    pub chunks: u64,
    pub raw: u64,
    pub new_chunks: u64,
    pub new_raw: u64,
    /// bytes added to pack files
    pub new_stored: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub snapshots: u64,
    pub bad_snapshots: Vec<String>,
    pub chunks_referenced: u64,
    pub missing: u64,
    pub damaged: u64,
    pub packs: u64,
    /// packs without an index (left by an interrupted write)
    pub orphan_packs: u64,
    pub bytes_read: u64,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.bad_snapshots.is_empty() && self.missing == 0 && self.damaged == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub packs_deleted: u64,
    pub packs_repacked: u64,
    pub bytes_freed: u64,
    pub chunks_removed: u64,
}

/// progress: (done, total) -> false to cancel
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

pub struct Repo {
    path: String,
    cfg: Mutex<Value>,
    keys: Keys,
    chunker: Chunker,
    codec: Codec,
    index: Mutex<BTreeMap<ChunkId, Loc>>,
    /// writers share, gc is exclusive
    gc_lock: RwLock<()>,
    files: Mutex<Vec<(PackId, Arc<plat::File>)>>,
    _lock: plat::FileLock,
}

impl Repo {
    fn cfg_path(path: &str) -> String {
        join(path, "repo.json")
    }

    /// create a repository in an empty (or new) folder
    pub fn create(path: &str, o: &CreateOptions) -> CasResult<Repo> {
        if plat::fs::read_dir(path).is_some_and(|l| !l.is_empty()) {
            return Err(CasError::NotEmpty);
        }
        for d in ["", "data", "index", "snapshots"] {
            let p = if d.is_empty() { String::from(path) } else { join(path, d) };
            if !plat::fs::mkdir_private(&p) {
                return Err(io("cannot create", &p));
            }
        }
        let repo_key: [u8; 32] = plat::random_bytes();
        let (n, w) = wrap(&o.master, AAD_MASTER, &repo_key);
        let mut keys = alloc::vec![Value::object([("kind", Value::string("master")), ("nonce", Value::string(n)), ("wrapped", Value::string(w))])];
        if let Some(pk) = &o.recovery {
            if let Some(sealed) = crypto::sealbox::seal(pk, &plat::random_bytes::<56>(), AAD_RECOVERY, &repo_key) {
                keys.push(Value::object([("kind", Value::string("recovery")), ("sealed", Value::string(b64::encode_url(&sealed)))]));
            }
        }
        let cfg = Value::object([
            ("format", Value::string(FORMAT)),
            ("id", Value::string(o.id.as_str())),
            ("created", Value::i64(plat::unix_time_ns() / 1_000_000_000)),
            ("chunk_avg", Value::u64(o.chunk_avg as u64)),
            ("compression", Value::string(o.compression.name())),
            ("immutable_days", Value::u64(o.immutable_days as u64)),
            ("signers", Value::Array(alloc::vec![Value::string(crypto::hex(&o.signer))])),
            ("keys", Value::Array(keys)),
        ]);
        write_cfg(path, &cfg)?;
        let mut rk = repo_key;
        let r = Repo::open_with_key(path, &rk, cfg);
        crypto::zeroize(&mut rk);
        r
    }

    /// open a repository
    pub fn open(path: &str, unlock: Unlock) -> CasResult<Repo> {
        let cfg = read_cfg(path)?;
        let entries = cfg.get("keys").and_then(|k| k.as_array()).unwrap_or(&[]);
        let mut key = None;
        for e in entries {
            key = match (&unlock, str_of(e, "kind")) {
                (Unlock::Master(k), "master") => unwrap(k, AAD_MASTER, str_of(e, "nonce"), str_of(e, "wrapped")),
                (Unlock::Passphrase(p), "passphrase") => {
                    let salt = b64::decode_url(str_of(e, "salt")).unwrap_or_default();
                    let params = argon2::Params {
                        m_kib: e.get("m").and_then(|x| x.as_u64()).unwrap_or(65536) as u32,
                        t: e.get("t").and_then(|x| x.as_u64()).unwrap_or(3) as u32,
                        p: e.get("p").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
                        out_len: 32,
                    };
                    let mut kek = [0u8; 32];
                    if argon2::argon2id(p.as_bytes(), &salt, &params, &mut kek) {
                        unwrap(&kek, AAD_PASS, str_of(e, "nonce"), str_of(e, "wrapped"))
                    } else {
                        None
                    }
                }
                (Unlock::Recovery(sk), "recovery") => b64::decode_url(str_of(e, "sealed")).and_then(|s| crypto::sealbox::open(sk, AAD_RECOVERY, &s)).and_then(|k| {
                    (k.len() == 32).then(|| {
                        let mut out = [0u8; 32];
                        out.copy_from_slice(&k);
                        out
                    })
                }),
                _ => None,
            };
            if key.is_some() {
                break;
            }
        }
        let Some(mut k) = key else { return Err(CasError::Locked) };
        let r = Repo::open_with_key(path, &k, cfg);
        crypto::zeroize(&mut k);
        r
    }

    fn open_with_key(path: &str, repo_key: &[u8; 32], cfg: Value) -> CasResult<Repo> {
        let lock = plat::FileLock::acquire(&join(path, "LOCK")).ok_or(CasError::InUse)?;
        let keys = Keys::derive(repo_key);
        let avg = cfg.get("chunk_avg").and_then(|x| x.as_u64()).unwrap_or(1 << 20) as usize;
        let codec = Codec::from_name(str_of(&cfg, "compression")).unwrap_or(Codec::Lz4);
        let chunker = Chunker::new(&keys.gear, avg);
        let repo = Repo { path: String::from(path), cfg: Mutex::new(cfg), keys, chunker, codec, index: Mutex::new(BTreeMap::new()), gc_lock: RwLock::new(()), files: Mutex::new(Vec::new()), _lock: lock };
        repo.load_index()?;
        Ok(repo)
    }

    fn load_index(&self) -> CasResult<()> {
        let dir = join(&self.path, "index");
        let mut map = BTreeMap::new();
        for e in plat::fs::read_dir(&dir).unwrap_or_default() {
            let Some(pid) = e.name.strip_suffix(".idx").and_then(pack::parse_pack) else { continue };
            let p = join(&dir, &e.name);
            let b = plat::read_file(&p).ok_or_else(|| io("cannot read", &p))?;
            for (id, loc) in pack::open_index(&self.keys, &pid, &b)? {
                map.entry(id).or_insert(loc);
            }
        }
        *self.index.lock() = map;
        Ok(())
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn id(&self) -> String {
        String::from(str_of(&self.cfg.lock(), "id"))
    }

    pub fn immutable_days(&self) -> u32 {
        self.cfg.lock().get("immutable_days").and_then(|x| x.as_u64()).unwrap_or(0) as u32
    }

    pub fn chunker(&self) -> &Chunker {
        &self.chunker
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// chunks in the index
    pub fn chunk_count(&self) -> usize {
        self.index.lock().len()
    }

    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        self.index.lock().contains_key(id)
    }

    /// bytes of all pack files
    pub fn stored_bytes(&self) -> u64 {
        let data = join(&self.path, "data");
        let mut sum = 0;
        for d in plat::fs::read_dir(&data).unwrap_or_default() {
            let sub = join(&data, &d.name);
            for f in plat::fs::read_dir(&sub).unwrap_or_default() {
                sum += plat::fs::stat(&join(&sub, &f.name)).map_or(0, |m| m.size);
            }
        }
        sum
    }

    fn update_cfg(&self, f: impl FnOnce(&mut Value)) -> CasResult<()> {
        let mut cfg = self.cfg.lock();
        let mut v = cfg.clone();
        f(&mut v);
        write_cfg(&self.path, &v)?;
        *cfg = v;
        Ok(())
    }

    /// let a passphrase open the repository (Argon2id, 64 MiB)
    pub fn add_passphrase(&self, pass: &str, label: &str, repo_key_from: Unlock) -> CasResult<()> {
        let rk = self.repo_key(repo_key_from)?;
        let salt: [u8; 16] = plat::random_bytes();
        let p = argon2::Params::KEY;
        let mut kek = [0u8; 32];
        if !argon2::argon2id(pass.as_bytes(), &salt, &p, &mut kek) {
            return Err(CasError::Locked);
        }
        let (n, w) = wrap(&kek, AAD_PASS, &rk);
        crypto::zeroize(&mut kek);
        self.update_cfg(|v| {
            let mut keys: Vec<Value> = v.get("keys").and_then(|k| k.as_array()).map(Vec::from).unwrap_or_default();
            keys.retain(|e| !(str_of(e, "kind") == "passphrase" && str_of(e, "label") == label));
            keys.push(Value::object([
                ("kind", Value::string("passphrase")),
                ("label", Value::string(label)),
                ("salt", Value::string(b64::encode_url(&salt))),
                ("m", Value::u64(p.m_kib as u64)),
                ("t", Value::u64(p.t as u64)),
                ("p", Value::u64(p.p as u64)),
                ("nonce", Value::string(n)),
                ("wrapped", Value::string(w)),
            ]));
            v.insert("keys", Value::Array(keys));
        })
    }

    /// whether a passphrase with this label opens the repository
    pub fn has_passphrase(&self, label: &str) -> bool {
        self.cfg.lock().get("keys").and_then(|k| k.as_array()).is_some_and(|a| a.iter().any(|e| str_of(e, "kind") == "passphrase" && str_of(e, "label") == label))
    }

    /// seal the repository key for a (new) recovery kit
    pub fn set_recovery(&self, pk: &[u8; 32], repo_key_from: Unlock) -> CasResult<()> {
        let rk = self.repo_key(repo_key_from)?;
        let sealed = crypto::sealbox::seal(pk, &plat::random_bytes::<56>(), AAD_RECOVERY, &rk).ok_or(CasError::Locked)?;
        self.update_cfg(|v| {
            let mut keys: Vec<Value> = v.get("keys").and_then(|k| k.as_array()).map(Vec::from).unwrap_or_default();
            keys.retain(|e| str_of(e, "kind") != "recovery");
            keys.push(Value::object([("kind", Value::string("recovery")), ("sealed", Value::string(b64::encode_url(&sealed)))]));
            v.insert("keys", Value::Array(keys));
        })
    }

    /// the repository key (to wrap it once more)
    fn repo_key(&self, from: Unlock) -> CasResult<[u8; 32]> {
        let cfg = self.cfg.lock().clone();
        for e in cfg.get("keys").and_then(|k| k.as_array()).unwrap_or(&[]) {
            if let (Unlock::Master(k), "master") = (&from, str_of(e, "kind")) {
                if let Some(rk) = unwrap(k, AAD_MASTER, str_of(e, "nonce"), str_of(e, "wrapped")) {
                    return Ok(rk);
                }
            }
        }
        Err(CasError::Locked)
    }

    /// raise the immutability (never lowers it)
    pub fn set_immutable_days(&self, days: u32) -> CasResult<()> {
        let cur = self.immutable_days();
        self.update_cfg(|v| {
            v.insert("immutable_days", Value::u64(days.max(cur) as u64));
        })
    }

    /// trust snapshots signed by another key too (a restored server)
    pub fn trust_signer(&self, pk: &[u8; 32]) -> CasResult<()> {
        let h = crypto::hex(pk);
        self.update_cfg(|v| {
            let mut s: Vec<Value> = v.get("signers").and_then(|k| k.as_array()).map(Vec::from).unwrap_or_default();
            if !s.iter().any(|x| x.as_str() == Some(&h)) {
                s.push(Value::string(h.as_str()));
            }
            v.insert("signers", Value::Array(s));
        })
    }

    fn trusted(&self, pk: &[u8; 32]) -> bool {
        let h = crypto::hex(pk);
        self.cfg.lock().get("signers").and_then(|k| k.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(&h)))
    }

    // ------------------------------------------------------------------ writing

    /// a writer for new chunks (backups run several at once; gc waits for them)
    pub fn writer(&self) -> Writer<'_> {
        Writer { repo: self, _g: self.gc_lock.read(), pack: PackBuf::new(), pending: BTreeMap::new(), stats: WriteStats::default(), codec: self.codec, level: compress::zstd::LEVEL_DEFAULT }
    }

    fn pack_path(&self, p: &PackId) -> String {
        let h = pack::pack_hex(p);
        join(&join(&join(&self.path, "data"), &h[..2]), &format!("{}.pack", h))
    }

    fn index_path(&self, p: &PackId) -> String {
        join(&join(&self.path, "index"), &format!("{}.idx", pack::pack_hex(p)))
    }

    /// write a full pack and its index durably
    fn store_pack(&self, buf: &mut PackBuf) -> CasResult<u64> {
        let bytes = buf.finish(&self.keys);
        let pp = self.pack_path(&buf.id);
        let dir = String::from(plat::fs::parent(&pp));
        if !plat::fs::mkdir_private(&dir) {
            return Err(io("cannot create", &dir));
        }
        if !plat::fs::write_file_private(&pp, &bytes) {
            return Err(io("cannot write", &pp));
        }
        let ip = self.index_path(&buf.id);
        if !plat::fs::write_file_private(&ip, &pack::seal_index(&self.keys, &buf.id, &buf.entries)) {
            return Err(io("cannot write", &ip));
        }
        let mut idx = self.index.lock();
        for (id, loc) in &buf.entries {
            idx.entry(*id).or_insert(*loc);
        }
        Ok(bytes.len() as u64)
    }

    // ------------------------------------------------------------------ reading

    fn pack_file(&self, p: &PackId) -> CasResult<Arc<plat::File>> {
        let mut files = self.files.lock();
        if let Some(i) = files.iter().position(|(id, _)| id == p) {
            let e = files.remove(i);
            let f = e.1.clone();
            files.push(e);
            return Ok(f);
        }
        let path = self.pack_path(p);
        let f = Arc::new(plat::File::open(&plat::fs::long_path(&path), plat::FO_READ).ok_or_else(|| io("cannot open", &path))?);
        if files.len() >= 32 {
            files.remove(0);
        }
        files.push((*p, f.clone()));
        Ok(f)
    }

    fn read_blob(&self, loc: &Loc) -> CasResult<Vec<u8>> {
        let f = self.pack_file(&loc.pack)?;
        let mut buf = alloc::vec![0u8; loc.len as usize];
        let mut done = 0;
        while done < buf.len() {
            match f.read_at(loc.off as u64 + done as u64, &mut buf[done..]) {
                Some(0) | None => return Err(CasError::Corrupt(format!("pack {} truncated", pack::pack_hex(&loc.pack)))),
                Some(n) => done += n,
            }
        }
        Ok(buf)
    }

    /// the plain content of a chunk
    pub fn read_chunk(&self, id: &ChunkId) -> CasResult<Vec<u8>> {
        let loc = self.index.lock().get(id).copied().ok_or_else(|| CasError::NotFound(format!("chunk {}", crypto::hex(&id[..8]))))?;
        let blob = self.read_blob(&loc)?;
        pack::open_blob(&self.keys, id, &blob, loc.raw as usize)
    }

    pub fn read_tree(&self, id: &ChunkId) -> CasResult<Tree> {
        Tree::decode(&self.read_chunk(id)?).ok_or_else(|| CasError::Corrupt(format!("tree {}", crypto::hex(&id[..8]))))
    }

    // ------------------------------------------------------------------ snapshots

    fn snap_path(&self, id: &str) -> String {
        join(&join(&self.path, "snapshots"), &format!("{}.snap", id))
    }

    /// store a snapshot (after its writer finished); signed by `signer`
    pub fn save_snapshot(&self, snap: &Snapshot, signer: &ed25519::Keypair) -> CasResult<()> {
        if !self.trusted(&signer.public) {
            self.trust_signer(&signer.public)?;
        }
        let b = snapshot::seal(&self.keys.snapshot, signer, snap)?;
        let p = self.snap_path(&snap.id);
        if !plat::fs::write_file_private(&p, &b) {
            return Err(io("cannot write", &p));
        }
        Ok(())
    }

    pub fn snapshot_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = plat::fs::read_dir(&join(&self.path, "snapshots"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| e.name.strip_suffix(".snap").map(String::from))
            .filter(|n| !n.contains(".tmp"))
            .collect();
        v.sort();
        v
    }

    /// a snapshot, checked (signature by a trusted key, authentic encryption)
    pub fn load_snapshot(&self, id: &str) -> CasResult<Snapshot> {
        if id.is_empty() || id.contains(['/', '\\', '.']) {
            return Err(CasError::NotFound(String::from(id)));
        }
        let p = self.snap_path(id);
        let b = plat::read_file(&p).ok_or_else(|| CasError::NotFound(String::from(id)))?;
        let (snap, signer) = snapshot::open(&self.keys.snapshot, id, &b)?;
        if !self.trusted(&signer) {
            return Err(CasError::Corrupt(format!("snapshot {} signed by an unknown key", id)));
        }
        Ok(snap)
    }

    /// delete a snapshot (its chunks go with the next gc) -- refused while it is immutable
    pub fn delete_snapshot(&self, id: &str, now: i64) -> CasResult<()> {
        let s = self.load_snapshot(id)?;
        let days = self.immutable_days() as i64;
        let until = s.immutable_until.max(if days > 0 { s.created + days * 86_400 } else { 0 });
        if until > now {
            return Err(CasError::Immutable(until));
        }
        if !plat::delete(&self.snap_path(id)) {
            return Err(io("cannot delete", &self.snap_path(id)));
        }
        plat::fs::sync_dir(&join(&self.path, "snapshots"));
        Ok(())
    }

    /// every chunk a snapshot references (trees and data)
    pub fn referenced(&self, snap: &Snapshot, out: &mut BTreeSet<ChunkId>) -> CasResult<()> {
        let mut stack: Vec<ChunkId> = Vec::new();
        for r in &snap.roots {
            out.extend(r.chunks.iter().copied());
            if let Some(t) = r.tree {
                if out.insert(t) {
                    stack.push(t);
                }
            }
        }
        while let Some(t) = stack.pop() {
            let tree = self.read_tree(&t)?;
            for n in &tree.nodes {
                out.extend(n.chunks.iter().copied());
                if let Some(sub) = n.tree {
                    if out.insert(sub) {
                        stack.push(sub);
                    }
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ maintenance

    fn pack_files(&self) -> Vec<(PackId, String)> {
        let data = join(&self.path, "data");
        let mut out = Vec::new();
        for d in plat::fs::read_dir(&data).unwrap_or_default() {
            let sub = join(&data, &d.name);
            for f in plat::fs::read_dir(&sub).unwrap_or_default() {
                if let Some(p) = f.name.strip_suffix(".pack").and_then(pack::parse_pack) {
                    out.push((p, join(&sub, &f.name)));
                }
            }
        }
        out
    }

    /// check the repository: every snapshot opens and verifies, every referenced chunk is
    /// indexed; with `read_data` every blob is read, decrypted and compared with its id
    pub fn check(&self, read_data: bool, progress: Progress) -> CasResult<CheckReport> {
        let _g = self.gc_lock.read();
        let mut rep = CheckReport::default();
        let mut referenced = BTreeSet::new();
        let ids = self.snapshot_ids();
        rep.snapshots = ids.len() as u64;
        for id in &ids {
            match self.load_snapshot(id).and_then(|s| self.referenced(&s, &mut referenced)) {
                Ok(()) => {}
                Err(e) => rep.bad_snapshots.push(format!("{}: {}", id, e)),
            }
        }
        rep.chunks_referenced = referenced.len() as u64;
        let idx = self.index.lock().clone();
        rep.missing = referenced.iter().filter(|c| !idx.contains_key(*c)).count() as u64;
        let packs = self.pack_files();
        rep.packs = packs.len() as u64;
        let indexed: BTreeSet<PackId> = idx.values().map(|l| l.pack).collect();
        rep.orphan_packs = packs.iter().filter(|(p, _)| !indexed.contains(p)).count() as u64;
        if read_data {
            let total = idx.len() as u64;
            for (k, (id, loc)) in idx.iter().enumerate() {
                if !progress(k as u64, total) {
                    return Err(CasError::Cancelled);
                }
                rep.bytes_read += loc.len as u64;
                if self.read_blob(loc).and_then(|b| pack::open_blob(&self.keys, id, &b, loc.raw as usize)).is_err() {
                    rep.damaged += 1;
                }
            }
        }
        Ok(rep)
    }

    /// remove what no snapshot references: empty packs are deleted, sparse packs (less than
    /// 60 % in use) are rewritten with their live blobs
    pub fn gc(&self, progress: Progress) -> CasResult<GcReport> {
        let _g = self.gc_lock.write();
        let mut live = BTreeSet::new();
        for id in self.snapshot_ids() {
            // a snapshot that does not open stops gc: its chunks must not be lost
            let s = self.load_snapshot(&id)?;
            self.referenced(&s, &mut live)?;
        }
        let mut rep = GcReport::default();
        let packs = self.pack_files();
        let total = packs.len() as u64;
        let mut by_pack: BTreeMap<PackId, Vec<(ChunkId, Loc)>> = BTreeMap::new();
        for (id, loc) in self.index.lock().iter() {
            by_pack.entry(loc.pack).or_default().push((*id, *loc));
        }
        let mut out = PackBuf::new();
        let mut moved: Vec<(ChunkId, Loc)> = Vec::new();
        let mut to_delete: Vec<(PackId, String, u64)> = Vec::new();
        for (k, (pid, path)) in packs.iter().enumerate() {
            if !progress(k as u64, total) {
                return Err(CasError::Cancelled);
            }
            let size = plat::fs::stat(path).map_or(0, |m| m.size);
            // the entries of this pack that are the index's copy and still referenced
            let entries: Vec<(ChunkId, Loc)> = by_pack.remove(pid).unwrap_or_default();
            let used: Vec<&(ChunkId, Loc)> = entries.iter().filter(|(i, _)| live.contains(i)).collect();
            let used_bytes: u64 = used.iter().map(|(_, l)| l.len as u64).sum();
            if used.is_empty() {
                to_delete.push((*pid, path.clone(), size));
                rep.chunks_removed += entries.len() as u64;
                continue;
            }
            if used_bytes * 10 >= size * 6 {
                continue; // dense enough
            }
            for (id, loc) in &used {
                let blob = self.read_blob(loc)?;
                let nl = out.push(*id, loc.kind, loc.raw as usize, &blob);
                moved.push((*id, nl));
                if out.data.len() >= pack::PACK_TARGET {
                    self.store_repacked(&mut out)?;
                    out = PackBuf::new();
                }
            }
            rep.chunks_removed += (entries.len() - used.len()) as u64;
            to_delete.push((*pid, path.clone(), size));
            rep.packs_repacked += 1;
        }
        if !out.is_empty() {
            self.store_repacked(&mut out)?;
        }
        // the moved chunks now live in the new packs
        {
            let mut idx = self.index.lock();
            for (id, loc) in &moved {
                idx.insert(*id, *loc);
            }
            let dead: BTreeSet<PackId> = to_delete.iter().map(|(p, _, _)| *p).collect();
            idx.retain(|_, l| !dead.contains(&l.pack));
        }
        self.files.lock().clear();
        for (pid, path, size) in to_delete {
            plat::delete(&self.index_path(&pid));
            if plat::delete(&path) {
                rep.bytes_freed += size;
                rep.packs_deleted += 1;
            }
        }
        plat::fs::sync_dir(&join(&self.path, "index"));
        Ok(rep)
    }

    fn store_repacked(&self, buf: &mut PackBuf) -> CasResult<()> {
        // the new pack is complete and durable before any old one goes
        let entries = buf.entries.clone();
        self.store_pack(buf)?;
        let mut idx = self.index.lock();
        for (id, loc) in entries {
            idx.insert(id, loc);
        }
        Ok(())
    }

    /// rebuild missing index files from the packs' trailers
    pub fn repair_index(&self) -> CasResult<u64> {
        let _g = self.gc_lock.write();
        let mut fixed = 0;
        for (pid, path) in self.pack_files() {
            if plat::file_exists(&self.index_path(&pid)) == 1 {
                continue;
            }
            let b = plat::read_file(&path).ok_or_else(|| io("cannot read", &path))?;
            let Ok(entries) = pack::trailer_index(&self.keys, &pid, &b) else {
                continue; // no trailer: an interrupted write, gc removes it
            };
            if !plat::fs::write_file_private(&self.index_path(&pid), &pack::seal_index(&self.keys, &pid, &entries)) {
                return Err(io("cannot write", &self.index_path(&pid)));
            }
            let mut idx = self.index.lock();
            for (id, loc) in entries {
                idx.entry(id).or_insert(loc);
            }
            fixed += 1;
        }
        Ok(fixed)
    }
}

fn read_cfg(path: &str) -> CasResult<Value> {
    let b = plat::read_file(&Repo::cfg_path(path)).ok_or(CasError::NotARepository)?;
    let v = json::parse_with_limits(&b, big_limits()).map_err(|_| CasError::NotARepository)?;
    if str_of(&v, "format") != FORMAT {
        return Err(CasError::NotARepository);
    }
    Ok(v)
}

fn write_cfg(path: &str, v: &Value) -> CasResult<()> {
    let text = json::stringify_with_limits(v, big_limits()).map_err(|_| CasError::TooLarge)?;
    let p = Repo::cfg_path(path);
    if !plat::fs::write_file_private(&p, text.as_bytes()) {
        return Err(io("cannot write", &p));
    }
    Ok(())
}

/// the id of a repository folder without opening it
pub fn peek_id(path: &str) -> CasResult<String> {
    Ok(String::from(str_of(&read_cfg(path)?, "id")))
}

/// writes new chunks into packs
pub struct Writer<'a> {
    repo: &'a Repo,
    _g: plat::sync::RwReadGuard<'a, ()>,
    pack: PackBuf,
    pending: BTreeMap<ChunkId, ()>,
    pub stats: WriteStats,
    codec: Codec,
    level: i32,
}

impl Writer<'_> {
    /// compress this writer's data chunks with `codec` (`level`: zstd's level); trees stay LZ4
    pub fn codec(mut self, codec: Codec, level: i32) -> Self {
        self.codec = codec;
        self.level = level;
        self
    }

    /// store a chunk (once) -> its id
    pub fn put(&mut self, kind: u8, data: &[u8]) -> CasResult<ChunkId> {
        let id = self.repo.keys.chunk_id(data);
        self.stats.chunks += 1;
        self.stats.raw += data.len() as u64;
        if self.pending.contains_key(&id) || self.repo.index.lock().contains_key(&id) {
            return Ok(id);
        }
        let (codec, level) = if kind == KIND_TREE { (Codec::Lz4, 0) } else { (self.codec, self.level) };
        let blob = pack::seal_blob(&self.repo.keys, &id, kind, data, codec, level);
        self.pack.push(id, kind, data.len(), &blob);
        self.pending.insert(id, ());
        self.stats.new_chunks += 1;
        self.stats.new_raw += data.len() as u64;
        if self.pack.data.len() >= pack::PACK_TARGET {
            self.flush()?;
        }
        Ok(id)
    }

    /// chunk a stream and store it -> (chunk ids, size); `read` fills the buffer, Ok(0) = end
    pub fn put_stream(&mut self, read: &mut dyn FnMut(&mut [u8]) -> Result<usize, String>, cancel: &dyn Fn() -> bool) -> CasResult<(Vec<ChunkId>, u64)> {
        let max = self.repo.chunker.max;
        let mut buf: Vec<u8> = Vec::with_capacity(max * 2);
        let mut ids = Vec::new();
        let mut size = 0u64;
        let mut eof = false;
        loop {
            while !eof && buf.len() < max * 2 {
                if cancel() {
                    return Err(CasError::Cancelled);
                }
                let old = buf.len();
                buf.resize(old + (max * 2 - old).min(4 << 20), 0);
                let n = read(&mut buf[old..]).map_err(CasError::Io)?;
                buf.truncate(old + n);
                if n == 0 {
                    eof = true;
                }
            }
            let mut pos = 0;
            while pos < buf.len() {
                let Some(n) = self.repo.chunker.cut(&buf[pos..], eof) else { break };
                ids.push(self.put(KIND_DATA, &buf[pos..pos + n])?);
                size += n as u64;
                pos += n;
            }
            buf.drain(..pos);
            if eof && buf.is_empty() {
                return Ok((ids, size));
            }
        }
    }

    pub fn put_tree(&mut self, t: &Tree) -> CasResult<ChunkId> {
        self.put(KIND_TREE, &t.encode())
    }

    fn flush(&mut self) -> CasResult<()> {
        if self.pack.is_empty() {
            return Ok(());
        }
        let mut p = core::mem::take(&mut self.pack);
        self.stats.new_stored += self.repo.store_pack(&mut p)?;
        self.pending.clear();
        Ok(())
    }

    /// write the last pack; every chunk put so far is durable afterwards
    pub fn finish(mut self) -> CasResult<WriteStats> {
        self.flush()?;
        Ok(self.stats)
    }
}
