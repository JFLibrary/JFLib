// probe -- (G7b part 2) what the window path costs, measured from outside the frame loop: a thread presses a key
// (F9, posted to the window as the system would send it) at random times and reads the screen where the sky is
//
//   Probe::start()                 the thread; it waits for the app's window (set_window) and a phase (begin)
//   probe.begin(Phase::Latency)    presses 25 .. 45 ms apart; the app counts them (F9 by its place, the key
//                                  events game::Input takes) into the sky's red code:
//                                  press -> the app took it -> a frame with it drawn -> on the screen
//   probe.begin(Phase::Pacing)     no presses: the frames as the screen shows them
//   probe.seen()                   the app took a press (in their order)
//   probe.drawn(frame, presses)    the app drew frame `frame` with the presses it had taken
//   probe.set_covered(on)          the probe's point lies under another window (the app looks)
//   probe.end(name) -> Report      the phase's numbers
//
// The sky carries two codes (sky_codes): red 24 + 28 (presses % 8), blue 8 + 16 (frame % 16); green 180.  The
// screen is read with plat::win_test_screen (BitBlt from the screen's DC: under DWM what the compositor last
// composed, as docs/GPU.md measured the UI's output; it waits for DWM itself and does not reach the light).  A
// frame shows when the blue code changes: the intervals between those are the displayed frame pacing, a jump of
// more than one number counts frames drawn and never shown, the time from the app's draw to the screen is the
// display delay.  A press shows when the red code reaches its number (presses merged into one frame show
// together).  Windows only (other targets cannot read the screen: the app's own times still count).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use rt::sync::SpinLock;

/// the sky's codes (sRGB, 0 ..= 255) for the presses taken and the frame's number
pub fn sky_codes(presses: u32, frame: u64) -> [u8; 3] {
    [(24 + (presses % 8) * 28) as u8, 180, (8 + (frame % 16) * 16) as u8]
}

/// a code as linear light (render3d encodes the clear colour back to it: srgb_clear)
pub fn code_linear(c: u8) -> f32 {
    let v = c as f64 / 255.0;
    (if v <= 0.040_45 { v / 12.92 } else { rt::x87::pow((v + 0.055) / 1.055, 2.4) }) as f32
}

/// the sky's clear colour (linear) for a frame
pub fn sky_clear(presses: u32, frame: u64) -> [f32; 4] {
    let c = sky_codes(presses, frame);
    [code_linear(c[0]), code_linear(c[1]), code_linear(c[2]), 1.0]
}

/// a press shown later than this (us) counts as one that fell into a stall of the compositor
const STALL_US: u64 = 150_000;

/// the classes of the codes: presses % 8 from red, frame % 16 from blue (a few levels off still read right)
fn red_class(r: u8) -> u32 {
    (r.saturating_sub(10) as u32 / 28).min(7)
}

fn blue_class(b: u8) -> u64 {
    b as u64 / 16
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Latency,
    Pacing,
}

/// one press: when it was posted, when the app took it, when a frame with it was drawn, when the screen showed it
#[derive(Clone, Copy, Default)]
struct Press {
    t0: u64,
    seen: u64,
    drawn: u64,
    screen: u64,
}

/// shared between the app and the thread
struct Shared {
    hwnd: AtomicU64,
    /// the probe's point on the screen (screen pixels)
    x: AtomicI32,
    y: AtomicI32,
    phase: AtomicU32,
    stop: AtomicBool,
    /// the presses of this phase
    presses: SpinLock<Vec<Press>>,
    /// the frames the app drew: (number, time, presses)
    drawn: SpinLock<Vec<(u64, u64, u32)>>,
    /// what the screen showed: (time, red code, blue code) at every change
    screen: SpinLock<Vec<(u64, u8, u8)>>,
    /// screen reads in this phase, the time they took, the longest
    reads: AtomicU64,
    read_us: AtomicU64,
    read_max: AtomicU64,
    /// the probe's point lay under another window (looks of the app)
    covered: AtomicU32,
    /// presses the app took in this phase
    seen: AtomicU32,
    /// the app's press count when the phase began
    base: AtomicU32,
}

pub struct Probe {
    s: Arc<Shared>,
    began: core::cell::Cell<u64>,
}

/// median, 95th and 99th percentile, largest, mean, standard deviation (ms) of times in us
#[derive(Clone, Copy, Default, Debug)]
pub struct Stat {
    pub med: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub mean: f64,
    pub sd: f64,
}

