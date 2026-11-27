// store -- lib/datastore: records survive reopening, snapshots and compaction, a log cut at
// every byte of its last frame opens with the complete transactions, damage in the middle is
// refused (or cut with repair), journals append and read back.  Indexes: fields, arrays, keys
// of a function, unique ones refusing what breaks them, ranges, and a random model against
// filter.  Encryption: nothing in the clear on disk, the wrong key and the other kind refused,
// torn, changed and planted frames, rekey from the clear, to another key and back; sealed
// journals and their rekey

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::Value;
use datastore::{Index, Journal, Key, Options, Order, Store, StoreError};

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
    index(t);
    index_model(t);
    sealed(t);
    rekey(t);
    journal_sealed(t);
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
        // again, nothing written between: the log it starts is the one open (Windows refused
        // to open it a second time)
        t.check("compact twice", s.compact().is_ok());
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

    // a last frame whose payload holds the magic: torn at every byte all the same (a bare "BCW1"
    // behind the cut made it a damaged frame with more data behind it)
    let dir = tmp("store-torn-magic");
    {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        for i in 0..4 {
            let _ = s.put("c", &format!("{}", i), rec(i));
        }
        let mut v = rec(4);
        v.insert("note", Value::string("BCW1 BCW1BCW1 xBCW1"));
        let _ = s.put("c", "4", v);
    }
    let wal = wal_of(&dir);
    let full = plat::read_file(&wal).unwrap_or_default();
    // the last frame's start by the lengths: magic, length (LE), seq, payload, 16 bytes digest
    let (mut i, mut last) = (0usize, 0usize);
    while i + 16 <= full.len() {
        last = i;
        i += 32 + u32::from_le_bytes([full[i + 4], full[i + 5], full[i + 6], full[i + 7]]) as usize;
    }
    let mut ok = i == full.len() && full[last..].windows(4).filter(|w| *w == b"BCW1").count() >= 5;
    for cut in last..full.len() {
        let _ = plat::write_file(&wal, &full[..cut]);
        match Store::open_with(&dir, opts()) {
            Ok(s) => {
                if s.count("c") != 4 || s.recovery().truncated_bytes != (cut - last) as u64 {
                    ok = false;
                }
            }
            Err(_) => ok = false,
        }
    }
    t.check("torn last frame with the magic in its payload", ok);
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

// ------------------------------------------------------------------ indexes

fn user(name: &str, email: &str, owner: u64, tags: &[&str]) -> Value {
    Value::object([
        ("name", Value::string(name)),
        ("email", Value::string(email)),
        ("owner", Value::object([("id", Value::u64(owner))])),
        ("tags", Value::Array(tags.iter().map(|t| Value::string(*t)).collect())),
    ])
}

fn ids_of(v: &[(String, Value)]) -> Vec<String> {
    v.iter().map(|(i, _)| i.clone()).collect()
}

