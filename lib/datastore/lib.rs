// datastore -- an embedded, crash-safe store for one server process: collections of JSON
// records (id -> value) kept in memory, made durable by a write-ahead log and snapshots, plus
// append-only journals for logs that grow without bound (audit, session logs)
//
//   Store::open(dir) / open_with(dir, Options) -> Result<Store, StoreError>
//   get(coll, id) / list(coll) / count(coll) / ids(coll)          (clones of the values)
//   write(|tx| { tx.put(coll, id, v); tx.delete(coll, id); }) -> Result<seq>
//   update(coll, id, |v| ...)                                     read-modify-write in one txn
//   compact()                                                     snapshot now
//   add_index(coll, name, Index::field("owner").unique()) / find / find_one / range
//                                 secondary indexes in memory (index.rs), unique ones refuse a
//                                 transaction that breaks them (StoreError::Conflict)
//   Store::open_encrypted(dir, opts, &key) / rekey(Some(&key) | None)   encryption at rest
//                                 (seal.rs): snapshots and log sealed, rekey encrypts, changes
//                                 the key or decrypts
//   Journal::open(dir, durable) / open_encrypted(dir, durable, &key) / append(name, yyyymm,
//        &value) / tail(name, n) / read_month(name, yyyymm) / rekey
//
// Files in `dir`: LOCK, snap-<seq>.bcs, wal-<first seq>.log; journal/<name>/<yyyymm>.jl.
//   snapshot: "BCS1" seq:u64 | colls:u32 | per collection name, records:u32 | per record id,
//             JSON | BLAKE3 of everything before (32 bytes); encrypted: "BCE1" (seal.rs)
//   WAL frame: "BCW1" len:u32 seq:u64 payload BLAKE3(len ‖ seq ‖ payload)[..16]; the payload is
//             a JSON array of ["put", coll, id, value] / ["del", coll, id]; encrypted: "BCW2"
//             and the payload sealed (seal.rs)
// A transaction is one frame, written and fsynced before it becomes visible.  Opening loads the
// newest valid snapshot and replays the frames after it.  A torn frame at the very end (a crash
// during a write) is cut off; a damaged frame with intact data behind it stops the open with
// StoreError::Corrupt unless Options::repair is set (then the log is cut there).  An encrypted
// store always has a snapshot (one at seq 0 when it is new): its key id names the key.  A store
// opened with the wrong key, or one of the other kind, is StoreError::Key, never repaired.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod index;
mod journal;
mod seal;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use core::ops::RangeBounds;

use formats::json::{self, Limits, Value};
use plat::sync::Mutex;

pub use index::{Index, Key, Order};
pub use journal::Journal;

use index::Idx;
use seal::{Keys, Unseal, SNAP_SEALED, WAL_SEALED};

const SNAP_MAGIC: &[u8; 4] = b"BCS1";
const WAL_MAGIC: &[u8; 4] = b"BCW1";
const FRAME_HEAD: usize = 16; // magic, len, seq
const FRAME_TAIL: usize = 16;
/// one transaction's payload at most
const MAX_FRAME: usize = 64 << 20;

/// the limits of one stored record
pub fn record_limits() -> Limits {
    Limits { bytes: 16 << 20, depth: 32, values: 1_000_000, string_bytes: 8 << 20, members: 200_000 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// another process holds the store
    Locked,
    /// reading or writing a file failed
    Io(String),
    /// a damaged log frame with intact data behind it (open with repair to cut the log there)
    Corrupt(String),
    /// a record or transaction over the size limits
    TooLarge,
    /// a transaction or a new index that would give two records the key of a unique index
    Conflict(String),
    /// the store is encrypted and no key was given, it is not and one was, or it is sealed
    /// with another key
    Key(String),
    /// no index of this name on the collection (Store::add_index)
    NoIndex(String),
}

const ENCRYPTED: &str = "the store is encrypted: open it with its key (Store::open_encrypted)";
const NOT_ENCRYPTED: &str = "the store is not encrypted: open it with Store::open (rekey encrypts it)";
const OTHER_KEY: &str = "the store is sealed with another key";

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreError::Locked => f.write_str("the data directory is in use by another process"),
            StoreError::Io(s) => write!(f, "I/O error: {}", s),
            StoreError::Corrupt(s) => write!(f, "damaged write-ahead log: {}", s),
            StoreError::TooLarge => f.write_str("record too large"),
            StoreError::Conflict(s) => write!(f, "unique index: {}", s),
            StoreError::Key(s) => f.write_str(s),
            StoreError::NoIndex(s) => write!(f, "no index {}", s),
        }
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// write a snapshot (and start a new log) when the log grows beyond this
    pub snapshot_bytes: u64,
    /// cut a damaged log at the damage instead of refusing to open
    pub repair: bool,
    /// fsync every transaction (off only for tests and bulk imports)
    pub durable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { snapshot_bytes: 16 << 20, repair: false, durable: true }
    }
}

