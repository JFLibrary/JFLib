// linux_thread -- Linux threads (clone + futex; linked against glibc: pthread_create, at the end of this file), semaphores, the wake-up of the UI thread,
// system information, the date                       [src/plat/linux/lx_thread.s, plat_linux.s]
//
// A thread's stack is an mmap with a PROT_NONE guard page below it; the thread record sits
// at its top.  The kernel clears the record's tid word and wakes it (a shared futex wake)
// when the thread ends, which is what join waits for.  Semaphores are process-private
// futex counters.  The wake-up is an eventfd: the display backends poll it next to their
// socket, wait_events polls it alone (headless).

use alloc::boxed::Box;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rt::os::*;

use crate::{Date, E_WAKE, Event};

const FUTEX_WAIT: usize = 0;
const FUTEX_WAKE: usize = 1;
const FUTEX_PRIVATE: usize = 128;
const EINTR: isize = 4;
// CLONE_VM | FS | FILES | SIGHAND | THREAD | SYSVSEM | PARENT_SETTID | CHILD_CLEARTID
const CLONE_THREAD_FLAGS: usize = 0x100 | 0x200 | 0x400 | 0x800 | 0x10000 | 0x40000 | 0x100000 | 0x200000;
#[cfg(target_arch = "x86_64")] const GUARD_SIZE: usize = 4096;
/// each thread's alternate signal stack (the crash handler runs on it: a stack overflow of the
/// thread still gets its report), between two guard pages below the thread's stack
const ALT_SIZE: usize = 65536;
const THREAD_STACK_DEF: usize = 0x10_0000; // 1 MB
const THREAD_STACK_MAX: usize = 0x1000_0000; // 256 MB

// ---------------------------------------------------------------- threads

type Start = Box<dyn FnOnce() + Send + 'static>;

/// the thread record at the top of the stack mapping (64 bytes)
#[repr(C)] #[cfg_attr(jfgui_glibc, allow(dead_code))]
struct ThreadRec {
    tid: AtomicU32, // set by clone, cleared by the kernel when the thread ends
    _pad: u32,
    base: usize,
    size: usize,
    start: *mut Start,
    _rsv: [usize; 4],
}

// clone(flags, stack = rec, parent_tid = &rec.tid, child_tid = &rec.tid, tls = 0; rdx, r10, r8 in
// x86-64's order, rt::os::clone_args): the child starts on its own stack (rsp = rec,
// 16-aligned) with the record in r9 and calls thread_main(rec)
#[cfg(all(target_arch = "x86_64", not(jfgui_glibc)))] core::arch::global_asm!(
    ".globl jfgui_rs_clone",
    "jfgui_rs_clone:",
    "    mov r9, rsi",
    "    mov rdx, rsi",
    "    mov r10, rsi",
    "    xor r8d, r8d",
    "    mov eax, {nr}",
    "    syscall",
    "    test rax, rax",
    "    jz 2f",
    "    ret",
    "2:",
    "    xor ebp, ebp",
    "    mov rdi, r9",
    "    call {main}",
    "    ud2",
    main = sym thread_main,
    nr = const SYS_CLONE,
);

#[cfg(not(jfgui_glibc))] unsafe extern "C" {
    fn jfgui_rs_clone(flags: usize, rec: *mut u8) -> isize;
}

#[cfg(not(jfgui_glibc))] extern "C" fn thread_main(rec: *mut ThreadRec) -> ! {
    // SAFETY: the alternate signal stack thread_spawn reserved in this thread's mapping (the
    // kernel gives a new thread none); rec is the record thread_spawn wrote, its closure is
    // taken over once.
    let f = unsafe {
        let ss = [(*rec).base + GUARD_SIZE, 0, ALT_SIZE];
        syscall3(SYS_SIGALTSTACK, ss.as_ptr() as usize, 0, 0);
        Box::from_raw((*rec).start)
    };
    f();
    // SAFETY: ends this thread only (the kernel clears and wakes rec.tid).
    unsafe { syscall3(SYS_EXIT, 0, 0, 0) };
    loop {}
}

