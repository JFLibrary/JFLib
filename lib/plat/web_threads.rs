// OS primitives for the existing pool.rs scheduler. Each Worker instantiates the
// same module and shared memory, with its own __stack_pointer. Only the app worker
// enters the UI. LLD's atomic memory initializer runs once across all instances.
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU32, Ordering};

#[link(wasm_import_module = "jfgui")]
unsafe extern "C" {
    #[link_name = "thread_spawn"]
    fn host_spawn(stack_top: usize, entry: usize, done: usize) -> u32;
    fn thread_wait(address: usize, expected: u32, timeout_ms: i32);
    fn thread_notify(address: usize, count: u32);
    fn thread_count() -> u32;
    fn thread_reap(done: usize);
}

pub(super) fn cpu_count() -> u32 {
    // SAFETY: scalar host query; the host bounds the worker count.
    unsafe { thread_count().clamp(1, crate::THR_MAX as u32) }
}

struct Start {
    f: Option<Box<dyn FnOnce() + Send>>,
    done: AtomicU32,
}

/// A browser worker with an exclusively owned stack and a join completion word.
pub struct Thread {
    start: usize,
    stack: usize,
    bytes: usize,
}
impl Thread {
    /// Wait until JavaScript has returned from WASM before releasing its stack.
    pub fn join(self) {
        // SAFETY: the record and stack are retained until the worker has returned;
        // the worker only moves f, while JavaScript atomically publishes done.
        unsafe {
            // Borrow only the atomic field: the worker may still be moving f.
            let done = &*core::ptr::addr_of!((*(self.start as *const Start)).done);
            while done.load(Ordering::Acquire) == 0 {
                thread_wait(core::ptr::from_ref(done) as usize, 0, 100);
            }
            thread_reap(core::ptr::from_ref(done) as usize);
            drop(Box::from_raw(self.start as *mut Start));
            rt::os::pages_free(self.stack as *mut u8, self.bytes);
        }
    }
}

/// Start a closure on a compute worker; return None if the host cannot start it.
pub fn spawn(stack: usize, f: Box<dyn FnOnce() + Send>) -> Option<Thread> {
    let bytes = stack.max(1024 * 1024).checked_add(65535)? & !65535;
    // SAFETY: the span stays exclusively owned by this worker until join.
    let memory = unsafe { rt::os::pages_alloc(bytes) } as usize;
    if memory == 0 {
        return None;
    }
    let start = Box::into_raw(Box::new(Start {
        f: Some(f),
        done: AtomicU32::new(0),
    }));
    // SAFETY: the host receives live pointers and a 16-byte-aligned stack top;
    // failure means no worker can use them. A success retains both through join.
    unsafe {
        if host_spawn(
            memory + bytes,
            start as usize,
            core::ptr::addr_of!((*start).done) as usize,
        ) == 0
        {
            drop(Box::from_raw(start));
            rt::os::pages_free(memory as *mut u8, bytes);
            return None;
        }
    }
    Some(Thread {
        start: start as usize,
        stack: memory,
        bytes,
    })
}

/// Host entry on a fresh compute worker, after assigning its private stack.
///
/// # Safety
/// The host must pass a live, unused Start pointer allocated by spawn, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jfgui_thread_entry(pointer: usize) {
    // SAFETY: the host contract grants this worker exclusive access to f; join
    // reads only the separate atomic completion word until this function returns.
    let f = unsafe { (*(pointer as *mut Start)).f.take() };
    if let Some(f) = f {
        f();
    }
}

/// Counting semaphore backed by WebAssembly atomics and worker-only Atomics.wait.
pub struct Sem(AtomicU32);
impl Sem {
    /// Construct a semaphore with the specified number of available units.
    pub fn new(initial: u32) -> Option<Self> {
        Some(Self(AtomicU32::new(initial)))
    }
    /// Publish work and wake up to count sleeping workers.
    pub fn post(&self, count: u32) {
        self.0.fetch_add(count, Ordering::Release);
        // SAFETY: the atomic is aligned and stays live for the synchronous call.
        unsafe {
            thread_notify(core::ptr::addr_of!(self.0) as usize, count);
        }
    }
    /// Take a unit; negative timeout waits indefinitely, zero only tries once.
    pub fn wait(&self, timeout: i32) -> bool {
        let deadline = crate::time_us().saturating_add(timeout.max(0) as u64 * 1000);
        loop {
            if self
                .0
                .fetch_update(Ordering::Acquire, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_ok()
            {
                return true;
            }
            let now = crate::time_us();
            if timeout >= 0 && now >= deadline {
                return false;
            }
            let ms = if timeout < 0 {
                100
            } else {
                (deadline - now).div_ceil(1000).min(100) as i32
            };
            // SAFETY: a live, aligned atomic; wait checks the expected value to
            // avoid missing a post between the failed decrement and sleeping.
            unsafe {
                thread_wait(core::ptr::addr_of!(self.0) as usize, 0, ms);
            }
        }
    }
}