fn index(t: &mut T) {
    let dir = tmp("store-index");
    let Ok(s) = Store::open_with(&dir, opts()) else {
        t.check("index: open", false);
        return;
    };
    let _ = s.put("users", "ada", user("Ada", "ada@x", 1, &["admin", "ops"]));
    let _ = s.put("users", "bob", user("Bob", "bob@x", 2, &["ops", "ops"]));
    let _ = s.put("users", "cy", user("Cy", "cy@x", 1, &[]));
    let _ = s.put("users", "dot", Value::object([("name", Value::string("Dot"))])); // no email, owner, tags
    t.check("index: add", s.add_index("users", "email", Index::field("email").unique()).is_ok() && s.add_index("users", "owner", Index::field("owner.id")).is_ok() && s.add_index("users", "tags", Index::field("tags")).is_ok());
    t.eq("index: names", s.index_names("users"), alloc::vec![String::from("email"), String::from("owner"), String::from("tags")]);
    t.check("index: find_one by a unique key", s.find_one("users", "email", &Key::from("bob@x")).is_ok_and(|r| r.is_some_and(|(id, _)| id == "bob")));
    t.check("index: find on a nested field", s.find("users", "owner", &Key::from(1u64)).is_ok_and(|r| ids_of(&r) == ["ada", "cy"]));
    t.check("index: an array gives a key for each element", s.find("users", "tags", &Key::from("ops")).is_ok_and(|r| ids_of(&r) == ["ada", "bob"]));
    t.check("index: a record without the field is in no index", s.count_key("users", "email", &Key::from("")).is_ok_and(|n| n == 0) && s.find("users", "owner", &Key::from(0u64)).is_ok_and(|r| r.is_empty()));
    t.check("index: unknown index", matches!(s.find("users", "nope", &Key::from("x")), Err(StoreError::NoIndex(_))) && matches!(s.find("nobody", "email", &Key::from("x")), Err(StoreError::NoIndex(_))));
    // a unique key taken: refused before anything is written
    let seq = s.seq();
    t.check("index: a second record with a unique key is refused", matches!(s.put("users", "eve", user("Eve", "ada@x", 3, &[])), Err(StoreError::Conflict(_))) && s.seq() == seq && s.get("users", "eve").is_none());
    let both = s.write(|_, tx| {
        tx.put("users", "f1", user("F1", "same@x", 4, &[]));
        tx.put("users", "f2", user("F2", "same@x", 4, &[]));
        Ok::<(), StoreError>(())
    });
    t.check("index: two records of one transaction with one key are refused", matches!(both, Err(StoreError::Conflict(_))) && s.get("users", "f1").is_none());
    // a swap of two keys in one transaction is no conflict; a record keeps its own key
    let swap = s.write(|_, tx| {
        tx.put("users", "ada", user("Ada", "bob@x", 1, &["admin", "ops"]));
        tx.put("users", "bob", user("Bob", "ada@x", 2, &["ops"]));
        Ok::<(), StoreError>(())
    });
    t.check("index: a swap of unique keys in one transaction", swap.is_ok() && s.find_one("users", "email", &Key::from("ada@x")).is_ok_and(|r| r.is_some_and(|(id, _)| id == "bob")));
    t.check("index: a record keeps its own unique key", s.update("users", "cy", |v| { v.insert("name", Value::string("Cyrus")); }).is_ok_and(|v| v.is_some()));
    let freed = s.write(|_, tx| {
        tx.delete("users", "cy");
        tx.put("users", "cy2", user("Cy", "cy@x", 1, &[]));
        Ok::<(), StoreError>(())
    });
    t.check("index: a key freed by a delete in the same transaction", freed.is_ok());
    // updates and deletes move the keys
    let _ = s.update("users", "ada", |v| {
        v.insert("owner", Value::object([("id", Value::u64(2))]));
    });
    t.check("index: an update moves the key", s.find("users", "owner", &Key::from(1u64)).is_ok_and(|r| ids_of(&r) == ["cy2"]) && s.find("users", "owner", &Key::from(2u64)).is_ok_and(|r| ids_of(&r) == ["ada", "bob"]));
    let _ = s.delete("users", "bob");
    t.check("index: a delete takes the keys out", s.find("users", "tags", &Key::from("ops")).is_ok_and(|r| ids_of(&r) == ["ada"]) && s.find_one("users", "email", &Key::from("ada@x")).is_ok_and(|r| r.is_none()));
    // a unique index over records that share a key is refused, and not added
    let _ = s.put("users", "g", user("G", "g@x", 2, &[]));
    t.check("index: a unique index over shared keys is refused", matches!(s.add_index("users", "owner_u", Index::field("owner.id").unique()), Err(StoreError::Conflict(_))) && !s.index_names("users").contains(&String::from("owner_u")));
    // keys computed from the record; ranges both ways, with a limit
    for i in 0..30u64 {
        let _ = s.put("ops", &format!("op{:02}", i), Value::object([("started", Value::u64(1_000 + i * 10)), ("ok", Value::Bool(i % 3 != 0))]));
    }
    t.check("index: by a function", s.add_index("ops", "started", Index::by(|_, v| v.get("started").and_then(Key::of).into_iter().collect())).is_ok() && s.add_index("ops", "ok", Index::field("ok")).is_ok());
    let r = s.range("ops", "started", Key::from(1_050u64)..Key::from(1_100u64), Order::Asc, usize::MAX);
    t.check("index: a range", r.is_ok_and(|r| ids_of(&r) == ["op05", "op06", "op07", "op08", "op09"]));
    let r = s.range("ops", "started", ..Key::from(1_300u64), Order::Desc, 3);
    t.check("index: the newest three", r.is_ok_and(|r| ids_of(&r) == ["op29", "op28", "op27"]));
    t.check("index: count of a key", s.count_key("ops", "ok", &Key::from(false)).is_ok_and(|n| n == 10));
    // bounds BTreeMap::range would panic on: no records, no panic
    use core::ops::Bound::{Excluded, Included};
    let k = Key::from(1_050u64);
    t.check("index: an empty range with both ends excluded", s.range("ops", "started", (Excluded(k.clone()), Excluded(k.clone())), Order::Asc, usize::MAX).is_ok_and(|r| r.is_empty()));
    t.check("index: a range that ends before it starts", s.range("ops", "started", (Included(Key::from(2_000u64)), Included(Key::from(1_000u64))), Order::Desc, usize::MAX).is_ok_and(|r| r.is_empty()));
    // keys of JSON numbers: integers exactly, fractions none
    let n = |text: &str| agent_core::json::parse(text.as_bytes()).ok().and_then(|v| Key::of(&v));
    t.check("index: Key::of", n("18446744073709551615") == Some(Key::Int(u64::MAX as i128)) && n("-9223372036854775808") == Some(Key::Int(i64::MIN as i128)) && n("1e2") == Some(Key::Int(100)) && n("1.5").is_none() && n("null").is_none() && n("\"x\"") == Some(Key::from("x")));
    t.check("index: the kinds in order", Key::from(true) < Key::from(0i64) && Key::from(i64::MAX) < Key::from(""));
    // indexes are not stored: after an open they are added again
    drop(s);
    let Ok(s) = Store::open_with(&dir, opts()) else { return };
    t.check("index: none after an open", matches!(s.find("users", "email", &Key::from("ada@x")), Err(StoreError::NoIndex(_))));
    t.check("index: added again, the same answers", s.add_index("users", "owner", Index::field("owner.id")).is_ok() && s.find("users", "owner", &Key::from(2u64)).is_ok_and(|r| ids_of(&r) == ["ada", "g"]));
    t.check("index: drop", s.drop_index("users", "owner") && !s.drop_index("users", "owner"));
    drop(s);
    rm(&dir);
}

