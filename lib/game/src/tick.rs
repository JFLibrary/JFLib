// tick -- the fixed simulation rate: an accumulator over time that comes from outside, a cap on the
// steps a frame may catch up, the alpha of the frame between two ticks; systems as functions run in a
// fixed order; Game ties world, systems and clock together
//
//   FixedTick         rate (60 Hz unless set), advance_to(now_ns) / advance_by(elapsed_ns) -> steps
//                     due, alpha() in [0, 1), at most max_steps (8 unless set) per call: the rest of a
//                     long frame is dropped (dropped_ns), reset() forgets the clock (after a pause)
//   Step              what a system gets: the tick's number and dt (1 / rate seconds)
//   Systems<R>        functions fn(&mut World, &mut R, &Step) in the order they were added
//   Game<R>           a World, the resources R (everything that is not a component: the physics
//                     world, the input of the tick, the map), the Systems and the FixedTick;
//                     frame(now_ns) runs the steps due, step() one step now
//
// Time is counted exactly: the accumulator holds nanoseconds times the rate, and a step takes
// 10^9 of it, so 60 Hz loses nothing to the 16.666... ms of a tick (no float sums, no drift after
// hours).  The clock comes from outside (plat::now_us in a window, follows the script's virtual clock
// in a headless run; a test passes its own numbers), so a test runs the same ticks every time.

use alloc::vec::Vec;

use crate::world::World;

/// the simulation rate unless set (Hz)
pub const DEFAULT_RATE: u32 = 60;
/// the steps one call may catch up unless set (8 ticks at 60 Hz: 133 ms of a frame that hung)
pub const DEFAULT_MAX_STEPS: u32 = 8;
const NS: u128 = 1_000_000_000;

/// the fixed rate's accumulator (see the module header)
#[derive(Clone, Debug)]
pub struct FixedTick {
    rate: u32,
    max_steps: u32,
    /// nanoseconds times the rate, below 10^9 after every call
    acc: u128,
    /// the clock of the last advance_to
    last: Option<u64>,
    dropped_ns: u64,
}

impl Default for FixedTick {
    fn default() -> Self {
        FixedTick::new(DEFAULT_RATE)
    }
}

impl FixedTick {
    /// `rate` ticks a second (1 to 10 000), at most DEFAULT_MAX_STEPS per call
    pub fn new(rate: u32) -> FixedTick {
        FixedTick { rate: rate.clamp(1, 10_000), max_steps: DEFAULT_MAX_STEPS, acc: 0, last: None, dropped_ns: 0 }
    }

    /// the same with another cap on the steps one call may return (at least 1)
    pub fn with_max_steps(mut self, n: u32) -> FixedTick {
        self.max_steps = n.max(1);
        self
    }

    /// ticks a second
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// the cap on the steps of one call
    pub fn max_steps(&self) -> u32 {
        self.max_steps
    }

    /// the length of a tick in seconds
    pub fn dt(&self) -> f64 {
        1.0 / self.rate as f64
    }

    /// the clock stands at now_ns (any monotonic clock in nanoseconds) -> the steps due.  The first call
    /// only starts the clock (0); a clock that goes back counts as standing still
    pub fn advance_to(&mut self, now_ns: u64) -> u32 {
        let elapsed = match self.last {
            Some(t) => now_ns.saturating_sub(t),
            None => 0,
        };
        if self.last.is_none_or(|t| now_ns > t) {
            self.last = Some(now_ns);
        }
        self.advance_by(elapsed)
    }

    /// elapsed_ns more have passed -> the steps due (at most max_steps; the time beyond them is dropped,
    /// what is left of a step stays for the next call and the alpha)
    pub fn advance_by(&mut self, elapsed_ns: u64) -> u32 {
        self.acc += elapsed_ns as u128 * self.rate as u128;
        let due = self.acc / NS;
        self.acc %= NS;
        if due > self.max_steps as u128 {
            let over = due - self.max_steps as u128;
            self.dropped_ns = self.dropped_ns.saturating_add((over * NS / self.rate as u128).min(u64::MAX as u128) as u64);
            return self.max_steps;
        }
        due as u32
    }

