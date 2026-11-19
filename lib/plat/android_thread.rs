// android_thread -- threads (Bionic's pthreads), semaphores (futex), the UI thread's wake-up (an
// eventfd), sleeps and the system's numbers on Android (docs/ANDROID.md AN1; the same functions as
// linux_thread.rs and apple_thread.rs)
//
// Threads only through pthread_create: Bionic gives every thread its TLS block, its signal stack and
// a guard page, and a thread of a raw clone would crash in the first Bionic function it calls.  The
// rest is the Linux way, with rt::kernel's numbers (decision 2): the semaphores are linux_thread.rs's
// futex counters, the wake-up is its eventfd.  The host's looper on the UI thread watches the eventfd
// (android/bridge.c, ALooper_addFd) and asks the Choreographer for a frame; that frame's tick turns
// the wake-up into E_WAKE (drain_wake).  A program without a UI (android/exe_main.c) waits on it in
// wait_events, as on Linux.  Signals stay Bionic's and debuggerd's: no handlers of our own (a panic
// that leaves a thread ends the program with exit code 3, rt::abort_on_unwind).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rt::os::*;

use crate::{E_WAKE, Event};

const THREAD_STACK_DEF: usize = 0x10_0000; // 1 MB, as on Linux
const THREAD_STACK_MAX: usize = 0x1000_0000; // 256 MB
const STACK_ALIGN: usize = 0x4000; // a multiple of the 16 KiB page (arm64 devices with 16 KiB pages)
const FUTEX_WAIT: usize = 0;
const FUTEX_WAKE: usize = 1;
const FUTEX_PRIVATE: usize = 128;
const EINTR: isize = 4;
const SC_NPROCESSORS_ONLN: i32 = 0x61; // Bionic's _SC_NPROCESSORS_ONLN

#[repr(C, align(8))]
struct PthreadAttr([u64; 8]); // Bionic's pthread_attr_t: 56 bytes on arm64-v8a and x86_64

