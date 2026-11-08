// What only a development build has (the live inspector's editing, hot reload): --cfg jfgui_dev,
// as build_rs.py sets it for its development builds.  A program with std on Linux is plat's glibc
// build (lib/plat/build.rs, which refuses musl), so ui's output goes through the GPU there too
// (--cfg jfgui_glibc: gpu_out.rs, not gpu_out_off.rs).
use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if env::var("PROFILE").as_deref() == Ok("debug") {
        println!("cargo::rustc-cfg=jfgui_dev");
    }
    let with_std = env::var_os("CARGO_FEATURE_STD").is_some();
    if with_std && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        println!("cargo::rustc-cfg=jfgui_glibc");
    }
}
