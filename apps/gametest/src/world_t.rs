// world_t -- the group "world": entities and generations, columns (sparse sets) and their order, versions,
// a random walk against a model of plain vectors, many entities, the canonical order

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use game::{Entity, Pose, SparseSet, Vec3, World};

use crate::{Ctx, Rng, out};

pub fn run(c: &mut Ctx) {
    entities(c);
    columns(c);
    random_walk(c);
    many(c);
    canonical(c);
}

/// a component of the tests
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hp(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vel([f64; 2]);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tag(u32);

fn entities(c: &mut Ctx) {
    let mut w = World::new();
    let a = w.spawn();
    let b = w.spawn();
    let d = w.spawn();
    c.check((a.index(), b.index(), d.index()) == (0, 1, 2) && a.generation() == 0 && d.generation() == 0, || format!("first ids {a:?} {b:?} {d:?}"));
    c.check(w.len() == 3 && w.alive(b), || "three alive".into());
    c.check(w.despawn(b) && !w.alive(b) && w.len() == 2, || "despawn".into());
    c.check(!w.despawn(b), || "a second despawn of the same id changes nothing".into());
    let b2 = w.spawn();
    c.check(b2.index() == 1 && b2.generation() == 1, || format!("the freed slot again, next generation: {b2:?}"));
    // the stale id: refused everywhere, the new entity untouched
    c.check(!w.insert(b, Hp(5)) && !w.has::<Hp>(b2), || "insert on a stale id refused".into());
    c.check(w.insert(b2, Hp(7)) && w.get::<Hp>(b) .is_none() && w.get::<Hp>(b2) == Some(&Hp(7)), || "stale id finds nothing".into());
    c.check(w.get_mut::<Hp>(b).is_none() && w.remove::<Hp>(b).is_none() && w.get::<Hp>(b2) == Some(&Hp(7)), || "stale get_mut / remove".into());
    c.check(!w.despawn(b) && w.alive(b2), || "despawn of a stale id".into());
    c.check(Entity::from_bits(b2.to_bits()) == b2 && b2.to_bits() == (1u64 << 32 | 1), || "to_bits / from_bits".into());
    c.check(!w.alive(Entity::NONE) && w.get::<Hp>(Entity::NONE).is_none() && !w.insert(Entity::NONE, Hp(1)), || "Entity::NONE".into());
    // the slot freed last comes first (LIFO), generations count up per slot
    let e: Vec<Entity> = (0..5).map(|_| w.spawn()).collect();
    w.despawn(e[1]);
    w.despawn(e[3]);
    let f = w.spawn();
    let g = w.spawn();
    c.check(f.index() == e[3].index() && g.index() == e[1].index() && f.generation() == 1 && g.generation() == 1, || format!("LIFO: {f:?} {g:?}"));
    let order: Vec<u32> = w.entities().iter().map(|x| x.index()).collect();
    c.check(order == (0..8).collect::<Vec<u32>>(), || format!("live entities in index order: {order:?}"));
    // one slot used 100 000 times: generation 100 000, never another index
    let mut w = World::new();
    let mut last = w.spawn();
    for _ in 0..c.n(100_000) {
        w.despawn(last);
        last = w.spawn();
    }
    c.check(last.index() == 0 && last.generation() as usize == c.n(100_000) && w.entities().slots() == 1, || format!("one slot reused: {last:?}"));
    // despawn takes the components of every column
    let mut w = World::new();
    let x = w.spawn_with((Hp(1), Vel([1.0, 2.0]), Tag(3)));
    let y = w.spawn_with((Hp(2), Tag(4)));
    c.check(w.get::<Vel>(x) == Some(&Vel([1.0, 2.0])) && w.get::<Tag>(y) == Some(&Tag(4)) && !w.has::<Vel>(y), || "spawn_with".into());
    w.despawn(x);
    let cols = w.columns();
    c.check(cols.iter().map(|(_, n)| *n).collect::<Vec<_>>() == vec![1, 0, 1], || format!("despawn empties every column: {cols:?}"));
    c.check(cols[0].0.ends_with("Hp") && cols[1].0.ends_with("Vel"), || format!("columns in the order made: {cols:?}"));
}

fn columns(c: &mut Ctx) {
    let mut w = World::new();
    let e: Vec<Entity> = (0..6).map(|_| w.spawn()).collect();
    for (i, &x) in e.iter().enumerate().take(4) {
        w.insert(x, Hp(i as u64 * 10));
    }
    let col = w.column::<Hp>().expect("column");
    c.check(col.entities() == &e[..4], || "insert appends".into());
    let v0 = col.version();
    c.check(v0 == 4 && col.changed_at(e[3]) == Some(4), || format!("four inserts: version {v0}"));
    // remove the second: the last moves into its place
    w.remove::<Hp>(e[1]);
    w.insert(e[4], Hp(40));
    let col = w.column::<Hp>().expect("column");
    c.check(col.entities() == [e[0], e[3], e[2], e[4]], || format!("swap remove: {:?}", col.entities()));
    c.check(col.values() == [Hp(0), Hp(30), Hp(20), Hp(40)] && col.removals() == 1, || "values follow".into());
    // a read through get_mut leaves the version of the entry, a write stamps it
    let before = w.column::<Hp>().unwrap().changed_at(e[2]);
    let r = w.get_mut::<Hp>(e[2]).map(|m| m.0);
    c.check(r == Some(20) && w.column::<Hp>().unwrap().changed_at(e[2]) == before, || "a read leaves the version".into());
    if let Some(mut m) = w.get_mut::<Hp>(e[2]) {
        m.0 = 21;
    }
    let col = w.column::<Hp>().unwrap();
    let now = col.changed_at(e[2]).unwrap_or(0);
    c.check(now > before.unwrap_or(0) && now == col.version(), || "a write stamps the entry".into());
    let changed: Vec<Entity> = col.changed_since(before.unwrap_or(0).max(4)).map(|(x, _)| x).collect();
    c.check(changed.contains(&e[2]) && changed.contains(&e[4]), || format!("changed_since: {changed:?}"));
    // iter_mut: only what it writes takes the new version
    let mark = w.column::<Hp>().unwrap().version();
    for (x, mut v) in w.column_mut::<Hp>().iter_mut() {
        if x == e[0] || x == e[4] {
            v.0 += 1;
        } else {
            let _ = v.0;
        }
    }
    let col = w.column::<Hp>().unwrap();
    let changed: Vec<Entity> = col.changed_since(mark).map(|(x, _)| x).collect();
    c.check(changed == [e[0], e[4]], || format!("iter_mut stamps what it writes: {changed:?}"));
    // untracked: written, not stamped
    let mark = w.column::<Hp>().unwrap().version();
    if let Some(m) = w.get_mut::<Hp>(e[3]) {
        m.untracked().0 = 99;
    }
    let col = w.column::<Hp>().unwrap();
    c.check(col.get(e[3]) == Some(&Hp(99)) && col.changed_since(mark).count() == 0, || "untracked".into());
    // touch, clear
    let col = w.column_mut::<Hp>();
    let mark = col.version();
    c.check(col.touch(e[3]) && !col.touch(e[5]) && col.changed_since(mark).count() == 1, || "touch".into());
    let n = col.len() as u64;
    let r0 = col.removals();
    col.clear();
    c.check(col.is_empty() && col.removals() == r0 + n && !col.contains(e[0]), || "clear".into());
    // pair_mut: one column written from another
    let mut w = World::new();
    for i in 0..10 {
        w.spawn_with((Hp(i), Vel([i as f64, 0.0])));
    }
    let (hp, vel) = w.pair_mut::<Hp, Vel>();
    for (x, mut h) in hp.iter_mut() {
        if let Some(v) = vel.get(x) {
            h.0 += v.0[0] as u64;
        }
    }
    let sum: u64 = w.column::<Hp>().unwrap().values().iter().map(|h| h.0).sum();
    c.check(sum == 90, || format!("pair_mut: {sum}"));
    let (vel, hp) = w.pair_mut::<Vel, Hp>();
    c.check(vel.len() == 10 && hp.len() == 10, || "pair_mut the other way round".into());
    // a sparse column of one entity far out: a page, not four bytes per entity
    let mut s: SparseSet<Tag> = SparseSet::new();
    let far = Entity::from_bits(999_999);
    s.insert(far, Tag(1));
    c.check(s.get(far) == Some(&Tag(1)) && s.index_bytes() < 32 * 1024, || format!("paged index: {} bytes", s.index_bytes()));
    // sort: ascending entity order, values and versions along
    let mut s: SparseSet<Tag> = SparseSet::new();
    for i in [5u64, 2, 9, 0, 7] {
        s.insert(Entity::from_bits(i), Tag(i as u32 * 3));
    }
    let v9 = s.changed_at(Entity::from_bits(9));
    s.sort();
    let idx: Vec<u32> = s.entities().iter().map(|x| x.index()).collect();
    c.check(idx == [0, 2, 5, 7, 9] && s.values().iter().map(|t| t.0).collect::<Vec<_>>() == [0, 6, 15, 21, 27], || format!("sort: {idx:?}"));
    c.check(s.changed_at(Entity::from_bits(9)) == v9 && s.get(Entity::from_bits(7)) == Some(&Tag(21)), || "sort keeps versions and lookups".into());
}

/// the model of a column: entries in the order of iteration by the rule push / swap remove, versions; a
/// plain array from entity index to position
struct MCol<T> {
    entries: Vec<(Entity, T, u64)>,
    at: Vec<usize>,
    version: u64,
    removals: u64,
}

impl<T: Copy + PartialEq> MCol<T> {
    fn new() -> Self {
        MCol { entries: Vec::new(), at: Vec::new(), version: 0, removals: 0 }
    }
    fn slot(&self, index: u32) -> Option<usize> {
        self.at.get(index as usize).copied().filter(|&p| p != usize::MAX)
    }
    fn pos(&self, e: Entity) -> Option<usize> {
        self.slot(e.index()).filter(|&p| self.entries[p].0 == e)
    }
    fn insert(&mut self, e: Entity, v: T) {
        self.version += 1;
        match self.slot(e.index()) {
            Some(p) => self.entries[p] = (e, v, self.version),
            None => {
                if self.at.len() <= e.index() as usize {
                    self.at.resize(e.index() as usize + 1, usize::MAX);
                }
                self.at[e.index() as usize] = self.entries.len();
                self.entries.push((e, v, self.version));
            }
        }
    }
    fn remove(&mut self, e: Entity) -> Option<T> {
        let p = self.pos(e)?;
        self.removals += 1;
        let last = self.entries.len() - 1;
        let moved = self.entries[last].0;
        self.at[moved.index() as usize] = p;
        self.at[e.index() as usize] = usize::MAX;
        Some(self.entries.swap_remove(p).1)
    }
}

/// the model of the world: generations and liveness per slot, the freed slots (last on top)
struct Model {
    gens: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
    hp: MCol<Hp>,
    vel: MCol<Vel>,
}

impl Model {
    fn spawn(&mut self) -> Entity {
        if let Some(i) = self.free.pop() {
            self.alive[i as usize] = true;
            return Entity::from_bits((self.gens[i as usize] as u64) << 32 | i as u64);
        }
        self.gens.push(0);
        self.alive.push(true);
        Entity::from_bits((self.gens.len() - 1) as u64)
    }
    fn alive(&self, e: Entity) -> bool {
        let i = e.index() as usize;
        i < self.alive.len() && self.alive[i] && self.gens[i] == e.generation()
    }
    fn despawn(&mut self, e: Entity) -> bool {
        if !self.alive(e) {
            return false;
        }
        let i = e.index() as usize;
        self.alive[i] = false;
        self.gens[i] += 1;
        self.free.push(e.index());
        self.hp.remove(e);
        self.vel.remove(e);
        true
    }
    fn live(&self) -> Vec<Entity> {
        (0..self.alive.len()).filter(|&i| self.alive[i]).map(|i| Entity::from_bits((self.gens[i] as u64) << 32 | i as u64)).collect()
    }
}

fn same_col<T: Copy + PartialEq + core::fmt::Debug + 'static>(c: &mut Ctx, w: &World, m: &MCol<T>, what: &str, step: usize) -> bool {
    let col = w.column::<T>();
    let (es, vs, cs, ver, rem) = match col {
        Some(col) => (col.entities().to_vec(), col.values().to_vec(), col.versions().to_vec(), col.version(), col.removals()),
        None => (Vec::new(), Vec::new(), Vec::new(), 0, 0),
    };
    let me: Vec<Entity> = m.entries.iter().map(|x| x.0).collect();
    let mv: Vec<T> = m.entries.iter().map(|x| x.1).collect();
    let mc: Vec<u64> = m.entries.iter().map(|x| x.2).collect();
    c.check(es == me && vs == mv && cs == mc && ver == m.version && rem == m.removals,
            || format!("{what} at step {step}: entities {} vs {}, version {ver} vs {}, removals {rem} vs {}", es.len(), me.len(), m.version, m.removals))
}

