// mem -- rt/mem.rs on ARM64: memcpy, memmove, memset and the 8-byte load of memcmp, in assembly so
// the optimizer cannot turn them back into calls to themselves (rt/mem.rs keeps the heap, memcmp,
// bcmp and strlen for every architecture).  Linux and Windows let user code load and store at any
// alignment, so nothing aligns first:
//   memcpy, memmove forward  32 bytes per step (two ldp / stp of x registers), the rest below 32
//                            bytes by its bits: 16, 8, 4, 2, 1
//   memmove backward         the same steps from the end down
//   memset                   the byte in all eight bytes of a register, the same steps
// Each step loads all its bytes before it stores any, which makes the forward copy right for
// dst < src and the backward one for dst > src when the ranges overlap.

use core::ffi::c_void;

#[cfg_attr(not(feature = "std"), unsafe(no_mangle))]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    // SAFETY: the caller guarantees valid ranges of n bytes that do not overlap (memmove calls it
    // for dst <= src too, which the order of the loads and stores allows).
    unsafe {
        core::arch::asm!(
            "2:",
            "cmp {n}, #32",
            "b.lo 3f",
            "ldp {a}, {b}, [{s}]",
            "ldp {c}, {e}, [{s}, #16]",
            "add {s}, {s}, #32",
            "sub {n}, {n}, #32",
            "stp {a}, {b}, [{d}]",
            "stp {c}, {e}, [{d}, #16]",
            "add {d}, {d}, #32",
            "b 2b",
            "3:",
            "tbz {n}, #4, 4f",
            "ldp {a}, {b}, [{s}], #16",
            "stp {a}, {b}, [{d}], #16",
            "4:",
            "tbz {n}, #3, 5f",
            "ldr {a}, [{s}], #8",
            "str {a}, [{d}], #8",
            "5:",
            "tbz {n}, #2, 6f",
            "ldr {a:w}, [{s}], #4",
            "str {a:w}, [{d}], #4",
            "6:",
            "tbz {n}, #1, 7f",
            "ldrh {a:w}, [{s}], #2",
            "strh {a:w}, [{d}], #2",
            "7:",
            "tbz {n}, #0, 8f",
            "ldrb {a:w}, [{s}]",
            "strb {a:w}, [{d}]",
            "8:",
            n = inout(reg) n => _, s = inout(reg) src => _, d = inout(reg) dst => _,
            a = out(reg) _, b = out(reg) _, c = out(reg) _, e = out(reg) _,
            options(nostack),
        );
    }
    dst
}

#[cfg_attr(not(feature = "std"), unsafe(no_mangle))]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as usize, src as usize);
    if n == 0 || d <= s || d >= s + n {
        // SAFETY: the forward copy is right when dst does not start inside src.
        unsafe { memcpy(dst, src, n) };
        return dst;
    }
    // SAFETY: the caller guarantees valid ranges of n bytes; from the end down, as memcpy's steps.
    unsafe {
        core::arch::asm!(
            "add {s}, {s}, {n}",
            "add {d}, {d}, {n}",
            "2:",
            "cmp {n}, #32",
            "b.lo 3f",
            "ldp {a}, {b}, [{s}, #-16]",
            "ldp {c}, {e}, [{s}, #-32]!",
            "sub {n}, {n}, #32",
            "stp {a}, {b}, [{d}, #-16]",
            "stp {c}, {e}, [{d}, #-32]!",
            "b 2b",
            "3:",
            "tbz {n}, #4, 4f",
            "ldp {a}, {b}, [{s}, #-16]!",
            "stp {a}, {b}, [{d}, #-16]!",
            "4:",
            "tbz {n}, #3, 5f",
            "ldr {a}, [{s}, #-8]!",
            "str {a}, [{d}, #-8]!",
            "5:",
            "tbz {n}, #2, 6f",
            "ldr {a:w}, [{s}, #-4]!",
            "str {a:w}, [{d}, #-4]!",
            "6:",
            "tbz {n}, #1, 7f",
            "ldrh {a:w}, [{s}, #-2]!",
            "strh {a:w}, [{d}, #-2]!",
            "7:",
            "tbz {n}, #0, 8f",
            "ldrb {a:w}, [{s}, #-1]",
            "strb {a:w}, [{d}, #-1]",
            "8:",
            n = inout(reg) n => _, s = inout(reg) s => _, d = inout(reg) d => _,
            a = out(reg) _, b = out(reg) _, c = out(reg) _, e = out(reg) _,
            options(nostack),
        );
    }
    dst
}

#[cfg_attr(not(feature = "std"), unsafe(no_mangle))]
pub unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let v = c as u8 as u64 * 0x0101_0101_0101_0101;
    // SAFETY: the caller guarantees a valid range of n bytes.
    unsafe {
        core::arch::asm!(
            "2:",
            "cmp {n}, #32",
            "b.lo 3f",
            "stp {v}, {v}, [{d}]",
            "stp {v}, {v}, [{d}, #16]",
            "add {d}, {d}, #32",
            "sub {n}, {n}, #32",
            "b 2b",
            "3:",
            "tbz {n}, #4, 4f",
            "stp {v}, {v}, [{d}], #16",
            "4:",
            "tbz {n}, #3, 5f",
            "str {v}, [{d}], #8",
            "5:",
            "tbz {n}, #2, 6f",
            "str {v:w}, [{d}], #4",
            "6:",
            "tbz {n}, #1, 7f",
            "strh {v:w}, [{d}], #2",
            "7:",
            "tbz {n}, #0, 8f",
            "strb {v:w}, [{d}]",
            "8:",
            n = inout(reg) n => _, d = inout(reg) dst => _, v = in(reg) v,
            options(nostack),
        );
    }
    dst
}

// 8 bytes through an asm load: the optimizer cannot turn memcmp's compare loop into a call to
// memcmp / bcmp (itself)
#[inline(always)]
pub(super) unsafe fn load8(p: *const u8) -> u64 {
    let v: u64;
    // SAFETY: the caller guarantees 8 readable bytes at p (any alignment).
    unsafe {
        core::arch::asm!("ldr {v}, [{p}]", v = out(reg) v, p = in(reg) p,
                         options(pure, readonly, nostack, preserves_flags));
    }
    v
}