/// what open() found
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    pub snapshot_seq: u64,
    pub frames_replayed: u64,
    /// bytes cut from the end of the log (a torn last write)
    pub truncated_bytes: u64,
    /// the damage that was cut away with Options::repair
    pub repaired: Option<String>,
}

enum Op {
    Put(String, String, Value),
    Del(String, String),
}

/// the changes of one transaction
pub struct Tx {
    ops: Vec<Op>,
}

impl Tx {
    pub fn put(&mut self, coll: &str, id: &str, v: Value) {
        self.ops.push(Op::Put(String::from(coll), String::from(id), v));
    }

    pub fn delete(&mut self, coll: &str, id: &str) {
        self.ops.push(Op::Del(String::from(coll), String::from(id)));
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// the records of every collection: collection -> id -> value
pub type Data = BTreeMap<String, BTreeMap<String, Value>>;

struct State {
    data: Data,
    seq: u64,
    wal: plat::File,
    wal_path: String,
    wal_len: u64,
    snap_seq: u64,
    /// the keys of an encrypted store
    keys: Option<Keys>,
    /// collection -> index name -> index
    indexes: BTreeMap<String, BTreeMap<String, Idx>>,
}

impl State {
    fn index(&self, coll: &str, name: &str) -> StoreResult<&Idx> {
        self.indexes.get(coll).and_then(|m| m.get(name)).ok_or_else(|| StoreError::NoIndex(format!("{}.{}", coll, name)))
    }

    /// whether a transaction keeps every unique index unique -> the first conflict
    fn conflict(&self, ops: &[Op]) -> Option<String> {
        let mut after: BTreeMap<&str, BTreeMap<&str, Option<&Value>>> = BTreeMap::new();
        for op in ops {
            match op {
                Op::Put(c, i, v) => after.entry(c.as_str()).or_default().insert(i.as_str(), Some(v)),
                Op::Del(c, i) => after.entry(c.as_str()).or_default().insert(i.as_str(), None),
            };
        }
        for (coll, recs) in &after {
            for (name, idx) in self.indexes.get(*coll).into_iter().flatten() {
                if let Some(why) = idx.check(recs) {
                    return Some(format!("{}.{}: {}", coll, name, why));
                }
            }
        }
        None
    }
}

/// the store (share it between threads by reference or Arc)
pub struct Store {
    dir: String,
    opts: Options,
    st: Mutex<State>,
    _lock: plat::FileLock,
    recovery: Recovery,
}

fn join(dir: &str, name: &str) -> String {
    plat::fs::join(dir, name)
}

fn io(what: &str, path: &str) -> StoreError {
    StoreError::Io(format!("{} {}", what, path))
}

fn digest16(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = crypto::Blake3::new();
    for p in parts {
        h.update(p);
    }
    let d = h.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[..16]);
    out
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.bytes(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.bytes(8)?;
        Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u32()? as usize;
        core::str::from_utf8(self.bytes(n)?).ok()
    }
}

fn encode_snapshot(data: &Data, seq: u64) -> StoreResult<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(SNAP_MAGIC);
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    for (coll, recs) in data {
        put_str(&mut out, coll);
        out.extend_from_slice(&(recs.len() as u32).to_le_bytes());
        for (id, v) in recs {
            put_str(&mut out, id);
            let text = json::stringify_with_limits(v, record_limits()).map_err(|_| StoreError::TooLarge)?;
            put_str(&mut out, &text);
        }
    }
    let d = crypto::blake3::hash(&out);
    out.extend_from_slice(&d);
    Ok(out)
}

