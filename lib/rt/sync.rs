// sync -- a spin lock for short critical sections (allocator-free, usable in statics)
//
//   SpinLock::new(value)        const
//   lock.lock() -> guard        spins a little, then gives the time slice away
//   lock.try_lock() -> Option   the guard only when the lock is free (crash paths)
//
// For data that several threads touch briefly (the environment overrides, the pool
// queue); long waits belong on a semaphore (plat::Sem).

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock<T> {
    locked: AtomicBool,
    val: UnsafeCell<T>,
}

// SAFETY: the lock hands out one guard at a time, so T moves between threads only.
unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

pub struct Guard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(v: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), val: UnsafeCell::new(v) }
    }

    pub fn lock(&self) -> Guard<'_, T> {
        let mut spins = 0u32;
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins < 64 {
                core::hint::spin_loop();
            } else {
                crate::os::yield_now();
            }
        }
        Guard { lock: self }
    }

    /// the guard when the lock is free now, else None (never waits)
    pub fn try_lock(&self) -> Option<Guard<'_, T>> {
        self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok().then_some(Guard { lock: self })
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.lock.val.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.lock.val.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
