// pool -- the worker thread pool: parallel loops and background jobs     [src/core/thread.s]
//
//   thr_init(n) -> workers          start the pool once: n workers (0 = cpu_count), at most
//                                   THR_MAX; `--threads N` on the command line wins.  Later
//                                   calls return the count; every function below starts it
//   thr_count() -> workers          thr_shutdown()   stop and join the workers (no job running)
//   parallel_for(count, &f) -> Ok / Err(Cancelled)
//        f(index, job) for 0..count on the workers AND the calling thread; returns when every
//        item is done.  f may call job.cancel() to skip the remaining items.  Nested loops
//        (from inside f) are fine.
//   Job: a background job -- set(count, f), start() (false: still running), wait(),
//        cancel(), done(), items_done().  When it finishes the pool calls wake() (E_WAKE).
//   thr_self() -> slot              0..n-1 on worker k, n on any other thread: per-thread
//                                   arrays of thr_count() + 1 entries
//
// Design (as in ASMLIB): one queue of runnable jobs under a short spin lock (loops at the
// head, background jobs at the tail).  A thread takes part in a job by counting itself in
// `active` under the lock, then claims item indexes with fetch_add on `next`.  The thread
// that sees every index handed out (or the cancel flag) unlinks the job; the last one to
// leave finishes it: state DONE (from then on the owner may drop the record), wake() for
// background jobs, and every thread sleeping in a job wait is woken.  Idle workers spin
// briefly, then sleep on a semaphore; waiters spin, then sleep on a second one at most
// WAIT_POLL_MS at a time (a semaphore post may be taken by another waiter).  Work depends
// only on the item index, so results are the same for any number of threads.
//
// The loop record lives on the caller's stack and holds a pointer to the borrowed closure:
// parallel_for returns only after the last participant left, which is what makes handing
// the borrow to the workers sound (the unsafe below).

use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering::SeqCst};

use crate::{Sem, Thread, cpu_count, thread_spawn, thread_yield, wake};

pub const THR_MAX: usize = 64;
const SPIN_LOCK: u32 = 64; // pause rounds before yielding on the queue lock
const SPIN_IDLE: u32 = 256; // worker: pause rounds watching the queue before sleeping
const SPIN_WAIT: u32 = 2000; // job waiter: pause rounds before sleeping
const WAIT_POLL_MS: i32 = 2; // job waiter: longest sleep before it looks at the job again
const THR_STACK: usize = 0x10_0000; // 1 MB per worker
const JF_QUEUED: u32 = 1;
const JF_ASYNC: u32 = 2;
const TS_NONE: u32 = 0;
const TS_INIT: u32 = 1;
const TS_READY: u32 = 2;
const JS_RUNNING: u32 = 1;
const JS_DONE: u32 = 2;

/// a loop or job ended through cancel()
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

type ItemFn = dyn Fn(usize, &JobCtl) + Sync;

/// a job as its item functions see it: cancel the rest, look at the progress
pub struct JobCtl {
    f: UnsafeCell<*const ItemFn>, // fixed while the job runs
    count: UnsafeCell<u64>,       // fixed while the job runs
    next: AtomicU64,              // next index to hand out
    done: AtomicU64,              // items finished
    cancel: AtomicU32,
    state: AtomicU32,
    active: UnsafeCell<u32>,           // threads working on it (queue lock)
    flags: UnsafeCell<u32>,            // JF_* (queue lock)
    link: UnsafeCell<*const JobCtl>,   // next job in the queue (queue lock)
}

// SAFETY: the UnsafeCell fields are written under the queue lock or while no other thread
// can see the job (not running); everything else is atomic.
unsafe impl Sync for JobCtl {}
unsafe impl Send for JobCtl {}

impl JobCtl {
    fn new(f: *const ItemFn, count: u64) -> JobCtl {
        JobCtl {
            f: UnsafeCell::new(f),
            count: UnsafeCell::new(count),
            next: AtomicU64::new(0),
            done: AtomicU64::new(0),
            cancel: AtomicU32::new(0),
            state: AtomicU32::new(0),
            active: UnsafeCell::new(0),
            flags: UnsafeCell::new(0),
            link: UnsafeCell::new(core::ptr::null()),
        }
    }

    /// skip the items not started yet (callable from inside an item)  [thr_job_cancel]
    pub fn cancel(&self) {
        job_cancel(self)
    }

    /// long items poll this
    pub fn cancelled(&self) -> bool {
        self.cancel.load(SeqCst) != 0
    }

