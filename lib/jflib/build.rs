// The style an app gets as jflib::style: the environment variable JFLIB_STYLE, without it winui.
// All styles are always there under their own names, for the run-time style switch.
// The guide for the docs (jflib::guide): docs/GUIDE.md of the package, or of the repository.
use std::path::Path;

const STYLES: [&str; 5] = ["winui", "classic", "kde", "macos", "netlab"];
const REPO: &str = "https://github.com/JFLibrary/JFLib/blob/main/";

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
    guide();
}

/// OUT_DIR/GUIDE.md for jflib::guide: docs/GUIDE.md with its links into the repository made
/// absolute (docs.rs has only this page, not the files next to it).  Watches only the file it
/// found: a missing path in rerun-if-changed would run this script on every build.
fn guide() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let found = ["docs/GUIDE.md", "../../docs/GUIDE.md"].into_iter().map(|p| Path::new(&dir).join(p)).find(|p| p.is_file());
    let text = match &found {
        Some(path) => {
            println!("cargo::rerun-if-changed={}", path.display());
            absolute_links(&std::fs::read_to_string(path).unwrap_or_default())
        }
        None => format!("The guide is `docs/GUIDE.md` of the repository: <{REPO}docs/GUIDE.md>
"),
    };
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_default()).join("GUIDE.md");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(text.as_str()) {
        let _ = std::fs::write(&out, text);
    }
}

/// `](FILE.md)` and `](../apps/...)` -> links into the repository (docs/ is the guide's folder);
/// `](#anchor)` and `](https://...)` stay
fn absolute_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4096);
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        out.push_str(&rest[..at + 2]);
        rest = &rest[at + 2..];
        let relative = !(rest.starts_with('#') || rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("mailto:"));
        if relative && rest.find(')').is_some_and(|end| end > 0 && !rest[..end].contains(char::is_whitespace)) {
            match rest.strip_prefix("../") {
                Some(up) => {
                    out.push_str(REPO);
                    rest = up;
                }
                None => {
                    out.push_str(REPO);
                    out.push_str("docs/");
                }
            }
        }
    }
    out.push_str(rest);
    out
}
