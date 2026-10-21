// task -- async execution: core::future on the thread that owns an executor, blocking work on
// a pool, timers, cancellation, joins                                              (task.rs)
//
//   Executor::ui()                  the executor of the UI thread: a wake-up from another thread
//                                   posts plat::wake (E_WAKE); lib/ui runs it between passes
//                                   (ui.spawn), web / Android from the host's frame callback
//   Executor::new()                 an executor that parks its thread (block_on, console programs)
//   exec.spawn(fut) -> Task<T>      fut runs on the executor's thread (it need not be Send)
//   exec.spawner() -> Spawner       spawn from inside tasks (a clone moves into them)
//   exec.run() -> polled            fire the thread's due timers, run queued blocking jobs where
//                                   there are no threads, poll every woken task once
//   exec.accept_remote() -> bool    (Executor::ui) take the wake-ups that came from other threads;
//                                   the UI calls it for E_WAKE, so headless scripts see them only
//                                   at `sync` (deterministic)
//   exec.has_ready() / busy() / live()   work to poll now / work in flight (`sync`) / tasks alive
//   exec.block_on(fut), block_on(fut)    run until fut is done (spawned tasks run alongside)
//   Task<T>: is_done / is_cancelled / take / cancel / detach; awaiting it gives Result<T,
//            Cancelled>; dropping the handle cancels the task (detach keeps it running)
//   spawn_blocking(f) -> Blocking<T>     f on a thread of the blocking pool (files, network,
//                                   parallel_for); spawn_cancellable(f(&token)): token is
//                                   cancelled once nobody awaits the result (a job not started
//                                   then never starts).  A caller inside policy's trusted() keeps
//                                   that on the pool thread.  blocking_pending() -> jobs in flight
//   sleep(ms) / sleep_until(us)     timers of the polling thread's executor on now_us()
//   now_us() / clock_set(Some(us) | None)   the task clock: time_us(), or the virtual clock of
//                                   headless UI runs (scripts' `wait`)
//   next_timer() -> Option<us>      the earliest timer of this thread (event loop deadlines)
//   CancelToken: cancel / is_cancelled / check / cancelled() future / run(fut) -> Result<T,
//                Cancelled> (fut is dropped when the token wins)
//   Progress: set_total / set / add / done / total / permille / changed() future; a report
//             wakes the event loop (at most every PROGRESS_WAKE_MS, always at the end)
//   join(a, b) / join3(a, b, c) / join_all(v) / select(a, b) -> Either / yield_now()
//
// Design.  An executor keeps its tasks in slots; a task's Waker (alloc::task::Wake, no unsafe)
// names slot and version and queues itself once until it is polled again.  Wake-ups on the
// owning thread go to the local list, those from other threads to the remote list plus a signal
// (plat::wake for the UI, the park semaphore for block_on); the UI takes remote ones only with
// E_WAKE.  Timers live in one list with the thread that registered them; each executor fires
// its own.  Blocking jobs run on up to BLOCKING_MAX threads of their own (not the compute pool:
// a job waiting for a disk must not starve parallel_for); where no thread can start (the
// browser without shared memory, Android) the executors run them inline, one per run.  A panic
// in a task or job ends the process like any panic (rt's handler: crash hook, exit code 3).

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};
use core::task::{Context, Poll, Waker};

use rt::sync::SpinLock;

pub use crate::Cancelled;

/// threads of the blocking pool at most (more jobs wait in its queue)
pub const BLOCKING_MAX: u32 = 8;
/// a Progress report wakes the event loop at most this often (the last one always)
pub const PROGRESS_WAKE_MS: u64 = 30;
const BLOCKING_STACK: usize = 512 * 1024;

#[cfg(any(windows, target_os = "linux"))]
fn tid() -> u64 {
    crate::plat_tid()
}

// no thread ids: every wake-up counts as local, and the owner is told (the browser's workers)
#[cfg(not(any(windows, target_os = "linux")))]
fn tid() -> u64 {
    0
}

// ---------------------------------------------------------------- the task clock

static VCLOCK: AtomicU64 = AtomicU64::new(u64::MAX); // u64::MAX: the real clock

/// the task clock in microseconds: time_us(), or the virtual time clock_set gave
pub fn now_us() -> u64 {
    match VCLOCK.load(SeqCst) {
        u64::MAX => crate::time_us(),
        v => v,
    }
}