    pub fn count(&self) -> u64 {
        // SAFETY: fixed while the job runs; the owner changes it only while it does not.
        unsafe { *self.count.get() }
    }

    /// items finished so far (progress = items_done / count)
    pub fn items_done(&self) -> u64 {
        self.done.load(SeqCst)
    }

    fn result(&self) -> Result<(), Cancelled> {
        if self.cancel.load(SeqCst) != 0 { Err(Cancelled) } else { Ok(()) }
    }
}

// ---------------------------------------------------------------- the pool

struct Pool {
    lock: AtomicU32,
    head: AtomicPtr<JobCtl>,
    tail: AtomicPtr<JobCtl>,
    work: UnsafeCell<Option<Sem>>,     // work was queued (written by init / shutdown only)
    donesem: UnsafeCell<Option<Sem>>,  // a job finished (dwait sleepers)
    dwait: AtomicI32,
    state: AtomicU32,
    n: AtomicU32,
    quit: AtomicU32,
    threads: UnsafeCell<[Option<Thread>; THR_MAX]>,
    stk: [AtomicUsize; THR_MAX], // a stack address of each worker's first frame (thr_self)
}

// SAFETY: the UnsafeCell fields change only in thr_init / thr_shutdown, which no job may
// overlap (as in ASMLIB); the rest is atomic.
unsafe impl Sync for Pool {}

static POOL: Pool = Pool {
    lock: AtomicU32::new(0),
    head: AtomicPtr::new(core::ptr::null_mut()),
    tail: AtomicPtr::new(core::ptr::null_mut()),
    work: UnsafeCell::new(None),
    donesem: UnsafeCell::new(None),
    dwait: AtomicI32::new(0),
    state: AtomicU32::new(TS_NONE),
    n: AtomicU32::new(0),
    quit: AtomicU32::new(0),
    threads: UnsafeCell::new([const { None }; THR_MAX]),
    stk: [const { AtomicUsize::new(0) }; THR_MAX],
};

fn work_sem() -> Option<&'static Sem> {
    // SAFETY: see Pool.
    unsafe { (*POOL.work.get()).as_ref() }
}

fn done_sem() -> Option<&'static Sem> {
    // SAFETY: see Pool.
    unsafe { (*POOL.donesem.get()).as_ref() }
}

/// start the pool (once): the number of worker threads  [thr_init]
pub fn thr_init(n: u32) -> u32 {
    loop {
        if POOL.state.load(SeqCst) == TS_READY {
            return POOL.n.load(SeqCst);
        }
        if POOL.state.compare_exchange(TS_NONE, TS_INIT, SeqCst, SeqCst).is_ok() {
            break;
        }
        thread_yield(); // another thread is starting the pool
    }
    let mut want = n as usize;
    if let Some(v) = crate::arg_value("--threads").and_then(|v| v.parse::<i32>().ok()) {
        if v > 0 {
            want = v as usize;
        }
    }
    if want == 0 {
        want = cpu_count() as usize;
    }
    let want = want.min(THR_MAX);
    POOL.quit.store(0, SeqCst);
    POOL.head.store(core::ptr::null_mut(), SeqCst);
    POOL.tail.store(core::ptr::null_mut(), SeqCst);
    POOL.dwait.store(0, SeqCst);
    let mut started = 0;
    // SAFETY: TS_INIT: this thread alone touches the pool's cells.
    unsafe {
        *POOL.work.get() = Sem::new(0);
        *POOL.donesem.get() = Sem::new(0);
        if work_sem().is_some() && done_sem().is_some() {
            let threads = &mut *POOL.threads.get();
            while started < want {
                let k = started;
                match thread_spawn(THR_STACK, Box::new(move || worker(k))) {
                    Some(t) => threads[k] = Some(t),
                    None => break,
                }
                started += 1;
            }
        }
    }
    POOL.n.store(started as u32, SeqCst);
    POOL.state.swap(TS_READY, SeqCst);
    started as u32
}

/// the number of worker threads (starts the pool on first use)  [thr_count]
pub fn thr_count() -> u32 {
    if POOL.state.load(SeqCst) == TS_READY { POOL.n.load(SeqCst) } else { thr_init(0) }
}

