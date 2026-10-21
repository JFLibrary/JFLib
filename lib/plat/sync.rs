// sync -- blocking primitives for worker threads (servers, background jobs)
//
//   Mutex<T> (lock / try_lock -> MutexGuard)     Condvar (wait / wait_timeout / notify_*)
//   RwLock<T> (read / write; writers first)      channel(cap) -> (Sender<T>, Receiver<T>)
//
// One primitive per OS: wait on a 32-bit word while it holds a value, wake one / all waiters
// (Windows WaitOnAddress, Linux private futexes).  Mutex and Condvar are the futex algorithms
// of Rust's std; RwLock and the channel are built on them.  A lock is not poisoned by a panic
// (panics abort the process here).
// Not here: rt::SpinLock (rt keeps spin locks for the allocator and for modules, where plat is
// replaced).

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(windows)]
mod os {
    use core::sync::atomic::AtomicU32;

    #[link(name = "api-ms-win-core-synch-l1-2-0", kind = "raw-dylib")]
    unsafe extern "system" {
        fn WaitOnAddress(addr: *const u32, cmp: *const u32, size: usize, ms: u32) -> i32;
        fn WakeByAddressSingle(addr: *const u32);
        fn WakeByAddressAll(addr: *const u32);
    }

    /// sleep while *a == expected (None: no timeout) -> false on timeout
    pub fn wait(a: &AtomicU32, expected: u32, ms: Option<u32>) -> bool {
        // INFINITE is u32::MAX: a finite wait stays below it
        let t = ms.map_or(u32::MAX, |m| m.min(u32::MAX - 1));
        // SAFETY: both pointers are valid for 4 bytes for the duration of the call.
        unsafe { WaitOnAddress(a.as_ptr(), &expected, 4, t) != 0 }
    }

    pub fn wake_one(a: &AtomicU32) {
        // SAFETY: a valid address; waking nobody is fine.
        unsafe { WakeByAddressSingle(a.as_ptr()) }
    }

    pub fn wake_all(a: &AtomicU32) {
        // SAFETY: as above.
        unsafe { WakeByAddressAll(a.as_ptr()) }
    }
}

#[cfg(target_os = "linux")]
mod os {
    use core::sync::atomic::AtomicU32;

    use rt::os::syscall6;

    const SYS_FUTEX: usize = 202;
    const FUTEX_WAIT_PRIVATE: usize = 128;
    const FUTEX_WAKE_PRIVATE: usize = 129;
    const ETIMEDOUT: isize = 110;

    pub fn wait(a: &AtomicU32, expected: u32, ms: Option<u32>) -> bool {
        let ts = ms.map(|m| [(m / 1000) as i64, (m % 1000) as i64 * 1_000_000]);
        let tp = ts.as_ref().map_or(0, |t| t.as_ptr() as usize);
        // SAFETY: the futex word and the relative timespec outlive the call.
        let r = unsafe { syscall6(SYS_FUTEX, a.as_ptr() as usize, FUTEX_WAIT_PRIVATE, expected as usize, tp, 0, 0) };
        r != -ETIMEDOUT
    }

    pub fn wake_one(a: &AtomicU32) {
        // SAFETY: waking a private futex word we own.
        unsafe { syscall6(SYS_FUTEX, a.as_ptr() as usize, FUTEX_WAKE_PRIVATE, 1, 0, 0, 0) };
    }

    pub fn wake_all(a: &AtomicU32) {
        // SAFETY: as above.
        unsafe { syscall6(SYS_FUTEX, a.as_ptr() as usize, FUTEX_WAKE_PRIVATE, i32::MAX as usize, 0, 0, 0) };
    }
}

pub(crate) use os::{wait, wake_all, wake_one};

// ------------------------------------------------------------------ Mutex

/// a mutual-exclusion lock around a value
pub struct Mutex<T: ?Sized> {
    /// 0 unlocked, 1 locked, 2 locked with waiters
    state: AtomicU32,
    data: UnsafeCell<T>,
}

