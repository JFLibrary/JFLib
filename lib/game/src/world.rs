// world -- entities with a generation and their components, one column per component type
//
//   Entity            an index and a generation: an id of a despawned entity never means the entity
//                     that gets its index next (every use of it is refused or finds nothing)
//   Entities          the allocator: spawn takes the index freed last (a free list), despawn raises the
//                     generation; a slot whose generations are spent is never handed out again
//   World             the entities and a column (SparseSet) per component type, found by the type;
//                     spawn / despawn / insert / remove / get / get_mut / column / column_mut / pair_mut;
//                     begin_tick: the interpolation's bookkeeping of Pose (pose.rs)
//
// No archetypes and no scheduler (docs/GAME.md, G7b: until a measurement asks for them).  A system is a
// function over the world (tick.rs) that takes the columns it needs: one column is a slice of values in
// the order of iteration, a join walks the smaller column and looks the entity up in the other.  The
// world keeps no unordered container: columns are found by their TypeId in a sorted list (a lookup,
// nothing iterates in that order), entities and values iterate in index and column order.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::{Any, TypeId};

use crate::pose::Pose;
use crate::sparse::{Mut, SparseSet};

/// an entity: an index (its slot) and a generation (how often the slot was handed out before).  Plain
/// data: the world decides whether it lives
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    /// no entity: never alive, in no column
    pub const NONE: Entity = Entity { index: u32::MAX, generation: u32::MAX };

    /// the slot of the entity
    pub const fn index(self) -> u32 {
        self.index
    }

    /// how often its slot was handed out before it
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// generation in the high, index in the low 32 bits (for a message or a file)
    pub const fn to_bits(self) -> u64 {
        (self.generation as u64) << 32 | self.index as u64
    }

    /// the entity of to_bits
    pub const fn from_bits(bits: u64) -> Entity {
        Entity { index: bits as u32, generation: (bits >> 32) as u32 }
    }
}

/// the allocator of entities (World holds one)
#[derive(Clone, Debug, Default)]
pub struct Entities {
    /// the generation of every slot now (the live one's, or the next one's)
    generations: Vec<u32>,
    alive: Vec<bool>,
    /// slots to hand out again, the last freed on top
    free: Vec<u32>,
    live: u32,
}

/// the last generation of a slot: an entity with it frees its slot for good
const LAST_GENERATION: u32 = u32::MAX - 1;

impl Entities {
    /// no entities
    pub const fn new() -> Entities {
        Entities { generations: Vec::new(), alive: Vec::new(), free: Vec::new(), live: 0 }
    }

    /// a new entity: the slot freed last, with its next generation, else a new slot
    pub fn spawn(&mut self) -> Entity {
        self.live += 1;
        if let Some(i) = self.free.pop() {
            self.alive[i as usize] = true;
            return Entity { index: i, generation: self.generations[i as usize] };
        }
        let i = self.generations.len();
        assert!(i < u32::MAX as usize, "Entities: more than 2^32 - 1 slots");
        self.generations.push(0);
        self.alive.push(true);
        Entity { index: i as u32, generation: 0 }
    }

    /// e dies, its slot is free for the next generation (false: e was not alive)
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.alive(e) {
            return false;
        }
        let i = e.index as usize;
        self.alive[i] = false;
        self.live -= 1;
        if self.generations[i] < LAST_GENERATION {
            self.generations[i] += 1;
            self.free.push(e.index);
        }
        true
    }

    /// e lives: its slot is taken and has e's generation
    pub fn alive(&self, e: Entity) -> bool {
        let i = e.index as usize;
        i < self.alive.len() && self.alive[i] && self.generations[i] == e.generation
    }

    /// the entity alive in slot `index`
    pub fn at(&self, index: u32) -> Option<Entity> {
        let i = index as usize;
        (i < self.alive.len() && self.alive[i]).then(|| Entity { index, generation: self.generations[i] })
    }

    /// entities alive
    pub fn len(&self) -> u32 {
        self.live
    }

    /// none alive
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// slots so far (alive, free, spent)
    pub fn slots(&self) -> u32 {
        self.generations.len() as u32
    }

    /// the live entities in ascending order of their index
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive.iter().enumerate().filter(|(_, a)| **a)
            .map(|(i, _)| Entity { index: i as u32, generation: self.generations[i] })
    }
}

