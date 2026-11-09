// cargo jflib test: build the package's program, then run every tests/*.uis with --script in the
// package's folder (a script's first line "#!args ..." adds arguments), as JFLib's build_rs.py does.
// A line "#!only windows linux .." among the script's first lines runs it on these systems only
// (target_os of --target, else of this machine; others: skipped), e.g. scripts of native plugins.

use std::fs;
use std::process::Command;

use crate::cargo;

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let o = crate::opts(args)?;
    let p = cargo::project(o.package.as_deref())?;
    let mut cmd = cargo::command();
    cmd.args(["build", "-p", &p.package.name]);
    if o.release {
        cmd.arg("--release");
    }
    if let Some(t) = &o.target {
        cmd.args(["--target", t]);
    }
    let exe = cargo::build(&mut cmd)?
        .into_iter()
        .filter(|a| a.package_id == p.package.id && a.kinds.iter().any(|k| k == "bin"))
        .find_map(|a| a.executable)
        .ok_or_else(|| format!("{} has no program", p.package.name))?;
    let dir = p.package.dir.join("tests");
    let mut scripts: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.extension().is_some_and(|x| x == "uis"))
        .collect();
    scripts.sort();
    let os = target_os(o.target.as_deref());
    let (mut failed, mut skipped) = (0, 0);
    for s in &scripts {
        let text = fs::read_to_string(s).map_err(|e| format!("{}: {e}", s.display()))?;
        if !runs_on(&text, os) {
            skipped += 1;
            println!("skip tests/{} (#!only: not on {os})", s.file_name().unwrap_or_default().to_string_lossy());
            continue;
        }
        let extra: Vec<&str> = text.lines().next().and_then(|l| l.strip_prefix("#!args")).map(|a| a.split_whitespace().collect()).unwrap_or_default();
        // the folders a `shot FILE` or `dump FILE` writes into (target/ may lie elsewhere: CARGO_TARGET_DIR)
        for file in text.lines().filter_map(|l| l.strip_prefix("shot ").or_else(|| l.strip_prefix("dump "))).filter_map(|r| r.split_whitespace().next()) {
            if let Some(parent) = p.package.dir.join(file).parent() {
                let _ = fs::create_dir_all(parent);
            }
        }
        let rel = format!("tests/{}", s.file_name().unwrap_or_default().to_string_lossy());
        let out = Command::new(&exe).args(&extra).args(["--script", &rel]).current_dir(&p.package.dir).output()
            .map_err(|e| format!("{}: {e}", exe.display()))?;
        if out.status.success() {
            println!("ok   {rel}");
        } else {
            failed += 1;
            println!("FAIL {rel} (exit {})", out.status.code().unwrap_or(-1));
            let all = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            for line in all.lines().filter(|l| ["FAIL", "error", "panic"].iter().any(|p| l.starts_with(p))) {
                println!("     {line}");
            }
        }
    }
    let ran = scripts.len() - skipped;
    println!("{} of {ran} scripts passed{}", ran - failed, if skipped > 0 { format!(" ({skipped} skipped: #!only)") } else { String::new() });
    if failed > 0 { Err(format!("{failed} scripts failed")) } else { Ok(()) }
}

/// the target_os the program is built for: of --target's triple, else of this machine
fn target_os(triple: Option<&str>) -> &'static str {
    let Some(t) = triple else { return std::env::consts::OS };
    if t.contains("windows") {
        "windows"
    } else if t.contains("android") {
        "android"
    } else if t.contains("linux") {
        "linux"
    } else if t.contains("apple-darwin") {
        "macos"
    } else if t.contains("apple-ios") {
        "ios"
    } else if t.starts_with("wasm32") {
        "wasm32"
    } else {
        "other"
    }
}

/// a "#!only os .." line among the script's first five lines: the script runs on these only
fn runs_on(text: &str, os: &str) -> bool {
    match text.lines().take(5).find_map(|l| l.strip_prefix("#!only")) {
        Some(list) => list.split_whitespace().any(|w| w == os),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lines() {
        assert!(runs_on("#!args --x\n# text\n", "macos"));
        assert!(runs_on("#!args --x\n#!only windows linux\n", "linux") && !runs_on("#!args --x\n#!only windows linux\n", "macos"));
        assert_eq!((target_os(Some("aarch64-apple-darwin")), target_os(Some("x86_64-pc-windows-msvc")), target_os(Some("aarch64-linux-android"))),
                   ("macos", "windows", "android"));
    }
}
