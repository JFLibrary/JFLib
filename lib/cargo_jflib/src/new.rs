// cargo jflib new [--std] NAME: the template of the crate jflib (lib/jflib/template in JFLib's
// repository; the copy in this package is kept equal there), with the app's name.  --std makes the
// same app with the standard library (jflib's feature std): no #![no_std], a fn main of its own, a
// .cargo/config.toml that only links the C runtime in on Windows (std links with the system's
// linker) and no PERMISSIONS (in a program with std it covers only JFLib's own calls,
// docs/PERMISSIONS.md).  Its Cargo.toml names the feature unwind, for an app that catches panics.

use std::fs;
use std::path::Path;

/// the template's files: path in the new project, text
const FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../template/Cargo.toml.in")),
    (".cargo/config.toml", include_str!("../template/.cargo/config.toml")),
    (".gitignore", include_str!("../template/.gitignore")),
    ("build.rs", include_str!("../template/build.rs")),
    ("PACKAGE", include_str!("../template/PACKAGE")),
    ("PERMISSIONS", include_str!("../template/PERMISSIONS")),
    ("src/main.rs", include_str!("../template/src/main.rs")),
    ("src/lib.rs", include_str!("../template/src/lib.rs")),
    ("src/locales/ui.tsv", include_str!("../template/src/locales/ui.tsv")),
    ("tests/smoke.uis", include_str!("../template/tests/smoke.uis")),
];

/// a template file with the app's name in place of the template's (hallo); Err when the
/// template no longer has the text this replaces
pub(crate) fn personal(path: &str, text: &str, name: &str) -> Result<String, String> {
    let krate = name.replace('-', "_");
    let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    let pairs: Vec<(&str, String)> = match path {
        "Cargo.toml" => vec![
            ("# The template of a JFLib app (README.md of the crate jflib): rename this file to Cargo.toml.",
             format!("# {name}: a JFLib app (README.md of the crate jflib), made by cargo jflib new")),
            ("name = \"hallo\"", format!("name = \"{name}\"")),
        ],
        "src/main.rs" => vec![("extern crate hallo;", format!("extern crate {krate};"))],
        "src/lib.rs" => vec![
            ("// hallo -- a JFLib app with a window", format!("// {name} -- a JFLib app with a window")),
            ("target/debug/hallo --script", format!("target/debug/{name} --script")),
        ],
        "PACKAGE" => vec![("display=Hallo", format!("display={name}")), ("app_id=org.example.Hallo", format!("app_id=org.example.{id}"))],
        "PERMISSIONS" => vec![("# hallo:", format!("# {name}:")), ("cargo clean -p hallo", format!("cargo clean -p {name}"))],
        "tests/smoke.uis" => vec![("# hallo smoke test", format!("# {name} smoke test"))],
        _ => vec![],
    };
    let mut text = text.replace("\r\n", "\n");
    for (old, new) in pairs {
        if !text.contains(old) {
            return Err(format!("the template's {path} has no {old:?} any more (cargo-jflib's new.rs)"));
        }
        text = text.replacen(old, &new, 1);
    }
    Ok(text)
}

/// the program of an app with std (src/main.rs)
const STD_MAIN: &str = r#"// The program.  The app is the library crate (src/lib.rs), so the same code also builds for the
// browser and Android.  With std (jflib's feature std) the program has a fn main of its own, which
// starts the app: plat::std_start sets JFLib up and calls ui::main!'s entry point.
extern crate hallo;

fn main() {
    jflib::plat::std_start()
}
"#;

/// the .cargo/config.toml of an app with std
const STD_CONFIG: &str = r#"# What a build script cannot set (README.md of the crate jflib).  An app with std: on Windows the C
# runtime is linked in, so the program needs no VCRUNTIME140.dll (not every Windows has it).  Linux
# needs nothing here: a program with std is built on Linux and links through cc against glibc.
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]