/// Some(us): the task clock stands at us until the next call (headless UI runs drive it with
/// the script's virtual clock); None: back to time_us()
pub fn clock_set(us: Option<u64>) {
    VCLOCK.store(us.map_or(u64::MAX, |v| v.min(u64::MAX - 1)), SeqCst);
}

// ---------------------------------------------------------------- executors

type Boxed = Pin<Box<dyn Future<Output = ()>>>;

struct Shared {
    owner: u64,
    /// None: the UI event loop (plat::wake); Some: block_on's semaphore
    park: Option<Arc<crate::Sem>>,
    parked: bool,
    local: SpinLock<Vec<(u32, u32)>>,
    remote: SpinLock<Vec<(u32, u32)>>,
    notified: AtomicBool,
}

impl Shared {
    fn push(&self, e: (u32, u32)) {
        let here = tid();
        if here == self.owner && here != 0 {
            self.local.lock().push(e);
        } else if here == 0 {
            self.local.lock().push(e);
            self.signal();
        } else {
            self.remote.lock().push(e);
            self.signal();
        }
    }

    fn signal(&self) {
        if !self.parked {
            crate::wake();
        } else if !self.notified.swap(true, SeqCst) {
            if let Some(s) = &self.park {
                s.post(1);
            }
        }
    }
}

struct TaskWaker {
    sh: Arc<Shared>,
    slot: u32,
    ver: u32,
    queued: AtomicBool,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if !self.queued.swap(true, SeqCst) {
            self.sh.push((self.slot, self.ver));
        }
    }
}

struct Running {
    fut: Boxed,
    waker: Waker,
    tw: Arc<TaskWaker>,
}

struct Slot {
    ver: u32,
    task: Option<Running>,
}

/// one thread's tasks (see the head of task.rs)
pub struct Executor {
    sh: Arc<Shared>,
    slots: Vec<Slot>,
    free: Vec<u32>,
    queue: Rc<RefCell<Vec<Boxed>>>,
    live: usize,
}

impl Executor {
    fn make(park: bool) -> Executor {
        let sem = if park { crate::Sem::new(0).map(Arc::new) } else { None };
        let sh = Shared {
            owner: tid(),
            parked: park,
            park: sem,
            local: SpinLock::new(Vec::new()),
            remote: SpinLock::new(Vec::new()),
            notified: AtomicBool::new(false),
        };
        Executor { sh: Arc::new(sh), slots: Vec::new(), free: Vec::new(), queue: Rc::new(RefCell::new(Vec::new())), live: 0 }
    }

    /// the UI thread's executor: other threads' wake-ups post plat::wake (E_WAKE)
    pub fn ui() -> Executor {
        Executor::make(false)
    }

    /// an executor that parks this thread while it waits (block_on)
    pub fn new() -> Executor {
        Executor::make(true)
    }

    /// a handle that spawns onto this executor (from inside its tasks)
    pub fn spawner(&self) -> Spawner {
        Spawner { q: self.queue.clone() }
    }

