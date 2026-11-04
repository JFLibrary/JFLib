// heap -- the values of the script VM and the arena they live in
//
//   Value       16 bytes: nil, bool, int, float, or a typed reference into the heap (str,
//               bytes, list, map, fn) or to a native function by name
//   Heap        a Vec of objects addressed by index; free slots form a list; mark bits and the
//               sweep for the VM's mark-and-sweep collector (the VM marks from its roots)
//   Map         insertion ordered hash map (keys str, int, bool): entries in order plus an
//               open-addressing index once it has more than 8 entries; removing leaves a hole
//               that the next compaction closes
//   sizes       every object has an accounted size computed from its contents (lengths, never
//               capacities), so the heap limit and the collector's timing are the same on
//               every target
//
// Hashes are fixed functions (FNV-1a for text): iteration order is insertion order anyway.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

/// a reference to an object of the VM's heap (only meaningful for the VM that made it)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Ref(pub(crate) u32);

/// a value of a script; strings, bytes, lists, maps and functions live on the VM's heap.  A
/// value that refers to the heap stays valid while the script can still reach it (or it is
/// pinned): keep values the host holds across runs with Vm::pin.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Ref),
    Bytes(Ref),
    List(Ref),
    Map(Ref),
    /// a function of the script (a closure)
    Func(Ref),
    /// a function of the host, by the index of its name in the VM's name table
    Native(u32),
    /// a captured variable (inside the VM only; scripts and hosts never see one)
    #[doc(hidden)]
    Cell(Ref),
}

impl Value {
    /// the type name scripts see: nil bool int float str bytes list map fn
    pub fn type_name(self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Func(_) | Value::Native(_) => "fn",
            Value::Cell(_) => "cell",
        }
    }

    /// false for nil and false, true for everything else (also 0, "" and [])
    pub fn truthy(self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub(crate) fn heap_ref(self) -> Option<Ref> {
        match self {
            Value::Str(r) | Value::Bytes(r) | Value::List(r) | Value::Map(r) | Value::Func(r) | Value::Cell(r) => Some(r),
            _ => None,
        }
    }
}

/// a string: the text, its FNV-1a hash and its length in characters
#[derive(Clone, Debug)]
pub(crate) struct StrObj {
    pub s: Box<str>,
    pub hash: u32,
    pub chars: u32,
}

impl StrObj {
    pub fn new(s: &str) -> StrObj {
        StrObj { s: Box::from(s), hash: hash_str(s), chars: s.chars().count() as u32 }
    }
}

/// a closure: its function (index into the program's protos) and the cells it captured
#[derive(Clone, Debug)]
pub(crate) struct Closure {
    pub proto: u32,
    pub ups: Box<[Ref]>,
}

#[derive(Clone, Debug)]
pub(crate) enum Obj {
    Free,
    Str(StrObj),
    Bytes(Box<[u8]>),
    List(Vec<Value>),
    Map(Box<Map>),
    Func(Closure),
    Cell(Value),
}

