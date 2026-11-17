// store -- (G7f) what removing meshes, materials and textures needs (docs/RENDER3D.md "Entfernen"): ids with a generation,
// the table of live ids and their users that a Renderer shares with its scenes, and a pool buffer that hands out ranges
// and takes them back.
//
// Ids: the low 24 bits are a slot (an index into the renderer's tables), the high 8 bits count how often the slot was
// used before.  A removed id stays invalid: its slot is used again with the next count, and a slot whose count is spent
// (255 uses) is never used again.  A renderer that removes nothing hands out 0, 1, 2, ... as before.
//
// Users: a scene counts its instances per mesh and per material itself; the shared table (under a spin lock) counts the
// scenes that have at least one, and says which id is alive in each slot.  So Scene::add checks an id at once and takes
// the lock only for the first instance of a mesh or material in that scene, and the renderer refuses to remove what a
// scene still uses.  A shared table that grows needs a lock without std and unsafe; rt's spin lock is the smallest.

use alloc::vec::Vec;

use gpu::*;
use rt::sync::SpinLock;

/// bits of an id that name its slot
pub(crate) const SLOT_BITS: u32 = 24;
pub(crate) const SLOT_MASK: u32 = (1 << SLOT_BITS) - 1;
/// the last generation of a slot (it is not used again after that)
const GEN_LAST: u32 = 255;
/// a slot whose object was removed
pub(crate) const DEAD: u32 = u32::MAX;

/// the slot of an id
pub(crate) const fn slot(id: u32) -> usize {
    (id & SLOT_MASK) as usize
}

/// the ids of one kind of object: per slot its id (the last one when dead) and whether it is alive; the free slots
pub(crate) struct Ids {
    ids: Vec<u32>,
    live: Vec<bool>,
    free: Vec<u32>,
    count: u32,
}

impl Ids {
    pub(crate) const fn new() -> Ids {
        Ids { ids: Vec::new(), live: Vec::new(), free: Vec::new(), count: 0 }
    }

    /// the slot of a live id
    pub(crate) fn get(&self, id: u32) -> Option<usize> {
        let s = slot(id);
        (self.live.get(s) == Some(&true) && self.ids[s] == id).then_some(s)
    }

    /// the id of a slot (the last one when it is dead)
    pub(crate) fn id(&self, s: usize) -> u32 {
        self.ids[s]
    }

    /// a slot for a new object, one that came free first, and its id; None: no slot left
    pub(crate) fn make(&mut self) -> Option<(usize, u32)> {
        let s = match self.free.pop() {
            Some(s) => {
                let s = s as usize;
                self.ids[s] = (s as u32) | ((self.ids[s] >> SLOT_BITS) + 1) << SLOT_BITS;
                s
            }
            None => {
                let s = self.ids.len();
                if s > SLOT_MASK as usize {
                    return None;
                }
                self.ids.push(s as u32);
                self.live.push(false);
                s
            }
        };
        self.live[s] = true;
        self.count += 1;
        Some((s, self.ids[s]))
    }

    /// the slot's object is gone; the slot comes free unless its generations are spent
    pub(crate) fn kill(&mut self, s: usize) {
        self.live[s] = false;
        self.count -= 1;
        if self.ids[s] >> SLOT_BITS < GEN_LAST {
            self.free.push(s as u32);
        }
    }

    /// live objects
    pub(crate) fn count(&self) -> u32 {
        self.count
    }
}

/// a slot of the shared table: its live id (DEAD when removed) and the scenes that have instances of it
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub(crate) id: u32,
    pub(crate) scenes: u32,
}

/// the meshes and materials of a renderer as its scenes see them
pub(crate) struct Registry {
    pub(crate) meshes: Vec<Entry>,
    pub(crate) materials: Vec<Entry>,
}

/// what a Renderer shares with its scenes (they know by this object whose they are)
pub(crate) struct Counts {
    pub(crate) reg: SpinLock<Registry>,
}

impl Counts {
    pub(crate) const fn new() -> Counts {
        Counts { reg: SpinLock::new(Registry { meshes: Vec::new(), materials: Vec::new() }) }
    }
}

/// which of the table's two kinds
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mesh,
    Material,
}

impl Registry {
    pub(crate) fn of(&mut self, k: Kind) -> &mut Vec<Entry> {
        match k {
            Kind::Mesh => &mut self.meshes,
            Kind::Material => &mut self.materials,
        }
    }

    /// the slot of an id is alive with it (the renderer made or removed it)
    pub(crate) fn set(&mut self, k: Kind, s: usize, id: u32) {
        let v = self.of(k);
        if v.len() <= s {
            v.resize(s + 1, Entry { id: DEAD, scenes: 0 });
        }
        v[s].id = id;
    }
}

/// a scene's instances of the mesh (or material) in a slot, and its id
#[derive(Clone, Copy, Default)]
pub(crate) struct Use {
    pub(crate) id: u32,
    pub(crate) n: u32,
}

/// one more instance of id in a scene: Invalid when the id is not alive (at once: the first instance of a slot asks the
/// shared table, the next ones know it from the scene's own count)
pub(crate) fn take(counts: &Counts, uses: &mut Vec<Use>, k: Kind, id: u32) -> Result<()> {
    let s = slot(id);
    let bad = || match k {
        Kind::Mesh => GpuError::Invalid("Scene: a mesh its Renderer does not have"),
        Kind::Material => GpuError::Invalid("Scene: a material its Renderer does not have"),
    };
    if let Some(u) = uses.get_mut(s) {
        if u.n > 0 {
            // (an object in use cannot be removed: the slot still holds it)
            return if u.id == id {
                u.n += 1;
                Ok(())
            } else {
                Err(bad())
            };
        }
    }
    {
        let mut reg = counts.reg.lock();
        match reg.of(k).get_mut(s) {
            Some(e) if e.id == id && id != DEAD => e.scenes += 1,
            _ => return Err(bad()),
        }
    }
    if uses.len() <= s {
        uses.resize(s + 1, Use::default());
    }
    uses[s] = Use { id, n: 1 };
    Ok(())
}

