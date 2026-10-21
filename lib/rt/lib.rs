// rt -- the runtime under everything: allocator, memory intrinsics, exact SSE float
// semantics, the minimal OS calls the runtime itself needs, panic handling.
//
//   rt::os::{pages_alloc, pages_free, write_stdout, write_stderr, exit, yield_now}   (and on
//   Linux the raw syscall helpers plat builds on)
//   rt::fx   cvtt / cvt / sqrt / minss / maxss / floor_i / ceil_i / div255
//   rt::x87  the extended-precision steps of f64 <-> decimal text (ini.s), fpatan
//   rt::sync SpinLock
//   rt::cpu  CPU feature bits (CPUID, cached; force_baseline for --simd sse2)
//   rt::crash the crash hook, CrashInfo, report (exit 3) -- also for panics
//   rt::boot  what build_rs.py builds into a program (name, manifest, assets) for plat's start
//   rt::main!(body)  the entry symbol of console programs (boot first, then body)
//   rt::VERSION, VERSION_MAJOR / _MINOR / _PATCH   the library's version (lib/VERSION, SemVer;
//            docs/VERSIONING.md), also as plat::VERSION and ui::VERSION
//   rt::app_version!()  the app's own version (version= of apps/NAME/PACKAGE)
//
// One of the four crates that may contain `unsafe` (rt, simd, plat, plugin).
#![no_std]

extern crate alloc;

/// the library's version, "MAJOR.MINOR.PATCH" (lib/VERSION; docs/VERSIONING.md says what a
/// change of each part promises).  Not the app's version: that is `rt::app_version!()`.
pub const VERSION: &str = include_str!("../VERSION").trim_ascii();
/// the first number of VERSION (incompatible API changes; while 0, the MINOR number counts them)
pub const VERSION_MAJOR: u32 = version_part(VERSION, 0);
/// the second number of VERSION
pub const VERSION_MINOR: u32 = version_part(VERSION, 1);
/// the third number of VERSION
pub const VERSION_PATCH: u32 = version_part(VERSION, 2);
const _: () = assert!(version_part(VERSION, 3) == u32::MAX, "lib/VERSION must be MAJOR.MINOR.PATCH");

/// the n-th number of "1.2.3" (or "1.2.3-pre+build"); u32::MAX when it has no n-th number
const fn version_part(v: &str, n: usize) -> u32 {
    let b = v.as_bytes();
    let (mut i, mut part, mut value, mut digits) = (0, 0, 0u32, 0);
    while i <= b.len() {
        let c = if i < b.len() { b[i] } else { b'.' };
        if c.is_ascii_digit() && digits < 9 {
            value = value * 10 + (c - b'0') as u32;
            digits += 1;
        } else if c == b'.' || c == b'-' || c == b'+' {
            if digits == 0 {
                return u32::MAX;
            }
            if part == n {
                return value;
            }
            if c != b'.' {
                return u32::MAX; // the pre-release / build part has no numbers to count
            }
            (part, value, digits) = (part + 1, 0, 0);
        } else {
            return u32::MAX;
        }
        i += 1;
    }
    u32::MAX
}

/// The app's own version: `version=` of apps/NAME/PACKAGE, which build_rs.py hands to rustc as
/// JFGUI_VERSION (every program of the app); "0.0.0" without one.  An expression of type
/// `&'static str`: `const APP_VERSION: &str = rt::app_version!();`, `update::Config::new(app,
/// rt::app_version!(), ..)`.
#[macro_export]
macro_rules! app_version {
    () => {
        match option_env!("JFGUI_VERSION") {
            Some(v) => v,
            None => "0.0.0",
        }
    };
}

/// Define the common app entry symbol without requiring the GUI crate in headless tools.  It
/// first hands what build_rs.py built in (boot.rs: the app's name, its manifest, its assets)
/// to the platform layer, which installs them as ui::main! does; a refused manifest ends the
/// program with exit code 2.
#[macro_export]
macro_rules! main {
    ($body:expr) => {
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        fn jfgui_main() -> i32 {
            #[cfg(jfgui_assets)]
            static ASSETS: &[u8] = include_bytes!(env!("JFGUI_ASSETS"));
            #[cfg(not(jfgui_assets))]
            static ASSETS: &[u8] = &[];
            let boot = $crate::boot::Boot { app: option_env!("JFGUI_APP"), manifest: option_env!("JFGUI_PERMISSIONS"), assets: ASSETS };
            if let Err(code) = $crate::boot::start(&boot) {
                return code;
            }
            $body
        }
    };
}

pub mod boot;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), path = "portable/cpu.rs")]
pub mod cpu;
pub mod crash;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), path = "portable/fx.rs")]
pub mod fx;
#[cfg(not(target_os = "android"))]
#[cfg_attr(target_arch = "wasm32", path = "portable/mem.rs")]
mod mem;
#[cfg_attr(target_arch = "wasm32", path = "portable/os.rs")]
#[cfg_attr(target_os = "android", path = "android.rs")]
pub mod os;
pub mod sync;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), path = "portable/math.rs")]
pub mod x87;

/// Portable arithmetic, available on native targets for exact backend comparisons.
#[cfg(not(target_arch = "wasm32"))]
pub mod portable {
    pub mod fx;
    pub mod math;
}

/// panic: one line on stderr, the crash hook (CK_PANIC), exit code 3
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    if crash::busy() {
        os::exit(3); // a panic inside the crash hook
    }
    let mut s = crash::StackText::<1024>::new(); // no allocation on the crash path
    let _ = writeln!(s, "panic: {info}");
    os::write_stderr(s.bytes());
    let text = s.as_str().trim_end();
    let mut ci = crash::CrashInfo::empty();
    ci.kind = crash::CK_PANIC;
    ci.msg = text.strip_prefix("panic: ").unwrap_or(text);
    crash::report(&ci)
}
