// cargo jflib new NAME: the template of the crate jflib (lib/jflib/template in JFLib's repository;
// the copy in this package is kept equal there), with the app's name.

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
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap_or(dir)).map_err(|e| format!("{}: {e}", file.display()))?;
        fs::write(&file, personal(path, text, name)?).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    println!("made {name}/: a JFLib app with a window\n");
    println!("    cd {name}");
    println!("    cargo run                      the app");
    println!("    cargo jflib test               its tests/*.uis");
    println!("    cargo jflib web --serve        in the browser");
    println!("    cargo build --target x86_64-unknown-linux-gnu    for Linux (always with --target)");
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
}
