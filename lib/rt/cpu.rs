// cpu -- CPU feature detection for SIMD dispatch                             [src/core/cpu.s]
//
//   features() -> CPU_* bits     cached; SSE2 is the baseline and has no bit
//   force_baseline()             report 0 from now on (plat calls it at process start for
//                                `--simd sse2`: baseline kernels only, to compare results)
//   cpuid(leaf) -> CpuidResult
//
// CPU_AVX2 and CPU_FMA are only reported when the OS saves the YMM state (CPUID.1:ECX.OSXSAVE
// and XGETBV(0) bits 1 and 2).  Thread safe: every caller computes the same cached value.
// Lives in rt so that simd (which may not depend on plat) dispatches on the same bits.

use core::arch::x86_64::{CpuidResult, __cpuid_count};
use core::sync::atomic::{AtomicU32, Ordering};

pub const CPU_SSE41: u32 = 1;
pub const CPU_AVX2: u32 = 2; // AVX2 + OS-enabled YMM state
pub const CPU_FMA: u32 = 4; // FMA3 + OS-enabled YMM state
pub const CPU_POPCNT: u32 = 8;
pub const CPU_BMI2: u32 = 16;

const VALID: u32 = 0x8000_0000; // cache marker
static FEATURES: AtomicU32 = AtomicU32::new(0);

#[allow(unused_unsafe)]
pub fn cpuid(leaf: u32) -> CpuidResult {
    // SAFETY: CPUID exists on every x86-64 processor.
    unsafe { __cpuid_count(leaf, 0) }
}

fn xgetbv0() -> u32 {
    let lo: u32;
    // SAFETY: only called when CPUID reports OSXSAVE (XGETBV is enabled).
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") lo, out("edx") _, options(nomem, nostack)) };
    lo
}

fn detect() -> u32 {
    let mut f = 0;
    let max = cpuid(0).eax;
    if max != 0 {
        let c1 = cpuid(1).ecx;
        if c1 & 1 << 19 != 0 {
            f |= CPU_SSE41;
        }
        if c1 & 1 << 23 != 0 {
            f |= CPU_POPCNT;
        }
        let mut ymm = false; // the OS saves XMM + YMM state
        if c1 & 1 << 27 != 0 && c1 & 1 << 28 != 0 && xgetbv0() & 6 == 6 {
            ymm = true;
            if c1 & 1 << 12 != 0 {
                f |= CPU_FMA;
            }
        }
        if max >= 7 {
            let b7 = cpuid(7).ebx;
            if b7 & 1 << 8 != 0 {
                f |= CPU_BMI2;
            }
            if ymm && b7 & 1 << 5 != 0 {
                f |= CPU_AVX2;
            }
        }
    }
    f
}

/// CPU_* bits (cached)  [cpu_features]
#[inline]
pub fn features() -> u32 {
    let v = FEATURES.load(Ordering::Relaxed);
    if v != 0 {
        return v & !VALID;
    }
    let f = detect();
    FEATURES.store(f | VALID, Ordering::Relaxed);
    f
}

/// report no optional features from now on (`--simd sse2`)
pub fn force_baseline() {
    FEATURES.store(VALID, Ordering::Relaxed);
}