// SAFETY: the lock hands out the value to one thread at a time.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
// SAFETY: as above; &Mutex only gives access through the lock.
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(v: T) -> Mutex<T> {
        Mutex { state: AtomicU32::new(0), data: UnsafeCell::new(v) }
    }

    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Mutex::new(T::default())
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock(&self) -> MutexGuard<'_, T> {
        if self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
            self.lock_contended();
        }
        MutexGuard { m: self }
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).ok().map(|_| MutexGuard { m: self })
    }

    /// the value without locking (the borrow proves exclusivity)
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }

    fn spin(&self) -> u32 {
        let mut n = 100;
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s != 1 || n == 0 {
                return s;
            }
            n -= 1;
            core::hint::spin_loop();
        }
    }

    fn lock_contended(&self) {
        let mut s = self.spin();
        if s == 0 {
            match self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return,
                Err(e) => s = e,
            }
        }
        loop {
            if s != 2 && self.state.swap(2, Ordering::Acquire) == 0 {
                return;
            }
            wait(&self.state, 2, None);
            s = self.spin();
        }
    }

    fn unlock(&self) {
        if self.state.swap(0, Ordering::Release) == 2 {
            wake_one(&self.state);
        }
    }
}

/// the locked value; unlocks when dropped
pub struct MutexGuard<'a, T: ?Sized> {
    m: &'a Mutex<T>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.m.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock exclusively.
        unsafe { &mut *self.m.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.m.unlock();
    }
}

// ------------------------------------------------------------------ Condvar

/// a condition variable for Mutex
#[derive(Default)]
pub struct Condvar {
    seq: AtomicU32,
}

impl Condvar {
    pub const fn new() -> Condvar {
        Condvar { seq: AtomicU32::new(0) }
    }

    /// unlock, sleep until notified (or spuriously), lock again
    pub fn wait<'a, T: ?Sized>(&self, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.wait_timeout(g, None).0
    }

    /// as wait, at most `ms` milliseconds (None: forever) -> (guard, timed out)
    pub fn wait_timeout<'a, T: ?Sized>(&self, g: MutexGuard<'a, T>, ms: Option<u32>) -> (MutexGuard<'a, T>, bool) {
        let m = g.m;
        let s = self.seq.load(Ordering::Relaxed);
        drop(g);
        let woken = wait(&self.seq, s, ms);
        (m.lock(), !woken)
    }

    /// wait while `cond` holds (re-checked after every wake-up)
    pub fn wait_while<'a, T: ?Sized>(&self, mut g: MutexGuard<'a, T>, mut cond: impl FnMut(&mut T) -> bool) -> MutexGuard<'a, T> {
        while cond(&mut g) {
            g = self.wait(g);
        }
        g
    }

    pub fn notify_one(&self) {
        self.seq.fetch_add(1, Ordering::Relaxed);
        wake_one(&self.seq);
    }

    pub fn notify_all(&self) {
        self.seq.fetch_add(1, Ordering::Relaxed);
        wake_all(&self.seq);
    }
}

// ------------------------------------------------------------------ RwLock

#[derive(Default)]
struct RwState {
    readers: u32,
    writer: bool,
    writers_waiting: u32,
}

/// many readers or one writer; a waiting writer stops new readers
pub struct RwLock<T: ?Sized> {
    st: Mutex<RwState>,
    cv: Condvar,
    data: UnsafeCell<T>,
}

// SAFETY: access to the value follows the reader / writer protocol of the state.
unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
// SAFETY: readers share &T (needs Sync), writers get &mut T (needs Send).
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub const fn new(v: T) -> RwLock<T> {
        RwLock { st: Mutex::new(RwState { readers: 0, writer: false, writers_waiting: 0 }), cv: Condvar::new(), data: UnsafeCell::new(v) }
    }
}

impl<T: ?Sized> RwLock<T> {
    pub fn read(&self) -> RwReadGuard<'_, T> {
        let mut s = self.cv.wait_while(self.st.lock(), |s| s.writer || s.writers_waiting > 0);
        s.readers += 1;
        RwReadGuard { l: self }
    }

    pub fn write(&self) -> RwWriteGuard<'_, T> {
        let mut s = self.st.lock();
        s.writers_waiting += 1;
        let mut s = self.cv.wait_while(s, |s| s.writer || s.readers > 0);
        s.writers_waiting -= 1;
        s.writer = true;
        RwWriteGuard { l: self }
    }
}

pub struct RwReadGuard<'a, T: ?Sized> {
    l: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: a reader holds the lock; no writer is active.
        unsafe { &*self.l.data.get() }
    }
}