/// a snapshot file -> its data, None when it is damaged; Err(Key) for one of the other kind
/// (sealed without a key, plain with one) or sealed with another key
fn load_snapshot(b: &[u8], keys: Option<&Keys>) -> StoreResult<Option<(u64, Data)>> {
    match (b.get(..4), keys) {
        (Some(m), None) if m == SNAP_SEALED => Err(StoreError::Key(String::from(ENCRYPTED))),
        (Some(m), Some(_)) if m == SNAP_MAGIC => Err(StoreError::Key(String::from(NOT_ENCRYPTED))),
        (_, None) => Ok(decode_snapshot(b)),
        (_, Some(k)) => match k.open_snapshot(b) {
            Err(Unseal::OtherKey) => Err(StoreError::Key(String::from(OTHER_KEY))),
            Err(Unseal::Damaged) => Ok(None),
            // (the seq of the sealed header and of the snapshot inside agree)
            Ok((seq, plain)) => Ok(decode_snapshot(&plain).filter(|(s, _)| *s == seq)),
        },
    }
}

fn decode_snapshot(b: &[u8]) -> Option<(u64, Data)> {
    if b.len() < 4 + 8 + 4 + 32 || &b[..4] != SNAP_MAGIC {
        return None;
    }
    let (body, sum) = b.split_at(b.len() - 32);
    if !crypto::ct_eq(&crypto::blake3::hash(body), sum) {
        return None;
    }
    let mut r = Reader { b: body, i: 4 };
    let seq = r.u64()?;
    let colls = r.u32()?;
    let mut data = Data::new();
    for _ in 0..colls {
        let name = String::from(r.str()?);
        let n = r.u32()?;
        let mut recs = BTreeMap::new();
        for _ in 0..n {
            let id = String::from(r.str()?);
            let v = json::parse_with_limits(r.str()?.as_bytes(), record_limits()).ok()?;
            recs.insert(id, v);
        }
        data.insert(name, recs);
    }
    Some((seq, data))
}

fn encode_ops(ops: &[Op]) -> StoreResult<Vec<u8>> {
    let arr: Vec<Value> = ops
        .iter()
        .map(|o| match o {
            Op::Put(c, i, v) => Value::Array(alloc::vec![Value::string("put"), Value::string(c.as_str()), Value::string(i.as_str()), v.clone()]),
            Op::Del(c, i) => Value::Array(alloc::vec![Value::string("del"), Value::string(c.as_str()), Value::string(i.as_str())]),
        })
        .collect();
    let lim = Limits { bytes: MAX_FRAME, depth: 34, values: 4_000_000, string_bytes: 8 << 20, members: 400_000 };
    json::stringify_with_limits(&Value::Array(arr), lim).map(String::into_bytes).map_err(|_| StoreError::TooLarge)
}

fn apply_payload(data: &mut Data, payload: &[u8]) -> bool {
    let lim = Limits { bytes: MAX_FRAME, depth: 34, values: 4_000_000, string_bytes: 8 << 20, members: 400_000 };
    let Ok(v) = json::parse_with_limits(payload, lim) else { return false };
    let Some(ops) = v.as_array() else { return false };
    for op in ops {
        let Some(f) = op.as_array() else { return false };
        let (Some(kind), Some(coll), Some(id)) = (f.first().and_then(Value::as_str), f.get(1).and_then(Value::as_str), f.get(2).and_then(Value::as_str)) else {
            return false;
        };
        match kind {
            "put" => {
                let Some(val) = f.get(3) else { return false };
                data.entry(String::from(coll)).or_default().insert(String::from(id), val.clone());
            }
            "del" => {
                if let Some(c) = data.get_mut(coll) {
                    c.remove(id);
                }
            }
            _ => return false,
        }
    }
    true
}

fn frame(magic: &[u8; 4], seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_HEAD + payload.len() + FRAME_TAIL);
    out.extend_from_slice(magic);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(payload);
    let d = digest16(&[&out[4..16], payload]);
    out.extend_from_slice(&d);
    out
}

/// the result of scanning one log file
enum Scan {
    /// every frame valid
    Clean,
    /// valid up to this offset, then an incomplete or damaged last frame
    Torn(u64),
    /// valid up to this offset, damaged frame with more data behind it
    Damaged(u64, String),
    /// a frame of the other kind (sealed without a key, plain with one): never repaired
    Key(String),
}

/// the magic of a frame, plain or sealed
fn frame_magic(b: &[u8]) -> bool {
    b == WAL_MAGIC || b == WAL_SEALED
}

