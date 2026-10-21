// sim_thread -- runs the simulation on a worker thread (native targets only), with the
// commands of jfn_sim::Inline, so realtime mode and long runs never hold up the UI
//
//   Threaded::new(engine) -> Option<Threaded>   starts the worker (paused)
//   send(Cmd, wall_us) -> Result<Tick>           RunFor / RunIdle go to the worker and answer
//                                                at once (busy() until done); every other
//                                                command runs right away and answers as Inline
//   tick(wall_us) -> Tick                        what the worker did since the last tick: its
//                                                events, new frames, the clock and the view
//   lock() -> MutexGuard<Inline>                 the engine for queries (view, trace, problems)
//   busy() / wait_idle(ms) / pending()           long runs outstanding; pending() covers every
//                                                driver and suits ui.sync_hook_set
//
// The worker paces realtime mode itself (plat::task::now_us, a slice every FRAME_MS) and runs
// long commands in slices of `budget` events, so the lock is never held for long.  After each
// slice it calls plat::wake(): the UI gets UIC_WAKE and calls tick.  jfn_sim never names plat
// (it stays deterministic and runs on the web), which is why this driver lives in the app.
// Dropping a Threaded stops the worker and waits for it.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use jfn_sim::{ActErr, Cmd, Engine, Inline, Mode, RunOut, Tick};
use plat::sync::{channel, Mutex, MutexGuard, Receiver, RecvError, Sender};

/// milliseconds between two realtime slices
const FRAME_MS: u32 = 10;

/// the worker's stack
const STACK: usize = 1 << 20;

/// long runs not finished yet, of every driver (ui.sync_hook_set takes a plain fn)
static BUSY: AtomicU32 = AtomicU32::new(0);

/// a long run of some driver is still going (the sync hook of test scripts)
pub fn pending() -> bool {
    BUSY.load(Ordering::Acquire) > 0
}

enum Msg {
    /// the mode may have changed, or it is time to stop
    Kick,
    /// RunFor / RunIdle
    Run(Cmd),
}

/// what the worker did since the UI last looked
#[derive(Default)]
struct Out {
    events: usize,
    new_hops: bool,
}

struct Shared {
    sim: Mutex<Inline>,
    out: Mutex<Out>,
    /// long runs of this driver not finished yet
    busy: AtomicU32,
    quit: AtomicBool,
}

impl Shared {
    fn done_one(&self) {
        self.busy.fetch_sub(1, Ordering::AcqRel);
        BUSY.fetch_sub(1, Ordering::AcqRel);
    }
}

/// the engine on a worker thread
pub struct Threaded {
    sh: Arc<Shared>,
    tx: Sender<Msg>,
    worker: Option<plat::Thread>,
}

impl Threaded {
    /// starts the worker; None when no thread can be started
    pub fn new(eng: Engine) -> Option<Threaded> {
        let sh = Arc::new(Shared { sim: Mutex::new(Inline::new(eng)), out: Mutex::new(Out::default()), busy: AtomicU32::new(0), quit: AtomicBool::new(false) });
        let (tx, rx) = channel::<Msg>(64);
        let w = sh.clone();
        let worker = plat::thread_spawn(STACK, Box::new(move || work(w, rx)))?;
        Some(Threaded { sh, tx, worker: Some(worker) })
    }

    /// a command from the user interface (see the head of the file)
    pub fn send(&mut self, c: Cmd, wall_us: u64) -> Result<Tick, ActErr> {
        if matches!(c, Cmd::RunFor(_) | Cmd::RunIdle) {
            self.sh.busy.fetch_add(1, Ordering::AcqRel);
            BUSY.fetch_add(1, Ordering::AcqRel);
            return match self.tx.try_send(Msg::Run(c)) {
                Ok(()) => Ok(self.peek()),
                Err(Msg::Run(c)) => {
                    // the queue is full: run it here
                    self.sh.done_one();
                    let t = self.sh.sim.lock().send(c, wall_us);
                    t.map(|t| Tick { wake_ms: None, ..t })
                }
                Err(Msg::Kick) => Ok(self.peek()),
            };
        }
        let t = self.sh.sim.lock().send(c, wall_us);
        // a kick is only a hint: a full queue wakes the worker anyway
        let _ = self.tx.try_send(Msg::Kick);
        t.map(|t| Tick { wake_ms: None, ..t })
    }