    /// run fut on this executor's thread; it starts with the next run()
    pub fn spawn<F: Future + 'static>(&self, fut: F) -> Task<F::Output> {
        self.spawner().spawn(fut)
    }

    // a spawned future gets a slot and is queued for its first poll
    fn insert(&mut self, fut: Boxed) {
        let slot = match self.free.pop() {
            Some(s) => s,
            None => {
                self.slots.push(Slot { ver: 0, task: None });
                (self.slots.len() - 1) as u32
            }
        };
        let ver = self.slots[slot as usize].ver;
        let tw = Arc::new(TaskWaker { sh: self.sh.clone(), slot, ver, queued: AtomicBool::new(true) });
        let waker = Waker::from(tw.clone());
        self.slots[slot as usize].task = Some(Running { fut, waker, tw });
        self.live += 1;
        self.sh.local.lock().push((slot, ver));
    }

    /// take the wake-ups other threads sent (the UI calls it for E_WAKE) -> any
    pub fn accept_remote(&mut self) -> bool {
        let mut r = core::mem::take(&mut *self.sh.remote.lock());
        if r.is_empty() {
            return false;
        }
        self.sh.local.lock().append(&mut r);
        true
    }

    /// fire this thread's due timers, run a queued blocking job where no thread can start,
    /// poll every woken task once -> the number of polls
    pub fn run(&mut self) -> usize {
        let spawned = core::mem::take(&mut *self.queue.borrow_mut());
        for f in spawned {
            self.insert(f);
        }
        fire_timers(now_us());
        run_inline(1);
        let mut ready = core::mem::take(&mut *self.sh.local.lock());
        if self.sh.parked {
            ready.append(&mut core::mem::take(&mut *self.sh.remote.lock()));
        }
        let mut n = 0;
        for (slot, ver) in ready {
            let Some(s) = self.slots.get_mut(slot as usize) else { continue };
            if s.ver != ver {
                continue; // the slot belongs to a later task
            }
            let Some(r) = s.task.as_mut() else { continue };
            r.tw.queued.store(false, SeqCst); // a wake-up from now on queues it again
            n += 1;
            let Running { fut, waker, .. } = r;
            if fut.as_mut().poll(&mut Context::from_waker(waker)).is_ready() {
                let done = s.task.take();
                s.ver = s.ver.wrapping_add(1);
                self.free.push(slot);
                self.live -= 1;
                drop(done); // after the slot is free: its drop may wake other tasks
            }
        }
        n
    }

    /// tasks to poll now (woken or new, a blocking job to run inline)
    pub fn has_ready(&self) -> bool {
        !self.queue.borrow().is_empty()
            || !self.sh.local.lock().is_empty()
            || (self.sh.parked && !self.sh.remote.lock().is_empty())
            || inline_pending()
    }

    /// work in flight: ready tasks, wake-ups of other threads not taken yet, blocking jobs
    /// (the scripts' `sync` waits until it is false; tasks waiting for timers do not count)
    pub fn busy(&self) -> bool {
        self.has_ready() || !self.sh.remote.lock().is_empty() || blocking_pending() > 0
    }

    /// tasks alive (spawned, not finished)
    pub fn live(&self) -> usize {
        self.live + self.queue.borrow().len()
    }

    /// run until fut is done; the executor's other tasks run alongside.  The thread sleeps on a
    /// semaphore between wake-ups (Executor::ui: it only yields)
    pub fn block_on<F: Future>(&mut self, fut: F) -> F::Output {
        let mut fut = core::pin::pin!(fut);
        let main = Arc::new(MainWaker { sh: self.sh.clone(), woken: AtomicBool::new(true) });
        let waker = Waker::from(main.clone());
        loop {
            self.sh.notified.store(false, SeqCst);
            if main.woken.swap(false, SeqCst) {
                if let Poll::Ready(v) = fut.as_mut().poll(&mut Context::from_waker(&waker)) {
                    return v;
                }
            }
            self.run();
            if main.woken.load(SeqCst) || self.has_ready() {
                continue;
            }
            self.park();
        }
    }

    // sleep until a wake-up or this thread's next timer
    fn park(&self) {
        let ms = match next_timer() {
            None => -1,
            Some(at) => {
                let now = now_us();
                if at <= now {
                    return;
                }
                ((at - now + 999) / 1000).min(i32::MAX as u64) as i32
            }
        };
        match &self.sh.park {
            Some(s) => {
                s.wait(ms);
            }
            None => crate::thread_yield(),
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new()
    }
}

struct MainWaker {
    sh: Arc<Shared>,
    woken: AtomicBool,
}

impl Wake for MainWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, SeqCst);
        if tid() != self.sh.owner || tid() == 0 {
            self.sh.signal();
        }
    }
}

/// run fut to its end on this thread (a parking executor; console programs, tests)
pub fn block_on<F: Future>(fut: F) -> F::Output {
    Executor::new().block_on(fut)
}

/// spawns onto the executor it came from (clone it into tasks)
#[derive(Clone)]
pub struct Spawner {
    q: Rc<RefCell<Vec<Boxed>>>,
}

impl Spawner {
    /// run fut on the executor's thread; it starts with the executor's next run
    pub fn spawn<F: Future + 'static>(&self, fut: F) -> Task<F::Output> {
        let st = Arc::new(TaskState { flags: AtomicU32::new(0), out: SpinLock::new(None), waiter: SpinLock::new(None), me: SpinLock::new(None) });
        let w = Wrap { fut: Some(Box::pin(fut)), st: st.clone() };
        self.q.borrow_mut().push(Box::pin(w));
        Task { st: Some(st) }
    }
}

// ---------------------------------------------------------------- task handles

const T_DONE: u32 = 1;
const T_CANCEL: u32 = 2;
const T_DROPPED: u32 = 4; // finished without a value (cancelled, or its executor went away)

