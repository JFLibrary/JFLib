// snapshot -- the replicated world of one tick and its delta encoding
//
//   NetState      what is replicated of one entity: a kind (the game's) and up to MAX_FIELDS integers
//                 (already quantised by the game: positions on the grid of quant.rs, angles in steps)
//   Ent           a net id (u16), a generation (u8: a new entity under an old id) and its NetState
//   Snapshot      a tick and its entities, sorted by id
//   Schema        how many fields each kind has (both sides know it; a kind with none is invalid)
//   diff          the changes from a base to a snapshot: Update (fields that differ), Add (new, or a new
//                 generation or kind under an old id), Remove
//   encode        the chosen changes as bits: a count, then per change its id gap, an op (2 bits), and for
//                 an update a mask of the changed fields with their differences (svar); an add sends its
//                 generation, kind and fields against the map's baseline entry (when it has one of the same
//                 kind) or against zero
//   decode        the inverse against the same base (None for anything malformed: never a panic)
//   select        the changes that fit a budget in a given order (the caller's priorities), exactly
//                 measured; apply builds what the client will hold: the base with the chosen changes
//
// The delta is taken against a base the client holds (docs/NETCODE.md): the newest snapshot it confirmed
// over UDP, the last one sent over a WebSocket, or the map's baseline before it has any.  A change left out
// for the budget is simply not in the as-sent snapshot; the next delta carries it.

use alloc::vec::Vec;

use super::bits::{svar_bits, uvar_bits, BitReader, BitWriter};

/// fields an entity may have at most
pub const MAX_FIELDS: usize = 16;
/// entities a snapshot may have at most (a decoder refuses more)
pub const MAX_ENTITIES: usize = 8192;

/// the replicated values of one entity (fields past its kind's count stay 0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetState {
    /// the game's kind of entity (Schema says its field count)
    pub kind: u8,
    /// the values, quantised by the game
    pub fields: [i32; MAX_FIELDS],
}

impl NetState {
    /// a state of `kind` with these fields (the rest 0)
    pub fn new(kind: u8, f: &[i32]) -> NetState {
        let mut fields = [0; MAX_FIELDS];
        let n = f.len().min(MAX_FIELDS);
        fields[..n].copy_from_slice(&f[..n]);
        NetState { kind, fields }
    }
}

/// an entity of a snapshot
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ent {
    /// the net id (the server's; not the World's Entity)
    pub id: u16,
    /// counts up when the id is given to a new entity, so a client never blends the old into the new
    pub generation: u8,
    /// the values
    pub state: NetState,
}

/// the replicated world of a tick
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// the server's tick (wraps: compare with tick_diff)
    pub tick: u32,
    /// the entities, sorted by id, ids unique
    pub ents: Vec<Ent>,
}

/// a - b for ticks that wrap (positive: a is later)
pub fn tick_diff(a: u32, b: u32) -> i32 {
    a.wrapping_sub(b) as i32
}

impl Snapshot {
    /// an empty snapshot of a tick
    pub fn new(tick: u32) -> Snapshot {
        Snapshot { tick, ents: Vec::new() }
    }

    /// the entity with this id
    pub fn get(&self, id: u16) -> Option<&Ent> {
        self.ents.binary_search_by_key(&id, |e| e.id).ok().map(|i| &self.ents[i])
    }

    /// put an entity in (replacing one with its id)
    pub fn put(&mut self, e: Ent) {
        match self.ents.binary_search_by_key(&e.id, |x| x.id) {
            Ok(i) => self.ents[i] = e,
            Err(i) => self.ents.insert(i, e),
        }
    }

    /// take an entity out
    pub fn remove(&mut self, id: u16) -> Option<Ent> {
        self.ents.binary_search_by_key(&id, |e| e.id).ok().map(|i| self.ents.remove(i))
    }

    /// sort by id (a builder that pushed in another order); false when two share an id
    pub fn sort(&mut self) -> bool {
        self.ents.sort_by_key(|e| e.id);
        self.ents.windows(2).all(|w| w[0].id < w[1].id)
    }
}

/// the field count of every kind (both sides of a game share it)
#[derive(Clone, Debug)]
pub struct Schema {
    counts: [u8; 256],
}

impl Default for Schema {
    fn default() -> Self {
        Schema::new()
    }
}

impl Schema {
    /// no kinds
    pub const fn new() -> Schema {
        Schema { counts: [0; 256] }
    }

    /// the same with `kind` having `fields` fields (1 ..= MAX_FIELDS)
    pub fn with(mut self, kind: u8, fields: usize) -> Schema {
        self.counts[kind as usize] = fields.clamp(1, MAX_FIELDS) as u8;
        self
    }