/// replay the frames of a log file with seq > after into data (`keys`: an encrypted store,
/// whose frames after the snapshot are all sealed)
fn replay(bytes: &[u8], after: u64, keys: Option<&Keys>, data: &mut Data, last_seq: &mut u64, replayed: &mut u64) -> Scan {
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest.len() < FRAME_HEAD + FRAME_TAIL || !frame_magic(&rest[..4]) {
            // an incomplete header, or garbage: torn when nothing valid can follow
            return if rest.len() < FRAME_HEAD + FRAME_TAIL || find_frame(rest).is_none() {
                Scan::Torn(i as u64)
            } else {
                Scan::Damaged(i as u64, format!("bad frame header at {}", i))
            };
        }
        let len = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let seq = u64::from_le_bytes([rest[8], rest[9], rest[10], rest[11], rest[12], rest[13], rest[14], rest[15]]);
        let total = FRAME_HEAD + len + FRAME_TAIL;
        if len > MAX_FRAME || rest.len() < total {
            return if len <= MAX_FRAME && find_frame(&rest[4..]).is_none() {
                Scan::Torn(i as u64)
            } else {
                Scan::Damaged(i as u64, format!("frame length {} at {}", len, i))
            };
        }
        let payload = &rest[FRAME_HEAD..FRAME_HEAD + len];
        let want = digest16(&[&rest[4..16], payload]);
        if !crypto::ct_eq(&want, &rest[FRAME_HEAD + len..total]) {
            // damaged: torn when it is the last thing in the file
            return if rest.len() == total || find_frame(&rest[total..]).is_none() {
                Scan::Torn(i as u64)
            } else {
                Scan::Damaged(i as u64, format!("checksum of frame {} at {}", seq, i))
            };
        }
        if seq > after {
            if seq != last_seq.saturating_add(1) && *last_seq != 0 {
                return Scan::Damaged(i as u64, format!("sequence gap {} -> {}", last_seq, seq));
            }
            let sealed = &rest[..4] == WAL_SEALED;
            let opened;
            let plain = match (keys, sealed) {
                (None, false) => payload,
                (Some(k), true) => match k.open_frame(seq, payload) {
                    Some(p) => {
                        opened = p;
                        &opened[..]
                    }
                    // its checksum holds: changed, not torn
                    None => return Scan::Damaged(i as u64, format!("frame {} does not authenticate", seq)),
                },
                (None, true) => return Scan::Key(String::from(ENCRYPTED)),
                (Some(_), false) => return Scan::Key(String::from(NOT_ENCRYPTED)),
            };
            if !apply_payload(data, plain) {
                return Scan::Damaged(i as u64, format!("unreadable frame {}", seq));
            }
            *replayed += 1;
        }
        *last_seq = (*last_seq).max(seq);
        i += total;
    }
    Scan::Clean
}

/// the offset of a whole valid frame in the bytes (its magic, length and checksum): only that
/// makes a damaged frame one with more behind it.  A bare "BCW1" was enough, and the payload of
/// a torn last frame holding those bytes made the store refuse to open instead of cutting it
fn find_frame(b: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(k) = b[from..].windows(4).position(frame_magic) {
        let at = from + k;
        let rest = &b[at..];
        if rest.len() >= FRAME_HEAD + FRAME_TAIL {
            let len = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
            let total = FRAME_HEAD + len + FRAME_TAIL;
            if len <= MAX_FRAME && rest.len() >= total {
                let want = digest16(&[&rest[4..16], &rest[FRAME_HEAD..FRAME_HEAD + len]]);
                if crypto::ct_eq(&want, &rest[FRAME_HEAD + len..total]) {
                    return Some(at);
                }
            }
        }
        from = at + 1;
    }
    None
}

fn parse_hex_name(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let h = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    u64::from_str_radix(h, 16).ok()
}

impl Store {
    pub fn open(dir: &str) -> StoreResult<Store> {
        Store::open_with(dir, Options::default())
    }

    pub fn open_with(dir: &str, opts: Options) -> StoreResult<Store> {
        Store::open_keyed(dir, opts, None)
    }

    /// open an encrypted store (a new one in an empty or missing folder) with its 32-byte key,
    /// for example one kept in the system's secret store (plat::secret::secret_get).  A store
    /// in the clear, or one sealed with another key, is StoreError::Key
    pub fn open_encrypted(dir: &str, opts: Options, key: &[u8; 32]) -> StoreResult<Store> {
        Store::open_keyed(dir, opts, Some(Keys::derive(key)))
    }