/// FNV-1a over the bytes
pub(crate) fn hash_str(s: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in s.as_bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

pub(crate) fn hash_int(i: i64) -> u32 {
    let x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    ((x >> 32) as u32) ^ (x as u32)
}

fn hash_bool(b: bool) -> u32 {
    if b { 0x5BD1_E995 } else { 0x1B87_3593 }
}

/// a map key while looking it up: the value's content and hash
#[derive(Clone, Copy)]
pub(crate) enum Key<'a> {
    Int(i64),
    Bool(bool),
    Str(&'a str, u32),
}

impl Key<'_> {
    pub fn hash(&self) -> u32 {
        match *self {
            Key::Int(i) => hash_int(i),
            Key::Bool(b) => hash_bool(b),
            Key::Str(_, h) => h,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    /// Nil: removed
    pub key: Value,
    pub val: Value,
    pub hash: u32,
}

/// an insertion ordered hash map
#[derive(Clone, Debug, Default)]
pub(crate) struct Map {
    pub entries: Vec<Entry>,
    /// 0 empty, TOMB removed, else entry index + 1; empty while there are at most SMALL entries
    index: Vec<u32>,
    pub live: u32,
}

const SMALL: usize = 8;
const TOMB: u32 = u32::MAX;

impl Map {
    /// the position of a key in entries
    pub fn find(&self, objs: &[Obj], key: Key) -> Option<usize> {
        let h = key.hash();
        let same = |e: &Entry| {
            e.hash == h
                && match (key, e.key) {
                    (Key::Int(a), Value::Int(b)) => a == b,
                    (Key::Bool(a), Value::Bool(b)) => a == b,
                    (Key::Str(a, _), Value::Str(r)) => matches!(&objs[r.0 as usize], Obj::Str(s) if &*s.s == a),
                    _ => false,
                }
        };
        if self.index.is_empty() {
            return self.entries.iter().position(same);
        }
        let mask = self.index.len() - 1;
        let mut i = h as usize & mask;
        loop {
            match self.index[i] {
                0 => return None,
                TOMB => {}
                n => {
                    if same(&self.entries[n as usize - 1]) {
                        return Some(n as usize - 1);
                    }
                }
            }
            i = (i + 1) & mask;
        }
    }

    /// append a key that is not in the map
    pub fn push(&mut self, key: Value, hash: u32, val: Value) {
        self.entries.push(Entry { key, val, hash });
        self.live += 1;
        if self.entries.len() > SMALL {
            if self.entries.len() * 2 > self.index.len() {
                self.reindex();
            } else {
                self.place(self.entries.len() - 1);
            }
        }
    }

    fn place(&mut self, n: usize) {
        let mask = self.index.len() - 1;
        let mut i = self.entries[n].hash as usize & mask;
        while self.index[i] != 0 {
            i = (i + 1) & mask;
        }
        self.index[i] = n as u32 + 1;
    }

    fn reindex(&mut self) {
        if self.entries.len() <= SMALL {
            self.index = Vec::new();
            return;
        }
        let mut cap = 16;
        while cap < self.entries.len() * 2 + 2 {
            cap *= 2;
        }
        self.index = vec![0; cap];
        for n in 0..self.entries.len() {
            if self.entries[n].key != Value::Nil {
                self.place(n);
            }
        }
    }

    /// remove the entry at position n; compacts when holes outnumber the entries
    pub fn remove_at(&mut self, n: usize) {
        let e = &mut self.entries[n];
        e.key = Value::Nil;
        e.val = Value::Nil;
        self.live -= 1;
        if !self.index.is_empty() {
            let mask = self.index.len() - 1;
            let mut i = self.entries[n].hash as usize & mask;
            while self.index[i] != n as u32 + 1 {
                i = (i + 1) & mask;
            }
            self.index[i] = TOMB;
        }
        let holes = self.entries.len() - self.live as usize;
        if holes > self.live as usize || self.entries.len() <= SMALL {
            self.entries.retain(|e| e.key != Value::Nil);
            self.reindex();
        }
    }

    /// the live entries in order
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.key != Value::Nil)
    }
}

/// the accounted size of an object in bytes (the same on every target)
pub(crate) fn size_of(o: &Obj) -> u64 {
    match o {
        Obj::Free => 0,
        Obj::Str(s) => 32 + s.s.len() as u64,
        Obj::Bytes(b) => 32 + b.len() as u64,
        Obj::List(l) => 32 + 16 * l.len() as u64,
        Obj::Map(m) => 48 + 40 * m.entries.len() as u64 + 4 * m.index.len() as u64,
        Obj::Func(f) => 32 + 4 * f.ups.len() as u64,
        Obj::Cell(_) => 32,
    }
}

/// the objects of one VM
#[derive(Clone, Debug, Default)]
pub(crate) struct Heap {
    pub objs: Vec<Obj>,
    marks: Vec<bool>,
    free: Vec<u32>,
    /// accounted bytes of the objects that can be collected
    pub bytes: u64,
    /// live objects
    pub count: u32,
}

impl Heap {
    /// store an object (no limit check, no collection: the VM does both)
    pub fn insert(&mut self, o: Obj) -> Ref {
        self.bytes += size_of(&o);
        self.count += 1;
        if let Some(i) = self.free.pop() {
            self.objs[i as usize] = o;
            Ref(i)
        } else {
            self.objs.push(o);
            self.marks.push(false);
            Ref(self.objs.len() as u32 - 1)
        }
    }

    pub fn get_mut(&mut self, r: Ref) -> &mut Obj {
        &mut self.objs[r.0 as usize]
    }

    pub fn str(&self, r: Ref) -> &StrObj {
        match &self.objs[r.0 as usize] {
            Obj::Str(s) => s,
            _ => unreachable!("not a str"),
        }
    }

    pub fn bytes_of(&self, r: Ref) -> &[u8] {
        match &self.objs[r.0 as usize] {
            Obj::Bytes(b) => b,
            _ => unreachable!("not bytes"),
        }
    }

    pub fn list(&self, r: Ref) -> &Vec<Value> {
        match &self.objs[r.0 as usize] {
            Obj::List(l) => l,
            _ => unreachable!("not a list"),
        }
    }

