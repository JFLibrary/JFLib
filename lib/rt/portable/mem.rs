// Browser allocator and compiler memory entry points. Bytewise volatile loops cannot
// be optimized into recursive calls to memcpy/memset; scalar kernels remain portable.

use crate::sync::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

const CLASSES: usize = 8;
const CHUNK: usize = 65536;

struct Heap {
    free: [usize; CLASSES],
    cursor: usize,
    end: usize,
}
static HEAP: SpinLock<Heap> = SpinLock::new(Heap {
    free: [0; CLASSES],
    cursor: 0,
    end: 0,
});
struct Alloc;
#[global_allocator]
static ALLOC: Alloc = Alloc;

fn class(layout: Layout) -> Option<usize> {
    let needed = layout.size().max(layout.align()).max(16);
    if needed > 2048 {
        None
    } else {
        Some((needed.next_power_of_two().trailing_zeros() - 4) as usize)
    }
}

// SAFETY: all heap metadata is protected by HEAP, and returned blocks do not overlap.
unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(c) = class(layout) {
            let mut heap = HEAP.lock();
            let p = heap.free[c];
            if p != 0 {
                // SAFETY: a freed class block has room for its next pointer.
                heap.free[c] = unsafe { *(p as *const usize) };
                return p as *mut u8;
            }
            let bytes = 16usize << c;
            let mut at = (heap.cursor + bytes - 1) & !(bytes - 1);
            if at.checked_add(bytes).is_none_or(|end| end > heap.end) {
                // SAFETY: this slab is retained for the lifetime of the instance.
                let chunk = unsafe { crate::os::pages_alloc(CHUNK) } as usize;
                if chunk == 0 {
                    return core::ptr::null_mut();
                }
                at = chunk;
                heap.end = chunk + CHUNK;
            }
            heap.cursor = at + bytes;
            return at as *mut u8;
        }
        let align = layout.align().max(core::mem::align_of::<usize>());
        let header = 2 * core::mem::size_of::<usize>();
        let Some(total) = layout
            .size()
            .checked_add(header)
            .and_then(|x| x.checked_add(align - 1))
        else {
            return core::ptr::null_mut();
        };
        // SAFETY: the allocation includes its header and alignment padding.
        let base = unsafe { crate::os::pages_alloc(total) };
        if base.is_null() {
            return base;
        }
        let addr = (base as usize + header + align - 1) & !(align - 1);
        // SAFETY: both header words are aligned and entirely inside the allocated span.
        unsafe {
            ((addr - header) as *mut usize).write(base as usize);
            ((addr - header) as *mut usize).add(1).write(total);
        }
        addr as *mut u8
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if let Some(c) = class(layout) {
            let mut heap = HEAP.lock();
            // SAFETY: the caller releases its block; one word fits every class.
            unsafe {
                p.cast::<usize>().write(heap.free[c]);
            }
            heap.free[c] = p as usize;
        } else {
            // SAFETY: a large allocation stores the original span in the two preceding words.
            unsafe {
                let header = p.cast::<usize>().sub(2);
                crate::os::pages_free(header.read() as *mut u8, header.add(1).read());
            }
        }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: alloc establishes a writable range of layout.size() bytes.
        let p = unsafe { self.alloc(layout) };
        if !p.is_null() {
            // SAFETY: p is live and uniquely owned.
            unsafe {
                clear(p, layout.size());
            }
        }
        p
    }
}

/// Clear a live byte range without calling a compiler memory intrinsic.
pub(crate) unsafe fn clear(p: *mut u8, n: usize) {
    for i in 0..n {
        // SAFETY: the caller supplies n writable bytes.
        unsafe {
            p.add(i).write_volatile(0);
        }
    }
}

/// C ABI nonoverlapping byte copy required by compiler-generated code.
/// # Safety
/// Both ranges must be valid for n bytes and must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    for i in 0..n {
        // SAFETY: guaranteed by the caller; volatile accesses prevent self-recursion.
        unsafe {
            dst.cast::<u8>()
                .add(i)
                .write_volatile(src.cast::<u8>().add(i).read_volatile());
        }
    }
    dst
}
/// C ABI overlapping byte copy.
/// # Safety
/// Both ranges must be valid for n bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if (dst as usize) <= src as usize {
        // SAFETY: forward order is safe for this overlap direction.
        unsafe {
            memcpy(dst, src, n);
        }
    } else {
        for i in (0..n).rev() {
            // SAFETY: backward order protects unread overlapping bytes.
            unsafe {
                dst.cast::<u8>()
                    .add(i)
                    .write_volatile(src.cast::<u8>().add(i).read_volatile());
            }
        }
    }
    dst
}
/// C ABI byte fill.
/// # Safety
/// The destination must be writable for n bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    for i in 0..n {
        // SAFETY: guaranteed by the caller.
        unsafe {
            dst.cast::<u8>().add(i).write_volatile(c as u8);
        }
    }
    dst
}
/// C ABI lexicographic unsigned-byte comparison.
/// # Safety
/// Both ranges must be readable for n bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: guaranteed by the caller.
        let (x, y) = unsafe {
            (
                a.cast::<u8>().add(i).read_volatile(),
                b.cast::<u8>().add(i).read_volatile(),
            )
        };
        if x != y {
            return x as i32 - y as i32;
        }
    }
    0
}
/// C ABI byte equality comparison.
/// # Safety
/// Both ranges must be readable for n bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    // SAFETY: same valid ranges as memcmp.
    unsafe { memcmp(a, b, n) }
}
/// C ABI NUL-terminated string length.
/// # Safety
/// The pointer must reference a readable NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const core::ffi::c_char) -> usize {
    let mut n = 0;
    // SAFETY: the caller guarantees a terminating NUL byte.
    while unsafe { s.add(n).read_volatile() } != 0 {
        n += 1;
    }
    n
}
