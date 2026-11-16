// plat's build script: which Linux build plat is.  A program for x86_64-unknown-linux-gnu (or the
// ARM64 triple) links against glibc, as every GPU driver is a glibc library (--cfg jfgui_glibc:
// pthreads, the dynamic linker, linux_glibc.rs); -C target-feature=+crt-static in the project's
// rustflags asks for one static file without a C runtime instead (plat's own threads, windows
// through shared memory only).  So `cargo run` needs no flags; rt, gpu and ui follow the same rule
// in their build.rs.  A program with std always links against glibc: musl has no such build,
// plat's own threads there have no thread-local storage, on which `thread_local!` of a std crate
// would crash.
use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let std = env::var_os("CARGO_FEATURE_STD").is_some();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let crt_static = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default().split(',').any(|f| f == "crt-static");
    if os != "linux" {
        return;
    }
    if target_env == "gnu" {
        if std || !crt_static {
            println!("cargo::rustc-cfg=jfgui_glibc");
        }
    } else if std {
        println!("cargo::error=a JFLib program with std links against glibc on Linux: build it for x86_64-unknown-linux-gnu, not {}",
                 env::var("TARGET").unwrap_or_default());
    }
}