    pub fn map(&self, r: Ref) -> &Map {
        match &self.objs[r.0 as usize] {
            Obj::Map(m) => m,
            _ => unreachable!("not a map"),
        }
    }

    pub fn closure(&self, r: Ref) -> &Closure {
        match &self.objs[r.0 as usize] {
            Obj::Func(f) => f,
            _ => unreachable!("not a function"),
        }
    }

    pub fn cell(&self, r: Ref) -> Value {
        match &self.objs[r.0 as usize] {
            Obj::Cell(v) => *v,
            _ => unreachable!("not a cell"),
        }
    }

    pub fn set_cell(&mut self, r: Ref, v: Value) {
        if let Obj::Cell(c) = &mut self.objs[r.0 as usize] {
            *c = v;
        }
    }

    /// change an object and keep the byte count right
    pub fn update<T>(&mut self, r: Ref, f: impl FnOnce(&mut Obj) -> T) -> T {
        let o = &mut self.objs[r.0 as usize];
        let before = size_of(o);
        let out = f(o);
        let after = size_of(&self.objs[r.0 as usize]);
        self.bytes = (self.bytes + after).saturating_sub(before);
        out
    }

    /// the lookup key of a value; None for types that cannot be keys
    pub fn key(&self, v: Value) -> Option<Key<'_>> {
        match v {
            Value::Int(i) => Some(Key::Int(i)),
            Value::Bool(b) => Some(Key::Bool(b)),
            Value::Str(r) => {
                let s = self.str(r);
                Some(Key::Str(&s.s, s.hash))
            }
            _ => None,
        }
    }

    /// the entry position of key in map m
    pub fn map_find(&self, m: Ref, key: Key) -> Option<usize> {
        self.map(m).find(&self.objs, key)
    }

    /// set m[k] = v; k must be a valid key value
    pub fn map_set(&mut self, m: Ref, k: Value, v: Value) {
        let (pos, hash) = {
            let key = self.key(k).expect("valid key");
            (self.map_find(m, key), key.hash())
        };
        self.update(m, |o| {
            if let Obj::Map(map) = o {
                match pos {
                    Some(n) => map.entries[n].val = v,
                    None => map.push(k, hash, v),
                }
            }
        });
    }

    // ---- collection

    /// mark a value as reachable; objects with children go on the gray list
    pub fn mark(&mut self, v: Value, gray: &mut Vec<u32>) {
        if let Some(r) = v.heap_ref() {
            self.mark_ref(r, gray);
        }
    }

    pub fn mark_ref(&mut self, r: Ref, gray: &mut Vec<u32>) {
        let i = r.0 as usize;
        if !self.marks[i] {
            self.marks[i] = true;
            if !matches!(self.objs[i], Obj::Str(_) | Obj::Bytes(_)) {
                gray.push(r.0);
            }
        }
    }

    /// mark everything reachable from the gray objects
    pub fn trace(&mut self, gray: &mut Vec<u32>) {
        let mut kids: Vec<Value> = Vec::new();
        while let Some(i) = gray.pop() {
            kids.clear();
            match &self.objs[i as usize] {
                Obj::List(l) => kids.extend_from_slice(l),
                Obj::Map(m) => {
                    for e in &m.entries {
                        kids.push(e.key);
                        kids.push(e.val);
                    }
                }
                Obj::Func(f) => kids.extend(f.ups.iter().map(|&r| Value::Cell(r))),
                Obj::Cell(v) => kids.push(*v),
                _ => {}
            }
            for k in kids.iter() {
                self.mark(*k, gray);
            }
        }
    }

    /// free the unmarked objects (except those below `keep`, the VM's permanent constants)
    /// and clear the marks
    pub fn sweep(&mut self, keep: u32) {
        for i in 0..self.objs.len() {
            if self.marks[i] {
                self.marks[i] = false;
            } else if i as u32 >= keep && !matches!(self.objs[i], Obj::Free) {
                self.bytes = self.bytes.saturating_sub(size_of(&self.objs[i]));
                self.count -= 1;
                self.objs[i] = Obj::Free;
                self.free.push(i as u32);
            }
        }
    }
}

/// the text of a string object
pub(crate) fn text(heap: &Heap, r: Ref) -> &str {
    &heap.str(r).s
}

/// a string with the char index i (None when out of range)
pub(crate) fn char_at(s: &StrObj, i: i64) -> Option<char> {
    if i < 0 || i >= s.chars as i64 {
        return None;
    }
    if s.chars as usize == s.s.len() {
        return Some(s.s.as_bytes()[i as usize] as char);
    }
    s.s.chars().nth(i as usize)
}