/// a thread started by thread_spawn; join it to wait for its end (and free its stack)
pub struct Thread(*mut ThreadRec);

// SAFETY: the record is only read by join, after the thread ended or through atomics.
unsafe impl Send for Thread {}

/// run f on a new thread with stack bytes of stack (0 = 1 MB)  [plat_thread_create]
pub fn thread_spawn(stack: usize, f: Start) -> Option<Thread> {
    let size = if stack == 0 { THREAD_STACK_DEF } else { stack.min(THREAD_STACK_MAX) }; #[cfg(all(jfgui_glibc, target_arch = "aarch64"))] let size = size.max(PTHREAD_STACK_MIN);
    let size = ((size + 4095) & !4095) + 2 * GUARD_SIZE + ALT_SIZE; #[cfg(jfgui_glibc)] let size = size + TLS_ROOM;
    // SAFETY: a fresh anonymous mapping (PROT_READ | PROT_WRITE; MAP_PRIVATE | MAP_ANONYMOUS |
    // MAP_NORESERVE | MAP_STACK): from the bottom a guard page, the alternate signal stack,
    // a guard page, the stack.
    unsafe {
        let base = syscall6(SYS_MMAP, 0, size, 3, 0x02 | 0x20 | 0x4000 | 0x20000, usize::MAX, 0);
        if base < 0 {
            return None;
        }
        let base = base as usize;
        if syscall3(SYS_MPROTECT, base, GUARD_SIZE, 0) < 0
            || syscall3(SYS_MPROTECT, base + GUARD_SIZE + ALT_SIZE, GUARD_SIZE, 0) < 0 {
            syscall3(SYS_MUNMAP, base, size, 0);
            return None;
        }
        let rec = (base + size - 64) as *mut ThreadRec;
        let start = Box::into_raw(Box::new(f));
        rec.write(ThreadRec { tid: AtomicU32::new(0), _pad: 0, base, size, start, _rsv: [0; 4] });
        if jfgui_rs_clone(CLONE_THREAD_FLAGS, rec as *mut u8) < 0 {
            drop(Box::from_raw(start));
            syscall3(SYS_MUNMAP, base, size, 0);
            return None;
        }
        Some(Thread(rec))
    }
}

#[cfg(not(jfgui_glibc))] impl Thread {
    /// wait until the thread has ended, then free its stack  [plat_thread_join]
    pub fn join(self) {
        // SAFETY: the record stays mapped until the munmap below, which runs after the
        // kernel cleared the tid (the thread no longer uses its stack).
        unsafe {
            let rec = &*self.0;
            loop {
                let t = rec.tid.load(Ordering::SeqCst);
                if t == 0 {
                    break;
                }
                // shared wait: matches the kernel's exit wake
                syscall6(SYS_FUTEX, &rec.tid as *const AtomicU32 as usize, FUTEX_WAIT, t as usize, 0, 0, 0);
            }
            let (base, size) = (rec.base, rec.size);
            syscall3(SYS_MUNMAP, base, size, 0);
        }
    }
}

/// give the rest of the time slice away  [plat_thread_yield]
pub fn thread_yield() {
    rt::os::yield_now()
}

// ---------------------------------------------------------------- semaphores

struct SemInner {
    count: AtomicI32, // the futex word
    waiters: AtomicU32,
}

/// a counting semaphore
pub struct Sem(Box<SemInner>);

