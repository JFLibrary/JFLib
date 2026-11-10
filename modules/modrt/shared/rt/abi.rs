// abi -- the calling convention of plugins and loadable modules (docs/ARM64.md, decision 4)
//
//   rt::plugin_abi!(fn(args) -> ret)                 the type of such a function pointer
//   rt::plugin_abi! { #[attrs] pub fn name(args) -> ret { body } }   such a function
//
// x86-64 keeps the Win64 convention on every system (extern "win64": the plugins and modules
// built so far stay loadable, and a record holds the same pointers on Windows and Linux); every
// other processor takes the system's C convention (extern "C").  On ARM64 that is AAPCS64 on
// Linux, Android and Apple and Microsoft's ARM64 convention on Windows: for the integer and
// pointer arguments and results of these ABIs the two pass the same registers (x0-x7, the
// result in x0 / x1), Windows only keeps x18 for itself, which code from rustc never touches.
// A plugin or module is built per processor and system anyway (docs/ARM64.md, decision 4: an
// x86-64 file is refused on ARM64 with LIBE_ARCH and the other way round).
//
// Used by plat (plib.rs, modhost.rs, modabi.rs), the module runtime modules/modrt (a copy of
// this file in its shared/), the modules and the plugins' hosts (JFPainter).  A function the
// host runs under plat::guard_call must have this convention.

/// The calling convention of plugins and modules: `extern "win64"` on x86-64, `extern "C"`
/// elsewhere (docs/ARM64.md, decision 4).  `plugin_abi!(fn(u32) -> u32)` is the type of such a
/// function pointer; `plugin_abi! { pub fn f(a: u32) -> u32 { a } }` defines such a function
/// (attributes, visibility, `unsafe`, generics and the body pass through unchanged).
#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! plugin_abi {
    (fn ($($a:tt)*) $($r:tt)*) => { extern "win64" fn($($a)*) $($r)* };
    (unsafe fn ($($a:tt)*) $($r:tt)*) => { unsafe extern "win64" fn($($a)*) $($r)* };
    ($(#[$($m:tt)*])* $v:vis unsafe fn $($i:tt)*) => { $(#[$($m)*])* $v unsafe extern "win64" fn $($i)* };
    ($(#[$($m:tt)*])* $v:vis fn $($i:tt)*) => { $(#[$($m)*])* $v extern "win64" fn $($i)* };
}

/// The calling convention of plugins and modules: `extern "win64"` on x86-64, `extern "C"`
/// elsewhere (docs/ARM64.md, decision 4).  `plugin_abi!(fn(u32) -> u32)` is the type of such a
/// function pointer; `plugin_abi! { pub fn f(a: u32) -> u32 { a } }` defines such a function
/// (attributes, visibility, `unsafe`, generics and the body pass through unchanged).
#[cfg(not(target_arch = "x86_64"))]
#[macro_export]
macro_rules! plugin_abi {
    (fn ($($a:tt)*) $($r:tt)*) => { extern "C" fn($($a)*) $($r)* };
    (unsafe fn ($($a:tt)*) $($r:tt)*) => { unsafe extern "C" fn($($a)*) $($r)* };
    ($(#[$($m:tt)*])* $v:vis unsafe fn $($i:tt)*) => { $(#[$($m)*])* $v unsafe extern "C" fn $($i)* };
    ($(#[$($m:tt)*])* $v:vis fn $($i:tt)*) => { $(#[$($m)*])* $v extern "C" fn $($i)* };
}
