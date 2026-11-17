// keymap -- a map from cell keys to values for one calculation, in insertion order
//
// The engine looks cells up several times per formula; a B-tree over the nested key costs
// most of the time there.  This is an open-addressing hash table of indices into a vector of
// entries.  The keys come from files (row indices, section indices), so a probe sequence is
// bounded: a key that does not find a place within MAX_PROBE slots goes to an ordered
// overflow map, and no choice of keys makes a lookup slower than the B-tree.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::engine::NodeKey;
use crate::document::{Owner, SheetRef};
use crate::sheet::RowKey;

const MAX_PROBE: usize = 32;
const EMPTY: u32 = u32::MAX;

pub(crate) struct KeyMap<V> {
    entries: Vec<(NodeKey, V)>,
    table: Vec<u32>,
    overflow: BTreeMap<NodeKey, u32>,
}

fn mix(mut h: u64, x: u64) -> u64 {
    h ^= x.wrapping_add(0x9e37_79b9_7f4a_7c15).wrapping_add(h << 6).wrapping_add(h >> 2);
    h
}

fn hash(k: &NodeKey) -> u64 {
    let (kind, a, b) = match k.sheet {
        SheetRef::Document => (0u64, 0u64, 0u64),
        SheetRef::Style(s) => (1, s.0 as u64, 0),
        SheetRef::Page(p) => (2, p.0 as u64, 0),
        SheetRef::Master(m) => (3, m.0 as u64, 0),
        SheetRef::Shape(r) => match r.owner {
            Owner::Page(p) => (4, p.0 as u64, r.slot as u64),
            Owner::Master(m) => (5, m.0 as u64, r.slot as u64),
        },
    };
    let mut h = mix(kind, a);
    h = mix(h, b);
    if let Some(s) = k.sec {
        h = mix(h, ((s.name.0 as u64) << 32) | s.index as u64);
    }
    match k.row {
        Some(RowKey::Index(i)) => h = mix(h, (1 << 40) | i as u64),
        Some(RowKey::Named(n)) => h = mix(h, (2 << 40) | n.0 as u64),
        None => {}
    }
    h = mix(h, k.cell.0 as u64);
    // a final avalanche (splitmix64)
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

impl<V> KeyMap<V> {
    pub(crate) fn new() -> Self {
        KeyMap { entries: Vec::new(), table: alloc::vec![EMPTY; 64], overflow: BTreeMap::new() }
    }

    fn find(&self, k: &NodeKey) -> Option<u32> {
        let mask = self.table.len() - 1;
        let mut i = hash(k) as usize & mask;
        for _ in 0..MAX_PROBE {
            let e = self.table[i];
            if e == EMPTY {
                return self.overflow.get(k).copied();
            }
            if self.entries[e as usize].0 == *k {
                return Some(e);
            }
            i = (i + 1) & mask;
        }
        self.overflow.get(k).copied()
    }

    pub(crate) fn get(&self, k: &NodeKey) -> Option<&V> {
        self.find(k).map(|i| &self.entries[i as usize].1)
    }

    pub(crate) fn contains_key(&self, k: &NodeKey) -> bool {
        self.find(k).is_some()
    }

    fn place(&mut self, k: &NodeKey, idx: u32) {
        let mask = self.table.len() - 1;
        let mut i = hash(k) as usize & mask;
        for _ in 0..MAX_PROBE {
            if self.table[i] == EMPTY {
                self.table[i] = idx;
                return;
            }
            i = (i + 1) & mask;
        }
        self.overflow.insert(*k, idx);
    }

    fn grow(&mut self) {
        let n = self.table.len() * 2;
        self.table = alloc::vec![EMPTY; n];
        self.overflow.clear();
        for i in 0..self.entries.len() {
            let k = self.entries[i].0;
            self.place(&k, i as u32);
        }
    }

    pub(crate) fn insert(&mut self, k: NodeKey, v: V) {
        if let Some(i) = self.find(&k) {
            self.entries[i as usize].1 = v;
            return;
        }
        if (self.entries.len() + 1) * 2 > self.table.len() {
            self.grow();
        }
        let idx = self.entries.len() as u32;
        self.entries.push((k, v));
        self.place(&k, idx);
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&NodeKey, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}