pub fn stat(v: &[u64]) -> Stat {
    if v.is_empty() {
        return Stat::default();
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    let at = |q: f64| s[((s.len() - 1) as f64 * q + 0.5) as usize] as f64 / 1000.0;
    let mean = s.iter().sum::<u64>() as f64 / s.len() as f64 / 1000.0;
    let var = s.iter().map(|&x| (x as f64 / 1000.0 - mean) * (x as f64 / 1000.0 - mean)).sum::<f64>() / s.len() as f64;
    Stat { med: at(0.5), p95: at(0.95), p99: at(0.99), max: s[s.len() - 1] as f64 / 1000.0, mean, sd: physics::math::sqrt(var) }
}

/// the numbers of a phase
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub phase: String,
    /// the presses: posted -> the app took it, -> a frame with it drawn, -> on the screen
    pub to_app: Stat,
    pub to_drawn: Stat,
    pub to_screen: Stat,
    pub presses: usize,
    /// presses the screen never showed; showed only after STALL_US (not in to_screen)
    pub lost: usize,
    pub stalled: usize,
    /// frames drawn by the app, and their intervals
    pub drawn: usize,
    pub draw_gap: Stat,
    /// frames the screen showed (a change of the blue code), their intervals, frames drawn and never shown
    /// (jumps of the number), the time from the draw to the screen
    pub shown: usize,
    pub shown_gap: Stat,
    pub skipped: usize,
    pub delay: Stat,
    /// screen reads, their mean and longest time (ms); looks that found the probe's point under another window
    pub reads: u64,
    pub read_ms: f64,
    pub read_max: f64,
    pub covered: u32,
    pub secs: f64,
    /// (ms from the phase's start) where the longest gap between two frames on the screen began; the first press
    pub gap_at: f64,
    pub first_press: f64,
}

impl Report {
    /// the report as one line (RESULT, for the scripts that gather them)
    pub fn line(&self) -> String {
        let s = |x: &Stat| format!("{:.2}/{:.2}/{:.2}", x.med, x.p95, x.max);
        format!(
            "RESULT {} presses={} lost={} stalled={} to_app={} to_drawn={} to_screen={} screen_mean={:.2} drawn={} draw_gap={} draw_sd={:.2} shown={} shown_gap={} shown_p99={:.2} shown_sd={:.2} skipped={} delay={} reads={} read_ms={:.3} read_max={:.1} covered={} secs={:.1} gap_at={:.0} first_press={:.0}",
            self.phase, self.presses, self.lost, self.stalled, s(&self.to_app), s(&self.to_drawn), s(&self.to_screen), self.to_screen.mean, self.drawn, s(&self.draw_gap), self.draw_gap.sd,
            self.shown, s(&self.shown_gap), self.shown_gap.p99, self.shown_gap.sd, self.skipped, s(&self.delay), self.reads, self.read_ms, self.read_max, self.covered,
            self.secs, self.gap_at, self.first_press
        )
    }
}

impl Probe {
    /// the thread (it does nothing until a window and a phase are set)
    pub fn start() -> Probe {
        let s = Arc::new(Shared {
            hwnd: AtomicU64::new(0),
            x: AtomicI32::new(0),
            y: AtomicI32::new(0),
            phase: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            presses: SpinLock::new(Vec::new()),
            drawn: SpinLock::new(Vec::new()),
            screen: SpinLock::new(Vec::new()),
            reads: AtomicU64::new(0),
            read_us: AtomicU64::new(0),
            read_max: AtomicU64::new(0),
            covered: AtomicU32::new(0),
            seen: AtomicU32::new(0),
            base: AtomicU32::new(0),
        });
        // (two threads: a read of the screen waits for the compositor, a press posted after it would come right
        // after a composition every time)
        let t = s.clone();
        if let Some(h) = plat::thread_spawn(0, alloc::boxed::Box::new(move || read(t))) {
            drop(h);
        }
        let t = s.clone();
        if let Some(h) = plat::thread_spawn(0, alloc::boxed::Box::new(move || presser(t))) {
            drop(h);
        }
        Probe { s, began: core::cell::Cell::new(0) }
    }

    /// (Windows) the app's plat window `w`: the presses go to it, the probe's point is its client point cx, cy; it
    /// counts as covered when another window lies on top there (`child`: the window's swapchain lives on a child
    /// window, lib/ui's way).  Elsewhere nothing: no way to post a key or to read the screen
    pub fn locate(&self, w: i32, cx: i32, cy: i32, child: bool) {
        #[cfg(windows)]
        if let (Some(n), Some(r)) = (plat::win_native(w), plat::win_get_rect(w)) {
            let (x, y) = (r[0] + cx, r[1] + cy);
            self.set_window(n.hwnd as u64, x, y);
            let on_top = if child { plat::win_test(plat::WT_FROM_POINT, w, [cx, cy, 0, 0]) != 0 } else { plat::win_test_window_at(x, y) == n.hwnd };
            self.set_covered(!on_top);
        }
        #[cfg(not(windows))]
        let _ = (w, cx, cy, child);
    }