/// random puts, updates and deletes; after each round every key's find equals filter
fn index_model(t: &mut T) {
    let dir = tmp("store-index-model");
    let Ok(s) = Store::open_with(&dir, opts()) else { return };
    let _ = s.add_index("r", "owner", Index::field("owner"));
    let _ = s.add_index("r", "tags", Index::field("tags"));
    let _ = s.add_index("r", "code", Index::field("code").unique());
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    let (mut ok, mut conflicts, mut rounds) = (true, 0u32, 0u32);
    for round in 0..60 {
        for _ in 0..50 {
            let id = format!("i{:03}", next(200));
            let r = match next(4) {
                0 => s.delete("r", &id).map(|_| ()),
                _ => {
                    let tags: Vec<Value> = (0..next(3)).map(|_| Value::u64(next(6))).collect();
                    let mut v = Value::object([("owner", Value::u64(next(12))), ("tags", Value::Array(tags))]);
                    if next(2) == 0 {
                        v.insert("code", Value::u64(next(400)));
                    }
                    s.put("r", &id, v).map(|_| ())
                }
            };
            match r {
                Err(StoreError::Conflict(_)) => conflicts += 1,
                Err(_) => ok = false,
                Ok(()) => {}
            }
        }
        let check = |field: &str, k: u64| {
            let by_index = s.find("r", field, &Key::from(k)).map(|r| ids_of(&r)).unwrap_or_default();
            let by_scan = ids_of(&s.filter("r", |_, v| match v.get(field) {
                Some(Value::Array(a)) => a.iter().any(|e| e.as_u64() == Some(k)),
                Some(e) => e.as_u64() == Some(k),
                None => false,
            }));
            by_index == by_scan
        };
        ok &= (0..12).all(|k| check("owner", k)) && (0..6).all(|k| check("tags", k)) && (0..400).all(|k| check("code", k) && s.count_key("r", "code", &Key::from(k)).is_ok_and(|n| n <= 1));
        rounds = round + 1;
    }
    t.check(&format!("index: {rounds} rounds of random changes, every find as filter ({conflicts} unique conflicts refused)"), ok && conflicts > 0);
    drop(s);
    rm(&dir);
}

