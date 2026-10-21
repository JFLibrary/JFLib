// journal -- append-only logs of JSON lines, one file per name and month
// (<dir>/<name>/<yyyymm>.jl): audit entries, session logs.  A line is complete or ignored (a
// crash during an append leaves at most one partial last line, which readers skip).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Value};
use plat::sync::Mutex;

use crate::{record_limits, StoreError, StoreResult};

pub struct Journal {
    dir: String,
    /// serialises appends (the files are shared by all threads)
    lock: Mutex<()>,
    durable: bool,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Journal {
    /// journals below `dir`; `durable`: fsync each append
    pub fn open(dir: &str, durable: bool) -> StoreResult<Journal> {
        if !plat::fs::mkdir_private(dir) {
            return Err(StoreError::Io(format!("cannot create {}", dir)));
        }
        Ok(Journal { dir: String::from(dir), lock: Mutex::new(()), durable })
    }

    fn file(&self, name: &str, month: u32) -> String {
        plat::fs::join(&plat::fs::join(&self.dir, name), &format!("{:06}.jl", month))
    }

    /// append one entry to journal `name` in month `yyyymm` (e.g. 202610)
    pub fn append(&self, name: &str, month: u32, v: &Value) -> StoreResult<()> {
        if !valid_name(name) {
            return Err(StoreError::Io(format!("bad journal name {}", name)));
        }
        let mut line = json::stringify_with_limits(v, record_limits()).map_err(|_| StoreError::TooLarge)?.into_bytes();
        line.push(b'\n');
        let _g = self.lock.lock();
        let d = plat::fs::join(&self.dir, name);
        if !plat::fs::mkdir_private(&d) {
            return Err(StoreError::Io(format!("cannot create {}", d)));
        }
        let path = self.file(name, month);
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
        Ok(())
    }

    /// the months that have entries, oldest first
    pub fn months(&self, name: &str) -> Vec<u32> {
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
        let Some(b) = plat::read_file(&self.file(name, month)) else { return Vec::new() };
        b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).filter_map(|l| json::parse_with_limits(l, record_limits()).ok()).collect()
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
        for m in self.months(name) {
            plat::delete(&self.file(name, m));
        }
        plat::delete(&plat::fs::join(&self.dir, name));
    }
}