    /// the fields of a kind (0: not a kind of this game)
    pub fn fields(&self, kind: u8) -> usize {
        self.counts[kind as usize] as usize
    }
}

/// what a change does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// fields of an entity the base has changed
    Update,
    /// an entity the base does not have (or has under another generation or kind)
    Add,
    /// an entity of the base is gone
    Remove,
}

/// one change from a base to a snapshot
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    /// the entity
    pub id: u16,
    /// what happens to it
    pub op: Op,
    /// the bits of its op and body (without the id gap)
    pub bits: usize,
}

fn mask_of(a: &NetState, b: &NetState, n: usize) -> u32 {
    let mut m = 0u32;
    for i in 0..n {
        if a.fields[i] != b.fields[i] {
            m |= 1 << i;
        }
    }
    m
}

fn body_bits(base: &NetState, cur: &NetState, n: usize) -> usize {
    let mut b = n;
    for i in 0..n {
        if base.fields[i] != cur.fields[i] {
            b += svar_bits(cur.fields[i].wrapping_sub(base.fields[i]));
        }
    }
    b
}

const ZERO: NetState = NetState { kind: 0, fields: [0; MAX_FIELDS] };

// what an add is encoded against: the baseline's entry of the same id and kind, else zero
fn add_ref<'a>(baseline: Option<&'a Snapshot>, e: &Ent) -> (bool, &'a NetState) {
    match baseline.and_then(|b| b.get(e.id)) {
        Some(b) if b.state.kind == e.state.kind => (true, &b.state),
        _ => (false, &ZERO),
    }
}

/// the changes from `base` (None: nothing) to `cur`, in id order
pub fn diff(schema: &Schema, base: Option<&Snapshot>, cur: &Snapshot, baseline: Option<&Snapshot>, out: &mut Vec<Change>) {
    out.clear();
    let empty: &[Ent] = &[];
    let b = base.map(|s| &s.ents[..]).unwrap_or(empty);
    let c = &cur.ents[..];
    let (mut i, mut j) = (0, 0);
    let add = |e: &Ent| {
        let n = schema.fields(e.state.kind);
        let (_, r) = add_ref(baseline, e);
        Change { id: e.id, op: Op::Add, bits: 2 + 8 + 8 + 1 + body_bits(r, &e.state, n) }
    };
    while i < b.len() || j < c.len() {
        if j >= c.len() || (i < b.len() && b[i].id < c[j].id) {
            out.push(Change { id: b[i].id, op: Op::Remove, bits: 2 });
            i += 1;
        } else if i >= b.len() || c[j].id < b[i].id {
            out.push(add(&c[j]));
            j += 1;
        } else {
            let (x, y) = (&b[i], &c[j]);
            if x.generation != y.generation || x.state.kind != y.state.kind {
                out.push(add(y));
            } else if x.state != y.state {
                let n = schema.fields(y.state.kind);
                out.push(Change { id: y.id, op: Op::Update, bits: 2 + body_bits(&x.state, &y.state, n) });
            }
            i += 1;
            j += 1;
        }
    }
}

/// the exact bits of encoding `chosen` (in id order): the count and every change with its id gap
pub fn encoded_bits(chosen: &[Change]) -> usize {
    let mut bits = uvar_bits(chosen.len() as u32);
    let mut prev: i32 = -1;
    for c in chosen {
        bits += uvar_bits((c.id as i32 - prev - 1) as u32) + c.bits;
        prev = c.id as i32;
    }
    bits
}

/// the changes of `order` (indices into `changes`, the most important first) that fit `budget` bits, in id
/// order.  Greedy by importance with the id gaps counted at 10 bits, then measured exactly and the least
/// important dropped while too large
pub fn select(changes: &[Change], order: &[usize], budget: usize) -> Vec<Change> {
    let mut picked: Vec<usize> = Vec::new();
    let mut est = uvar_bits(changes.len() as u32);
    for &k in order {
        let cost = 10 + changes[k].bits;
        if est + cost <= budget {
            est += cost;
            picked.push(k);
        }
    }
    loop {
        let mut chosen: Vec<Change> = picked.iter().map(|&k| changes[k]).collect();
        chosen.sort_by_key(|c| c.id);
        if encoded_bits(&chosen) <= budget || picked.is_empty() {
            return chosen;
        }
        picked.pop();
    }
}

