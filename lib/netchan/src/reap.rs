// reap -- background threads nobody waits for: they end on their own (a flag, a closed socket),
// their handles are joined once they are done, on the next spawn (plat::Thread frees its stack
// only in join)

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use plat::sync::Mutex;

static THREADS: Mutex<Vec<(Arc<AtomicBool>, plat::Thread)>> = Mutex::new(Vec::new());

/// run f on a thread of its own -> false when no thread could be made
pub(crate) fn spawn(stack: usize, f: impl FnOnce() + Send + 'static) -> bool {
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let t = plat::thread_spawn(
        stack,
        Box::new(move || {
            f();
            d.store(true, Ordering::Release);
        }),
    );
    let mut list = THREADS.lock();
    let mut i = 0;
    while i < list.len() {
        if list[i].0.load(Ordering::Acquire) {
            let (_, t) = list.swap_remove(i);
            t.join();
        } else {
            i += 1;
        }
    }
    match t {
        Some(t) => {
            list.push((done, t));
            true
        }
        None => false,
    }
}
