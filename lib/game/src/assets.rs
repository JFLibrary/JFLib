// assets -- (G7b part 2) files through the VFS, decoded in the background, behind handles with states, reference
// counts and unloading (docs/GAMECORE.md "Assets")
//
//   impl Asset for T               decode(bytes, files) -> Result<T, String>: a file's bytes into the value; files
//                                  reads the files beside it by relative name (a .gltf's buffers and images).  It
//                                  runs on a worker thread (Send); lib/game_render3d has glTF models and images
//   Assets::<T>::new(Loading)      a store of one kind: Background (plat::task::spawn_blocking: threads where the
//                                  target has them), Frames { budget_us } (decoded in update(), one after the other
//                                  until the budget is spent, at least one per update: the browser without
//                                  threads, deterministic tests)
//   assets.load(path) -> Handle<T> the asset of path (app://, data://, mem://, a file ...): the same entry for the
//                                  same path while it lives (another handle), else a new one, Loading
//   assets.insert(name, value)     an asset made in code, Ready at once
//   assets.state(&h) / get(&h) / error(&h) / path(&h) / refs(&h) / id()
//   assets.update() -> Vec<Change> finished decodes become Ready or Failed; assets nobody holds a handle of (for
//                                  `linger` updates) are unloaded: their value goes, their path is forgotten;
//                                  what happened, for those who keep something per asset (the GPU's copies)
//   Handle<T>                      a reference: Clone counts one more, Drop one less; AssetId (index, generation)
//                                  names the entry, also after it is gone (a stale id finds nothing)
//
// The simulation does not depend on assets (they are what a frame shows and plays): with threads the moment an
// asset becomes Ready depends on the machine.  Everything else in lib/game stays deterministic; a test that needs
// the same frames on every target loads with Loading::Frames.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

/// a kind of asset: how a file's bytes become its value (on a worker thread)
pub trait Asset: Sized + Send + 'static {
    /// decode `bytes` (the file at the handle's path); `files` reads a file beside it by a relative name (None: none)
    fn decode(bytes: &[u8], files: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<Self, String>;
}

/// the raw bytes of a file (an Asset for anything a game decodes itself)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Asset for Bytes {
    fn decode(bytes: &[u8], _files: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<Self, String> {
        Ok(Bytes(bytes.to_vec()))
    }
}

/// how decoding runs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loading {
    /// on the blocking pool of plat::task (threads; where none can start, the executor runs the jobs between
    /// frames, one per run: lib/ui does)
    Background,
    /// in update(), one asset after the other until budget_us is spent (at least one per update)
    Frames { budget_us: u64 },
}

impl Loading {
    /// Background on targets with threads, Frames { 4 ms } in the browser (a bundle without shared memory has none)
    pub fn default_for_target() -> Loading {
        if cfg!(target_arch = "wasm32") { Loading::Frames { budget_us: 4_000 } } else { Loading::Background }
    }
}

/// where an asset is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// being read and decoded
    Loading,
    /// decoded: get() gives it
    Ready,
    /// not read or not decoded: error() says why
    Failed,
}

/// an asset's entry: its index and generation (a reused index has a new one)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId {
    pub index: u32,
    pub generation: u32,
}

/// what update() did to an asset
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Ready(AssetId),
    Failed(AssetId),
    /// gone: nobody held a handle (its id finds nothing from now on)
    Unloaded(AssetId),
}

// what a handle shares with its entry (the count of handles is the count of these)
struct Key {
    id: AssetId,
}

/// a reference to an asset of kind T: while one lives the asset stays (Clone: one more, Drop: one less)
pub struct Handle<T> {
    key: Rc<Key>,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Handle { key: self.key.clone(), _t: PhantomData }
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, o: &Self) -> bool {
        self.key.id == o.key.id
    }
}

impl<T> Eq for Handle<T> {}

impl<T> core::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Handle({}, {})", self.key.id.index, self.key.id.generation)
    }
}

impl<T> Handle<T> {
    /// the entry it refers to
    pub fn id(&self) -> AssetId {
        self.key.id
    }
}

