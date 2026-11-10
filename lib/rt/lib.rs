// rt -- the runtime under everything: allocator, memory intrinsics, exact SSE float
// semantics, the minimal OS calls the runtime itself needs, panic handling.
//
//   rt::os::{pages_alloc, pages_free, write_stdout, write_stderr, exit, yield_now}   (and on
//   Linux the raw syscall helpers plat builds on)
//   rt::kernel the Linux kernel ABI per architecture: SYS_*, O_*, Stat, clone_args (also as rt::os::*)
//   rt::fx   cvtt / cvt / sqrt / minss / maxss / floor_i / ceil_i / div255
//   rt::x87  the extended-precision steps of f64 <-> decimal text (ini.s), fpatan
//   rt::sync SpinLock
//   rt::cpu  CPU feature bits (CPUID, cached; force_baseline for --simd sse2)
//   rt::crash the crash hook, CrashInfo, report (exit 3) -- also for panics
//   rt::boot  what build_rs.py builds into a program (name, manifest, assets) for plat's start
//   rt::main!(body)  the entry symbol of console programs (boot first, then body); with the
//            feature std also their fn main
//   rt::panic_hook()  (feature std) std's panics take the crash report of rt's panic handler
//   rt::abort_on_unwind(f)  a panic that leaves f ends the program (feature unwind; else f())
//   rt::panic_take()  (feature unwind) the last panic of the calling thread, as text
//   rt::Alloc  (feature std, Windows and Linux) rt's allocator for a program with std that wants it
//   rt::VERSION, VERSION_MAJOR / _MINOR / _PATCH   the library's version (the root Cargo.toml, SemVer;
//            docs/VERSIONING.md), also as plat::VERSION and ui::VERSION
//   rt::app_version!()  the app's own version (version= of apps/NAME/PACKAGE)
//
// One of the four crates that may contain `unsafe` (rt, simd, plat, plugin).
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

/// the library's version, "MAJOR.MINOR.PATCH" (the root Cargo.toml; docs/VERSIONING.md says what a
/// change of each part promises).  Not the app's version: that is `rt::app_version!()`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// the first number of VERSION (incompatible API changes; while 0, the MINOR number counts them)
pub const VERSION_MAJOR: u32 = version_part(VERSION, 0);
/// the second number of VERSION
pub const VERSION_MINOR: u32 = version_part(VERSION, 1);
/// the third number of VERSION
pub const VERSION_PATCH: u32 = version_part(VERSION, 2);
const _: () = assert!(version_part(VERSION, 3) == u32::MAX, "the version in Cargo.toml must be MAJOR.MINOR.PATCH");

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
/// program with exit code 2.  A Cargo build hands the manifest over as a file
/// (JFGUI_PERMISSIONS_FILE with --cfg jfgui_manifest), because an environment variable of a build
/// script holds one line only.
///
/// In a program with std (feature `std`) it also defines `fn main`, which hands over to plat's
/// start of a std program (`jfgui_std_start`: crash handlers, the panic hook, then `jfgui_main`,
/// the end through `std::process::exit`); the crate then has neither `#![no_std]` nor `#![no_main]`.
#[cfg(not(feature = "std"))]
#[macro_export]
macro_rules! main {
    ($body:expr) => {
        $crate::__jfgui_main!($body);
    };
}

/// (see above)
#[cfg(feature = "std")]
#[macro_export]
macro_rules! main {
    ($body:expr) => {
        $crate::__jfgui_main!($body);
        $crate::__std_main!();
    };
}

/// rt::main!'s check of the panic strategy: JFLib's runtime needs `panic = "abort"`, a program with
/// std built with the feature `unwind` may unwind as well
#[cfg(not(feature = "unwind"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __rt_panic_check {
    () => {
        #[cfg(panic = "unwind")]
        compile_error!("JFLib needs panic = \"abort\": add [profile.dev] and [profile.release] with panic = \"abort\" to this project's Cargo.toml (a program with std may turn on jflib's feature unwind instead)");
    };
}

/// (see above)
#[cfg(feature = "unwind")]
#[doc(hidden)]
#[macro_export]
macro_rules! __rt_panic_check {
    () => {};
}