struct TaskState<T> {
    flags: AtomicU32,
    out: SpinLock<Option<T>>,
    /// a task awaiting the handle
    waiter: SpinLock<Option<Waker>>,
    /// the task's own waker (cancel wakes it so the executor drops the future)
    me: SpinLock<Option<Waker>>,
}

impl<T> TaskState<T> {
    fn finish(&self, v: Option<T>) {
        let bits = if v.is_some() { T_DONE } else { T_DONE | T_DROPPED };
        *self.out.lock() = v;
        self.flags.fetch_or(bits, SeqCst);
        *self.me.lock() = None;
        let w = self.waiter.lock().take();
        if let Some(w) = w {
            w.wake();
        }
    }

    fn cancel(&self) {
        if self.flags.load(SeqCst) & (T_DONE | T_CANCEL) != 0 || self.flags.fetch_or(T_CANCEL, SeqCst) & (T_DONE | T_CANCEL) != 0 {
            return;
        }
        let me = self.me.lock().clone();
        if let Some(w) = me {
            w.wake();
        }
    }
}

// the spawned future: its result into the handle's state; cancelled: dropped unpolled
struct Wrap<F: Future> {
    fut: Option<Pin<Box<F>>>,
    st: Arc<TaskState<F::Output>>,
}

impl<F: Future> Future for Wrap<F> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        if this.st.flags.load(SeqCst) & T_CANCEL != 0 {
            this.fut = None;
            this.st.finish(None);
            return Poll::Ready(());
        }
        let Some(f) = this.fut.as_mut() else { return Poll::Ready(()) };
        {
            let mut me = this.st.me.lock();
            if !me.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
                *me = Some(cx.waker().clone());
            }
        }
        match f.as_mut().poll(cx) {
            Poll::Ready(v) => {
                this.fut = None;
                this.st.finish(Some(v));
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<F: Future> Drop for Wrap<F> {
    fn drop(&mut self) {
        if self.fut.take().is_some() {
            self.st.finish(None); // the executor went away: the handle reports cancelled
        }
    }
}

/// the handle of a spawned task; dropping it cancels the task (detach keeps it running)
pub struct Task<T> {
    st: Option<Arc<TaskState<T>>>,
}

impl<T> Task<T> {
    /// finished, cancelled or dropped by its executor (the future is gone or goes with the
    /// executor's next run)
    pub fn is_done(&self) -> bool {
        self.st.as_ref().is_none_or(|s| s.flags.load(SeqCst) & T_DONE != 0)
    }

    /// it ended without a value: cancel() came first, or its executor went away
    pub fn is_cancelled(&self) -> bool {
        self.st.as_ref().is_some_and(|s| s.flags.load(SeqCst) & T_DROPPED != 0)
    }

    /// the result, once (None: still running, cancelled or already taken)
    pub fn take(&self) -> Option<T> {
        let s = self.st.as_ref()?;
        if s.flags.load(SeqCst) & T_DONE == 0 {
            return None;
        }
        s.out.lock().take()
    }

    /// stop the task: its future is dropped at the executor's next run (a finished task keeps
    /// its result)
    pub fn cancel(&self) {
        if let Some(s) = &self.st {
            s.cancel();
        }
    }

    /// let the task run on without a handle
    pub fn detach(mut self) {
        self.st = None;
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if let Some(s) = &self.st {
            s.cancel();
        }
    }
}

impl<T> Future for Task<T> {
    type Output = Result<T, Cancelled>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(s) = self.st.as_ref() else { return Poll::Ready(Err(Cancelled)) };
        if s.flags.load(SeqCst) & T_DONE == 0 {
            *s.waiter.lock() = Some(cx.waker().clone());
            if s.flags.load(SeqCst) & T_DONE == 0 {
                return Poll::Pending;
            }
        }
        Poll::Ready(s.out.lock().take().ok_or(Cancelled))
    }
}

// ---------------------------------------------------------------- timers

struct Timer {
    at: u64,
    id: u64,
    owner: u64,
    waker: Waker,
}

static TIMERS: SpinLock<Vec<Timer>> = SpinLock::new(Vec::new());
static TIMER_ID: AtomicU64 = AtomicU64::new(1);

/// a future ready ms milliseconds of the task clock from now
pub fn sleep(ms: u32) -> Sleep {
    sleep_until(now_us().saturating_add(ms as u64 * 1000))
}

/// a future ready when the task clock reaches at_us
pub fn sleep_until(at_us: u64) -> Sleep {
    Sleep { at: at_us, id: 0 }
}

/// the future of sleep / sleep_until
pub struct Sleep {
    at: u64,
    id: u64,
}

impl Sleep {
    /// when it is ready (task clock, us)
    pub fn deadline(&self) -> u64 {
        self.at
    }
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if now_us() >= self.at {
            if self.id != 0 {
                let id = core::mem::replace(&mut self.id, 0);
                TIMERS.lock().retain(|t| t.id != id);
            }
            return Poll::Ready(());
        }
        let mut g = TIMERS.lock();
        let found = if self.id == 0 { None } else { g.iter_mut().find(|t| t.id == self.id) };
        match found {
            Some(t) => {
                if !t.waker.will_wake(cx.waker()) {
                    t.waker = cx.waker().clone();
                }
                t.owner = tid();
            }
            None => {
                if self.id == 0 {
                    self.id = TIMER_ID.fetch_add(1, SeqCst);
                }
                g.push(Timer { at: self.at, id: self.id, owner: tid(), waker: cx.waker().clone() });
            }
        }
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if self.id != 0 {
            TIMERS.lock().retain(|t| t.id != self.id);
        }
    }
}