type Job<T> = plat::task::Blocking<Result<T, String>>;

enum Slot<T> {
    /// waiting for its decode: Background's job, or Frames' turn (None)
    Loading(Option<Job<T>>),
    Ready(T),
    Failed(String),
}

struct Entry<T> {
    path: String,
    generation: u32,
    key: Weak<Key>,
    slot: Slot<T>,
    /// updates without a handle so far
    idle: u32,
}

/// what a store holds now
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetStats {
    pub loading: u32,
    pub ready: u32,
    pub failed: u32,
    /// decodes finished, unloads so far
    pub decoded: u64,
    pub unloaded: u64,
}

/// the assets of one kind (see the module header)
pub struct Assets<T: Asset> {
    entries: Vec<Option<Entry<T>>>,
    /// the generation of each index's last entry (a new one counts on)
    generations: Vec<u32>,
    free: Vec<u32>,
    paths: BTreeMap<String, u32>,
    mode: Loading,
    /// Frames: the entries waiting for their decode, in the order they were asked for
    queue: VecDeque<u32>,
    linger: u32,
    decoded: u64,
    unloaded: u64,
}

impl<T: Asset> Default for Assets<T> {
    fn default() -> Self {
        Assets::new(Loading::default_for_target())
    }
}

/// path's folder (with its slash) joined with a relative name; None for one that leaves the folder or names a scheme
pub fn sibling(path: &str, rel: &str) -> Option<String> {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') || rel.contains("://") || rel.split(['/', '\\']).any(|p| p == "..") {
        return None;
    }
    let dir = match path.rfind(['/', '\\']) {
        Some(i) => &path[..=i],
        None => "",
    };
    Some(format!("{dir}{rel}"))
}

/// read path through the VFS and decode it (the job of a load)
fn read_decode<T: Asset>(path: &str) -> Result<T, String> {
    let bytes = plat::read_file(path).ok_or_else(|| format!("{path}: not readable"))?;
    let mut files = |rel: &str| sibling(path, rel).and_then(|p| plat::read_file(&p));
    T::decode(&bytes, &mut files).map_err(|e| format!("{path}: {e}"))
}

/// a finished job's value without waiting (None: still running)
fn poll_job<T>(job: &mut plat::task::Blocking<T>) -> Option<T> {
    if !job.is_done() {
        return None;
    }
    let mut cx = Context::from_waker(Waker::noop());
    match Pin::new(job).poll(&mut cx) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    }
}

impl<T: Asset> Assets<T> {
    /// an empty store that decodes as `mode` says
    pub fn new(mode: Loading) -> Assets<T> {
        Assets {
            entries: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            paths: BTreeMap::new(),
            mode,
            queue: VecDeque::new(),
            linger: 0,
            decoded: 0,
            unloaded: 0,
        }
    }

    /// how it decodes
    pub fn mode(&self) -> Loading {
        self.mode
    }

    /// an asset nobody holds stays this many updates before it is unloaded (0, the default: the next update
    /// unloads it; a load of its path before then takes it again, without reading it anew)
    pub fn set_linger(&mut self, updates: u32) {
        self.linger = updates;
    }

    fn entry(&self, id: AssetId) -> Option<&Entry<T>> {
        match self.entries.get(id.index as usize) {
            Some(Some(e)) if e.generation == id.generation => Some(e),
            _ => None,
        }
    }

    fn new_entry(&mut self, path: String, slot: Slot<T>) -> Handle<T> {
        let index = match self.free.pop() {
            Some(i) => i,
            None => {
                self.entries.push(None);
                self.generations.push(0);
                (self.entries.len() - 1) as u32
            }
        };
        let generation = self.generations[index as usize].wrapping_add(1);
        self.generations[index as usize] = generation;
        let key = Rc::new(Key { id: AssetId { index, generation } });
        if !path.is_empty() {
            self.paths.insert(path.clone(), index);
        }
        self.entries[index as usize] = Some(Entry { path, generation, key: Rc::downgrade(&key), slot, idle: 0 });
        Handle { key, _t: PhantomData }
    }