/// the world against the model over random steps; the same steps in a second world iterate alike
fn random_walk(c: &mut Ctx) {
    let steps = c.n(200_000);
    let mut runs: Vec<(Vec<Entity>, Vec<Hp>, Vec<u64>)> = Vec::new();
    for run in 0..2 {
        let mut r = Rng::new(77);
        let mut w = World::new();
        let mut m = Model { gens: Vec::new(), alive: Vec::new(), free: Vec::new(), hp: MCol::new(), vel: MCol::new() };
        let mut known: Vec<Entity> = Vec::new(); // every id handed out, dead ones too
        let mut bad = 0u64;
        for step in 0..steps {
            // (mostly one of the newest ids, mostly alive; else any id handed out, often a stale one)
            let pick = |r: &mut Rng, known: &Vec<Entity>| match known.len() {
                0 => Entity::NONE,
                n if r.below(10) < 7 => known[n - 1 - r.below(n.min(256))],
                n => known[r.below(n)],
            };
            match r.below(100) {
                0..=19 => {
                    let (a, b) = (w.spawn(), m.spawn());
                    if a != b {
                        bad += 1;
                    }
                    known.push(a);
                }
                20..=34 => {
                    let e = pick(&mut r, &known);
                    if w.despawn(e) != m.despawn(e) {
                        bad += 1;
                    }
                }
                35..=49 => {
                    let e = pick(&mut r, &known);
                    let v = Hp(r.next_u64());
                    let ok = w.insert(e, v);
                    if ok != m.alive(e) {
                        bad += 1;
                    }
                    if ok {
                        m.hp.insert(e, v);
                    }
                }
                50..=59 => {
                    let e = pick(&mut r, &known);
                    let v = Vel([r.f(), r.f()]);
                    if w.insert(e, v) {
                        m.vel.insert(e, v);
                    }
                }
                60..=67 => {
                    let e = pick(&mut r, &known);
                    if w.remove::<Hp>(e) != m.hp.remove(e) {
                        bad += 1;
                    }
                }
                68..=72 => {
                    let e = pick(&mut r, &known);
                    if w.remove::<Vel>(e) != m.vel.remove(e) {
                        bad += 1;
                    }
                }
                73..=84 => {
                    // a write through get_mut: the entry takes the column's next version
                    let e = pick(&mut r, &known);
                    let v = r.next_u64();
                    let had = m.hp.pos(e);
                    match w.get_mut::<Hp>(e) {
                        Some(mut x) => {
                            x.0 = v;
                            m.hp.version += 1;
                            match had {
                                Some(p) => m.hp.entries[p] = (e, Hp(v), m.hp.version),
                                None => bad += 1,
                            }
                        }
                        None => {
                            if had.is_some() {
                                bad += 1;
                            }
                        }
                    }
                }
                _ => {
                    let e = pick(&mut r, &known);
                    if w.alive(e) != m.alive(e) || w.get::<Hp>(e) != m.hp.pos(e).map(|p| &m.hp.entries[p].1) {
                        bad += 1;
                    }
                }
            }
            if step % 997 == 0 || step + 1 == steps {
                if run == 0 {
                    same_col(c, &w, &m.hp, "Hp", step);
                    same_col(c, &w, &m.vel, "Vel", step);
                    let live: Vec<Entity> = w.entities().iter().collect();
                    c.check(live == m.live() && w.len() as usize == live.len(), || format!("live entities at step {step}"));
                }
            }
        }
        c.check(bad == 0, || format!("{bad} answers of the world differ from the model"));
        let col = w.column::<Hp>().unwrap();
        runs.push((col.entities().to_vec(), col.values().to_vec(), col.versions().to_vec()));
        if run == 0 {
            out(&format!("  world: {steps} random steps, {} entities alive, {} slots, Hp {} / Vel {} values\n",
                         w.len(), w.entities().slots(), col.len(), w.column::<Vel>().map_or(0, |v| v.len())));
        }
    }
    c.check(runs[0] == runs[1], || "the same steps iterate alike in a second world".into());
}