impl Sem {
    pub fn new(initial: u32) -> Option<Sem> {
        Some(Sem(Box::new(SemInner { count: AtomicI32::new(initial as i32), waiters: AtomicU32::new(0) })))
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
        let deadline = if ms > 0 { Some(super::time_us() + ms as u64 * 1000) } else { None };
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
                    let now = super::time_us();
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

/// the non-blocking wake eventfd (process start)
pub(crate) fn wake_init() {
    // SAFETY: eventfd2(0, EFD_CLOEXEC | EFD_NONBLOCK).
    let fd = unsafe { syscall3(SYS_EVENTFD2, 0, 0x80000 | 0x800, 0) };
    WAKE_FD.store(if fd < 0 { -1 } else { fd as i32 }, Ordering::SeqCst);
}

pub(crate) fn wake_fd() -> i32 {
    WAKE_FD.load(Ordering::SeqCst)
}

/// UI thread: reset the eventfd, then the flag (in this order: a wake() between the two is
/// answered by the E_WAKE delivered now -- its work was queued before it; the other order
/// could drain the write of a wake() that already set the flag again, and every later wake()
/// would then find the flag set and stay silent); the E_WAKE to deliver
pub(crate) fn wake_consume() -> Event {
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        let mut v = 0u64;
        // SAFETY: non-blocking read of the 8-byte counter (EAGAIN is fine).
        unsafe { syscall3(SYS_READ, fd as usize, &mut v as *mut u64 as usize, 8) };
    }
    WAKE_FLAG.store(0, Ordering::SeqCst);
    Event { typ: E_WAKE, ..Default::default() }
}

/// any thread: make the UI thread's wait return with E_WAKE (coalesced)  [plat_wake]
pub fn wake() {
    if WAKE_FLAG.swap(1, Ordering::SeqCst) != 0 {
        return; // already signalled, not consumed yet
    }
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd < 0 {
        return;
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

/// headless wait (no display): until a wake-up (E_WAKE to on_event, returns 1) or the
/// deadline in time_us() (0 = only poll, u64::MAX = forever; returns 0)  [lx_wait_nodisp]
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
        // one pollfd: fd, POLLIN (a negative fd is ignored: a plain sleep)
        let mut pfd = [(WAKE_FD.load(Ordering::SeqCst) as u32 as i64) | (1i64 << 32)];
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

/// CPUs this process may run on (sched_getaffinity), >= 1  [plat_cpu_count]
pub fn cpu_count() -> u32 {
    let mut mask = [0u64; 16]; // 1024 CPUs
    // SAFETY: the mask buffer holds 128 bytes.
    let r = unsafe { syscall3(SYS_SCHED_GETAFFINITY, 0, 128, mask.as_mut_ptr() as usize) };
    let mut n = 0;
    if r > 0 {
        for m in &mask[..(r as usize).min(128) / 8] {
            n += m.count_ones();
        }
    }
    n.max(1)
}

/// physical memory in bytes (sysinfo), 0 unknown  [plat_mem_total]
pub fn mem_total() -> u64 {
    let mut si = [0u64; 14]; // struct sysinfo (112 bytes)
    // SAFETY: the buffer holds a struct sysinfo.
    if unsafe { syscall3(SYS_SYSINFO, si.as_mut_ptr() as usize, 0, 0) } != 0 {
        return 0;
    }
    let unit = si[13] as u32; // mem_unit at 104
    if unit == 0 { si[4] } else { si[4] * unit as u64 } // totalram at 32
}

/// the date and time in UTC (no time zone database); the flag is true (UTC)  [plat_date]
pub fn date() -> (Date, bool) {
    let mut ts = [0i64; 2];
    // SAFETY: CLOCK_REALTIME into a timespec.
    unsafe { syscall3(SYS_CLOCK_GETTIME, 0, ts.as_mut_ptr() as usize, 0) };
    let secs = ts[0].max(0) as u64;
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
        ms: (ts[1] / 1_000_000) as u16,
        wday: ((days + 4) % 7) as u16,
    };
    (d, true)
}

// ---------------------------------------------------------------- threads of the glibc build
//
// (build_rs.py section "glibc", linux_glibc.rs) Drivers assume every calling thread is a glibc
// thread: thread-local storage, a thread control block.  So pthread_create makes the thread, on
// the mapping thread_spawn made: glibc takes its control block and the static TLS from the top of
// the stack, which TLS_ROOM pays for (about 5 KB with libc alone; libraries opened later take
// dynamic TLS, which glibc allocates elsewhere), so the stack that is left is the size asked for.
// The record stays above it; _rsv[0] holds the pthread_t.  pthread_join is what makes the munmap
// safe; glibc caches no stack of ours.

#[cfg(jfgui_glibc)]
const TLS_ROOM: usize = 16384;

#[cfg(jfgui_glibc)]
extern "C" fn thread_main(rec: *mut u8) -> *mut u8 {
    let rec = rec as *mut ThreadRec;
    // SAFETY: as in the other thread_main: the alternate signal stack thread_spawn reserved, the
    // closure taken over once.
    let f = unsafe {
        let ss = [(*rec).base + GUARD_SIZE, 0, ALT_SIZE];
        syscall3(SYS_SIGALTSTACK, ss.as_ptr() as usize, 0, 0);
        Box::from_raw((*rec).start)
    };
    f();
    core::ptr::null_mut()
}

/// thread_spawn's call: start the thread on rec's mapping -> < 0 when it did not start
#[cfg(jfgui_glibc)]
unsafe fn jfgui_rs_clone(_flags: usize, rec: *mut u8) -> isize {
    use super::glibc::*;
    let r = rec as *mut ThreadRec;
    let mut attr = PthreadAttr([0; PTHREAD_ATTR]);
    // SAFETY: attr is a pthread_attr_t for the time of the call; the stack lies between the upper
    // guard page and the record, thread_spawn wrote the record before this call.
    unsafe {
        let lo = (*r).base + 2 * GUARD_SIZE + ALT_SIZE;
        if pthread_attr_init(&mut attr) != 0 {
            return -1;
        }
        let e = pthread_attr_setstack(&mut attr, lo as *mut u8, rec as usize - lo);
        let e = if e != 0 { e } else { pthread_create(&mut (*r)._rsv[0], &attr, thread_main, rec) };
        pthread_attr_destroy(&mut attr);
        if e != 0 { -1 } else { 0 }
    }
}

#[cfg(jfgui_glibc)]
impl Thread {
    /// wait until the thread has ended, then free its stack  [plat_thread_join]
    pub fn join(self) {
        // SAFETY: the thread was created joinable and only this call joins it; after that nothing
        // uses its mapping any more.
        unsafe {
            let rec = &*self.0;
            super::glibc::pthread_join(rec._rsv[0], core::ptr::null_mut());
            let (base, size) = (rec.base, rec.size);
            syscall3(SYS_MUNMAP, base, size, 0);
        }
    }
}

// ARM64 (docs/ARM64.md): jfgui_rs_clone as above.  clone(flags, stack = rec, parent_tid = &rec.tid,
// tls = 0, child_tid = &rec.tid) in ARM64's order (rt::os::clone_args); the kernel keeps every register
// but x0, so the child finds the record in x1 and starts on its own stack (sp = rec, 16-aligned) with
// frame pointer and link register 0
#[cfg(all(target_arch = "aarch64", not(jfgui_glibc)))]
core::arch::global_asm!(
    ".globl jfgui_rs_clone",
    "jfgui_rs_clone:",
    "    mov x2, x1",
    "    mov x3, xzr",
    "    mov x4, x1",
    "    mov x8, #{nr}",
    "    svc 0",
    "    cbz x0, 2f",
    "    ret",
    "2:",
    "    mov x29, xzr",
    "    mov x30, xzr",
    "    mov x0, x1",
    "    bl {main}",
    "    brk #1",
    main = sym thread_main,
    nr = const SYS_CLONE,
);

// The guard pages on ARM64: 64 KB, the largest page size an ARM64 kernel uses (4, 16 or 64 KB), so both
// guards and the alternate stack between them start on a page boundary whatever the kernel's page size
// is (mprotect needs that); they are reserved, not committed.  glibc on ARM64 refuses stacks below its
// PTHREAD_STACK_MIN (128 KB, for 64 KB pages): thread_spawn asks for at least that much there.
#[cfg(target_arch = "aarch64")]
const GUARD_SIZE: usize = 65536;
#[cfg(all(jfgui_glibc, target_arch = "aarch64"))]
const PTHREAD_STACK_MIN: usize = 131072;