/// wake this thread's timers that are due at now, earliest deadline first -> how many
pub fn fire_timers(now: u64) -> usize {
    let me = tid();
    let mut due = Vec::new();
    {
        let mut g = TIMERS.lock();
        let mut i = 0;
        while i < g.len() {
            if g[i].owner == me && g[i].at <= now {
                let t = g.remove(i);
                due.push((t.at, t.id, t.waker));
            } else {
                i += 1;
            }
        }
    }
    due.sort_by_key(|d| (d.0, d.1)); // the same deadline: the older timer first
    let n = due.len();
    for (_, _, w) in due {
        w.wake();
    }
    n
}

/// the earliest timer of this thread (task clock, us)
pub fn next_timer() -> Option<u64> {
    let me = tid();
    TIMERS.lock().iter().filter(|t| t.owner == me).map(|t| t.at).min()
}

/// a future that lets the executor's other tasks run once
pub fn yield_now() -> YieldNow {
    YieldNow(false)
}

/// the future of yield_now
pub struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

// ---------------------------------------------------------------- the blocking pool

type BJob = Box<dyn FnOnce() + Send>;

struct BPool {
    jobs: VecDeque<BJob>,
    sem: Option<Arc<crate::Sem>>,
    threads: u32,
    idle: u32,
    no_threads: bool,
}

static BPOOL: SpinLock<BPool> = SpinLock::new(BPool { jobs: VecDeque::new(), sem: None, threads: 0, idle: 0, no_threads: false });
static INFLIGHT: AtomicU32 = AtomicU32::new(0);

/// blocking jobs submitted and not finished yet (every executor's)
pub fn blocking_pending() -> u32 {
    INFLIGHT.load(SeqCst)
}

fn submit(job: BJob) {
    let mut g = BPOOL.lock();
    g.jobs.push_back(job);
    if g.no_threads {
        return; // the executors run it (run_inline)
    }
    if g.sem.is_none() {
        g.sem = crate::Sem::new(0).map(Arc::new);
    }
    let Some(sem) = g.sem.clone() else {
        g.no_threads = true;
        return;
    };
    let start = g.jobs.len() as u32 > g.idle && g.threads < BLOCKING_MAX;
    if start {
        g.threads += 1;
    }
    drop(g);
    if start {
        let s = sem.clone();
        // the workers live as long as the process: the handle is never joined
        if crate::thread_spawn(BLOCKING_STACK, Box::new(move || blocking_worker(s))).is_none() {
            let mut g = BPOOL.lock();
            g.threads -= 1;
            if g.threads == 0 {
                g.no_threads = true;
            }
        }
    }
    sem.post(1);
}

fn blocking_worker(sem: Arc<crate::Sem>) {
    loop {
        let job = {
            let mut g = BPOOL.lock();
            let j = g.jobs.pop_front();
            if j.is_none() {
                g.idle += 1;
            }
            j
        };
        match job {
            Some(j) => j(),
            None => {
                sem.wait(-1);
                BPOOL.lock().idle -= 1;
            }
        }
    }
}

fn inline_pending() -> bool {
    let g = BPOOL.lock();
    g.no_threads && !g.jobs.is_empty()
}

// where no thread can start: up to n queued jobs on this thread
fn run_inline(n: usize) {
    for _ in 0..n {
        let job = {
            let mut g = BPOOL.lock();
            if !g.no_threads {
                return;
            }
            g.jobs.pop_front()
        };
        match job {
            Some(j) => j(),
            None => return,
        }
    }
}