/// write `chosen` (from diff, in id order) against `base`
pub fn encode(w: &mut BitWriter, schema: &Schema, base: Option<&Snapshot>, cur: &Snapshot, baseline: Option<&Snapshot>, chosen: &[Change]) {
    w.uvar(chosen.len() as u32);
    let mut prev: i32 = -1;
    for c in chosen {
        w.uvar((c.id as i32 - prev - 1) as u32);
        prev = c.id as i32;
        match c.op {
            Op::Remove => w.bits(2, 2),
            Op::Update => {
                w.bits(0, 2);
                let (Some(x), Some(y)) = (base.and_then(|b| b.get(c.id)), cur.get(c.id)) else { continue };
                let n = schema.fields(y.state.kind);
                write_fields(w, &x.state, &y.state, n);
            }
            Op::Add => {
                w.bits(1, 2);
                let Some(y) = cur.get(c.id) else { continue };
                let n = schema.fields(y.state.kind);
                let (from, r) = add_ref(baseline, y);
                w.u8(y.generation);
                w.u8(y.state.kind);
                w.bool(from);
                write_fields(w, r, &y.state, n);
            }
        }
    }
}

fn write_fields(w: &mut BitWriter, base: &NetState, cur: &NetState, n: usize) {
    let m = mask_of(base, cur, n);
    w.bits(m, n as u32);
    for i in 0..n {
        if m & (1 << i) != 0 {
            w.svar(cur.fields[i].wrapping_sub(base.fields[i]));
        }
    }
}

fn read_fields(r: &mut BitReader, base: &NetState, kind: u8, n: usize) -> NetState {
    let m = r.bits(n as u32);
    let mut s = NetState { kind, fields: [0; MAX_FIELDS] };
    s.fields[..n].copy_from_slice(&base.fields[..n]);
    for i in 0..n {
        if m & (1 << i) != 0 {
            s.fields[i] = base.fields[i].wrapping_add(r.svar());
        }
    }
    s
}

/// base with the chosen changes of cur: what the client holds after decoding them (the as-sent snapshot)
pub fn apply(base: Option<&Snapshot>, cur: &Snapshot, chosen: &[Change]) -> Snapshot {
    let mut s = base.cloned().unwrap_or_default();
    s.tick = cur.tick;
    for c in chosen {
        match c.op {
            Op::Remove => {
                s.remove(c.id);
            }
            Op::Update | Op::Add => {
                if let Some(e) = cur.get(c.id) {
                    s.put(*e);
                }
            }
        }
    }
    s
}

/// read the changes against `base` into the snapshot of `tick` (None: malformed, an update or remove of
/// an id the base lacks, a kind the schema does not know, ids out of order)
pub fn decode(r: &mut BitReader, schema: &Schema, base: Option<&Snapshot>, baseline: Option<&Snapshot>, tick: u32) -> Option<Snapshot> {
    let n = r.uvar() as usize;
    if !r.ok() || n > MAX_ENTITIES {
        return None;
    }
    let mut s = base.cloned().unwrap_or_default();
    s.tick = tick;
    let mut prev: i32 = -1;
    for _ in 0..n {
        let gap = r.uvar() as i64;
        let id = prev as i64 + 1 + gap;
        if !r.ok() || id > u16::MAX as i64 {
            return None;
        }
        let id = id as u16;
        prev = id as i32;
        match r.bits(2) {
            0 => {
                let x = *s.get(id)?;
                let k = schema.fields(x.state.kind);
                if k == 0 {
                    return None;
                }
                let st = read_fields(r, &x.state, x.state.kind, k);
                s.put(Ent { id, generation: x.generation, state: st });
            }
            1 => {
                let generation = r.u8();
                let kind = r.u8();
                let from = r.bool();
                let k = schema.fields(kind);
                if k == 0 {
                    return None;
                }
                let refst = if from {
                    match baseline.and_then(|b| b.get(id)) {
                        Some(b) if b.state.kind == kind => b.state,
                        _ => return None,
                    }
                } else {
                    ZERO
                };
                let st = read_fields(r, &refst, kind, k);
                s.put(Ent { id, generation, state: st });
            }
            2 => {
                s.remove(id)?;
            }
            _ => return None,
        }
        if !r.ok() {
            return None;
        }
        if s.ents.len() > MAX_ENTITIES {
            return None;
        }
    }
    Some(s)
}

/// the bits of the whole snapshot encoded against `base` without a budget (the size of a join's state)
pub fn full_bits(schema: &Schema, base: Option<&Snapshot>, cur: &Snapshot, baseline: Option<&Snapshot>) -> usize {
    let mut ch = Vec::new();
    diff(schema, base, cur, baseline, &mut ch);
    encoded_bits(&ch)
}
