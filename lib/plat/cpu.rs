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
#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))] use rt::cpu::cpuid;

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

/// the processor name from CPUID 0x80000002..4 (macOS, iOS: sysctl machdep.cpu.brand_string,
/// "Apple M2"), blanks trimmed  [cpu_brand]
#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))] pub fn cpu_brand() -> String {
    #[cfg(target_vendor = "apple")]
    {
        let mut b = [0u8; 128];
        let mut len = b.len();
        // SAFETY: a NUL-terminated name; b holds len bytes.
        let r = unsafe { crate::apple::ffi::sysctlbyname(c"machdep.cpu.brand_string".as_ptr(), b.as_mut_ptr().cast(), &mut len, core::ptr::null_mut(), 0) };
        if r != 0 {
            return String::new();
        }
        let end = b[..len.min(b.len())].iter().position(|&c| c == 0).unwrap_or(len.min(b.len()));
        return String::from_utf8_lossy(&b[..end]).trim_matches(' ').into();
    }
    #[allow(unreachable_code)]
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
#[cfg(any(target_arch = "wasm32", all(target_os = "android", not(target_arch = "x86_64")), target_vendor = "apple", all(target_os = "linux", target_arch = "aarch64")))]
pub fn avx2_probe() -> bool { false }

/// WebAssembly's compiler-managed value/call stacks satisfy their platform ABI.
#[cfg(any(target_arch = "wasm32", all(target_os = "android", not(target_arch = "x86_64")), target_vendor = "apple"))]
pub fn stack_aligned() -> bool { true }

// ---- Linux on ARM64 (docs/ARM64.md): no AVX2 (above), the stack pointer read directly, the name from
// MIDR_EL1 (implementer and part), which Linux lets programs read when AT_HWCAP has HWCAP_CPUID (it
// answers for the core the thread runs on: on big.LITTLE chips that may be either kind)

/// the stack pointer is 16-byte aligned (ARM64 faults on a memory access through a misaligned sp: a
/// thread started with one would show here first)  [plattest th_raw]
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub fn stack_aligned() -> bool {
    let sp: usize;
    // SAFETY: reads sp only.
    unsafe { core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags)) };
    sp & 15 == 0
}

/// the processor's name from MIDR_EL1 ("ARM Cortex-A720"; "Qualcomm CPU 0x001" for parts not listed),
/// "ARM64" when the kernel does not let user space read it
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub fn cpu_brand() -> String {
    const HWCAP_CPUID: usize = 1 << 11;
    if rt::os::auxv(rt::os::AT_HWCAP) & HWCAP_CPUID == 0 {
        return String::from("ARM64");
    }
    let midr: u64;
    // SAFETY: with HWCAP_CPUID the kernel emulates reads of the ID registers for user space.
    unsafe { core::arch::asm!("mrs {}, midr_el1", out(reg) midr, options(nomem, nostack, preserves_flags)) };
    let (imp, part) = ((midr >> 24) & 0xFF, (midr >> 4) & 0xFFF);
    let vendor = match imp {
        0x41 => "ARM",
        0x42 => "Broadcom",
        0x43 => "Cavium",
        0x46 => "Fujitsu",
        0x48 => "HiSilicon",
        0x4E => "NVIDIA",
        0x50 => "APM",
        0x51 => "Qualcomm",
        0x61 => "Apple",
        0x6D => "Microsoft",
        0xC0 => "Ampere",
        _ => "",
    };
    let core = match (imp, part) {
        (0x41, 0xD03) => "Cortex-A53",
        (0x41, 0xD04) => "Cortex-A35",
        (0x41, 0xD05) => "Cortex-A55",
        (0x41, 0xD07) => "Cortex-A57",
        (0x41, 0xD08) => "Cortex-A72",
        (0x41, 0xD09) => "Cortex-A73",
        (0x41, 0xD0A) => "Cortex-A75",
        (0x41, 0xD0B) => "Cortex-A76",
        (0x41, 0xD0C) => "Neoverse-N1",
        (0x41, 0xD0D) => "Cortex-A77",
        (0x41, 0xD40) => "Neoverse-V1",
        (0x41, 0xD41) => "Cortex-A78",
        (0x41, 0xD44) => "Cortex-X1",
        (0x41, 0xD46) => "Cortex-A510",
        (0x41, 0xD47) => "Cortex-A710",
        (0x41, 0xD48) => "Cortex-X2",
        (0x41, 0xD49) => "Neoverse-N2",
        (0x41, 0xD4D) => "Cortex-A715",
        (0x41, 0xD4E) => "Cortex-X3",
        (0x41, 0xD4F) => "Neoverse-V2",
        (0x41, 0xD80) => "Cortex-A520",
        (0x41, 0xD81) => "Cortex-A720",
        (0x41, 0xD82) => "Cortex-X4",
        (0x41, 0xD84) => "Neoverse-V3",
        (0x41, 0xD85) => "Cortex-X925",
        (0x41, 0xD87) => "Cortex-A725",
        (0x41, 0xD8E) => "Neoverse-N3",
        _ => "",
    };
    match (vendor, core) {
        ("", _) => alloc::format!("ARM64 CPU 0x{imp:02x}/0x{part:03x}"),
        (v, "") => alloc::format!("{v} CPU 0x{part:03x}"),
        (v, c) => alloc::format!("{v} {c}"),
    }
}