/// stop the workers once the queue is empty, join them; a later call starts a new pool
pub fn thr_shutdown() {
    if POOL.state.load(SeqCst) != TS_READY {
        return;
    }
    POOL.quit.store(1, SeqCst);
    let n = POOL.n.load(SeqCst);
    if let Some(s) = work_sem() {
        s.post(n);
    }
    // SAFETY: no job runs (the caller's promise); the workers only read the cells.
    unsafe {
        let threads = &mut *POOL.threads.get();
        for t in threads.iter_mut().take(n as usize) {
            if let Some(t) = t.take() {
                t.join();
            }
        }
        *POOL.work.get() = None;
        *POOL.donesem.get() = None;
    }
    POOL.n.store(0, SeqCst);
    POOL.state.store(TS_NONE, SeqCst);
}

// ---------------------------------------------------------------- queue (internal)

fn q_lock() {
    loop {
        if POOL.lock.swap(1, SeqCst) == 0 {
            return;
        }
        let mut spin = SPIN_LOCK;
        loop {
            core::hint::spin_loop();
            if POOL.lock.load(SeqCst) == 0 {
                break;
            }
            spin -= 1;
            if spin == 0 {
                thread_yield();
                break;
            }
        }
    }
}

fn q_unlock() {
    POOL.lock.store(0, SeqCst);
}

// The queue functions run with the lock held; job pointers in the queue are alive (a job
// is unlinked before it finishes).
unsafe fn q_push_head(j: *const JobCtl) {
    unsafe {
        *(*j).link.get() = POOL.head.load(SeqCst);
        POOL.head.store(j as *mut JobCtl, SeqCst);
        if POOL.tail.load(SeqCst).is_null() {
            POOL.tail.store(j as *mut JobCtl, SeqCst);
        }
        *(*j).flags.get() |= JF_QUEUED;
    }
}

unsafe fn q_push_tail(j: *const JobCtl) {
    unsafe {
        *(*j).link.get() = core::ptr::null();
        let t = POOL.tail.load(SeqCst);
        if t.is_null() {
            POOL.head.store(j as *mut JobCtl, SeqCst);
        } else {
            *(*t).link.get() = j;
        }
        POOL.tail.store(j as *mut JobCtl, SeqCst);
        *(*j).flags.get() |= JF_QUEUED;
    }
}

unsafe fn q_unlink(j: *const JobCtl) {
    unsafe {
        let mut prev: *const JobCtl = core::ptr::null();
        let mut cur = POOL.head.load(SeqCst) as *const JobCtl;
        while !cur.is_null() {
            if cur == j {
                let next = *(*j).link.get();
                if prev.is_null() {
                    POOL.head.store(next as *mut JobCtl, SeqCst);
                } else {
                    *(*prev).link.get() = next;
                }
                if POOL.tail.load(SeqCst) as *const JobCtl == j {
                    POOL.tail.store(prev as *mut JobCtl, SeqCst);
                }
                *(*j).flags.get() &= !JF_QUEUED;
                return;
            }
            prev = cur;
            cur = *(*cur).link.get();
        }
    }
}

/// the first queued job, now counting this thread in `active`, or null
fn pick() -> *const JobCtl {
    q_lock();
    let h = POOL.head.load(SeqCst) as *const JobCtl;
    if !h.is_null() {
        // SAFETY: lock held; queued jobs are alive.
        unsafe { *(*h).active.get() += 1 };
    }
    q_unlock();
    h
}

/// claim and run items until none are left or the job is cancelled
fn run_items(j: &JobCtl) {
    // SAFETY: f is fixed and alive while the job runs (parallel_for / Job keep it).
    let f = unsafe { &**j.f.get() };
    let count = j.count();
    while j.cancel.load(SeqCst) == 0 {
        let i = j.next.fetch_add(1, SeqCst);
        if i >= count {
            break;
        }
        f(i as usize, j);
        j.done.fetch_add(1, SeqCst);
    }
}

/// stop taking part in a job; the last participant finishes it (a raw pointer: the
/// owner may drop the job as soon as finish marked it DONE)
fn leave(j: *const JobCtl) {
    q_lock();
    // SAFETY: lock held; this thread is still counted in `active`, so the job is alive.
    let fin = unsafe {
        let j = &*j;
        if *j.flags.get() & JF_QUEUED != 0
            && (j.cancel.load(SeqCst) != 0 || j.next.load(SeqCst) >= j.count())
        {
            q_unlink(j); // every index is handed out: nothing left to pick
        }
        *j.active.get() -= 1;
        *j.active.get() == 0 && *j.flags.get() & JF_QUEUED == 0
    };
    q_unlock();
    if fin {
        finish(j);
    }
}