#[link(name = "c")]
unsafe extern "C" {
    fn pthread_attr_init(attr: *mut PthreadAttr) -> i32;
    fn pthread_attr_setstacksize(attr: *mut PthreadAttr, size: usize) -> i32;
    fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;
    fn pthread_create(out: *mut usize, attr: *const PthreadAttr, start: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> i32;
    fn pthread_join(thread: usize, out: *mut *mut c_void) -> i32;
    fn sysconf(name: i32) -> i64;
    fn setpriority(which: i32, who: u32, prio: i32) -> i32;
}

/// Android's THREAD_PRIORITY_BACKGROUND: the nice value of threads that must not hold up the UI thread
const NICE_BACKGROUND: i32 = 10;

/// (pool.rs, task.rs) the calling thread works in the background: below the UI thread (nice 10, what
/// Android's Process.setThreadPriority(THREAD_PRIORITY_BACKGROUND) sets), so a busy pool does not
/// take the UI thread's turns; it still gets every core nobody else wants
pub(crate) fn thread_background() {
    // SAFETY: PRIO_PROCESS (0) with a thread id changes that thread alone on Linux; no pointers.
    unsafe { setpriority(0, plat_tid() as u32, NICE_BACKGROUND) };
}

// ---------------------------------------------------------------- threads

type Start = Box<dyn FnOnce() + Send + 'static>;

extern "C" fn thread_main(arg: *mut c_void) -> *mut c_void {
    // SAFETY: thread_spawn handed over a Box<Start> through Box::into_raw, taken back once here.
    let f = unsafe { Box::from_raw(arg as *mut Start) };
    rt::abort_on_unwind(f); // feature unwind: a panic that leaves the thread ends the program
    core::ptr::null_mut()
}

/// a thread started by thread_spawn; join it to wait for its end
pub struct Thread(usize); // pthread_t

/// run f on a new thread with stack bytes of stack (0 = 1 MB; rounded up to 16 KiB)  [plat_thread_create]
pub fn thread_spawn(stack: usize, f: Start) -> Option<Thread> {
    let size = if stack == 0 { THREAD_STACK_DEF } else { stack.min(THREAD_STACK_MAX) };
    let size = (size + STACK_ALIGN - 1) & !(STACK_ALIGN - 1);
    let start = Box::into_raw(Box::new(f));
    let mut attr = PthreadAttr([0; 8]);
    let mut t = 0usize;
    // SAFETY: attr is at least as large as Bionic's pthread_attr_t and lives across the calls; start
    // is handed to the thread, or taken back when it did not start.
    unsafe {
        if pthread_attr_init(&mut attr) != 0 {
            drop(Box::from_raw(start));
            return None;
        }
        let mut e = pthread_attr_setstacksize(&mut attr, size);
        if e == 0 {
            e = pthread_create(&mut t, &attr, thread_main, start as *mut c_void);
        }
        pthread_attr_destroy(&mut attr);
        if e != 0 {
            drop(Box::from_raw(start));
            return None;
        }
    }
    Some(Thread(t))
}

impl Thread {
    /// wait until the thread has ended  [plat_thread_join]
    pub fn join(self) {
        // SAFETY: the thread was created joinable and only this call joins it (join takes self).
        unsafe { pthread_join(self.0, core::ptr::null_mut()) };
    }
}

/// give the rest of the time slice away  [plat_thread_yield]
pub fn thread_yield() {
    rt::os::yield_now()
}

/// the calling thread's id (gettid)  [plat_tid]
pub(crate) fn plat_tid() -> u64 {
    // SAFETY: no arguments.
    unsafe { syscall3(SYS_GETTID, 0, 0, 0) as u64 }
}

/// sleep ms milliseconds (ppoll without descriptors, as on Linux); not on the UI thread, which must
/// answer within five seconds (an ANR)
pub fn sleep_ms(ms: u32) {
    // SAFETY: no descriptors: a sleep.
    unsafe { rt::os::poll(0, 0, ms.min(i32::MAX as u32) as i32) };
}

// ---------------------------------------------------------------- semaphores (as linux_thread.rs)

struct SemInner {
    count: AtomicI32, // the futex word
    waiters: AtomicU32,
}

/// a counting semaphore
pub struct Sem(Box<SemInner>);

impl Sem {
    pub fn new(initial: u32) -> Option<Sem> {
        let initial = initial.min(i32::MAX as u32) as i32;
        Some(Sem(Box::new(SemInner { count: AtomicI32::new(initial), waiters: AtomicU32::new(0) })))
    }

    /// add n units, wake sleepers (0: nothing)
    pub fn post(&self, n: u32) {
        if n == 0 {
            return;
        }
        // saturating: an i32 overflow would turn the count negative and block waiters for good
        let add = n.min(i32::MAX as u32) as i32;
        let _ = self.0.count.try_update(Ordering::SeqCst, Ordering::SeqCst, |c| Some(c.saturating_add(add)));
        if self.0.waiters.load(Ordering::SeqCst) != 0 {
            // SAFETY: the futex word lives as long as self.
            unsafe {
                syscall6(SYS_FUTEX, self.0.count.as_ptr() as usize, FUTEX_WAKE | FUTEX_PRIVATE, n as usize, 0, 0, 0)
            };
        }
    }

    /// take one unit: true taken, false timeout (ms -1 = forever, 0 = only try)
    pub fn wait(&self, ms: i32) -> bool {
        let deadline = if ms > 0 { Some(crate::time_us() + ms as u64 * 1000) } else { None };
        loop {
            let mut c = self.0.count.load(Ordering::SeqCst);
            while c > 0 {
                match self.0.count.compare_exchange(c, c - 1, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => return true,
                    Err(v) => c = v,
                }
            }
            if ms == 0 {
                return false;
            }
            let mut ts = [0i64; 2];
            let tsp = match deadline {
                Some(d) => {
                    let now = crate::time_us();
                    if now >= d {
                        return false;
                    }
                    let left = d - now;
                    ts[0] = (left / 1_000_000) as i64;
                    ts[1] = (left % 1_000_000 * 1000) as i64;
                    ts.as_ptr() as usize
                }
                None => 0,
            };
            self.0.waiters.fetch_add(1, Ordering::SeqCst);
            // SAFETY: sleeps only while the count is still 0; ts lives during the call.
            unsafe { syscall6(SYS_FUTEX, self.0.count.as_ptr() as usize, FUTEX_WAIT | FUTEX_PRIVATE, 0, tsp, 0, 0) };
            self.0.waiters.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

// ---------------------------------------------------------------- UI wake-up

static WAKE_FD: AtomicI32 = AtomicI32::new(-1);
static WAKE_FLAG: AtomicU32 = AtomicU32::new(0);

/// the non-blocking wake eventfd, made on first use (any thread), -1 when the kernel gave none
fn wake_fd() -> i32 {
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        return fd;
    }
    // SAFETY: eventfd2(0, EFD_CLOEXEC | EFD_NONBLOCK): a descriptor of our own.
    let new = unsafe { syscall3(SYS_EVENTFD2, 0, 0x80000 | 0x800, 0) };
    if new < 0 {
        return -1;
    }
    match WAKE_FD.compare_exchange(-1, new as i32, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => new as i32,
        Err(first) => {
            // another thread made one first: ours was never handed out (fdsan knows neither)
            // SAFETY: closes the descriptor this call made.
            unsafe { syscall3(SYS_CLOSE, new as usize, 0, 0) };
            first
        }
    }
}

/// (android/bridge.c) the descriptor the UI thread's looper watches (ALooper_addFd): readable after
/// a wake(); -1 without one
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_wake_fd() -> i32 {
    wake_fd()
}

/// UI thread: reset the eventfd, then the flag (linux_thread.rs has why in this order); the E_WAKE
/// to deliver
fn wake_consume() -> Event {
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        let mut v = 0u64;
        // SAFETY: non-blocking read of the 8-byte counter (EAGAIN is fine).
        unsafe { syscall3(SYS_READ, fd as usize, &mut v as *mut u64 as usize, 8) };
    }
    WAKE_FLAG.store(0, Ordering::SeqCst);
    Event { typ: E_WAKE, ..Default::default() }
}

/// (web.rs, the host's tick and input) a wake() came: E_WAKE into the queue; a full queue gets it
/// again with the next frame
pub(super) fn drain_wake() {
    if WAKE_FLAG.load(Ordering::SeqCst) == 0 {
        return;
    }
    if !crate::evq_push(wake_consume()) {
        wake();
    }
}

/// any thread: E_WAKE on the UI thread (coalesced); under the host its looper asks for a frame at
/// once  [plat_wake]
pub fn wake() {
    if WAKE_FLAG.swap(1, Ordering::SeqCst) != 0 {
        return; // already signalled, not consumed yet
    }
    let fd = wake_fd();
    if fd < 0 {
        return; // the host's next tick still takes the flag
    }
    let one = 1u64;
    loop {
        // SAFETY: writes the 8-byte counter increment.
        let r = unsafe { syscall3(SYS_WRITE, fd as usize, &one as *const u64 as usize, 8) };
        if r != -EINTR {
            break;
        }
    }
}

/// headless wait (a program without a UI; scripts): until a wake-up (E_WAKE to on_event, returns 1)
/// or the deadline in time_us() (0 = only poll, u64::MAX = forever; returns 0)
pub fn wait_events(deadline_us: u64, on_event: &mut dyn FnMut(&Event)) -> u32 {
    loop {
        if WAKE_FLAG.load(Ordering::SeqCst) != 0 && wake_fd() < 0 {
            on_event(&wake_consume()); // no eventfd: the flag alone
            return 1;
        }
        let ms: i32 = if deadline_us == u64::MAX {
            -1
        } else if deadline_us == 0 {
            0
        } else {
            let now = crate::time_us();
            if now >= deadline_us { 0 } else { ((deadline_us - now + 999) / 1000).min(0x7FFF_FFFF) as i32 }
        };
        // one pollfd: fd, POLLIN (a negative fd is ignored: a plain sleep)
        let mut pfd = [(wake_fd() as u32 as i64) | (1i64 << 32)];
        // SAFETY: one pollfd.
        let r = unsafe { rt::os::poll(pfd.as_mut_ptr() as usize, 1, ms) };
        if r == -EINTR {
            continue;
        }
        if r > 0 && (pfd[0] >> 48) & 1 != 0 {
            on_event(&wake_consume());
            return 1;
        }
        return 0;
    }
}

// ---------------------------------------------------------------- system information

/// CPUs online (sysconf), >= 1.  big.LITTLE phones count every kind of core  [plat_cpu_count]
pub fn cpu_count() -> u32 {
    // SAFETY: no pointers.
    let n = unsafe { sysconf(SC_NPROCESSORS_ONLN) };
    n.clamp(1, u32::MAX as i64) as u32
}

/// physical memory in bytes (MemTotal of /proc/meminfo), 0 unknown  [plat_mem_total]
pub fn mem_total() -> u64 {
    let Some(text) = proc_file("/proc/meminfo\0") else { return 0 };
    let Some(line) = text.lines().find_map(|l| l.strip_prefix("MemTotal:")) else { return 0 };
    let mut words = line.split_whitespace();
    let n = words.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    match words.next() {
        Some("kB") => n.saturating_mul(1024),
        _ => n,
    }
}

/// a file of /proc (path NUL-terminated), at most 256 KB -> its text
pub(crate) fn proc_file(path: &str) -> Option<String> {
    debug_assert!(path.ends_with('\0'));
    // SAFETY: path is NUL-terminated; read-only, close-on-exec.
    let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY | O_CLOEXEC, 0, 0, 0) };
    if fd < 0 {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    while out.len() < 256 * 1024 {
        // SAFETY: buf is writable for its length.
        let n = unsafe { syscall3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len()) };
        if n == -EINTR {
            continue;
        }
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    // SAFETY: our own descriptor, opened above.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
    String::from_utf8(out).ok()
}
