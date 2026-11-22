// journal -- append-only logs of JSON lines, one file per name and month
// (<dir>/<name>/<yyyymm>.jl): audit entries, session logs.  A line is complete or ignored (a
// crash during an append leaves at most one partial last line, which readers skip).
// Encrypted (open_encrypted): every line sealed on its own ("~" + base64url, seal.rs), bound
// to its journal and month; lines that do not open -- plain ones too -- are skipped.
// Journal::rekey rewrites the files for another key, or none.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use formats::json::{self, Value};
use plat::sync::Mutex;

use crate::seal::Keys;
use crate::{record_limits, StoreError, StoreResult};

pub struct Journal {
    dir: String,
    /// serialises appends (the files are shared by all threads)
    lock: Mutex<()>,
    durable: bool,
    /// an encrypted journal's keys
    keys: Option<Keys>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Journal {
    /// journals below `dir`; `durable`: fsync each append
    pub fn open(dir: &str, durable: bool) -> StoreResult<Journal> {
        Journal::open_keyed(dir, durable, None)
    }

    /// journals below `dir` whose lines are sealed with a 32-byte key (the key of a Store may
    /// be used: the journal derives keys of its own)
    pub fn open_encrypted(dir: &str, durable: bool, key: &[u8; 32]) -> StoreResult<Journal> {
        Journal::open_keyed(dir, durable, Some(Keys::derive(key)))
    }

    fn open_keyed(dir: &str, durable: bool, keys: Option<Keys>) -> StoreResult<Journal> {
        if !plat::fs::mkdir_private(dir) {
            return Err(StoreError::Io(format!("cannot create {}", dir)));
        }
        Ok(Journal { dir: String::from(dir), lock: Mutex::new(()), durable, keys })
    }

    /// whether the lines are sealed
    pub fn encrypted(&self) -> bool {
        self.keys.is_some()
    }

    fn file(&self, name: &str, month: u32) -> String {
        plat::fs::join(&plat::fs::join(&self.dir, name), &format!("{:06}.jl", month))
    }

    /// append one entry to journal `name` in month `yyyymm` (e.g. 202610)
    pub fn append(&self, name: &str, month: u32, v: &Value) -> StoreResult<()> {
        if !valid_name(name) {
            return Err(StoreError::Io(format!("bad journal name {}", name)));
        }
        let text = json::stringify_with_limits(v, record_limits()).map_err(|_| StoreError::TooLarge)?;
        let mut line = match &self.keys {
            Some(k) => k.seal_line(name, month, text.as_bytes()).into_bytes(),
            None => text.into_bytes(),
        };
        line.push(b'\n');
        let _g = self.lock.lock();
        let d = plat::fs::join(&self.dir, name);
        let new_dir = plat::fs::read_dir(&d).is_none();
        if !plat::fs::mkdir_private(&d) {
            return Err(StoreError::Io(format!("cannot create {}", d)));
        }
        let path = self.file(name, month);
        let new_file = plat::fs::read_dir(&d).map_or(true, |l| !l.iter().any(|e| e.name == format!("{:06}.jl", month)));
        let f = plat::File::open(&path, plat::FO_RW).ok_or_else(|| StoreError::Io(format!("cannot open {}", path)))?;
        let mut end = f.size().unwrap_or(0);
        // a partial last line from a crash: start the new entry on a fresh line
        if end > 0 {
            let mut last = [0u8; 1];
            if f.read_at(end - 1, &mut last) == Some(1) && last[0] != b'\n' {
                if f.write_at(end, b"\n") != Some(1) {
                    return Err(StoreError::Io(format!("cannot write {}", path)));
                }
                end += 1;
            }
        }
        if f.write_at(end, &line) != Some(line.len()) {
            f.set_len(end);
            return Err(StoreError::Io(format!("cannot write {}", path)));
        }
        if self.durable && !f.sync() {
            return Err(StoreError::Io(format!("cannot sync {}", path)));
        }
        if self.durable && new_file {
            plat::fs::sync_dir(&d);
            if new_dir {
                plat::fs::sync_dir(&self.dir);
            }
        }
        Ok(())
    }

    /// the months that have entries, oldest first
    pub fn months(&self, name: &str) -> Vec<u32> {
        if !valid_name(name) {
            return Vec::new();
        }
        let mut m: Vec<u32> = plat::fs::read_dir(&plat::fs::join(&self.dir, name))
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.name.strip_suffix(".jl")?.parse().ok())
            .collect();
        m.sort_unstable();
        m
    }