/// DONE (the owner may drop the record from then on), wake() for background jobs, wake
/// every thread sleeping in wait_done
fn finish(j: *const JobCtl) {
    // SAFETY: the job is alive until its state is DONE; flags are read before that.
    unsafe {
        let flags = *(*j).flags.get();
        (*j).state.swap(JS_DONE, SeqCst);
        // (j must not be touched from here on)
        finish_rest(flags);
    }
}

fn finish_rest(flags: u32) {
    if flags & JF_ASYNC != 0 {
        wake();
    }
    let d = POOL.dwait.swap(0, SeqCst);
    if d > 0 {
        if let Some(s) = done_sem() {
            s.post(d as u32);
        }
    }
}

/// block until the job is DONE: a short spin, then the done semaphore WAIT_POLL_MS at a time
fn wait_done(j: &JobCtl) {
    for _ in 0..SPIN_WAIT {
        if j.state.load(SeqCst) == JS_DONE {
            return;
        }
        core::hint::spin_loop();
    }
    loop {
        // register first, then check: finish() sets the state before it reads the count
        POOL.dwait.fetch_add(1, SeqCst);
        if j.state.load(SeqCst) == JS_DONE {
            return;
        }
        match done_sem() {
            Some(s) => {
                if !s.wait(WAIT_POLL_MS) {
                    // timed out: take the registration back (unless finish made it a post)
                    let mut v = POOL.dwait.load(SeqCst);
                    while v > 0 {
                        match POOL.dwait.compare_exchange(v, v - 1, SeqCst, SeqCst) {
                            Ok(_) => break,
                            Err(x) => v = x,
                        }
                    }
                }
            }
            None => thread_yield(),
        }
        if j.state.load(SeqCst) == JS_DONE {
            return;
        }
    }
}

fn worker(k: usize) {
    let top = 0u8;
    POOL.stk[k].store(core::hint::black_box(&top) as *const u8 as usize, SeqCst);
    'outer: loop {
        let j = pick();
        if !j.is_null() {
            // SAFETY: pick counted this thread in `active`: the job stays alive until leave.
            let j = unsafe { &*j };
            run_items(j);
            leave(j);
            continue;
        }
        if POOL.quit.load(SeqCst) != 0 {
            break;
        }
        for _ in 0..SPIN_IDLE {
            core::hint::spin_loop();
            if !POOL.head.load(SeqCst).is_null() {
                continue 'outer;
            }
        }
        if let Some(s) = work_sem() {
            s.wait(-1);
        }
    }
    POOL.stk[k].store(0, SeqCst);
}

// ---------------------------------------------------------------- loops and jobs

/// f(index, job) for every index in 0..count, on the workers and this thread  [thr_parallel_for]
pub fn parallel_for(count: usize, f: &(dyn Fn(usize, &JobCtl) + Sync)) -> Result<(), Cancelled> {
    if count == 0 {
        return Ok(());
    }
    // a plugin's loop (policy::on_behalf): its items keep the plugin's rights on every worker
    let scopes = crate::policy::scopes_here();
    if !scopes.is_empty() {
        let s = &scopes;
        return loop_items(count, &|i, j| crate::policy::in_scopes(s, || f(i, j)));
    }
    loop_items(count, f)
}

fn loop_items(count: usize, f: &(dyn Fn(usize, &JobCtl) + Sync)) -> Result<(), Cancelled> {
    // SAFETY: the lifetime of f is erased for the record; the workers use it only until the
    // last of them left the job, and this function returns only after that (wait_done).
    let fp: *const ItemFn = unsafe { core::mem::transmute::<&(dyn Fn(usize, &JobCtl) + Sync), *const ItemFn>(f) };
    let job = JobCtl::new(fp, count as u64);
    job.state.store(JS_RUNNING, SeqCst);
    // SAFETY: not shared yet.
    unsafe { *job.active.get() = 1 }; // the calling thread
    let n = thr_count();
    if n == 0 || count <= 1 {
        run_items(&job);
        job.state.store(JS_DONE, SeqCst);
    } else {
        q_lock();
        // SAFETY: lock held; job outlives its time in the queue (see above).
        unsafe { q_push_head(&job) }; // the caller waits: before background jobs
        q_unlock();
        if let Some(s) = work_sem() {
            s.post((count - 1).min(n as usize) as u32);
        }
        run_items(&job);
        leave(&job);
        wait_done(&job);
    }
    job.result()
}