    /// the asset of `path`: another handle of its entry while it lives, else a new entry that starts loading
    pub fn load(&mut self, path: &str) -> Handle<T> {
        if let Some(&i) = self.paths.get(path) {
            if let Some(Some(e)) = self.entries.get_mut(i as usize) {
                e.idle = 0;
                let key = match e.key.upgrade() {
                    Some(k) => k,
                    None => {
                        // (its last handle went, the unload has not come yet: it lives on)
                        let k = Rc::new(Key { id: AssetId { index: i, generation: e.generation } });
                        e.key = Rc::downgrade(&k);
                        k
                    }
                };
                return Handle { key, _t: PhantomData };
            }
        }
        let slot = match self.mode {
            Loading::Background => {
                let p = String::from(path);
                Slot::Loading(Some(plat::task::spawn_blocking(move || read_decode::<T>(&p))))
            }
            Loading::Frames { .. } => Slot::Loading(None),
        };
        let h = self.new_entry(String::from(path), slot);
        if matches!(self.mode, Loading::Frames { .. }) {
            self.queue.push_back(h.id().index);
        }
        h
    }

    /// an asset made in code (no file): Ready at once; `name` is its path for path() (it is not looked up by load)
    pub fn insert(&mut self, name: &str, value: T) -> Handle<T> {
        let h = self.new_entry(String::new(), Slot::Ready(value));
        if let Some(Some(e)) = self.entries.get_mut(h.id().index as usize) {
            e.path = String::from(name);
        }
        h
    }

    /// where the asset is (None: a handle of another store, which cannot happen through the API)
    pub fn state(&self, h: &Handle<T>) -> State {
        self.state_of(h.id()).unwrap_or(State::Failed)
    }

    /// the state of an entry by its id (None: gone)
    pub fn state_of(&self, id: AssetId) -> Option<State> {
        self.entry(id).map(|e| match e.slot {
            Slot::Loading(_) => State::Loading,
            Slot::Ready(_) => State::Ready,
            Slot::Failed(_) => State::Failed,
        })
    }

    /// the value once Ready
    pub fn get(&self, h: &Handle<T>) -> Option<&T> {
        self.get_id(h.id())
    }

    /// the value of an entry by its id (None: not ready, failed or gone)
    pub fn get_id(&self, id: AssetId) -> Option<&T> {
        match self.entry(id) {
            Some(Entry { slot: Slot::Ready(v), .. }) => Some(v),
            _ => None,
        }
    }

    /// why it failed
    pub fn error(&self, h: &Handle<T>) -> Option<&str> {
        match self.entry(h.id()) {
            Some(Entry { slot: Slot::Failed(e), .. }) => Some(e),
            _ => None,
        }
    }

    /// the path it was loaded from (an inserted one: its name)
    pub fn path(&self, h: &Handle<T>) -> &str {
        self.entry(h.id()).map_or("", |e| &e.path)
    }

    /// handles of the asset that live now
    pub fn refs(&self, h: &Handle<T>) -> u32 {
        self.refs_of(h.id())
    }

    /// handles of an entry that live now (0: gone, or held by nobody)
    pub fn refs_of(&self, id: AssetId) -> u32 {
        self.entry(id).map_or(0, |e| e.key.strong_count() as u32)
    }

    /// entries (every state)
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// what the store holds
    pub fn stats(&self) -> AssetStats {
        let mut s = AssetStats { decoded: self.decoded, unloaded: self.unloaded, ..AssetStats::default() };
        for e in self.entries.iter().flatten() {
            match e.slot {
                Slot::Loading(_) => s.loading += 1,
                Slot::Ready(_) => s.ready += 1,
                Slot::Failed(_) => s.failed += 1,
            }
        }
        s
    }

