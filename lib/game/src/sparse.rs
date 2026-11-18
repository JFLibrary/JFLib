// sparse -- a component column: the values of one type in dense arrays, found by entity through a
// paged sparse index (a sparse set)
//
//   SparseSet<T>      insert / remove / get / get_mut / contains in O(1); iter, iter_mut in the order of
//                     the dense arrays; the version of every entry's last change (changed_since)
//   Mut<'a, T>        what get_mut and iter_mut hand out: reading leaves the entry as it was, writing
//                     through it (DerefMut) stamps the entry with a new version
//
// The order of iteration is the order of the dense arrays: an insert appends, a remove moves the last
// entry into the hole (swap remove).  It follows from the sequence of operations alone, never from
// addresses or hashes, so two columns with the same history iterate alike on every target; sort()
// puts the entries in ascending entity order (a column rebuilt another way, from a snapshot, iterates
// as the original after both are sorted).  The sparse index is paged (1024 entity indices a page,
// allocated when an entity of the page gets a value): a column of a few entities among a million
// costs a few pages, not four bytes per entity.
//
// Versions: every column counts its changes (version()); an insert, a get_mut written through, a
// touch, and the entries iter_mut writes through take a new version.  A consumer (the bridge to
// render3d, a network snapshot) remembers the version it saw last and asks for what changed since
// (changed_since); removals() counts the entries that left, so a consumer knows when to look for
// gone ones.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};

use crate::world::Entity;

/// entity indices per page of the sparse index
const PAGE: usize = 1024;
/// a page's entry without a value
const NONE: u32 = u32::MAX;

/// the values of one component type, by entity (see the module header)
pub struct SparseSet<T> {
    /// entity index -> position in the dense arrays (NONE: none), in pages
    pages: Vec<Option<Box<[u32]>>>,
    dense: Vec<Entity>,
    data: Vec<T>,
    /// the version of each entry's last change
    changed: Vec<u64>,
    version: u64,
    removals: u64,
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        SparseSet::new()
    }
}

/// a value of a column handed out for writing: reads leave it unchanged, a write through it (DerefMut)
/// stamps its entry with the version the access took
pub struct Mut<'a, T> {
    value: &'a mut T,
    stamp: &'a mut u64,
    version: u64,
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        *self.stamp = self.version;
        self.value
    }
}

impl<'a, T> Mut<'a, T> {
    /// the value for writing without stamping it (a change no consumer needs to see, such as a cache
    /// inside the component)
    pub fn untracked(self) -> &'a mut T {
        self.value
    }
}

impl<T> SparseSet<T> {
    /// an empty column
    pub const fn new() -> SparseSet<T> {
        SparseSet { pages: Vec::new(), dense: Vec::new(), data: Vec::new(), changed: Vec::new(), version: 0, removals: 0 }
    }

    /// entities with a value
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// no entity has a value
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// the position of an entity index in the dense arrays (whatever the generation there)
    fn pos_of(&self, index: u32) -> Option<usize> {
        let page = self.pages.get(index as usize / PAGE)?.as_ref()?;
        match page[index as usize % PAGE] {
            NONE => None,
            p => Some(p as usize),
        }
    }

    fn set_pos(&mut self, index: u32, pos: u32) {
        let p = index as usize / PAGE;
        if self.pages.len() <= p {
            self.pages.resize_with(p + 1, || None);
        }
        let page = self.pages[p].get_or_insert_with(|| vec![NONE; PAGE].into_boxed_slice());
        page[index as usize % PAGE] = pos;
    }

    /// the position of e's value in the dense arrays (the order of iteration), None without one.  A stale
    /// id (an older generation of the index) has none
    pub fn position(&self, e: Entity) -> Option<usize> {
        self.pos_of(e.index()).filter(|&p| self.dense[p] == e)
    }

    /// e has a value
    pub fn contains(&self, e: Entity) -> bool {
        self.position(e).is_some()
    }

    /// e's value
    pub fn get(&self, e: Entity) -> Option<&T> {
        self.position(e).map(|p| &self.data[p])
    }

