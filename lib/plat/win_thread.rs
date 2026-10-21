// win_thread -- Windows threads, semaphores, the wake-up of the UI thread, system information
//                                                               [src/plat/win/win_thread.s]
//
// wake(): a flag coalesces bursts.  With a window the wake-up is a posted WM_APP_WAKE (the
// window turns it into E_WAKE, also inside modal loops); without one (headless, before the
// window exists) an auto-reset event that wait_events waits for.  Everything here is
// thread safe.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use crate::{E_WAKE, Event};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(sa: usize, stack: usize, start: extern "system" fn(*mut c_void) -> u32, param: *mut c_void,
                    flags: u32, id: *mut u32) -> isize;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn CloseHandle(h: isize) -> i32;
    fn CreateSemaphoreW(sa: usize, initial: i32, max: i32, name: usize) -> isize;
    fn ReleaseSemaphore(h: isize, n: i32, prev: usize) -> i32;
    fn CreateEventW(sa: usize, manual: i32, initial: i32, name: usize) -> isize;
    fn SetEvent(h: isize) -> i32;
    fn GetProcessAffinityMask(p: isize, proc_mask: *mut usize, sys_mask: *mut usize) -> i32;
    fn GetSystemInfo(si: *mut [u32; 12]);
    fn GlobalMemoryStatusEx(ms: *mut [u64; 8]) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PostMessageW(hwnd: isize, msg: u32, wp: usize, lp: isize) -> i32;
}

const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_TIMEOUT: u32 = 0x102;
pub(crate) const WM_APP_WAKE: u32 = 0x8001;
const THREAD_STACK_DEF: usize = 0x10_0000; // 1 MB
const THREAD_STACK_MAX: usize = 0x1000_0000; // 256 MB

// ---------------------------------------------------------------- threads

/// a thread started by thread_spawn; join it to wait for its end
pub struct Thread(isize);

// SAFETY: a thread handle may be waited for from any thread.
unsafe impl Send for Thread {}

type Start = Box<dyn FnOnce() + Send + 'static>;

extern "system" fn thread_entry(p: *mut c_void) -> u32 {
    // SAFETY: p is the Box<Start> thread_spawn leaked; this thread takes it over once.
    let f = unsafe { Box::from_raw(p as *mut Start) };
    f();
    0
}

/// run f on a new thread with stack bytes of stack (0 = 1 MB), committed up front  [plat_thread_create]
pub fn thread_spawn(stack: usize, f: Start) -> Option<Thread> {
    let size = if stack == 0 { THREAD_STACK_DEF } else { stack.min(THREAD_STACK_MAX) };
    let size = (size + 0xFFFF) & !0xFFFF; // 64 KB granularity
    let p = Box::into_raw(Box::new(f)) as *mut c_void;
    // SAFETY: thread_entry takes over p; flags 0 = the stack size is committed.
    let h = unsafe { CreateThread(0, size, thread_entry, p, 0, core::ptr::null_mut()) };
    if h == 0 {
        // SAFETY: the thread was not created, p is still ours.
        drop(unsafe { Box::from_raw(p as *mut Start) });
        return None;
    }
    Some(Thread(h))
}

impl Thread {
    /// wait until the thread has ended  [plat_thread_join]
    pub fn join(self) {
        // SAFETY: the handle is owned by this Thread.
        unsafe {
            WaitForSingleObject(self.0, INFINITE);
            CloseHandle(self.0);
        }
    }
}

/// give the rest of the time slice away  [plat_thread_yield]
pub fn thread_yield() {
    rt::os::yield_now()
}

// ---------------------------------------------------------------- semaphores

/// a counting semaphore
pub struct Sem(isize);

// SAFETY: semaphore handles are made for use from several threads.
unsafe impl Send for Sem {}
unsafe impl Sync for Sem {}

impl Sem {
    pub fn new(initial: u32) -> Option<Sem> {
        // SAFETY: no pointers.
        let h = unsafe { CreateSemaphoreW(0, initial as i32, 0x7FFF_FFFF, 0) };
        if h == 0 { None } else { Some(Sem(h)) }
    }

    /// add n units (0: nothing)
    pub fn post(&self, n: u32) {
        if n > 0 {
            // SAFETY: a valid handle.
            unsafe { ReleaseSemaphore(self.0, n as i32, 0) };
        }
    }