/// cancel: items not started are skipped; a queued job nobody works on finishes at once
/// (callers keep j alive: its owner, or an item while it runs)
fn job_cancel(j: &JobCtl) {
    j.cancel.swap(1, SeqCst);
    if j.state.load(SeqCst) != JS_RUNNING {
        return;
    }
    q_lock();
    // SAFETY: lock held.
    let fin = unsafe {
        if *j.flags.get() & JF_QUEUED != 0 && *j.active.get() == 0 {
            q_unlink(j);
            true
        } else {
            false
        }
    };
    q_unlock();
    if fin {
        finish(j);
    }
}

/// a background job: count items of f on the workers; wake() when it is finished
pub struct Job {
    rec: Box<JobCtl>,
    f: Option<Box<dyn Fn(usize, &JobCtl) + Send + Sync>>,
}

impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}

impl Job {
    pub fn new() -> Job {
        Job { rec: Box::new(JobCtl::new(core::ptr::null::<fn(usize, &JobCtl)>() as *const ItemFn, 0)), f: None }
    }

    fn running(&self) -> bool {
        self.rec.state.load(SeqCst) == JS_RUNNING
    }

    /// the work of the next start: count items of f (false while the job runs)
    pub fn set(&mut self, count: u64, f: impl Fn(usize, &JobCtl) + Send + Sync + 'static) -> bool {
        if self.running() {
            return false;
        }
        let b: Box<dyn Fn(usize, &JobCtl) + Send + Sync> = Box::new(f);
        // SAFETY: not running: no worker reads the cells; the box lives in self.f.
        unsafe {
            *self.rec.f.get() = &*b as &(dyn Fn(usize, &JobCtl) + Sync) as *const ItemFn;
            *self.rec.count.get() = count;
        }
        self.f = Some(b);
        true
    }

    /// another item count for the same function (false while the job runs)
    pub fn set_count(&mut self, count: u64) -> bool {
        if self.running() {
            return false;
        }
        // SAFETY: not running.
        unsafe { *self.rec.count.get() = count };
        true
    }

    /// run it in the background; false: it is still running  [thr_job_start]
    pub fn start(&self) -> bool {
        let j: &JobCtl = &self.rec;
        if self.running() {
            return false;
        }
        let n = thr_count();
        j.next.store(0, SeqCst);
        j.done.store(0, SeqCst);
        j.cancel.store(0, SeqCst);
        // SAFETY: not running: nobody else sees the cells.
        unsafe {
            *j.link.get() = core::ptr::null();
            *j.active.get() = 0;
            *j.flags.get() = JF_ASYNC;
        }
        j.state.store(JS_RUNNING, SeqCst);
        if j.count() == 0 || self.f.is_none() {
            finish(j);
            return true;
        }
        if n == 0 {
            // SAFETY: not shared.
            unsafe { *j.active.get() = 1 }; // no workers: run it right here
            run_items(j);
            finish(j);
            return true;
        }
        q_lock();
        // SAFETY: lock held; the record lives in a Box that Drop keeps until the job is done.
        unsafe { q_push_tail(j) };
        q_unlock();
        if let Some(s) = work_sem() {
            s.post(j.count().min(n as u64) as u32);
        }
        true
    }

    /// block until the job is finished (this thread helps with items not taken yet)
    pub fn wait(&self) -> Result<(), Cancelled> {
        let j: &JobCtl = &self.rec;
        if self.running() {
            q_lock();
            // SAFETY: lock held.
            let help = unsafe {
                if *j.flags.get() & JF_QUEUED != 0 {
                    *j.active.get() += 1;
                    true
                } else {
                    false
                }
            };
            q_unlock();
            if help {
                run_items(j);
                leave(j);
            }
            wait_done(j);
        }
        j.result()
    }

    /// skip the items not started yet; the job still finishes (wake)
    pub fn cancel(&self) {
        job_cancel(&self.rec)
    }

    /// finished (or never started)
    pub fn done(&self) -> bool {
        !self.running()
    }

    pub fn items_done(&self) -> u64 {
        self.rec.items_done()
    }

    pub fn count(&self) -> u64 {
        self.rec.count()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.running() {
            self.cancel();
            let _ = self.wait();
        }
    }
}

/// the calling thread's slot: k on worker k, the worker count on any other thread  [thr_self]
pub fn thr_self() -> usize {
    let here = 0u8;
    let sp = core::hint::black_box(&here) as *const u8 as usize;
    let n = (POOL.n.load(SeqCst) as usize).min(THR_MAX);
    for k in 0..n {
        let top = POOL.stk[k].load(SeqCst);
        if top != 0 && sp <= top && sp > top.wrapping_sub(THR_STACK - 65536) {
            return k;
        }
    }
    n
}