/// what the world does with every column without knowing its type
trait Column: Any {
    /// e's value goes (despawn)
    fn drop_entity(&mut self, e: Entity) -> bool;
    fn len(&self) -> usize;
    fn type_name(&self) -> &'static str;
    fn sort(&mut self);
}

struct Typed<T: 'static> {
    set: SparseSet<T>,
}

impl<T: 'static> Column for Typed<T> {
    fn drop_entity(&mut self, e: Entity) -> bool {
        self.set.remove(e).is_some()
    }
    fn len(&self) -> usize {
        self.set.len()
    }
    fn type_name(&self) -> &'static str {
        core::any::type_name::<T>()
    }
    fn sort(&mut self) {
        self.set.sort();
    }
}

/// a bundle of components for World::spawn_with: a tuple of one to six values of different types
pub trait Bundle {
    /// every value to e (World::insert)
    fn insert_into(self, world: &mut World, e: Entity);
}

macro_rules! bundle {
    ($($t:ident $v:ident),+) => {
        impl<$($t: 'static),+> Bundle for ($($t,)+) {
            fn insert_into(self, world: &mut World, e: Entity) {
                let ($($v,)+) = self;
                $(world.insert(e, $v);)+
            }
        }
    };
}
bundle!(A a);
bundle!(A a, B b);
bundle!(A a, B b, C c);
bundle!(A a, B b, C c, D d);
bundle!(A a, B b, C c, D d, E e);
bundle!(A a, B b, C c, D d, E e, F f);

/// entities and their components (see the module header)
#[derive(Default)]
pub struct World {
    entities: Entities,
    /// the columns in the order they were made
    columns: Vec<Box<dyn Column>>,
    /// (type, index into columns), sorted by type: the lookup
    lookup: Vec<(TypeId, usize)>,
    tick: u64,
    /// the Pose column's version when the systems of the latest tick began
    pose_mark: u64,
}

impl World {
    /// an empty world
    pub fn new() -> World {
        World::default()
    }

    /// the allocator: which entities live
    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    /// a new entity without components
    pub fn spawn(&mut self) -> Entity {
        self.entities.spawn()
    }

    /// a new entity with the components of a tuple: `world.spawn_with((Pose::at(p), Health(100)))`
    pub fn spawn_with<B: Bundle>(&mut self, bundle: B) -> Entity {
        let e = self.entities.spawn();
        bundle.insert_into(self, e);
        e
    }