impl<T: ?Sized> Drop for RwReadGuard<'_, T> {
    fn drop(&mut self) {
        let mut s = self.l.st.lock();
        s.readers -= 1;
        if s.readers == 0 {
            self.l.cv.notify_all();
        }
    }
}

pub struct RwWriteGuard<'a, T: ?Sized> {
    l: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the writer holds the lock exclusively.
        unsafe { &*self.l.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as above.
        unsafe { &mut *self.l.data.get() }
    }
}

impl<T: ?Sized> Drop for RwWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.l.st.lock().writer = false;
        self.l.cv.notify_all();
    }
}

// ------------------------------------------------------------------ channel

struct ChanState<T> {
    items: VecDeque<T>,
    cap: usize,
    senders: usize,
    receiver: bool,
}

struct Chan<T> {
    st: Mutex<ChanState<T>>,
    not_empty: Condvar,
    not_full: Condvar,
}

/// a bounded multi-producer, single-consumer queue (cap >= 1)
pub fn channel<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    let c = Arc::new(Chan {
        st: Mutex::new(ChanState { items: VecDeque::new(), cap: cap.max(1), senders: 1, receiver: true }),
        not_empty: Condvar::new(),
        not_full: Condvar::new(),
    });
    (Sender { c: c.clone() }, Receiver { c })
}

pub struct Sender<T> {
    c: Arc<Chan<T>>,
}

pub struct Receiver<T> {
    c: Arc<Chan<T>>,
}

/// why recv_timeout returned nothing
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvError {
    Timeout,
    Disconnected,
}

impl<T> Sender<T> {
    /// queue a value, waiting while the queue is full -> Err(v) when the receiver is gone
    pub fn send(&self, v: T) -> Result<(), T> {
        let mut s = self.c.not_full.wait_while(self.c.st.lock(), |s| s.receiver && s.items.len() >= s.cap);
        if !s.receiver {
            return Err(v);
        }
        s.items.push_back(v);
        drop(s);
        self.c.not_empty.notify_one();
        Ok(())
    }

    /// queue a value if there is room -> Err(v) when full or the receiver is gone
    pub fn try_send(&self, v: T) -> Result<(), T> {
        let mut s = self.c.st.lock();
        if !s.receiver || s.items.len() >= s.cap {
            return Err(v);
        }
        s.items.push_back(v);
        drop(s);
        self.c.not_empty.notify_one();
        Ok(())
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.c.st.lock().senders += 1;
        Sender { c: self.c.clone() }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut s = self.c.st.lock();
        s.senders -= 1;
        if s.senders == 0 {
            drop(s);
            self.c.not_empty.notify_all();
        }
    }
}

impl<T> Receiver<T> {
    /// the next value, waiting for one -> None when the queue is empty and every sender is gone
    pub fn recv(&self) -> Option<T> {
        let mut s = self.c.not_empty.wait_while(self.c.st.lock(), |s| s.items.is_empty() && s.senders > 0);
        let v = s.items.pop_front();
        drop(s);
        if v.is_some() {
            self.c.not_full.notify_one();
        }
        v
    }

    /// the next value, waiting at most `ms` milliseconds
    pub fn recv_timeout(&self, ms: u32) -> Result<T, RecvError> {
        let deadline = crate::time_us().saturating_add(ms as u64 * 1000);
        let mut s = self.c.st.lock();
        loop {
            if let Some(v) = s.items.pop_front() {
                drop(s);
                self.c.not_full.notify_one();
                return Ok(v);
            }
            if s.senders == 0 {
                return Err(RecvError::Disconnected);
            }
            let now = crate::time_us();
            if now >= deadline {
                return Err(RecvError::Timeout);
            }
            let left = ((deadline - now).div_ceil(1000)).min(u32::MAX as u64) as u32;
            s = self.c.not_empty.wait_timeout(s, Some(left)).0;
        }
    }

    /// a value if one is queued
    pub fn try_recv(&self) -> Option<T> {
        let v = self.c.st.lock().items.pop_front();
        if v.is_some() {
            self.c.not_full.notify_one();
        }
        v
    }

    /// values queued right now
    pub fn len(&self) -> usize {
        self.c.st.lock().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut s = self.c.st.lock();
        s.receiver = false;
        s.items.clear();
        drop(s);
        self.c.not_full.notify_all();
    }
}