    /// how far the clock is into the next tick, in [0, 1): the alpha a frame interpolates with
    /// (Pose::interpolate)
    pub fn alpha(&self) -> f64 {
        self.acc as f64 / NS as f64
    }

    /// the time the cap has dropped so far (frames that hung longer than max_steps ticks)
    pub fn dropped_ns(&self) -> u64 {
        self.dropped_ns
    }

    /// forget the clock and the part of a tick: the next advance_to starts again (after a pause, so that
    /// the time paused is not caught up)
    pub fn reset(&mut self) {
        self.acc = 0;
        self.last = None;
    }
}

/// what a system gets for its tick
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// the tick's number (World::tick: 1 for the first)
    pub tick: u64,
    /// its length in seconds (1 / rate)
    pub dt: f64,
}

/// a system: a function over the world and the game's resources, once per tick
pub type System<R> = fn(&mut World, &mut R, &Step);

/// systems run in the order they were added (no scheduler: the order is the program's)
pub struct Systems<R> {
    list: Vec<(&'static str, System<R>)>,
}

impl<R> Default for Systems<R> {
    fn default() -> Self {
        Systems::new()
    }
}

impl<R> Systems<R> {
    /// none
    pub const fn new() -> Systems<R> {
        Systems { list: Vec::new() }
    }

    /// a system after the others (the name is for lists and messages)
    pub fn add(&mut self, name: &'static str, f: System<R>) -> &mut Systems<R> {
        self.list.push((name, f));
        self
    }

    /// a system right before the one named `before` (at the end without it)
    pub fn add_before(&mut self, before: &str, name: &'static str, f: System<R>) -> &mut Systems<R> {
        let at = self.list.iter().position(|(n, _)| *n == before).unwrap_or(self.list.len());
        self.list.insert(at, (name, f));
        self
    }

    /// the system named so goes (false: none)
    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.list.len();
        self.list.retain(|(x, _)| *x != name);
        self.list.len() != n
    }

    /// every system once, in order
    pub fn run(&self, world: &mut World, res: &mut R, step: &Step) {
        for (_, f) in &self.list {
            f(world, res, step);
        }
    }

    /// their names in order
    pub fn names(&self) -> Vec<&'static str> {
        self.list.iter().map(|(n, _)| *n).collect()
    }

    /// how many
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// none
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// a world, the game's resources, its systems and its clock (see the module header)
pub struct Game<R> {
    /// the entities and their components
    pub world: World,
    /// everything else the systems use
    pub res: R,
    /// what runs every tick, in order
    pub systems: Systems<R>,
    /// the fixed rate
    pub clock: FixedTick,
}

impl<R> Game<R> {
    /// an empty world with these resources, no systems, 60 Hz
    pub fn new(res: R) -> Game<R> {
        Game { world: World::new(), res, systems: Systems::new(), clock: FixedTick::default() }
    }

    /// the clock stands at now_ns: the steps due run (FixedTick::advance_to) -> how many ran.  Then
    /// alpha() says where the frame lies between the last two states
    pub fn frame(&mut self, now_ns: u64) -> u32 {
        let n = self.clock.advance_to(now_ns);
        for _ in 0..n {
            self.step();
        }
        n
    }

    /// one tick now, whatever the clock says: World::begin_tick, then every system
    pub fn step(&mut self) {
        self.world.begin_tick();
        let step = Step { tick: self.world.tick(), dt: self.clock.dt() };
        self.systems.run(&mut self.world, &mut self.res, &step);
    }

    /// the frame's place between the last two ticks (FixedTick::alpha)
    pub fn alpha(&self) -> f64 {
        self.clock.alpha()
    }
}