    fn open_keyed(dir: &str, opts: Options, keys: Option<Keys>) -> StoreResult<Store> {
        if !plat::fs::mkdir_private(dir) {
            return Err(io("cannot create", dir));
        }
        let lock = plat::FileLock::acquire(&join(dir, "LOCK")).ok_or(StoreError::Locked)?;
        let entries = plat::fs::read_dir(dir).ok_or_else(|| io("cannot list", dir))?;
        let mut snaps: Vec<u64> = entries.iter().filter_map(|e| parse_hex_name(&e.name, "snap-", ".bcs")).collect();
        let mut wals: Vec<u64> = entries.iter().filter_map(|e| parse_hex_name(&e.name, "wal-", ".log")).collect();
        snaps.sort_unstable();
        wals.sort_unstable();
        // the newest snapshot that checks out
        let mut data = Data::new();
        let mut snap_seq = 0u64;
        let mut skipped = None;
        for &s in snaps.iter().rev() {
            let loaded = match plat::read_file(&join(dir, &snap_name(s))) {
                Some(b) => load_snapshot(&b, keys.as_ref())?,
                None => None,
            };
            if let Some((seq, d)) = loaded {
                data = d;
                snap_seq = seq;
                break;
            }
            // the logs before a newer snapshot are gone, so an older one is a silent rollback
            if !opts.repair {
                return Err(StoreError::Corrupt(format!("{} is damaged (open with repair to use an older snapshot)", snap_name(s))));
            }
            skipped.get_or_insert(snap_name(s));
        }
        let mut rec = Recovery { snapshot_seq: snap_seq, ..Recovery::default() };
        if let Some(s) = skipped {
            rec.repaired = Some(format!("{} damaged, older snapshot used", s));
        }
        // logs that start after seq 1 need the snapshot that precedes them
        if snap_seq == 0 && !opts.repair && wals.first().is_some_and(|&w| w > 1) {
            return Err(StoreError::Corrupt(format!("{} needs a snapshot that is missing (open with repair to use what is left)", wal_name(wals[0]))));
        }
        let mut last = snap_seq;
        let mut replayed = 0u64;
        let n = wals.len();
        for (k, &w) in wals.iter().enumerate() {
            let path = join(dir, &wal_name(w));
            let bytes = plat::read_file(&path).ok_or_else(|| io("cannot read", &path))?;
            let is_last = k + 1 == n;
            match replay(&bytes, snap_seq, keys.as_ref(), &mut data, &mut last, &mut replayed) {
                Scan::Clean => {}
                Scan::Key(why) => return Err(StoreError::Key(why)),
                Scan::Torn(at) if is_last => {
                    cut(&path, at)?;
                    rec.truncated_bytes = bytes.len() as u64 - at;
                }
                Scan::Torn(at) | Scan::Damaged(at, _) if opts.repair => {
                    cut(&path, at)?;
                    rec.repaired = Some(format!("{} cut at {}", wal_name(w), at));
                    // later logs depend on the lost frames
                    for &later in &wals[k + 1..] {
                        plat::delete(&join(dir, &wal_name(later)));
                    }
                    break;
                }
                Scan::Torn(at) => return Err(StoreError::Corrupt(format!("{}: incomplete frame at {} before later logs", wal_name(w), at))),
                Scan::Damaged(_, why) => return Err(StoreError::Corrupt(format!("{}: {}", wal_name(w), why))),
            }
        }
        rec.frames_replayed = replayed;
        // continue in the newest log (or start one)
        let start = wals.last().copied().unwrap_or(last + 1);
        let wal_path = join(dir, &wal_name(start));
        let wal = plat::File::open(&wal_path, plat::FO_RW).ok_or_else(|| io("cannot open", &wal_path))?;
        let wal_len = wal.size().unwrap_or(0);
        if wals.is_empty() {
            plat::fs::sync_dir(dir);
        }
        let pin = keys.is_some() && snaps.is_empty();
        let st = State { data, seq: last, wal, wal_path, wal_len, snap_seq, keys, indexes: BTreeMap::new() };
        let store = Store { dir: String::from(dir), opts, st: Mutex::new(st), _lock: lock, recovery: rec };
        if pin {
            // an encrypted store always has a snapshot: its key id names the key, so a wrong
            // one is StoreError::Key on the next open (a new store: seq 0)
            let mut st = store.st.lock();
            store.snapshot(&mut st)?;
        }
        Ok(store)
    }

    /// whether the store is encrypted
    pub fn encrypted(&self) -> bool {
        self.st.lock().keys.is_some()
    }

    /// what open() found and repaired
    pub fn recovery(&self) -> &Recovery {
        &self.recovery
    }

    /// the sequence number of the last transaction
    pub fn seq(&self) -> u64 {
        self.st.lock().seq
    }

