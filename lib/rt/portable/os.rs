// WebAssembly runtime services. The page allocator is protected by a spin lock,
// including memory.grow; shared-memory builds use real atomic lock operations.
// Freed spans are reused and coalesced across the app and compute workers.

use crate::sync::SpinLock;

const PAGE: usize = 4096;
const WASM_PAGE: usize = 65536;

#[link(wasm_import_module = "jfgui")]
unsafe extern "C" {
    #[link_name = "write"]
    fn host_write(stream: u32, ptr: *const u8, len: usize);
    #[link_name = "exit"]
    fn host_exit(code: i32);
}
unsafe extern "C" {
    static __heap_base: u8;
}

struct Pages {
    cursor: usize,
    free: usize,
}
static PAGES: SpinLock<Pages> = SpinLock::new(Pages { cursor: 0, free: 0 });

#[repr(C)]
struct Free {
    bytes: usize,
    next: usize,
}

/// Worker code is single threaded; the host regains control between UI steps.
pub fn yield_now() {}
/// End this instance through the host, trapping if a faulty host returns.
pub fn exit(code: i32) -> ! {
    // SAFETY: scalar host notification; the fallback guarantees divergence.
    unsafe {
        host_exit(code);
    }
    core::arch::wasm32::unreachable()
}
/// Write UTF-8 or diagnostic bytes to the host's stdout channel.
pub fn write_stdout(b: &[u8]) {
    // SAFETY: the host synchronously copies the valid slice before returning.
    unsafe {
        host_write(1, b.as_ptr(), b.len());
    }
}
/// Write bytes to the separate diagnostic channel.
pub fn write_stderr(b: &[u8]) {
    // SAFETY: the host synchronously copies the valid slice before returning.
    unsafe {
        host_write(2, b.as_ptr(), b.len());
    }
}

/// Allocate zeroed, page-aligned memory, or return null on exhaustion.
///
/// # Safety
/// Release the allocation once with `pages_free`, using the same requested size.
pub unsafe fn pages_alloc(n: usize) -> *mut u8 {
    let Some(bytes) = n.max(1).checked_add(PAGE - 1).map(|v| v & !(PAGE - 1)) else {
        return core::ptr::null_mut();
    };
    let mut pages = PAGES.lock();
    let mut prior = 0;
    let mut at = pages.free;
    while at != 0 {
        // SAFETY: the free list contains aligned spans released under this lock.
        let span = unsafe { &mut *(at as *mut Free) };
        if span.bytes >= bytes {
            let next = if span.bytes > bytes {
                let rest = at + bytes;
                // SAFETY: the remainder is a whole page within this free span.
                unsafe {
                    (rest as *mut Free).write(Free {
                        bytes: span.bytes - bytes,
                        next: span.next,
                    });
                }
                rest
            } else {
                span.next
            };
            if prior == 0 {
                pages.free = next;
            } else {
                // SAFETY: prior is another valid list node while the lock is held.
                unsafe {
                    (*(prior as *mut Free)).next = next;
                }
            }
            drop(pages);
            // SAFETY: this allocation is exclusively owned and has bytes writable bytes.
            unsafe {
                super::mem::clear(at as *mut u8, bytes);
            }
            return at as *mut u8;
        }
        prior = at;
        at = span.next;
    }
    if pages.cursor == 0 {
        pages.cursor = (core::ptr::addr_of!(__heap_base) as usize + PAGE - 1) & !(PAGE - 1);
    }
    let start = pages.cursor;
    let Some(end) = start.checked_add(bytes) else {
        return core::ptr::null_mut();
    };
    let memory_pages = core::arch::wasm32::memory_size::<0>();
    let wanted_pages = end.div_ceil(WASM_PAGE);
    if wanted_pages > memory_pages
        && core::arch::wasm32::memory_grow::<0>(wanted_pages - memory_pages) == usize::MAX
    {
        return core::ptr::null_mut();
    }
    pages.cursor = end;
    start as *mut u8
}

/// Return a page allocation to this instance's ordered, coalescing free list.
///
/// # Safety
/// `p` and `n` must describe a live allocation returned by `pages_alloc`.
pub unsafe fn pages_free(p: *mut u8, n: usize) {
    let bytes = (n.max(1) + PAGE - 1) & !(PAGE - 1);
    let addr = p as usize;
    let mut pages = PAGES.lock();
    let mut prior = 0usize;
    let mut at = pages.free;
    while at != 0 && at < addr {
        prior = at;
        // SAFETY: every list link refers to a released page span.
        at = unsafe { (*(at as *const Free)).next };
    }
    // SAFETY: p is exclusively owned page-aligned memory at least one page long.
    unsafe {
        p.cast::<Free>().write(Free { bytes, next: at });
    }
    let node = if prior != 0 {
        // SAFETY: prior is a valid list node protected by the lock.
        let prev = unsafe { &mut *(prior as *mut Free) };
        if prior + prev.bytes == addr {
            prev.bytes += bytes;
            prev.next = at;
            prior
        } else {
            prev.next = addr;
            addr
        }
    } else {
        pages.free = addr;
        addr
    };
    // SAFETY: node is either the newly freed span or its preceding coalesced span.
    let current = unsafe { &mut *(node as *mut Free) };
    if at != 0 && node + current.bytes == at {
        // SAFETY: the successor is a distinct valid free-list node.
        let next = unsafe { &*(at as *const Free) };
        current.bytes += next.bytes;
        current.next = next.next;
    }
}
