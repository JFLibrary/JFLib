// datastore -- an embedded, crash-safe store for one server process: collections of JSON
// records (id -> value) kept in memory, made durable by a write-ahead log and snapshots, plus
// append-only journals for logs that grow without bound (audit, session logs)
//
//   Store::open(dir) / open_with(dir, Options) -> Result<Store, StoreError>
//   get(coll, id) / list(coll) / count(coll) / ids(coll)          (clones of the values)
//   write(|tx| { tx.put(coll, id, v); tx.delete(coll, id); }) -> Result<seq>
//   update(coll, id, |v| ...)                                     read-modify-write in one txn
//   compact()                                                     snapshot now
//   Journal::open(dir) / append(name, &value) / tail(name, n) / read_month(name, yyyymm)
//
// Files in `dir`: LOCK, snap-<seq>.bcs, wal-<first seq>.log; journal/<name>/<yyyymm>.jl.
//   snapshot: "BCS1" seq:u64 | colls:u32 | per collection name, records:u32 | per record id,
//             JSON | BLAKE3 of everything before (32 bytes)
//   WAL frame: "BCW1" len:u32 seq:u64 payload BLAKE3(len ‖ seq ‖ payload)[..16]; the payload is
//             a JSON array of ["put", coll, id, value] / ["del", coll, id]
// A transaction is one frame, written and fsynced before it becomes visible.  Opening loads the
// newest valid snapshot and replays the frames after it.  A torn frame at the very end (a crash
// during a write) is cut off; a damaged frame with intact data behind it stops the open with
// StoreError::Corrupt unless Options::repair is set (then the log is cut there).
// Values are plain JSON: secrets are sealed by the caller (crypto::aead) before they get here.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod journal;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Limits, Value};
use plat::sync::Mutex;

pub use journal::Journal;

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
pub enum StoreError {
    /// another process holds the store
    Locked,
    /// reading or writing a file failed
    Io(String),
    /// a damaged log frame with intact data behind it (open with repair to cut the log there)
    Corrupt(String),
    /// a record or transaction over the size limits
    TooLarge,
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreError::Locked => f.write_str("the data directory is in use by another process"),
            StoreError::Io(s) => write!(f, "I/O error: {}", s),
            StoreError::Corrupt(s) => write!(f, "damaged write-ahead log: {}", s),
            StoreError::TooLarge => f.write_str("record too large"),
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

fn frame(seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_HEAD + payload.len() + FRAME_TAIL);
    out.extend_from_slice(WAL_MAGIC);
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
}

/// replay the frames of a log file with seq > after into data
fn replay(bytes: &[u8], after: u64, data: &mut Data, last_seq: &mut u64, replayed: &mut u64) -> Scan {
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest.len() < FRAME_HEAD + FRAME_TAIL || &rest[..4] != WAL_MAGIC {
            // an incomplete header, or garbage: torn when nothing valid can follow
            return if rest.len() < FRAME_HEAD + FRAME_TAIL || find_frame(rest) .is_none() {
                Scan::Torn(i as u64)
            } else {
                Scan::Damaged(i as u64, format!("bad frame header at {}", i))
            };
        }
        let len = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let seq = u64::from_le_bytes([rest[8], rest[9], rest[10], rest[11], rest[12], rest[13], rest[14], rest[15]]);
        let total = FRAME_HEAD + len + FRAME_TAIL;
        if len > MAX_FRAME || rest.len() < total {
            return if len <= MAX_FRAME { Scan::Torn(i as u64) } else { Scan::Damaged(i as u64, format!("frame length {} at {}", len, i)) };
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
            if seq != *last_seq + 1 && *last_seq != 0 {
                return Scan::Damaged(i as u64, format!("sequence gap {} -> {}", last_seq, seq));
            }
            if !apply_payload(data, payload) {
                return Scan::Damaged(i as u64, format!("unreadable frame {}", seq));
            }
            *replayed += 1;
        }
        *last_seq = (*last_seq).max(seq);
        i += total;
    }
    Scan::Clean
}

/// the offset of a plausible frame start (magic) in the bytes
fn find_frame(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == WAL_MAGIC)
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
        for &s in snaps.iter().rev() {
            if let Some((seq, d)) = plat::read_file(&join(dir, &snap_name(s))).and_then(|b| decode_snapshot(&b)) {
                data = d;
                snap_seq = seq;
                break;
            }
        }
        let mut rec = Recovery { snapshot_seq: snap_seq, ..Recovery::default() };
        let mut last = snap_seq;
        let mut replayed = 0u64;
        let n = wals.len();
        for (k, &w) in wals.iter().enumerate() {
            let path = join(dir, &wal_name(w));
            let bytes = plat::read_file(&path).ok_or_else(|| io("cannot read", &path))?;
            let is_last = k + 1 == n;
            match replay(&bytes, snap_seq, &mut data, &mut last, &mut replayed) {
                Scan::Clean => {}
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
        let st = State { data, seq: last, wal, wal_path, wal_len, snap_seq };
        Ok(Store { dir: String::from(dir), opts, st: Mutex::new(st), _lock: lock, recovery: rec })
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
        let payload = encode_ops(&tx.ops)?;
        if payload.len() > MAX_FRAME {
            return Err(StoreError::TooLarge);
        }
        let seq = st.seq + 1;
        let f = frame(seq, &payload);
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
                    st.data.entry(c).or_default().insert(i, v);
                }
                Op::Del(c, i) => {
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
        let bytes = encode_snapshot(&st.data, st.seq)?;
        let path = join(&self.dir, &snap_name(st.seq));
        if !plat::fs::write_file_private(&path, &bytes) {
            return Err(io("cannot write", &path));
        }
        // the new log starts after the snapshot
        let wal_path = join(&self.dir, &wal_name(st.seq + 1));
        let wal = plat::File::open(&wal_path, plat::FO_RW).ok_or_else(|| io("cannot open", &wal_path))?;
        plat::fs::sync_dir(&self.dir);
        let old_wal = core::mem::replace(&mut st.wal_path, wal_path);
        st.wal = wal;
        st.wal_len = 0;
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
        let _ = old_wal;
        Ok(())
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
