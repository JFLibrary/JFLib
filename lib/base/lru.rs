// lru -- a map that forgets the least recently used entries: a limit on the number of entries
// and one on their weight (bytes), ordered maps only (deterministic, no hashing); caches of
// simulators and servers
//
//   Lru::new(max_len, max_weight)          0: no limit
//   get(k) (counts as a use) / peek(k) / contains(k) / get_mut(k)
//   insert(k, v, weight) -> evicted        the entries that had to go, oldest first (the new
//                                          one itself when it alone is too heavy)
//   remove(k) / pop_lru() / clear() / len() / weight() / keys() (oldest first)
//
// Every use moves an entry to the young end; eviction takes from the old end until both
// limits hold.  Costs are O(log n) per call.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot<V> {
    v: V,
    tick: u64,
    weight: u64,
}

/// a least-recently-used map
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lru<K: Ord + Clone, V> {
    map: BTreeMap<K, Slot<V>>,
    /// use tick -> key, the oldest first
    order: BTreeMap<u64, K>,
    tick: u64,
    weight: u64,
    /// entries at most (0: no limit)
    pub max_len: usize,
    /// summed weight at most (0: no limit)
    pub max_weight: u64,
}

impl<K: Ord + Clone, V> Lru<K, V> {
    pub fn new(max_len: usize, max_weight: u64) -> Lru<K, V> {
        Lru { map: BTreeMap::new(), order: BTreeMap::new(), tick: 0, weight: 0, max_len, max_weight }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// the summed weight of the entries
    pub fn weight(&self) -> u64 {
        self.weight
    }
    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }
    /// an entry without counting it as a use
    pub fn peek(&self, k: &K) -> Option<&V> {
        self.map.get(k).map(|s| &s.v)
    }
    fn touch(&mut self, k: &K) {
        self.tick += 1;
        let t = self.tick;
        if let Some(s) = self.map.get_mut(k) {
            self.order.remove(&s.tick);
            s.tick = t;
            self.order.insert(t, k.clone());
        }
    }
    /// an entry; it becomes the most recently used
    pub fn get(&mut self, k: &K) -> Option<&V> {
        if !self.map.contains_key(k) {
            return None;
        }
        self.touch(k);
        self.map.get(k).map(|s| &s.v)
    }
    /// an entry to change in place; it becomes the most recently used
    pub fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        if !self.map.contains_key(k) {
            return None;
        }
        self.touch(k);
        self.map.get_mut(k).map(|s| &mut s.v)
    }
    /// stores an entry (replacing one with the same key) as the most recently used; the
    /// entries that had to go for the limits, oldest first
    pub fn insert(&mut self, k: K, v: V, weight: u64) -> Vec<(K, V)> {
        let _ = self.remove(&k);
        self.tick += 1;
        let t = self.tick;
        self.order.insert(t, k.clone());
        self.map.insert(k, Slot { v, tick: t, weight });
        self.weight += weight;
        let mut out = Vec::new();
        while (self.max_len > 0 && self.map.len() > self.max_len) || (self.max_weight > 0 && self.weight > self.max_weight) {
            match self.pop_lru() {
                Some(e) => out.push(e),
                None => break,
            }
        }
        out
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let s = self.map.remove(k)?;
        self.order.remove(&s.tick);
        self.weight -= s.weight;
        Some(s.v)
    }
    /// takes the least recently used entry
    pub fn pop_lru(&mut self) -> Option<(K, V)> {
        let (&t, _) = self.order.iter().next()?;
        let k = self.order.remove(&t)?;
        let s = self.map.remove(&k)?;
        self.weight -= s.weight;
        Some((k, s.v))
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.weight = 0;
    }
    /// the keys, least recently used first
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.order.values()
    }
    /// the entries in key order (no use counted)
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.map.iter().map(|(k, s)| (k, &s.v))
    }
}
