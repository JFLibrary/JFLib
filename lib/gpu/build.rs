// gpu's build script: a Linux program against glibc (plat's build.rs: the default, and always with
// std) reaches Vulkan through the system's dynamic linker (--cfg jfgui_glibc); one static file
// (-C target-feature=+crt-static) has no GPU.
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
