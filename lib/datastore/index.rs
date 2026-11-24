// index -- secondary indexes of a collection: keys of a record (a field, or a function of the
// record) -> the ids that have them, kept in memory beside the data and updated by every
// commit.  A unique index refuses a transaction that would give two records one key
// (StoreError::Conflict) before anything is written.
//
//   Key            Bool < Int < Str (by kind, then value); Key::of(&Value): strings, booleans
//                  and integral numbers (i64 and u64), nothing else
//   Index::field("owner.id") / Index::by(|id, v| keys) / .unique()
//   Store::add_index / drop_index / find / find_one / range (lib.rs)
//
// Indexes are not stored: an app adds them after every open, and they are built from the
// records then (one pass).  An array at a field's path gives one key per element; missing
// members, null, objects and fractions give none (the record is not in the index).

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use formats::json::Value;

/// a key of an index
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Bool(bool),
    /// an integral number (every i64 and u64)
    Int(i128),
    Str(String),
}

impl Key {
    /// a JSON value as a key: a string, a boolean or an integral number; None for the rest
    /// (null, arrays, objects, fractions and numbers beyond i64 and u64)
    pub fn of(v: &Value) -> Option<Key> {
        match v {
            Value::Bool(b) => Some(Key::Bool(*b)),
            Value::String(s) => Some(Key::Str(s.clone())),
            Value::Number(_) => v.as_i64().map(|n| Key::Int(n as i128)).or_else(|| v.as_u64().map(|n| Key::Int(n as i128))),
            _ => None,
        }
    }
}

impl From<&str> for Key {
    fn from(s: &str) -> Key {
        Key::Str(String::from(s))
    }
}

impl From<String> for Key {
    fn from(s: String) -> Key {
        Key::Str(s)
    }
}

impl From<i64> for Key {
    fn from(n: i64) -> Key {
        Key::Int(n as i128)
    }
}

impl From<u64> for Key {
    fn from(n: u64) -> Key {
        Key::Int(n as i128)
    }
}

impl From<bool> for Key {
    fn from(b: bool) -> Key {
        Key::Bool(b)
    }
}

type KeyFn = Box<dyn Fn(&str, &Value) -> Vec<Key> + Send + Sync>;

/// what an index takes from a record: Index::field or Index::by, maybe unique
pub struct Index {
    keys: KeyFn,
    unique: bool,
}

impl Index {
    /// the value at a path of members, '.' between them ("owner" or "owner.id"); an array
    /// there gives a key for each element that is one (Key::of)
    pub fn field(path: &str) -> Index {
        let parts: Vec<String> = path.split('.').map(String::from).collect();
        Index {
            keys: Box::new(move |_, v| {
                let mut at = v;
                for p in &parts {
                    match at.get(p) {
                        Some(x) => at = x,
                        None => return Vec::new(),
                    }
                }
                match at.as_array() {
                    Some(a) => a.iter().filter_map(Key::of).collect(),
                    None => Key::of(at).into_iter().collect(),
                }
            }),
            unique: false,
        }
    }

    /// keys computed from a record (its id and value): a member name with a '.', several
    /// fields together (as one Key::Str), a fraction scaled to an integer.  The keys of a
    /// record are kept with it, so they are removed exactly when it changes
    pub fn by(f: impl Fn(&str, &Value) -> Vec<Key> + Send + Sync + 'static) -> Index {
        Index { keys: Box::new(f), unique: false }
    }

    /// no two records may share a key (a record may have a key twice)
    pub fn unique(mut self) -> Index {
        self.unique = true;
        self
    }

    pub fn is_unique(&self) -> bool {
        self.unique
    }

    /// the keys of a record, sorted, each once
    pub(crate) fn keys_of(&self, id: &str, v: &Value) -> Vec<Key> {
        let mut k = (self.keys)(id, v);
        k.sort();
        k.dedup();
        k
    }
}

/// an index with its entries
pub(crate) struct Idx {
    pub def: Index,
    /// key -> the ids that have it
    pub map: BTreeMap<Key, BTreeSet<String>>,
    /// id -> its keys (to take them out again)
    pub of: BTreeMap<String, Vec<Key>>,
}

impl Idx {
    /// build from the records of a collection -> Err(conflict) for a unique index
    pub fn build<'a>(def: Index, recs: impl Iterator<Item = (&'a String, &'a Value)>) -> Result<Idx, String> {
        let mut idx = Idx { def, map: BTreeMap::new(), of: BTreeMap::new() };
        for (id, v) in recs {
            let keys = idx.def.keys_of(id, v);
            if idx.def.unique {
                if let Some((k, other)) = keys.iter().find_map(|k| idx.map.get(k).and_then(|s| s.iter().next()).map(|o| (k, o))) {
                    return Err(format!("key {:?} of {} and {}", k, other, id));
                }
            }
            idx.put_keys(id, keys);
        }
        Ok(idx)
    }

    fn put_keys(&mut self, id: &str, keys: Vec<Key>) {
        for k in &keys {
            self.map.entry(k.clone()).or_default().insert(String::from(id));
        }
        if !keys.is_empty() {
            self.of.insert(String::from(id), keys);
        }
    }

    pub fn remove(&mut self, id: &str) {
        for k in self.of.remove(id).unwrap_or_default() {
            if let Some(s) = self.map.get_mut(&k) {
                s.remove(id);
                if s.is_empty() {
                    self.map.remove(&k);
                }
            }
        }
    }

    pub fn put(&mut self, id: &str, v: &Value) {
        self.remove(id);
        let keys = self.def.keys_of(id, v);
        self.put_keys(id, keys);
    }

    /// whether the records `after` (id -> the value a transaction leaves, None: deleted) keep
    /// the index unique -> the first conflict
    pub fn check(&self, after: &BTreeMap<&str, Option<&Value>>) -> Option<String> {
        if !self.def.unique {
            return None;
        }
        let mut claimed: BTreeMap<Key, &str> = BTreeMap::new();
        for (&id, v) in after {
            let Some(v) = v else { continue };
            for k in self.def.keys_of(id, v) {
                // another record of this transaction
                if let Some(other) = claimed.get(&k) {
                    return Some(format!("key {:?} of {} and {}", k, other, id));
                }
                // a record that keeps it (one the transaction changes has its keys here anew)
                if let Some(holder) = self.map.get(&k).and_then(|s| s.iter().find(|h| h.as_str() != id && !after.contains_key(h.as_str()))) {
                    return Some(format!("key {:?} of {} and {}", k, holder, id));
                }
                claimed.insert(k, id);
            }
        }
        None
    }
}

/// the order of Store::range
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// by key, then id
    Asc,
    /// the other way round
    Desc,
}
