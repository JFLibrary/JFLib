//! `cargo jflib` -- what Cargo does not do for a JFLib app: a new project, its test scripts, the
//! bundle for the browser.  `cargo install cargo-jflib`, then:
//!
//! ```text
//! cargo jflib new NAME                 a new app NAME from the template (a window)
//! cargo jflib test [-p PKG] [--release] [--target TRIPLE]
//!                                      build, then run every tests/*.uis with --script
//! cargo jflib web [-p PKG] [--release] [--modes scalar,simd,threads] [--serve [PORT]]
//!                                      the app for the browser in target/jflib-web/NAME/
//! cargo jflib android [-p PKG] [--release] [--abi arm64-v8a|x86_64] [--install]
//!                                      the app as an APK in target/jflib-android/
//! ```
//!
//! It calls Cargo for the builds and needs nothing else; the browser with threads needs the
//! Rust sources (`rustup component add rust-src`) for a core library with atomics, Android the
//! SDK (build-tools, platform 36), the NDK and a JDK.

mod android;
mod cargo;
mod json;
mod new;
mod notices;
mod test;
mod web;
mod zip;

use std::process::ExitCode;

const HELP: &str = "cargo jflib -- a JFLib app's new project, tests and browser bundle

USAGE:
    cargo jflib new NAME
        a new app NAME from the template: a window, its tests, Linux and Windows settings
    cargo jflib test [-p PKG] [--release] [--target TRIPLE]
        build, then run every tests/*.uis of the package with --script
    cargo jflib web [-p PKG] [--release] [--modes scalar,simd,threads] [--serve [PORT]]
        the app for the browser: target/jflib-web/NAME/ (index.html, the modules, the host);
        --serve then serves it on 127.0.0.1 (default port 8766) with the headers threads need
    cargo jflib android [-p PKG] [--release] [--abi arm64-v8a|x86_64] [--install]
        the app as an APK: target/jflib-android/NAME-ABI-debug.apk (--release: signed with the key
        of jfgui_ANDROID_KEYSTORE); needs the Android SDK and NDK and a JDK; --install: adb install
    cargo jflib --version";

/// the options every command shares
#[derive(Default, Debug)]
pub(crate) struct Opts {
    pub package: Option<String>,
    pub release: bool,
    pub target: Option<String>,
    pub modes: Option<String>,
    pub serve: Option<u16>,
    pub free: Vec<String>,
}

pub(crate) fn opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    let value = |i: &mut usize, flag: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("{flag} needs a value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--package" => o.package = Some(value(&mut i, "-p")?),
            "--release" => o.release = true,
            "--target" => o.target = Some(value(&mut i, "--target")?),
            "--modes" => o.modes = Some(value(&mut i, "--modes")?),
            "--serve" => {
                let port = args.get(i + 1).and_then(|p| p.parse::<u16>().ok());
                if port.is_some() {
                    i += 1;
                }
                o.serve = Some(port.unwrap_or(8766));
            }
            a if a.starts_with('-') => return Err(format!("unknown option {a}")),
            a => o.free.push(a.to_string()),
        }
        i += 1;
    }
    Ok(o)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("jflib") {
        args.remove(0); // `cargo jflib X` runs `cargo-jflib jflib X`
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new::run(&args[1..]),
        Some("test") => test::run(&args[1..]),
        Some("web") => web::run(&args[1..]),
        Some("android") => android::run_cmd(&args[1..]),
        Some("--version" | "-V") => {
            println!("cargo-jflib {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None | Some("--help" | "-h" | "help") => {
            println!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}\n\n{HELP}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