struct Cell<T> {
    val: SpinLock<Option<T>>,
    waker: SpinLock<Option<Waker>>,
    done: AtomicBool,
}

/// the result of spawn_blocking: a future of f's value
pub struct Blocking<T> {
    cell: Arc<Cell<T>>,
    tok: CancelToken,
    taken: bool,
}

/// run f on the blocking pool (a thread of its own: files, network, parallel_for loops)
pub fn spawn_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Blocking<T> {
    spawn_cancellable(move |_| f())
}

/// run f on the blocking pool; its token is cancelled when the Blocking future is dropped
/// before the result was taken (a job not started yet is skipped)
pub fn spawn_cancellable<T: Send + 'static>(f: impl FnOnce(&CancelToken) -> T + Send + 'static) -> Blocking<T> {
    let cell = Arc::new(Cell { val: SpinLock::new(None), waker: SpinLock::new(None), done: AtomicBool::new(false) });
    let tok = CancelToken::new();
    let (c, t) = (cell.clone(), tok.clone());
    let trusted = crate::policy::trusted_here();
    let scopes = crate::policy::scopes_here(); // a plugin's job keeps its rights
    INFLIGHT.fetch_add(1, SeqCst);
    submit(Box::new(move || {
        if !t.is_cancelled() {
            let run = || if trusted { crate::policy::trusted_scope(|| f(&t)) } else { f(&t) };
            let v = crate::policy::in_scopes(&scopes, run);
            *c.val.lock() = Some(v);
        } else {
            drop(f); // what it holds (an AtomicFile ...) goes before the job counts as done
        }
        c.done.store(true, SeqCst);
        let w = c.waker.lock().take();
        if let Some(w) = w {
            w.wake();
        }
        INFLIGHT.fetch_sub(1, SeqCst); // after the wake-up: busy() never sees a gap
    }));
    Blocking { cell, tok, taken: false }
}

impl<T> Blocking<T> {
    /// the job has finished (polling gives its value now)
    pub fn is_done(&self) -> bool {
        self.cell.done.load(SeqCst)
    }
}

impl<T> Future for Blocking<T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        if !self.cell.done.load(SeqCst) {
            *self.cell.waker.lock() = Some(cx.waker().clone());
            if !self.cell.done.load(SeqCst) {
                return Poll::Pending;
            }
        }
        let v = self.cell.val.lock().take();
        match v {
            Some(v) => {
                self.taken = true;
                Poll::Ready(v)
            }
            None => Poll::Pending, // polled again after the value was taken
        }
    }
}

impl<T> Drop for Blocking<T> {
    fn drop(&mut self) {
        if !self.taken {
            self.tok.cancel();
        }
    }
}

// ---------------------------------------------------------------- cancellation

struct CancelInner {
    flag: AtomicBool,
    waiters: SpinLock<Vec<(u64, Waker)>>,
}

/// a shared cancel flag: cancel() from any thread, checked between chunks of work or awaited
#[derive(Clone)]
pub struct CancelToken(Arc<CancelInner>);

static WAIT_ID: AtomicU64 = AtomicU64::new(1);

impl CancelToken {
    /// a token that is not cancelled
    pub fn new() -> CancelToken {
        CancelToken(Arc::new(CancelInner { flag: AtomicBool::new(false), waiters: SpinLock::new(Vec::new()) }))
    }

    /// set the flag; every cancelled() future and run() wrapper wakes up
    pub fn cancel(&self) {
        if self.0.flag.swap(true, SeqCst) {
            return;
        }
        let w = core::mem::take(&mut *self.0.waiters.lock());
        for (_, w) in w {
            w.wake();
        }
    }

    /// cancel() was called
    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(SeqCst)
    }

    /// Err(Cancelled) once the token is cancelled (`token.check()?` between chunks)
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() { Err(Cancelled) } else { Ok(()) }
    }

    /// a future that is ready once the token is cancelled
    pub fn cancelled(&self) -> WaitCancelled {
        WaitCancelled { tok: self.clone(), id: 0 }
    }

    /// fut until the token is cancelled: Ok(its value), or Err(Cancelled) (fut is dropped)
    pub fn run<F: Future>(&self, fut: F) -> Guarded<F> {
        Guarded { fut: Some(Box::pin(fut)), wait: self.cancelled() }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        CancelToken::new()
    }
}

