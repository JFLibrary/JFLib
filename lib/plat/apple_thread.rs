// apple_thread -- threads (pthreads), semaphores (libdispatch), the wake-up of the UI thread, system
// information and the date on macOS and iOS (the same functions as linux_thread.rs)
//
// A dispatch semaphore is created at 0 and posted to its initial count: libdispatch ends the process
// when a semaphore is freed with a count below the one it was created with.  The wake-up is a
// non-blocking pipe: wait_events polls it (headless), a window backend will watch it next to its
// own events (AP4).

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use super::ffi;
use crate::{Date, E_WAKE, Event};

const THREAD_STACK_DEF: usize = 0x10_0000; // 1 MB
const THREAD_STACK_MAX: usize = 0x1000_0000; // 256 MB
const STACK_ALIGN: usize = 0x4000; // pthread stacks: a multiple of the 16 KiB page

// ---------------------------------------------------------------- threads

type Start = Box<dyn FnOnce() + Send + 'static>;

extern "C" fn thread_main(arg: *mut c_void) -> *mut c_void {
    super::crash::thread_alt_stack(); // the crash handler runs there (a stack overflow of this thread too)
    // SAFETY: thread_spawn handed over a Box<Start> through Box::into_raw, taken back once here.
    let f = unsafe { Box::from_raw(arg as *mut Start) };
    rt::abort_on_unwind(f); // feature unwind: a panic that leaves the thread ends the program
    core::ptr::null_mut()
}

/// a thread started by thread_spawn; join it to wait for its end
pub struct Thread(usize); // pthread_t

/// run f on a new thread with stack bytes of stack (0 = 1 MB)  [plat_thread_create]
pub fn thread_spawn(stack: usize, f: Start) -> Option<Thread> {
    let size = if stack == 0 { THREAD_STACK_DEF } else { stack.min(THREAD_STACK_MAX) };
    let size = (size + STACK_ALIGN - 1) & !(STACK_ALIGN - 1);
    let start = Box::into_raw(Box::new(f));
    let mut attr = ffi::PthreadAttr([0; 64]);
    let mut t = 0usize;
    // SAFETY: attr is a pthread_attr_t for the time of the call; start is handed to the thread,
    // or taken back when it did not start.
    unsafe {
        if ffi::pthread_attr_init(&mut attr) != 0 {
            drop(Box::from_raw(start));
            return None;
        }
        let mut e = ffi::pthread_attr_setstacksize(&mut attr, size);
        if e == 0 {
            e = ffi::pthread_create(&mut t, &attr, thread_main, start as *mut c_void);
        }
        ffi::pthread_attr_destroy(&mut attr);
        if e != 0 {
            drop(Box::from_raw(start));
            return None;
        }
    }
    Some(Thread(t))
}

impl Thread {
    /// let the thread end on its own: nobody joins it, its stack goes when it ends
    #[allow(dead_code)]
    pub(crate) fn detach(self) {
        // SAFETY: the thread was created joinable and nobody joins it any more.
        unsafe { ffi::pthread_detach(self.0) };
    }

    /// wait until the thread has ended  [plat_thread_join]
    pub fn join(self) {
        // SAFETY: the thread was created joinable and only this call joins it.
        unsafe { ffi::pthread_join(self.0, core::ptr::null_mut()) };
    }
}

/// give the rest of the time slice away  [plat_thread_yield]
pub fn thread_yield() {
    rt::os::yield_now()
}

/// the calling thread's system-wide id (pthread_threadid_np)
#[allow(dead_code)]
pub(crate) fn plat_tid() -> u64 {
    let mut id = 0u64;
    // SAFETY: 0 = the calling thread; id is writable.
    unsafe { ffi::pthread_threadid_np(0, &mut id) };
    id
}

// ---------------------------------------------------------------- semaphores

/// a counting semaphore
pub struct Sem(*mut c_void); // dispatch_semaphore_t

// SAFETY: dispatch semaphores are made to be signalled and waited on from any thread.
unsafe impl Send for Sem {}
// SAFETY: as above.
unsafe impl Sync for Sem {}

impl Sem {
    pub fn new(initial: u32) -> Option<Sem> {
        // SAFETY: a new semaphore at 0, raised to its initial count (see the top of the file).
        let s = unsafe { ffi::dispatch_semaphore_create(0) };
        if s.is_null() {
            return None;
        }
        for _ in 0..initial {
            // SAFETY: s is ours.
            unsafe { ffi::dispatch_semaphore_signal(s) };
        }
        Some(Sem(s))
    }

    /// add n units, wake sleepers (0: nothing)
    pub fn post(&self, n: u32) {
        for _ in 0..n {
            // SAFETY: the semaphore lives as long as self.
            unsafe { ffi::dispatch_semaphore_signal(self.0) };
        }
    }

    /// take one unit: true taken, false timeout (ms -1 = forever, 0 = only try)
    pub fn wait(&self, ms: i32) -> bool {
        let timeout = match ms {
            ..0 => ffi::DISPATCH_TIME_FOREVER,
            0 => ffi::DISPATCH_TIME_NOW,
            // SAFETY: no pointers; a point in time ms from now.
            _ => unsafe { ffi::dispatch_time(ffi::DISPATCH_TIME_NOW, ms as i64 * 1_000_000) },
        };
        // SAFETY: the semaphore lives as long as self.
        unsafe { ffi::dispatch_semaphore_wait(self.0, timeout) == 0 }
    }
}

