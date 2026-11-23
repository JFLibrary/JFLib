// apple_power_ios -- (power.rs) keep an iPhone or iPad awake: UIApplication's idle timer
//
//   hold(reason, what) -> Some(Held)   INHIBIT_IDLE: idleTimerDisabled while a guard of this
//                                      process holds it (the screen stays on, no auto-lock; iOS
//                                      honours it while the app is in front).  INHIBIT_SUSPEND
//                                      alone -> None: iOS has no sleep of its own besides the
//                                      screen's lock, and an app in the background runs only
//                                      seconds; the reason is not shown anywhere
//   release(held)                      the last guard turns the timer on again
//   ios_idle_timer_off() -> bool       (tests) idleTimerDisabled as UIKit has it now
//
// The property belongs to the main thread: set there at once, from another thread through the
// main queue.  Each setting applies the count of that moment, so settings that arrive out of
// order still end in the right state.  Without an app (no UIApplication) nothing happens and
// hold still answers: the request is noted for an app that starts later in the process.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::INHIBIT_IDLE;
use crate::apple::objc::{Id, class, msg};

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_sync_f(queue: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
}

/// the live INHIBIT_IDLE guards of the process
static IDLE: AtomicU32 = AtomicU32::new(0);

/// what one guard holds
pub struct Held {
    idle: bool,
}

fn app() -> Id {
    // SAFETY: the shared application (nil outside an app; a message to nil answers 0).
    unsafe { msg!(class(c"UIApplication"), c"sharedApplication"; Id) }
}

/// idleTimerDisabled = (a guard is live); on the main thread
extern "C" fn apply(_ctx: *mut c_void) {
    let a = app();
    if !a.is_null() {
        let off = IDLE.load(Ordering::SeqCst) != 0;
        // SAFETY: UIApplication's property, set on the main thread.
        unsafe { msg!(a, c"setIdleTimerDisabled:", off => bool; ()) };
    }
}

fn update() {
    // SAFETY: pthread_main_np reads the thread; apply runs on the main queue, which outlives the
    // process's threads, without a context.
    unsafe {
        if pthread_main_np() != 0 {
            apply(core::ptr::null_mut());
        } else {
            dispatch_async_f(&raw const _dispatch_main_q, core::ptr::null_mut(), apply);
        }
    }
}

pub fn hold(_reason: &str, what: u32) -> Option<Held> {
    if what & INHIBIT_IDLE == 0 {
        return None;
    }
    if IDLE.fetch_add(1, Ordering::SeqCst) == 0 {
        update();
    }
    Some(Held { idle: true })
}

pub fn release(h: &mut Held) {
    if core::mem::take(&mut h.idle) && IDLE.fetch_sub(1, Ordering::SeqCst) == 1 {
        update();
    }
}

/// (tests) UIApplication's idleTimerDisabled now (false outside an app)
pub fn ios_idle_timer_off() -> bool {
    extern "C" fn read(ctx: *mut c_void) {
        let a = app();
        // SAFETY: ctx is the bool below, alive while dispatch_sync_f waits; the property is read on
        // the main thread.
        unsafe { *(ctx as *mut bool) = !a.is_null() && msg!(a, c"isIdleTimerDisabled"; bool) };
    }
    let mut v = false;
    // SAFETY: read writes v; on the main thread itself it runs directly (dispatch_sync_f onto the
    // own queue would wait forever).
    unsafe {
        if pthread_main_np() != 0 {
            read(&raw mut v as *mut c_void);
        } else {
            dispatch_sync_f((&raw const _dispatch_main_q).cast_mut(), &raw mut v as *mut c_void, read);
        }
    }
    v
}