[target.aarch64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
"#;

/// the end of the profiles' note in the Cargo.toml of an app with std: the feature unwind
const STD_UNWIND: &str = "build JFLib is tested with.  An app that catches panics
# (std::panic::catch_unwind) takes jflib's feature \"unwind\" in place of \"std\" and panic = \"unwind\"
# in both profiles (docs/GUIDE.md of jflib, \"An app with std\").
";

/// the files an app with std leaves out: see the head of this file
const NOT_STD: [&str; 1] = ["PERMISSIONS"];

/// a template file of an app with std (after `personal`); Err when the template no longer has the
/// text this replaces
pub(crate) fn with_std(path: &str, text: &str, name: &str) -> Result<String, String> {
    let krate = name.replace('-', "_");
    let pairs: Vec<(&str, String)> = match path {
        "Cargo.toml" => vec![
            ("made by cargo jflib new", "made by cargo jflib new --std".to_string()),
            ("jflib = \"", "jflib = { features = [\"std\"], version = \"".to_string()),
            ("build JFLib is tested with.\n", STD_UNWIND.to_string()),
        ],
        "src/main.rs" => return Ok(STD_MAIN.replace("hallo", &krate)),
        ".cargo/config.toml" => return Ok(STD_CONFIG.to_string()),
        "src/lib.rs" => vec![
            ("a JFLib app with a window: the template of the crate jflib, from apps/template of the
// JFLib repository",
             "a JFLib app with a window and std (cargo jflib new --std): the template of the
// crate jflib, from apps/template of the JFLib repository".to_string()),
            ("#![no_std]\n", String::new()),
            ("\nextern crate alloc;\n", String::new()),
            ("alloc::format!", "format!".to_string()),
        ],
        _ => vec![],
    };
    let mut text = text.to_string();
    for (old, new) in pairs {
        if !text.contains(old) {
            return Err(format!("the template's {path} has no {old:?} any more (cargo-jflib's new.rs)"));
        }
        text = text.replacen(old, &new, 1);
    }
    if path == "Cargo.toml" {
        // the version string ends the table: `jflib = { features = ["std"], version = "0.2" }`
        let at = text.find("jflib = { features").expect("replaced above");
        let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
        text.insert_str(end, " }");
    }
    Ok(text)
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let o = crate::opts(args)?;
    let [name] = o.free.as_slice() else { return Err("cargo jflib new NAME".to_string()) };
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && name.len() <= 64;
    if !ok {
        return Err(format!("{name}: a letter, then letters, digits, '_' or '-' (at most 64)"));
    }
    let dir = Path::new(name);
    if dir.exists() {
        return Err(format!("{name} exists already"));
    }
    for (path, text) in FILES {
        if o.std && NOT_STD.contains(path) {
            continue;
        }
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap_or(dir)).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut text = personal(path, text, name)?;
        if o.std {
            text = with_std(path, &text, name)?;
        }
        fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    println!("made {name}/: a JFLib app with a window{}\n", if o.std { " and std" } else { "" });
    println!("    cd {name}");
    println!("    cargo run                      the app");
    println!("    cargo jflib test               its tests/*.uis");
    println!("    cargo jflib web --serve        in the browser");
    if o.std {
        println!("    cargo build (on Linux)         for Linux: with std it is built on Linux itself");
    } else {
        println!("    cargo build --target x86_64-unknown-linux-gnu    for Linux (always with --target)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_takes_the_name() {
        for (path, text) in FILES {
            let t = personal(path, text, "my-app").unwrap();
            assert!(!t.contains("hallo"), "{path} still names hallo");
        }
        assert!(personal("src/main.rs", FILES[6].1, "my-app").unwrap().contains("extern crate my_app;"));
        assert!(personal("PACKAGE", FILES[4].1, "my-app").unwrap().contains("app_id=org.example.my_app"));
    }

    #[test]
    fn std_app() {
        for (path, text) in FILES {
            let t = with_std(path, &personal(path, text, "my-app").unwrap(), "my-app").unwrap();
            assert!(!t.contains("hallo"), "{path} still names hallo");
            assert!(!t.contains("no_std") && !t.contains("no_main") && !t.contains("alloc::"), "{path} is not std");
        }
        let cargo = with_std("Cargo.toml", &personal("Cargo.toml", FILES[0].1, "my-app").unwrap(), "my-app").unwrap();
        let line = cargo.lines().find(|l| l.starts_with("jflib = ")).unwrap();
        assert!(line.starts_with("jflib = { features = [\"std\"], version = \"") && line.ends_with("\" }"), "{line}");
        assert!(cargo.contains("feature \"unwind\"") && cargo.contains("panic = \"abort\""), "{cargo}");
        assert!(with_std("src/main.rs", "", "my-app").unwrap().contains("extern crate my_app;"));
        let config = with_std(".cargo/config.toml", "", "my-app").unwrap();
        assert!(config.contains("+crt-static") && !config.contains("rust-lld") && !config.contains("jfgui_glibc"));
    }
}
