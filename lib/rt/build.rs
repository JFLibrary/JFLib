// `links` in the manifest needs a build script: it makes two versions of rt in one program a clear
// message from Cargo, not E0152 (two panic handlers) from the compiler.  It also picks rt's Linux
// build: against glibc (--cfg jfgui_glibc) unless -C target-feature=+crt-static asks for one static
// file without a C runtime, so `cargo run` needs no flags; the same rule as plat's build.rs.
use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let std = env::var_os("CARGO_FEATURE_STD").is_some();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let crt_static = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default().split(',').any(|f| f == "crt-static");
    if os == "linux" && target_env == "gnu" && (std || !crt_static) {
        println!("cargo::rustc-cfg=jfgui_glibc");
    }
}
