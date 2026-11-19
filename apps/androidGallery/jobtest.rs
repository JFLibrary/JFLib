// (docs/ANDROID.md, AN1) jfgui_SELFTEST=job: the UI stays smooth while a long job runs on the pool, and a
// job that ends while the UI rests wakes it at once.  No texts of its own (only the start page's progress
// bar moves); the numbers go to the log and to data://job-state.txt, as one line;
// apps/androidGallery/tests/android_job.py starts it, reads them and judges them.
//
//   phase 0  SETTLE_MS after the first frame (the app comes up), then BASE_MS of the same frames without
//            a job: the device's own longest gap, for comparison
//   phase 1  ITEMS items of ITEM_MS ms of arithmetic on the pool (plat::Job) while a timer asks for a
//            frame every FRAME_MS, as an animation would: how many frames came, the longest gap between two,
//            how many gaps were longer than SLOW_MS
//   phase 2  ROUNDS jobs of one item that sleeps while the UI asks for nothing: the time from the end of
//            the item to UIC_WAKE (on Android the looper's wake-up, android/bridge.c), against the
//            250 ms of an idle host's tick

use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering::SeqCst};
use plat::Job;
use ui::*;

const ITEMS: u64 = 240;
const ITEM_MS: u64 = 20;
const SLOW_MS: u64 = 50;
const SETTLE_MS: u64 = 1500;
const BASE_MS: u64 = 1000;
const ROUNDS: usize = 5;
const TIMER: u32 = 0x4A0B_0001; // the animation's timer: a frame every FRAME_MS, as an app animates
const FRAME_MS: u32 = 16;
const STATE: &str = "data://job-state.txt";

// time_us() at the end of phase 2's item (0: not yet)
static ENDED: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct JobTest {
    on: Option<bool>,
    phase: u32, // 0 settling and the frames without a job, 1 the long job, 2 the wake-ups, 3 done
    first_us: u64,
    base_frames: u32,
    base_gap_us: u64,
    job: Option<Job>,
    start_us: u64,
    last_us: u64,
    frames: u32,
    max_gap_us: u64,
    slow: u32,
    gaps: Vec<(u64, u64)>, // (ms after the job's start, ms) of the gaps over SLOW_MS
    job_ms: u64,
    wakes: Vec<u64>,
    workers: u32,
}

// ms of arithmetic nobody can skip
fn spin(ms: u64) -> u64 {
    let end = plat::time_us() + ms * 1000;
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    while plat::time_us() < end {
        for _ in 0..1000 {
            x = core::hint::black_box(x.rotate_left(5) ^ x.wrapping_mul(0x2545_F491_4F6C_DD1D));
        }
    }
    x
}

impl JobTest {
    fn on(&mut self) -> bool {
        *self.on.get_or_insert_with(|| plat::env("jfgui_SELFTEST").as_deref() == Some("job"))
    }

    /// (the start page's frame) -> the progress bar's value while the long job runs
    pub fn frame(&mut self, ui: &mut Ui) -> Option<i32> {
        if !self.on() {
            return None;
        }
        let now = plat::time_us();
        match self.phase {
            0 => {
                if self.first_us == 0 {
                    (self.first_us, self.last_us) = (now, now);
                }
                let gap = now - self.last_us;
                self.last_us = now;
                if now - self.first_us > SETTLE_MS * 1000 {
                    self.base_frames += 1;
                    self.base_gap_us = self.base_gap_us.max(gap);
                }
                ui.timer(TIMER, FRAME_MS);
                if now - self.first_us < (SETTLE_MS + BASE_MS) * 1000 {
                    return Some(0);
                }
                self.workers = plat::thr_count();
                let mut job = Job::new();
                job.set(ITEMS, |_, _| {
                    spin(ITEM_MS);
                });
                job.start();
                self.job = Some(job);
                self.phase = 1;
                (self.start_us, self.last_us) = (now, now);
                ui.timer(TIMER, FRAME_MS);
                Some(0)
            }
            1 => {
                let gap = now - self.last_us;
                self.last_us = now;
                self.frames += 1;
                self.max_gap_us = self.max_gap_us.max(gap);
                if gap > SLOW_MS * 1000 {
                    self.slow += 1;
                    self.gaps.push(((now - self.start_us) / 1000, gap / 1000));
                }
                let (done, items) = self.job.as_ref().map_or((true, ITEMS), |j| (j.done(), j.items_done()));
                if done {
                    self.job_ms = (now - self.start_us) / 1000;
                    self.phase = 2;
                    self.round();
                } else {
                    ui.timer(TIMER, FRAME_MS);
                }
                Some((items * 100 / ITEMS) as i32)
            }
            _ => None,
        }
    }

    // phase 2: the next job of one item that sleeps (not in step with the idle tick)
    fn round(&mut self) {
        let wait = 200 + 37 * self.wakes.len() as u32;
        ENDED.store(0, SeqCst);
        let mut job = Job::new();
        job.set(1, move |_, _| {
            plat::sleep_ms(wait);
            ENDED.store(plat::time_us(), SeqCst);
        });
        job.start();
        self.job = Some(job);
    }

    /// (App::command) the animation's timer: a frame; a wake-up: phase 2's job may have ended.  Nothing
    /// is taken from the other cards
    pub fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) {
        if id == UIC_TIMER && arg == TIMER {
            ui.invalidate();
            return;
        }
        if id != UIC_WAKE || self.phase != 2 || !self.job.as_ref().is_some_and(|j| j.done()) {
            return;
        }
        let ended = ENDED.load(SeqCst);
        if ended == 0 {
            return;
        }
        self.wakes.push(plat::time_us().saturating_sub(ended) / 1000);
        if self.wakes.len() < ROUNDS {
            self.round();
        } else {
            self.phase = 3;
            self.report();
        }
    }

    fn report(&self) {
        let wakes: Vec<String> = self.wakes.iter().map(|w| format!("{w}")).collect();
        let gaps: Vec<String> = self.gaps.iter().take(64).map(|(at, g)| format!("{g}@{at}")).collect();
        let line = format!(
            "jobtest: workers={} cpus={} base_frames={} base_gap_ms={} items={ITEMS} item_ms={ITEM_MS} job_ms={} frames={} max_gap_ms={} slow_frames={} slow={} wake_ms={}\n",
            self.workers, plat::cpu_count(), self.base_frames, self.base_gap_us / 1000, self.job_ms, self.frames,
            self.max_gap_us / 1000, self.slow, gaps.join(","), wakes.join(","));
        plat::write_stdout(line.as_bytes());
        let _ = plat::write_file(STATE, line.as_bytes());
    }
}