impl Drop for Sem {
    fn drop(&mut self) {
        // SAFETY: ours; nobody waits on a semaphore that is being dropped.
        unsafe { ffi::dispatch_release(self.0) };
    }
}

// ---------------------------------------------------------------- UI wake-up

static WAKE_RD: AtomicI32 = AtomicI32::new(-1);
static WAKE_WR: AtomicI32 = AtomicI32::new(-1);
static WAKE_FLAG: AtomicU32 = AtomicU32::new(0);

/// the non-blocking wake pipe (process start)
pub(crate) fn wake_init() {
    let mut fds = [-1i32; 2];
    // SAFETY: fds holds two descriptors; both ends are made non-blocking and close-on-exec.
    unsafe {
        if ffi::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        for fd in fds {
            ffi::fcntl(fd, ffi::F_SETFL, ffi::O_NONBLOCK);
            ffi::fcntl(fd, 2 /* F_SETFD */, 1 /* FD_CLOEXEC */);
        }
    }
    WAKE_RD.store(fds[0], Ordering::SeqCst);
    WAKE_WR.store(fds[1], Ordering::SeqCst);
}

/// UI thread: empty the pipe, then reset the flag (the order of linux_thread.rs); the E_WAKE to
/// deliver
pub(crate) fn wake_consume() -> Event {
    let fd = WAKE_RD.load(Ordering::SeqCst);
    if fd >= 0 {
        let mut buf = [0u8; 64];
        // SAFETY: non-blocking reads into buf until the pipe is empty (EAGAIN).
        while unsafe { ffi::read(fd, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    }
    WAKE_FLAG.store(0, Ordering::SeqCst);
    Event { typ: E_WAKE, ..Default::default() }
}

/// any thread: make the UI thread's wait return with E_WAKE (coalesced)  [plat_wake]
pub fn wake() {
    if WAKE_FLAG.swap(1, Ordering::SeqCst) != 0 {
        return; // already signalled, not consumed yet
    }
    let fd = WAKE_WR.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let one = 1u8;
    // SAFETY: one byte into the non-blocking pipe; a full pipe already wakes the reader.
    while unsafe { ffi::write(fd, (&one as *const u8).cast(), 1) } < 0 && ffi::errno() == ffi::EINTR {}
    // a window backend waits in AppKit's loop, not on the pipe
    #[cfg(target_os = "macos")]
    super::mac::wake_app();
    #[cfg(target_os = "ios")]
    super::disp::wake_app();
}

/// (apple_mac.rs wait) a wake() is waiting to be taken (wake_consume)
#[allow(dead_code)]
pub(crate) fn wake_pending() -> bool {
    WAKE_FLAG.load(Ordering::SeqCst) != 0
}

/// headless wait (no display): until a wake-up (E_WAKE to on_event, returns 1) or the deadline in
/// time_us() (0 = only poll, u64::MAX = forever; returns 0)
pub fn wait_events(deadline_us: u64, on_event: &mut dyn FnMut(&Event)) -> u32 {
    loop {
        let ms: i32 = if deadline_us == u64::MAX {
            -1
        } else if deadline_us == 0 {
            0
        } else {
            let now = super::time_us();
            if now >= deadline_us { 0 } else { ((deadline_us - now + 999) / 1000).min(0x7FFF_FFFF) as i32 }
        };
        let mut pfd = ffi::Pollfd { fd: WAKE_RD.load(Ordering::SeqCst), events: ffi::POLLIN, revents: 0 };
        // SAFETY: one pollfd (a negative fd is ignored: a plain sleep).
        let r = unsafe { ffi::poll(&mut pfd, 1, ms) };
        if r < 0 && ffi::errno() == ffi::EINTR {
            continue;
        }
        if r > 0 && pfd.revents & ffi::POLLIN != 0 {
            on_event(&wake_consume());
            return 1;
        }
        return 0;
    }
}

// ---------------------------------------------------------------- system information

/// CPUs online, >= 1  [plat_cpu_count]
pub fn cpu_count() -> u32 {
    // SAFETY: no pointers.
    let n = unsafe { ffi::sysconf(ffi::SC_NPROCESSORS_ONLN) };
    n.clamp(1, u32::MAX as _) as u32
}

/// physical memory in bytes (hw.memsize), 0 unknown  [plat_mem_total]
pub fn mem_total() -> u64 {
    let mut v = 0u64;
    let mut len = core::mem::size_of::<u64>();
    // SAFETY: a NUL-terminated name; v holds len bytes.
    let r = unsafe { ffi::sysctlbyname(c"hw.memsize".as_ptr(), (&mut v as *mut u64).cast(), &mut len, core::ptr::null_mut(), 0) };
    if r == 0 { v } else { 0 }
}

/// the date and time in UTC (as linux_thread.rs: no time zone database); the flag is true (UTC)
/// [plat_date]
pub fn date() -> (Date, bool) {
    let ns = super::unix_time_ns().max(0) as u64;
    let secs = ns / 1_000_000_000;
    let days = secs / 86400;
    let sod = secs % 86400;
    // civil from days (H. Hinnant)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as u64;
    let d = Date {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        hour: (sod / 3600) as u16,
        min: (sod % 3600 / 60) as u16,
        sec: (sod % 60) as u16,
        ms: (ns % 1_000_000_000 / 1_000_000) as u16,
        wday: ((days + 4) % 7) as u16,
    };
    (d, true)
}
