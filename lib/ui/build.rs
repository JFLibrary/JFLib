// What only a development build has (the live inspector's editing, hot reload): --cfg jfgui_dev,
// as build_rs.py sets it for its development builds.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        println!("cargo::rustc-cfg=jfgui_dev");
    }
}
