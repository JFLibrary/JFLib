// store -- lib/datastore: records survive reopening, snapshots and compaction, a log cut at
// every byte of its last frame opens with the complete transactions, damage in the middle is
// refused (or cut with repair), journals append and read back

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::Value;
use datastore::{Journal, Options, Store, StoreError};

use crate::T;

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

fn rec(n: u64) -> Value {
    Value::object([("n", Value::u64(n)), ("name", Value::string(format!("job {}", n)))])
}

fn opts() -> Options {
    Options { durable: false, ..Options::default() }
}

/// the newest log file of a store directory
fn wal_of(dir: &str) -> String {
    let mut w: Vec<String> = plat::fs::read_dir(dir).unwrap_or_default().into_iter().map(|e| e.name).filter(|n| n.starts_with("wal-")).collect();
    w.sort();
    plat::fs::join(dir, w.last().map_or("", |s| s.as_str()))
}

pub fn run(t: &mut T) {
    basics(t);
    torn(t);
    damage(t);
    journal(t);
}

fn basics(t: &mut T) {
    let dir = tmp("store");
    {
        let s = Store::open_with(&dir, opts());
        t.check("open new", s.is_ok());
        let Ok(s) = s else { return };
        t.check("second open is locked", matches!(Store::open_with(&dir, opts()), Err(StoreError::Locked)));
        for i in 0..50 {
            let _ = s.put("jobs", &format!("j{:03}", i), rec(i));
        }
        let _ = s.delete("jobs", "j007");
        let r = s.write(|data, tx| {
            let n = data.get("jobs").map_or(0, |c| c.len());
            tx.put("meta", "count", Value::u64(n as u64));
            tx.put("jobs", "j100", rec(100));
            Ok::<(), StoreError>(())
        });
        t.check("transaction", r.is_ok());
        let u = s.update("jobs", "j001", |v| {
            v.insert("name", Value::string("renamed"));
        });
        t.check("update", u.is_ok_and(|v| v.is_some()));
        t.eq("count", s.count("jobs"), 50);
        t.eq("seq", s.seq(), 53);
    }
    let s = Store::open_with(&dir, opts());
    t.check("reopen", s.is_ok());
    if let Ok(s) = s {
        t.eq("reopen count", s.count("jobs"), 50);
        t.check("deleted stays deleted", s.get("jobs", "j007").is_none());
        t.eq("transaction data", s.get("meta", "count").and_then(|v| v.as_u64()), Some(49));
        t.eq("update data", s.get("jobs", "j001").and_then(|v| v.get("name").and_then(|x| x.as_str()).map(String::from)), Some(String::from("renamed")));
        t.eq("replayed frames", s.recovery().frames_replayed, 53);
        t.check("compact", s.compact().is_ok());
        let _ = s.put("jobs", "after", rec(1));
    }
    let s = Store::open_with(&dir, opts());
    if let Ok(s) = s {
        t.eq("after snapshot count", s.count("jobs"), 51);
        t.eq("snapshot seq", s.recovery().snapshot_seq, 53);
        t.eq("replayed after snapshot", s.recovery().frames_replayed, 1);
        let files = plat::fs::read_dir(&dir).unwrap_or_default().len();
        t.check("old logs removed", files <= 4); // LOCK, snapshot, one log (+ an old snapshot)
    } else {
        t.check("open after snapshot", false);
    }
    // automatic snapshots on a small threshold
    let dir2 = tmp("store-auto");
    if let Ok(s) = Store::open_with(&dir2, Options { snapshot_bytes: 4096, durable: false, repair: false }) {
        for i in 0..400 {
            let _ = s.put("x", &format!("{}", i), rec(i));
        }
        drop(s);
        let s = Store::open_with(&dir2, opts());
        t.check("auto snapshots", s.as_ref().is_ok_and(|s| s.count("x") == 400 && s.recovery().snapshot_seq > 0));
    }
    rm(&dir);
    rm(&dir2);
}

fn torn(t: &mut T) {
    let dir = tmp("store-torn");
    {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        for i in 0..5 {
            let _ = s.put("c", &format!("{}", i), rec(i));
        }
    }
    let wal = wal_of(&dir);
    let full = plat::read_file(&wal).unwrap_or_default();
    // the start of the last frame: the fifth "BCW1"
    let starts: Vec<usize> = full.windows(4).enumerate().filter(|(_, w)| *w == b"BCW1").map(|(i, _)| i).collect();
    let last = starts.last().copied().unwrap_or(0);
    let mut ok = true;
    for cut in last..full.len() {
        let _ = plat::write_file(&wal, &full[..cut]);
        match Store::open_with(&dir, opts()) {
            Ok(s) => {
                if s.count("c") != 4 || s.recovery().truncated_bytes != (cut - last) as u64 {
                    ok = false;
                }
                // writing goes on after the cut
                if s.put("c", "new", rec(9)).is_err() {
                    ok = false;
                }
            }
            Err(_) => ok = false,
        }
    }
    t.check("torn last frame at every byte", ok);
    let _ = plat::write_file(&wal, &full);
    t.check("intact log", Store::open_with(&dir, opts()).is_ok_and(|s| s.count("c") == 5));
    rm(&dir);
}

fn damage(t: &mut T) {
    let dir = tmp("store-damage");
    {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        for i in 0..6 {
            let _ = s.put("c", &format!("{}", i), rec(i));
        }
    }
    let wal = wal_of(&dir);
    let mut b = plat::read_file(&wal).unwrap_or_default();
    let starts: Vec<usize> = b.windows(4).enumerate().filter(|(_, w)| *w == b"BCW1").map(|(i, _)| i).collect();
    // flip a byte inside the payload of the third frame
    let at = starts[2] + 20;
    b[at] ^= 0x20;
    let _ = plat::write_file(&wal, &b);
    t.check("damage refused", matches!(Store::open_with(&dir, opts()), Err(StoreError::Corrupt(_))));
    let r = Store::open_with(&dir, Options { repair: true, durable: false, snapshot_bytes: 16 << 20 });
    t.check("repair", r.as_ref().is_ok_and(|s| s.count("c") == 2 && s.recovery().repaired.is_some()));
    drop(r);
    t.check("repaired store opens", Store::open_with(&dir, opts()).is_ok_and(|s| s.count("c") == 2));
    rm(&dir);
}

fn journal(t: &mut T) {
    let dir = tmp("journal");
    let Ok(j) = Journal::open(&dir, false) else {
        t.check("journal open", false);
        return;
    };
    for i in 0..30u64 {
        let month = if i < 10 { 202609 } else { 202610 };
        let _ = j.append("audit", month, &rec(i));
    }
    t.eq("journal months", j.months("audit"), alloc::vec![202609, 202610]);
    let tail = j.tail("audit", 25);
    t.eq("journal tail length", tail.len(), 25);
    t.eq("journal tail order", tail.first().and_then(|v| v.get("n")).and_then(|v| v.as_u64()), Some(5));
    // a partial last line (crash during an append) is skipped, the next append starts fresh
    let f = plat::fs::join(&plat::fs::join(&dir, "audit"), "202610.jl");
    let mut b = plat::read_file(&f).unwrap_or_default();
    b.extend_from_slice(b"{\"n\": 99, \"na");
    let _ = plat::write_file(&f, &b);
    let _ = j.append("audit", 202610, &rec(31));
    let m = j.read_month("audit", 202610);
    t.eq("journal partial line skipped", m.len(), 21);
    t.eq("journal prune", j.prune_before("audit", 202610), 1);
    t.check("journal bad name", j.append("../x", 202610, &rec(0)).is_err());
    rm(&dir);
}
