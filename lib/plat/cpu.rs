// cpu -- CPU feature detection for SIMD dispatch                             [src/core/cpu.s]
//
//   cpu_features() -> CPU_* bits     cached; SSE2 is the baseline and has no bit.
//                                    `--simd sse2` on the command line forces 0 (baseline
//                                    kernels only, to compare results)
//   cpu_brand() -> String            the processor name ("" when unknown)
//
// The detection itself lives in rt::cpu (the simd crate dispatches on it); the process entry
// applies `--simd sse2` before the app starts.

use alloc::string::String;

pub use rt::cpu::{CPU_AVX2, CPU_BMI2, CPU_FMA, CPU_POPCNT, CPU_SSE41};
use rt::cpu::cpuid;

/// CPU_* bits (cached; rt::cpu)  [cpu_features]
pub fn cpu_features() -> u32 {
    rt::cpu::features()
}

/// `--simd sse2` on the command line: no optional features (called by the process entry)
pub(crate) fn simd_override() {
    if crate::arg_value("--simd").as_deref() == Some("sse2") {
        rt::cpu::force_baseline();
    }
}

/// the processor name from CPUID 0x80000002..4, blanks trimmed  [cpu_brand]
pub fn cpu_brand() -> String {
    if cpuid(0x8000_0000).eax < 0x8000_0004 {
        return String::new();
    }
    let mut b = [0u8; 48];
    for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
        let r = cpuid(leaf);
        for (j, v) in [r.eax, r.ebx, r.ecx, r.edx].iter().enumerate() {
            b[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    let end = b.iter().position(|&c| c == 0).unwrap_or(48);
    String::from_utf8_lossy(&b[..end]).trim_matches(' ').into()
}

/// run two 256-bit AVX2 instructions (they fault without OS support); only call it when
/// cpu_features() reports CPU_AVX2  [plattest t_cpu]
#[cfg(target_arch = "x86_64")]
pub fn avx2_probe() -> bool {
    if cpu_features() & CPU_AVX2 == 0 {
        return false;
    }
    // SAFETY: AVX2 and the YMM state are enabled (checked above); xmm0 is declared clobbered
    // (the upper half is zeroed again by vzeroupper, which ends the 256-bit code).
    unsafe {
        core::arch::asm!("vpxor ymm0, ymm0, ymm0", "vpaddd ymm0, ymm0, ymm0", "vzeroupper",
                         out("xmm0") _, options(nomem, nostack));
    }
    true
}

/// the stack pointer is 16-byte aligned where a call could happen (the compiler relies on
/// it: a thread started with a misaligned stack shows here)  [plattest th_raw]
#[cfg(target_arch = "x86_64")]
pub fn stack_aligned() -> bool {
    let sp: usize;
    // SAFETY: reads rsp only; without `nostack` the compiler aligns the stack for a call.
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, preserves_flags)) };
    sp & 15 == 0
}

/// The browser does not advertise or execute native AVX2 instructions.
#[cfg(any(target_arch = "wasm32", all(target_os = "android", not(target_arch = "x86_64"))))]
pub fn avx2_probe() -> bool { false }

/// WebAssembly's compiler-managed value/call stacks satisfy their platform ABI.
#[cfg(any(target_arch = "wasm32", all(target_os = "android", not(target_arch = "x86_64"))))]
pub fn stack_aligned() -> bool { true }
