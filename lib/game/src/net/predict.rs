// predict -- the client predicts its own figure: every tick it applies its input at once, and when the
// server's state for a tick comes it compares; on a difference it goes back to the server's state and
// applies its inputs since then again (reconciliation)
//
//   Movement      a figure's movement: the tick's input in, the next state out.  The same code runs on the
//                 server and in the prediction, so it must give the same bits: f64 with + - * /, the square
//                 root and the trigonometry of lib/physics::math, no mul_add, no unordered containers, no
//                 time; and its state must stay on the grid of quant.rs (it is replicated as integers, and
//                 the server's state must be what the snapshot says).  Whatever the step needs from the past
//                 (the buttons held before, for edges) belongs in the state.  The reference is
//                 walker::Walker; G7h's character controller takes its place
//   Predictor     the inputs and predicted states since the newest confirmed tick; step, reconcile, the
//                 current and previous state (a frame blends the two by the tick's alpha)
//   Smoother      what a frame draws after a correction: the jump of the predicted position becomes an offset
//                 that decays in a short time (and snaps above a distance); the predicted state stays exact
//
// A prediction is exact when the server used the client's own inputs: reconcile then finds the same
// state and only drops history.  It differs after a missed input (the server repeated the last one), a
// teleport, damage or anything else the server did to the figure; then the replay puts the figure where
// the server's state and the client's later inputs lead.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use physics::Vec3;

use super::input::NetInput;
use super::snapshot::tick_diff;

/// a figure's movement (see the module header)
pub trait Movement {
    /// the movement state (on the grid; Default: before the server said where the figure is)
    type State: Copy + PartialEq + Default + core::fmt::Debug;
    /// the fields of the state at the start of the figure's replicated entity
    const FIELDS: usize;
    /// the state after one tick of `input` (dt: the tick's length in seconds)
    fn step(&self, map: &physics::World, state: &Self::State, input: &NetInput, dt: f64) -> Self::State;
    /// the state as FIELDS integers (lossless)
    fn to_fields(state: &Self::State, out: &mut [i32]);
    /// the state of FIELDS integers
    fn from_fields(f: &[i32]) -> Self::State;
    /// where the figure is (the error measure, the camera)
    fn position(state: &Self::State) -> Vec3;
}

/// what reconcile found
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// the server's state is older than one already confirmed (a late snapshot)
    Stale,
    /// the prediction for that tick was exact
    Match,
    /// it differed by `error` metres there; the replay moved the current state by `shift` metres
    Corrected { error: f64, shift: f64 },
    /// there was no prediction for that tick (the first state, or history ran out): started from it
    Reset,
}

/// what a predictor counted
#[derive(Clone, Debug, Default)]
pub struct PredictStats {
    /// ticks predicted
    pub steps: u64,
    /// server states compared
    pub checks: u64,
    /// of those, different (Corrected)
    pub mismatches: u64,
    /// starts without a prediction to compare (Reset)
    pub resets: u64,
    /// ticks applied again in replays
    pub replayed: u64,
    /// the error of every mismatch in metres (for percentiles; the first 100 000)
    pub errors: Vec<f32>,
    /// the shift of the current state of every mismatch
    pub shifts: Vec<f32>,
}

/// the history of a prediction (see the module header)
pub struct Predictor<M: Movement> {
    hist: VecDeque<(u32, NetInput, M::State)>,
    base: Option<(u32, M::State)>,
    cap: usize,
    stats: PredictStats,
}

impl<M: Movement> Default for Predictor<M> {
    fn default() -> Self {
        Predictor::new()
    }
}

impl<M: Movement> Predictor<M> {
    /// nothing predicted; history of up to 256 ticks
    pub fn new() -> Predictor<M> {
        Predictor { hist: VecDeque::new(), base: None, cap: 256, stats: PredictStats::default() }
    }

    /// whether a server state has come (before it the figure stands at the default state)
    pub fn known(&self) -> bool {
        self.base.is_some()
    }

    /// predict `tick` with `input` -> the state after it
    pub fn step(&mut self, mover: &M, map: &physics::World, tick: u32, input: NetInput, dt: f64) -> M::State {
        let prev = self.current_or_base();
        let s = if self.base.is_some() { mover.step(map, &prev, &input, dt) } else { prev };
        self.hist.push_back((tick, input, s));
        if self.hist.len() > self.cap {
            self.hist.pop_front();
        }
        self.stats.steps += 1;
        s
    }

    fn current_or_base(&self) -> M::State {
        match (self.hist.back(), self.base) {
            (Some(h), _) => h.2,
            (None, Some(b)) => b.1,
            _ => M::State::default(),
        }
    }

    /// the newest predicted state (the base without history)
    pub fn current(&self) -> Option<M::State> {
        self.base.map(|_| self.current_or_base())
    }

    /// the state one tick before the current one (for the alpha of a frame)
    pub fn previous(&self) -> Option<M::State> {
        self.base?;
        let n = self.hist.len();
        if n >= 2 {
            Some(self.hist[n - 2].2)
        } else {
            self.base.map(|b| b.1)
        }
    }