    /// the entries of one month, oldest first
    pub fn read_month(&self, name: &str, month: u32) -> Vec<Value> {
        if !valid_name(name) {
            return Vec::new();
        }
        let Some(b) = plat::read_file(&self.file(name, month)) else { return Vec::new() };
        b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).filter_map(|l| entry(self.keys.as_ref(), name, month, l)).collect()
    }

    /// the newest `n` entries (oldest first), across months
    pub fn tail(&self, name: &str, n: usize) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for m in self.months(name).into_iter().rev() {
            let mut v = self.read_month(name, m);
            let take = n.saturating_sub(out.len());
            if v.len() > take {
                v.drain(..v.len() - take);
            }
            v.append(&mut out);
            out = v;
            if out.len() >= n {
                break;
            }
        }
        out
    }

    /// remove the months before `month` (retention)
    pub fn prune_before(&self, name: &str, month: u32) -> usize {
        let mut n = 0;
        for m in self.months(name) {
            if m < month && plat::delete(&self.file(name, m)) {
                n += 1;
            }
        }
        n
    }

    /// remove a whole journal (e.g. the log of a deleted session)
    pub fn remove(&self, name: &str) {
        if !valid_name(name) {
            return;
        }
        for m in self.months(name) {
            plat::delete(&self.file(name, m));
        }
        plat::delete(&plat::fs::join(&self.dir, name));
    }

    /// rewrite the journals below `dir` for another key: `from` (None: in the clear) is the
    /// key they have, `to` the one they get (None: in the clear) -> the files rewritten.  Each
    /// file is replaced whole (written aside, synced, renamed).  Interrupted, it is run again
    /// with the same keys: lines that are under `to` already stay.  Lines that open under
    /// neither (damaged, a third key) stay as they are, unreadable as before.  No Journal may
    /// be open on `dir` meanwhile (an append could be lost)
    pub fn rekey(dir: &str, from: Option<&[u8; 32]>, to: Option<&[u8; 32]>) -> StoreResult<usize> {
        let (from, to) = (from.map(Keys::derive), to.map(Keys::derive));
        let mut n = 0;
        for d in plat::fs::read_dir(dir).unwrap_or_default() {
            if d.kind != plat::fs::FileKind::Dir || !valid_name(&d.name) {
                continue;
            }
            let sub = plat::fs::join(dir, &d.name);
            for f in plat::fs::read_dir(&sub).unwrap_or_default() {
                let Some(month) = f.name.strip_suffix(".jl").and_then(|m| m.parse::<u32>().ok()) else { continue };
                let path = plat::fs::join(&sub, &f.name);
                let b = plat::read_file(&path).ok_or_else(|| StoreError::Io(format!("cannot read {}", path)))?;
                let mut out = Vec::with_capacity(b.len());
                let mut changed = false;
                for l in b.split(|&c| c == b'\n').filter(|l| !l.is_empty()) {
                    let line = match (entry(to.as_ref(), &d.name, month, l), entry(from.as_ref(), &d.name, month, l)) {
                        (Some(_), _) => None,
                        (None, Some(v)) => {
                            let text = json::stringify_with_limits(&v, record_limits()).map_err(|_| StoreError::TooLarge)?;
                            Some(match &to {
                                Some(k) => k.seal_line(&d.name, month, text.as_bytes()).into_bytes(),
                                None => text.into_bytes(),
                            })
                        }
                        (None, None) => None,
                    };
                    changed |= line.is_some();
                    out.extend_from_slice(line.as_deref().unwrap_or(l));
                    out.push(b'\n');
                }
                if changed {
                    if !plat::fs::write_file_private(&path, &out) {
                        return Err(StoreError::Io(format!("cannot write {}", path)));
                    }
                    n += 1;
                }
            }
        }
        Ok(n)
    }
}

/// one line of a journal: JSON, or sealed with `keys` -- the other kind is no entry
fn entry(keys: Option<&Keys>, name: &str, month: u32, line: &[u8]) -> Option<Value> {
    match keys {
        Some(k) => json::parse_with_limits(&k.open_line(name, month, line)?, record_limits()).ok(),
        None => json::parse_with_limits(line, record_limits()).ok(),
    }
}