    pub fn get(&self, coll: &str, id: &str) -> Option<Value> {
        self.st.lock().data.get(coll)?.get(id).cloned()
    }

    pub fn contains(&self, coll: &str, id: &str) -> bool {
        self.st.lock().data.get(coll).is_some_and(|c| c.contains_key(id))
    }

    /// every record of a collection, ordered by id
    pub fn list(&self, coll: &str) -> Vec<(String, Value)> {
        self.st.lock().data.get(coll).map(|c| c.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
    }

    /// the records for which `f` holds
    pub fn filter(&self, coll: &str, mut f: impl FnMut(&str, &Value) -> bool) -> Vec<(String, Value)> {
        self.st.lock().data.get(coll).map(|c| c.iter().filter(|(k, v)| f(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
    }

    pub fn ids(&self, coll: &str) -> Vec<String> {
        self.st.lock().data.get(coll).map(|c| c.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn count(&self, coll: &str) -> usize {
        self.st.lock().data.get(coll).map_or(0, |c| c.len())
    }

    /// run a transaction: the changes `f` records become durable and visible together
    /// (f sees the current data; nothing is written when it returns Err or records nothing)
    pub fn write<E: From<StoreError>>(&self, f: impl FnOnce(&Data, &mut Tx) -> Result<(), E>) -> Result<u64, E> {
        let mut st = self.st.lock();
        let mut tx = Tx { ops: Vec::new() };
        f(&st.data, &mut tx)?;
        if tx.ops.is_empty() {
            return Ok(st.seq);
        }
        self.commit(&mut st, tx).map_err(E::from)
    }

    /// put one record
    pub fn put(&self, coll: &str, id: &str, v: Value) -> StoreResult<u64> {
        self.write(|_, tx| {
            tx.put(coll, id, v);
            Ok::<(), StoreError>(())
        })
    }

    /// delete one record
    pub fn delete(&self, coll: &str, id: &str) -> StoreResult<u64> {
        self.write(|_, tx| {
            tx.delete(coll, id);
            Ok::<(), StoreError>(())
        })
    }

    /// change one record in place (None: it does not exist)
    pub fn update(&self, coll: &str, id: &str, f: impl FnOnce(&mut Value)) -> StoreResult<Option<Value>> {
        let mut out = None;
        self.write(|data, tx| {
            if let Some(v) = data.get(coll).and_then(|c| c.get(id)) {
                let mut v = v.clone();
                f(&mut v);
                out = Some(v.clone());
                tx.put(coll, id, v);
            }
            Ok::<(), StoreError>(())
        })?;
        Ok(out)
    }

    fn commit(&self, st: &mut State, tx: Tx) -> StoreResult<u64> {
        // the unique indexes, before anything is written
        if let Some(why) = st.conflict(&tx.ops) {
            return Err(StoreError::Conflict(why));
        }
        let plain = encode_ops(&tx.ops)?;
        let seq = st.seq + 1;
        let (magic, payload) = match &st.keys {
            Some(k) => (WAL_SEALED, k.seal_frame(seq, &plain)),
            None => (WAL_MAGIC, plain),
        };
        if payload.len() > MAX_FRAME {
            return Err(StoreError::TooLarge);
        }
        let f = frame(magic, seq, &payload);
        let off = st.wal_len;
        if st.wal.write_at(off, &f) != Some(f.len()) {
            // leave no half frame behind
            st.wal.set_len(off);
            return Err(io("cannot write", &st.wal_path));
        }
        if self.opts.durable && !st.wal.sync() {
            st.wal.set_len(off);
            return Err(io("cannot sync", &st.wal_path));
        }
        st.wal_len = off + f.len() as u64;
        st.seq = seq;
        for op in tx.ops {
            match op {
                Op::Put(c, i, v) => {
                    for idx in st.indexes.get_mut(&c).into_iter().flat_map(|m| m.values_mut()) {
                        idx.put(&i, &v);
                    }
                    st.data.entry(c).or_default().insert(i, v);
                }
                Op::Del(c, i) => {
                    for idx in st.indexes.get_mut(&c).into_iter().flat_map(|m| m.values_mut()) {
                        idx.remove(&i);
                    }
                    if let Some(m) = st.data.get_mut(&c) {
                        m.remove(&i);
                    }
                }
            }
        }
        if st.wal_len > self.opts.snapshot_bytes {
            // the transaction is durable already; a failed snapshot is retried next time
            let _ = self.snapshot(st);
        }
        Ok(seq)
    }

    /// write a snapshot now and start a new log
    pub fn compact(&self) -> StoreResult<()> {
        let mut st = self.st.lock();
        self.snapshot(&mut st)
    }

    fn snapshot(&self, st: &mut State) -> StoreResult<()> {
        let plain = encode_snapshot(&st.data, st.seq)?;
        let bytes = match &st.keys {
            Some(k) => k.seal_snapshot(st.seq, &plain),
            None => plain,
        };
        let path = join(&self.dir, &snap_name(st.seq));
        if !plat::fs::write_file_private(&path, &bytes) {
            return Err(io("cannot write", &path));
        }
        // the new log starts after the snapshot
        self.start_log(st)?;
        let old_snap = st.snap_seq;
        st.snap_seq = st.seq;
        // older logs and all but the previous snapshot go
        if let Some(list) = plat::fs::read_dir(&self.dir) {
            for e in list {
                if let Some(w) = parse_hex_name(&e.name, "wal-", ".log") {
                    if w <= st.seq && join(&self.dir, &e.name) != st.wal_path {
                        plat::delete(&join(&self.dir, &e.name));
                    }
                }
                if let Some(s) = parse_hex_name(&e.name, "snap-", ".bcs") {
                    if s < old_snap {
                        plat::delete(&join(&self.dir, &e.name));
                    }
                }
            }
        }
        Ok(())
    }

    /// go on in the log that starts after st.seq (the snapshot there is written).  That is the
    /// open one when nothing was written since it began (a second compact): Windows opens a
    /// log for writing once, so it is kept, emptied (it holds no frame yet)
    fn start_log(&self, st: &mut State) -> StoreResult<()> {
        let wal_path = join(&self.dir, &wal_name(st.seq + 1));
        if wal_path != st.wal_path {
            // the file, its name and its length change together, once it is empty: on an error
            // the log open so far stays the one written to (the new one was set beside it with
            // the old length)
            let wal = plat::File::open(&wal_path, plat::FO_RW).ok_or_else(|| io("cannot open", &wal_path))?;
            if !wal.set_len(0) {
                return Err(io("cannot truncate", &wal_path));
            }
            st.wal = wal;
            st.wal_path = wal_path;
        } else if !st.wal.set_len(0) {
            return Err(io("cannot truncate", &st.wal_path));
        }
        st.wal_len = 0;
        plat::fs::sync_dir(&self.dir);
        Ok(())
    }

    /// encrypt the store with `key`, change its key, or (None) keep it in the clear again: a
    /// snapshot under the new key and a new log, then every other snapshot and log goes (they
    /// are under the old key).  Removed, not wiped: on an SSD nothing promises the old blocks
    /// are gone.  A crash in between leaves the new snapshot, which the new key opens.  Err(Io)
    /// after the switch names files under the old key that could not be removed
    pub fn rekey(&self, key: Option<&[u8; 32]>) -> StoreResult<()> {
        let mut st = self.st.lock();
        let keys = key.map(Keys::derive);
        let plain = encode_snapshot(&st.data, st.seq)?;
        let bytes = match &keys {
            Some(k) => k.seal_snapshot(st.seq, &plain),
            None => plain,
        };
        let path = join(&self.dir, &snap_name(st.seq));
        if !plat::fs::write_file_private(&path, &bytes) {
            return Err(io("cannot write", &path));
        }
        // the newest snapshot is under the new key now: so is everything written after it, also
        // when the new log does not start (the frames then follow in the open one; an open
        // skips those up to the snapshot unread)
        st.snap_seq = st.seq;
        st.keys = keys;
        // the new log, empty (no frame can be in it yet: it starts after the snapshot)
        self.start_log(&mut st)?;
        let (keep_snap, keep_wal) = (st.seq, st.seq + 1);
        // still locked: a commit or compact of another thread would make a newer snapshot and
        // log, which this would take for old ones (all commits after them lost)
        let mut left = Vec::new();
        for e in plat::fs::read_dir(&self.dir).unwrap_or_default() {
            let old = parse_hex_name(&e.name, "snap-", ".bcs").is_some_and(|s| s != keep_snap) || parse_hex_name(&e.name, "wal-", ".log").is_some_and(|w| w != keep_wal);
            if old && !plat::delete(&join(&self.dir, &e.name)) {
                left.push(e.name);
            }
        }
        plat::fs::sync_dir(&self.dir);
        if !left.is_empty() {
            return Err(StoreError::Io(format!("the store has its new key; files under the old one stay: {}", left.join(", "))));
        }
        Ok(())
    }

    // ------------------------------------------------------------------ indexes

    /// index the records of a collection (built now from what is there, kept up to date by
    /// every commit; replaces an index of the same name).  A unique index over records that
    /// share a key is refused (Conflict), and from then on every transaction that would make
    /// two share one.  Indexes are not stored: add them after each open
    pub fn add_index(&self, coll: &str, name: &str, def: Index) -> StoreResult<()> {
        let mut st = self.st.lock();
        let empty = BTreeMap::new();
        let idx = Idx::build(def, st.data.get(coll).unwrap_or(&empty).iter()).map_err(|why| StoreError::Conflict(format!("{}.{}: {}", coll, name, why)))?;
        st.indexes.entry(String::from(coll)).or_default().insert(String::from(name), idx);
        Ok(())
    }

    /// remove an index -> whether there was one
    pub fn drop_index(&self, coll: &str, name: &str) -> bool {
        self.st.lock().indexes.get_mut(coll).is_some_and(|m| m.remove(name).is_some())
    }

    /// the names of a collection's indexes
    pub fn index_names(&self, coll: &str) -> Vec<String> {
        self.st.lock().indexes.get(coll).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    /// the records that have `key` in the index, ordered by id
    pub fn find(&self, coll: &str, index: &str, key: &Key) -> StoreResult<Vec<(String, Value)>> {
        self.range(coll, index, (core::ops::Bound::Included(key.clone()), core::ops::Bound::Included(key.clone())), Order::Asc, usize::MAX)
    }

    /// the first record that has `key` (the one of a unique index)
    pub fn find_one(&self, coll: &str, index: &str, key: &Key) -> StoreResult<Option<(String, Value)>> {
        let k = (core::ops::Bound::Included(key.clone()), core::ops::Bound::Included(key.clone()));
        Ok(self.range(coll, index, k, Order::Asc, 1)?.pop())
    }

    /// the records whose keys lie in `keys`, by key and then id (Desc: the other way round),
    /// at most `limit` (usize::MAX: all); a record with several keys in the range comes once
    /// for each
    pub fn range(&self, coll: &str, index: &str, keys: impl RangeBounds<Key>, order: Order, limit: usize) -> StoreResult<Vec<(String, Value)>> {
        let st = self.st.lock();
        let idx = st.index(coll, index)?;
        let mut out = Vec::new();
        let (Some(recs), true) = (st.data.get(coll), bounds_ok(&keys)) else { return Ok(out) };
        let r = idx.map.range((keys.start_bound().cloned(), keys.end_bound().cloned()));
        let mut take = |ids: &mut dyn Iterator<Item = &String>| {
            for id in ids {
                if out.len() >= limit {
                    return false;
                }
                if let Some(v) = recs.get(id) {
                    out.push((id.clone(), v.clone()));
                }
            }
            true
        };
        match order {
            Order::Asc => {
                for (_, ids) in r {
                    if !take(&mut ids.iter()) {
                        break;
                    }
                }
            }
            Order::Desc => {
                for (_, ids) in r.rev() {
                    if !take(&mut ids.iter().rev()) {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }

    /// how many records have `key` in the index
    pub fn count_key(&self, coll: &str, index: &str, key: &Key) -> StoreResult<usize> {
        Ok(self.st.lock().index(coll, index)?.map.get(key).map_or(0, BTreeSet::len))
    }
}

/// bounds BTreeMap::range takes (it panics on a start after the end, and on an empty range
/// with both ends excluded)
fn bounds_ok(r: &impl RangeBounds<Key>) -> bool {
    use core::ops::Bound::{Excluded, Included};
    match (r.start_bound(), r.end_bound()) {
        (Included(s) | Excluded(s), Included(e) | Excluded(e)) if s > e => false,
        (Excluded(s), Excluded(e)) if s == e => false,
        _ => true,
    }
}

fn snap_name(seq: u64) -> String {
    format!("snap-{:016x}.bcs", seq)
}

fn wal_name(seq: u64) -> String {
    format!("wal-{:016x}.log", seq)
}

fn cut(path: &str, at: u64) -> StoreResult<()> {
    let f = plat::File::open(path, plat::FO_RW).ok_or_else(|| io("cannot open", path))?;
    if !f.set_len(at) || !f.sync() {
        return Err(io("cannot truncate", path));
    }
    Ok(())
}
