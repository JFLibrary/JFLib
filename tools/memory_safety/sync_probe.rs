// TSan harness for the unchanged SpinLock implementation. The C host joins both
// threads before returning, equivalent to a scoped thread bridge requiring Sync.
#![no_std]
use core::{cell::Cell, ffi::c_void};
#[path = "../../lib/rt/sync.rs"]
mod sync;
mod os {
    unsafe extern "C" { fn sched_yield() -> i32; }
    pub fn yield_now() {
        // SAFETY: no pointer arguments, yields the current test thread.
        unsafe { sched_yield(); }
    }
}
unsafe extern "C" {
    fn host_two_threads(work: extern "C" fn(*const c_void), data: *const c_void);
    fn abort() -> !;
}
fn share<T: Sync>(value: &T, work: extern "C" fn(*const c_void)) {
    // SAFETY: T: Sync; two shared accesses. The host joins both threads before
    // this borrowed value can go out of scope. Neither callback moves or drops it.
    unsafe { host_two_threads(work, value as *const T as *const c_void) }
}
extern "C" fn cell_work(p: *const c_void) {
    // SAFETY: share passes its live guard with this exact type.
    let g = unsafe { &*(p as *const sync::Guard<'_, Cell<u32>>) };
    for _ in 0..10000 { g.set(g.get().wrapping_add(1)); }
}
extern "C" fn plain_work(p: *const c_void) {
    // SAFETY: share passes its live guard with this exact type.
    let g = unsafe { &*(p as *const sync::Guard<'_, u32>) };
    for _ in 0..10000 { core::hint::black_box(**g); }
}
#[unsafe(no_mangle)]
pub extern "C" fn review_sync(mode: u32) -> u32 {
    if mode == 0 {
        let lock = sync::SpinLock::new(42u32);
        let guard = lock.lock();
        share(&guard, plain_work);
        **&guard
    } else {
        let lock = sync::SpinLock::new(Cell::new(0u32));
        let guard = lock.lock();
        share(&guard, cell_work);
        guard.get()
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: terminate only the isolated test process.
    unsafe { abort() }
}
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
