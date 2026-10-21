// Browser dynamic-library boundary. Native machine code and arbitrary address access are
// explicitly unsupported; no host or shell escape is substituted for the native API.

/// Native guarded-call fault code.
pub const ERR_FAULT: i32 = -64;
/// Readable native segment flag.
pub const LIBA_R: u32 = 1;
/// Writable native segment flag.
pub const LIBA_W: u32 = 2;
/// Executable native segment flag.
pub const LIBA_X: u32 = 4;
/// Native library input failure.
pub const LIBE_IO: u32 = 1;
/// Unsupported native binary format, including all browser DLL/SO attempts.
pub const LIBE_FORMAT: u32 = 2;
/// Unsupported native dependencies.
pub const LIBE_DEPS: u32 = 3;
/// Unsupported relocation.
pub const LIBE_RELOC: u32 = 4;
/// Allocation failure.
pub const LIBE_NOMEM: u32 = 5;
/// Unsupported initializer.
pub const LIBE_INIT: u32 = 6;
/// Unsupported native mapping.
pub const LIBE_MAP: u32 = 7;
/// Native plugin size limit, retained in the common API.
pub const LIB_FILE_MAX: u64 = 64 << 20;
/// Native segment count limit.
pub const LH_SEGMAX: usize = 16;
/// No recorded fault.
pub const GK_NONE: u32 = 0;
/// Access violation category.
pub const GK_ACCESS: u32 = 1;
/// Illegal instruction category.
pub const GK_ILLEGAL: u32 = 2;
/// Arithmetic fault category.
pub const GK_ARITH: u32 = 3;
/// Bus fault category.
pub const GK_BUS: u32 = 4;
/// Breakpoint category.
pub const GK_BREAK: u32 = 5;
/// Stack exhaustion category.
pub const GK_STACK: u32 = 6;
/// Native guard slot count.
pub const GT_MAX: usize = 128;

/// Native segment description; no such segments are loaded in the browser.
#[derive(Clone, Copy, Default, Debug)]
pub struct LibSeg {
    /// Inclusive segment start.
    pub start: usize,
    /// Exclusive segment end.
    pub end: usize,
    /// LIBA flags.
    pub flags: u32,
}
/// Opaque native plugin handle; browser loading always returns an error.
pub struct Lib {
    _private: (),
}
impl Lib {
    /// No native image base exists.
    pub fn base(&self) -> usize {
        0
    }
    /// No native image mapping exists.
    pub fn span(&self) -> usize {
        0
    }
    /// No native segments exist.
    pub fn segs(&self) -> &[LibSeg] {
        &[]
    }
}
/// Reject native DLL/SO input on WebAssembly.
pub fn lib_open(_path: &str) -> Result<Lib, u32> {
    Err(LIBE_FORMAT)
}
/// Native symbols cannot be resolved.
pub fn lib_sym(_lib: &Lib, _name: &str) -> usize {
    0
}
/// Consume an opaque handle.
pub fn lib_close(_lib: Lib) {}
/// No native segment can contain a requested address.
pub fn lib_seg(_lib: &Lib, _address: usize) -> (usize, u32) {
    (0, 0)
}

/// Compatibility fault record for APIs returning an unsupported-operation error.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct GuardInfo {
    /// Fault kind.
    pub kind: u32,
    /// Platform error code.
    pub code: u32,
    /// Faulting code address.
    pub pc: u64,
    /// Faulting data address.
    pub addr: u64,
    /// Caller tag.
    pub tag: u64,
}
/// Browser traps cannot be installed as recoverable native exception handlers.
pub fn guard_init() -> i32 {
    crate::ERR_UNSUPPORTED
}
/// No native guard stack is installed.
pub fn guard_tag() -> u64 {
    0
}
/// Reject arbitrary function-pointer execution through the native plugin boundary.
pub fn guard_call(
    _f: usize,
    _a: u64,
    _b: u64,
    _c: u64,
    _d: u64,
    _info: Option<&mut GuardInfo>,
) -> (u64, u64) {
    (
        crate::ERR_UNSUPPORTED as i64 as u64,
        crate::ERR_UNSUPPORTED as i64 as u64,
    )
}
/// Reject a request for native fault recovery; the supplied closure is not invoked.
pub fn guard_run<R, F: FnOnce() -> R>(_f: F, _info: Option<&mut GuardInfo>) -> Result<R, i32> {
    Err(crate::ERR_UNSUPPORTED)
}
/// Arbitrary address reads are unavailable; use bounded host buffers instead.
pub fn guard_read(_address: usize, _len: usize) -> Option<alloc::vec::Vec<u8>> {
    None
}
/// Arbitrary address writes are unavailable.
pub fn guard_write(_address: usize, _bytes: &[u8]) -> bool {
    false
}
/// Native library memory is unavailable.
pub fn lib_read(_lib: &Lib, _address: usize, _len: usize) -> Option<alloc::vec::Vec<u8>> {
    None
}
/// Native instruction fault tests are unavailable.
pub fn guard_test_code(_kind: u32) -> usize {
    0
}
/// No native guard test state exists.
pub fn guard_test_state() -> (u32, u32, bool) {
    (0, 0, false)
}
