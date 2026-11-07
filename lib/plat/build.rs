// plat's build script: a program with std (feature std) links against glibc on Linux, as every
// Rust program for x86_64-unknown-linux-gnu does, so it gets plat's glibc build (--cfg jfgui_glibc:
// pthreads, the dynamic linker, linux_glibc.rs).  musl has no such build: plat's own threads there
// have no thread-local storage, on which `thread_local!` of a std crate would crash.
use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let std = env::var_os("CARGO_FEATURE_STD").is_some();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if std && os == "linux" {
        if target_env == "gnu" {
            println!("cargo::rustc-cfg=jfgui_glibc");
        } else {
            println!("cargo::error=a JFLib program with std links against glibc on Linux: build it for x86_64-unknown-linux-gnu, not {}",
                     env::var("TARGET").unwrap_or_default());
        }
    }
}