// ------------------------------------------------------------------ encryption at rest

/// whether any file below dir holds `needle`
fn holds(dir: &str, needle: &[u8]) -> bool {
    plat::fs::read_dir(dir).unwrap_or_default().into_iter().any(|e| {
        let p = plat::fs::join(dir, &e.name);
        if e.kind == plat::fs::FileKind::Dir {
            holds(&p, needle)
        } else {
            plat::read_file(&p).is_some_and(|b| b.windows(needle.len()).any(|w| w == needle))
        }
    })
}

fn secret_rec(n: u64) -> Value {
    Value::object([("n", Value::u64(n)), ("secret", Value::string(format!("Geheimnis-{}", n)))])
}

/// a log frame as lib/datastore writes it (magic, length, seq, payload, BLAKE3 digest)
fn raw_frame(magic: &[u8; 4], seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::from(&magic[..]);
    f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    f.extend_from_slice(&seq.to_le_bytes());
    f.extend_from_slice(payload);
    let mut h = crypto::Blake3::new();
    h.update(&f[4..16]);
    h.update(payload);
    f.extend_from_slice(&h.finalize()[..16]);
    f
}

fn sealed(t: &mut T) {
    let key = [7u8; 32];
    let dir = tmp("store-sealed");
    {
        let s = Store::open_encrypted(&dir, opts(), &key);
        t.check("sealed: a new store", s.as_ref().is_ok_and(|s| s.encrypted()));
        let Ok(s) = s else { return };
        for i in 0..20 {
            let _ = s.put("secrets", &format!("s{:02}", i), secret_rec(i));
        }
        let _ = s.delete("secrets", "s03");
    }
    t.check("sealed: a snapshot names the key from the start", plat::fs::read_dir(&dir).unwrap_or_default().iter().any(|e| e.name == "snap-0000000000000000.bcs"));
    t.check("sealed: nothing of the records in the files", !holds(&dir, b"Geheimnis") && !holds(&dir, b"secrets") && !holds(&dir, b"s07"));
    t.check("sealed: in the clear it does not open", matches!(Store::open_with(&dir, opts()), Err(StoreError::Key(_))));
    t.check("sealed: another key does not open it", matches!(Store::open_encrypted(&dir, opts(), &[8u8; 32]), Err(StoreError::Key(_))));
    t.check("sealed: ... not with repair either", matches!(Store::open_encrypted(&dir, Options { repair: true, ..opts() }, &[8u8; 32]), Err(StoreError::Key(_))));
    {
        let s = Store::open_encrypted(&dir, opts(), &key);
        t.check("sealed: its key opens it", s.as_ref().is_ok_and(|s| s.count("secrets") == 19 && s.get("secrets", "s07") == Some(secret_rec(7)) && s.recovery().frames_replayed == 21));
        let Ok(s) = s else { return };
        t.check("sealed: compact", s.compact().is_ok() && s.put("secrets", "late", secret_rec(99)).is_ok());
    }
    t.check("sealed: after a snapshot", Store::open_encrypted(&dir, opts(), &key).is_ok_and(|s| s.count("secrets") == 20 && s.recovery().snapshot_seq == 21 && s.recovery().frames_replayed == 1));
    t.check("sealed: still nothing in the clear", !holds(&dir, b"Geheimnis"));
    // a torn sealed frame at every byte: opens with the transactions before it
    let wal = wal_of(&dir);
    let full = plat::read_file(&wal).unwrap_or_default();
    let mut ok = full.len() > 32 && &full[..4] == b"BCW2";
    for cut in 0..full.len() {
        let _ = plat::write_file(&wal, &full[..cut]);
        ok &= Store::open_encrypted(&dir, opts(), &key).is_ok_and(|s| s.count("secrets") == 19 && s.recovery().truncated_bytes == cut as u64);
    }
    t.check("sealed: a torn last frame at every byte", ok);
    let _ = plat::write_file(&wal, &full);
    // a frame changed and its checksum made right: the AEAD finds it
    let len = u32::from_le_bytes([full[4], full[5], full[6], full[7]]) as usize;
    let mut payload = Vec::from(&full[16..16 + len]);
    payload[30] ^= 1;
    let forged = raw_frame(b"BCW2", 22, &payload);
    let _ = plat::write_file(&wal, &forged);
    t.check("sealed: a changed frame is refused", matches!(Store::open_encrypted(&dir, opts(), &key), Err(StoreError::Corrupt(_))));
    t.check("sealed: ... cut with repair", Store::open_encrypted(&dir, Options { repair: true, ..opts() }, &key).is_ok_and(|s| s.count("secrets") == 19 && s.recovery().repaired.is_some()));
    // a frame in the clear put into the log of an encrypted store is not taken
    let plain = raw_frame(b"BCW1", 22, br#"[["put","secrets","planted",{"n":1}]]"#);
    let _ = plat::write_file(&wal_of(&dir), &plain);
    t.check("sealed: a frame in the clear is refused", matches!(Store::open_encrypted(&dir, Options { repair: true, ..opts() }, &key), Err(StoreError::Key(_))));
    let _ = plat::write_file(&wal_of(&dir), b"");
    // the seq in a snapshot's header is bound to it
    let snap = plat::fs::join(&dir, "snap-0000000000000015.bcs");
    let sb = plat::read_file(&snap).unwrap_or_default();
    let mut moved = sb.clone();
    if moved.len() > 28 {
        moved[20] ^= 1;
    }
    let _ = plat::write_file(&snap, &moved);
    t.check("sealed: a snapshot with another seq in its header is damaged", matches!(Store::open_encrypted(&dir, opts(), &key), Err(StoreError::Corrupt(_))));
    let _ = plat::write_file(&snap, &sb);
    t.check("sealed: and opens again", Store::open_encrypted(&dir, opts(), &key).is_ok_and(|s| s.count("secrets") == 19));
    rm(&dir);
}

fn rekey(t: &mut T) {
    let dir = tmp("store-rekey");
    {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        for i in 0..10 {
            let _ = s.put("secrets", &format!("s{}", i), secret_rec(i));
        }
    }
    let (k1, k2) = ([1u8; 32], [2u8; 32]);
    t.check("rekey: a store in the clear does not open with a key", matches!(Store::open_encrypted(&dir, opts(), &k1), Err(StoreError::Key(_))));
    {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        t.check("rekey: encrypt", s.rekey(Some(&k1)).is_ok() && s.encrypted() && s.put("secrets", "after", secret_rec(50)).is_ok());
    }
    t.check("rekey: no file in the clear left", !holds(&dir, b"Geheimnis"));
    t.check("rekey: the key opens it", Store::open_encrypted(&dir, opts(), &k1).is_ok_and(|s| s.count("secrets") == 11));
    t.check("rekey: in the clear no more", matches!(Store::open_with(&dir, opts()), Err(StoreError::Key(_))));
    {
        let Ok(s) = Store::open_encrypted(&dir, opts(), &k1) else { return };
        t.check("rekey: another key", s.rekey(Some(&k2)).is_ok() && s.put("secrets", "after2", secret_rec(51)).is_ok());
    }
    t.check("rekey: the old key opens nothing", matches!(Store::open_encrypted(&dir, opts(), &k1), Err(StoreError::Key(_))));
    t.check("rekey: the new key opens all", Store::open_encrypted(&dir, opts(), &k2).is_ok_and(|s| s.count("secrets") == 12 && s.get("secrets", "s4") == Some(secret_rec(4))));
    {
        let Ok(s) = Store::open_encrypted(&dir, opts(), &k2) else { return };
        t.check("rekey: decrypt", s.rekey(None).is_ok() && !s.encrypted());
    }
    t.check("rekey: in the clear again", Store::open_with(&dir, opts()).is_ok_and(|s| s.count("secrets") == 12) && holds(&dir, b"Geheimnis"));
    rm(&dir);
    rekey_no_log(t);
}

/// rekey whose new log cannot start (here a folder has its name): the snapshot under the new
/// key was written, but the store kept sealing with the old one, so its next open failed under
/// either key
fn rekey_no_log(t: &mut T) {
    let dir = tmp("store-rekey-nolog");
    let k = [3u8; 32];
    let block = {
        let Ok(s) = Store::open_with(&dir, opts()) else { return };
        for i in 0..3 {
            let _ = s.put("secrets", &format!("s{}", i), secret_rec(i));
        }
        // where the log after the new snapshot would start
        let block = plat::fs::join(&dir, &format!("wal-{:016x}.log", s.seq() + 1));
        let _ = plat::fs::mkdir_private(&block);
        t.check("rekey without a new log: an error", s.rekey(Some(&k)).is_err());
        t.check("rekey without a new log: still writes", s.put("secrets", "after", secret_rec(9)).is_ok());
        block
    };
    plat::delete(&block);
    let again = Store::open_encrypted(&dir, opts(), &k);
    let why = match &again {
        Ok(s) => format!("{} records", s.count("secrets")),
        Err(e) => format!("{e:?}"),
    };
    t.check(&format!("rekey without a new log: the new key opens everything ({why})"), again.is_ok_and(|s| s.count("secrets") == 4 && s.get("secrets", "after") == Some(secret_rec(9))));
    rm(&dir);
}

fn journal_sealed(t: &mut T) {
    let dir = tmp("journal-sealed");
    let key = [5u8; 32];
    {
        let Ok(j) = Journal::open_encrypted(&dir, false, &key) else {
            t.check("journal sealed: open", false);
            return;
        };
        for i in 0..12u64 {
            let _ = j.append("audit", if i < 4 { 202609 } else { 202610 }, &secret_rec(i));
        }
        t.check("journal sealed: read back", j.encrypted() && j.tail("audit", 100).len() == 12 && j.read_month("audit", 202609) == (0..4).map(secret_rec).collect::<Vec<_>>());
    }
    t.check("journal sealed: nothing in the clear", !holds(&dir, b"Geheimnis"));
    t.check("journal sealed: in the clear, no entries", Journal::open(&dir, false).is_ok_and(|j| j.tail("audit", 100).is_empty()));
    t.check("journal sealed: another key, no entries", Journal::open_encrypted(&dir, false, &[6u8; 32]).is_ok_and(|j| j.tail("audit", 100).is_empty()));
    // a line in the clear and a line moved to another month are not taken
    let f9 = plat::fs::join(&plat::fs::join(&dir, "audit"), "202609.jl");
    let f10 = plat::fs::join(&plat::fs::join(&dir, "audit"), "202610.jl");
    let line10 = plat::read_file(&f10).unwrap_or_default().split(|&c| c == b'\n').next().map(Vec::from).unwrap_or_default();
    let mut b = plat::read_file(&f9).unwrap_or_default();
    b.extend_from_slice(b"{\"n\":77,\"secret\":\"planted\"}\n");
    b.extend_from_slice(&line10);
    b.push(b'\n');
    let _ = plat::write_file(&f9, &b);
    t.check("journal sealed: planted and moved lines are skipped", Journal::open_encrypted(&dir, false, &key).is_ok_and(|j| j.read_month("audit", 202609).len() == 4));
    // rekey: to another key, again (nothing to do), to the clear; what no key opens stays
    let k2 = [9u8; 32];
    t.check("journal sealed: rekey", Journal::rekey(&dir, Some(&key), Some(&k2)).is_ok_and(|n| n == 2));
    t.check("journal sealed: rekey again changes nothing", Journal::rekey(&dir, Some(&key), Some(&k2)).is_ok_and(|n| n == 0));
    t.check("journal sealed: the new key reads all", Journal::open_encrypted(&dir, false, &k2).is_ok_and(|j| j.tail("audit", 100).len() == 12) && Journal::open_encrypted(&dir, false, &key).is_ok_and(|j| j.tail("audit", 100).is_empty()));
    t.check("journal sealed: to the clear", Journal::rekey(&dir, Some(&k2), None).is_ok_and(|n| n == 2) && Journal::open(&dir, false).is_ok_and(|j| j.tail("audit", 100).len() == 13));
    t.check("journal sealed: the line that opened under no key stays", plat::read_file(&f9).is_some_and(|b| b.split(|&c| c == b'\n').any(|l| l == &line10[..])));
    t.check("journal sealed: and from the clear", Journal::rekey(&dir, None, Some(&key)).is_ok_and(|n| n == 2) && !holds(&dir, b"Geheimnis") && Journal::open_encrypted(&dir, false, &key).is_ok_and(|j| j.tail("audit", 100).len() == 13));
    rm(&dir);
}