/// the future of CancelToken::cancelled
pub struct WaitCancelled {
    tok: CancelToken,
    id: u64,
}

impl Future for WaitCancelled {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.tok.is_cancelled() {
            return Poll::Ready(());
        }
        if self.id == 0 {
            self.id = WAIT_ID.fetch_add(1, SeqCst);
        }
        {
            let mut g = self.tok.0.waiters.lock();
            match g.iter_mut().find(|(i, _)| *i == self.id) {
                Some((_, w)) => {
                    if !w.will_wake(cx.waker()) {
                        *w = cx.waker().clone();
                    }
                }
                None => g.push((self.id, cx.waker().clone())),
            }
        }
        if self.tok.is_cancelled() { Poll::Ready(()) } else { Poll::Pending }
    }
}

impl Drop for WaitCancelled {
    fn drop(&mut self) {
        if self.id != 0 {
            let id = self.id;
            self.tok.0.waiters.lock().retain(|(i, _)| *i != id);
        }
    }
}

/// the future of CancelToken::run
pub struct Guarded<F: Future> {
    fut: Option<Pin<Box<F>>>,
    wait: WaitCancelled,
}

impl<F: Future> Unpin for Guarded<F> {}

impl<F: Future> Future for Guarded<F> {
    type Output = Result<F::Output, Cancelled>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        if this.wait.tok.is_cancelled() {
            this.fut = None;
            return Poll::Ready(Err(Cancelled));
        }
        let Some(f) = this.fut.as_mut() else { return Poll::Ready(Err(Cancelled)) };
        if let Poll::Ready(v) = f.as_mut().poll(cx) {
            this.fut = None;
            return Poll::Ready(Ok(v));
        }
        if Pin::new(&mut this.wait).poll(cx).is_ready() {
            this.fut = None;
            return Poll::Ready(Err(Cancelled));
        }
        Poll::Pending
    }
}

// ---------------------------------------------------------------- progress

struct ProgInner {
    done: AtomicU64,
    total: AtomicU64,
    seq: AtomicU64,
    last_wake: AtomicU64,
    waiters: SpinLock<Vec<Waker>>,
}

/// the progress of an operation: the worker reports, the UI reads (cheap clones share it)
#[derive(Clone)]
pub struct Progress(Arc<ProgInner>);

impl Progress {
    /// nothing done, total unknown
    pub fn new() -> Progress {
        Progress(Arc::new(ProgInner {
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            seq: AtomicU64::new(0),
            last_wake: AtomicU64::new(0),
            waiters: SpinLock::new(Vec::new()),
        }))
    }

    /// the amount of work (bytes, items); 0 = unknown
    pub fn set_total(&self, n: u64) {
        self.0.total.store(n, SeqCst);
        self.report();
    }

    /// the work done so far
    pub fn set(&self, done: u64) {
        self.0.done.store(done, SeqCst);
        self.report();
    }

    /// n more done
    pub fn add(&self, n: u64) {
        self.0.done.fetch_add(n, SeqCst);
        self.report();
    }

    /// the work done so far
    pub fn done(&self) -> u64 {
        self.0.done.load(SeqCst)
    }

    /// the amount of work (0: unknown)
    pub fn total(&self) -> u64 {
        self.0.total.load(SeqCst)
    }

    /// done / total in 0..=1000 (0 while the total is unknown)
    pub fn permille(&self) -> u32 {
        let (d, t) = (self.done(), self.total());
        if t == 0 { 0 } else { (d.min(t) as u128 * 1000 / t as u128) as u32 }
    }

    /// reports so far (changed() waits for the next one)
    pub fn reports(&self) -> u64 {
        self.0.seq.load(SeqCst)
    }

    /// a future that is ready after the next report
    pub fn changed(&self) -> Changed {
        Changed { p: self.clone(), seen: self.reports() }
    }

    // wake the waiters; plat::wake at most every PROGRESS_WAKE_MS (always once done == total)
    fn report(&self) {
        self.0.seq.fetch_add(1, SeqCst);
        let w = core::mem::take(&mut *self.0.waiters.lock());
        for w in w {
            w.wake();
        }
        let now = crate::time_us();
        let last = self.0.last_wake.load(SeqCst);
        let end = self.total() != 0 && self.done() >= self.total();
        if end || now.wrapping_sub(last) >= PROGRESS_WAKE_MS * 1000 {
            self.0.last_wake.store(now, SeqCst);
            crate::wake();
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Progress::new()
    }
}

/// the future of Progress::changed
pub struct Changed {
    p: Progress,
    seen: u64,
}

impl Future for Changed {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.p.reports() != self.seen {
            return Poll::Ready(());
        }
        {
            let mut g = self.p.0.waiters.lock();
            if !g.iter().any(|w| w.will_wake(cx.waker())) {
                g.push(cx.waker().clone());
            }
        }
        if self.p.reports() != self.seen { Poll::Ready(()) } else { Poll::Pending }
    }
}

