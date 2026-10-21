// Browser CPU contract: SIMD128 is selected by bundle compilation, never by x86 CPUID.

/// Native SSE4.1 capability bit; never advertised by the scalar browser backend.
pub const CPU_SSE41: u32 = 1;
/// Native AVX2 capability bit; never advertised by the scalar browser backend.
pub const CPU_AVX2: u32 = 2;
/// Native FMA capability bit; never advertised by the scalar browser backend.
pub const CPU_FMA: u32 = 4;
/// Native POPCNT capability bit; never advertised by the scalar browser backend.
pub const CPU_POPCNT: u32 = 8;
/// Native BMI2 capability bit; never advertised by the scalar browser backend.
pub const CPU_BMI2: u32 = 16;

/// CPUID-shaped compatibility record for architecture-independent platform callers.
#[derive(Clone, Copy, Default)]
pub struct CpuidResult {
    /// EAX result; zero when the CPUID instruction is unavailable.
    pub eax: u32,
    /// EBX result.
    pub ebx: u32,
    /// ECX result.
    pub ecx: u32,
    /// EDX result.
    pub edx: u32,
}

/// No x86 CPU leaves exist in WebAssembly.
pub fn cpuid(_leaf: u32) -> CpuidResult {
    CpuidResult::default()
}
/// No native x86 capability bits apply to either scalar or SIMD128 WASM.
pub fn features() -> u32 {
    0
}
/// Browser backend selection happens before loading its scalar/SIMD128 bundle.
pub fn force_baseline() {}