    /// the window the presses go to (its native handle) and the probe's point (screen pixels)
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set_window(&self, hwnd: u64, x: i32, y: i32) {
        self.s.x.store(x, Ordering::Relaxed);
        self.s.y.store(y, Ordering::Relaxed);
        self.s.hwnd.store(hwnd, Ordering::Relaxed);
    }

    /// the probe's point lies under another window (the app looks, about every 250 ms)
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set_covered(&self, on: bool) {
        if on {
            self.s.covered.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// a new phase (`presses`: what the app has taken so far); what was recorded goes
    pub fn begin(&self, p: Phase, presses: u32) {
        self.s.phase.store(0, Ordering::SeqCst);
        plat::sleep_ms(3);
        self.s.presses.lock().clear();
        self.s.drawn.lock().clear();
        self.s.screen.lock().clear();
        self.s.reads.store(0, Ordering::Relaxed);
        self.s.read_us.store(0, Ordering::Relaxed);
        self.s.read_max.store(0, Ordering::Relaxed);
        self.s.covered.store(0, Ordering::Relaxed);
        self.s.seen.store(0, Ordering::Relaxed);
        self.s.base.store(presses, Ordering::Relaxed);
        self.began.set(plat::time_us());
        self.s.phase.store(if p == Phase::Latency { 1 } else { 2 }, Ordering::SeqCst);
    }

    /// the app took a press (in the order they came)
    pub fn seen(&self) {
        let k = self.s.seen.fetch_add(1, Ordering::Relaxed) as usize;
        if let Some(p) = self.s.presses.lock().get_mut(k) {
            p.seen = plat::time_us();
        }
    }

    /// the app drew frame `frame` with `presses` taken
    pub fn drawn(&self, frame: u64, presses: u32) {
        let now = plat::time_us();
        if self.s.phase.load(Ordering::Relaxed) == 0 {
            return;
        }
        // (the presses this frame is the first to carry: drawn now)
        let n = presses.wrapping_sub(self.s.base.load(Ordering::Relaxed)) as usize;
        let mut ps = self.s.presses.lock();
        for p in ps.iter_mut().take(n).filter(|p| p.drawn == 0) {
            p.drawn = now;
        }
        drop(ps);
        self.s.drawn.lock().push((frame, now, presses));
    }

    /// presses posted in this phase
    pub fn presses(&self) -> usize {
        self.s.presses.lock().len()
    }

    /// the phase ends -> its numbers
    pub fn end(&self, name: &str) -> Report {
        self.s.phase.store(0, Ordering::SeqCst);
        plat::sleep_ms(3);
        let presses = self.s.presses.lock().clone();
        let drawn = self.s.drawn.lock().clone();
        let screen = self.s.screen.lock().clone();
        let base = self.s.base.load(Ordering::Relaxed);
        let mut r = Report { phase: String::from(name), secs: plat::time_us().saturating_sub(self.began.get()) as f64 / 1e6, ..Report::default() };
        // presses on the screen: the red class is the presses shown mod 8; the screen cannot show more presses than
        // were posted by then, so the count a sample shows is the largest one up to that, congruent to its class
        // (robust against reads that missed changes or stalled)
        let mut ps = presses.clone();
        for &(t, red, _) in &screen {
            let posted = ps.iter().filter(|p| p.t0 < t).count() as u64 + base as u64;
            let c = red_class(red) as u64;
            let count = posted - (posted + 8 - c) % 8;
            let upto = count.saturating_sub(base as u64) as usize;
            for p in ps.iter_mut().take(upto).filter(|p| p.screen == 0 && p.t0 <= t) {
                p.screen = t;
            }
        }
        r.presses = ps.len();
        r.to_app = stat(&ps.iter().filter(|p| p.seen != 0).map(|p| p.seen.saturating_sub(p.t0)).collect::<Vec<_>>());
        r.to_drawn = stat(&ps.iter().filter(|p| p.drawn != 0).map(|p| p.drawn.saturating_sub(p.t0)).collect::<Vec<_>>());
        // (a press the screen showed only after STALL_US fell into a stall of the compositor: on this shared machine
        // the screen stood still for 0.5 .. 0.8 s every few seconds, in every variant alike; counted apart)
        r.to_screen = stat(&ps.iter().filter(|p| p.screen != 0 && p.screen - p.t0 <= STALL_US).map(|p| p.screen.saturating_sub(p.t0)).collect::<Vec<_>>());
        r.stalled = ps.iter().filter(|p| p.screen != 0 && p.screen - p.t0 > STALL_US).count();
        r.lost = ps.iter().filter(|p| p.screen == 0).count();
        r.drawn = drawn.len();
        r.draw_gap = stat(&drawn.windows(2).map(|w| w[1].1 - w[0].1).collect::<Vec<_>>());
        // the frames on the screen: a change of the blue class
        let mut shown: Vec<(u64, u64)> = Vec::new(); // (time, frame % 16)
        for &(t, _, b) in &screen {
            let c = blue_class(b);
            if shown.last().is_none_or(|&(_, l)| l != c) {
                shown.push((t, c));
            }
        }
        r.shown = shown.len().saturating_sub(1);
        r.shown_gap = stat(&shown.windows(2).map(|w| w[1].0 - w[0].0).collect::<Vec<_>>());
        // (where the longest gap began, from the phase's start, and the first press's time)
        if let Some(w) = shown.windows(2).max_by_key(|w| w[1].0 - w[0].0) {
            r.gap_at = w[0].0.saturating_sub(self.began.get()) as f64 / 1000.0;
        }
        r.first_press = ps.first().map_or(0.0, |p| p.t0.saturating_sub(self.began.get()) as f64 / 1000.0);
        r.skipped = shown.windows(2).map(|w| ((w[1].1 + 16 - w[0].1) % 16).saturating_sub(1) as usize).sum();
        // from the draw to the screen: the newest frame with that number drawn before the change
        let mut delay = Vec::new();
        for &(t, c) in shown.iter().skip(1) {
            if let Some(&(_, td, _)) = drawn.iter().rev().find(|d| d.1 <= t && d.0 % 16 == c) {
                delay.push(t - td);
            }
        }
        r.delay = stat(&delay);
        r.reads = self.s.reads.load(Ordering::Relaxed);
        r.read_ms = self.s.read_us.load(Ordering::Relaxed) as f64 / r.reads.max(1) as f64 / 1000.0;
        r.read_max = self.s.read_max.load(Ordering::Relaxed) as f64 / 1000.0;
        r.covered = self.s.covered.load(Ordering::Relaxed);
        r
    }

    /// the thread ends
    pub fn stop(&self) {
        self.s.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.stop();
    }
}

/// F9 (its virtual key and scan code) posted as WM_KEYDOWN and WM_KEYUP, as the system sends a key
#[cfg(windows)]
fn press(hwnd: u64) {
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const VK_F9: usize = 0x78;
    plat::win_test_post(hwnd as isize, WM_KEYDOWN, VK_F9, 0x0043_0001);
    plat::win_test_post(hwnd as isize, WM_KEYUP, VK_F9, 0xC043_0001u32 as i32 as isize);
}

#[cfg(not(windows))]
fn press(_hwnd: u64) {}

/// the screen's pixel at x, y: (red, blue) codes
#[cfg(windows)]
fn screen_px(x: i32, y: i32) -> Option<(u8, u8)> {
    plat::win_test_screen(x, y, 1, 1).map(|v| (v[2], v[0]))
}

#[cfg(not(windows))]
fn screen_px(_x: i32, _y: i32) -> Option<(u8, u8)> {
    None
}

/// the presses: 25 .. 45 ms apart at random (to the microsecond: the last stretch is spun)
fn presser(s: Arc<Shared>) {
    let mut seed = 0x9E37_79B9u32;
    let mut next_press = 0u64;
    while !s.stop.load(Ordering::Relaxed) {
        let hwnd = s.hwnd.load(Ordering::Relaxed);
        if s.phase.load(Ordering::SeqCst) != 1 || hwnd == 0 {
            next_press = 0;
            plat::sleep_ms(1);
            continue;
        }
        let now = plat::time_us();
        if next_press == 0 {
            next_press = now + 100_000;
        }
        if now + 1500 < next_press {
            plat::sleep_ms(1);
            continue;
        }
        while plat::time_us() < next_press {
            core::hint::spin_loop();
        }
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        next_press += 25_000 + (seed >> 8) as u64 % 20_000;
        s.presses.lock().push(Press { t0: plat::time_us(), ..Press::default() });
        press(hwnd);
    }
}

/// the screen at the probe's point, as fast as it can be read
fn read(s: Arc<Shared>) {
    let mut last: Option<(u8, u8)> = None;
    while !s.stop.load(Ordering::Relaxed) {
        let hwnd = s.hwnd.load(Ordering::Relaxed);
        if s.phase.load(Ordering::SeqCst) == 0 || hwnd == 0 {
            last = None;
            plat::sleep_ms(1);
            continue;
        }
        let (x, y) = (s.x.load(Ordering::Relaxed), s.y.load(Ordering::Relaxed));
        let t = plat::time_us();
        let Some(px) = screen_px(x, y) else {
            plat::sleep_ms(1);
            continue;
        };
        let t1 = plat::time_us();
        s.reads.fetch_add(1, Ordering::Relaxed);
        s.read_us.fetch_add(t1 - t, Ordering::Relaxed);
        s.read_max.fetch_max(t1 - t, Ordering::Relaxed);
        if last != Some(px) {
            // (the read's middle: the screen changed somewhere in between)
            s.screen.lock().push((t + (t1 - t) / 2, px.0, px.1));
            last = Some(px);
        }
    }
}