/// a million entities (100 000 with --quick): spawn with components, iterate, despawn a third, spawn again
fn many(c: &mut Ctx) {
    let n = c.n(1_000_000) as u32;
    let mut w = World::new();
    let t0 = plat::time_us();
    let mut ids = Vec::with_capacity(n as usize);
    for i in 0..n {
        let e = if i % 4 == 0 {
            w.spawn_with((Pose::at(Vec3::new(i as f64, 0.0, 0.0)), Tag(i)))
        } else {
            w.spawn_with((Pose::at(Vec3::new(i as f64, 0.0, 0.0)),))
        };
        ids.push(e);
    }
    let t1 = plat::time_us();
    let sum: f64 = w.column::<Pose>().unwrap().values().iter().map(|p| p.now.pos.x).sum();
    let expect = (n as f64 - 1.0) * n as f64 / 2.0;
    c.check(sum == expect && w.column::<Tag>().unwrap().len() as u32 == n.div_ceil(4), || format!("sum {sum} vs {expect}"));
    // a join: the tagged ones, looked up in the poses
    let poses = w.column::<Pose>().unwrap();
    let joined = w.column::<Tag>().unwrap().iter().filter(|(e, t)| poses.get(*e).is_some_and(|p| p.now.pos.x == t.0 as f64)).count();
    c.check(joined as u32 == n.div_ceil(4), || format!("join: {joined}"));
    let t2 = plat::time_us();
    for e in ids.iter().step_by(3) {
        w.despawn(*e);
    }
    let gone = n.div_ceil(3);
    c.check(w.len() == n - gone && w.column::<Pose>().unwrap().len() as u32 == n - gone, || "a third despawned".into());
    let again: Vec<Entity> = (0..gone).map(|_| w.spawn()).collect();
    c.check(again.iter().all(|e| e.generation() == 1) && w.entities().slots() == n, || "the freed slots again, generation 1".into());
    c.check(ids.iter().step_by(3).all(|e| !w.alive(*e) && w.get::<Pose>(*e).is_none()), || "the old ids stay dead".into());
    let t3 = plat::time_us();
    out(&format!("  many: {n} entities: spawn with components {} ms, sum and join {} ms, despawn a third and spawn again {} ms\n",
                 (t1 - t0) / 1000, (t2 - t1) / 1000, (t3 - t2) / 1000));
}

