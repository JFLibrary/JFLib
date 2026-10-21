// mem -- the global allocator and the memory intrinsics the compiler calls
//
// Small blocks (up to 2 KB) come from per-size-class free lists carved out of 64 KB
// chunks; larger ones are whole page runs from the OS (os::pages_alloc), like mem_alloc
// in ASMLIB.  Medium runs (up to 1 MB, rounded to a power of two of pages) are kept in a
// cache when freed (at most CACHE_MAX bytes) and handed out again: worker threads that
// allocate scratch per item then do not pay a VirtualAlloc / mmap, page faults and a free
// each time.  One spin lock makes it safe for threads.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::os;

const CLASSES: usize = 8; // 16, 32, ..., 2048 bytes
const CHUNK: usize = 64 * 1024;
const PAGE: usize = 4096;
const MID: usize = 9; // cached page runs: PAGE << k bytes, k = 0..8 (4 KB .. 1 MB)
const CACHE_MAX: usize = 32 << 20; // bytes of page runs the cache keeps at most

struct Heap {
    free: [*mut u8; CLASSES],
    chunk: *mut u8,
    chunk_left: usize,
    mid: [*mut u8; MID], // cached runs (header word 1 = the next run)
    mid_bytes: usize,
}

pub struct Alloc {
    lock: AtomicBool,
    heap: core::cell::UnsafeCell<Heap>,
}

unsafe impl Sync for Alloc {}

#[global_allocator]
static ALLOC: Alloc = Alloc {
    lock: AtomicBool::new(false),
    heap: core::cell::UnsafeCell::new(Heap {
        free: [core::ptr::null_mut(); CLASSES],
        chunk: core::ptr::null_mut(),
        chunk_left: 0,
        mid: [core::ptr::null_mut(); MID],
        mid_bytes: 0,
    }),
};

/// the cache bucket of a page run of `total` bytes (rounded up to PAGE << k), None = too large
fn mid_of(total: usize) -> Option<usize> {
    let pages = total.div_ceil(PAGE);
    let k = (usize::BITS - (pages - 1).leading_zeros()) as usize; // ceil(log2(pages))
    if k < MID { Some(k) } else { None }
}

fn class_of(size: usize) -> Option<usize> {
    let mut c = 0;
    let mut cap = 16;
    while c < CLASSES {
        if size <= cap {
            return Some(c);
        }
        c += 1;
        cap <<= 1;
    }
    None
}

impl Alloc {
    fn lock(&self) {
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }
    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }
}

impl Alloc {
    /// the block and whether its bytes may be non-zero (reused memory)
    unsafe fn alloc_inner(&self, layout: Layout) -> (*mut u8, bool) {
        let size = layout.size().max(layout.align());
        match class_of(size) {
            Some(c) if layout.align() <= 16 << c => {
                self.lock();
                // SAFETY: the lock gives exclusive access to the heap state.
                let h = unsafe { &mut *self.heap.get() };
                let p = h.free[c];
                let r = if !p.is_null() {
                    // SAFETY: a free block stores the next pointer in its first word.
                    h.free[c] = unsafe { *(p as *mut *mut u8) };
                    p
                } else {
                    let bytes = 16usize << c;
                    if h.chunk_left < bytes {
                        // SAFETY: plain page allocation.
                        h.chunk = unsafe { os::pages_alloc(CHUNK) };
                        h.chunk_left = if h.chunk.is_null() { 0 } else { CHUNK };
                    }
                    if h.chunk_left < bytes {
                        core::ptr::null_mut()
                    } else {
                        let q = h.chunk;
                        // SAFETY: q + bytes stays inside the chunk.
                        h.chunk = unsafe { h.chunk.add(bytes) };
                        h.chunk_left -= bytes;
                        q
                    }
                };
                self.unlock();
                (r, true)
            }
            _ => {
                // page run with a 64-byte header holding the byte count
                let mut total = (size + 64 + PAGE - 1) & !(PAGE - 1);
                if let Some(k) = mid_of(total) {
                    total = PAGE << k;
                    self.lock();
                    // SAFETY: exclusive access under the lock; a cached run keeps its byte
                    // count in header word 0 and the next cached run in word 1.
                    let p = unsafe {
                        let h = &mut *self.heap.get();
                        let p = h.mid[k];
                        if !p.is_null() {
                            h.mid[k] = *(p as *mut *mut u8).add(1);
                            h.mid_bytes -= total;
                        }
                        p
                    };
                    self.unlock();
                    if !p.is_null() {
                        // SAFETY: the run is at least PAGE bytes; its header is word 0.
                        return (unsafe { p.add(64) }, true);
                    }
                }
                // SAFETY: plain page allocation.
                let p = unsafe { os::pages_alloc(total) };
                if p.is_null() {
                    return (p, false);
                }
                // SAFETY: the header lies inside the run.
                unsafe {
                    *(p as *mut usize) = total;
                    (p.add(64), false)
                }
            }
        }
    }
}

unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards to alloc_inner.
        unsafe { self.alloc_inner(layout).0 }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(layout.align());
        match class_of(size) {
            Some(c) if layout.align() <= 16 << c => {
                self.lock();
                // SAFETY: exclusive access under the lock; the block is at least 16 bytes.
                unsafe {
                    let h = &mut *self.heap.get();
                    *(ptr as *mut *mut u8) = h.free[c];
                    h.free[c] = ptr;
                }
                self.unlock();
            }
            _ => unsafe {
                // SAFETY: ptr came from the page path of alloc.
                let base = ptr.sub(64);
                let total = *(base as *mut usize);
                if let Some(k) = mid_of(total).filter(|&k| PAGE << k == total) {
                    self.lock();
                    let h = &mut *self.heap.get();
                    let keep = h.mid_bytes + total <= CACHE_MAX;
                    if keep {
                        // SAFETY: exclusive access under the lock; header word 1 links the run.
                        *(base as *mut *mut u8).add(1) = h.mid[k];
                        h.mid[k] = base;
                        h.mid_bytes += total;
                    }
                    self.unlock();
                    if keep {
                        return;
                    }
                }
                os::pages_free(base, total);
            },
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards to alloc_inner; small blocks and cached page runs are reused
        // memory, so clear them.  Fresh page runs come from the OS (VirtualAlloc / mmap: zero
        // pages) and need no clearing.
        let (p, reused) = unsafe { self.alloc_inner(layout) };
        if !p.is_null() && reused {
            unsafe { memset(p as *mut c_void, 0, layout.size()) };
        }
        p
    }
}

// ---- memory intrinsics (no C runtime): rep movsb / stosb, so the compiler cannot turn
// them back into calls to themselves
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    // SAFETY: the caller guarantees valid, non-overlapping ranges.
    unsafe {
        core::arch::asm!("rep movsb", inout("rcx") n => _, inout("rdi") dst => _,
                         inout("rsi") src => _, options(nostack, preserves_flags));
    }
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as *mut u8, src as *const u8);
    if n == 0 || (d as usize) <= (s as usize) || (d as usize) >= (s as usize) + n {
        // SAFETY: forward copy is correct when dst does not start inside src.
        unsafe { memcpy(dst, src, n) };
    } else {
        // SAFETY: backward copy with the direction flag, cleared again afterwards.
        unsafe {
            core::arch::asm!("std", "rep movsb", "cld",
                             inout("rcx") n => _, inout("rdi") d.add(n - 1) => _,
                             inout("rsi") s.add(n - 1) => _, options(nostack));
        }
    }
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    // SAFETY: the caller guarantees a valid range.
    unsafe {
        core::arch::asm!("rep stosb", inout("rcx") n => _, inout("rdi") dst => _,
                         in("al") c as u8, options(nostack, preserves_flags));
    }
    dst
}

// 8 bytes through an asm load: the optimizer cannot turn the compare loop into a call to
// memcmp / bcmp (itself)
#[inline(always)]
unsafe fn load8(p: *const u8) -> u64 {
    let v: u64;
    // SAFETY: the caller guarantees 8 readable bytes at p (any alignment).
    unsafe {
        core::arch::asm!("mov {v}, qword ptr [{p}]", v = out(reg) v, p = in(reg) p,
                         options(pure, readonly, nostack, preserves_flags));
    }
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a as *const u8, b as *const u8);
    let mut i = 0;
    // 8 bytes at a time (little endian: the lowest differing byte is the first one)
    while i + 8 <= n {
        // SAFETY: i + 8 <= n, so both ranges hold these 8 bytes.
        let (x, y) = unsafe { (load8(a.add(i)), load8(b.add(i))) };
        if x != y {
            let sh = (x ^ y).trailing_zeros() & !7;
            return ((x >> sh) as u8) as i32 - ((y >> sh) as u8) as i32;
        }
        i += 8;
    }
    while i < n {
        // SAFETY: the caller guarantees valid ranges of n bytes; volatile reads keep the
        // optimizer from turning this loop back into a memcmp call.
        let (x, y) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    // SAFETY: same contract as memcmp.
    unsafe { memcmp(a, b, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const core::ffi::c_char) -> usize {
    let s = s as *const u8;
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string; volatile reads keep the loop a loop.
    while unsafe { s.add(n).read_volatile() } != 0 {
        n += 1;
    }
    n
}