    /// the predicted state of `tick`, if still in the history
    pub fn at(&self, tick: u32) -> Option<M::State> {
        self.hist.iter().find(|h| h.0 == tick).map(|h| h.2)
    }

    /// the server's state of `tick`: compare, and on a difference go back to it and replay
    pub fn reconcile(&mut self, mover: &M, map: &physics::World, tick: u32, server: M::State, dt: f64) -> Outcome {
        if let Some((bt, _)) = self.base {
            if tick_diff(tick, bt) <= 0 {
                return Outcome::Stale;
            }
        }
        while self.hist.front().is_some_and(|h| tick_diff(h.0, tick) < 0) {
            self.hist.pop_front();
        }
        self.stats.checks += 1;
        let had_base = self.base.is_some();
        match self.hist.front() {
            Some(h) if h.0 == tick && had_base => {
                let predicted = h.2;
                self.hist.pop_front();
                self.base = Some((tick, server));
                if predicted == server {
                    return Outcome::Match;
                }
                let before = self.current_or_base();
                self.replay(mover, map, dt);
                let after = self.current_or_base();
                let error = (M::position(&predicted) - M::position(&server)).length();
                let shift = (M::position(&before) - M::position(&after)).length();
                self.stats.mismatches += 1;
                if self.stats.errors.len() < 100_000 {
                    self.stats.errors.push(error as f32);
                    self.stats.shifts.push(shift as f32);
                }
                Outcome::Corrected { error, shift }
            }
            _ => {
                // no prediction of that tick: start from the server's state with the inputs after it
                if self.hist.front().is_some_and(|h| h.0 == tick) {
                    self.hist.pop_front();
                }
                self.base = Some((tick, server));
                self.replay(mover, map, dt);
                self.stats.resets += 1;
                Outcome::Reset
            }
        }
    }

    fn replay(&mut self, mover: &M, map: &physics::World, dt: f64) {
        let Some((_, mut s)) = self.base else { return };
        for h in self.hist.iter_mut() {
            s = mover.step(map, &s, &h.1, dt);
            h.2 = s;
            self.stats.replayed += 1;
        }
    }

    /// the newest confirmed tick and state
    pub fn confirmed(&self) -> Option<(u32, M::State)> {
        self.base
    }

    /// ticks predicted beyond the confirmed one
    pub fn pending(&self) -> usize {
        self.hist.len()
    }

    /// what it counted
    pub fn stats(&self) -> &PredictStats {
        &self.stats
    }

    /// forget everything (a new figure, a new room)
    pub fn clear(&mut self) {
        self.hist.clear();
        self.base = None;
    }
}

/// the drawn position of the own figure after corrections (see the module header).  A correction moves the
/// predicted position by a jump; the offset takes the jump back, so the picture does not move at that moment, and
/// decays like exp(-t / decay) (a rational approximation, the same for any frame rate within a few percent).  Above
/// `snap` metres (a teleport, a return after death) the offset is dropped: the picture jumps with the state.  The
/// prediction itself never sees the offset: it stays bit for bit the server's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoother {
    offset: Vec3,
    /// the time constant of the decay (ms; 0: no smoothing)
    pub decay_ms: f64,
    /// a correction (with what is still left) larger than this is not smoothed (m)
    pub snap: f64,
    /// corrections taken in, and of those dropped by snap
    pub corrections: u64,
    /// see corrections
    pub snaps: u64,
}

impl Smoother {
    /// an offset of 0 decaying with `decay_ms`, snapping above `snap` metres
    pub fn new(decay_ms: f64, snap: f64) -> Smoother {
        Smoother { offset: Vec3::ZERO, decay_ms, snap, corrections: 0, snaps: 0 }
    }

    /// the predicted position jumped by `jump` (new minus old): the offset takes it back
    pub fn correct(&mut self, jump: Vec3) {
        self.corrections += 1;
        let o = self.offset - jump;
        if self.decay_ms <= 0.0 || !(o.length() <= self.snap) {
            self.offset = Vec3::ZERO;
            self.snaps += 1;
        } else {
            self.offset = o;
        }
    }

    /// forget the offset (a new figure, a reset of the prediction)
    pub fn reset(&mut self) {
        self.offset = Vec3::ZERO;
    }

    /// `dt` seconds passed
    pub fn advance(&mut self, dt: f64) {
        if self.offset == Vec3::ZERO || !(dt > 0.0) {
            return;
        }
        if self.decay_ms <= 0.0 {
            self.offset = Vec3::ZERO;
            return;
        }
        // exp(-x) as 1 / (1 + y + y^2/2 + y^3/6) to the n-th power with y = x/n <= 1/2: within 0.1 % per step,
        // the same for any frame rate
        let x = dt * 1000.0 / self.decay_ms;
        let n = (x * 2.0) as u32 + 1;
        let y = x / n as f64;
        let k = 1.0 / (1.0 + y * (1.0 + y * (0.5 + y / 6.0)));
        for _ in 0..n.min(64) {
            self.offset = self.offset * k;
        }
        if self.offset.length() < 1e-4 {
            self.offset = Vec3::ZERO;
        }
    }

    /// what a frame adds to the predicted position (blended by alpha) of the own figure
    pub fn offset(&self) -> Vec3 {
        self.offset
    }
}
