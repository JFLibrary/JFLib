// Isolated review harness: the production rt allocator, memory intrinsics and SIMD API.
#![no_std]
#![no_main]
extern crate alloc;

use core::alloc::Layout;
use core::fmt::Write;
use core::ffi::c_void;
use simd::{Kernel, Simd};

fn line(args: core::fmt::Arguments) {
    let mut text = rt::crash::StackText::<512>::new();
    let _ = text.write_fmt(args);
    let _ = text.write_str("\n");
    rt::os::write_stdout(text.bytes());
}

fn allocator() -> usize {
    let mut failed = 0;
    let mut checked = 0;
    let sizes = [1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129,
                 255, 256, 257, 511, 512, 513, 1023, 1024, 1025, 2047, 2048,
                 2049, 4096, 65536, 1048576];
    for align in [1, 2, 4, 8, 16, 32, 64, 128, 256, 4096] {
        for size in sizes {
            let layout = Layout::from_size_align(size, align).unwrap();
            // SAFETY: nonzero valid layouts; byte accesses only, even if the allocator
            // violates its alignment contract. Every block is freed with its own layout.
            unsafe {
                let p = alloc::alloc::alloc(layout);
                checked += 1;
                if p.is_null() {
                    failed += 1;
                    line(format_args!("FAIL allocation size={size} align={align}"));
                    continue;
                }
                if p as usize % align != 0 {
                    failed += 1;
                    line(format_args!("FAIL alignment size={size} align={align} remainder={}", p as usize % align));
                }
                core::ptr::write_bytes(p, 0xA5, size);
                let new_size = size + 7;
                let q = alloc::alloc::realloc(p, layout, new_size);
                if q.is_null() {
                    failed += 1;
                    alloc::alloc::dealloc(p, layout);
                } else {
                    if core::slice::from_raw_parts(q, size).iter().any(|&b| b != 0xA5) {
                        failed += 1;
                        line(format_args!("FAIL realloc preservation size={size} align={align}"));
                    }
                    alloc::alloc::dealloc(q, Layout::from_size_align(new_size, align).unwrap());
                }
                let z = alloc::alloc::alloc_zeroed(layout);
                if z.is_null() {
                    failed += 1;
                } else {
                    if core::slice::from_raw_parts(z, size).iter().any(|&b| b != 0) {
                        failed += 1;
                        line(format_args!("FAIL zeroed size={size} align={align}"));
                    }
                    alloc::alloc::dealloc(z, layout);
                }
            }
        }
    }
    line(format_args!("ALLOCATOR layouts={checked} failed={failed}"));
    failed
}

unsafe extern "C" {
    fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
    fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32;
}

fn memory() -> usize {
    let mut failed = 0;
    let mut checked = 0;
    for len in 0..=64 {
        for src in 0..16 {
            for dst in 0..16 {
                let mut actual = [0u8; 96];
                for (i, x) in actual.iter_mut().enumerate() { *x = i as u8; }
                let mut expected = actual;
                expected.copy_within(src..src + len, dst);
                // SAFETY: both ranges are inside actual, including overlapping cases.
                unsafe {
                    let f = core::hint::black_box(memmove as unsafe extern "C" fn(*mut c_void, *const c_void, usize) -> *mut c_void);
                    f(actual.as_mut_ptr().add(dst).cast(), actual.as_ptr().add(src).cast(), len);
                    if memcmp(actual.as_ptr().cast(), expected.as_ptr().cast(), actual.len()) != 0 { failed += 1; }
                }
                checked += 1;
            }
        }
    }
    line(format_args!("MEMORY overlap_cases={checked} failed={failed}"));
    failed
}

struct SimdCheck;
impl Kernel for SimdCheck {
    type Out = usize;
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> usize {
        let mut failed = 0;
        for offset in 0..32 {
            let mut input = [0u8; 128];
            for (i, x) in input.iter_mut().enumerate() { *x = i as u8; }
            let mut out = [0xA5u8; 128];
            let v = s.load(&input[offset..offset + S::BYTES]);
            s.store(&mut out[offset..offset + S::BYTES], v);
            if out[..offset].iter().chain(out[offset + S::BYTES..].iter()).any(|&b| b != 0xA5)
                || out[offset..offset + S::BYTES] != input[offset..offset + S::BYTES] { failed += 1; }
        }
        failed
    }
}

#[unsafe(no_mangle)]
extern "C" fn review_main() -> i32 {
    let alloc_fail = allocator();
    let memory_fail = memory();
    let simd_fail = simd::dispatch_sse2(SimdCheck) + simd::dispatch(SimdCheck);
    line(format_args!("SIMD offsets=64 avx2={} failed={simd_fail}", simd::has_avx2()));
    (alloc_fail + memory_fail + simd_fail != 0) as i32
}

#[cfg(windows)]
#[unsafe(no_mangle)]
extern "C" fn mainCRTStartup() -> ! { rt::os::exit(review_main()) }

#[cfg(target_os = "linux")]
core::arch::global_asm!(
    ".globl _start", "_start:", "and rsp, -16", "call {main}",
    "mov edi, eax", "mov eax, 231", "syscall", "ud2", main = sym review_main,
);