// ---------------------------------------------------------------- joins

enum MaybeDone<F: Future> {
    Run(Pin<Box<F>>),
    Done(Option<F::Output>),
}

impl<F: Future> MaybeDone<F> {
    fn new(f: F) -> Self {
        MaybeDone::Run(Box::pin(f))
    }
    // true once done
    fn poll(&mut self, cx: &mut Context<'_>) -> bool {
        if let MaybeDone::Run(f) = self {
            match f.as_mut().poll(cx) {
                Poll::Ready(v) => *self = MaybeDone::Done(Some(v)),
                Poll::Pending => return false,
            }
        }
        true
    }
    fn take(&mut self) -> F::Output {
        match self {
            MaybeDone::Done(v) => v.take().expect("join: polled after completion"),
            MaybeDone::Run(_) => unreachable!(),
        }
    }
}

/// the future of join
pub struct Join<A: Future, B: Future>(MaybeDone<A>, MaybeDone<B>);
impl<A: Future, B: Future> Unpin for Join<A, B> {}

/// both futures, concurrently on the same task -> (a's value, b's value)
pub fn join<A: Future, B: Future>(a: A, b: B) -> Join<A, B> {
    Join(MaybeDone::new(a), MaybeDone::new(b))
}

impl<A: Future, B: Future> Future for Join<A, B> {
    type Output = (A::Output, B::Output);
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let a = this.0.poll(cx);
        let b = this.1.poll(cx);
        if a && b { Poll::Ready((this.0.take(), this.1.take())) } else { Poll::Pending }
    }
}

/// the future of join3
pub struct Join3<A: Future, B: Future, C: Future>(MaybeDone<A>, MaybeDone<B>, MaybeDone<C>);
impl<A: Future, B: Future, C: Future> Unpin for Join3<A, B, C> {}

/// three futures concurrently -> their values
pub fn join3<A: Future, B: Future, C: Future>(a: A, b: B, c: C) -> Join3<A, B, C> {
    Join3(MaybeDone::new(a), MaybeDone::new(b), MaybeDone::new(c))
}

impl<A: Future, B: Future, C: Future> Future for Join3<A, B, C> {
    type Output = (A::Output, B::Output, C::Output);
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let a = this.0.poll(cx);
        let b = this.1.poll(cx);
        let c = this.2.poll(cx);
        if a && b && c { Poll::Ready((this.0.take(), this.1.take(), this.2.take())) } else { Poll::Pending }
    }
}

/// the future of join_all
pub struct JoinAll<F: Future>(Vec<MaybeDone<F>>);
impl<F: Future> Unpin for JoinAll<F> {}

/// every future concurrently -> their values in order
pub fn join_all<F: Future>(v: impl IntoIterator<Item = F>) -> JoinAll<F> {
    JoinAll(v.into_iter().map(MaybeDone::new).collect())
}

impl<F: Future> Future for JoinAll<F> {
    type Output = Vec<F::Output>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut all = true;
        for f in self.0.iter_mut() {
            all &= f.poll(cx);
        }
        if all { Poll::Ready(self.0.iter_mut().map(|f| f.take()).collect()) } else { Poll::Pending }
    }
}

/// which of two futures finished first
#[derive(Debug, PartialEq, Eq)]
pub enum Either<A, B> {
    Left(A),
    Right(B),
}

/// the future of select
pub struct Select<A: Future, B: Future>(Pin<Box<A>>, Pin<Box<B>>);

/// the first of two futures to finish (polled in order a, b); the other one is dropped
pub fn select<A: Future, B: Future>(a: A, b: B) -> Select<A, B> {
    Select(Box::pin(a), Box::pin(b))
}

impl<A: Future, B: Future> Future for Select<A, B> {
    type Output = Either<A::Output, B::Output>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(v) = self.0.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(v));
        }
        if let Poll::Ready(v) = self.1.as_mut().poll(cx) {
            return Poll::Ready(Either::Right(v));
        }
        Poll::Pending
    }
}