    /// what the worker did since the last tick (wake_ms stays None: the worker wakes the UI)
    pub fn tick(&mut self, _wall_us: u64) -> Tick {
        let o = core::mem::take(&mut *self.sh.out.lock());
        let sim = self.sh.sim.lock();
        Tick { events: o.events, now: sim.eng.now(), wake_ms: None, new_hops: o.new_hops, view: sim.view() }
    }

    fn peek(&self) -> Tick {
        let sim = self.sh.sim.lock();
        Tick { events: 0, now: sim.eng.now(), wake_ms: None, new_hops: false, view: sim.view() }
    }

    pub fn mode(&self) -> Mode {
        self.sh.sim.lock().mode()
    }

    /// the driver and its engine for queries; the worker waits meanwhile
    pub fn lock(&self) -> MutexGuard<'_, Inline> {
        self.sh.sim.lock()
    }

    /// long runs of this driver are not finished
    pub fn busy(&self) -> bool {
        self.sh.busy.load(Ordering::Acquire) > 0
    }

    /// waits until the long runs are done, at most `ms` milliseconds (false: still busy)
    pub fn wait_idle(&self, ms: u32) -> bool {
        let end = plat::time_us() + ms as u64 * 1000;
        while self.busy() {
            if plat::time_us() >= end {
                return false;
            }
            plat::sleep_ms(1);
        }
        true
    }
}

impl Drop for Threaded {
    fn drop(&mut self) {
        self.sh.quit.store(true, Ordering::Release);
        let _ = self.tx.try_send(Msg::Kick);
        if let Some(w) = self.worker.take() {
            w.join();
        }
        // runs the worker never finished
        let left = self.sh.busy.swap(0, Ordering::AcqRel);
        BUSY.fetch_sub(left, Ordering::AcqRel);
    }
}

/// adds a slice's work to what the UI takes with tick, and wakes the UI
fn publish(sh: &Shared, events: usize, new_hops: bool) {
    let mut o = sh.out.lock();
    o.events += events;
    o.new_hops |= new_hops;
    drop(o);
    plat::wake();
}

fn work(sh: Arc<Shared>, rx: Receiver<Msg>) {
    loop {
        if sh.quit.load(Ordering::Acquire) {
            return;
        }
        // realtime: the clock follows the wall clock, slice by slice
        let realtime = {
            let mut sim = sh.sim.lock();
            if let Mode::Realtime { .. } = sim.mode() {
                let t = sim.tick(plat::task::now_us());
                drop(sim);
                publish(&sh, t.events, t.new_hops);
                true
            } else {
                false
            }
        };
        let msg = if realtime {
            match rx.recv_timeout(FRAME_MS) {
                Ok(m) => Some(m),
                Err(RecvError::Timeout) => None,
                Err(RecvError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Some(m) => Some(m),
                None => return,
            }
        };
        if let Some(Msg::Run(c)) = msg {
            run(&sh, c);
            sh.done_one();
        }
    }
}

/// a long run in slices of `budget` events
fn run(sh: &Shared, c: Cmd) {
    let target = {
        let mut sim = sh.sim.lock();
        let _ = sim.send(Cmd::Live, 0);
        match c {
            Cmd::RunFor(d) => Some(sim.eng.now() + d),
            _ => None,
        }
    };
    loop {
        if sh.quit.load(Ordering::Acquire) {
            return;
        }
        let mut sim = sh.sim.lock();
        let (hops, before, budget) = (sim.eng.trace.next_hop_id(), sim.eng.events_done(), sim.budget);
        let out = match target {
            Some(t) => sim.eng.run_until(t, budget),
            None => sim.eng.run_idle(budget),
        };
        let (n, new) = ((sim.eng.events_done() - before) as usize, sim.eng.trace.next_hop_id() != hops);
        if out != RunOut::Budget
            && let Mode::Realtime { speed_pm } = sim.mode()
        {
            // realtime goes on from here
            let _ = sim.send(Cmd::Realtime { speed_pm }, plat::task::now_us());
        }
        drop(sim);
        publish(sh, n, new);
        if out != RunOut::Budget {
            return;
        }
    }
}