    /// e's value for writing; a write through it takes a new version
    pub fn get_mut(&mut self, e: Entity) -> Option<Mut<'_, T>> {
        let p = self.position(e)?;
        self.version += 1;
        Some(Mut { value: &mut self.data[p], stamp: &mut self.changed[p], version: self.version })
    }

    /// e gets the value (a new version); the value it had, if any.  The column does not know which
    /// entities live (World::insert refuses dead ones): another generation of the same index loses its
    /// value to e, and that value comes back
    pub fn insert(&mut self, e: Entity, value: T) -> Option<T> {
        self.version += 1;
        if let Some(p) = self.pos_of(e.index()) {
            self.dense[p] = e;
            self.changed[p] = self.version;
            return Some(core::mem::replace(&mut self.data[p], value));
        }
        let p = self.dense.len();
        assert!(p < NONE as usize, "SparseSet: more than 2^32 - 1 values");
        self.set_pos(e.index(), p as u32);
        self.dense.push(e);
        self.data.push(value);
        self.changed.push(self.version);
        None
    }

    /// e's value goes (the last entry takes its place in the order)
    pub fn remove(&mut self, e: Entity) -> Option<T> {
        let p = self.position(e)?;
        let last = self.dense.len() - 1;
        if p != last {
            let moved = self.dense[last];
            self.set_pos(moved.index(), p as u32);
        }
        self.set_pos(e.index(), NONE);
        self.dense.swap_remove(p);
        self.changed.swap_remove(p);
        self.removals += 1;
        Some(self.data.swap_remove(p))
    }

    /// stamp e's value with a new version without writing it (true: e has one)
    pub fn touch(&mut self, e: Entity) -> bool {
        match self.position(e) {
            Some(p) => {
                self.version += 1;
                self.changed[p] = self.version;
                true
            }
            None => false,
        }
    }

    /// every value goes
    pub fn clear(&mut self) {
        self.removals += self.dense.len() as u64;
        self.pages.clear();
        self.dense.clear();
        self.data.clear();
        self.changed.clear();
    }

    /// the entities with a value, in the order of iteration
    pub fn entities(&self) -> &[Entity] {
        &self.dense
    }

    /// the values, in the order of iteration
    pub fn values(&self) -> &[T] {
        &self.data
    }

    /// (entity, value) in the order of iteration
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.dense.iter().copied().zip(self.data.iter())
    }

    /// (entity, value for writing) in the order of iteration; the entries written through take one new
    /// version
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, Mut<'_, T>)> + '_ {
        self.version += 1;
        let version = self.version;
        self.dense.iter().copied().zip(self.data.iter_mut().zip(self.changed.iter_mut()))
            .map(move |(e, (value, stamp))| (e, Mut { value, stamp, version }))
    }

    /// the column's version: the newest any of its entries can have
    pub fn version(&self) -> u64 {
        self.version
    }

    /// the version of e's last change (None: no value)
    pub fn changed_at(&self, e: Entity) -> Option<u64> {
        self.position(e).map(|p| self.changed[p])
    }

    /// the versions of the entries' last changes, in the order of iteration
    pub fn versions(&self) -> &[u64] {
        &self.changed
    }

    /// (entity, value) of the entries changed after version v (inserted, written, touched), in the order
    /// of iteration.  The whole column is looked at: a scan of one u64 per entry
    pub fn changed_since(&self, v: u64) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.dense.iter().copied().zip(self.data.iter()).zip(self.changed.iter())
            .filter(move |(_, c)| **c > v).map(|(x, _)| x)
    }

    /// (entity, value for writing) of the entries changed after version v, in the order of iteration; the
    /// ones written through take one new version (newer than every entry the walk skips)
    pub fn changed_mut_since(&mut self, v: u64) -> impl Iterator<Item = (Entity, Mut<'_, T>)> + '_ {
        self.version += 1;
        let version = self.version;
        self.dense.iter().copied().zip(self.data.iter_mut().zip(self.changed.iter_mut()))
            .filter(move |(_, (_, stamp))| **stamp > v)
            .map(move |(e, (value, stamp))| (e, Mut { value, stamp, version }))
    }

    /// how many values have left the column so far (remove, clear, despawned entities): a consumer that
    /// sees the same number as last time has nothing gone to look for
    pub fn removals(&self) -> u64 {
        self.removals
    }

    /// the entries in ascending order of their entity index: the canonical order, the same for every
    /// history that ends with the same entities (versions are kept)
    pub fn sort(&mut self) {
        let n = self.dense.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        // (indices are unique: an unstable sort is deterministic)
        order.sort_unstable_by_key(|&i| self.dense[i as usize].index());
        if order.iter().enumerate().all(|(k, &i)| k == i as usize) {
            return;
        }
        let mut data: Vec<Option<T>> = self.data.drain(..).map(Some).collect();
        let dense = core::mem::take(&mut self.dense);
        let changed = core::mem::take(&mut self.changed);
        for &i in &order {
            let i = i as usize;
            self.dense.push(dense[i]);
            self.changed.push(changed[i]);
            if let Some(v) = data[i].take() {
                self.data.push(v);
            }
        }
        for k in 0..n {
            let index = self.dense[k].index();
            self.set_pos(index, k as u32);
        }
    }

    /// the bytes of the sparse index's pages (a measure for tests)
    pub fn index_bytes(&self) -> usize {
        self.pages.iter().flatten().count() * PAGE * 4 + self.pages.len() * core::mem::size_of::<Option<Box<[u32]>>>()
    }
}