/// rt::main!'s `jfgui_main`, without and with std
#[doc(hidden)]
#[macro_export]
macro_rules! __jfgui_main {
    ($body:expr) => {
        $crate::__rt_panic_check!();
        #[allow(unsafe_code, unexpected_cfgs)]
        #[unsafe(no_mangle)]
        fn jfgui_main() -> i32 {
            #[cfg(jfgui_manifest)]
            const JF_MANIFEST: Option<&str> = Some(include_str!(env!("JFGUI_PERMISSIONS_FILE")));
            #[cfg(not(jfgui_manifest))]
            const JF_MANIFEST: Option<&str> = option_env!("JFGUI_PERMISSIONS");
            #[cfg(jfgui_assets)]
            static ASSETS: &[u8] = include_bytes!(env!("JFGUI_ASSETS"));
            #[cfg(not(jfgui_assets))]
            static ASSETS: &[u8] = &[];
            let boot = $crate::boot::Boot { app: option_env!("JFGUI_APP"), manifest: JF_MANIFEST, assets: ASSETS };
            if let Err(code) = $crate::boot::start(&boot) {
                return code;
            }
            $body
        }
    };
}

/// the `fn main` of a program with std: plat's start (jfgui_std_start), which calls jfgui_main.  In
/// a library crate (an app whose program only links it, like the template) it is not the entry
/// point and stays unused: the program's own main calls `plat::std_start()`.
#[doc(hidden)]
#[macro_export]
macro_rules! __std_main {
    () => {
        #[allow(unsafe_code, dead_code)]
        fn main() {
            unsafe extern "Rust" {
                fn jfgui_std_start() -> !;
            }
            // SAFETY: plat defines it in a program with std; it takes no arguments.
            unsafe { jfgui_std_start() }
        }
    };
}

pub mod boot;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_vendor = "apple", target_arch = "aarch64"), path = "portable/cpu.rs")]
pub mod cpu;
pub mod crash;
#[cfg_attr(target_arch = "aarch64", path = "aarch64/fx.rs")] #[cfg_attr(all(not(target_arch = "aarch64"), any(target_arch = "wasm32", target_os = "android")), path = "portable/fx.rs")]
pub mod fx;
#[cfg(target_os = "linux")]
pub mod kernel;
// Android and Apple: the system's malloc and memory functions (android.rs, apple.rs)
#[cfg(not(any(target_os = "android", target_vendor = "apple")))]
#[cfg_attr(target_arch = "wasm32", path = "portable/mem.rs")]
#[cfg_attr(feature = "std", allow(dead_code))] // std brings the allocator and the memory functions
mod mem;
/// rt's allocator, for a program with std that wants it instead of the system's:
/// `#[global_allocator] static ALLOC: jflib::rt::Alloc = jflib::rt::Alloc::new();`
#[cfg(all(feature = "std", not(any(target_os = "android", target_arch = "wasm32", target_vendor = "apple"))))]
pub use mem::Alloc;
#[cfg_attr(target_arch = "wasm32", path = "portable/os.rs")]
#[cfg_attr(target_os = "android", path = "android.rs")]
#[cfg_attr(target_vendor = "apple", path = "apple.rs")]
pub mod os;
pub mod sync;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_vendor = "apple", target_arch = "aarch64"), path = "portable/math.rs")]
pub mod x87;

/// Portable arithmetic, available on native targets for exact backend comparisons.
#[cfg(not(target_arch = "wasm32"))]
pub mod portable {
    pub mod fx;
    pub mod math;
}

/// panic: one line on stderr, the crash hook (CK_PANIC), exit code 3
#[cfg(not(feature = "std"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    panic_report(info)
}

#[cfg(feature = "std")]
mod hosted;
#[cfg(feature = "std")]
pub use hosted::panic_hook;
#[cfg(feature = "unwind")]
pub use hosted::{abort_on_unwind, panic_take};

/// run `f`; with the feature `unwind` (a program with std built with `panic = "unwind"`) a panic
/// that leaves it ends the program as an uncaught panic: JFLib's crash report, exit code 3.  plat
/// runs its threads and the program's start through it.  Without that feature a panic never
/// unwinds, and this is `f()`.
#[cfg(not(feature = "unwind"))]
#[inline(always)]
pub fn abort_on_unwind<R>(f: impl FnOnce() -> R) -> R {
    f()
}

pub(crate) fn panic_report(info: &dyn core::fmt::Display) -> ! {
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