    /// the ids of every entry that lives, in index order
    pub fn ids(&self) -> Vec<AssetId> {
        self.entries.iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|e| AssetId { index: i as u32, generation: e.generation })).collect()
    }

    /// read and decode the asset again (a file that changed): Loading until then; its value stays until the new
    /// one has come (a failed reload keeps the old value and reports Failed only when there was none)
    pub fn reload(&mut self, h: &Handle<T>) {
        let i = h.id().index;
        let mode = self.mode;
        let Some(Some(e)) = self.entries.get_mut(i as usize) else { return };
        if e.generation != h.id().generation || e.path.is_empty() || matches!(e.slot, Slot::Loading(_)) {
            return;
        }
        match mode {
            Loading::Background => {
                let p = e.path.clone();
                e.slot = Slot::Loading(Some(plat::task::spawn_blocking(move || read_decode::<T>(&p))));
            }
            Loading::Frames { .. } => {
                e.slot = Slot::Loading(None);
                self.queue.push_back(i);
            }
        }
    }

    /// once a frame: decodes that finished (Frames: decoded now, within the budget) become Ready or Failed; assets
    /// nobody holds are unloaded -> what happened
    pub fn update(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        // ---- finished jobs
        for i in 0..self.entries.len() {
            let Some(e) = self.entries[i].as_mut() else { continue };
            if let Slot::Loading(Some(job)) = &mut e.slot {
                if let Some(r) = poll_job(job) {
                    let id = AssetId { index: i as u32, generation: e.generation };
                    self.decoded += 1;
                    match r {
                        Ok(v) => {
                            e.slot = Slot::Ready(v);
                            out.push(Change::Ready(id));
                        }
                        Err(err) => {
                            e.slot = Slot::Failed(err);
                            out.push(Change::Failed(id));
                        }
                    }
                }
            }
        }
        // ---- Frames: decode here, the oldest first, until the budget is spent
        if let Loading::Frames { budget_us } = self.mode {
            let t0 = plat::time_us();
            let mut first = true;
            while let Some(i) = self.queue.pop_front() {
                if !first && plat::time_us().saturating_sub(t0) >= budget_us {
                    self.queue.push_front(i);
                    break;
                }
                let Some(e) = self.entries[i as usize].as_mut() else { continue };
                if !matches!(e.slot, Slot::Loading(None)) {
                    continue;
                }
                // (nobody holds it any more: no decode, the unload below takes it)
                if e.key.strong_count() == 0 {
                    continue;
                }
                first = false;
                let id = AssetId { index: i, generation: e.generation };
                self.decoded += 1;
                match read_decode::<T>(&e.path) {
                    Ok(v) => {
                        e.slot = Slot::Ready(v);
                        out.push(Change::Ready(id));
                    }
                    Err(err) => {
                        e.slot = Slot::Failed(err);
                        out.push(Change::Failed(id));
                    }
                }
            }
        }
        // ---- unloading: entries without a handle for more than `linger` updates
        for i in 0..self.entries.len() {
            let Some(e) = self.entries[i].as_mut() else { continue };
            if e.key.strong_count() != 0 {
                e.idle = 0;
                continue;
            }
            if e.idle < self.linger {
                e.idle += 1;
                continue;
            }
            let id = AssetId { index: i as u32, generation: e.generation };
            if self.paths.get(&e.path) == Some(&(i as u32)) {
                self.paths.remove(&e.path);
            }
            // (a job still running is cancelled: its Blocking goes, a job not started never starts)
            self.entries[i] = None;
            self.free.push(i as u32);
            self.unloaded += 1;
            out.push(Change::Unloaded(id));
        }
        out
    }

    /// Background: wait until nothing is loading (at most timeout_us; tests, a loading screen) -> nothing is
    pub fn wait_idle(&mut self, timeout_us: u64, changes: &mut Vec<Change>) -> bool {
        let t0 = plat::time_us();
        loop {
            changes.extend(self.update());
            if self.stats().loading == 0 {
                return true;
            }
            if plat::time_us().saturating_sub(t0) >= timeout_us {
                return false;
            }
            plat::sleep_ms(1);
        }
    }
}
