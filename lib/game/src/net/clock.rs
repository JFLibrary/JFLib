// clock -- a client's two clocks: the ticks it predicts and sends inputs for (ahead of the server by the
// trip of its inputs plus a margin), and the time it shows the other entities at (behind the newest
// snapshot by an interpolation delay); both run on time from outside, like FixedTick
//
//   SteeredClock    the prediction's ticks.  The server reports how early the client's inputs arrive (the
//                   smallest margin per snapshot, in ticks); the clock runs up to 10 % faster or slower to
//                   hold the margin at its target (2 ticks unless set), jumps ahead when far too late and
//                   holds still when far too early.  All in integers: the speed in ppm, the accumulator in
//                   ns x rate x ppm
//   InterpClock     the picture of the others: a render time in server ticks (a tick and a fraction, so it
//                   survives the wrap of u32), interp_ticks behind the newest snapshot, steered by up to
//                   10 % towards that place at every snapshot, snapped when it is far off, never past the
//                   newest snapshot (it holds)
//
// The two are separate from the tick interpolation of Pose (that blends the last two predicted ticks of
// the own figure); docs/NETCODE.md, "Zeit".

use super::snapshot::tick_diff;

const NS: u128 = 1_000_000_000;
const PPM: u128 = 1_000_000;

/// the prediction's clock (see the module header)
#[derive(Clone, Debug)]
pub struct SteeredClock {
    rate: u32,
    tick: u32,
    acc: u128,
    last: Option<u64>,
    speed_ppm: u32,
    margin: Option<f64>,
    target: f64,
    extra: u32,
    hold: u32,
    jumps: u32,
}

impl SteeredClock {
    /// `rate` ticks a second, holding the margin of the inputs at `target` ticks
    pub fn new(rate: u32, target: f64) -> SteeredClock {
        SteeredClock { rate: rate.clamp(1, 10_000), tick: 0, acc: 0, last: None, speed_ppm: 1_000_000, margin: None, target, extra: 0, hold: 0, jumps: 0 }
    }

    /// start at `tick` (the next tick advance hands out)
    pub fn set_tick(&mut self, tick: u32) {
        self.tick = tick;
        self.acc = 0;
        self.margin = None;
    }

    /// the clock stands at now_ns -> the ticks due (take each with next_tick)
    pub fn advance(&mut self, now_ns: u64) -> u32 {
        let elapsed = match self.last {
            Some(t) => now_ns.saturating_sub(t),
            None => 0,
        };
        if self.last.is_none_or(|t| now_ns > t) {
            self.last = Some(now_ns);
        }
        self.acc += elapsed as u128 * self.rate as u128 * self.speed_ppm as u128;
        let step = NS * PPM;
        let mut due = (self.acc / step) as u32;
        self.acc %= step;
        if self.hold > 0 {
            let h = self.hold.min(due);
            self.hold -= h;
            due -= h;
        }
        // (a hung frame catches up at most 16 ticks, a jump at most 32 more)
        due = due.min(16) + core::mem::take(&mut self.extra).min(32);
        due
    }

    /// the tick to run now (call once per tick advance returned)
    pub fn next_tick(&mut self) -> u32 {
        let t = self.tick;
        self.tick = self.tick.wrapping_add(1);
        t
    }

    /// the next tick next_tick hands out
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// how far the clock is into the next tick, in [0, 1) (a frame blends the last two predicted states)
    pub fn alpha(&self) -> f64 {
        self.acc as f64 / (NS * PPM) as f64
    }

    /// the server's report: the smallest margin of the client's inputs (ticks they arrived early)
    pub fn on_margin(&mut self, m: i32) {
        let ema = match self.margin {
            None => m as f64,
            Some(e) => e + (m as f64 - e) * 0.2,
        };
        let err = ema - self.target;
        if err < -6.0 {
            // far too late: jump ahead (the inputs of the skipped ticks repeat the current one)
            let k = (-err) as u32 + 1;
            self.extra = self.extra.saturating_add(k);
            self.margin = Some(ema + k as f64);
            self.jumps += 1;
            self.speed_ppm = 1_000_000;
            return;
        }
        if err > 30.0 {
            // far too early: hold still for most of it
            self.hold = self.hold.saturating_add(err as u32 - 2);
            self.margin = Some(ema - (err - 2.0));
            self.jumps += 1;
            self.speed_ppm = 1_000_000;
            return;
        }
        self.margin = Some(ema);
        let adj = (err * 20_000.0).clamp(-100_000.0, 100_000.0) as i32;
        self.speed_ppm = (1_000_000 - adj) as u32;
    }

    /// the speed now in ppm (1 000 000: nominal)
    pub fn speed_ppm(&self) -> u32 {
        self.speed_ppm
    }

    /// the smoothed margin (None before the first report)
    pub fn margin(&self) -> Option<f64> {
        self.margin
    }

    /// how often it jumped or held
    pub fn jumps(&self) -> u32 {
        self.jumps
    }
}

/// x rounded down (core has no f64::floor without std)
pub(crate) fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// the time the others are shown at (see the module header)
#[derive(Clone, Debug)]
pub struct InterpClock {
    rate: u32,
    delay: f64,
    anchor: u32,
    frac: f64,
    newest: Option<u32>,
    last: Option<u64>,
    speed: f64,
    snaps: u32,
}

impl InterpClock {
    /// `rate` ticks a second, `delay` ticks behind the newest snapshot
    pub fn new(rate: u32, delay: f64) -> InterpClock {
        InterpClock { rate: rate.clamp(1, 10_000), delay, anchor: 0, frac: 0.0, newest: None, last: None, speed: 1.0, snaps: 0 }
    }

    fn norm(&mut self) {
        let w = floor(self.frac);
        self.anchor = self.anchor.wrapping_add(w as i64 as u32);
        self.frac -= w;
    }

    // render time minus `t`, in ticks
    fn minus(&self, t: u32) -> f64 {
        tick_diff(self.anchor, t) as f64 + self.frac
    }

    /// a snapshot of `tick` came (older ones than the newest only count as nothing)
    pub fn on_snapshot(&mut self, tick: u32) {
        let newer = self.newest.is_none_or(|n| tick_diff(tick, n) > 0);
        if !newer {
            return;
        }
        self.newest = Some(tick);
        self.snaps += 1;
        let err = self.minus(tick) + self.delay;
        if self.snaps == 1 || !(-(self.delay * 3.0 + 30.0)..=self.delay * 3.0 + 30.0).contains(&err) {
            self.anchor = tick;
            self.frac = -self.delay;
            self.norm();
            self.speed = 1.0;
            return;
        }
        self.speed = 1.0 - (err * 0.05).clamp(-0.1, 0.1);
    }

    /// the clock stands at now_ns
    pub fn advance(&mut self, now_ns: u64) {
        let elapsed = match self.last {
            Some(t) => now_ns.saturating_sub(t),
            None => 0,
        };
        if self.last.is_none_or(|t| now_ns > t) {
            self.last = Some(now_ns);
        }
        let Some(n) = self.newest else { return };
        self.frac += elapsed as f64 * self.rate as f64 / 1e9 * self.speed;
        self.norm();
        if self.minus(n) > 0.0 {
            self.anchor = n;
            self.frac = 0.0;
        }
    }

    /// the render time: a tick and the fraction towards the next (None before the first snapshot)
    pub fn time(&self) -> Option<(u32, f64)> {
        self.newest.map(|_| (self.anchor, self.frac))
    }

    /// how far the render time is behind the newest snapshot, in ticks
    pub fn behind(&self) -> f64 {
        self.newest.map_or(0.0, |n| -self.minus(n))
    }
}
