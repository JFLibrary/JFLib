// ui_only -- the UI thread's own state (the windows' records, the X11 and portal connections,
// AppKit's objects): UiOnly<T> hands it out to the UI thread only.  The first thread that
// touches any such state becomes the UI thread for the rest of the process (in practice the one
// that creates the windows); any other thread panics before it reaches the state, so a safe
// plat call on the wrong thread is refused instead of racing on Cells and RefCells.
// One check per access: an atomic load and the thread's id (Windows reads it from the TEB,
// Apple from pthread_threadid_np, Linux asks the kernel).  Windows, Linux and Apple; the browser,
// Android and iOS keep their app in web.rs's slot, which the host drives.
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering::Relaxed;

/// the UI thread's id + 1 (0: no UI-thread state used yet)
static UI: AtomicU64 = AtomicU64::new(0);

/// state that only the UI thread touches; get() checks the calling thread
pub(crate) struct UiOnly<T>(T);

// SAFETY: get() hands out &T on the UI thread alone, so no two threads ever reach the state:
// T needs neither Sync nor Send.  (A thread id the system gives out again after the UI thread
// ended names a new thread, which reaches the state only after the old one is gone.)
unsafe impl<T> Sync for UiOnly<T> {}

impl<T> UiOnly<T> {
    pub(crate) const fn new(v: T) -> Self {
        UiOnly(v)
    }

    /// the state, on the UI thread; panics on any other thread
    #[inline]
    pub(crate) fn get(&self) -> &T {
        check();
        &self.0
    }
}

/// whether this thread is the UI thread, claiming it when no thread is yet
fn is_ui_thread() -> bool {
    let me = tid().wrapping_add(1);
    let ui = UI.load(Relaxed);
    // one location: every thread sees the same first claim, the losers fail the exchange
    ui == me || (ui == 0 && UI.compare_exchange(0, me, Relaxed, Relaxed).is_ok())
}

#[inline]
fn check() {
    if !is_ui_thread() {
        wrong_thread();
    }
}

#[cold]
#[inline(never)]
fn wrong_thread() -> ! {
    let ui = UI.load(Relaxed).wrapping_sub(1);
    panic!("plat: the UI thread's state used on thread {} (the UI thread is {}: the first thread that used it)", tid(), ui)
}

fn tid() -> u64 {
    crate::plat_tid()
}