/// two worlds with the same entities and values from different histories iterate alike once sorted
fn canonical(c: &mut Ctx) {
    let mut a = World::new();
    let mut b = World::new();
    let ea: Vec<Entity> = (0..50).map(|_| a.spawn()).collect();
    let eb: Vec<Entity> = (0..50).map(|_| b.spawn()).collect();
    for i in 0..50 {
        a.insert(ea[i], Tag(i as u32));
    }
    for i in (0..50).rev() {
        b.insert(eb[i], Tag(i as u32 + 1000));
    }
    for i in (0..50).rev() {
        b.insert(eb[i], Tag(i as u32));
    }
    for i in [3usize, 17, 40] {
        a.remove::<Tag>(ea[i]);
        b.remove::<Tag>(eb[i]);
    }
    let differ = a.column::<Tag>().unwrap().entities() != b.column::<Tag>().unwrap().entities();
    a.sort_columns();
    b.sort_columns();
    let (ca, cb) = (a.column::<Tag>().unwrap(), b.column::<Tag>().unwrap());
    c.check(differ && ca.entities() == cb.entities() && ca.values() == cb.values(), || "sorted columns iterate alike".into());
    c.check(ca.entities().windows(2).all(|p| p[0].index() < p[1].index()), || "ascending".into());
}
