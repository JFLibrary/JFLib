// The style an app gets as jflib::style: the environment variable JFLIB_STYLE, without it winui.
// All styles are always there under their own names, for the run-time style switch.
const STYLES: [&str; 4] = ["winui", "classic", "kde", "macos"];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=JFLIB_STYLE");
    let style = std::env::var("JFLIB_STYLE").unwrap_or_else(|_| STYLES[0].to_string());
    if !STYLES.contains(&style.as_str()) {
        println!("cargo::error=JFLIB_STYLE={style}: the styles are {}", STYLES.join(", "));
    }
    println!("cargo::rustc-cfg=jflib_style=\"{style}\"");
    // the app's build script (jflib-build) reads it as DEP_JFLIB_STD
    if std::env::var_os("CARGO_FEATURE_STD").is_some() {
        println!("cargo::metadata=std=1");
    }
}