    /// take one unit: true taken, false timeout (ms -1 = forever, 0 = only try)
    pub fn wait(&self, ms: i32) -> bool {
        // SAFETY: a valid handle; -1 = INFINITE.
        unsafe { WaitForSingleObject(self.0, ms as u32) == 0 }
    }
}

impl Drop for Sem {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this Sem.
        unsafe { CloseHandle(self.0) };
    }
}

// ---------------------------------------------------------------- UI wake-up

static WAKE_EVENT: AtomicIsize = AtomicIsize::new(0);
static WAKE_FLAG: AtomicU32 = AtomicU32::new(0);
static WAKE_HWND: AtomicIsize = AtomicIsize::new(0);

/// the auto-reset event of headless wake-ups (process start, before any thread)
pub(crate) fn wake_init() {
    // SAFETY: no pointers.
    WAKE_EVENT.store(unsafe { CreateEventW(0, 0, 0, 0) }, Ordering::SeqCst);
}

/// the window that receives WM_APP_WAKE from now on (0: none, back to the event)
pub(crate) fn wake_window(hwnd: isize) {
    WAKE_HWND.store(hwnd, Ordering::SeqCst);
}

pub(crate) fn wake_event() -> isize {
    WAKE_EVENT.load(Ordering::SeqCst)
}

/// UI thread: the wake-up arrived -- clear the flag (the caller delivers E_WAKE)
pub(crate) fn wake_consume() -> Event {
    WAKE_FLAG.store(0, Ordering::SeqCst);
    Event { typ: E_WAKE, ..Default::default() }
}

/// any thread: make the UI thread's wait return with E_WAKE (coalesced)  [plat_wake]
pub fn wake() {
    let old = WAKE_FLAG.swap(1, Ordering::SeqCst);
    let hwnd = WAKE_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        if old != 0 {
            return; // a wake-up is already on its way
        }
        // SAFETY: posting to a window handle is harmless even if it is gone.
        if unsafe { PostMessageW(hwnd, WM_APP_WAKE, 0, 0) } != 0 {
            return;
        }
    }
    let ev = WAKE_EVENT.load(Ordering::SeqCst);
    if ev != 0 {
        // SAFETY: a valid event handle.
        unsafe { SetEvent(ev) };
    }
}

/// headless wait (no window): until a wake-up (E_WAKE to on_event, returns 1) or the
/// deadline in time_us() (0 = only poll, u64::MAX = forever; returns 0)  [plat_wait]
pub fn wait_events(deadline_us: u64, on_event: &mut dyn FnMut(&Event)) -> u32 {
    let ev = WAKE_EVENT.load(Ordering::SeqCst);
    if ev == 0 {
        return 0;
    }
    // SAFETY: a valid event handle.
    if unsafe { WaitForSingleObject(ev, 0) } == 0 {
        on_event(&wake_consume());
        return 1;
    }
    if deadline_us == 0 {
        return 0;
    }
    let ms = if deadline_us == u64::MAX {
        INFINITE
    } else {
        let now = super::time_us();
        if now >= deadline_us {
            return 0;
        }
        ((deadline_us - now + 999) / 1000).min(0xFFFF_FFFE) as u32
    };
    // SAFETY: a valid event handle.
    let r = unsafe { WaitForSingleObject(ev, ms) };
    if r == WAIT_TIMEOUT || r != 0 {
        return 0;
    }
    on_event(&wake_consume());
    1
}

// ---------------------------------------------------------------- system information

/// logical processors this process may use (>= 1)  [plat_cpu_count]
pub fn cpu_count() -> u32 {
    let (mut pm, mut sm) = (0usize, 0usize);
    // SAFETY: -1 = this process; plain out-parameters.
    let mut n = if unsafe { GetProcessAffinityMask(-1, &mut pm, &mut sm) } != 0 { pm.count_ones() } else { 0 };
    if n == 0 {
        let mut si = [0u32; 12];
        // SAFETY: SYSTEM_INFO is 48 bytes.
        unsafe { GetSystemInfo(&mut si) };
        n = si[8]; // dwNumberOfProcessors
    }
    n.max(1)
}

/// physical memory in bytes (0 unknown)  [plat_mem_total]
pub fn mem_total() -> u64 {
    let mut ms = [0u64; 8];
    ms[0] = 64; // dwLength
    // SAFETY: MEMORYSTATUSEX is 64 bytes.
    if unsafe { GlobalMemoryStatusEx(&mut ms) } == 0 { 0 } else { ms[1] }
}
