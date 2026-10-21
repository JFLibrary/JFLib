// Cooperative worker jobs for the browser. Parallel loops run sequentially in the app
// worker; background jobs advance a bounded number of items between browser UI steps.

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use rt::sync::SpinLock;

/// Maximum native worker count; the browser has zero auxiliary compute threads.
pub const THR_MAX: usize = 64;
/// The caller cancelled a loop or background job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

/// Cooperative progress and cancellation state exposed to one work item.
pub struct JobCtl {
    count: usize,
    done: AtomicUsize,
    cancel: AtomicBool,
    running: AtomicBool,
}
impl JobCtl {
    fn new(count: usize) -> Self {
        Self {
            count,
            done: AtomicUsize::new(0),
            cancel: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }
    /// Cancel work not yet started; long items can poll `cancelled`.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    /// Whether cancellation has been requested.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Number of items in this job.
    pub fn count(&self) -> u64 {
        self.count as u64
    }
    /// Number of completely processed items.
    pub fn items_done(&self) -> u64 {
        self.done.load(Ordering::Relaxed) as u64
    }
}

struct Record {
    ctl: JobCtl,
    function: Option<Box<dyn Fn(usize, &JobCtl) + Send + Sync>>,
}
static QUEUE: SpinLock<Vec<Arc<Record>>> = SpinLock::new(Vec::new());

fn step(record: &Record) -> bool {
    let ctl = &record.ctl;
    if !ctl.running.load(Ordering::Relaxed) {
        return false;
    }
    let i = ctl.done.load(Ordering::Relaxed);
    if i >= ctl.count || ctl.cancelled() || record.function.is_none() {
        ctl.running.store(false, Ordering::Relaxed);
        crate::wake();
        return false;
    }
    if let Some(function) = &record.function {
        function(i, ctl);
    }
    ctl.done.store(i + 1, Ordering::Relaxed);
    if i + 1 >= ctl.count || ctl.cancelled() {
        ctl.running.store(false, Ordering::Relaxed);
        crate::wake();
    }
    true
}

/// Process at most `items` background work items before yielding to the host again.
pub fn pump_jobs(items: usize) {
    for _ in 0..items {
        let record = {
            let mut q = QUEUE.lock();
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        };
        let Some(record) = record else {
            break;
        };
        let _ = step(&record);
        if record.ctl.running.load(Ordering::Relaxed) {
            QUEUE.lock().push(record);
        }
    }
}

/// Browser compute remains on its app worker; no auxiliary OS threads are started.
pub fn thr_init(_count: u32) -> u32 {
    0
}
/// Number of auxiliary workers.
pub fn thr_count() -> u32 {
    0
}
/// Cancel and complete all queued cooperative jobs.
pub fn thr_shutdown() {
    let queue = core::mem::take(&mut *QUEUE.lock());
    for record in queue {
        record.ctl.cancel();
        let _ = step(&record);
    }
}
/// The app worker always uses scratch slot zero.
pub fn thr_self() -> usize {
    0
}
/// Run the existing parallel-loop contract sequentially, checking cancellation per item.
pub fn parallel_for(count: usize, f: &(dyn Fn(usize, &JobCtl) + Sync)) -> Result<(), Cancelled> {
    let ctl = JobCtl::new(count);
    for i in 0..count {
        if ctl.cancelled() {
            break;
        }
        f(i, &ctl);
        ctl.done.store(i + 1, Ordering::Relaxed);
    }
    if ctl.cancelled() {
        Err(Cancelled)
    } else {
        Ok(())
    }
}

/// A background job whose items run between browser frames.
pub struct Job {
    record: Arc<Record>,
}
impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}
impl Job {
    /// Create an empty, completed job.
    pub fn new() -> Self {
        Self {
            record: Arc::new(Record {
                ctl: JobCtl::new(0),
                function: None,
            }),
        }
    }
    /// Replace a completed job's work; item counts must fit wasm32 address space.
    pub fn set(&mut self, count: u64, f: impl Fn(usize, &JobCtl) + Send + Sync + 'static) -> bool {
        if !self.done() || count > usize::MAX as u64 {
            return false;
        }
        self.record = Arc::new(Record {
            ctl: JobCtl::new(count as usize),
            function: Some(Box::new(f)),
        });
        true
    }
    /// Replace only the item count after completion.
    pub fn set_count(&mut self, count: u64) -> bool {
        if !self.done() || count > usize::MAX as u64 {
            return false;
        }
        // Stale completed queue references are removed before requesting unique ownership.
        QUEUE.lock().retain(|r| !Arc::ptr_eq(r, &self.record));
        let Some(record) = Arc::get_mut(&mut self.record) else {
            return false;
        };
        record.ctl.count = count as usize;
        true
    }
    /// Queue the job; execution begins during the next host step.
    pub fn start(&self) -> bool {
        if !self.done() {
            return false;
        }
        let ctl = &self.record.ctl;
        ctl.done.store(0, Ordering::Relaxed);
        ctl.cancel.store(false, Ordering::Relaxed);
        QUEUE.lock().retain(|r| !Arc::ptr_eq(r, &self.record));
        if ctl.count == 0 || self.record.function.is_none() {
            crate::wake();
            return true;
        }
        ctl.running.store(true, Ordering::Relaxed);
        QUEUE.lock().push(self.record.clone());
        crate::wake();
        true
    }
    /// Finish remaining work synchronously when a caller explicitly waits.
    pub fn wait(&self) -> Result<(), Cancelled> {
        while step(&self.record) {}
        if self.record.ctl.cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
    /// Request cancellation at the next item boundary.
    pub fn cancel(&self) {
        self.record.ctl.cancel();
    }
    /// Whether the job has completed or has not started.
    pub fn done(&self) -> bool {
        !self.record.ctl.running.load(Ordering::Relaxed)
    }
    /// Completed item count.
    pub fn items_done(&self) -> u64 {
        self.record.ctl.items_done()
    }
    /// Total item count.
    pub fn count(&self) -> u64 {
        self.record.ctl.count()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.wait();
    }
}