/// one instance of id less in a scene (it took it)
pub(crate) fn give(counts: &Counts, uses: &mut [Use], k: Kind, id: u32) {
    let s = slot(id);
    let Some(u) = uses.get_mut(s) else { return };
    if u.n == 0 {
        return;
    }
    u.n -= 1;
    if u.n == 0 {
        let mut reg = counts.reg.lock();
        if let Some(e) = reg.of(k).get_mut(s) {
            e.scenes = e.scenes.saturating_sub(1);
        }
    }
}

/// every instance of a scene that goes (Scene's drop)
pub(crate) fn give_all(counts: &Counts, meshes: &[Use], materials: &[Use]) {
    let mut reg = counts.reg.lock();
    for (k, uses) in [(Kind::Mesh, meshes), (Kind::Material, materials)] {
        let v = reg.of(k);
        for (s, u) in uses.iter().enumerate() {
            if u.n > 0 {
                if let Some(e) = v.get_mut(s) {
                    e.scenes = e.scenes.saturating_sub(1);
                }
            }
        }
    }
}

/// one growing buffer of a pool: a new one takes over, the GPU copies the old contents across.  (G7f) Ranges come back
/// (release) and are handed out again, the best fitting first; a range at the end lowers the end.  Nothing is moved
/// (no compaction: no measurement asked for it)
pub(crate) struct Pool {
    pub(crate) buf: Buffer,
    usage: BufferUsage,
    /// bytes per element
    size: u64,
    pub(crate) used: u32,
    pub(crate) cap: u32,
    /// free ranges (first, count) below `used`, by first, none touching another
    free: Vec<(u32, u32)>,
}

impl Pool {
    pub(crate) fn new(dev: &Device, usage: BufferUsage, size: u64, cap: u32) -> Result<Pool> {
        let buf = dev.create_buffer(&BufferDesc { size: size * cap as u64, usage, memory: MemoryKind::Device })?;
        Ok(Pool { buf, usage, size, used: 0, cap, free: Vec::new() })
    }

    /// n elements fit without growing
    pub(crate) fn fits(&self, n: u32) -> bool {
        self.free.iter().any(|r| r.1 >= n) || self.used.checked_add(n).is_some_and(|e| e <= self.cap)
    }

    /// the first of n elements: the free range that fits best, else at the end (growing when needed)
    pub(crate) fn alloc(&mut self, dev: &Device, n: u32) -> Result<u32> {
        let best = self.free.iter().enumerate().filter(|(_, r)| r.1 >= n).min_by_key(|(_, r)| r.1).map(|(k, _)| k);
        if let Some(k) = best {
            let (first, len) = self.free[k];
            if len == n {
                self.free.remove(k);
            } else {
                self.free[k] = (first + n, len - n);
            }
            return Ok(first);
        }
        self.reserve(dev, n)?;
        let first = self.used;
        self.used += n;
        Ok(first)
    }

    /// n elements from first are free again (merged with their neighbours; at the end, the end moves back)
    pub(crate) fn release(&mut self, first: u32, n: u32) {
        if n == 0 {
            return;
        }
        let k = self.free.partition_point(|r| r.0 < first);
        self.free.insert(k, (first, n));
        // with the one after, then the one before
        if k + 1 < self.free.len() && self.free[k].0 + self.free[k].1 == self.free[k + 1].0 {
            self.free[k].1 += self.free[k + 1].1;
            self.free.remove(k + 1);
        }
        if k > 0 && self.free[k - 1].0 + self.free[k - 1].1 == self.free[k].0 {
            self.free[k - 1].1 += self.free[k].1;
            self.free.remove(k);
        }
        if let Some(&(f, l)) = self.free.last() {
            if f + l == self.used {
                self.used = f;
                self.free.pop();
            }
        }
    }

    /// elements in free ranges below the end
    pub(crate) fn holes(&self) -> u32 {
        self.free.iter().map(|r| r.1).sum()
    }

    /// room for n more elements at the end.  Uploads still on their way run first (they lead the next submission),
    /// so the copy has them; the old buffer goes when the GPU is done with it.
    fn reserve(&mut self, dev: &Device, n: u32) -> Result<()> {
        let need = self.used.checked_add(n).ok_or(GpuError::Invalid("the mesh pool is full"))?;
        if need <= self.cap {
            return Ok(());
        }
        let cap = need.max(self.cap.saturating_mul(2));
        let buf = dev.create_buffer(&BufferDesc { size: self.size * cap as u64, usage: self.usage, memory: MemoryKind::Device })?;
        if self.used > 0 {
            let mut e = dev.encoder()?;
            e.barrier(Access::COPY_WRITE, Access::COPY_READ);
            e.copy_buffer(&self.buf, 0, &buf, 0, self.size * self.used as u64);
            e.barrier(Access::COPY_WRITE, Access::COPY_READ | Access::COPY_WRITE | Access::INDEX_VERTEX | Access::GRAPHICS_READ);
            dev.queue().submit([e.finish()?])?;
        }
        self.buf = buf;
        self.cap = cap;
        Ok(())
    }
}