    /// e dies with all its components (false: e was not alive: a stale id changes nothing)
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.entities.despawn(e) {
            return false;
        }
        for c in &mut self.columns {
            c.drop_entity(e);
        }
        true
    }

    /// e lives
    pub fn alive(&self, e: Entity) -> bool {
        self.entities.alive(e)
    }

    /// entities alive
    pub fn len(&self) -> u32 {
        self.entities.len()
    }

    /// none alive
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    fn find(&self, t: TypeId) -> Option<usize> {
        self.lookup.binary_search_by(|(k, _)| k.cmp(&t)).ok().map(|i| self.lookup[i].1)
    }

    /// the column of T, made if there is none
    fn index_of<T: 'static>(&mut self) -> usize {
        let t = TypeId::of::<T>();
        match self.lookup.binary_search_by(|(k, _)| k.cmp(&t)) {
            Ok(i) => self.lookup[i].1,
            Err(i) => {
                let c = self.columns.len();
                self.columns.push(Box::new(Typed::<T> { set: SparseSet::new() }));
                self.lookup.insert(i, (t, c));
                c
            }
        }
    }

    fn typed<T: 'static>(c: &dyn Column) -> &SparseSet<T> {
        let any: &dyn Any = c;
        &any.downcast_ref::<Typed<T>>().expect("World: a column of another type").set
    }

    fn typed_mut<T: 'static>(c: &mut dyn Column) -> &mut SparseSet<T> {
        let any: &mut dyn Any = c;
        &mut any.downcast_mut::<Typed<T>>().expect("World: a column of another type").set
    }

    /// make the column of T now (a column is otherwise made by its first insert or column_mut)
    pub fn register<T: 'static>(&mut self) {
        self.index_of::<T>();
    }

    /// the column of T (None: nothing ever asked for it)
    pub fn column<T: 'static>(&self) -> Option<&SparseSet<T>> {
        self.find(TypeId::of::<T>()).map(|i| World::typed::<T>(&*self.columns[i]))
    }

    /// the column of T for writing, made if there is none.  Its values may be written; insert values
    /// through World::insert, which refuses dead entities (the column alone does not know them)
    pub fn column_mut<T: 'static>(&mut self) -> &mut SparseSet<T> {
        let i = self.index_of::<T>();
        World::typed_mut::<T>(&mut *self.columns[i])
    }

    /// the columns of two different types for writing at once (a system that writes one by the other).
    /// Panics when A and B are the same type
    pub fn pair_mut<A: 'static, B: 'static>(&mut self) -> (&mut SparseSet<A>, &mut SparseSet<B>) {
        let (a, b) = (self.index_of::<A>(), self.index_of::<B>());
        assert!(a != b, "World::pair_mut: two columns of the same type");
        let (ca, cb) = if a < b {
            let (lo, hi) = self.columns.split_at_mut(b);
            (&mut lo[a], &mut hi[0])
        } else {
            let (lo, hi) = self.columns.split_at_mut(a);
            (&mut hi[0], &mut lo[b])
        };
        (World::typed_mut::<A>(&mut **ca), World::typed_mut::<B>(&mut **cb))
    }

    /// e gets the component (replacing one of the same type); false: e is not alive, nothing changes
    pub fn insert<T: 'static>(&mut self, e: Entity, value: T) -> bool {
        if !self.entities.alive(e) {
            return false;
        }
        self.column_mut::<T>().insert(e, value);
        true
    }

    /// e's component of type T goes; the value
    pub fn remove<T: 'static>(&mut self, e: Entity) -> Option<T> {
        let i = self.find(TypeId::of::<T>())?;
        World::typed_mut::<T>(&mut *self.columns[i]).remove(e)
    }

    /// e's component of type T
    pub fn get<T: 'static>(&self, e: Entity) -> Option<&T> {
        self.column::<T>()?.get(e)
    }

    /// e's component of type T for writing (a write through it takes a new version)
    pub fn get_mut<T: 'static>(&mut self, e: Entity) -> Option<Mut<'_, T>> {
        let i = self.find(TypeId::of::<T>())?;
        World::typed_mut::<T>(&mut *self.columns[i]).get_mut(e)
    }

    /// e has a component of type T
    pub fn has<T: 'static>(&self, e: Entity) -> bool {
        self.column::<T>().is_some_and(|c| c.contains(e))
    }

    /// (type name, values) of every column, in the order they were made (what a world holds: tests, an
    /// inspector)
    pub fn columns(&self) -> Vec<(&'static str, usize)> {
        self.columns.iter().map(|c| (c.type_name(), c.len())).collect()
    }

    /// every column in ascending entity order (SparseSet::sort): the canonical order of iteration, the
    /// same for a world rebuilt from a snapshot as for the one it came from
    pub fn sort_columns(&mut self) {
        for c in &mut self.columns {
            c.sort();
        }
    }

    /// the start of a tick, before its systems (Game::step calls it): the tick count goes up, and every
    /// Pose written in the tick before gets that state as its previous one (prev = now), the state the
    /// next frames interpolate from.  Only those poses are touched: a scan of one version per pose
    pub fn begin_tick(&mut self) {
        self.tick += 1;
        let mark = self.pose_mark;
        if let Some(i) = self.find(TypeId::of::<Pose>()) {
            let poses = World::typed_mut::<Pose>(&mut *self.columns[i]);
            crate::pose::settle(poses, mark);
            self.pose_mark = poses.version();
        }
    }

    /// ticks begun so far (begin_tick)
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// the Pose column's version when the systems of the latest tick began: poses with a newer version
    /// moved in that tick and are between two states (the bridge to render3d writes them every frame)
    pub fn pose_mark(&self) -> u64 {
        self.pose_mark
    }
}
